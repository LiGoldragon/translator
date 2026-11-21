//! What a Nexus, its translator and its clients share: the place they start in,
//! the sockets and files named under it, the caller, the framing of Signal on a
//! stream, and the emission socket as the Nexus holds it.

mod caller;
mod emission;
mod framing;
mod place;

pub use caller::{Caller, ProcessId, UserId};
pub use emission::{EmissionSocket, Held, Unobserved};
pub use framing::{Framed, MAXIMUM_FRAME};
pub use place::{CliConfiguration, Named, Place, PlaceName, Placed, Runtime, State, Unsent, actualize};
