//! One text lane of a translator, at the socket it is served on.

use std::os::unix::net::UnixStream;
use std::path::PathBuf;

use nexus_exchange::lane::{Frame, Kind, Lane, LaneReading, LaneWriting, TransportError};
use nexus_exchange::{Named, SocketPaths};
use shared_translator::placement::{Place, PlaceResolutionError, resolve_place};

use crate::generated::TextLane;

/// One text lane of a translator, at the socket it is served on, and the
/// largest frame written to it or read from it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextClient {
    pub socket: PathBuf,
    pub lane: TextLane,
    pub admission_limit: usize,
}

impl From<TextLane> for Lane {
    fn from(lane: TextLane) -> Self {
        match lane {
            TextLane::PublicText => Self::PublicText,
            TextLane::MetaText => Self::MetaText,
        }
    }
}

/// A text lane found by the place it is served in.
pub trait TranslatorPlacing: Sized {
    /// The lane of the translator of the Nexus `Component` in `place`: its
    /// socket is named after the Nexus, in the place's runtime directory.
    fn at<Component: Named>(place: &Place, lane: TextLane, admission_limit: usize) -> Self;

    /// The lane of the translator of the Nexus `Component` in the place named
    /// `place_name`, resolved from the environment.
    fn placed<Component: Named>(
        place_name: &str,
        lane: TextLane,
        admission_limit: usize,
    ) -> Result<Self, PlaceResolutionError> {
        resolve_place(place_name).map(|place| Self::at::<Component>(&place, lane, admission_limit))
    }
}

impl TranslatorPlacing for TextClient {
    fn at<Component: Named>(place: &Place, lane: TextLane, admission_limit: usize) -> Self {
        let paths = SocketPaths::<Component>::from(place);
        let socket = match lane {
            TextLane::PublicText => paths.translator,
            TextLane::MetaText => paths.meta_translator,
        };
        Self {
            socket,
            lane,
            admission_limit,
        }
    }
}

/// A text lane a datom is sent on and answered on.
pub trait TextRequesting {
    /// Sends `text` and returns the translator's first answer, closing the
    /// connection after it.
    fn request(&self, text: &str) -> Result<String, ClientError>;

    /// Sends `text` and delivers each answer in order, until the translator
    /// ends them: `Ended` is `Ok`, `Failed` is the error carrying its text,
    /// and a connection closed before either is a broken lane. Returning
    /// false stops after that answer and closes the connection.
    fn request_stream(
        &self,
        text: &str,
        receive: impl FnMut(&str) -> bool,
    ) -> Result<(), ClientError>;
}

impl TextRequesting for TextClient {
    fn request(&self, text: &str) -> Result<String, ClientError> {
        let mut first = None;
        self.request_stream(text, |answer| {
            first = Some(answer.to_owned());
            false
        })?;
        first.ok_or_else(|| ClientError::Protocol("translator ended without an answer".into()))
    }

    fn request_stream(
        &self,
        text: &str,
        mut receive: impl FnMut(&str) -> bool,
    ) -> Result<(), ClientError> {
        let lane = Lane::from(self.lane.clone());
        let mut stream = UnixStream::connect(&self.socket).map_err(ClientError::Io)?;
        stream
            .write_lane_frame(
                &Frame {
                    lane: lane.clone(),
                    kind: Kind::Request,
                    payload: text.as_bytes().to_vec(),
                },
                self.admission_limit,
            )
            .map_err(ClientError::Transport)?;
        loop {
            let response = stream
                .read_lane_frame(self.admission_limit)
                .map_err(ClientError::Transport)?;
            if response.lane != lane {
                return Err(ClientError::Protocol(
                    "translator answered on another lane".into(),
                ));
            }
            let text = String::from_utf8(response.payload)
                .map_err(|_| ClientError::Protocol("translator returned non-UTF-8 text".into()))?;
            match response.kind {
                Kind::Answered if receive(&text) => {}
                Kind::Answered | Kind::Ended => return Ok(()),
                Kind::Failed => return Err(ClientError::Protocol(text)),
                Kind::Request => {
                    return Err(ClientError::Protocol(
                        "translator answered with request kind".into(),
                    ));
                }
            }
        }
    }
}

/// What goes wrong for a CLI on a text lane.
#[derive(Debug)]
pub enum ClientError {
    Io(std::io::Error),
    Transport(TransportError),
    /// The translator broke the lane's protocol, or failed the request with
    /// this text.
    Protocol(String),
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "translator client error: {self:?}")
    }
}

impl std::error::Error for ClientError {}
