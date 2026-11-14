//! A Nexus's place file: its Place, framed signal, at
//! `$XDG_CONFIG_HOME/nexus/<place-name>/<nexus-name>.signal`
//! (`$HOME/.config` when the variable is unset or empty), read before any
//! socket opens. The type and the file are the same for every Nexus, so one
//! tool, `nexus-configure`, compiles a place written in datom into it.

use crate::placement::{
    FileFault, FilePath, NexusName, Place, PlaceFileRefusal, PlaceName, Reason,
};
use crate::{Framable, FrameCapacity, FrameReading, Restorable, Signal, Signalizable};
use std::{
    ffi::OsStr,
    io::{ErrorKind, Read},
    path::{Path, PathBuf},
};

/// The directory every Nexus's configuration lies under:
/// `$XDG_CONFIG_HOME`, else `$HOME/.config`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigurationHome {
    pub directory: PathBuf,
}

/// One place file, where it lies.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlaceFile {
    pub path: PathBuf,
}

pub trait LocatesConfigurationHome: Sized {
    /// The configuration home is the value of `XDG_CONFIG_HOME`, else, unset
    /// or empty, `<HOME>/.config`; either must be an absolute path.
    fn under(
        configuration_home: Option<&OsStr>,
        home: Option<&OsStr>,
    ) -> Result<Self, PlaceFileRefusal>;

    /// `under` this process's `XDG_CONFIG_HOME` and `HOME`.
    fn of_process() -> Result<Self, PlaceFileRefusal> {
        Self::under(
            std::env::var_os("XDG_CONFIG_HOME").as_deref(),
            std::env::var_os("HOME").as_deref(),
        )
    }
}

impl LocatesConfigurationHome for ConfigurationHome {
    fn under(
        configuration_home: Option<&OsStr>,
        home: Option<&OsStr>,
    ) -> Result<Self, PlaceFileRefusal> {
        let configuration_home = configuration_home.filter(|directory| !directory.is_empty());
        let directory = match (configuration_home, home) {
            (Some(directory), _) => PathBuf::from(directory),
            (None, Some(home)) => PathBuf::from(home).join(".config"),
            (None, None) => {
                return Err(PlaceFileRefusal::Unlocated(Reason(
                    "neither XDG_CONFIG_HOME nor HOME is set".into(),
                )));
            }
        };
        if directory.is_absolute() {
            Ok(Self { directory })
        } else {
            Err(PlaceFileRefusal::Unlocated(Reason(format!(
                "the configuration home {} is not absolute",
                directory.display()
            ))))
        }
    }
}

pub trait LocatesPlaceFile {
    const NEXUS_DIRECTORY: &'static str = "nexus";
    const EXTENSION: &'static str = "signal";
    /// `<configuration home>/nexus/<place-name>/<nexus-name>.signal`; a
    /// place name that is no single path component is refused.
    fn place_file(
        &self,
        nexus_name: &NexusName,
        place_name: &PlaceName,
    ) -> Result<PlaceFile, PlaceFileRefusal>;
}

impl LocatesPlaceFile for ConfigurationHome {
    fn place_file(
        &self,
        nexus_name: &NexusName,
        place_name: &PlaceName,
    ) -> Result<PlaceFile, PlaceFileRefusal> {
        let path = self
            .directory
            .join(Self::NEXUS_DIRECTORY)
            .join(&place_name.0)
            .join(format!("{}.{}", nexus_name.0, Self::EXTENSION));
        let fault = |reason: String| {
            PlaceFileRefusal::Invalid(FileFault {
                file_path: FilePath(path.to_string_lossy().into_owned()),
                reason: Reason(reason),
            })
        };
        place_name.check().map_err(|reason| fault(reason.0))?;
        nexus_name.check().map_err(|reason| fault(reason.0))?;
        Ok(PlaceFile { path })
    }
}

/// What a name or a place must be to stand in a path.
pub trait ChecksPlace {
    fn check(&self) -> Result<(), Reason>;
}

impl ChecksPlace for PlaceName {
    fn check(&self) -> Result<(), Reason> {
        crate::placement::validate_place_name(&self.0)
            .map_err(|_| Reason(format!("{:?} is not a place name", self.0)))
    }
}

impl ChecksPlace for NexusName {
    fn check(&self) -> Result<(), Reason> {
        crate::placement::validate_place_name(&self.0)
            .map_err(|_| Reason(format!("{:?} is not a Nexus name", self.0)))
    }
}

/// A place names itself, and its Runtime and State are absolute
/// directories.
impl ChecksPlace for Place {
    fn check(&self) -> Result<(), Reason> {
        self.place_name.check()?;
        for (part, directory) in [("Runtime", &self.runtime.0), ("State", &self.state.0)] {
            if !Path::new(directory).is_absolute() {
                return Err(Reason(format!(
                    "the place's {part} {directory} is not absolute"
                )));
            }
        }
        Ok(())
    }
}

/// A fault of the file at `path`.
pub trait FaultsFile {
    fn fault(&self, reason: impl ToString) -> FileFault;
}

impl FaultsFile for Path {
    fn fault(&self, reason: impl ToString) -> FileFault {
        FileFault {
            file_path: FilePath(self.to_string_lossy().into_owned()),
            reason: Reason(reason.to_string()),
        }
    }
}

pub trait ReadsPlaceFile {
    /// The most a place file holds.
    const CAPACITY: usize = 64 * 1024;
    /// The place the file holds. There is no default place: where there is
    /// no file, or the file is not one framed Place, or its Place is not
    /// `place_name`'s or not absolute, the Nexus is refused naming the file.
    fn read_place(&self, place_name: &PlaceName) -> Result<Place, PlaceFileRefusal>;
}

impl ReadsPlaceFile for PlaceFile {
    fn read_place(&self, place_name: &PlaceName) -> Result<Place, PlaceFileRefusal> {
        let path = self.path.as_path();
        let mut file = match std::fs::File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == ErrorKind::NotFound => {
                return Err(PlaceFileRefusal::Missing(FilePath(
                    path.to_string_lossy().into_owned(),
                )));
            }
            Err(error) => return Err(PlaceFileRefusal::Unreadable(path.fault(error))),
        };
        let body = file
            .read_frame(FrameCapacity::from(Self::CAPACITY))
            .map_err(|error| PlaceFileRefusal::Undecodable(path.fault(format!("{error:?}"))))?;
        let mut rest = [0_u8; 1];
        match file.read(&mut rest) {
            Ok(0) => {}
            Ok(_) => {
                return Err(PlaceFileRefusal::Undecodable(
                    path.fault("bytes follow the place's frame"),
                ));
            }
            Err(error) => return Err(PlaceFileRefusal::Unreadable(path.fault(error))),
        }
        let place = Signal::<Place>::from(Vec::<u8>::from(body))
            .restore()
            .map_err(|error| PlaceFileRefusal::Undecodable(path.fault(format!("{error:?}"))))?;
        place
            .check()
            .map_err(|reason| PlaceFileRefusal::Invalid(path.fault(reason.0)))?;
        if &place.place_name != place_name {
            return Err(PlaceFileRefusal::Invalid(path.fault(format!(
                "the file holds the place {}, not {}",
                place.place_name.0, place_name.0
            ))));
        }
        Ok(place)
    }
}

/// A Nexus, and the client that speaks for it, find the place by its name
/// and the configuration home, in the place file and nowhere else.
pub trait FindsPlace {
    /// The place `nexus_name`'s file holds under `home`.
    fn find_place_under(
        &self,
        nexus_name: &NexusName,
        home: &ConfigurationHome,
    ) -> Result<Place, PlaceFileRefusal>;
    /// `find_place_under` this process's configuration home.
    fn find_place(&self, nexus_name: &NexusName) -> Result<Place, PlaceFileRefusal> {
        self.find_place_under(nexus_name, &ConfigurationHome::of_process()?)
    }
}

impl FindsPlace for PlaceName {
    fn find_place_under(
        &self,
        nexus_name: &NexusName,
        home: &ConfigurationHome,
    ) -> Result<Place, PlaceFileRefusal> {
        home.place_file(nexus_name, self)?.read_place(self)
    }
}

pub trait WritesPlaceFile {
    /// Writes `place`, one frame, replacing the file whole: it is written
    /// beside its name and moved there, its directory made first.
    fn write_place(&self, place: &Place) -> Result<(), PlaceFileRefusal>;
}

impl WritesPlaceFile for PlaceFile {
    fn write_place(&self, place: &Place) -> Result<(), PlaceFileRefusal> {
        let path = self.path.as_path();
        let unwritable = |reason: String| PlaceFileRefusal::Unwritable(path.fault(reason));
        place
            .check()
            .map_err(|reason| PlaceFileRefusal::Invalid(path.fault(reason.0)))?;
        let framed = place
            .signalize()
            .map_err(|error| unwritable(format!("{error:?}")))?
            .framed(FrameCapacity::from(<Self as ReadsPlaceFile>::CAPACITY))
            .map_err(|error| unwritable(format!("{error:?}")))?;
        let Some(name) = path.file_name() else {
            return Err(unwritable("the path names no file".into()));
        };
        let directory = path.parent().unwrap_or(Path::new("."));
        std::fs::create_dir_all(directory).map_err(|error| unwritable(error.to_string()))?;
        let mut written = name.to_owned();
        written.push(format!(".{}.writing", std::process::id()));
        let written = directory.join(written);
        std::fs::write(&written, framed).map_err(|error| unwritable(error.to_string()))?;
        std::fs::rename(&written, path).map_err(|error| {
            let _ = std::fs::remove_file(&written);
            unwritable(error.to_string())
        })
    }
}

#[cfg(feature = "text")]
pub use compiling::CompilesPlace;

#[cfg(feature = "text")]
mod compiling {
    use super::{ChecksPlace, FaultsFile, PlaceFile, WritesPlaceFile};
    use crate::placement::{Compilation, Configured, Place, PlaceFileRefusal, SignalFile};
    use datom_codec::Budget;
    use std::path::{Path, PathBuf};

    /// `nexus-configure`'s one act: a place written in datom, compiled into
    /// its place file.
    pub trait CompilesPlace {
        /// The most a datom file holds.
        const DATOM_BYTES: usize = 64 * 1024;
        fn compile(&self) -> Configured;
    }

    impl CompilesPlace for Compilation {
        fn compile(&self) -> Configured {
            let datom_file = Path::new(&self.datom_file.0);
            let compiled = std::fs::read_to_string(datom_file)
                .map_err(|error| PlaceFileRefusal::Unreadable(datom_file.fault(error)))
                .and_then(|source| {
                    let mut budget = Budget {
                        remaining: Self::DATOM_BYTES as i64,
                        reader: protos::ReaderBudget::try_from(Self::DATOM_BYTES)
                            .expect("a constant reader budget"),
                        depth: 0,
                        maximum_depth: 64,
                    };
                    crate::decode::<Place>(source.trim(), &mut budget).map_err(|error| {
                        PlaceFileRefusal::Undecodable(datom_file.fault(format!("{error:?}")))
                    })
                })
                .and_then(|place| {
                    place
                        .check()
                        .map_err(|reason| PlaceFileRefusal::Invalid(datom_file.fault(reason.0)))?;
                    PlaceFile {
                        path: PathBuf::from(&self.signal_file.0),
                    }
                    .write_place(&place)
                });
            match compiled {
                Ok(()) => Configured::Compiled(SignalFile(self.signal_file.0.clone())),
                Err(refusal) => Configured::Refused(refusal),
            }
        }
    }
}
