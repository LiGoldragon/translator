//! A CLI's whole invocation: one datom, and the place its configuration
//! names.

use std::process::ExitCode;

use nexus_exchange::Named;
use shared_translator::placement::PlaceResolutionError;

use crate::client::{ClientError, TextClient, TextRequesting, TranslatorPlacing};
use crate::configuration::{ConfigurationFile, ConfigurationLocating, ConfigurationReading};
use crate::generated::{ConfigurationRefusal, TextLane};

/// The text lane a CLI speaks on, and the one datom it sends.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextInvocation {
    pub client: TextClient,
    pub text: String,
}

/// Why a CLI sends nothing.
#[derive(Debug)]
pub enum InvocationRefusal {
    /// The arguments are not one datom.
    Usage,
    /// The configuration cannot be found, read or understood.
    Configuration(ConfigurationRefusal),
    /// The place the configuration names does not resolve.
    Place(PlaceResolutionError),
}

pub trait Invoking: Sized {
    /// Reads `'<datom>'`, the CLI's whole argument list, and places the lane
    /// `lane` of the translator of the Nexus `Component` in the place
    /// `configuration` names, resolved under the runtime root. Each frame is
    /// bounded by Signal's largest.
    fn from_arguments<Component: Named>(
        lane: TextLane,
        arguments: impl Iterator<Item = String>,
        configuration: &ConfigurationFile,
    ) -> Result<Self, InvocationRefusal>;
}

impl Invoking for TextInvocation {
    fn from_arguments<Component: Named>(
        lane: TextLane,
        mut arguments: impl Iterator<Item = String>,
        configuration: &ConfigurationFile,
    ) -> Result<Self, InvocationRefusal> {
        let (Some(text), None) = (arguments.next(), arguments.next()) else {
            return Err(InvocationRefusal::Usage);
        };
        let configured = configuration
            .read()
            .map_err(InvocationRefusal::Configuration)?;
        let client = TextClient::placed::<Component>(
            &configured.place_name.0,
            lane,
            signal::MAXIMUM_SIGNAL_BYTES,
        )
        .map_err(InvocationRefusal::Place)?;
        Ok(Self { client, text })
    }
}

pub trait AnswerPrinting {
    /// Sends the datom and prints each answer on a line of its own, until
    /// the translator ends them.
    fn print_answers(&self) -> Result<(), ClientError>;
}

impl AnswerPrinting for TextInvocation {
    fn print_answers(&self) -> Result<(), ClientError> {
        self.client.request_stream(&self.text, |answer| {
            println!("{answer}");
            true
        })
    }
}

/// A CLI run whole: its arguments, its configuration, its answers.
pub trait TextCommand {
    /// Runs the CLI of the Nexus `Component` on `lane`: each answer printed,
    /// then success; a failure the translator answered, or a broken lane,
    /// printed on standard error, exit 1; a refusal before anything is sent,
    /// printed on standard error, a configuration refusal as its datom
    /// naming the file, exit 2.
    fn run<Component: Named>(lane: TextLane, arguments: impl Iterator<Item = String>) -> ExitCode;
}

impl TextCommand for TextInvocation {
    fn run<Component: Named>(lane: TextLane, arguments: impl Iterator<Item = String>) -> ExitCode {
        let invocation = ConfigurationFile::of_process::<Component>()
            .map_err(InvocationRefusal::Configuration)
            .and_then(|configuration| {
                Self::from_arguments::<Component>(lane, arguments, &configuration)
            });
        match invocation {
            Ok(invocation) => match invocation.print_answers() {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => {
                    eprintln!("{error}");
                    ExitCode::FAILURE
                }
            },
            Err(refusal) => {
                eprintln!("{}", refusal.text::<Component>());
                ExitCode::from(InvocationRefusal::EXIT)
            }
        }
    }
}

/// A refusal as the CLI reports it.
pub trait RefusalReporting {
    /// The status a CLI that sent nothing exits with.
    const EXIT: u8 = 2;

    /// What the CLI prints: a configuration refusal as its datom, naming the
    /// file.
    fn text<Component: Named>(&self) -> String;
}

impl RefusalReporting for InvocationRefusal {
    fn text<Component: Named>(&self) -> String {
        use datom_codec::Datomizable;
        use protos::{Protosizable, Textualizable};
        match self {
            Self::Usage => format!(
                "usage: {name} '<datom>'; the place is the one {name}.datom names in the configuration home",
                name = Component::name().0,
            ),
            Self::Configuration(refusal) => refusal.datomize(Vec::new()).protosize().textualize(),
            Self::Place(error) => error.to_string(),
        }
    }
}
