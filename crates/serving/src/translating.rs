//! The bodies a translator writes.

use datom_codec::{Composing, Datomizable};
use nexus_exchange::Named;
use shared_translator::Caller;
use signal::Signalizable;

use crate::budget::TextBudgeting;
use crate::calling::{EmissionRestoring, NexusCalling};
use crate::generated::NexusFailure;

/// A translator: the process that speaks text for the Nexus `Component`,
/// ready once its own settings are.
pub trait Translator: Send + Sync + 'static {
    /// The Nexus it speaks for, whose name gives the sockets.
    type Component: Named;
    /// The translator's own settings, every one set.
    type Ready: TextBudgeting;

    /// The settings as they stand, or `None` while one is missing.
    fn ready(&self) -> Option<Self::Ready>;
}

/// The ordinary text lane: the Nexus's public contract, and the call that
/// carries a query of it to the Nexus with its Caller. Every answer of the
/// call's exchange is written back as it comes, until the Nexus ends it.
pub trait PublicTranslating: Translator {
    /// The public contract's query, read from a CLI's text.
    type PublicInput: Composing;
    /// The public contract's answer, written back as text.
    type PublicOutput: Datomizable;
    /// The call the Nexus's public socket answers.
    type PublicCall: NexusCalling<Self::PublicResponse>;
    /// What the Nexus's public socket answers a call with.
    type PublicResponse;

    /// The call carrying the query and the Caller it came from.
    fn public_call(caller: Caller, input: Self::PublicInput) -> Self::PublicCall;

    /// The answer a response carries, or the failure the Nexus answered with.
    fn public_answered(response: Self::PublicResponse) -> Result<Self::PublicOutput, NexusFailure>;
}

/// How a meta query is settled: answered by the translator, forwarded as the
/// part the Nexus answers, or forwarded as a subscription of the Nexus's,
/// which the translator's own changes are carried into for as long as the
/// Nexus keeps it open.
pub enum Settling<Output, Forwarded> {
    Answered(Output),
    Forwarded(Forwarded),
    Subscribed(Forwarded),
}

/// The meta text lane: the meta contract, split between the translator's own
/// settings and the Nexus's.
pub trait MetaTranslating: Translator {
    /// The meta contract's query, read from the meta CLI's text.
    type MetaInput: Composing;
    /// The meta contract's answer, written back as text.
    type MetaOutput: Datomizable;
    /// The part of a meta query the Nexus answers.
    type NexusMetaInput;
    /// The call the Nexus's meta socket answers.
    type MetaCall: NexusCalling<Self::MetaResponse>;
    /// What the Nexus's meta socket answers a call with.
    type MetaResponse;

    /// Answers a query of the translator's own, or names the Nexus's part.
    fn settle(&self, input: Self::MetaInput) -> Settling<Self::MetaOutput, Self::NexusMetaInput>;

    /// The call carrying the Nexus's part and the Caller it came from.
    fn meta_call(caller: Caller, input: Self::NexusMetaInput) -> Self::MetaCall;

    /// The meta answer a response of the Nexus's carries, the translator's
    /// own settings as they stand merged into it where it reads out the
    /// configuration, or the failure the Nexus answered with. Readiness is
    /// the pair's: a readout names the translator's settings still awaited
    /// first, then the Nexus's, and is ready only when both are.
    fn meta_answered(&self, response: Self::MetaResponse)
    -> Result<Self::MetaOutput, NexusFailure>;

    /// The answer every open subscription is sent when the translator has
    /// answered `answered` itself: the change of its own setting that answer
    /// applied, as a frame of the configuration stream, or `None` when it
    /// changed nothing a subscription follows.
    fn announced(&self, answered: &Self::MetaOutput) -> Option<Self::MetaOutput>;
}

/// The emission lane: what the translator does in the world when its Nexus
/// asks.
pub trait EmissionAnswering: Translator {
    /// The emission, as the Nexus sends it.
    type Emission: EmissionRestoring;
    /// The outcome of what the translator observed, sent back.
    type Outcome: Signalizable;

    /// Does what the emission asks, under the settings as they stand.
    fn emitted(&self, ready: &Self::Ready, emission: Self::Emission) -> Self::Outcome;
}
