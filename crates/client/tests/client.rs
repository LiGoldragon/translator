//! A CLI's text lane: found in its place by the Nexus's name, every answer
//! delivered in order until the translator ends them, and a failure answered
//! as the error. A CLI's place is its configuration's, and a configuration
//! that cannot be found, read or understood is refused naming the file.

use std::os::unix::net::UnixListener;
use std::path::PathBuf;

use nexus_exchange::lane::{Frame, Kind, Lane, LaneReading, LaneWriting};
use nexus_exchange::{ComponentName, ComponentTitle, Named};
use shared_translator::placement::{Place, PlaceName, Runtime, State};
use translator_client::{
    ClientConfiguration, ClientError, ConfigurationFault, ConfigurationFile, ConfigurationLocating,
    ConfigurationPath, ConfigurationReading, ConfigurationRefusal, InvocationRefusal, Invoking,
    RefusalReporting, TextClient, TextInvocation, TextLane, TextRequesting, TranslatorPlacing,
};

const LIMIT: usize = 4096;

struct Probe;

impl Named for Probe {
    fn name() -> ComponentName {
        ComponentName("probe".into())
    }

    fn title() -> ComponentTitle {
        ComponentTitle("Probe Nexus".into())
    }
}

fn place(runtime: &std::path::Path) -> Place {
    Place {
        place_name: PlaceName("probe".into()),
        runtime: Runtime(runtime.to_string_lossy().into_owned()),
        state: State(runtime.to_string_lossy().into_owned()),
    }
}

fn fixture(name: &str) -> PathBuf {
    let directory =
        std::env::temp_dir().join(format!("translator-client-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).expect("fixture directory");
    directory
}

fn answer(lane: Lane, kind: Kind, text: &str) -> Frame {
    Frame {
        lane,
        kind,
        payload: text.as_bytes().to_vec(),
    }
}

/// A translator that reads one request on `lane` and writes `answers`.
fn translator(
    client: &TextClient,
    lane: Lane,
    answers: Vec<Frame>,
) -> std::thread::JoinHandle<String> {
    let listener = UnixListener::bind(&client.socket).expect("translator socket");
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("client connection");
        let request = stream.read_lane_frame(LIMIT).expect("request frame");
        assert_eq!(request.lane, lane);
        assert_eq!(request.kind, Kind::Request);
        for frame in answers {
            stream
                .write_lane_frame(&frame, LIMIT)
                .expect("answer frame");
        }
        String::from_utf8(request.payload).expect("request text")
    })
}

#[test]
fn a_lane_is_found_in_its_place_by_the_nexus_name() {
    let runtime = std::path::Path::new("/run/user/1000/nexus/probe");
    let public = TextClient::at::<Probe>(&place(runtime), TextLane::PublicText, LIMIT);
    let meta = TextClient::at::<Probe>(&place(runtime), TextLane::MetaText, LIMIT);
    assert_eq!(public.socket, runtime.join("probe-translator.sock"));
    assert_eq!(meta.socket, runtime.join("probe-meta-translator.sock"));
}

#[test]
fn every_answer_is_delivered_in_order_until_the_translator_ends_them() {
    let directory = fixture("stream");
    let client = TextClient::at::<Probe>(&place(&directory), TextLane::PublicText, LIMIT);
    let served = translator(
        &client,
        Lane::PublicText,
        vec![
            answer(Lane::PublicText, Kind::Answered, "Opened.0"),
            answer(Lane::PublicText, Kind::Answered, "Changed.1"),
            answer(Lane::PublicText, Kind::Ended, ""),
        ],
    );
    let mut answers = Vec::new();
    client
        .request_stream("Follow", |answer| {
            answers.push(answer.to_owned());
            true
        })
        .expect("followed");
    assert_eq!(served.join().expect("translator"), "Follow");
    assert_eq!(answers, ["Opened.0", "Changed.1"]);
    std::fs::remove_dir_all(directory).expect("fixture cleanup");
}

#[test]
fn a_meta_request_returns_its_first_answer() {
    let directory = fixture("meta");
    let client = TextClient::at::<Probe>(&place(&directory), TextLane::MetaText, LIMIT);
    let served = translator(
        &client,
        Lane::MetaText,
        vec![
            answer(Lane::MetaText, Kind::Answered, "Configured"),
            answer(Lane::MetaText, Kind::Ended, ""),
        ],
    );
    assert_eq!(
        client.request("Configure.1").expect("answered"),
        "Configured"
    );
    served.join().expect("translator");
    std::fs::remove_dir_all(directory).expect("fixture cleanup");
}

#[test]
fn a_failed_answer_is_the_error_and_carries_its_text() {
    let directory = fixture("failed");
    let client = TextClient::at::<Probe>(&place(&directory), TextLane::PublicText, LIMIT);
    let served = translator(
        &client,
        Lane::PublicText,
        vec![answer(
            Lane::PublicText,
            Kind::Failed,
            "translator is awaiting settings",
        )],
    );
    let failure = client.request("Count.1").expect_err("failed");
    assert!(
        matches!(&failure, ClientError::Protocol(text) if text == "translator is awaiting settings"),
        "{failure}"
    );
    served.join().expect("translator");
    std::fs::remove_dir_all(directory).expect("fixture cleanup");
}

#[test]
fn a_lane_closed_before_its_end_is_broken() {
    let directory = fixture("unended");
    let client = TextClient::at::<Probe>(&place(&directory), TextLane::PublicText, LIMIT);
    let served = translator(
        &client,
        Lane::PublicText,
        vec![answer(Lane::PublicText, Kind::Answered, "Opened.0")],
    );
    let mut answers = Vec::new();
    let broken = client
        .request_stream("Follow", |answer| {
            answers.push(answer.to_owned());
            true
        })
        .expect_err("no end was read");
    assert!(matches!(broken, ClientError::Transport(_)), "{broken}");
    assert_eq!(answers, ["Opened.0"]);
    served.join().expect("translator");
    std::fs::remove_dir_all(directory).expect("fixture cleanup");
}

#[test]
fn the_configuration_lies_under_the_configuration_home_else_under_home() {
    use std::ffi::OsStr;
    use std::path::Path;
    assert_eq!(
        ConfigurationFile::under::<Probe>(Some(OsStr::new("/c")), Some(OsStr::new("/home/li")))
            .expect("located")
            .path,
        Path::new("/c/nexus/probe.datom")
    );
    for unset in [None, Some(OsStr::new(""))] {
        assert_eq!(
            ConfigurationFile::under::<Probe>(unset, Some(OsStr::new("/home/li")))
                .expect("located")
                .path,
            Path::new("/home/li/.config/nexus/probe.datom")
        );
    }
    for (configuration_home, home) in [
        (None, None),
        (Some(OsStr::new("c")), Some(OsStr::new("/home/li"))),
        (None, Some(OsStr::new("home/li"))),
    ] {
        assert!(matches!(
            ConfigurationFile::under::<Probe>(configuration_home, home),
            Err(ConfigurationRefusal::Unlocated(_))
        ));
    }
}

/// A configuration home of the test's own, its `nexus` directory made.
fn configuration_home(name: &str) -> (PathBuf, ConfigurationFile) {
    let home = fixture(name);
    std::fs::create_dir_all(home.join("nexus")).expect("configuration directory");
    let file = ConfigurationFile::under::<Probe>(Some(home.as_os_str()), None).expect("located");
    (home, file)
}

#[test]
fn a_configuration_missing_unreadable_or_invalid_is_refused_naming_its_file() {
    let (home, file) = configuration_home("refusals");
    let path = ConfigurationPath(file.path.to_string_lossy().into_owned());
    assert_eq!(
        file.read(),
        Err(ConfigurationRefusal::Missing(path.clone()))
    );
    let printed = InvocationRefusal::Configuration(ConfigurationRefusal::Missing(path.clone()))
        .text::<Probe>();
    assert!(
        printed.starts_with("Missing.") && printed.contains(&path.0),
        "{printed}"
    );
    assert_eq!(InvocationRefusal::EXIT, 2);
    for invalid in ["{ next main }", "{ .. }", "{ }", ""] {
        std::fs::write(&file.path, invalid).expect("configuration");
        assert!(
            matches!(
                file.read(),
                Err(ConfigurationRefusal::Invalid(ConfigurationFault { configuration_path, .. }))
                    if configuration_path == path
            ),
            "{invalid:?}: {:?}",
            file.read()
        );
    }
    std::fs::remove_file(&file.path).expect("configuration removed");
    std::fs::create_dir(&file.path).expect("a directory in its place");
    let unreadable = file.read();
    assert!(
        matches!(
            &unreadable,
            Err(ConfigurationRefusal::Unreadable(ConfigurationFault { configuration_path, .. }))
                if *configuration_path == path
        ),
        "{unreadable:?}"
    );
    let printed =
        InvocationRefusal::Configuration(unreadable.expect_err("unreadable")).text::<Probe>();
    assert!(
        printed.starts_with("Unreadable.") && printed.contains(&path.0),
        "{printed}"
    );
    let refused = TextInvocation::from_arguments::<Probe>(
        TextLane::PublicText,
        ["Count.1".to_owned()].into_iter(),
        &file,
    )
    .expect_err("nothing is sent");
    assert!(matches!(
        refused,
        InvocationRefusal::Configuration(ConfigurationRefusal::Unreadable(_))
    ));
    std::fs::remove_dir_all(home).expect("fixture cleanup");
}

#[test]
fn a_cli_takes_its_one_datom_alone() {
    let (home, file) = configuration_home("usage");
    std::fs::write(&file.path, "{ next }").expect("configuration");
    for arguments in [vec![], vec!["next", "Count.1"]] {
        let refused = TextInvocation::from_arguments::<Probe>(
            TextLane::PublicText,
            arguments.into_iter().map(str::to_owned),
            &file,
        )
        .expect_err("not one datom");
        assert!(matches!(refused, InvocationRefusal::Usage), "{refused:?}");
    }
    std::fs::remove_dir_all(home).expect("fixture cleanup");
}

/// The variable that marks this test binary run again as the child of
/// [`the_configured_place_resolves_under_the_runtime_root`].
const CHILD: &str = "TRANSLATOR_CLIENT_PLACE_CHILD";

#[test]
fn the_configured_place_resolves_under_the_runtime_root() {
    let (home, file) = configuration_home("place");
    std::fs::write(&file.path, "{ next }\n").expect("configuration");
    assert_eq!(
        file.read(),
        Ok(ClientConfiguration {
            place_name: shared_translator::PlaceName("next".into())
        })
    );
    // The runtime root is the process's environment, which a test sets
    // only for a process of its own: this binary again, running the child.
    let status = std::process::Command::new(std::env::current_exe().expect("test binary"))
        .args([
            "--exact",
            "the_configured_place_resolved_in_a_child",
            "--ignored",
        ])
        .env(CHILD, &file.path)
        .env("XDG_RUNTIME_DIR", "/run/user/1001")
        .env("XDG_STATE_HOME", "/state")
        .status()
        .expect("child");
    assert!(status.success(), "{status}");
    std::fs::remove_dir_all(home).expect("fixture cleanup");
}

#[test]
#[ignore = "run by the_configured_place_resolves_under_the_runtime_root"]
fn the_configured_place_resolved_in_a_child() {
    let path = std::env::var_os(CHILD).expect("run as the child");
    let invocation = TextInvocation::from_arguments::<Probe>(
        TextLane::MetaText,
        ["Configure.1".to_owned()].into_iter(),
        &ConfigurationFile { path: path.into() },
    )
    .expect("placed");
    assert_eq!(invocation.text, "Configure.1");
    assert_eq!(
        invocation.client.socket,
        std::path::Path::new("/run/user/1001/nexus/next/probe-meta-translator.sock")
    );
}
