//! The architecture-one translator runtime; the translator's data is only its
//! Place, while OS sockets belong to the running service.
#![forbid(unsafe_code)]

use nexus_client::Named;
mod component;
mod configuration;
mod serving;
pub use component::{ConfigurationFollowing, Emittable, Translating};
use configuration::{BudgetHandle, ConfigurationWatch};
pub use configuration::{Budgeting, ConfigurationObservation, ObservingConfiguration};
pub use serving::{
    BindingTranslator, BoundTranslator, ConfigurationWatching, RuntimeError, Serving,
};

use std::path::PathBuf;
pub use translator::{Caller, Place, PlaceName, ProcessId, Runtime, State, Translator, UserId};

// Trait exception: Too trivial; this returns the single shared typed constant.
/// The shared Signal frame capacity, used by client, Nexus and translator.
pub fn frame_capacity() -> signal::FrameCapacity {
    signal::FrameCapacity::from(translator::FRAME_CAPACITY_BYTES.0 as usize)
}

/// Derived OS paths, never a second source of routing configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SocketPaths {
    pub ordinary_nexus: PathBuf,
    pub meta_nexus: PathBuf,
    pub ordinary_text: PathBuf,
    pub meta_text: PathBuf,
    pub emission: PathBuf,
}

pub trait SocketLocating {
    fn socket_paths<N: Named>(&self) -> SocketPaths;
}

impl SocketLocating for Translator {
    fn socket_paths<N: Named>(&self) -> SocketPaths {
        let runtime = PathBuf::from(&self.place.runtime.0);
        let name = N::name();
        SocketPaths {
            ordinary_nexus: runtime.join(format!("{name}.sock")),
            meta_nexus: runtime.join(format!("{name}-meta.sock")),
            ordinary_text: runtime.join(format!("{name}-translator.sock")),
            meta_text: runtime.join(format!("{name}-meta-translator.sock")),
            emission: runtime.join(format!("{name}-emission.sock")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Example;
    impl Named for Example {
        fn name() -> String {
            "example".into()
        }
    }
    #[test]
    fn all_sockets_derive_from_component_and_place_runtime_only() {
        let translator = Translator {
            place: Place {
                place_name: PlaceName("next".into()),
                runtime: Runtime("/runtime/next".into()),
                state: State("/state/next".into()),
            },
        };
        let sockets = translator.socket_paths::<Example>();
        assert_eq!(
            sockets.ordinary_nexus,
            PathBuf::from("/runtime/next/example.sock")
        );
        assert_eq!(
            sockets.meta_nexus,
            PathBuf::from("/runtime/next/example-meta.sock")
        );
        assert_eq!(
            sockets.ordinary_text,
            PathBuf::from("/runtime/next/example-translator.sock")
        );
        assert_eq!(
            sockets.meta_text,
            PathBuf::from("/runtime/next/example-meta-translator.sock")
        );
        assert_eq!(
            sockets.emission,
            PathBuf::from("/runtime/next/example-emission.sock")
        );
    }
}
