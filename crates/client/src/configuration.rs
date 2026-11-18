//! What a CLI knows besides its datom: the place its configuration names.
//!
//! A CLI of the Nexus `Component` reads one datom,
//! `<configuration home>/nexus/<nexus-name>.datom`, the configuration home
//! being `$XDG_CONFIG_HOME`, or `~/.config` when the variable is unset or
//! empty. It holds a [`ClientConfiguration`], the place first among its
//! values: `{ main }`. A configuration that cannot be found, read or
//! understood is a [`ConfigurationRefusal`] naming the file, and the CLI
//! reaches no Nexus.

use std::ffi::OsStr;
use std::path::PathBuf;

use nexus_exchange::Named;

use crate::generated::{ConfigurationRefusal, Reason};

/// The file a CLI's configuration is read from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigurationFile {
    pub path: PathBuf,
}

pub trait ConfigurationLocating: Sized {
    /// The directory of the configuration home every Nexus's CLI file is in.
    const NEXUS_DIRECTORY: &'static str = "nexus";

    /// The configuration of the CLIs of the Nexus `Component`:
    /// `<configuration home>/nexus/<nexus-name>.datom`, the configuration
    /// home being `configuration_home` (`XDG_CONFIG_HOME`), or, unset or
    /// empty, `<home>/.config`; either an absolute path.
    fn under<Component: Named>(
        configuration_home: Option<&OsStr>,
        home: Option<&OsStr>,
    ) -> Result<Self, ConfigurationRefusal>;

    /// The configuration this process finds, from its `XDG_CONFIG_HOME` and
    /// its `HOME`.
    fn of_process<Component: Named>() -> Result<Self, ConfigurationRefusal> {
        Self::under::<Component>(
            std::env::var_os("XDG_CONFIG_HOME").as_deref(),
            std::env::var_os("HOME").as_deref(),
        )
    }
}

impl ConfigurationLocating for ConfigurationFile {
    fn under<Component: Named>(
        configuration_home: Option<&OsStr>,
        home: Option<&OsStr>,
    ) -> Result<Self, ConfigurationRefusal> {
        let unlocated = |reason: String| ConfigurationRefusal::Unlocated(Reason(reason));
        // An empty XDG_CONFIG_HOME is an unset one, as the standard has it.
        let directory = match (configuration_home.filter(|home| !home.is_empty()), home) {
            (Some(directory), _) => PathBuf::from(directory),
            (None, Some(home)) => PathBuf::from(home).join(".config"),
            (None, None) => {
                return Err(unlocated("neither XDG_CONFIG_HOME nor HOME is set".into()));
            }
        };
        if !directory.is_absolute() {
            return Err(unlocated(format!(
                "the configuration home {} is not absolute",
                directory.display()
            )));
        }
        Ok(Self {
            path: directory
                .join(Self::NEXUS_DIRECTORY)
                .join(format!("{}.datom", Component::name().0)),
        })
    }
}

#[cfg(feature = "datom")]
pub trait ConfigurationReading {
    /// The configuration the file holds: a file that is missing, cannot be
    /// read, or holds no `ClientConfiguration` naming a valid place is
    /// refused, naming the file.
    fn read(&self) -> Result<crate::generated::ClientConfiguration, ConfigurationRefusal>;
}

#[cfg(feature = "datom")]
impl ConfigurationReading for ConfigurationFile {
    fn read(&self) -> Result<crate::generated::ClientConfiguration, ConfigurationRefusal> {
        use crate::generated::{ClientConfiguration, ConfigurationFault, ConfigurationPath};
        use datom_codec::{Actualizing, Budget, Potential};
        let named = || ConfigurationPath(self.path.to_string_lossy().into_owned());
        let fault = |reason: String| ConfigurationFault {
            configuration_path: named(),
            reason: Reason(reason),
        };
        let source = match std::fs::read_to_string(&self.path) {
            Ok(source) => source,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(ConfigurationRefusal::Missing(named()));
            }
            Err(error) => {
                return Err(ConfigurationRefusal::Unreadable(fault(error.to_string())));
            }
        };
        // The configuration is read within Signal's own bounds, which any
        // text a frame can carry fits.
        let mut budget = Budget {
            remaining: i64::try_from(signal::MAXIMUM_SIGNAL_BYTES)
                .expect("Signal's bound fits i64"),
            reader: protos::ReaderBudget::try_from(signal::MAXIMUM_SIGNAL_BYTES)
                .expect("Signal's bound fits the reader budget"),
            depth: 0,
            maximum_depth: i64::try_from(signal::MAXIMUM_SIGNAL_DEPTH)
                .expect("Signal's depth fits i64"),
        };
        let configuration = Potential::<ClientConfiguration>::from(source.trim())
            .actualize(&mut budget)
            .map_err(|error| ConfigurationRefusal::Invalid(fault(format!("{error:?}"))))?;
        shared_translator::placement::validate_place_name(&configuration.place_name.0)
            .map_err(|error| ConfigurationRefusal::Invalid(fault(error.to_string())))?;
        Ok(configuration)
    }
}
