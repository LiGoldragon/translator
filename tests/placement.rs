use translator::placement::{
    resolve_place, resolve_runtime, runtime_path, state_path, validate_place_name, Place,
    PlaceName, PlacePreparationError, PlaceResolutionError, Preparable, Runtime, State,
};

static ENVIRONMENT: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn fixed_place_uses_runtime_and_xdg_state_roots() {
    let _environment = ENVIRONMENT.lock().expect("environment lock");
    std::env::set_var("XDG_RUNTIME_DIR", "/run/user/1000");
    std::env::set_var("HOME", "/home/one");
    std::env::set_var("XDG_STATE_HOME", "/state/one");
    let place = resolve_place("alpha").expect("place");
    assert_eq!(place.place_name.0, "alpha");
    assert_eq!(
        runtime_path(&place),
        std::path::Path::new("/run/user/1000/nexus/alpha")
    );
    assert_eq!(
        state_path(&place),
        std::path::Path::new("/state/one/nexus/alpha")
    );
}

#[test]
fn state_defaults_to_home_for_unset_empty_or_relative_xdg_state_home() {
    let _environment = ENVIRONMENT.lock().expect("environment lock");
    std::env::set_var("XDG_RUNTIME_DIR", "/run/user/1000");
    std::env::set_var("HOME", "/home/one");
    for state_home in [None, Some(""), Some("relative")] {
        match state_home {
            Some(value) => std::env::set_var("XDG_STATE_HOME", value),
            None => std::env::remove_var("XDG_STATE_HOME"),
        }
        assert_eq!(
            state_path(&resolve_place("alpha").expect("place")),
            std::path::Path::new("/home/one/.local/state/nexus/alpha")
        );
    }
}

#[test]
fn runtime_resolution_has_no_state_environment_dependency() {
    let _environment = ENVIRONMENT.lock().expect("environment lock");
    std::env::set_var("XDG_RUNTIME_DIR", "/run/user/1000");
    std::env::remove_var("HOME");
    std::env::remove_var("XDG_STATE_HOME");
    assert_eq!(
        resolve_runtime("alpha").expect("runtime").0,
        "/run/user/1000/nexus/alpha"
    );
}

#[test]
fn place_name_is_one_safe_path_component() {
    assert!(validate_place_name("alpha-1").is_ok());
    for name in ["", ".", "..", "a/b", "/a", "a\0b"] {
        assert!(validate_place_name(name).is_err(), "accepted {name:?}");
    }
}

#[test]
fn resolver_requires_designated_absolute_environment_roots() {
    let _environment = ENVIRONMENT.lock().expect("environment lock");
    std::env::remove_var("XDG_RUNTIME_DIR");
    assert_eq!(
        resolve_runtime("alpha"),
        Err(PlaceResolutionError::MissingEnvironment("XDG_RUNTIME_DIR"))
    );
    std::env::set_var("XDG_RUNTIME_DIR", "");
    assert_eq!(
        resolve_runtime("alpha"),
        Err(PlaceResolutionError::EmptyEnvironment("XDG_RUNTIME_DIR"))
    );
    std::env::set_var("XDG_RUNTIME_DIR", "relative");
    assert_eq!(
        resolve_runtime("alpha"),
        Err(PlaceResolutionError::RelativeEnvironment("XDG_RUNTIME_DIR"))
    );
    std::env::set_var("XDG_RUNTIME_DIR", "/run/user/1000");
    assert_eq!(
        resolve_runtime("../alpha"),
        Err(PlaceResolutionError::UnsafeName("../alpha".into()))
    );
}

#[cfg(unix)]
#[test]
fn runtime_refuses_a_non_unicode_root() {
    use std::os::unix::ffi::OsStringExt;

    let _environment = ENVIRONMENT.lock().expect("environment lock");
    std::env::set_var(
        "XDG_RUNTIME_DIR",
        std::ffi::OsString::from_vec(vec![b'/', 0xff]),
    );
    assert_eq!(
        resolve_runtime("alpha"),
        Err(PlaceResolutionError::NonUnicodeEnvironment(
            "XDG_RUNTIME_DIR"
        ))
    );
}

struct TestRoot(std::path::PathBuf);

impl TestRoot {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let sequence = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Self(std::env::temp_dir().join(format!(
            "translator-place-{}-{sequence}",
            std::process::id()
        )))
    }
}

impl Drop for TestRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn test_place(root: &std::path::Path) -> Place {
    Place {
        place_name: PlaceName("alpha".into()),
        runtime: Runtime(
            root.join("runtime/nexus/alpha")
                .to_string_lossy()
                .into_owned(),
        ),
        state: State(
            root.join("state/nexus/alpha")
                .to_string_lossy()
                .into_owned(),
        ),
    }
}

#[test]
fn start_preparation_creates_both_new_place_directories() {
    let root = TestRoot::new();
    let place = test_place(&root.0);

    place.prepare_for_start().expect("prepare fresh place");

    assert!(runtime_path(&place).is_dir());
    assert!(state_path(&place).is_dir());
}

#[test]
fn start_preparation_moves_the_sole_legacy_state_store() {
    let root = TestRoot::new();
    let place = test_place(&root.0);
    let legacy = root.0.join("state/metaflow/alpha");
    let old_runtime = root.0.join("runtime/metaflow/alpha");
    std::fs::create_dir_all(&legacy).expect("legacy state");
    std::fs::write(legacy.join("memory"), "kept").expect("legacy memory");
    std::fs::create_dir_all(&old_runtime).expect("legacy runtime");

    place.prepare_for_start().expect("migrate legacy state");

    assert!(!legacy.exists());
    assert_eq!(
        std::fs::read_to_string(state_path(&place).join("memory")).expect("migrated memory"),
        "kept"
    );
    assert!(old_runtime.is_dir(), "legacy runtime stays untouched");
    assert!(runtime_path(&place).is_dir());
}

#[test]
fn start_preparation_refuses_two_state_stores_and_names_both() {
    let root = TestRoot::new();
    let place = test_place(&root.0);
    let legacy = root.0.join("state/metaflow/alpha");
    let current = state_path(&place);
    std::fs::create_dir_all(&legacy).expect("legacy state");
    std::fs::create_dir_all(&current).expect("current state");

    match place.prepare_for_start() {
        Err(PlacePreparationError::StateConflict {
            legacy: found_legacy,
            current: found_current,
        }) => {
            assert_eq!(found_legacy, legacy);
            assert_eq!(found_current, current);
        }
        other => panic!("expected state conflict, got {other:?}"),
    }
    assert!(legacy.is_dir());
    assert!(current.is_dir());
}

#[test]
fn explicit_state_paths_outside_the_standard_relationship_are_not_migrated() {
    let root = TestRoot::new();
    let mut place = test_place(&root.0);
    place.state = State(root.0.join("explicit/state").to_string_lossy().into_owned());
    let lookalike = root.0.join("explicit/metaflow/alpha");
    std::fs::create_dir_all(&lookalike).expect("unrelated legacy-looking state");

    place.prepare_for_start().expect("prepare explicit state");

    assert!(lookalike.is_dir());
    assert!(state_path(&place).is_dir());
}
