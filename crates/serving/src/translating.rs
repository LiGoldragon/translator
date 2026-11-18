//! The bodies a translator writes.

use datom_codec::{Composing, Datomizable};
use nexus_exchange::Named;
use shared_translator::Caller;
use signal::Signalizable;

use crate::budget::TextBudgeting;
use crate::calling::{EmissionRestoring, NexusCalling};
use crate::generated::{Answers, NexusFailure};

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
/// carries a query of it to the Nexus with its Caller.
pub trait PublicTranslating: Translator {
    /// The public contract's query, read from a CLI's text.
    type PublicInput: Composing;
    /// The public contract's answer, written back as text.
    type PublicOutput: Datomizable;
    /// The call the Nexus's public socket answers.
    type PublicCall: NexusCalling<Self::PublicResponse>;
    /// What the Nexus's public socket answers a call with.
    type PublicResponse;

    /// Whether the query is answered once, or followed: every answer of its
    /// exchange written back as it comes, until the Nexus ends it.
    fn answers(input: &Self::PublicInput) -> Answers;

    /// The call carrying the query and the Caller it came from.
    fn public_call(caller: Caller, input: Self::PublicInput) -> Self::PublicCall;

    /// The answer a response carries, or the failure the Nexus answered with.
    fn public_answered(response: Self::PublicResponse) -> Result<Self::PublicOutput, NexusFailure>;
}

/// How a meta query is settled: answered by the translator, or forwarded as
/// the part the Nexus answers.
pub enum Settling<Output, Forwarded> {
    Answered(Output),
    Forwarded(Forwarded),
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

    /// The meta answer a response carries, or the failure the Nexus answered
    /// with.
    fn meta_answered(response: Self::MetaResponse) -> Result<Self::MetaOutput, NexusFailure>;
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
