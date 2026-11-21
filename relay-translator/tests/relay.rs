//! The Relay Nexus and its translator, run in one process in a place of their
//! own: the text socket and the CLI, the emission that runs a program, the
//! lasting emission that follows a file, and the hook socket.

use std::io::Write;
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use relay_nexus::Nexus;
use relay_signal::signal::{Call, Hook, Input, Output};
use relay_signal::{ETHOS, Emission, Relay};
use relay_translator::RelayTranslator;
use translator::{Calling, Caller, Ending, Framed, Hooking, Named, Place, PlaceName, Placed, Runtime, State, TextSocket, Translator};

fn scratch(name: &str) -> PathBuf {
    let directory = std::env::temp_dir().join(format!("rt-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).expect("a scratch directory");
    directory
}

fn place(directory: &Path) -> Place {
    let text = directory.to_string_lossy().into_owned();
    Place { place_name: PlaceName("main".into()), runtime: Runtime(text.clone()), state: State(text) }
}

fn nexus(place: &Place) {
    place.prepare().expect("prepared");
    let listener = UnixListener::bind(place.path::<Relay>(Placed::NexusSocket)).expect("bound");
    let nexus = Nexus::new(place);
    thread::spawn(move || nexus.listen(listener));
}

fn translator(place: &Place) -> Option<Output> {
    let mut translator = Translator::<RelayTranslator>::new(place.clone());
    translator.text::<Input, Call, Output>(Placed::TextSocket, Placed::NexusSocket, ETHOS, |caller, input| Call { caller, input }).expect("text");
    translator.hooks::<Hook, Call, Output>(|caller, hook| Call { caller, input: Input::Report(hook) }).expect("hooks");
    translator.emissions::<Emission>().expect("emissions");
    let caller = Caller::itself().expect("itself");
    let acknowledged: Option<Output> = translator.listening(&Call { caller, input: Input::Listening });
    thread::spawn(move || translator.serve());
    acknowledged
}

/// Both started in the place, the Nexus first, as the translator tells it that it listens.
fn started(name: &str) -> (Place, PathBuf) {
    let directory = scratch(name);
    let place = place(&directory);
    nexus(&place);
    assert_eq!(translator(&place), Some(Output::Acknowledged));
    (place, directory)
}

fn call(place: &Place, datom: &str) -> (Ending, String) {
    let mut out = Vec::new();
    let ending = RelayTranslator::exchange(place, TextSocket::Ordinary, datom, &mut out);
    (ending, String::from_utf8(out).expect("text"))
}

/// What the Nexus has seen, asked until it is what is wanted or the time is up.
fn seen_until(place: &Place, wanted: &str) -> String {
    let until = Instant::now() + Duration::from_secs(10);
    loop {
        let (_, seen) = call(place, "Seen");
        if seen.contains(wanted) || Instant::now() > until {
            return seen;
        }
        thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn the_name_of_the_translator_is_the_name_of_the_nexus() {
    assert_eq!(RelayTranslator::NAME, "relay");
}

#[test]
fn a_datom_typed_at_the_cli_is_answered_as_a_datom() {
    let (place, _) = started("text");
    assert_eq!(call(&place, "Seen"), (Ending::Answered, "Noticed.[]\n".to_owned()));
}

#[test]
fn a_cli_called_with_no_datom_prints_the_ethos_of_the_signal() {
    let (place, _) = started("describe");
    assert_eq!(call(&place, ""), (Ending::Answered, ETHOS.to_owned()));
}

#[test]
fn a_text_that_is_no_input_is_unreadable_and_the_cli_fails() {
    let (place, _) = started("unreadable");
    assert_eq!(call(&place, "Nonsense"), (Ending::Failed, "Unreadable\n".to_owned()));
}

#[test]
fn a_cli_that_finds_no_translator_fails() {
    let directory = scratch("down");
    assert_eq!(call(&place(&directory), "Seen"), (Ending::Failed, String::new()));
}

#[test]
fn a_translator_whose_nexus_is_down_says_so_and_the_cli_fails() {
    let directory = scratch("nexus-down");
    let place = place(&directory);
    place.prepare().expect("prepared");
    assert_eq!(translator(&place), None);
    assert_eq!(call(&place, "Seen"), (Ending::Failed, "NexusDown\n".to_owned()));
}

#[test]
fn an_emission_runs_a_program_and_its_outcome_is_the_answer() {
    let (place, _) = started("run");
    assert_eq!(call(&place, "Run.{ «/bin/sh» [ «-c» «echo hello» ] }"), (Ending::Answered, "Ran.Done.«hello\n»\n".to_owned()));
    assert_eq!(call(&place, "Run.{ «/bin/sh» [ «-c» «exit 3» ] }"), (Ending::Answered, "Ran.Refused.3\n".to_owned()));
}

#[test]
fn an_emission_that_finds_no_translator_is_unsubmitted() {
    let directory = scratch("unsubmitted");
    let place = place(&directory);
    nexus(&place);
    // No translator listens for the Nexus's emission, so the Nexus is asked directly.
    let mut stream = std::os::unix::net::UnixStream::connect(place.path::<Relay>(Placed::NexusSocket)).expect("the Nexus");
    let command = relay_signal::emission::Command { program: relay_signal::emission::Program("/bin/sh".into()), argument_vector: vec![] };
    Call { caller: Caller::itself().expect("itself"), input: Input::Run(command) }.write_to(&mut stream).expect("written");
    let output = Output::read_from(&mut stream).expect("read").expect("an output");
    assert_eq!(output, Output::Unobserved(translator::Unobserved::Unsubmitted));
}

#[test]
fn the_hook_socket_reports_each_payload_to_the_nexus() {
    let (place, _) = started("hook");
    RelayTranslator::report(&place, b"Started alpha");
    RelayTranslator::report(&place, b"Ended alpha");
    RelayTranslator::report(&place, b"Nonsense alpha");
    assert_eq!(call(&place, "Seen"), (Ending::Answered, "Noticed.[ Hooked.Started.alpha Hooked.Ended.alpha ]\n".to_owned()));
}

#[test]
fn a_lasting_emission_follows_a_file_as_it_grows() {
    let (place, directory) = started("follow");
    let file = directory.join("lines").join("transcript");
    let datom = format!("Follow.{{ «{}» 0 16 }}", file.display());
    // The file is not yet written, nor is its directory: it is waited for.
    assert_eq!(call(&place, &datom), (Ending::Answered, "Following\n".to_owned()));
    std::fs::create_dir(file.parent().expect("a directory")).expect("created");
    std::fs::write(&file, "first\nsecond\n").expect("written");
    assert_eq!(seen_until(&place, "Lined.second"), "Noticed.[ Lined.first Lined.second ]\n");
    std::fs::OpenOptions::new().append(true).open(&file).expect("opened").write_all(b"third\n").expect("appended");
    assert_eq!(seen_until(&place, "Lined.third"), "Noticed.[ Lined.first Lined.second Lined.third ]\n");
}

#[test]
fn a_followed_path_with_no_directory_at_all_ends_the_emission_and_the_nexus_remembers_nothing() {
    let (place, _) = started("lost");
    assert_eq!(call(&place, "Follow.{ «nowhere/at/all» 0 16 }"), (Ending::Answered, "Following\n".to_owned()));
    thread::sleep(Duration::from_millis(100));
    assert_eq!(call(&place, "Seen"), (Ending::Answered, "Noticed.[]\n".to_owned()));
}
