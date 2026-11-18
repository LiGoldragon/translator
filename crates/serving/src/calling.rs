//! The translator's calls to its Nexus, on the exchange layer.

use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

use nexus_querying::{Patience, Querying, QueryingConnection};
use signal::{
    Contracted, Delivery, Dispatch, ExchangeId, FrameCapacity, Restorable, Signal, Signalizable,
};

use crate::error::TranslatorError;

/// A call a Nexus socket answers with `Response`: one query of its call
/// contract, carried on one exchange of a greeted connection.
pub trait NexusCalling<Response>: Contracted + Sized {
    /// Greets the socket, opens one exchange carrying the call and returns
    /// its one answer, waiting as `patience` allows.
    fn ask(
        self,
        socket: &Path,
        patience: Patience,
        capacity: FrameCapacity,
    ) -> Result<Response, TranslatorError>;

    /// Greets the socket, opens one exchange carrying the call and hands each
    /// answer to `answered`, until the Nexus ends the exchange or closes the
    /// connection. Each frame is read and written within `progress`.
    fn follow(
        self,
        socket: &Path,
        progress: Duration,
        capacity: FrameCapacity,
        answered: impl FnMut(Response) -> Result<(), TranslatorError>,
    ) -> Result<(), TranslatorError>;
}

impl<Query, Response> NexusCalling<Response> for Query
where
    Query: Contracted,
    Dispatch<Query>: Signalizable,
    Response: rkyv::Archive,
    Signal<Delivery<Response>>: Restorable<Delivery<Response>>,
{
    fn ask(
        self,
        socket: &Path,
        patience: Patience,
        capacity: FrameCapacity,
    ) -> Result<Response, TranslatorError> {
        let stream = UnixStream::connect(socket).map_err(TranslatorError::Io)?;
        let mut connection =
            QueryingConnection::<Query, Response>::greeted(stream, patience, capacity)?;
        let exchange = connection.open(self)?;
        connection.only_answer(exchange)
    }

    fn follow(
        self,
        socket: &Path,
        progress: Duration,
        capacity: FrameCapacity,
        mut answered: impl FnMut(Response) -> Result<(), TranslatorError>,
    ) -> Result<(), TranslatorError> {
        let stream = UnixStream::connect(socket).map_err(TranslatorError::Io)?;
        let mut connection = QueryingConnection::<Query, Response>::greeted(
            stream,
            Patience::Progress(progress),
            capacity,
        )?;
        let exchange = connection.open(self)?;
        loop {
            match connection.delivery()? {
                Some(Delivery::Answer(answer)) if answer.exchange == exchange => {
                    answered(answer.response)?;
                }
                Some(Delivery::End(ending)) if ending.exchange == exchange => return Ok(()),
                Some(_) => {
                    return Err(TranslatorError::Signal(
                        "Nexus answered another exchange".into(),
                    ));
                }
                None => return Ok(()),
            }
        }
    }
}

/// The querying end of a one-answer exchange.
trait OnlyAnswering {
    type Response;

    /// The one answer of the exchange.
    fn only_answer(&mut self, exchange: ExchangeId) -> Result<Self::Response, TranslatorError>;
}

impl<Query, Response> OnlyAnswering for QueryingConnection<Query, Response>
where
    Query: Contracted,
    Dispatch<Query>: Signalizable,
    Response: rkyv::Archive,
    Signal<Delivery<Response>>: Restorable<Delivery<Response>>,
{
    type Response = Response;

    fn only_answer(&mut self, exchange: ExchangeId) -> Result<Response, TranslatorError> {
        match self.delivery()? {
            Some(Delivery::Answer(answer)) if answer.exchange == exchange => Ok(answer.response),
            Some(Delivery::End(ending)) => Err(TranslatorError::Signal(format!(
                "Nexus ended the exchange without an answer: {:?}",
                ending.conclusion
            ))),
            Some(_) => Err(TranslatorError::Signal(
                "Nexus answered another exchange".into(),
            )),
            None => Err(TranslatorError::Signal(
                "Nexus closed without an answer".into(),
            )),
        }
    }
}

/// An emission restored from the Signal value the Nexus sends.
pub trait EmissionRestoring: Sized {
    fn restored(payload: Vec<u8>) -> Result<Self, TranslatorError>;
}

impl<Emission> EmissionRestoring for Emission
where
    Emission: rkyv::Archive,
    Signal<Emission>: Restorable<Emission>,
{
    fn restored(payload: Vec<u8>) -> Result<Self, TranslatorError> {
        Signal::<Emission>::from(payload)
            .restore()
            .map_err(|error| TranslatorError::Signal(format!("{error:?}")))
    }
}
