//! A translator that writes only its bodies is served whole: ordinary text is
//! carried to its Nexus with the Caller and answered as text until the Nexus
//! ends the exchange, each refusal is answered before anything is read, and
//! the translator's own settings are merged into the Nexus's meta readouts
//! and carried into its open subscriptions.

#![cfg(feature = "text")]

use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::RwLock;
use std::time::Duration;

use nexus_exchange::lane::{Frame, Kind, Lane, LaneReading, LaneWriting};
use nexus_exchange::{ComponentName, ComponentTitle, Named, ValidAdmission};
use nexus_serving::{OwnedListener, SocketBinding};
use shared_translator::placement::{Place, PlaceName, Runtime, State};
use shared_translator::{Caller, PeerIdentity, ProcessId, UserId};
use signal::{
    Answer as Answered, ByteViewable, Contracted, Delivery, Dispatch, Ending, FrameCapacity,
    FrameReading, FrameWriting, Restorable, Signal, Signalizable,
};
use translator_serving::{
    Connection, ConnectionServing, DepthLimit, EmissionAnswering, Endpoint, FreshInstance,
    InstanceId, MetaSubscriptions, MetaTranslating, NexusFailure, NodeLimit, PairReading,
    PairedPeer, Pairing, PeerMatching, PublicTranslating, Settling, TextBudgeting, TextLimits,
    Translator, ValidTextBudget,
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
    Read,
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
    Refused(i64),
    Read(Settings),
    Changed(i64),
}

/// A readout of the settings: the Nexus's, and the translator's own, which
/// the Nexus reads out unset as zero.
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
struct Settings {
    nexus: i64,
    translator: i64,
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

/// A counter's translator; `settled` is whether its own settings are set,
/// and `count` is its own setting, which the meta lane sets.
struct CounterTranslator {
    settled: bool,
    count: RwLock<i64>,
}

fn translator(settled: bool) -> CounterTranslator {
    CounterTranslator {
        settled,
        count: RwLock::new(5),
    }
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

    /// A count is the translator's own setting; a read and a follow are the
    /// Nexus's, the follow its subscription.
    fn settle(&self, input: Query) -> Settling<Reply, Query> {
        match input {
            Query::Count(count) => {
                *self.count.write().expect("count") = count;
                Settling::Answered(Reply::Counted(count))
            }
            Query::Read => Settling::Forwarded(Query::Read),
            Query::Follow => Settling::Subscribed(Query::Follow),
        }
    }

    fn meta_call(caller: Caller, query: Query) -> Call {
        Call { caller, query }
    }

    /// A readout of the settings carries the translator's own count.
    fn meta_answered(&self, response: Response) -> Result<Reply, NexusFailure> {
        Ok(match Self::public_answered(response)? {
            Reply::Read(settings) => Reply::Read(Settings {
                translator: *self.count.read().expect("count"),
                ..settings
            }),
            other => other,
        })
    }

    fn announced(&self, answered: &Reply) -> Option<Reply> {
        match answered {
            Reply::Counted(count) => Some(Reply::Changed(*count)),
            _ => None,
        }
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

/// What every connection of a test's translator is served under.
struct Fixture {
    translator: CounterTranslator,
    place: Place,
    subscriptions: MetaSubscriptions,
}

fn fixture_at(translator: CounterTranslator, runtime: &Path) -> Fixture {
    Fixture {
        translator,
        place: place(runtime),
        subscriptions: MetaSubscriptions::default(),
    }
}

/// Serves one request on `lane` to `endpoint`, as a connection of its own
/// among the fixture's, on a thread of the `scope`, and returns the client's
/// end of it.
fn serving<'scope>(
    scope: &'scope std::thread::Scope<'scope, '_>,
    fixture: &'scope Fixture,
    endpoint: Endpoint,
    nexus: Option<&'scope PairedPeer>,
    lane: Lane,
    payload: &[u8],
) -> UnixStream {
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
    scope.spawn(move || {
        connection(fixture, endpoint, nexus)
            .serve(&mut server)
            .expect("request served");
    });
    client
}

/// One connection of the fixture's translator, on `endpoint`.
fn connection<'a>(
    fixture: &'a Fixture,
    endpoint: Endpoint,
    nexus: Option<&'a PairedPeer>,
) -> Connection<'a, CounterTranslator> {
    Connection {
        translator: &fixture.translator,
        place: &fixture.place,
        endpoint,
        admission: admission(),
        bootstrap_budget: ValidTextBudget {
            nodes: 64,
            depth: 8,
        },
        nexus,
        subscriptions: &fixture.subscriptions,
    }
}

/// Serves one request on `lane` to `endpoint` and returns every frame it is
/// answered with.
fn served(
    translator: CounterTranslator,
    runtime: &Path,
    endpoint: Endpoint,
    nexus: Option<&PairedPeer>,
    lane: Lane,
    payload: &[u8],
) -> Vec<Frame> {
    let fixture = fixture_at(translator, runtime);
    std::thread::scope(|scope| {
        let mut client = serving(scope, &fixture, endpoint, nexus, lane, payload);
        let mut frames = Vec::new();
        while let Ok(frame) = client.read_lane_frame(LIMIT) {
            frames.push(frame);
        }
        frames
    })
}

/// The one frame a request is answered with.
fn only(mut frames: Vec<Frame>) -> Frame {
    assert_eq!(frames.len(), 1, "{frames:?}");
    frames.remove(0)
}

/// The text of every answer of a request, which its last frame ends.
fn answered(frames: &[Frame]) -> Vec<&str> {
    let (last, answers) = frames.split_last().expect("an ending frame");
    assert_eq!(last.kind, Kind::Ended, "{frames:?}");
    assert!(last.payload.is_empty());
    answers
        .iter()
        .map(|answer| {
            assert_eq!(answer.kind, Kind::Answered);
            std::str::from_utf8(&answer.payload).expect("answer text")
        })
        .collect()
}

/// One exchange of a Nexus socket, as the Nexus answers it.
enum Answering {
    /// Each response, then the exchange's end.
    Ended(Vec<Response>),
    /// Each response, then the connection closed with no end.
    Closed(Vec<Response>),
    /// Each response, then the end once `release` is received.
    Held(Vec<Response>, std::sync::mpsc::Receiver<()>),
}

/// A Nexus socket `socket` in `runtime` that greets one connection, checks
/// that its one exchange carries `expected` from this process, and answers
/// it as `answering` says.
fn nexus_at(
    runtime: &Path,
    socket: &str,
    expected: Query,
    answering: Answering,
) -> std::thread::JoinHandle<()> {
    let listener = UnixListener::bind(runtime.join(socket)).expect("Nexus socket");
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
        let deliver = |stream: &mut UnixStream, delivery: Delivery<Response>| {
            stream
                .write_frame(&delivery.signalize().expect("delivery"), capacity)
                .expect("delivery frame");
        };
        let (responses, ending) = match answering {
            Answering::Ended(responses) => (responses, Some(None)),
            Answering::Closed(responses) => (responses, None),
            Answering::Held(responses, release) => (responses, Some(Some(release))),
        };
        for response in responses {
            deliver(
                &mut stream,
                Delivery::Answer(Answered {
                    exchange: opening.exchange,
                    response,
                }),
            );
        }
        if let Some(release) = ending {
            if let Some(release) = release {
                release.recv().expect("released");
            }
            deliver(
                &mut stream,
                Delivery::End(Ending::completed(opening.exchange)),
            );
        }
    })
}

/// The public socket of a counter Nexus, ending its one exchange after
/// `responses`, as every exchange ends.
fn nexus(runtime: &Path, expected: Query, responses: Vec<Response>) -> std::thread::JoinHandle<()> {
    nexus_at(
        runtime,
        "counter.sock",
        expected,
        Answering::Ended(responses),
    )
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
fn a_query_is_carried_with_its_caller_and_answered_as_text_then_ended() {
    let directory = fixture("once");
    let answering = nexus(
        &directory,
        Query::Count(3),
        vec![Response::Answered(Reply::Counted(6))],
    );
    let frames = served(
        translator(true),
        &directory,
        Endpoint::Public,
        None,
        Lane::PublicText,
        b"Count.3",
    );
    answering.join().expect("Nexus");
    assert_eq!(answered(&frames), ["Counted.6"]);
    std::fs::remove_dir_all(directory).expect("fixture cleanup");
}

#[test]
fn every_answer_is_written_back_until_the_nexus_ends_the_exchange() {
    let directory = fixture("followed");
    let answering = nexus(
        &directory,
        Query::Follow,
        vec![
            Response::Answered(Reply::Counted(1)),
            Response::Answered(Reply::Counted(2)),
        ],
    );
    let frames = served(
        translator(true),
        &directory,
        Endpoint::Public,
        None,
        Lane::PublicText,
        b"Follow",
    );
    answering.join().expect("Nexus");
    assert_eq!(answered(&frames), ["Counted.1", "Counted.2"]);
    std::fs::remove_dir_all(directory).expect("fixture cleanup");
}

/// A subscription the Nexus refuses is answered once, its refusal, and
/// ended; the translator stops on the end, knowing nothing of which answer
/// is a frame, and the CLI's client returns after reading the refusal.
#[test]
fn a_refused_subscription_ends_and_the_client_returns_after_its_refusal() {
    let directory = fixture("refused");
    let answering = nexus(
        &directory,
        Query::Follow,
        vec![Response::Answered(Reply::Refused(9))],
    );
    let fixture = fixture_at(translator(true), &directory);
    let listener =
        UnixListener::bind(directory.join("counter-translator.sock")).expect("translator socket");
    let client = translator_client::TextClient {
        socket: directory.join("counter-translator.sock"),
        lane: translator_client::TextLane::PublicText,
        admission_limit: LIMIT,
    };
    std::thread::scope(|scope| {
        scope.spawn(|| {
            let (mut stream, _) = listener.accept().expect("client connection");
            connection(&fixture, Endpoint::Public, None)
                .serve(&mut stream)
                .expect("served");
        });
        let mut printed = Vec::new();
        translator_client::TextRequesting::request_stream(&client, "Follow", |answer| {
            printed.push(answer.to_owned());
            true
        })
        .expect("the client returns at the end");
        assert_eq!(printed, ["Refused.9"]);
    });
    answering.join().expect("Nexus");
    std::fs::remove_dir_all(directory).expect("fixture cleanup");
}

#[test]
fn a_nexus_that_closes_before_the_end_is_answered_as_a_failure() {
    let directory = fixture("unended");
    let answering = nexus_at(
        &directory,
        "counter.sock",
        Query::Count(3),
        Answering::Closed(vec![Response::Answered(Reply::Counted(6))]),
    );
    let frames = served(
        translator(true),
        &directory,
        Endpoint::Public,
        None,
        Lane::PublicText,
        b"Count.3",
    );
    answering.join().expect("Nexus");
    assert_eq!(frames.len(), 2, "{frames:?}");
    assert_eq!(frames[0].payload, b"Counted.6");
    assert!(
        refusal(&frames[1]).contains("closed before ending the exchange"),
        "{frames:?}"
    );
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
        translator(true),
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
        translator(true),
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
        translator(false),
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
    let frames = served(
        translator(false),
        Path::new("/nonexistent"),
        Endpoint::Meta,
        None,
        Lane::MetaText,
        b"Count.4",
    );
    assert_eq!(answered(&frames), ["Counted.4"]);
}

#[test]
fn a_forwarded_readout_carries_the_translators_own_setting() {
    let directory = fixture("readout");
    let answering = nexus_at(
        &directory,
        "counter-meta.sock",
        Query::Read,
        Answering::Ended(vec![Response::Answered(Reply::Read(Settings {
            nexus: 1,
            translator: 0,
        }))]),
    );
    let frames = served(
        translator(true),
        &directory,
        Endpoint::Meta,
        None,
        Lane::MetaText,
        b"Read",
    );
    answering.join().expect("Nexus");
    assert_eq!(answered(&frames), ["Read.{ 1 5 }"]);
    std::fs::remove_dir_all(directory).expect("fixture cleanup");
}

/// A subscription of the Nexus's carries the translator's own setting in
/// its first readout and each change of it after, between the Nexus's
/// answers, until the Nexus ends it.
#[test]
fn the_translators_own_change_is_carried_into_an_open_subscription() {
    let directory = fixture("subscription");
    let (release, released) = std::sync::mpsc::channel();
    let answering = nexus_at(
        &directory,
        "counter-meta.sock",
        Query::Follow,
        Answering::Held(
            vec![Response::Answered(Reply::Read(Settings {
                nexus: 1,
                translator: 0,
            }))],
            released,
        ),
    );
    let fixture = fixture_at(translator(true), &directory);
    std::thread::scope(|scope| {
        let mut subscriber = serving(
            scope,
            &fixture,
            Endpoint::Meta,
            None,
            Lane::MetaText,
            b"Follow",
        );
        let opened = subscriber.read_lane_frame(LIMIT).expect("opened");
        assert_eq!(opened.payload, b"Read.{ 1 5 }");
        let mut setter = serving(
            scope,
            &fixture,
            Endpoint::Meta,
            None,
            Lane::MetaText,
            b"Count.7",
        );
        let mut set = Vec::new();
        while let Ok(frame) = setter.read_lane_frame(LIMIT) {
            set.push(frame);
        }
        assert_eq!(answered(&set), ["Counted.7"]);
        let changed = subscriber.read_lane_frame(LIMIT).expect("changed");
        assert_eq!(
            (changed.kind, changed.payload),
            (Kind::Answered, b"Changed.7".to_vec())
        );
        release.send(()).expect("release the end");
        let ended = subscriber.read_lane_frame(LIMIT).expect("ended");
        assert_eq!(ended.kind, Kind::Ended);
    });
    answering.join().expect("Nexus");
    std::fs::remove_dir_all(directory).expect("fixture cleanup");
}

#[test]
fn a_fresh_instance_is_thirty_two_hexadecimal_digits_and_new_each_time() {
    let first = InstanceId::fresh().expect("first");
    let second = InstanceId::fresh().expect("second");
    assert_eq!(first.0.len(), 32);
    assert!(first.0.chars().all(|digit| digit.is_ascii_hexdigit()));
    assert_ne!(first, second);
}

#[test]
fn each_socket_carries_only_its_own_lanes() {
    let meta_on_public = only(served(
        translator(true),
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
        translator(true),
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
        translator(true),
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
