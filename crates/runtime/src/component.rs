//! Component capabilities. The shared runtime never inspects datom text to
//! decide whether an output is a subscription.
use datom_codec::{Composing, Datomizable};
use nexus::WireValue;
use nexus_client::Named;
use std::future::Future;
use translator::{DatomReadingBudget, OutputMode};

pub trait Emittable: WireValue {
    type Outcome: WireValue;
    fn emit(self) -> impl Future<Output = Self::Outcome> + Send;
}

pub trait Translating: Named + Send + Sync + 'static {
    type Input: WireValue + Composing;
    type Output: WireValue + Datomizable;
    type MetaInput: WireValue + Composing;
    type MetaOutput: WireValue + Datomizable;
    type Emission: Emittable<Outcome = Self::Outcome>;
    type Outcome: WireValue;
    fn describe() -> String;
    fn describe_meta() -> String;
    fn output_mode(output: &Self::Output) -> OutputMode;
    fn meta_output_mode(output: &Self::MetaOutput) -> OutputMode;
}

/// A component's own typed meta subscription supplies the one observed budget.
/// Snapshot must be first. A change may preserve the budget if it changes
/// another configuration field; None means the observed stream is invalid.
pub trait ConfigurationFollowing: Translating {
    fn subscribe_configuration() -> Self::MetaInput;
    fn snapshot(output: &Self::MetaOutput) -> Option<DatomReadingBudget>;
    fn after(current: &DatomReadingBudget, output: &Self::MetaOutput)
    -> Option<DatomReadingBudget>;
}
