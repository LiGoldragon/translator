use translator::placement::{
    resolve_place, runtime_path, state_path, validate_metaflow_name, PlaceResolutionError,
};

static ENVIRONMENT: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn fixed_place_uses_runtime_and_home_roots() {
    let _environment = ENVIRONMENT.lock().expect("environment lock");
    std::env::set_var("XDG_RUNTIME_DIR", "/run/user/1000");
    std::env::set_var("HOME", "/home/one");
    let place = resolve_place("alpha").expect("place");
    assert_eq!(place.metaflow_name.0, "alpha");
    assert_eq!(
        runtime_path(&place),
        std::path::Path::new("/run/user/1000/metaflow/alpha")
    );
    assert_eq!(
        state_path(&place),
        std::path::Path::new("/home/one/.local/state/metaflow/alpha")
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
    std::env::set_var("HOME", "/home/one");
    assert_eq!(
        resolve_place("alpha"),
        Err(PlaceResolutionError::MissingEnvironment("XDG_RUNTIME_DIR"))
    );
    std::env::set_var("XDG_RUNTIME_DIR", "relative");
    assert_eq!(
        resolve_place("alpha"),
        Err(PlaceResolutionError::RelativeEnvironment("XDG_RUNTIME_DIR"))
    );
}
