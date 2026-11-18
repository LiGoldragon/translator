//! A translator that writes only its bodies is served whole: ordinary text is
//! carried to its Nexus with the Caller and answered as text, and each
//! refusal is answered before anything is read.

#![cfg(feature = "text")]

use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

use nexus_exchange::lane::{Frame, Kind, Lane, LaneReading, LaneWriting};
use nexus_exchange::{ComponentName, ComponentTitle, Named, ValidAdmission};
use nexus_serving::{OwnedListener, SocketBinding};
use shared_translator::placement::{Place, PlaceName, Runtime, State};
use shared_translator::{Caller, PeerIdentity, ProcessId, UserId};
use signal::{
    Answer as Answered, ByteViewable, Contracted, Delivery, Dispatch, FrameCapacity, FrameReading,
    FrameWriting, Restorable, Signal, Signalizable,
};
use translator_serving::{
    Answers, Connection, ConnectionServing, DepthLimit, EmissionAnswering, Endpoint,
    MetaTranslating, NexusFailure, NodeLimit, PairReading, PairedPeer, Pairing, PeerMatching,
    PublicTranslating, Settling, TextBudgeting, TextLimits, Translator, ValidTextBudget,
};

const LIMIT: usize = 4096;

#[derive(
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
    datom_codec::Datomizable,
    datom_codec::Composing,
    Clone,
    Debug,
    PartialEq,
)]
enum Query {
    Count(i64),
    Follow,
}

#[derive(
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
    datom_codec::Datomizable,
    datom_codec::Composing,
    Clone,
    Debug,
    PartialEq,
)]
enum Reply {
    Counted(i64),
}

#[derive(rkyv::Archive, rkyv::Serialize, rkyv::Deserialize, Clone, Debug, PartialEq)]
struct Call {
    caller: Caller,
    query: Query,
}

impl Contracted for Call {
    const CONTRACT_SOURCE: &'static str = "Signal [] [ Call.{ Caller Query } ] [] []";
}

#[derive(rkyv::Archive, rkyv::Serialize, rkyv::Deserialize, Clone, Debug, PartialEq)]
enum Response {
    Answered(Reply),
    Failed(String),
}

#[derive(rkyv::Archive, rkyv::Serialize, rkyv::Deserialize, Clone, Debug, PartialEq)]
enum Ping {
    Ping,
}

#[derive(rkyv::Archive, rkyv::Serialize, rkyv::Deserialize, Clone, Debug, PartialEq)]
enum Pong {
    Pong,
}

struct Counter;

impl Named for Counter {
    fn name() -> ComponentName {
        ComponentName("counter".into())
    }

    fn title() -> ComponentTitle {
        ComponentTitle("Counter Nexus".into())
    }
}

/// The settings of a counter's translator, every one set.
struct Ready;

impl TextBudgeting for Ready {
    fn text_budget(&self) -> ValidTextBudget {
        ValidTextBudget {
            nodes: 64,
            depth: 8,
        }
    }
}

/// A counter's translator; `settled` is whether its own settings are set.
struct CounterTranslator {
    settled: bool,
}

impl Translator for CounterTranslator {
    type Component = Counter;
    type Ready = Ready;

    fn ready(&self) -> Option<Ready> {
        self.settled.then_some(Ready)
    }
}

impl PublicTranslating for CounterTranslator {
    type PublicInput = Query;
    type PublicOutput = Reply;
    type PublicCall = Call;
    type PublicResponse = Response;

    fn answers(input: &Query) -> Answers {
        match input {
            Query::Count(_) => Answers::Once,
            Query::Follow => Answers::Followed,
        }
    }

    fn public_call(caller: Caller, query: Query) -> Call {
        Call { caller, query }
    }

    fn public_answered(response: Response) -> Result<Reply, NexusFailure> {
        match response {
            Response::Answered(reply) => Ok(reply),
            Response::Failed(message) => Err(NexusFailure(message)),
        }
    }
}

impl MetaTranslating for CounterTranslator {
    type MetaInput = Query;
    type MetaOutput = Reply;
    type NexusMetaInput = Query;
    type MetaCall = Call;
    type MetaResponse = Response;

    fn settle(&self, input: Query) -> Settling<Reply, Query> {
        match input {
            Query::Count(count) => Settling::Answered(Reply::Counted(count)),
            follow => Settling::Forwarded(follow),
        }
    }

    fn meta_call(caller: Caller, query: Query) -> Call {
        Call { caller, query }
    }

    fn meta_answered(response: Response) -> Result<Reply, NexusFailure> {
        Self::public_answered(response)
    }
}

impl EmissionAnswering for CounterTranslator {
    type Emission = Ping;
    type Outcome = Pong;

    fn emitted(&self, _ready: &Ready, _emission: Ping) -> Pong {
        Pong::Pong
    }
}

fn place(runtime: &Path) -> Place {
    Place {
        place_name: PlaceName("counter".into()),
        runtime: Runtime(runtime.to_string_lossy().into_owned()),
        state: State(runtime.to_string_lossy().into_owned()),
    }
}

fn fixture(name: &str) -> PathBuf {
    let directory =
        std::env::temp_dir().join(format!("translator-serving-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).expect("fixture directory");
    directory
}

fn admission() -> ValidAdmission {
    ValidAdmission {
        deadline: Duration::from_secs(1),
        frame_capacity: LIMIT,
        retry_delay: Duration::from_millis(5),
    }
}

/// Serves one request on `lane` to `endpoint` and returns every frame it is
/// answered with.
fn served(
    translator: &CounterTranslator,
    runtime: &Path,
    endpoint: Endpoint,
    nexus: Option<&PairedPeer>,
    lane: Lane,
    payload: &[u8],
) -> Vec<Frame> {
    let (mut client, mut server) = UnixStream::pair().expect("socket pair");
    client
        .write_lane_frame(
            &Frame {
                lane,
                kind: Kind::Request,
                payload: payload.to_vec(),
            },
            LIMIT,
        )
        .expect("request frame");
    let place = place(runtime);
    Connection {
        translator,
        place: &place,
        endpoint,
        admission: admission(),
        bootstrap_budget: ValidTextBudget {
            nodes: 64,
            depth: 8,
        },
        nexus,
    }
    .serve(&mut server)
    .expect("request served");
    drop(server);
    let mut frames = Vec::new();
    while let Ok(frame) = client.read_lane_frame(LIMIT) {
        frames.push(frame);
    }
    frames
}

/// The one frame a request is answered with.
fn only(mut frames: Vec<Frame>) -> Frame {
    assert_eq!(frames.len(), 1, "{frames:?}");
    frames.remove(0)
}

/// A Nexus at `runtime` that greets one connection, checks that its one
/// exchange carries `expected` from this process, answers it with each of
/// `responses` and closes.
fn nexus(runtime: &Path, expected: Query, responses: Vec<Response>) -> std::thread::JoinHandle<()> {
    let listener = UnixListener::bind(runtime.join("counter.sock")).expect("Nexus socket");
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("translator connection");
        let capacity = FrameCapacity::from(LIMIT);
        let read = |stream: &mut UnixStream| {
            Signal::<Dispatch<Call>>::from(Vec::from(stream.read_frame(capacity).expect("frame")))
                .restore()
                .expect("dispatch")
        };
        let Dispatch::Greet(greeting) = read(&mut stream) else {
            panic!("the translator greets first");
        };
        stream
            .write_frame(
                &Delivery::<Response>::Greeted(Call::receipt(&greeting))
                    .signalize()
                    .expect("receipt"),
                capacity,
            )
            .expect("receipt frame");
        let Dispatch::Open(opening) = read(&mut stream) else {
            panic!("the translator opens an exchange");
        };
        assert_eq!(opening.query.query, expected);
        assert_eq!(
            opening.query.caller.process_id,
            ProcessId(i64::from(std::process::id()))
        );
        for response in responses {
            stream
                .write_frame(
                    &Delivery::Answer(Answered {
                        exchange: opening.exchange,
                        response,
                    })
                    .signalize()
                    .expect("answer"),
                    capacity,
                )
                .expect("answer frame");
        }
    })
}

fn refusal(frame: &Frame) -> &str {
    assert_eq!(frame.kind, Kind::Failed);
    std::str::from_utf8(&frame.payload).expect("refusal text")
}

/// A process of this login that is not this one, alive until it is killed.
fn other_process() -> std::process::Child {
    std::process::Command::new("sleep")
        .arg("30")
        .spawn()
        .expect("fixture process")
}

fn identity(process: &std::process::Child) -> PeerIdentity {
    PeerIdentity {
        user_id: UserId(i64::from(nix::unistd::getuid().as_raw())),
        process_id: ProcessId(i64::from(process.id())),
    }
}

#[test]
fn a_query_is_carried_with_its_caller_and_answered_as_text() {
    let directory = fixture("once");
    let answering = nexus(
        &directory,
        Query::Count(3),
        vec![Response::Answered(Reply::Counted(6))],
    );
    let answer = only(served(
        &CounterTranslator { settled: true },
        &directory,
        Endpoint::Public,
        None,
        Lane::PublicText,
        b"Count.3",
    ));
    answering.join().expect("Nexus");
    assert_eq!(answer.kind, Kind::Answered);
    assert_eq!(answer.payload, b"Counted.6");
    std::fs::remove_dir_all(directory).expect("fixture cleanup");
}

#[test]
fn a_followed_query_is_answered_with_every_answer_until_the_nexus_closes() {
    let directory = fixture("followed");
    let answering = nexus(
        &directory,
        Query::Follow,
        vec![
            Response::Answered(Reply::Counted(1)),
            Response::Answered(Reply::Counted(2)),
        ],
    );
    let answers = served(
        &CounterTranslator { settled: true },
        &directory,
        Endpoint::Public,
        None,
        Lane::PublicText,
        b"Follow",
    );
    answering.join().expect("Nexus");
    let texts: Vec<_> = answers
        .iter()
        .map(|answer| {
            assert_eq!(answer.kind, Kind::Answered);
            std::str::from_utf8(&answer.payload).expect("answer text")
        })
        .collect();
    assert_eq!(texts, ["Counted.1", "Counted.2"]);
    std::fs::remove_dir_all(directory).expect("fixture cleanup");
}

#[test]
fn the_nexuss_failure_is_answered_as_a_failure() {
    let directory = fixture("failure");
    let answering = nexus(
        &directory,
        Query::Count(3),
        vec![Response::Failed("no such count".into())],
    );
    let answer = only(served(
        &CounterTranslator { settled: true },
        &directory,
        Endpoint::Public,
        None,
        Lane::PublicText,
        b"Count.3",
    ));
    answering.join().expect("Nexus");
    assert_eq!(refusal(&answer), "Signal(\"no such count\")");
    std::fs::remove_dir_all(directory).expect("fixture cleanup");
}

#[test]
fn an_emission_from_the_paired_nexus_is_answered() {
    let nexus = PairedPeer::capture(PeerIdentity {
        user_id: UserId(i64::from(nix::unistd::getuid().as_raw())),
        process_id: ProcessId(i64::from(std::process::id())),
    })
    .expect("paired Nexus");
    let emission = Ping::Ping.signalize().expect("emission");
    let answer = only(served(
        &CounterTranslator { settled: true },
        Path::new("/nonexistent"),
        Endpoint::Public,
        Some(&nexus),
        Lane::EmissionSignal,
        emission.bytes(),
    ));
    assert_eq!(answer.kind, Kind::Answered);
    assert_eq!(
        Signal::<Pong>::from(answer.payload)
            .restore()
            .expect("outcome"),
        Pong::Pong
    );
}

#[test]
fn ordinary_text_waits_for_the_translators_own_settings() {
    let answer = only(served(
        &CounterTranslator { settled: false },
        Path::new("/nonexistent"),
        Endpoint::Public,
        None,
        Lane::PublicText,
        b"Count.3",
    ));
    assert_eq!(refusal(&answer), "translator is awaiting settings");
}

#[test]
fn meta_text_is_settled_by_the_translator_before_its_settings_are() {
    let answer = only(served(
        &CounterTranslator { settled: false },
        Path::new("/nonexistent"),
        Endpoint::Meta,
        None,
        Lane::MetaText,
        b"Count.4",
    ));
    assert_eq!(answer.kind, Kind::Answered);
    assert_eq!(answer.payload, b"Counted.4");
}

#[test]
fn each_socket_carries_only_its_own_lanes() {
    let translator = CounterTranslator { settled: true };
    let meta_on_public = only(served(
        &translator,
        Path::new("/nonexistent"),
        Endpoint::Public,
        None,
        Lane::MetaText,
        b"Count.4",
    ));
    assert_eq!(
        refusal(&meta_on_public),
        "lane is unavailable on this endpoint"
    );
    let public_on_meta = only(served(
        &translator,
        Path::new("/nonexistent"),
        Endpoint::Meta,
        None,
        Lane::PublicText,
        b"Count.4",
    ));
    assert_eq!(
        refusal(&public_on_meta),
        "lane is unavailable on this endpoint"
    );
}

#[test]
fn an_emission_from_any_process_but_the_paired_nexus_is_refused() {
    let mut other = other_process();
    let nexus = PairedPeer::capture(identity(&other)).expect("paired Nexus");
    let answer = only(served(
        &CounterTranslator { settled: true },
        Path::new("/nonexistent"),
        Endpoint::Public,
        Some(&nexus),
        Lane::EmissionSignal,
        b"not decoded",
    ));
    assert_eq!(refusal(&answer), "caller is not the paired Nexus");
    other.kill().expect("end fixture process");
    other.wait().expect("reap fixture process");
}

#[test]
fn an_exited_paired_nexus_is_no_longer_matched() {
    let mut other = other_process();
    let identity = identity(&other);
    let nexus = PairedPeer::capture(identity.clone()).expect("paired Nexus");
    let caller = Caller {
        user_id: identity.user_id,
        process_id: identity.process_id,
    };
    assert!(nexus.matches(&caller));
    other.kill().expect("end fixture process");
    other.wait().expect("reap fixture process");
    assert!(!nexus.matches(&caller));
}

#[test]
fn a_translator_socket_is_left_to_its_successor() {
    let directory = fixture("socket");
    let path = directory.join("counter-translator.sock");
    let first = OwnedListener::bind(path.clone()).expect("first bind");
    assert!(OwnedListener::bind(path.clone()).is_err());
    assert!(path.exists());
    drop(first);
    assert!(!path.exists());
    std::fs::remove_dir_all(directory).expect("fixture cleanup");
}

#[test]
fn a_text_budget_is_positive_and_within_signal() {
    let limits = |nodes, depth| TextLimits {
        node_limit: NodeLimit(nodes),
        depth_limit: DepthLimit(depth),
    };
    assert_eq!(
        ValidTextBudget::try_from(&limits(1_048_576, 64)),
        Ok(ValidTextBudget {
            nodes: 1_048_576,
            depth: 64
        })
    );
    assert!(ValidTextBudget::try_from(&limits(0, 64)).is_err());
    assert!(ValidTextBudget::try_from(&limits(64, 0)).is_err());
    assert!(ValidTextBudget::try_from(&limits(i64::MAX, 64)).is_err());
    assert!(ValidTextBudget::try_from(&limits(64, i64::MAX)).is_err());
}

#[test]
fn a_pairing_is_read_from_the_launchers_arguments() {
    let arguments = |extra: &[&str]| {
        [
            "{ counter /run/counter /state/counter }",
            "/bin/counter-nexus",
            "/bin/counter-translator",
            "70000",
            "8388608",
            "25",
            "1048576",
            "64",
        ]
        .iter()
        .chain(extra)
        .map(|argument| argument.to_string())
        .collect::<Vec<_>>()
    };
    let pairing = Pairing::from_arguments(arguments(&[]).into_iter()).expect("pairing");
    assert_eq!(pairing.place.place_name.0, "counter");
    assert_eq!(pairing.place.runtime.0, "/run/counter");
    assert_eq!(pairing.nexus_executable.0, "/bin/counter-nexus");
    assert_eq!(pairing.translator_executable.0, "/bin/counter-translator");
    assert_eq!(pairing.admission.frame_capacity_bytes.0, 8_388_608);
    assert_eq!(pairing.text_limits.depth_limit, DepthLimit(64));
    let extra = Pairing::from_arguments(arguments(&["more"]).into_iter())
        .expect_err("an extra argument is refused");
    assert_eq!(extra.to_string(), "unexpected launcher argument");
    let mut empty = arguments(&[]);
    empty[4] = "0".into();
    let refused = Pairing::from_arguments(empty.into_iter()).expect_err("no capacity");
    assert!(
        refused.to_string().starts_with("invalid admission: "),
        "{refused}"
    );
}
