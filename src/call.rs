//! The shared generic Call constructor. Like Signal's Frame and Stream, its
//! payload is a Rust type parameter. Ethos Zero cannot yet declare generic
//! data constructors; Fable qy50 permits this single handwritten projection.
use crate::{Caller, FrameCapacityBytes};

#[derive(rkyv::Archive, rkyv::Serialize, rkyv::Deserialize, Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(
    feature = "datom",
    derive(datom_codec::Datomizable, datom_codec::Composing)
)]
pub struct Call<Input> {
    pub caller: Caller,
    pub input: Input,
}

/// The frame capacity has one value: Signal's declared shared default.
pub const FRAME_CAPACITY_BYTES: FrameCapacityBytes =
    FrameCapacityBytes(signal::MAXIMUM_SIGNAL_BYTES as i64);
