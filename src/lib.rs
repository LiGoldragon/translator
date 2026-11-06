//! Shared v1 translator boundary: binary Signal transport is always available;
//! textual Datom conversion is enabled by the `text` feature.

pub use signal::{
    ByteViewable, Framable, FrameBody, FrameCapacity, FrameError, FrameReading, FrameWriting,
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
    use std::io;
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

    #[derive(Debug)]
    pub enum PlacePreparationError {
        StateConflict { legacy: PathBuf, current: PathBuf },
        Io { path: PathBuf, source: io::Error },
    }

    impl std::fmt::Display for PlacePreparationError {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                Self::StateConflict { legacy, current } => write!(
                    formatter,
                    "legacy state {} and current state {} both exist",
                    legacy.display(),
                    current.display()
                ),
                Self::Io { path, source } => {
                    write!(
                        formatter,
                        "place preparation at {} failed: {source}",
                        path.display()
                    )
                }
            }
        }
    }

    impl std::error::Error for PlacePreparationError {
        fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
            match self {
                Self::StateConflict { .. } => None,
                Self::Io { source, .. } => Some(source),
            }
        }
    }

    pub fn validate_place_name(name: &str) -> Result<(), PlaceResolutionError> {
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
            .join("nexus")
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
        let path = state_root()?.join("nexus").join(name);
        Ok(State(
            path.into_os_string()
                .into_string()
                .expect("validated Unicode root"),
        ))
    }

    pub fn resolve_runtime(name: &str) -> Result<Runtime, PlaceResolutionError> {
        validate_place_name(name)?;
        runtime(name)
    }

    pub fn resolve_place(name: &str) -> Result<Place, PlaceResolutionError> {
        validate_place_name(name)?;
        Ok(Place {
            place_name: PlaceName(name.to_owned()),
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

    #[cfg(not(unix))]
    fn legacy_state_path(place: &Place) -> Option<PathBuf> {
        let current = state_path(place);
        let nexus = current.parent()?;
        if current.file_name()? != place.place_name.0.as_str() || nexus.file_name()? != "nexus" {
            return None;
        }
        Some(nexus.parent()?.join("metaflow").join(&place.place_name.0))
    }

    fn create_directory(path: &Path) -> Result<(), PlacePreparationError> {
        std::fs::create_dir_all(path).map_err(|source| PlacePreparationError::Io {
            path: path.to_owned(),
            source,
        })
    }

    #[cfg(not(unix))]
    fn path_exists(path: &Path) -> Result<bool, PlacePreparationError> {
        path.try_exists()
            .map_err(|source| PlacePreparationError::Io {
                path: path.to_owned(),
                source,
            })
    }

    /// The root of a standard place path `<root>/nexus/<name>`: an absolute path
    /// of normal components whose last component is the place name and whose
    /// parent is named `nexus`.
    #[cfg(unix)]
    fn standard_root(path: &Path, name: &str) -> Option<PathBuf> {
        if !path.is_absolute()
            || !path
                .components()
                .all(|component| matches!(component, Component::RootDir | Component::Normal(_)))
        {
            return None;
        }
        let nexus = path.parent()?;
        if path.file_name()? != name || nexus.file_name()? != "nexus" {
            return None;
        }
        Some(nexus.parent()?.to_owned())
    }

    /// Start-only filesystem preparation for one Nexus place.
    ///
    /// A standard place (state at `<root>/nexus/<name>`) is migration capable:
    /// on Unix its preparation takes an exclusive per-destination lock and works
    /// descriptor-relative from the verified root, so cooperating preparers of
    /// one destination serialize. An explicit place (any other state path) only
    /// creates directories, never looks for or moves legacy state, and refuses
    /// an actual identity conflict with the standard destination of the same
    /// name. Neither plan owns the place for its lifetime or guards against
    /// other writers, and an alias the process cannot observe is outside the
    /// guarantee.
    pub trait Preparable {
        /// Move legacy state when it is the sole store, then create the new
        /// runtime and state directories. Resolution itself remains pure.
        fn prepare_for_start(&self) -> Result<(), PlacePreparationError>;
    }

    impl Preparable for Place {
        #[cfg(unix)]
        fn prepare_for_start(&self) -> Result<(), PlacePreparationError> {
            let current = state_path(self);
            match standard_root(&current, &self.place_name.0) {
                Some(root) => prepare_standard(self, &root),
                None => prepare_explicit(self, &current),
            }
        }

        /// Targets without the anchored primitives keep the previous
        /// preparation, dispatch and routine unchanged.
        #[cfg(not(unix))]
        fn prepare_for_start(&self) -> Result<(), PlacePreparationError> {
            prepare_previous(self)
        }
    }

    #[cfg(unix)]
    fn prepare_standard(place: &Place, root: &Path) -> Result<(), PlacePreparationError> {
        let runtime = runtime_path(place);
        let runtime_root = standard_root(&runtime, &place.place_name.0);
        crate::anchored::prepare_standard(
            root,
            runtime_root.as_deref(),
            &place.place_name.0,
            &runtime,
        )
    }

    #[cfg(not(unix))]
    fn prepare_previous(place: &Place) -> Result<(), PlacePreparationError> {
        let current = state_path(place);
        if let Some(legacy) = legacy_state_path(place) {
            let legacy_exists = path_exists(&legacy)?;
            let current_exists = path_exists(&current)?;
            if legacy_exists && current_exists {
                return Err(PlacePreparationError::StateConflict { legacy, current });
            }
            if legacy_exists {
                if let Some(parent) = current.parent() {
                    create_directory(parent)?;
                }
                std::fs::rename(&legacy, &current).map_err(|source| PlacePreparationError::Io {
                    path: legacy,
                    source,
                })?;
            }
        }
        create_directory(&runtime_path(place))?;
        create_directory(&current)
    }

    #[cfg(unix)]
    fn prepare_explicit(place: &Place, current: &Path) -> Result<(), PlacePreparationError> {
        refuse_standard_identity(&place.place_name.0, current)?;
        create_directory(&runtime_path(place))?;
        create_directory(current)
    }

    /// Refuse when the explicit state path, or the first existing directory on
    /// the way to it, is the very directory the standard plan would use for the
    /// same place name or the namespace directory that holds it.
    #[cfg(unix)]
    fn refuse_standard_identity(name: &str, current: &Path) -> Result<(), PlacePreparationError> {
        let Ok(root) = state_root() else {
            return Ok(());
        };
        let protected: Vec<(u64, u64)> = [
            root.join("nexus"),
            root.join("nexus").join(name),
            root.join("metaflow"),
            root.join("metaflow").join(name),
        ]
        .iter()
        .filter_map(|path| crate::anchored::identity(path))
        .collect();
        for ancestor in current.ancestors() {
            let Some(found) = crate::anchored::identity(ancestor) else {
                continue;
            };
            if protected.contains(&found) {
                return Err(PlacePreparationError::Io {
                    path: current.to_owned(),
                    source: io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "explicit state path reaches the standard state of this place",
                    ),
                });
            }
            break;
        }
        Ok(())
    }
}

#[cfg(unix)]
mod anchored;

#[path = "../generated/identity.rs"]
pub mod identity;

pub use identity::{
    Caller, GroupId, PeerBinding, PeerId, PeerIdentity, Principal, ProcessId, UserId,
};

pub use placement::{Place, PlaceName, PlacePreparationError, Preparable, Runtime, State};

#[cfg(feature = "unix")]
pub mod unix;
