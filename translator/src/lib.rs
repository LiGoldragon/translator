//! The translator library, written once for every Nexus: the sockets of a
//! translator and the serving of each, the caller, the CLI and the hook client,
//! a lasting emission and the running of a program. What a Nexus adds is its
//! name, and the words of its own calls.

mod client;
mod lasting;
mod running;
mod serving;
mod translatable;

pub use client::{Calling, Ending, Hooking, TextSocket};
pub use lasting::{Growth, Lasted, Lasting, Lines, Waited};
pub use running::{Captured, capture, run};
pub use serving::{Emitting, Served, Translator};
pub use translatable::{Translatable, Untranslated};
/// The transport, joined to the library: a Nexus's place, its framing and its emission socket.
pub use transport;
pub use transport::{Caller, EmissionSocket, Framed, Held, Named, Place, PlaceName, Placed, Runtime, State, Unobserved, Unsent};
