//! One operation on a prepared place: the standard plan keeps its exclusion for the whole
//! operation and releases it on success, error and unwind; the explicit plan runs unlocked.
#![cfg(unix)]

use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;

use translator::placement::{
    Held, Place, PlaceName, PlacePreparationError, Preparable, PreparationMode, Runtime,
    ScopedPreparation, State,
};

struct Root(PathBuf);

impl Root {
    fn new(label: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
        Self(std::env::temp_dir().join(format!(
            "translator-scoped-{label}-{}-{sequence}",
            std::process::id()
        )))
    }
}

impl Drop for Root {
    fn drop(&mut self) {
        // No child process is spawned here: a forked child holds a copy of every open
        // descriptor until its exec, and a flock stays held while any copy is open, so a
        // spawn in one test could keep another test's released lock taken.
        restore_access(&self.0);
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Gives the owner read, write and search access to a tree, so it can be removed.
fn restore_access(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let Ok(metadata) = std::fs::symlink_metadata(path) else {
        return;
    };
    if metadata.file_type().is_symlink() {
        return;
    }
    let mut permissions = metadata.permissions();
    permissions.set_mode(permissions.mode() | 0o700);
    let _ = std::fs::set_permissions(path, permissions);
    if metadata.is_dir() {
        if let Ok(entries) = std::fs::read_dir(path) {
            for entry in entries.flatten() {
                restore_access(&entry.path());
            }
        }
    }
}

fn standard(root: &Path) -> Place {
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

fn explicit(root: &Path) -> Place {
    Place {
        place_name: PlaceName("alpha".into()),
        runtime: Runtime(root.join("custom-run").to_string_lossy().into_owned()),
        state: State(root.join("custom-state").to_string_lossy().into_owned()),
    }
}

fn lock_file(root: &Path) -> PathBuf {
    root.join("state/nexus-place-locks/alpha")
}

fn free(path: &Path) -> bool {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .expect("open lock");
    file.try_lock().is_ok()
}

fn seed_legacy(root: &Path) -> PathBuf {
    let legacy = root.join("state/metaflow/alpha");
    std::fs::create_dir_all(&legacy).expect("legacy");
    std::fs::write(legacy.join("memory"), b"first\0second\n").expect("memory");
    legacy
}

#[test]
fn the_exclusion_is_held_during_the_operation_and_free_after() {
    let root = Root::new("held");
    let place = standard(&root.0);
    let lock = lock_file(&root.0);
    let held = place
        .with_prepared_place(|prepared| {
            assert_eq!(prepared.mode(), PreparationMode::MigrationExclusive);
            assert!(prepared.state().is_dir() && prepared.runtime().is_dir());
            Ok::<_, ()>(!free(&lock))
        })
        .expect("scoped");
    assert!(held, "the lock is taken while the operation runs");
    assert!(free(&lock), "the lock is free once the call returns");
}

#[test]
fn an_operation_error_releases_and_is_returned_typed() {
    let root = Root::new("error");
    let place = standard(&root.0);
    let outcome = place.with_prepared_place(|_| Err::<(), _>("refused"));
    assert!(matches!(outcome, Err(Held::Operation("refused"))));
    assert!(free(&lock_file(&root.0)));
}

#[test]
fn a_panicking_operation_releases_the_exclusion() {
    let root = Root::new("panic");
    let place = standard(&root.0);
    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = place.with_prepared_place(|_| -> Result<(), ()> { panic!("inside") });
    }));
    assert!(caught.is_err());
    assert!(free(&lock_file(&root.0)), "unwinding drops the guard");
}

#[test]
fn legacy_state_is_moved_intact_before_the_operation_sees_it() {
    let root = Root::new("legacy");
    let legacy = seed_legacy(&root.0);
    let place = standard(&root.0);
    let seen = place
        .with_prepared_place(|prepared| {
            Ok::<_, ()>((
                std::fs::read(prepared.state().join("memory")).expect("moved memory"),
                legacy.exists(),
            ))
        })
        .expect("scoped");
    assert_eq!(seen.0, b"first\0second\n");
    assert!(
        !seen.1,
        "the legacy directory is gone when the operation runs"
    );
}

#[test]
fn both_stores_present_refuse_without_running_the_operation() {
    let root = Root::new("both");
    let legacy = seed_legacy(&root.0);
    let current = root.0.join("state/nexus/alpha");
    std::fs::create_dir_all(&current).expect("current");
    std::fs::write(current.join("memory"), b"current").expect("current memory");
    let ran = AtomicUsize::new(0);
    let outcome = standard(&root.0).with_prepared_place(|_| {
        ran.fetch_add(1, Ordering::SeqCst);
        Ok::<(), ()>(())
    });
    assert!(matches!(
        outcome,
        Err(Held::Preparation(
            PlacePreparationError::StateConflict { .. }
        ))
    ));
    assert_eq!(ran.load(Ordering::SeqCst), 0);
    assert_eq!(
        std::fs::read(legacy.join("memory")).unwrap(),
        b"first\0second\n"
    );
    assert_eq!(std::fs::read(current.join("memory")).unwrap(), b"current");
}

#[test]
fn an_explicit_place_runs_unlocked_and_keeps_its_decoys() {
    let root = Root::new("explicit");
    let state = root.0.join("custom-state");
    std::fs::create_dir_all(&state).expect("state");
    std::fs::write(state.join("decoy"), b"decoy").expect("decoy");
    let legacy = seed_legacy(&root.0);
    let mode = explicit(&root.0)
        .with_prepared_place(|prepared| Ok::<_, ()>(prepared.mode()))
        .expect("scoped");
    assert_eq!(mode, PreparationMode::CreateOnly);
    assert!(
        !root.0.join("state/nexus-place-locks").exists(),
        "no lock is made for an explicit place"
    );
    assert_eq!(std::fs::read(state.join("decoy")).unwrap(), b"decoy");
    assert_eq!(
        std::fs::read(legacy.join("memory")).unwrap(),
        b"first\0second\n",
        "no legacy state is looked for"
    );
}

#[test]
fn prepare_for_start_is_the_empty_operation() {
    let root = Root::new("empty");
    let legacy = seed_legacy(&root.0);
    let place = standard(&root.0);
    place.prepare_for_start().expect("prepare");
    assert!(!legacy.exists());
    place.prepare_for_start().expect("repeat");
    assert_eq!(
        std::fs::read(root.0.join("state/nexus/alpha/memory")).unwrap(),
        b"first\0second\n"
    );
}

/// A filename selection a consumer might run inside the scope: rename a sole legacy file.
fn select(state: &Path) -> Result<bool, std::io::Error> {
    let current = state.join("belief");
    let legacy = state.join("belief.sema");
    match (current.exists(), legacy.exists()) {
        (false, true) => std::fs::rename(&legacy, &current).map(|()| true),
        _ => Ok(false),
    }
}

#[test]
fn concurrent_selections_inside_the_scope_move_once_without_error() {
    let root = Root::new("concurrent");
    let place = Arc::new(standard(&root.0));
    place.prepare_for_start().expect("prepare");
    std::fs::write(root.0.join("state/nexus/alpha/belief.sema"), b"bytes").expect("legacy file");
    let moved = Arc::new(AtomicUsize::new(0));
    let handles: Vec<_> = (0..8)
        .map(|_| {
            let place = Arc::clone(&place);
            let moved = Arc::clone(&moved);
            std::thread::spawn(move || {
                let first = place
                    .with_prepared_place(|prepared| select(prepared.state()))
                    .expect("no preparation or selection error, in particular no ENOENT");
                if first {
                    moved.fetch_add(1, Ordering::SeqCst);
                }
            })
        })
        .collect();
    for handle in handles {
        handle.join().expect("thread");
    }
    assert_eq!(moved.load(Ordering::SeqCst), 1);
    assert_eq!(
        std::fs::read(root.0.join("state/nexus/alpha/belief")).unwrap(),
        b"bytes"
    );
    let _ = File::open(root.0.join("state"));
}
