//! A Nexus served by `nexus-serving` and a translator served by this crate,
//! paired in one process: a meta Subscribe of the configuration stays open,
//! and a configuration change reaches it whether the Nexus makes it or the
//! translator's own setting does. The readout's readiness is the pair's: the
//! translator's settings still awaited first, then the Nexus's.

#![cfg(feature = "text")]

use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use futures_channel::mpsc::{UnboundedSender, unbounded};
use nexus_exchange::lane::{Frame, Kind, Lane, LaneReading, LaneWriting};
use nexus_exchange::{ComponentName, ComponentTitle, Named, ValidAdmission};
use nexus_serving::{Listeners, Listening, MetaAnswering, MetaConnection, MetaServing, Reply};
use shared_translator::placement::{Place, PlaceName, Runtime, State};
use shared_translator::{Caller, PeerIdentity, ProcessId, UserId};
use signal::Contracted;
use translator_serving::{
    Budgeting, Connection, ConnectionServing, EmissionAnswering, Endpoint, MetaSubscriptions,
    MetaTranslating, NexusFailure, PublicTranslating, Settling, TextBudgeting, Translator,
    ValidTextBudget,
};

const LIMIT: usize = 4096;

/// Derives every text and wire value of the pair's meta contract bears.
macro_rules! text_value {
    ($item:item) => {
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
        $item
    };
}

text_value! {
    /// A setting of the pair: the Nexus's own, or the translator's.
    enum Setting {
        Nexus(i64),
        Translator(i64),
    }
}

text_value! {
    enum SettingName {
        Nexus,
        Translator,
    }
}

text_value! {
    enum Readiness {
        Ready,
        Awaiting(Vec<SettingName>),
    }
}

text_value! {
    /// The pair's configuration, as the meta CLI reads it out.
    struct Configuration {
        nexus: Option<i64>,
        translator: Option<i64>,
        readiness: Readiness,
    }
}

text_value! {
    /// The meta CLI's query.
    enum MetaQuery {
        Configure(Setting),
        Subscribe,
    }
}

text_value! {
    /// The meta CLI's answer: a setting configured, or a frame of the
    /// configuration stream.
    enum MetaAnswer {
        Configured(Setting),
        Opened(Configuration),
        Changed(Setting),
    }
}

/// The part of a meta query the Nexus answers.
#[derive(rkyv::Archive, rkyv::Serialize, rkyv::Deserialize, Clone, Debug, PartialEq)]
enum NexusQuery {
    Configure(i64),
    Subscribe,
}

#[derive(rkyv::Archive, rkyv::Serialize, rkyv::Deserialize, Clone, Debug, PartialEq)]
struct NexusCall {
    caller: Caller,
    query: NexusQuery,
}

impl Contracted for NexusCall {
    const CONTRACT_SOURCE: &'static str = "Signal [] [ Call.{ Caller NexusQuery } ] [] []";
}

/// The Nexus's own configuration: its one setting and its readiness.
#[derive(rkyv::Archive, rkyv::Serialize, rkyv::Deserialize, Clone, Debug, PartialEq)]
struct NexusConfiguration {
    nexus: Option<i64>,
    readiness: Readiness,
}

#[derive(rkyv::Archive, rkyv::Serialize, rkyv::Deserialize, Clone, Debug, PartialEq)]
enum NexusResponse {
    Configured(i64),
    Followed(signal::Frame<NexusConfiguration, i64>),
}

struct Paired;

impl Named for Paired {
    fn name() -> ComponentName {
        ComponentName("paired".into())
    }

    fn title() -> ComponentTitle {
        ComponentTitle("Paired Nexus".into())
    }
}

type Frames =
    Pin<Box<dyn futures_util::Stream<Item = signal::Frame<NexusConfiguration, i64>> + Send>>;

/// The Nexus's meta side: its setting, and every subscription following it.
#[derive(Default)]
struct PairedNexus {
    setting: Mutex<Option<i64>>,
    following: Mutex<Vec<UnboundedSender<signal::Frame<NexusConfiguration, i64>>>>,
}

impl PairedNexus {
    fn configuration(setting: Option<i64>) -> NexusConfiguration {
        NexusConfiguration {
            nexus: setting,
            readiness: match setting {
                Some(_) => Readiness::Ready,
                None => Readiness::Awaiting(vec![SettingName::Nexus]),
            },
        }
    }

    /// Ends every subscription: each stream finishes, and its exchange ends.
    fn finish(&self) {
        self.following.lock().expect("following").clear();
    }
}

impl MetaAnswering for PairedNexus {
    type Component = Paired;
    type Query = NexusCall;
    type Response = NexusResponse;
    type State = NexusConfiguration;
    type Change = i64;

    fn answer(&self, call: NexusCall) -> Reply<NexusResponse, NexusConfiguration, i64> {
        let mut setting = self.setting.lock().expect("setting");
        let mut following = self.following.lock().expect("following");
        match call.query {
            NexusQuery::Configure(value) => {
                *setting = Some(value);
                following
                    .retain(|sender| sender.unbounded_send(signal::Frame::Changed(value)).is_ok());
                Reply::Once(NexusResponse::Configured(value))
            }
            NexusQuery::Subscribe => {
                let (sender, frames) = unbounded();
                sender
                    .unbounded_send(signal::Frame::Opened(Self::configuration(*setting)))
                    .expect("opened");
                following.push(sender);
                Reply::Subscribed(signal::Stream::from(Box::pin(frames) as Frames))
            }
        }
    }

    fn subscribed(frame: signal::Frame<NexusConfiguration, i64>) -> NexusResponse {
        NexusResponse::Followed(frame)
    }
}

struct Ready;

impl TextBudgeting for Ready {
    fn text_budget(&self) -> ValidTextBudget {
        ValidTextBudget {
            nodes: 64,
            depth: 8,
        }
    }
}

/// The translator: its own setting, unset until the meta lane sets it.
#[derive(Default)]
struct PairedTranslator {
    setting: RwLock<Option<i64>>,
}

impl Translator for PairedTranslator {
    type Component = Paired;
    type Ready = Ready;

    fn ready(&self) -> Option<Ready> {
        Some(Ready)
    }
}

impl PublicTranslating for PairedTranslator {
    type PublicInput = MetaQuery;
    type PublicOutput = MetaAnswer;
    type PublicCall = NexusCall;
    type PublicResponse = NexusResponse;

    fn public_call(caller: Caller, _input: MetaQuery) -> NexusCall {
        NexusCall {
            caller,
            query: NexusQuery::Subscribe,
        }
    }

    fn public_answered(_response: NexusResponse) -> Result<MetaAnswer, NexusFailure> {
        Err(NexusFailure("the pair has no public contract".into()))
    }
}

impl MetaTranslating for PairedTranslator {
    type MetaInput = MetaQuery;
    type MetaOutput = MetaAnswer;
    type NexusMetaInput = NexusQuery;
    type MetaCall = NexusCall;
    type MetaResponse = NexusResponse;

    fn settle(&self, input: MetaQuery) -> Settling<MetaAnswer, NexusQuery> {
        match input {
            MetaQuery::Configure(Setting::Translator(value)) => {
                *self.setting.write().expect("setting") = Some(value);
                Settling::Answered(MetaAnswer::Configured(Setting::Translator(value)))
            }
            MetaQuery::Configure(Setting::Nexus(value)) => {
                Settling::Forwarded(NexusQuery::Configure(value))
            }
            MetaQuery::Subscribe => Settling::Subscribed(NexusQuery::Subscribe),
        }
    }

    fn meta_call(caller: Caller, query: NexusQuery) -> NexusCall {
        NexusCall { caller, query }
    }

    /// The Nexus's readout with the translator's own setting merged in, its
    /// readiness the pair's: the translator's setting first among the
    /// awaited, then the Nexus's.
    fn meta_answered(&self, response: NexusResponse) -> Result<MetaAnswer, NexusFailure> {
        let translator = *self.setting.read().expect("setting");
        Ok(match response {
            NexusResponse::Configured(value) => MetaAnswer::Configured(Setting::Nexus(value)),
            NexusResponse::Followed(signal::Frame::Opened(nexus))
            | NexusResponse::Followed(signal::Frame::Lagged(nexus)) => {
                let mut awaiting = Vec::new();
                if translator.is_none() {
                    awaiting.push(SettingName::Translator);
                }
                if let Readiness::Awaiting(names) = nexus.readiness {
                    awaiting.extend(names);
                }
                MetaAnswer::Opened(Configuration {
                    nexus: nexus.nexus,
                    translator,
                    readiness: if awaiting.is_empty() {
                        Readiness::Ready
                    } else {
                        Readiness::Awaiting(awaiting)
                    },
                })
            }
            NexusResponse::Followed(signal::Frame::Changed(value)) => {
                MetaAnswer::Changed(Setting::Nexus(value))
            }
        })
    }

    fn announced(&self, answered: &MetaAnswer) -> Option<MetaAnswer> {
        match answered {
            MetaAnswer::Configured(Setting::Translator(value)) => {
                Some(MetaAnswer::Changed(Setting::Translator(*value)))
            }
            _ => None,
        }
    }
}

#[derive(rkyv::Archive, rkyv::Serialize, rkyv::Deserialize, Clone, Debug, PartialEq)]
enum Ping {
    Ping,
}

impl EmissionAnswering for PairedTranslator {
    type Emission = Ping;
    type Outcome = Ping;

    fn emitted(&self, _ready: &Ready, emission: Ping) -> Ping {
        emission
    }
}

fn admission() -> ValidAdmission {
    ValidAdmission {
        deadline: Duration::from_secs(2),
        frame_capacity: LIMIT,
        retry_delay: Duration::from_millis(5),
    }
}

fn fixture(name: &str) -> PathBuf {
    let directory =
        std::env::temp_dir().join(format!("translator-pair-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).expect("fixture directory");
    directory
}

/// The translator's side of the pair: what each of its connections is served
/// under.
struct TranslatorSide {
    translator: PairedTranslator,
    place: Place,
    subscriptions: MetaSubscriptions,
}

impl TranslatorSide {
    /// Serves one meta request carrying `query` on a thread of the `scope`,
    /// and returns the meta CLI's end of it.
    fn requested<'scope>(
        &'scope self,
        scope: &'scope std::thread::Scope<'scope, '_>,
        query: &MetaQuery,
    ) -> UnixStream {
        let (mut client, mut server) = UnixStream::pair().expect("socket pair");
        client
            .write_lane_frame(
                &Frame {
                    lane: Lane::MetaText,
                    kind: Kind::Request,
                    payload: shared_translator::encode(query).into_bytes(),
                },
                LIMIT,
            )
            .expect("request frame");
        scope.spawn(move || {
            Connection {
                translator: &self.translator,
                place: &self.place,
                endpoint: Endpoint::Meta,
                admission: admission(),
                bootstrap_budget: ValidTextBudget {
                    nodes: 64,
                    depth: 8,
                },
                nexus: None,
                subscriptions: &self.subscriptions,
            }
            .serve(&mut server)
            .expect("request served");
        });
        client
    }
}

/// The next frame the meta CLI reads: one answer, as text.
fn answer(client: &mut UnixStream) -> MetaAnswer {
    let frame = client.read_lane_frame(LIMIT).expect("an answer");
    assert_eq!(frame.kind, Kind::Answered, "{frame:?}");
    let text = std::str::from_utf8(&frame.payload).expect("answer text");
    shared_translator::decode::<MetaAnswer>(text, &mut Ready.text_budget().budget())
        .expect("a meta answer")
}

fn ended(client: &mut UnixStream) {
    let frame = client.read_lane_frame(LIMIT).expect("the end");
    assert_eq!(frame.kind, Kind::Ended, "{frame:?}");
}

#[test]
fn a_configuration_change_by_the_nexus_or_the_translator_reaches_an_open_meta_subscription() {
    let directory = fixture("subscription");
    let place = Place {
        place_name: PlaceName("paired".into()),
        runtime: Runtime(directory.to_string_lossy().into_owned()),
        state: State(directory.to_string_lossy().into_owned()),
    };
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("runtime");
    let this_process = PeerIdentity {
        user_id: UserId(i64::from(nix::unistd::getuid().as_raw())),
        process_id: ProcessId(i64::from(std::process::id())),
    };
    let listeners = Arc::new(
        Listeners::<Paired>::bind(&place, admission(), this_process).expect("the Nexus binds"),
    );
    let nexus = Arc::new(PairedNexus::default());
    {
        let listeners = Arc::clone(&listeners);
        let nexus = Arc::clone(&nexus);
        let runtime = runtime.handle().clone();
        std::thread::spawn(move || {
            while let Ok(stream) = listeners.accept_meta() {
                runtime.spawn(MetaConnection::<PairedNexus>::serve(
                    stream,
                    Arc::clone(&nexus),
                    admission(),
                ));
            }
        });
    }
    let side = TranslatorSide {
        translator: PairedTranslator::default(),
        place,
        subscriptions: MetaSubscriptions::default(),
    };
    std::thread::scope(|scope| {
        let mut subscriber = side.requested(scope, &MetaQuery::Subscribe);
        assert_eq!(
            answer(&mut subscriber),
            MetaAnswer::Opened(Configuration {
                nexus: None,
                translator: None,
                readiness: Readiness::Awaiting(vec![SettingName::Translator, SettingName::Nexus]),
            }),
            "the readiness is the pair's, the translator's setting awaited first"
        );

        let mut translator_set =
            side.requested(scope, &MetaQuery::Configure(Setting::Translator(3)));
        assert_eq!(
            answer(&mut translator_set),
            MetaAnswer::Configured(Setting::Translator(3))
        );
        ended(&mut translator_set);
        assert_eq!(
            answer(&mut subscriber),
            MetaAnswer::Changed(Setting::Translator(3)),
            "the translator's own change reaches the open subscription"
        );

        let mut nexus_set = side.requested(scope, &MetaQuery::Configure(Setting::Nexus(7)));
        assert_eq!(
            answer(&mut nexus_set),
            MetaAnswer::Configured(Setting::Nexus(7))
        );
        ended(&mut nexus_set);
        assert_eq!(
            answer(&mut subscriber),
            MetaAnswer::Changed(Setting::Nexus(7)),
            "the Nexus's change reaches the open subscription"
        );

        let mut reader = side.requested(scope, &MetaQuery::Subscribe);
        assert_eq!(
            answer(&mut reader),
            MetaAnswer::Opened(Configuration {
                nexus: Some(7),
                translator: Some(3),
                readiness: Readiness::Ready,
            }),
            "the pair is ready once both settings are set"
        );

        nexus.finish();
        ended(&mut subscriber);
        ended(&mut reader);
    });
    let _ = std::fs::remove_dir_all(directory);
}
