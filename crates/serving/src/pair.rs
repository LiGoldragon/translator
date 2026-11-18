//! The pair launcher: a Nexus and its translator, started together, each
//! naming the other, and ended together.
//!
//! The launcher waits on events alone, on a runtime of its own: tokio's
//! signal streams carry SIGTERM and SIGINT, read through handlers that every
//! child loses at exec, and a pidfd for each child becomes readable when that
//! child ends. The waiting is hand-written: it holds signal streams and file
//! descriptors, which Ethos cannot name.

use std::io::Write;
use std::os::fd::{AsFd, OwnedFd};
use std::process::{Child, Command, Stdio};

use nexus_exchange::{
    Admission, FrameCapacityBytes, FrameDeadlineMilliseconds, RetryDelayMilliseconds,
    ValidAdmission,
};
use nexus_serving::{InstanceId, NexusBootstrap};
use nix::sys::signal::{Signal as ProcessSignal, kill};
use nix::unistd::{Pid, Uid};
use shared_translator::placement::{
    Place, Preparable, runtime_path, state_path, validate_place_name,
};
use shared_translator::{
    FrameCapacity, FrameWriting, PeerIdentity, ProcessId, Signalizable, UserId,
};
use tokio::io::Interest;
use tokio::io::unix::AsyncFd;
use tokio::signal::unix::{Signal as SignalStream, SignalKind, signal};

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
    /// launch ends in an error naming the one that ended; when the launcher
    /// is told to end, by SIGTERM or SIGINT, both are stopped, and the launch
    /// ends in an error naming the signal. It never returns otherwise.
    ///
    /// SIGTERM and SIGINT are caught from before either child starts, so a
    /// signal at any moment after is read; each child starts with neither
    /// caught nor blocked, since exec resets a caught signal.
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

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_io()
            .build()?;
        let told = Told::listen(&runtime)?;
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

        let ending = (|| -> Result<Ending, Failure> {
            let nexus_ended = nexus.ended()?;
            let translator_ended = translator.ended()?;
            runtime.block_on(told.awaited(&nexus_ended, &translator_ended))
        })();
        nexus.stop();
        translator.stop();
        match ending? {
            Ending::Told(signal) => Err(format!("launcher told to end: {signal}").into()),
            Ending::Nexus => Err(format!("paired process ended: the Nexus {}", nexus.id()).into()),
            Ending::Translator => {
                Err(format!("paired process ended: the translator {}", translator.id()).into())
            }
        }
    }
}

/// What ended the pair's run.
enum Ending {
    /// The launcher was told to end, by this signal.
    Told(ProcessSignal),
    Nexus,
    Translator,
}

/// The signals that tell the launcher to end, read as events.
struct Told {
    terminate: SignalStream,
    interrupt: SignalStream,
}

impl Told {
    /// Catches SIGTERM and SIGINT on the launcher's runtime, before any child
    /// starts.
    fn listen(runtime: &tokio::runtime::Runtime) -> Result<Self, Failure> {
        let _entered = runtime.enter();
        Ok(Self {
            terminate: signal(SignalKind::terminate())?,
            interrupt: signal(SignalKind::interrupt())?,
        })
    }

    /// Waits for the first event: a signal caught, or a child's pidfd
    /// readable once it ends.
    async fn awaited(mut self, nexus: &OwnedFd, translator: &OwnedFd) -> Result<Ending, Failure> {
        let nexus = AsyncFd::with_interest(nexus.as_fd(), Interest::READABLE)?;
        let translator = AsyncFd::with_interest(translator.as_fd(), Interest::READABLE)?;
        tokio::select! {
            _ = self.terminate.recv() => Ok(Ending::Told(ProcessSignal::SIGTERM)),
            _ = self.interrupt.recv() => Ok(Ending::Told(ProcessSignal::SIGINT)),
            ended = nexus.readable() => ended.map(|_| Ending::Nexus).map_err(Into::into),
            ended = translator.readable() => ended.map(|_| Ending::Translator).map_err(Into::into),
        }
    }
}

/// One process of the pair.
trait Paired {
    /// Writes the process its one bootstrap frame and closes its standard
    /// input.
    fn bootstrap(&mut self, value: &impl Signalizable) -> Result<(), Failure>;

    fn pid(&self) -> Pid;

    /// A pidfd that becomes readable when the process ends.
    fn ended(&self) -> Result<OwnedFd, Failure>;

    /// Asks the process to end, unless it has, and waits for it.
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

    fn ended(&self) -> Result<OwnedFd, Failure> {
        Ok(rustix::process::pidfd_open(
            rustix::process::Pid::from_child(self),
            rustix::process::PidfdFlags::empty(),
        )?)
    }

    fn stop(&mut self) {
        if let Ok(Some(_)) = self.try_wait() {
            return;
        }
        let _ = kill(self.pid(), ProcessSignal::SIGTERM);
        let _ = self.wait();
    }
}
