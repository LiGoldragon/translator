//! Anchored, exclusive preparation of a standard-shaped Place on Unix.
//!
//! A standard Place has state at `<root>/nexus/<name>`, so a legacy store at
//! `<root>/metaflow/<name>` may be moved into it. Two preparers of the same
//! destination serialize on one advisory lock, taken here and held through the
//! existence checks, the move and the directory creations. The root is opened
//! once, by pathname, following symlinks the way the old preparation did, and
//! verified owned; every later step is relative to a retained descriptor with
//! no-follow. The guarantee is mutual exclusion between cooperating preparers
//! of one configured destination: not lifetime ownership of the place, not
//! protection from other writers, and not alias safety for conflicting
//! configurations of another plan.

use std::fs::File;
use std::io;
use std::path::Path;

use rustix::fd::{AsFd, OwnedFd};
use rustix::fs::{fstat, mkdirat, openat, statat, AtFlags, FileType, Mode, OFlags, Stat, CWD};
use rustix::io::Errno;
use rustix::process::geteuid;

use crate::placement::PlacePreparationError;

type Outcome<T> = Result<T, PlacePreparationError>;

const NEXUS: &str = "nexus";
const LEGACY: &str = "metaflow";
const LOCKS: &str = "nexus-place-locks";

fn failure(path: &Path, source: Errno) -> PlacePreparationError {
    PlacePreparationError::Io {
        path: path.to_owned(),
        source: io::Error::from(source),
    }
}

fn refusal(path: &Path, kind: io::ErrorKind, reason: &str) -> PlacePreparationError {
    PlacePreparationError::Io {
        path: path.to_owned(),
        source: io::Error::new(kind, reason.to_owned()),
    }
}

/// An object this preparation relies on: the expected kind, owned by the
/// effective user, and not writable by group or others. Group and other read
/// or execute bits are accepted, so directories the old preparation created
/// under a common umask stay valid.
fn validate(path: &Path, stat: &Stat, expected: FileType) -> Outcome<()> {
    if FileType::from_raw_mode(stat.st_mode as _) != expected {
        return Err(refusal(
            path,
            io::ErrorKind::InvalidInput,
            "not the expected kind of file (a symlink is not followed here)",
        ));
    }
    if stat.st_uid != geteuid().as_raw() {
        return Err(refusal(
            path,
            io::ErrorKind::PermissionDenied,
            "owned by another user",
        ));
    }
    if (stat.st_mode as u32) & 0o022 != 0 {
        return Err(refusal(
            path,
            io::ErrorKind::PermissionDenied,
            "writable by group or others",
        ));
    }
    Ok(())
}

/// Open one directory by pathname, following symlinks, as the old preparation
/// did.
fn open_by_pathname(path: &Path) -> Result<OwnedFd, Errno> {
    openat(
        CWD,
        path,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
    )
}

/// Acquire the root: open it once by pathname, following symlinks, and verify
/// the object reached. A missing root is created beneath the deepest existing
/// ancestor, one component at a time, descriptor-relative at mode 0700; that
/// ancestor must itself be verified, or be a sticky directory (a shared
/// temporary parent) whose child this process then owns.
fn acquire_root(path: &Path) -> Outcome<OwnedFd> {
    let mut missing = Vec::new();
    let mut base = path;
    let anchor = loop {
        match open_by_pathname(base) {
            Ok(anchor) => break anchor,
            Err(Errno::NOENT) => {
                let Some(name) = base.file_name() else {
                    return Err(failure(path, Errno::NOENT));
                };
                missing.push(name);
                base = base.parent().ok_or_else(|| failure(path, Errno::NOENT))?;
            }
            Err(errno) => return Err(failure(base, errno)),
        }
    };
    let stat = fstat(&anchor).map_err(|errno| failure(base, errno))?;
    if missing.is_empty() {
        validate(path, &stat, FileType::Directory)?;
        return Ok(anchor);
    }
    let sticky_shared = FileType::from_raw_mode(stat.st_mode as _) == FileType::Directory
        && (stat.st_mode as u32) & 0o1000 != 0
        && (stat.st_uid == 0 || stat.st_uid == geteuid().as_raw());
    if !sticky_shared {
        validate(base, &stat, FileType::Directory)?;
    }
    let mut current = anchor;
    let mut walked = base.to_owned();
    for name in missing.into_iter().rev() {
        walked.push(name);
        match mkdirat(&current, name, Mode::from_raw_mode(0o700)) {
            Ok(()) | Err(Errno::EXIST) => {}
            Err(errno) => return Err(failure(&walked, errno)),
        }
        let child = openat(
            &current,
            name,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|errno| failure(&walked, errno))?;
        let stat = fstat(&child).map_err(|errno| failure(&walked, errno))?;
        validate(&walked, &stat, FileType::Directory)?;
        current = child;
    }
    Ok(current)
}

/// Open an existing child directory without following a symlink, then verify
/// it by descriptor.
fn open_directory(parent: &impl AsFd, path: &Path, name: &str) -> Outcome<OwnedFd> {
    let child = openat(
        parent,
        name,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|errno| failure(path, errno))?;
    let stat = fstat(&child).map_err(|errno| failure(path, errno))?;
    validate(path, &stat, FileType::Directory)?;
    Ok(child)
}

/// Whether a child entry exists, without following a symlink.
fn exists(parent: &impl AsFd, path: &Path, name: &str) -> Outcome<bool> {
    match statat(parent, name, AtFlags::SYMLINK_NOFOLLOW) {
        Ok(_) => Ok(true),
        Err(Errno::NOENT) => Ok(false),
        Err(errno) => Err(failure(path, errno)),
    }
}

/// Create the child directory at mode 0700 when it is absent, treating a
/// concurrent creation as present, then open and verify it.
fn ensure_directory(parent: &impl AsFd, path: &Path, name: &str) -> Outcome<OwnedFd> {
    if !exists(parent, path, name)? {
        match mkdirat(parent, name, Mode::from_raw_mode(0o700)) {
            Ok(()) | Err(Errno::EXIST) => {}
            Err(errno) => return Err(failure(path, errno)),
        }
    }
    open_directory(parent, path, name)
}

/// The exclusive lock on one Place destination. It owns the only descriptor,
/// opened close-on-exec and never cloned or duplicated; dropping it closes the
/// descriptor and releases the lock, and process death does the same. The lock
/// file is never unlinked, so every preparer locks the same file.
struct Exclusion {
    _file: File,
}

impl Exclusion {
    fn take(root: &OwnedFd, root_path: &Path, name: &str) -> Outcome<Self> {
        let locks_path = root_path.join(LOCKS);
        let locks = ensure_directory(root, &locks_path, LOCKS)?;
        let file_path = locks_path.join(name);
        let descriptor = openat(
            &locks,
            name,
            OFlags::RDWR | OFlags::CREATE | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::from_raw_mode(0o600),
        )
        .map_err(|errno| failure(&file_path, errno))?;
        let stat = fstat(&descriptor).map_err(|errno| failure(&file_path, errno))?;
        validate(&file_path, &stat, FileType::RegularFile)?;
        if stat.st_nlink != 1 {
            return Err(refusal(
                &file_path,
                io::ErrorKind::InvalidInput,
                "lock file has more than one link",
            ));
        }
        let file = File::from(descriptor);
        loop {
            match file.lock() {
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(source) => {
                    return Err(PlacePreparationError::Io {
                        path: file_path,
                        source,
                    })
                }
                Ok(()) => return Ok(Self { _file: file }),
            }
        }
    }
}

/// Move a sole legacy store into the destination, atomically where the
/// platform refuses to replace.
fn move_legacy(
    legacy_parent: &OwnedFd,
    nexus: &OwnedFd,
    legacy_path: &Path,
    name: &str,
) -> Outcome<()> {
    #[cfg(any(target_os = "linux", target_os = "android"))]
    let moved = rustix::fs::renameat_with(
        legacy_parent,
        name,
        nexus,
        name,
        rustix::fs::RenameFlags::NOREPLACE,
    );
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    let moved = rustix::fs::renameat(legacy_parent, name, nexus, name);
    moved.map_err(|errno| failure(legacy_path, errno))
}

/// Prepare the state and runtime of a standard Place under its exclusion.
pub(crate) fn prepare_standard<T, E>(
    state_root: &Path,
    runtime_root: Option<&Path>,
    name: &str,
    runtime_path: &Path,
    operation: impl FnOnce() -> Result<T, E>,
) -> Result<T, crate::placement::Held<E>> {
    let exclusion = take_and_prepare(state_root, runtime_root, name, runtime_path)
        .map_err(crate::placement::Held::Preparation)?;
    let result = operation().map_err(crate::placement::Held::Operation);
    drop(exclusion);
    result
}

/// Prepare the state and runtime of a standard Place and hand back the held
/// exclusion; the caller drops it, on success, on error and on unwind alike.
fn take_and_prepare(
    state_root: &Path,
    runtime_root: Option<&Path>,
    name: &str,
    runtime_path: &Path,
) -> Outcome<Exclusion> {
    let root = acquire_root(state_root)?;
    let nexus_path = state_root.join(NEXUS);
    let nexus = ensure_directory(&root, &nexus_path, NEXUS)?;
    let exclusion = Exclusion::take(&root, state_root, name)?;

    let current_path = nexus_path.join(name);
    let legacy_dir_path = state_root.join(LEGACY);
    let legacy_path = legacy_dir_path.join(name);

    let current_exists = exists(&nexus, &current_path, name)?;
    let legacy_parent = if exists(&root, &legacy_dir_path, LEGACY)? {
        Some(open_directory(&root, &legacy_dir_path, LEGACY)?)
    } else {
        None
    };
    let legacy_exists = match &legacy_parent {
        Some(parent) => exists(parent, &legacy_path, name)?,
        None => false,
    };
    if legacy_exists && current_exists {
        return Err(PlacePreparationError::StateConflict {
            legacy: legacy_path,
            current: current_path,
        });
    }
    if current_exists {
        drop(open_directory(&nexus, &current_path, name)?);
    }
    if legacy_exists {
        let parent = legacy_parent.as_ref().expect("a legacy store has a parent");
        drop(open_directory(parent, &legacy_path, name)?);
        move_legacy(parent, &nexus, &legacy_path, name)?;
    }
    drop(ensure_directory(&nexus, &current_path, name)?);

    match runtime_root {
        Some(runtime_root) => {
            let root = acquire_root(runtime_root)?;
            let nexus_path = runtime_root.join(NEXUS);
            let nexus = ensure_directory(&root, &nexus_path, NEXUS)?;
            drop(ensure_directory(&nexus, &nexus_path.join(name), name)?);
        }
        None => {
            std::fs::create_dir_all(runtime_path).map_err(|source| PlacePreparationError::Io {
                path: runtime_path.to_owned(),
                source,
            })?;
        }
    }
    Ok(exclusion)
}

/// The identity of an existing directory, for refusing an actual conflict
/// between the two plans.
pub(crate) fn identity(path: &Path) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    let metadata = std::fs::metadata(path).ok()?;
    Some((metadata.dev(), metadata.ino()))
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::process::{Command, Stdio};

    fn private_root(label: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!(
            "translator-anchored-{label}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        root
    }

    #[test]
    fn the_lock_is_close_on_exec_and_released_when_the_guard_drops() {
        let root_path = private_root("lock");
        let root = acquire_root(&root_path).expect("acquire a missing private root");
        let exclusion = Exclusion::take(&root, &root_path, "alpha").expect("take the lock");
        let lock_path = root_path.join(LOCKS).join("alpha");

        let mut child = Command::new("sleep")
            .arg("30")
            .stdin(Stdio::null())
            .spawn()
            .expect("spawn a child that outlives the guard");
        let held_by_child = std::fs::read_dir(format!("/proc/{}/fd", child.id()))
            .expect("child descriptors")
            .filter_map(|entry| std::fs::read_link(entry.ok()?.path()).ok())
            .any(|target| target == lock_path);
        assert!(
            !held_by_child,
            "the child must not inherit the lock descriptor"
        );

        let probe = File::options()
            .read(true)
            .write(true)
            .open(&lock_path)
            .expect("open the lock file");
        assert!(
            probe.try_lock().is_err(),
            "the lock is held while the guard lives"
        );
        drop(exclusion);
        probe
            .try_lock()
            .expect("the lock is free once the guard drops, with the child still alive");
        child.kill().expect("stop the child");
        child.wait().expect("reap the child");
        let mode = std::fs::metadata(&lock_path)
            .expect("lock metadata")
            .permissions()
            .mode();
        assert_eq!(mode & 0o077, 0, "the lock file is private");
        let _ = std::fs::remove_dir_all(&root_path);
    }
}
