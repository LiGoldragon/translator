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

#[path = "../generated/placement.rs"]
pub mod placement;

#[path = "../generated/identity.rs"]
pub mod identity;

pub use identity::{
    GroupId, PeerBinding, PeerId, PeerIdentity, Principal, ProcessId, UserId,
};

#[cfg(feature = "text")]
pub mod registry {
    use super::placement::{Place, Registry};
    use datom_codec::{Actualizing, Budget, Potential};

    #[derive(Debug, Eq, PartialEq)]
    pub enum RegistryError { Invalid(String), Missing(String) }
    pub fn decode(text: &str, budget: &mut Budget) -> Result<Registry, RegistryError> {
        let mut potential = Potential::<Registry>::from(text);
        let registry = potential.actualize(budget).map_err(|e| RegistryError::Invalid(format!("{e:?}")))?;
        validate(&registry)?;
        Ok(registry)
    }
    pub fn validate(registry: &Registry) -> Result<(), RegistryError> {
        for (i, left) in registry.0.iter().enumerate() {
            if registry.0.iter().skip(i + 1).any(|right| right.metaflow_name.0 == left.metaflow_name.0) { return Err(RegistryError::Invalid(format!("duplicate metaflow name: {}", left.metaflow_name.0))); }
            if registry.0.iter().skip(i + 1).any(|right| right.directory.0 == left.directory.0) { return Err(RegistryError::Invalid(format!("duplicate directory: {}", left.directory.0))); }
        }
        Ok(())
    }
    pub fn select<'a>(registry: &'a Registry, name: &str) -> Result<&'a Place, RegistryError> {
        registry.0.iter().find(|place| place.metaflow_name.0 == name).ok_or_else(|| RegistryError::Missing(name.to_owned()))
    }
}
