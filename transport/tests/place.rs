//! The place: a datom reads into it, and each socket and file is found under it by the Nexus's name.

use std::path::PathBuf;

use translator_transport::{CliConfiguration, Named, Place, PlaceName, Placed, Runtime, State, actualize};

struct Example;

impl Named for Example {
    const NAME: &'static str = "example";
}

fn place() -> Place {
    Place { place_name: PlaceName("main".into()), runtime: Runtime("/run/user/1000/nexus/main".into()), state: State("/home/example/.local/state/nexus/main".into()) }
}

#[test]
fn a_place_is_read_from_its_datom() {
    let read: Place = actualize("{ main /run/user/1000/nexus/main /home/example/.local/state/nexus/main }").expect("a place");
    assert_eq!(read, place());
}

#[test]
fn the_configuration_is_the_place_name_alone() {
    let read: CliConfiguration = actualize("{ next }").expect("a configuration");
    assert_eq!(read.place_name, PlaceName("next".into()));
}

#[test]
fn a_text_that_is_no_place_is_refused() {
    assert!(actualize::<Place>("{ main }").is_err());
}

#[test]
fn the_sockets_are_named_for_the_nexus_and_in_the_runtime() {
    let place = place();
    let socket = |placed| place.path::<Example>(placed);
    let runtime = |file: &str| PathBuf::from("/run/user/1000/nexus/main").join(file);
    assert_eq!(socket(Placed::NexusSocket), runtime("example.sock"));
    assert_eq!(socket(Placed::MetaSocket), runtime("example-meta.sock"));
    assert_eq!(socket(Placed::TextSocket), runtime("example-translator.sock"));
    assert_eq!(socket(Placed::MetaTextSocket), runtime("example-meta-translator.sock"));
    assert_eq!(socket(Placed::HookSocket), runtime("example-hook.sock"));
    assert_eq!(socket(Placed::EmissionSocket), runtime("example-emission.sock"));
    assert_eq!(socket(Placed::ContractSocket("subscription")), runtime("example-subscription.sock"));
    assert_eq!(socket(Placed::ContractTextSocket("subscription")), runtime("example-subscription-translator.sock"));
    assert_eq!(socket(Placed::Beside("settings.json")), runtime("example-settings.json"));
}

#[test]
fn the_memory_and_the_trace_are_named_for_a_version_and_are_in_the_state() {
    let place = place();
    let state = |file: &str| PathBuf::from("/home/example/.local/state/nexus/main").join(file);
    assert_eq!(place.path::<Example>(Placed::Memory("1.0.0")), state("example-memory-1.0.0.signal"));
    assert_eq!(place.path::<Example>(Placed::Trace("1.0.0")), state("example-trace-1.0.0.signal"));
}
