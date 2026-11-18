//! The client end of a translator's text lanes.
//!
//! A CLI takes a place's name and one datom. It sends the datom, as text and
//! nothing more, on one of the two text lanes of the translator that speaks
//! for a Nexus, at the socket the translator serves that lane on in the
//! place's runtime directory, and reads every answer on the same connection.
//! The CLI holds no contract type: reading the text is the translator's.
//!
//! The kinds declared in `ethos/client.ethos` are generated into
//! `src/generated/client.rs` by ethos-zero; the flake's check refuses a
//! committed generation that differs from what the source generates.

#![forbid(unsafe_code)]

mod client;
#[rustfmt::skip]
#[path = "generated/client.rs"]
mod generated;
mod invocation;

pub use client::{ClientError, TextClient, TextRequesting, TranslatorPlacing};
pub use generated::TextLane;
pub use invocation::{AnswerPrinting, Invoking, TextInvocation};
