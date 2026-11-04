//! Shared v1 translator boundary: binary Signal transport is always available;
//! textual Datom conversion is enabled by the `text` feature.

pub use signal::{
    ByteViewable, FrameBody, FrameCapacity, FrameError, FrameReading, FrameWriting, Framable,
    Restorable, Signal, Signalizable,
};

#[cfg(feature = "text")]
pub use text::{decode, encode};

#[cfg(feature = "text")]
mod text {
    use datom_codec::{Actualizing, Budget, Composing, Datomizable, Potential};
    use protos::{Protosizable, Textualizable};

    /// Decode schema-directed Datom text into a typed value.
    pub fn decode<T: Composing>(text: &str, budget: &mut Budget) -> Result<T, datom_codec::Error> {
        let mut potential = Potential::<T>::from(text);
        potential.actualize(budget)
    }

    /// Encode a typed value as canonical Datom text.
    pub fn encode<T: Datomizable>(value: &T) -> String {
        value.datomize(Vec::new()).protosize().textualize()
    }
}
