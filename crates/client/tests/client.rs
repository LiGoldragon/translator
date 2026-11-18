//! A CLI's text lane: found in its place by the Nexus's name, every answer
//! delivered in order, and a failure answered as the error.

use std::os::unix::net::UnixListener;
use std::path::PathBuf;

use nexus_exchange::lane::{Frame, Kind, Lane, LaneReading, LaneWriting};
use nexus_exchange::{ComponentName, ComponentTitle, Named};
use shared_translator::placement::{Place, PlaceName, Runtime, State};
use translator_client::{ClientError, TextClient, TextLane, TextRequesting, TranslatorPlacing};

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
fn every_answer_is_delivered_in_order_until_the_translator_closes() {
    let directory = fixture("stream");
    let client = TextClient::at::<Probe>(&place(&directory), TextLane::PublicText, LIMIT);
    let served = translator(
        &client,
        Lane::PublicText,
        vec![
            answer(Lane::PublicText, Kind::Answered, "Opened.0"),
            answer(Lane::PublicText, Kind::Answered, "Changed.1"),
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
        vec![answer(Lane::MetaText, Kind::Answered, "Configured")],
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
