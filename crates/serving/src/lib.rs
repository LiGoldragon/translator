//! The serving every translator shares.
//!
//! A translator is the process in front of a Nexus that speaks text for it.
//! It serves two sockets in its place's runtime directory, named after the
//! Nexus: on the ordinary one a CLI's text and the Nexus's emissions, on the
//! meta one the meta CLI's text. Each connection's caller is the kernel's
//! account of its peer, and a caller of another login is refused. Ordinary
//! text is read, within the translator's own text budget, into the Nexus's
//! public query, carried to the Nexus with its Caller, and every answer of
//! its exchange written back as text, until the Nexus ends the exchange; the
//! translator then writes `Ended`, or `Failed` with what ended it. Every
//! exchange ends, a one-answer exchange after its answer as a subscription
//! after its last frame, so the translator needs no knowledge of which
//! answer is a frame. Meta text is read within the bootstrap's budget and
//! settled: the translator answers its own settings, announcing each change
//! to every open subscription, and carries the rest to the Nexus's meta
//! socket, merging its own settings into each answer the Nexus reads out. An
//! emission, accepted only from the paired Nexus, is answered by the
//! translator in the world. The pair launcher prepares the place, starts the
//! Nexus and its translator and hands each a bootstrap frame naming the
//! other: the Nexus its [`NexusBootstrap`], with a fresh [`InstanceId`], and
//! the translator its [`TranslatorBootstrap`], with the text limits its
//! budget starts at, which the launcher takes from its configuration. It
//! then waits on events, tokio's signal streams for SIGTERM and SIGINT and a
//! pidfd for each child, and ends both children when it is told to end or
//! when either ends.
//!
//! The lanes, the traits and the pair launcher read and write text, so they
//! are compiled in only under the `text` feature, which a translator turns on.
//!
//! What a translator writes is its own bodies: [`Translator`] names its Nexus
//! and its readiness, [`PublicTranslating`] its public contract and the call
//! carrying it, [`MetaTranslating`] how it splits a meta query between itself
//! and the Nexus and merges its own settings into the Nexus's answers, and
//! [`EmissionAnswering`] what it does for an emission. The kinds declared in
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
mod instance;
mod pair;
mod server;
mod subscriptions;
mod translating;

pub use bootstrap::BootstrapReading;
pub use budget::{Budgeting, TextBudgeting, ValidTextBudget};
pub use calling::{EmissionRestoring, NexusCalling};
pub use connection::{Connection, ConnectionServing};
pub use error::TranslatorError;
pub use generated::{
    DepthLimit, Endpoint, NexusExecutable, NexusFailure, NodeLimit, Pairing, Refusal, TextLimits,
    TranslatorBootstrap, TranslatorExecutable,
};
pub use instance::FreshInstance;
pub use nexus_serving::{InstanceId, NexusBootstrap, PairedPeer, PeerMatching};
pub use pair::{PairReading, PairRunning};
pub use server::TranslatorServing;
pub use subscriptions::MetaSubscriptions;
pub use translating::{
    EmissionAnswering, MetaTranslating, PublicTranslating, Settling, Translator,
};
