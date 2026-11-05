use translator::placement::{
    resolve_place, resolve_runtime, runtime_path, state_path, validate_metaflow_name,
    PlaceResolutionError,
};

static ENVIRONMENT: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn fixed_place_uses_runtime_and_xdg_state_roots() {
    let _environment = ENVIRONMENT.lock().expect("environment lock");
    std::env::set_var("XDG_RUNTIME_DIR", "/run/user/1000");
    std::env::set_var("HOME", "/home/one");
    std::env::set_var("XDG_STATE_HOME", "/state/one");
    let place = resolve_place("alpha").expect("place");
    assert_eq!(place.metaflow_name.0, "alpha");
    assert_eq!(
        runtime_path(&place),
        std::path::Path::new("/run/user/1000/metaflow/alpha")
    );
    assert_eq!(
        state_path(&place),
        std::path::Path::new("/state/one/metaflow/alpha")
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
            std::path::Path::new("/home/one/.local/state/metaflow/alpha")
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
        "/run/user/1000/metaflow/alpha"
    );
}

#[test]
fn metaflow_name_is_one_safe_path_component() {
    assert!(validate_metaflow_name("alpha-1").is_ok());
    for name in ["", ".", "..", "a/b", "/a", "a\0b"] {
        assert!(validate_metaflow_name(name).is_err(), "accepted {name:?}");
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
