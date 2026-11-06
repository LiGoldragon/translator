//! Place preparation: the migration plan serializes cooperating preparers of one destination
//! and validates what it relies on; the explicit plan only creates directories and refuses an
//! actual identity conflict with the standard destination of the same name.
#![cfg(unix)]

use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;

use translator::placement::{
    runtime_path, state_path, Place, PlaceName, PlacePreparationError, Preparable, Runtime, State,
};

static ENVIRONMENT: Mutex<()> = Mutex::new(());

struct Root(PathBuf);

impl Root {
    fn new(label: &str) -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let sequence = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Self(std::env::temp_dir().join(format!(
            "translator-prep-{label}-{}-{sequence}",
            std::process::id()
        )))
    }
}

impl Drop for Root {
    fn drop(&mut self) {
        let _ = Command::new("chmod")
            .arg("-R")
            .arg("u+rwx")
            .arg(&self.0)
            .status();
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn place(root: &Path) -> Place {
    Place {
        place_name: PlaceName("alpha".into()),
        runtime: Runtime(root.join("run/nexus/alpha").to_string_lossy().into_owned()),
        state: State(
            root.join("state/nexus/alpha")
                .to_string_lossy()
                .into_owned(),
        ),
    }
}

fn seed_legacy(root: &Path) -> PathBuf {
    let legacy = root.join("state/metaflow/alpha");
    std::fs::create_dir_all(legacy.join("nested")).expect("legacy state");
    std::fs::write(legacy.join("memory"), b"first\0second\n").expect("memory");
    std::fs::write(legacy.join("nested/binary"), [0xffu8, 0, 0xfe]).expect("binary");
    legacy
}

fn tree(path: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut found = Vec::new();
    let mut pending = vec![path.to_owned()];
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(&directory).expect("read directory") {
            let entry = entry.expect("entry").path();
            if entry.is_dir() {
                pending.push(entry);
            } else {
                found.push((
                    entry.strip_prefix(path).expect("relative").to_owned(),
                    std::fs::read(&entry).expect("file"),
                ));
            }
        }
    }
    found.sort();
    found
}

fn is_root_user() -> bool {
    rustix::process::geteuid().is_root()
}

#[test]
fn child_prepares() {
    let (Some(root), Some(go)) = (
        std::env::var_os("TRANSLATOR_CHILD_ROOT"),
        std::env::var_os("TRANSLATOR_CHILD_GO"),
    ) else {
        return;
    };
    while !Path::new(&go).exists() {
        std::thread::sleep(std::time::Duration::from_micros(200));
    }
    place(Path::new(&root))
        .prepare_for_start()
        .expect("child preparation");
}

#[test]
fn concurrent_processes_preparing_one_legacy_place_move_it_once() {
    for round in 0..12 {
        let root = Root::new(&format!("race{round}"));
        let legacy = seed_legacy(&root.0);
        let before = tree(&legacy);
        let go = root.0.join("go");
        let children: Vec<_> = (0..4)
            .map(|_| {
                Command::new(std::env::current_exe().expect("test binary"))
                    .args([
                        "--exact",
                        "child_prepares",
                        "--nocapture",
                        "--test-threads=1",
                    ])
                    .env("TRANSLATOR_CHILD_ROOT", &root.0)
                    .env("TRANSLATOR_CHILD_GO", &go)
                    .spawn()
                    .expect("spawn a preparer")
            })
            .collect();
        std::thread::sleep(std::time::Duration::from_millis(50));
        std::fs::write(&go, b"go").expect("release the preparers");
        for mut child in children {
            assert!(
                child.wait().expect("child status").success(),
                "every preparer succeeds, round {round}"
            );
        }
        assert!(!legacy.exists());
        assert_eq!(
            tree(&state_path(&place(&root.0))),
            before,
            "moved once, byte for byte"
        );
        assert!(runtime_path(&place(&root.0)).is_dir());
    }
}

#[test]
fn both_stores_present_are_refused_and_left_unchanged() {
    let root = Root::new("both");
    let legacy = seed_legacy(&root.0);
    let current = state_path(&place(&root.0));
    std::fs::create_dir_all(&current).expect("current");
    std::fs::write(current.join("memory"), b"current").expect("current memory");
    let (before_legacy, before_current) = (tree(&legacy), tree(&current));
    match place(&root.0).prepare_for_start() {
        Err(PlacePreparationError::StateConflict {
            legacy: found_legacy,
            current: found_current,
        }) => {
            assert_eq!(
                (found_legacy, found_current),
                (legacy.clone(), current.clone())
            );
        }
        other => panic!("expected the conflict, got {other:?}"),
    }
    assert_eq!(
        (tree(&legacy), tree(&current)),
        (before_legacy, before_current)
    );
    assert!(
        !runtime_path(&place(&root.0)).exists(),
        "nothing else was started"
    );
    assert!(
        root.0.join("state/nexus-place-locks/alpha").is_file(),
        "the lock file is the one added artifact"
    );
}

#[test]
fn a_failure_after_the_move_leaves_the_destination_and_a_retry_completes() {
    if is_root_user() {
        return;
    }
    let root = Root::new("retry");
    let legacy = seed_legacy(&root.0);
    let before = tree(&legacy);
    std::fs::create_dir_all(root.0.join("run")).expect("runtime root");
    std::fs::set_permissions(root.0.join("run"), std::fs::Permissions::from_mode(0o500))
        .expect("lock the runtime root");
    let prepared = place(&root.0);
    assert!(
        matches!(
            prepared.prepare_for_start(),
            Err(PlacePreparationError::Io { .. })
        ),
        "runtime creation fails"
    );
    assert!(!legacy.exists(), "the move had succeeded");
    assert_eq!(
        tree(&state_path(&prepared)),
        before,
        "the destination is intact"
    );
    std::fs::set_permissions(root.0.join("run"), std::fs::Permissions::from_mode(0o700))
        .expect("unlock");
    prepared.prepare_for_start().expect("the retry completes");
    assert_eq!(
        tree(&state_path(&prepared)),
        before,
        "no second move, nothing replaced"
    );
    assert!(runtime_path(&prepared).is_dir());
}

#[test]
fn symlinked_nexus_and_leaves_and_permissive_leaves_are_refused() {
    let root = Root::new("refuse");
    let elsewhere = root.0.join("elsewhere");
    std::fs::create_dir_all(&elsewhere).expect("elsewhere");
    std::fs::create_dir_all(root.0.join("state")).expect("state root");
    symlink(&elsewhere, root.0.join("state/nexus")).expect("symlinked nexus");
    assert!(matches!(
        place(&root.0).prepare_for_start(),
        Err(PlacePreparationError::Io { .. })
    ));
    std::fs::remove_file(root.0.join("state/nexus")).expect("remove symlink");

    std::fs::create_dir_all(root.0.join("state/nexus")).expect("nexus");
    symlink(&elsewhere, root.0.join("state/nexus/alpha")).expect("symlinked leaf");
    assert!(matches!(
        place(&root.0).prepare_for_start(),
        Err(PlacePreparationError::Io { .. })
    ));
    assert!(
        std::fs::read_dir(&elsewhere)
            .expect("target")
            .next()
            .is_none(),
        "nothing was created through the link"
    );
    std::fs::remove_file(root.0.join("state/nexus/alpha")).expect("remove leaf link");

    std::fs::create_dir_all(root.0.join("state/nexus/alpha")).expect("leaf");
    std::fs::set_permissions(
        root.0.join("state/nexus/alpha"),
        std::fs::Permissions::from_mode(0o777),
    )
    .expect("permissive");
    assert!(matches!(
        place(&root.0).prepare_for_start(),
        Err(PlacePreparationError::Io { .. })
    ));
    std::fs::set_permissions(
        root.0.join("state/nexus/alpha"),
        std::fs::Permissions::from_mode(0o755),
    )
    .expect("umask-style");
    place(&root.0)
        .prepare_for_start()
        .expect("a 0755 leaf the old preparation could have made is accepted");
}

#[test]
fn a_root_reached_through_a_symlink_is_acquired_and_verified() {
    let root = Root::new("home");
    let real = root.0.join("real-home");
    std::fs::create_dir_all(&real).expect("real home");
    symlink(&real, root.0.join("home-link")).expect("symlinked home");
    let place = Place {
        place_name: PlaceName("alpha".into()),
        runtime: Runtime(
            root.0
                .join("home-link/run/nexus/alpha")
                .to_string_lossy()
                .into_owned(),
        ),
        state: State(
            root.0
                .join("home-link/state/nexus/alpha")
                .to_string_lossy()
                .into_owned(),
        ),
    };
    place
        .prepare_for_start()
        .expect("prepare through the symlinked home");
    assert!(real.join("state/nexus/alpha").is_dir() && real.join("run/nexus/alpha").is_dir());
}

#[test]
fn an_explicit_place_that_is_the_standard_destination_of_its_name_is_refused() {
    let _guard = ENVIRONMENT.lock().unwrap_or_else(|p| p.into_inner());
    let root = Root::new("alias");
    std::fs::create_dir_all(root.0.join("state/nexus/alpha")).expect("standard destination");
    std::env::set_var("XDG_STATE_HOME", root.0.join("state"));
    std::env::set_var("HOME", &root.0);
    symlink(root.0.join("state/nexus"), root.0.join("alias")).expect("alias of the namespace");
    symlink(root.0.join("state/nexus/alpha"), root.0.join("destination"))
        .expect("alias of the destination");
    let through_namespace = Place {
        place_name: PlaceName("beta".into()),
        runtime: Runtime(root.0.join("run-beta").to_string_lossy().into_owned()),
        state: State(root.0.join("alias/beta").to_string_lossy().into_owned()),
    };
    let result = through_namespace.prepare_for_start();
    assert!(
        matches!(result, Err(PlacePreparationError::Io { .. })),
        "{result:?}"
    );
    assert!(
        !root.0.join("state/nexus/beta").exists(),
        "nothing was created in the protected namespace"
    );
    let onto_destination = Place {
        place_name: PlaceName("alpha".into()),
        runtime: Runtime(root.0.join("run-alpha").to_string_lossy().into_owned()),
        state: State(root.0.join("destination").to_string_lossy().into_owned()),
    };
    assert!(matches!(
        onto_destination.prepare_for_start(),
        Err(PlacePreparationError::Io { .. })
    ));
    let unrelated = Place {
        place_name: PlaceName("alpha".into()),
        runtime: Runtime(root.0.join("run-free").to_string_lossy().into_owned()),
        state: State(
            root.0
                .join("elsewhere/state")
                .to_string_lossy()
                .into_owned(),
        ),
    };
    unrelated
        .prepare_for_start()
        .expect("an unrelated explicit place is prepared");
    std::env::remove_var("XDG_STATE_HOME");
}
