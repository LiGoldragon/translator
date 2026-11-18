//! A CLI's whole invocation: a place's name and one datom.

use nexus_exchange::Named;

use crate::client::{ClientError, TextClient, TextRequesting, TranslatorPlacing};
use crate::generated::TextLane;

/// The text lane a CLI speaks on, and the one datom it sends.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextInvocation {
    pub client: TextClient,
    pub text: String,
}

pub trait Invoking: Sized {
    /// Reads `<place-name> '<datom>'`, the CLI's whole argument list, and
    /// places the lane `lane` of the translator of the Nexus `Component` in
    /// the named place. Each frame is bounded by Signal's largest.
    fn from_arguments<Component: Named>(
        lane: TextLane,
        arguments: impl Iterator<Item = String>,
    ) -> Result<Self, Box<dyn std::error::Error>>;
}

impl Invoking for TextInvocation {
    fn from_arguments<Component: Named>(
        lane: TextLane,
        mut arguments: impl Iterator<Item = String>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let name = arguments.next().ok_or("METAFLOW")?;
        let text = arguments.next().ok_or("DATOM")?;
        if arguments.next().is_some() {
            return Err("unexpected argument".into());
        }
        let client = TextClient::placed::<Component>(&name, lane, signal::MAXIMUM_SIGNAL_BYTES)?;
        Ok(Self { client, text })
    }
}

pub trait AnswerPrinting {
    /// Sends the datom and prints each answer on a line of its own: on the
    /// ordinary lane every answer, until the translator closes the connection;
    /// on the meta lane its one answer.
    fn print_answers(&self) -> Result<(), ClientError>;
}

impl AnswerPrinting for TextInvocation {
    fn print_answers(&self) -> Result<(), ClientError> {
        match self.client.lane {
            TextLane::PublicText => self.client.request_stream(&self.text, |answer| {
                println!("{answer}");
                true
            }),
            TextLane::MetaText => {
                println!("{}", self.client.request(&self.text)?);
                Ok(())
            }
        }
    }
}
