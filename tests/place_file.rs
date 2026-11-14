//! The place file: where it lies, and the one framed Place it holds.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use translator::place_file::{
    ConfigurationHome, LocatesConfigurationHome, LocatesPlaceFile, PlaceFile, ReadsPlaceFile,
    WritesPlaceFile,
};
use translator::placement::{
    FileFault, FilePath, NexusName, Place, PlaceFileRefusal, PlaceName, Runtime, State,
};
use translator::{Framable, FrameCapacity, Signalizable};

struct Root(PathBuf);

impl Root {
    fn new(label: &str) -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let sequence = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let directory = std::env::temp_dir().join(format!(
            "translator-place-file-{label}-{}-{sequence}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).expect("test root");
        Self(directory)
    }
}

impl Drop for Root {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn place(name: &str) -> Place {
    Place {
        place_name: PlaceName(name.into()),
        runtime: Runtime(format!("/run/user/1001/nexus/{name}")),
        state: State(format!("/home/li/.local/state/nexus/{name}")),
    }
}

fn flow() -> NexusName {
    NexusName("flow-nexus".into())
}

#[test]
fn the_place_file_lies_in_its_place_under_the_configuration_home() {
    let home = ConfigurationHome::under(Some(OsStr::new("/c")), Some(OsStr::new("/home/li")))
        .expect("configuration home");
    assert_eq!(
        home.place_file(&flow(), &PlaceName("main".into()))
            .unwrap()
            .path,
        Path::new("/c/nexus/main/flow-nexus.signal")
    );
    for unset in [None, Some(OsStr::new(""))] {
        assert_eq!(
            ConfigurationHome::under(unset, Some(OsStr::new("/home/li")))
                .unwrap()
                .place_file(&flow(), &PlaceName("next".into()))
                .unwrap()
                .path,
            Path::new("/home/li/.config/nexus/next/flow-nexus.signal")
        );
    }
    for (configuration_home, home) in [
        (None, None),
        (Some(OsStr::new("c")), Some(OsStr::new("/home/li"))),
        (None, Some(OsStr::new("home/li"))),
    ] {
        assert!(matches!(
            ConfigurationHome::under(configuration_home, home),
            Err(PlaceFileRefusal::Unlocated(_))
        ));
    }
    for refused in ["", ".", "..", "a/b", "/main"] {
        assert!(
            matches!(
                home.place_file(&flow(), &PlaceName(refused.into())),
                Err(PlaceFileRefusal::Invalid(_))
            ),
            "{refused:?}"
        );
    }
}

#[test]
fn a_written_place_reads_back_and_an_absent_file_is_none() {
    let root = Root::new("round-trip");
    let file = PlaceFile {
        path: root.0.join("nexus/main/flow-nexus.signal"),
    };
    let main = PlaceName("main".into());
    assert_eq!(file.read_place(&main), Ok(None));
    file.write_place(&place("main")).expect("written");
    assert_eq!(file.read_place(&main), Ok(Some(place("main"))));
    let leftovers = std::fs::read_dir(root.0.join("nexus/main"))
        .unwrap()
        .count();
    assert_eq!(leftovers, 1, "only the place file is left");
}

#[test]
fn a_file_holding_another_place_is_refused_naming_it() {
    let root = Root::new("another");
    let file = PlaceFile {
        path: root.0.join("flow-nexus.signal"),
    };
    file.write_place(&place("next")).expect("written");
    assert!(matches!(
        file.read_place(&PlaceName("main".into())),
        Err(PlaceFileRefusal::Invalid(FileFault { file_path, .. }))
            if file_path == FilePath(file.path.to_string_lossy().into_owned())
    ));
}

#[test]
fn a_file_that_is_no_framed_place_is_undecodable() {
    let root = Root::new("undecodable");
    let file = PlaceFile {
        path: root.0.join("flow-nexus.signal"),
    };
    let main = PlaceName("main".into());
    let framed = place("main")
        .signalize()
        .unwrap()
        .framed(FrameCapacity::default())
        .unwrap();
    let mut trailing = framed.clone();
    trailing.push(0);
    for bytes in [
        b"{ main /run /state }".to_vec(),
        Vec::new(),
        framed[..framed.len() - 1].to_vec(),
        trailing,
    ] {
        std::fs::write(&file.path, &bytes).unwrap();
        assert!(
            matches!(
                file.read_place(&main),
                Err(PlaceFileRefusal::Undecodable(FileFault { ref file_path, .. }))
                    if file_path.0 == file.path.to_string_lossy()
            ),
            "{bytes:?}: {:?}",
            file.read_place(&main)
        );
    }
}

#[test]
fn a_relative_place_is_neither_written_nor_read() {
    let root = Root::new("relative");
    let file = PlaceFile {
        path: root.0.join("flow-nexus.signal"),
    };
    let relative = Place {
        runtime: Runtime("run/nexus/main".into()),
        ..place("main")
    };
    assert!(matches!(
        file.write_place(&relative),
        Err(PlaceFileRefusal::Invalid(_))
    ));
    assert!(!file.path.exists());
    let framed = relative
        .signalize()
        .unwrap()
        .framed(FrameCapacity::default())
        .unwrap();
    std::fs::write(&file.path, framed).unwrap();
    assert!(matches!(
        file.read_place(&PlaceName("main".into())),
        Err(PlaceFileRefusal::Invalid(_))
    ));
}
