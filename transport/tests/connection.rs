//! The framing of Signal on a stream, the caller of a connection, and the
//! emission socket as the Nexus holds it.

use std::io::Cursor;
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::thread;

use translator_transport::{Caller, EmissionSocket, Framed, MAXIMUM_FRAME, Named, Place, PlaceName, Placed, Runtime, State, Unobserved};

struct Example;

impl Named for Example {
    const NAME: &'static str = "example";
}

#[derive(Clone, Debug, PartialEq, Eq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
enum Emission {
    Ask(String),
    Watch(i64),
}

#[derive(Clone, Debug, PartialEq, Eq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
enum Outcome {
    Told(String),
    Seen(i64),
}

fn scratch(name: &str) -> PathBuf {
    let directory = std::env::temp_dir().join(format!("tt-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).expect("a scratch directory");
    directory
}

fn place(directory: &std::path::Path) -> Place {
    Place { place_name: PlaceName("main".into()), runtime: Runtime(directory.to_string_lossy().into_owned()), state: State(directory.to_string_lossy().into_owned()) }
}

#[test]
fn a_frame_is_its_length_and_its_archive_and_reads_back() {
    let mut bytes = Vec::new();
    Emission::Ask("which".into()).write_to(&mut bytes).expect("written");
    Emission::Watch(7).write_to(&mut bytes).expect("written");
    let mut stream = Cursor::new(bytes);
    assert_eq!(Emission::read_from(&mut stream).expect("read"), Some(Emission::Ask("which".into())));
    assert_eq!(Emission::read_from(&mut stream).expect("read"), Some(Emission::Watch(7)));
    assert_eq!(Emission::read_from(&mut stream).expect("a stream that ended"), None);
}

#[test]
fn a_length_over_the_maximum_is_refused_before_it_is_read() {
    let mut stream = Cursor::new((MAXIMUM_FRAME + 1).to_le_bytes().to_vec());
    assert!(Emission::read_from(&mut stream).is_err());
}

#[test]
fn bytes_that_are_no_archive_are_refused() {
    let mut bytes = 3u32.to_le_bytes().to_vec();
    bytes.extend_from_slice(&[255, 255, 255]);
    assert!(Emission::read_from(&mut Cursor::new(bytes)).is_err());
}

#[test]
fn the_caller_of_a_connection_is_this_process_and_its_user() {
    let directory = scratch("caller");
    let listener = UnixListener::bind(directory.join("s")).expect("bound");
    let client = std::os::unix::net::UnixStream::connect(directory.join("s")).expect("connected");
    let (accepted, _) = listener.accept().expect("accepted");
    let caller = Caller::of(&accepted).expect("credentials");
    assert_eq!(caller, Caller::itself().expect("itself"));
    drop(client);
}

#[test]
fn an_emission_that_finds_no_translator_is_unsubmitted() {
    let directory = scratch("none");
    let socket = EmissionSocket::of::<Example>(&place(&directory));
    assert_eq!(socket.send::<Emission, Outcome>(&Emission::Watch(1)), Err(Unobserved::Unsubmitted));
    assert!(matches!(socket.hold::<Emission, Outcome>(&Emission::Watch(1)), Err(Unobserved::Unsubmitted)));
}

#[test]
fn an_emission_and_its_outcome_are_one_connection() {
    let directory = scratch("send");
    let place = place(&directory);
    let listener = UnixListener::bind(place.path::<Example>(Placed::EmissionSocket)).expect("bound");
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accepted");
        if let Some(Emission::Ask(question)) = Emission::read_from(&mut stream).expect("read") {
            Outcome::Told(format!("{question}!")).write_to(&mut stream).expect("written");
        }
    });
    let socket = EmissionSocket::of::<Example>(&place);
    assert_eq!(socket.send::<Emission, Outcome>(&Emission::Ask("which".into())), Ok(Outcome::Told("which!".into())));
}

#[test]
fn a_connection_that_closes_before_the_outcome_is_unanswered_and_after_one_the_next_is() {
    let directory = scratch("close");
    let place = place(&directory);
    let listener = UnixListener::bind(place.path::<Example>(Placed::EmissionSocket)).expect("bound");
    thread::spawn(move || {
        let (mut first, _) = listener.accept().expect("accepted");
        let _ = Emission::read_from(&mut first);
        drop(first);
        let (mut second, _) = listener.accept().expect("accepted");
        let _ = Emission::read_from(&mut second);
        Outcome::Seen(1).write_to(&mut second).expect("written");
        Outcome::Seen(2).write_to(&mut second).expect("written");
    });
    let socket = EmissionSocket::of::<Example>(&place);
    assert_eq!(socket.send::<Emission, Outcome>(&Emission::Watch(1)), Err(Unobserved::Unanswered));
    let mut held = socket.hold::<Emission, Outcome>(&Emission::Watch(2)).expect("held");
    assert_eq!(held.outcome(), Ok(Outcome::Seen(1)));
    assert_eq!(held.outcome(), Ok(Outcome::Seen(2)));
    assert_eq!(held.outcome(), Err(Unobserved::Unanswered));
}
