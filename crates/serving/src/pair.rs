//! The pair launcher: a Nexus and its translator, started together, each
//! naming the other, and ended together.

use std::io::Write;
use std::process::{Child, Command, Stdio};

use nexus_exchange::{
    Admission, FrameCapacityBytes, FrameDeadlineMilliseconds, RetryDelayMilliseconds,
    ValidAdmission,
};
use nexus_serving::{InstanceId, NexusBootstrap};
use nix::sys::signal::{Signal as ProcessSignal, kill};
use nix::sys::wait::waitpid;
use nix::unistd::{Pid, Uid};
use shared_translator::placement::{
    Place, Preparable, runtime_path, state_path, validate_place_name,
};
use shared_translator::{
    FrameCapacity, FrameWriting, PeerIdentity, ProcessId, Signalizable, UserId,
};

use crate::budget::{Budgeting, ValidTextBudget};
use crate::generated::{
    DepthLimit, NexusExecutable, NodeLimit, Pairing, TextLimits, TranslatorBootstrap,
    TranslatorExecutable,
};
use crate::instance::FreshInstance;

type Failure = Box<dyn std::error::Error>;

pub trait PairReading: Sized {
    /// Reads the launcher's whole argument list: `<Place> <nexus-executable>
    /// <translator-executable> <frame-deadline-milliseconds>
    /// <frame-capacity-bytes> <retry-delay-milliseconds> <text-node-limit>
    /// <text-depth-limit>`, the place as its datom. The admission is checked
    /// here; the text limits are the translator's to check.
    fn from_arguments(arguments: impl Iterator<Item = String>) -> Result<Self, Failure>;
}

impl PairReading for Pairing {
    fn from_arguments(mut arguments: impl Iterator<Item = String>) -> Result<Self, Failure> {
        let place_text = arguments.next().ok_or("PLACE")?;
        let nexus_executable = NexusExecutable(arguments.next().ok_or("NEXUS_EXECUTABLE")?);
        let translator_executable =
            TranslatorExecutable(arguments.next().ok_or("TRANSLATOR_EXECUTABLE")?);
        let admission = Admission {
            frame_deadline_milliseconds: FrameDeadlineMilliseconds(
                arguments.integer("FRAME_DEADLINE_MILLISECONDS")?,
            ),
            frame_capacity_bytes: FrameCapacityBytes(arguments.integer("FRAME_CAPACITY_BYTES")?),
            retry_delay_milliseconds: RetryDelayMilliseconds(
                arguments.integer("RETRY_DELAY_MILLISECONDS")?,
            ),
        };
        ValidAdmission::try_from(&admission)
            .map_err(|message| format!("invalid admission: {message}"))?;
        let text_limits = TextLimits {
            node_limit: NodeLimit(arguments.integer("TEXT_NODE_LIMIT")?),
            depth_limit: DepthLimit(arguments.integer("TEXT_DEPTH_LIMIT")?),
        };
        if arguments.next().is_some() {
            return Err("unexpected launcher argument".into());
        }
        let place =
            shared_translator::decode::<Place>(&place_text, &mut ValidTextBudget::MAXIMUM.budget())
                .map_err(|error| format!("{error:?}"))?;
        Ok(Self {
            place,
            nexus_executable,
            translator_executable,
            admission,
            text_limits,
        })
    }
}

/// An argument list read one integer at a time.
trait IntegerArguments {
    /// The next argument as an integer; `name` names it when it is missing.
    fn integer(&mut self, name: &str) -> Result<i64, Failure>;
}

impl<A: Iterator<Item = String>> IntegerArguments for A {
    fn integer(&mut self, name: &str) -> Result<i64, Failure> {
        self.next()
            .ok_or_else(|| name.to_owned())?
            .parse()
            .map_err(Into::into)
    }
}

pub trait PairRunning {
    /// Prepares the place, starts the Nexus and the translator, and writes
    /// each its bootstrap naming the other's process, so each pins the other:
    /// the Nexus's [`NexusBootstrap`] with a fresh [`InstanceId`] for this
    /// run, the translator's with the text limits it starts with.
    /// It then waits: when either process ends, the other is stopped, and the
    /// launch ends in an error naming the one that ended. It never returns
    /// otherwise.
    fn launch(self) -> Result<std::convert::Infallible, Failure>;
}

impl PairRunning for Pairing {
    fn launch(self) -> Result<std::convert::Infallible, Failure> {
        let place = &self.place;
        validate_place_name(&place.place_name.0)?;
        place.prepare_for_start()?;
        if !runtime_path(place).is_dir() {
            return Err(format!("metaflow runtime does not exist: {}", place.runtime.0).into());
        }
        if !state_path(place).is_dir() {
            return Err(format!("metaflow state does not exist: {}", place.state.0).into());
        }

        let mut nexus = Command::new(&self.nexus_executable.0)
            .stdin(Stdio::piped())
            .spawn()?;
        let mut translator = match Command::new(&self.translator_executable.0)
            .stdin(Stdio::piped())
            .spawn()
        {
            Ok(child) => child,
            Err(error) => {
                nexus.stop();
                return Err(error.into());
            }
        };
        let user_id = UserId(i64::from(Uid::current().as_raw()));
        let nexus_process = ProcessId(i64::from(nexus.id()));
        let translator_process = ProcessId(i64::from(translator.id()));
        let bootstrapped = (|| -> Result<(), Failure> {
            nexus.bootstrap(&NexusBootstrap {
                place: self.place.clone(),
                admission: self.admission.clone(),
                peer_identity: PeerIdentity {
                    user_id,
                    process_id: translator_process,
                },
                instance_id: InstanceId::fresh()?,
            })?;
            translator.bootstrap(&TranslatorBootstrap {
                place: self.place.clone(),
                admission: self.admission.clone(),
                text_limits: self.text_limits.clone(),
                peer_identity: PeerIdentity {
                    user_id,
                    process_id: nexus_process,
                },
            })
        })();
        if let Err(error) = bootstrapped {
            nexus.stop();
            translator.stop();
            return Err(error);
        }

        let nexus = nexus.pid();
        let translator = translator.pid();
        let ended = waitpid(Pid::from_raw(-1), None)?;
        let survivor = if ended.pid() == Some(nexus) {
            translator
        } else {
            nexus
        };
        let _ = kill(survivor, ProcessSignal::SIGTERM);
        let _ = waitpid(survivor, None);
        Err(format!("paired process ended: {ended:?}").into())
    }
}

/// One process of the pair.
trait Paired {
    /// Writes the process its one bootstrap frame and closes its standard
    /// input.
    fn bootstrap(&mut self, value: &impl Signalizable) -> Result<(), Failure>;

    fn pid(&self) -> Pid;

    /// Asks the process to end, and waits for it.
    fn stop(&mut self);
}

impl Paired for Child {
    fn bootstrap(&mut self, value: &impl Signalizable) -> Result<(), Failure> {
        let mut input = self.stdin.take().ok_or("bootstrap pipe")?;
        input.write_frame(
            &value.signalize()?,
            FrameCapacity::from(signal::MAXIMUM_SIGNAL_BYTES),
        )?;
        input.flush()?;
        Ok(())
    }

    fn pid(&self) -> Pid {
        Pid::from_raw(i32::try_from(self.id()).expect("child pid fits i32"))
    }

    fn stop(&mut self) {
        let pid = self.pid();
        let _ = kill(pid, ProcessSignal::SIGTERM);
        let _ = waitpid(pid, None);
    }
}
