//! nexus-configure: a place written in datom becomes its place file.
#![cfg(feature = "text")]

use std::path::PathBuf;
use std::process::Command;
use translator::place_file::{CompilesPlace, PlaceFile, ReadsPlaceFile};
use translator::placement::{
    Compilation, Configured, DatomFile, FileFault, Place, PlaceFileRefusal, PlaceName, Runtime,
    SignalFile, State,
};

struct Root(PathBuf);

impl Root {
    fn new(label: &str) -> Self {
        let directory = std::env::temp_dir().join(format!(
            "translator-nexus-configure-{label}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).expect("test root");
        Self(directory)
    }

    fn compilation(&self, datom: &str) -> Compilation {
        let datom_file = self.0.join("place.datom");
        std::fs::write(&datom_file, datom).expect("datom written");
        Compilation {
            datom_file: DatomFile(datom_file.to_string_lossy().into_owned()),
            signal_file: SignalFile(
                self.0
                    .join("nexus/main/flow-nexus.signal")
                    .to_string_lossy()
                    .into_owned(),
            ),
        }
    }
}

impl Drop for Root {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn a_datom_place_compiles_into_the_place_file() {
    let root = Root::new("compiles");
    let compilation =
        root.compilation("{ main /run/user/1001/nexus/main /home/li/.local/state/nexus/main }\n");
    assert_eq!(
        compilation.compile(),
        Configured::Compiled(compilation.signal_file.clone())
    );
    assert_eq!(
        PlaceFile {
            path: PathBuf::from(&compilation.signal_file.0)
        }
        .read_place(&PlaceName("main".into())),
        Ok(Place {
            place_name: PlaceName("main".into()),
            runtime: Runtime("/run/user/1001/nexus/main".into()),
            state: State("/home/li/.local/state/nexus/main".into()),
        })
    );
}

#[test]
fn a_datom_that_is_no_place_is_refused_naming_the_datom_file() {
    let root = Root::new("refused");
    for (datom, undecodable) in [
        ("{ main /run }", true),
        ("Place.{ main /run /state }", true),
        ("{ main run/nexus/main /state }", false),
        ("{ .. /run /state }", false),
    ] {
        let compilation = root.compilation(datom);
        let refused = compilation.compile();
        let named = |fault: &FileFault| fault.file_path.0 == compilation.datom_file.0;
        assert!(
            match &refused {
                Configured::Refused(PlaceFileRefusal::Undecodable(fault)) =>
                    undecodable && named(fault),
                Configured::Refused(PlaceFileRefusal::Invalid(fault)) =>
                    !undecodable && named(fault),
                _ => false,
            },
            "{datom}: {refused:?}"
        );
        assert!(!PathBuf::from(&compilation.signal_file.0).exists());
    }
}

#[test]
fn the_tool_takes_one_datom_and_answers_one() {
    let root = Root::new("tool");
    let compilation = root.compilation("{ main /run/user/1001/nexus/main /state/main }");
    let tool = env!("CARGO_BIN_EXE_nexus-configure");
    let compiled = Command::new(tool)
        .arg(format!(
            "Compile.{{ «{}» «{}» }}",
            compilation.datom_file.0, compilation.signal_file.0
        ))
        .output()
        .expect("nexus-configure runs");
    assert!(compiled.status.success(), "{compiled:?}");
    assert_eq!(
        String::from_utf8_lossy(&compiled.stdout).trim(),
        format!("Compiled.{}", compilation.signal_file.0)
    );
    let refused = Command::new(tool)
        .arg(format!(
            "Compile.{{ «{}» «{}» }}",
            root.0.join("absent.datom").display(),
            compilation.signal_file.0
        ))
        .output()
        .expect("nexus-configure runs");
    assert_eq!(refused.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&refused.stdout).starts_with("Refused.Unreadable."));
    let usage = Command::new(tool).output().expect("nexus-configure runs");
    assert_eq!(usage.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&usage.stderr).starts_with("usage:"));
}
