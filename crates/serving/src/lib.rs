//! The serving every translator shares.
//!
//! A translator is the process in front of a Nexus that speaks text for it.
//! It serves two sockets in its place's runtime directory, named after the
//! Nexus: on the ordinary one a CLI's text and the Nexus's emissions, on the
//! meta one the meta CLI's text. Each connection's caller is the kernel's
//! account of its peer, and a caller of another login is refused. Ordinary
//! text is read, within the translator's own text budget, into the Nexus's
//! public query, carried to the Nexus with its Caller, and the answer, or
//! every answer of a followed query, written back as text. Meta text is read
//! within the bootstrap's budget and settled: the translator answers its own
//! settings and carries the rest to the Nexus's meta socket. An emission,
//! accepted only from the paired Nexus, is answered by the translator in the
//! world. The pair launcher prepares the place, starts the Nexus and its
//! translator and hands each a bootstrap frame naming the other.
//!
//! The lanes, the traits and the pair launcher read and write text, so they
//! are compiled in only under the `text` feature, which a translator turns on.
//!
//! What a translator writes is its own bodies: [`Translator`] names its Nexus
//! and its readiness, [`PublicTranslating`] its public contract and the call
//! carrying it, [`MetaTranslating`] how it splits a meta query between itself
//! and the Nexus, [`EmissionAnswering`] what it does for an emission, and
//! [`PairLaunching`] the bootstrap its Nexus reads. The kinds declared in
//! `ethos/serving.ethos` are generated into `src/generated/serving.rs` by
//! ethos-zero; the flake's check refuses a committed generation that differs
//! from what the source generates.

#![forbid(unsafe_code)]

mod bootstrap;
mod budget;
mod calling;
mod connection;
mod error;
#[rustfmt::skip]
#[path = "generated/serving.rs"]
mod generated;
mod pair;
mod server;
mod translating;

pub use bootstrap::BootstrapReading;
pub use budget::{Budgeting, TextBudgeting, ValidTextBudget};
pub use calling::{EmissionRestoring, NexusCalling};
pub use connection::{Connection, ConnectionServing};
pub use error::TranslatorError;
pub use generated::{
    Answers, DepthLimit, Endpoint, NexusExecutable, NexusFailure, NodeLimit, Pairing, Refusal,
    TextLimits, TranslatorBootstrap, TranslatorExecutable,
};
pub use nexus_serving::{PairedPeer, PeerMatching};
pub use pair::{PairLaunching, PairReading, PairRunning};
pub use server::TranslatorServing;
pub use translating::{
    EmissionAnswering, MetaTranslating, PublicTranslating, Settling, Translator,
};
