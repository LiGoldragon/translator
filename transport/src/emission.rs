//! The translator's emission socket, as the Nexus holds it: one connection for
//! each emission, dialed by the Nexus.

use std::marker::PhantomData;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;

use datom_codec::{Composing, Datomizable};

use crate::{Framed, Named, Place, Placed};

/// An emission that found no translator, or whose answer never came: the two are
/// kept apart, since after the second the call may have been made.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize, Datomizable, Composing)]
pub enum Unobserved {
    /// No translator listens: nothing was done.
    Unsubmitted,
    /// The connection closed before the outcome: the call may have been made.
    Unanswered,
}

/// The translator's emission socket, from the Nexus's place.
#[derive(Clone, Debug)]
pub struct EmissionSocket {
    path: PathBuf,
}

impl EmissionSocket {
    pub fn of<N: Named>(place: &Place) -> EmissionSocket {
        EmissionSocket { path: place.path::<N>(Placed::EmissionSocket) }
    }

    fn dial<E: Framed>(&self, emission: &E) -> Result<UnixStream, Unobserved> {
        let mut stream = UnixStream::connect(&self.path).map_err(|_| Unobserved::Unsubmitted)?;
        emission.write_to(&mut stream).map_err(|_| Unobserved::Unsubmitted)?;
        Ok(stream)
    }

    /// One emission and its one outcome.
    pub fn send<E: Framed, O: Framed>(&self, emission: &E) -> Result<O, Unobserved> {
        let mut stream = self.dial(emission)?;
        match O::read_from(&mut stream) {
            Ok(Some(outcome)) => Ok(outcome),
            _ => Err(Unobserved::Unanswered),
        }
    }

    /// A lasting emission: the connection stays open, and an outcome comes back
    /// on it each time what was asked for happens, until it is dropped.
    pub fn hold<E: Framed, O: Framed>(&self, emission: &E) -> Result<Held<O>, Unobserved> {
        Ok(Held { stream: self.dial(emission)?, outcome: PhantomData })
    }
}

/// A lasting emission, as the Nexus holds it: dropping it closes the connection,
/// which ends what the translator holds for it.
#[derive(Debug)]
pub struct Held<O> {
    stream: UnixStream,
    outcome: PhantomData<fn() -> O>,
}

impl<O: Framed> Held<O> {
    /// The next outcome, when it comes; Unanswered when the translator closes the
    /// connection first.
    pub fn outcome(&mut self) -> Result<O, Unobserved> {
        match O::read_from(&mut self.stream) {
            Ok(Some(outcome)) => Ok(outcome),
            _ => Err(Unobserved::Unanswered),
        }
    }
}

impl<O> Held<O> {
    /// The connection, for a Nexus that waits on many at once.
    pub fn stream(&self) -> &UnixStream {
        &self.stream
    }
}

