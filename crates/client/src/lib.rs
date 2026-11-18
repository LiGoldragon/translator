//! The client end of a translator's text lanes.
//!
//! A CLI takes one datom and nothing else. Its place is its configuration's,
//! the one datom `$XDG_CONFIG_HOME/nexus/<nexus-name>.datom`
//! (`~/.config/nexus/<nexus-name>.datom` when the variable is unset), `{
//! <place-name> }`, resolved under the runtime root to the place's runtime
//! directory. It sends the datom, as text and nothing more, on one of the two
//! text lanes of the translator that speaks for a Nexus, at the socket the
//! translator serves that lane on in the place's runtime directory, and reads
//! every answer on the same connection until the translator ends them. A
//! configuration missing, unreadable or invalid is refused naming the file,
//! `Missing.<path>`, `Unreadable.{ <path> <reason> }` or `Invalid.{ <path>
//! <reason> }`, with exit 2, and nothing is sent. The CLI holds no contract
//! type: reading the text is the translator's. The invocation reads Datom,
//! so it is compiled in under the `datom` feature, which a CLI turns on.
//!
//! The kinds declared in `ethos/client.ethos` are generated into
//! `src/generated/client.rs` by ethos-zero; the flake's check refuses a
//! committed generation that differs from what the source generates.

#![forbid(unsafe_code)]

mod client;
mod configuration;
#[rustfmt::skip]
#[path = "generated/client.rs"]
mod generated;
#[cfg(feature = "datom")]
mod invocation;

pub use client::{ClientError, TextClient, TextRequesting, TranslatorPlacing};
#[cfg(feature = "datom")]
pub use configuration::ConfigurationReading;
pub use configuration::{ConfigurationFile, ConfigurationLocating};
pub use generated::{
    ClientConfiguration, ConfigurationFault, ConfigurationPath, ConfigurationRefusal, Reason,
    TextLane,
};
#[cfg(feature = "datom")]
pub use invocation::{
    AnswerPrinting, InvocationRefusal, Invoking, RefusalReporting, TextCommand, TextInvocation,
};
