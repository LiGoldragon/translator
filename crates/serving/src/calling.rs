//! The translator's calls to its Nexus, on the exchange layer.

use std::os::unix::net::UnixStream;
use std::path::Path;

use nexus_querying::{ExchangeFollowing, Patience, Querying, QueryingConnection};
use signal::{Contracted, Delivery, Dispatch, FrameCapacity, Restorable, Signal, Signalizable};

use crate::error::TranslatorError;

/// A call a Nexus socket answers with `Response`: one query of its call
/// contract, carried on one exchange of a greeted connection.
pub trait NexusCalling<Response>: Contracted + Sized {
    /// Greets the socket, opens one exchange carrying the call and hands each
    /// answer to `answered`, until the Nexus ends the exchange, waiting as
    /// `patience` allows. Every exchange ends, a one-answer exchange after
    /// its answer as a subscription after its last frame, so the call needs
    /// no knowledge of which it is: an end Completed is `Ok`, and a fault or
    /// a connection closed before the end is the error.
    fn call(
        self,
        socket: &Path,
        patience: Patience,
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
    fn call(
        self,
        socket: &Path,
        patience: Patience,
        capacity: FrameCapacity,
        answered: impl FnMut(Response) -> Result<(), TranslatorError>,
    ) -> Result<(), TranslatorError> {
        let stream = UnixStream::connect(socket).map_err(TranslatorError::Io)?;
        let mut connection =
            QueryingConnection::<Query, Response>::greeted(stream, patience, capacity)?;
        let exchange = connection.open(self)?;
        connection.follow(exchange, answered)
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
