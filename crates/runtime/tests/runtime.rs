//! Real Unix sockets, the shared client contract, real Publisher/Stream and
//! the shared typed sender. Every wait follows an event; there are no sleeps.
use datom_codec::{Actualizing, Budget, Composing, Datomizable, Potential};
use futures_util::StreamExt;
use nexus::{
    Capacity, EmissionSender, Publisher, Publishing, Sending, SignalReceiving, SignalSubmitting,
    Unobserved,
};
use nexus_client::{Named, Served, TextReply, TextRequest};
use signal::{Following, Frame};
use std::{
    future::Future,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::AsyncReadExt,
    net::{UnixListener, UnixStream},
    sync::{Notify, mpsc, watch},
    task::{JoinHandle, JoinSet},
};
use translator::{Call, DatomReadingBudget, OutputMode, ReadingDepth, ReadingSteps};
use translator_runtime::{
    BindingTranslator, ConfigurationFollowing, ConfigurationObservation, ConfigurationWatching,
    Emittable, ObservingConfiguration, Place, PlaceName, Runtime, Serving, SocketLocating, State,
    Translating, Translator, frame_capacity,
};

const EVENT_DEADLINE: Duration = Duration::from_secs(10);

#[derive(
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
    Clone,
    Debug,
    PartialEq,
    Eq,
    Composing,
    Datomizable,
)]
enum Input {
    Echo(String),
    Refuse,
    Subscribe,
    Disappear,
}
#[derive(
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
    Clone,
    Debug,
    PartialEq,
    Eq,
    Composing,
    Datomizable,
)]
struct Counter(i64);
impl Following for Counter {
    type Change = i64;
    fn after(state: Self, change: i64) -> Self {
        Self(state.0 + change)
    }
}
#[derive(
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
    Clone,
    Debug,
    PartialEq,
    Eq,
    Composing,
    Datomizable,
)]
enum Output {
    Echoed(String),
    Refused,
    Updates(Frame<Counter, i64>),
}
#[derive(
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
    Clone,
    Debug,
    PartialEq,
    Eq,
    Composing,
    Datomizable,
)]
enum MetaInput {
    SubscribeConfiguration,
    Disappear,
    Echo(String),
    Refuse,
}
#[derive(
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
    Clone,
    Debug,
    PartialEq,
    Eq,
    Composing,
    Datomizable,
)]
enum MetaOutput {
    Configuration(Frame<DatomReadingBudget, DatomReadingBudget>),
    Echoed(String),
    Refused,
}
#[derive(rkyv::Archive, rkyv::Serialize, rkyv::Deserialize, Clone, Debug, PartialEq, Eq)]
enum Emission {
    Add(i64),
    Hold,
}
#[derive(rkyv::Archive, rkyv::Serialize, rkyv::Deserialize, Clone, Debug, PartialEq, Eq)]
enum Outcome {
    Added(i64),
    Held,
}
static ADDED: AtomicUsize = AtomicUsize::new(0);
static ENTERED: Notify = Notify::const_new();
impl Emittable for Emission {
    type Outcome = Outcome;
    async fn emit(self) -> Outcome {
        match self {
            Self::Add(value) => {
                ADDED.fetch_add(1, Ordering::SeqCst);
                Outcome::Added(value + 1)
            }
            Self::Hold => {
                ENTERED.notify_one();
                std::future::pending::<()>().await;
                Outcome::Held
            }
        }
    }
}
struct Example;
impl Named for Example {
    fn name() -> String {
        "example".into()
    }
}
impl Translating for Example {
    type Input = Input;
    type Output = Output;
    type MetaInput = MetaInput;
    type MetaOutput = MetaOutput;
    type Emission = Emission;
    type Outcome = Outcome;
    fn describe() -> String {
        "ordinary input/output Ethos".into()
    }
    fn describe_meta() -> String {
        "meta input/output Ethos".into()
    }
    fn output_mode(output: &Output) -> OutputMode {
        match output {
            Output::Updates(_) => OutputMode::Streaming,
            _ => OutputMode::Single,
        }
    }
    fn meta_output_mode(output: &MetaOutput) -> OutputMode {
        match output {
            MetaOutput::Configuration(_) => OutputMode::Streaming,
            _ => OutputMode::Single,
        }
    }
}
impl ConfigurationFollowing for Example {
    fn subscribe_configuration() -> MetaInput {
        MetaInput::SubscribeConfiguration
    }
    fn snapshot(output: &MetaOutput) -> Option<DatomReadingBudget> {
        match output {
            MetaOutput::Configuration(Frame::Opened(budget)) => Some(budget.clone()),
            _ => None,
        }
    }
    fn after(_: &DatomReadingBudget, output: &MetaOutput) -> Option<DatomReadingBudget> {
        match output {
            MetaOutput::Configuration(Frame::Changed(budget) | Frame::Lagged(budget)) => {
                Some(budget.clone())
            }
            _ => None,
        }
    }
}

// Trait exception: Too trivial; the fixture supplies one explicit test budget.
fn budget(steps: i64) -> DatomReadingBudget {
    DatomReadingBudget {
        reading_steps: ReadingSteps(steps),
        reading_depth: ReadingDepth(64),
    }
}
// Trait exception: Too trivial; this is the test's event-deadline oracle.
async fn event<T>(future: impl Future<Output = T>) -> T {
    tokio::time::timeout(EVENT_DEADLINE, future)
        .await
        .expect("event deadline")
}
// Trait exception: Too trivial; this reads one fixture datom with its test policy.
fn decoded<T: Composing>(text: String) -> T {
    let mut reading = Budget {
        remaining: 10_000,
        reader: protos::ReaderBudget::try_from(10_000).unwrap(),
        depth: 0,
        maximum_depth: 64,
    };
    Potential::<T>::from(text)
        .actualize(&mut reading)
        .expect("fixture datom")
}

struct Fixture {
    _scratch: tempfile::TempDir,
    translator: Translator,
    configuration: watch::Sender<DatomReadingBudget>,
    configuration_gate: Option<Arc<Notify>>,
    subscriptions: Arc<AtomicUsize>,
    calls: mpsc::UnboundedReceiver<Call<Input>>,
    meta_calls: mpsc::UnboundedReceiver<Call<MetaInput>>,
    publisher: Arc<Publisher<Counter, i64>>,
    stream_closed: Arc<Notify>,
    nexus_tasks: Vec<JoinHandle<()>>,
    runtime: Option<JoinHandle<Result<(), translator_runtime::RuntimeError>>>,
    observation: Option<ConfigurationObservation>,
}
trait FixtureServing {
    fn new() -> Self;
    fn start_meta(&mut self);
    fn start_ordinary(&mut self);
    fn start_runtime(&mut self);
    fn observed(&mut self, expected: Option<DatomReadingBudget>) -> impl Future<Output = ()>;
    fn text(&self, meta: bool, request: TextRequest) -> impl Future<Output = UnixStream>;
    fn single(&self, meta: bool, text: &str) -> impl Future<Output = String>;
}
impl FixtureServing for Fixture {
    fn new() -> Self {
        let scratch = tempfile::tempdir().unwrap();
        let runtime = scratch.path().join("runtime");
        std::fs::create_dir_all(&runtime).unwrap();
        let translator = Translator {
            place: Place {
                place_name: PlaceName("fixture".into()),
                runtime: Runtime(runtime.to_string_lossy().into_owned()),
                state: State(scratch.path().join("state").to_string_lossy().into_owned()),
            },
        };
        let (configuration, _) = watch::channel(budget(10_000));
        let (_, calls) = mpsc::unbounded_channel();
        let (_, meta_calls) = mpsc::unbounded_channel();
        Self {
            _scratch: scratch,
            translator,
            configuration,
            configuration_gate: None,
            subscriptions: Arc::new(AtomicUsize::new(0)),
            calls,
            meta_calls,
            publisher: Arc::new(Publisher::create(Counter(0), Capacity(8)).unwrap()),
            stream_closed: Arc::new(Notify::new()),
            nexus_tasks: Vec::new(),
            runtime: None,
            observation: None,
        }
    }
    fn start_meta(&mut self) {
        let path = self.translator.socket_paths::<Example>().meta_nexus;
        let listener = UnixListener::bind(path).unwrap();
        let configuration = self.configuration.subscribe();
        let gate = self.configuration_gate.clone();
        let subscriptions = self.subscriptions.clone();
        let (calls, receiver) = mpsc::unbounded_channel();
        self.meta_calls = receiver;
        self.nexus_tasks.push(tokio::spawn(async move {
            let mut connections = JoinSet::new();
            loop {
                tokio::select! {
                    connection = listener.accept() => {
                        let (mut stream, _) = connection.unwrap();
                        let calls = calls.clone(); let mut configuration = configuration.clone();
                        let gate = gate.clone(); let subscriptions = subscriptions.clone();
                        connections.spawn(async move {
                            let call = stream.receive::<Call<MetaInput>>(frame_capacity()).await.unwrap();
                            calls.send(call.clone()).unwrap();
                            match call.input {
                                MetaInput::Disappear => {},
                                MetaInput::SubscribeConfiguration => {
                                    subscriptions.fetch_add(1, Ordering::SeqCst);
                                    if let Some(gate) = gate { gate.notified().await; }
                                    let initial = configuration.borrow_and_update().clone();
                                    if stream.submit(MetaOutput::Configuration(Frame::Opened(initial)), frame_capacity()).await.is_err() { return; }
                                    let mut byte = [0];
                                    loop {
                                        tokio::select! {
                                            change = configuration.changed() => {
                                                if change.is_err() { return; }
                                                let next = configuration.borrow_and_update().clone();
                                                if stream.submit(MetaOutput::Configuration(Frame::Changed(next)), frame_capacity()).await.is_err() { return; }
                                            }
                                            _ = stream.read(&mut byte) => return,
                                        }
                                    }
                                }
                                MetaInput::Echo(value) => { stream.submit(MetaOutput::Echoed(value), frame_capacity()).await.unwrap(); }
                                MetaInput::Refuse => { stream.submit(MetaOutput::Refused, frame_capacity()).await.unwrap(); }
                            }
                        });
                    }
                    _ = connections.join_next(), if !connections.is_empty() => {}
                }
            }
        }));
    }
    fn start_ordinary(&mut self) {
        let listener =
            UnixListener::bind(self.translator.socket_paths::<Example>().ordinary_nexus).unwrap();
        let publisher = self.publisher.clone();
        let closed = self.stream_closed.clone();
        let (calls, receiver) = mpsc::unbounded_channel();
        self.calls = receiver;
        self.nexus_tasks.push(tokio::spawn(async move {
            let mut connections = JoinSet::new();
            loop {
                tokio::select! {
                    connection = listener.accept() => {
                        let (mut stream, _) = connection.unwrap();
                        let calls = calls.clone(); let publisher = publisher.clone(); let closed = closed.clone();
                        connections.spawn(async move {
                            let call = stream.receive::<Call<Input>>(frame_capacity()).await.unwrap();
                            calls.send(call.clone()).unwrap();
                            match call.input {
                                Input::Disappear => {},
                                Input::Echo(value) => { stream.submit(Output::Echoed(value), frame_capacity()).await.unwrap(); }
                                Input::Refuse => { stream.submit(Output::Refused, frame_capacity()).await.unwrap(); }
                                Input::Subscribe => {
                                    let mut changes = publisher.open(); let mut byte = [0];
                                    loop {
                                        tokio::select! {
                                            frame = changes.next() => {
                                                let Some(frame) = frame else { return; };
                                                if stream.submit(Output::Updates(frame), frame_capacity()).await.is_err() { closed.notify_one(); return; }
                                            }
                                            _ = stream.read(&mut byte) => { closed.notify_one(); return; }
                                        }
                                    }
                                }
                            }
                        });
                    }
                    _ = connections.join_next(), if !connections.is_empty() => {}
                }
            }
        }));
    }
    fn start_runtime(&mut self) {
        let bound =
            <Translator as BindingTranslator<Example>>::bind(self.translator.clone()).unwrap();
        self.observation = Some(bound.configuration());
        self.runtime = Some(tokio::spawn(bound.serve()));
    }
    async fn observed(&mut self, expected: Option<DatomReadingBudget>) {
        let observation = self.observation.as_mut().unwrap();
        event(async {
            loop {
                if observation.budget() == expected {
                    return;
                }
                observation.changed().await.unwrap();
            }
        })
        .await;
    }
    async fn text(&self, meta: bool, request: TextRequest) -> UnixStream {
        let paths = self.translator.socket_paths::<Example>();
        let mut stream = UnixStream::connect(if meta {
            paths.meta_text
        } else {
            paths.ordinary_text
        })
        .await
        .unwrap();
        stream.submit(request, frame_capacity()).await.unwrap();
        stream
    }
    async fn single(&self, meta: bool, text: &str) -> String {
        let mut stream = self.text(meta, TextRequest::Datom(text.into())).await;
        let TextReply::Datom(reply) = event(stream.receive(frame_capacity())).await.unwrap() else {
            panic!("datom reply")
        };
        assert_eq!(
            event(stream.receive::<TextReply>(frame_capacity()))
                .await
                .unwrap(),
            TextReply::Served(Served::Answered)
        );
        reply
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(task) = &self.runtime {
            task.abort();
        }
        for task in &self.nexus_tasks {
            task.abort();
        }
    }
}

#[tokio::test]
async fn ordinary_meta_success_refusal_unreadable_and_describe() {
    let mut fixture = Fixture::new();
    fixture.start_meta();
    fixture.start_ordinary();
    fixture.start_runtime();
    fixture.observed(Some(budget(10_000))).await;
    assert_eq!(
        decoded::<Output>(fixture.single(false, "Echo.«whole text»").await),
        Output::Echoed("whole text".into())
    );
    assert_eq!(
        decoded::<Output>(fixture.single(false, "Refuse").await),
        Output::Refused
    );
    assert_eq!(
        decoded::<MetaOutput>(fixture.single(true, "Echo.«meta text»").await),
        MetaOutput::Echoed("meta text".into())
    );
    assert_eq!(
        decoded::<MetaOutput>(fixture.single(true, "Refuse").await),
        MetaOutput::Refused
    );
    for meta in [false, true] {
        let mut invalid = fixture
            .text(meta, TextRequest::Datom("{ malformed }".into()))
            .await;
        assert!(matches!(
            event(invalid.receive::<TextReply>(frame_capacity()))
                .await
                .unwrap(),
            TextReply::Served(Served::Unreadable(_))
        ));
        let mut describe = fixture.text(meta, TextRequest::Describe).await;
        let expected = if meta {
            Example::describe_meta()
        } else {
            Example::describe()
        };
        assert_eq!(
            event(describe.receive::<TextReply>(frame_capacity()))
                .await
                .unwrap(),
            TextReply::Datom(expected)
        );
        assert_eq!(
            event(describe.receive::<TextReply>(frame_capacity()))
                .await
                .unwrap(),
            TextReply::Served(Served::Answered)
        );
    }
    for meta in [false, true] {
        let mut vanished = fixture
            .text(meta, TextRequest::Datom("Disappear".into()))
            .await;
        assert_eq!(
            event(vanished.receive::<TextReply>(frame_capacity()))
                .await
                .unwrap(),
            TextReply::Served(Served::NexusDown)
        );
    }
    let bootstrap = fixture.meta_calls.recv().await.unwrap();
    assert_eq!(bootstrap.input, MetaInput::SubscribeConfiguration);
    assert_eq!(bootstrap.caller.process_id.0, i64::from(std::process::id()));
    assert_eq!(
        bootstrap.caller.user_id.0,
        i64::from(rustix::process::getuid().as_raw())
    );
}

#[tokio::test]
async fn stream_uses_publisher_snapshot_then_changes_and_quiet_disconnect() {
    let mut fixture = Fixture::new();
    fixture.start_meta();
    fixture.start_ordinary();
    fixture.start_runtime();
    fixture.observed(Some(budget(10_000))).await;
    let mut text = fixture
        .text(false, TextRequest::Datom("Subscribe".into()))
        .await;
    let TextReply::Datom(snapshot) = event(text.receive(frame_capacity())).await.unwrap() else {
        panic!("snapshot")
    };
    assert_eq!(
        decoded::<Output>(snapshot),
        Output::Updates(Frame::Opened(Counter(0)))
    );
    for change in [3, 5] {
        fixture.publisher.publish(change);
        let TextReply::Datom(frame) = event(text.receive(frame_capacity())).await.unwrap() else {
            panic!("change")
        };
        assert_eq!(
            decoded::<Output>(frame),
            Output::Updates(Frame::Changed(change))
        );
    }
    drop(text);
    event(fixture.stream_closed.notified()).await;
}

#[tokio::test]
async fn nexus_ending_first_is_nexus_down_not_a_successful_stream_end() {
    let mut fixture = Fixture::new();
    fixture.start_meta();
    fixture.start_ordinary();
    fixture.start_runtime();
    fixture.observed(Some(budget(10_000))).await;
    let mut text = fixture
        .text(false, TextRequest::Datom("Subscribe".into()))
        .await;
    assert!(matches!(
        event(text.receive::<TextReply>(frame_capacity()))
            .await
            .unwrap(),
        TextReply::Datom(_)
    ));
    fixture.nexus_tasks[1].abort();
    event(&mut fixture.nexus_tasks[1]).await.unwrap_err();
    assert_eq!(
        event(text.receive::<TextReply>(frame_capacity()))
            .await
            .unwrap(),
        TextReply::Served(Served::NexusDown)
    );
}

#[tokio::test]
async fn shared_sender_dispatches_once_and_classifies_both_unobserved_cases() {
    let mut fixture = Fixture::new();
    let sender = EmissionSender {
        socket_path: fixture.translator.socket_paths::<Example>().emission,
        frame_capacity: frame_capacity(),
    };
    assert_eq!(
        sender.send::<Emission, Outcome>(Emission::Add(1)).await,
        Err(Unobserved::Unsubmitted)
    );
    fixture.start_runtime();
    let before = ADDED.load(Ordering::SeqCst);
    assert_eq!(
        event(sender.send::<Emission, Outcome>(Emission::Add(41))).await,
        Ok(Outcome::Added(42))
    );
    assert_eq!(ADDED.load(Ordering::SeqCst), before + 1);
    let pending =
        tokio::spawn(async move { sender.send::<Emission, Outcome>(Emission::Hold).await });
    event(ENTERED.notified()).await;
    let runtime = fixture.runtime.take().unwrap();
    runtime.abort();
    event(runtime).await.unwrap_err();
    assert_eq!(event(pending).await.unwrap(), Err(Unobserved::Unanswered));
}

#[tokio::test]
async fn startup_without_nexus_and_socket_creation_follow_the_configuration_events() {
    let mut fixture = Fixture::new();
    fixture.start_runtime();
    let mut absent = fixture
        .text(false, TextRequest::Datom("Echo.value".into()))
        .await;
    assert_eq!(
        event(absent.receive::<TextReply>(frame_capacity()))
            .await
            .unwrap(),
        TextReply::Served(Served::NexusDown)
    );
    fixture.start_meta();
    fixture.start_ordinary();
    fixture.observed(Some(budget(10_000))).await;
    assert_eq!(
        decoded::<Output>(fixture.single(false, "Echo.value").await),
        Output::Echoed("value".into())
    );
    fixture.configuration.send_replace(budget(1));
    fixture.observed(Some(budget(1))).await;
    let mut limited = fixture
        .text(false, TextRequest::Datom("Echo.value".into()))
        .await;
    assert!(matches!(
        event(limited.receive::<TextReply>(frame_capacity()))
            .await
            .unwrap(),
        TextReply::Served(Served::Unreadable(_))
    ));
    fixture.configuration.send_replace(budget(10_000));
    fixture.observed(Some(budget(10_000))).await;
    assert_eq!(
        decoded::<Output>(fixture.single(false, "Echo.value").await),
        Output::Echoed("value".into())
    );
}

#[test]
#[ignore = "launched by the original-peer-credentials integration test"]
fn child_text_connection() {
    let path = std::env::var_os("TRANSLATOR_TEST_SOCKET").unwrap();
    let stream = std::os::unix::net::UnixStream::connect(path).unwrap();
    use nexus_client::Exchanging;
    let served = nexus_client::TextExchange {
        stream,
        capacity: frame_capacity(),
    }
    .exchange(TextRequest::Datom("Echo.child".into()), &mut Vec::new())
    .unwrap();
    assert_eq!(served, Served::Answered);
}

#[tokio::test]
async fn original_child_peer_credentials_are_forwarded_not_translator_credentials() {
    let mut fixture = Fixture::new();
    fixture.start_meta();
    fixture.start_ordinary();
    fixture.start_runtime();
    fixture.observed(Some(budget(10_000))).await;
    let child = tokio::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "child_text_connection",
            "--ignored",
            "--nocapture",
        ])
        .env(
            "TRANSLATOR_TEST_SOCKET",
            fixture.translator.socket_paths::<Example>().ordinary_text,
        )
        .spawn()
        .unwrap();
    let child_pid = child.id().unwrap();
    let call = event(fixture.calls.recv()).await.unwrap();
    assert_eq!(call.caller.process_id.0, i64::from(child_pid));
    assert_ne!(call.caller.process_id.0, i64::from(std::process::id()));
    assert_eq!(
        call.caller.user_id.0,
        i64::from(rustix::process::getuid().as_raw())
    );
    assert_eq!(call.input, Input::Echo("child".into()));
    assert!(
        event(child.wait_with_output())
            .await
            .unwrap()
            .status
            .success()
    );
}

#[tokio::test]
async fn concurrent_arrivals_share_one_bootstrap_and_stream_loss_waits_for_an_event() {
    let mut fixture = Fixture::new();
    let gate = Arc::new(Notify::new());
    fixture.configuration_gate = Some(gate.clone());
    fixture.start_meta();
    fixture.start_ordinary();
    fixture.start_runtime();
    let path = fixture.translator.socket_paths::<Example>().ordinary_text;
    let (ready, mut arrivals) = mpsc::unbounded_channel();
    let mut clients = JoinSet::new();
    for _ in 0..10 {
        let path = path.clone();
        let ready = ready.clone();
        clients.spawn(async move {
            let mut stream = UnixStream::connect(path).await.unwrap();
            stream
                .submit(TextRequest::Datom("Echo.value".into()), frame_capacity())
                .await
                .unwrap();
            ready.send(()).unwrap();
            assert!(matches!(
                stream.receive::<TextReply>(frame_capacity()).await.unwrap(),
                TextReply::Datom(_)
            ));
            assert_eq!(
                stream.receive::<TextReply>(frame_capacity()).await.unwrap(),
                TextReply::Served(Served::Answered)
            );
        });
    }
    for _ in 0..10 {
        event(arrivals.recv()).await.unwrap();
    }
    gate.notify_one();
    while let Some(result) = event(clients.join_next()).await {
        result.unwrap();
    }
    assert_eq!(fixture.subscriptions.load(Ordering::SeqCst), 1);
    fixture.nexus_tasks[0].abort();
    event(&mut fixture.nexus_tasks[0]).await.unwrap_err();
    fixture.observed(None).await;
    assert_eq!(fixture.subscriptions.load(Ordering::SeqCst), 1);
    let mut absent = fixture
        .text(false, TextRequest::Datom("Echo.value".into()))
        .await;
    assert_eq!(
        event(absent.receive::<TextReply>(frame_capacity()))
            .await
            .unwrap(),
        TextReply::Served(Served::NexusDown)
    );
    assert_eq!(fixture.subscriptions.load(Ordering::SeqCst), 1);
    std::fs::remove_file(fixture.translator.socket_paths::<Example>().meta_nexus).unwrap();
    fixture.configuration_gate = None;
    fixture.start_meta();
    fixture.observed(Some(budget(10_000))).await;
    assert_eq!(fixture.subscriptions.load(Ordering::SeqCst), 2);
}
