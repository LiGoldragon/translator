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
mod generated_placement;

pub mod placement {
    pub use crate::generated_placement::*;
    use std::path::{Component, Path, PathBuf};

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub enum PlaceResolutionError {
        EmptyName,
        UnsafeName(String),
        MissingEnvironment(&'static str),
        EmptyEnvironment(&'static str),
        NonUnicodeEnvironment(&'static str),
        RelativeEnvironment(&'static str),
    }

    impl std::fmt::Display for PlaceResolutionError {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(formatter, "place resolution failed: {self:?}")
        }
    }

    impl std::error::Error for PlaceResolutionError {}

    pub fn validate_metaflow_name(name: &str) -> Result<(), PlaceResolutionError> {
        if name.is_empty() {
            return Err(PlaceResolutionError::EmptyName);
        }
        let mut components = Path::new(name).components();
        let safe = matches!(components.next(), Some(Component::Normal(component)) if component == name)
            && components.next().is_none()
            && !name.contains('\0');
        if safe {
            Ok(())
        } else {
            Err(PlaceResolutionError::UnsafeName(name.to_owned()))
        }
    }

    fn environment_path(name: &'static str) -> Result<PathBuf, PlaceResolutionError> {
        let value = std::env::var_os(name)
            .ok_or(PlaceResolutionError::MissingEnvironment(name))?
            .into_string()
            .map_err(|_| PlaceResolutionError::NonUnicodeEnvironment(name))?;
        if value.is_empty() {
            return Err(PlaceResolutionError::EmptyEnvironment(name));
        }
        let path = PathBuf::from(value);
        if path.is_absolute() {
            Ok(path)
        } else {
            Err(PlaceResolutionError::RelativeEnvironment(name))
        }
    }

    fn runtime(name: &str) -> Result<Runtime, PlaceResolutionError> {
        let path = environment_path("XDG_RUNTIME_DIR")?
            .join("metaflow")
            .join(name);
        Ok(Runtime(
            path.into_os_string()
                .into_string()
                .expect("validated Unicode root"),
        ))
    }

    fn home_state_root() -> Result<PathBuf, PlaceResolutionError> {
        Ok(environment_path("HOME")?.join(".local").join("state"))
    }

    fn state_root() -> Result<PathBuf, PlaceResolutionError> {
        let Some(value) = std::env::var_os("XDG_STATE_HOME") else {
            return home_state_root();
        };
        if value.is_empty() {
            return home_state_root();
        }
        let value = value
            .into_string()
            .map_err(|_| PlaceResolutionError::NonUnicodeEnvironment("XDG_STATE_HOME"))?;
        let path = PathBuf::from(value);
        if path.is_absolute() {
            Ok(path)
        } else {
            home_state_root()
        }
    }

    fn state(name: &str) -> Result<State, PlaceResolutionError> {
        let path = state_root()?.join("metaflow").join(name);
        Ok(State(
            path.into_os_string()
                .into_string()
                .expect("validated Unicode root"),
        ))
    }

    pub fn resolve_runtime(name: &str) -> Result<Runtime, PlaceResolutionError> {
        validate_metaflow_name(name)?;
        runtime(name)
    }

    pub fn resolve_place(name: &str) -> Result<Place, PlaceResolutionError> {
        validate_metaflow_name(name)?;
        Ok(Place {
            metaflow_name: MetaflowName(name.to_owned()),
            runtime: runtime(name)?,
            state: state(name)?,
        })
    }

    pub fn runtime_path(place: &Place) -> PathBuf {
        PathBuf::from(&place.runtime.0)
    }

    pub fn state_path(place: &Place) -> PathBuf {
        PathBuf::from(&place.state.0)
    }
}

#[path = "../generated/identity.rs"]
pub mod identity;

pub use identity::{
    Caller, GroupId, PeerBinding, PeerId, PeerIdentity, Principal, ProcessId, UserId,
};

pub use placement::{MetaflowName, Place, Runtime, State};

#[cfg(feature = "unix")]
pub mod unix;
