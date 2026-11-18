//! What goes wrong for a translator, and the text a client is answered with.

use nexus_exchange::ExchangeError;
use nexus_exchange::lane::TransportError;
use shared_translator::FrameError;

use crate::generated::{NexusFailure, Refusal};

/// What goes wrong serving one connection. A failure answered to a client is
/// this error's text.
#[derive(Debug)]
pub enum TranslatorError {
    Io(std::io::Error),
    Frame(FrameError),
    Transport(TransportError),
    Signal(String),
}

impl std::fmt::Display for TranslatorError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for TranslatorError {}

impl From<ExchangeError> for TranslatorError {
    fn from(error: ExchangeError) -> Self {
        match error {
            ExchangeError::Frame(FrameError::Io(error)) => Self::Io(error),
            ExchangeError::Frame(error) => Self::Frame(error),
            other => Self::Signal(other.to_string()),
        }
    }
}

impl From<NexusFailure> for TranslatorError {
    fn from(failure: NexusFailure) -> Self {
        Self::Signal(failure.0)
    }
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::LaneUnavailable => "lane is unavailable on this endpoint",
            Self::ForeignLogin => "caller is outside the bound login",
            Self::UnpairedEmitter => "caller is not the paired Nexus",
            Self::AwaitingSettings => "translator is awaiting settings",
        })
    }
}
