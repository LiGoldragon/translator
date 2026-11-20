//! Component capabilities. The shared runtime never inspects datom text to
//! decide whether an output is a subscription.
use datom_codec::{Composing, Datomizable};
use nexus::WireValue;
use nexus_client::Named;
use std::future::Future;
use translator::{Call, Caller, DatomReadingBudget, OutputMode};

/// A generated component input can be wrapped in the one generic Call.
/// The blanket implementation contains the archive bounds; components add no codec.
pub trait Callable: WireValue + Composing {
    fn framed_call(
        self,
        caller: Caller,
        capacity: signal::FrameCapacity,
    ) -> Result<Vec<u8>, nexus::TransmissionError>;
}
impl<T: WireValue + Composing> Callable for T
where
    Call<T>: WireValue,
{
    fn framed_call(
        self,
        caller: Caller,
        capacity: signal::FrameCapacity,
    ) -> Result<Vec<u8>, nexus::TransmissionError> {
        Call {
            caller,
            input: self,
        }
        .framed_value(capacity)
    }
}

pub trait Emittable: WireValue {
    type Outcome: WireValue;
    fn emit(self) -> impl Future<Output = Self::Outcome> + Send;
}

pub trait Translating: Named + Send + Sync + 'static {
    type Input: Callable;
    type Output: WireValue + Datomizable;
    type MetaInput: Callable;
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
