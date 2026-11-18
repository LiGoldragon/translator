//! One connection to a translator socket: one request, and its answers.

use std::io::Write;
use std::os::unix::net::UnixStream;
use std::time::Instant;

use nexus_exchange::lane::{
    Frame, Kind, Lane, LaneReading, LaneWriting, PeerCalling, Textual, TransportError,
};
use nexus_exchange::{DeadlineStream, ValidAdmission};
use nexus_querying::Patience;
use nexus_serving::{PairedPeer, PeerMatching, SocketPaths};
use shared_translator::Caller;
use shared_translator::placement::Place;
use signal::{ByteViewable, FrameCapacity, Signalizable};

use crate::budget::{Budgeting, TextBudgeting, ValidTextBudget};
use crate::calling::{EmissionRestoring, NexusCalling};
use crate::error::TranslatorError;
use crate::generated::{Endpoint, Refusal};
use crate::subscriptions::{MetaSubscriptions, SubscriptionCarrying};
use crate::translating::{EmissionAnswering, MetaTranslating, PublicTranslating, Settling};

/// One connection to one of a translator's sockets, and what it is served
/// under.
pub struct Connection<'a, T> {
    pub translator: &'a T,
    pub place: &'a Place,
    /// The socket the connection came in on.
    pub endpoint: Endpoint,
    pub admission: ValidAdmission,
    /// The budget meta text is read within: the bootstrap's, so the meta lane
    /// reads before the translator's own settings are set.
    pub bootstrap_budget: ValidTextBudget,
    /// The Nexus the translator is paired with; `None` admits no emission.
    pub nexus: Option<&'a PairedPeer>,
    /// The meta connections following a subscription of the Nexus's, which
    /// the translator's own changes are carried into.
    pub subscriptions: &'a MetaSubscriptions,
}

pub trait ConnectionServing {
    /// Serves one connection: its one request frame, read within the
    /// admission deadline, then its answers. A text request is answered with
    /// each answer of its exchange and then `Ended`, or with `Failed`
    /// carrying the refusal or failure that ended it; an emission with its
    /// one outcome, or `Failed`.
    fn serve(&self, stream: &mut UnixStream) -> Result<(), TranslatorError>;
}

impl<T> ConnectionServing for Connection<'_, T>
where
    T: PublicTranslating + MetaTranslating + EmissionAnswering,
{
    fn serve(&self, stream: &mut UnixStream) -> Result<(), TranslatorError> {
        let caller = stream.peer_caller().map_err(TranslatorError::Transport)?;
        let mut io = self.deadline(stream)?;
        let request = io
            .read_lane_frame(self.admission.frame_capacity)
            .map_err(TranslatorError::Transport)?;
        if request.kind != Kind::Request {
            return Err(TranslatorError::Transport(TransportError::InvalidLength));
        }
        if let Some(refusal) = self.refusal(&request.lane, &caller) {
            return self.fail(&mut io, request.lane, &refusal.to_string());
        }
        drop(io);
        match request.lane {
            Lane::EmissionSignal => {
                let response = self
                    .emission(request.payload)
                    .unwrap_or_else(|error| Frame {
                        lane: Lane::EmissionSignal,
                        kind: Kind::Failed,
                        payload: error.to_string().into_bytes(),
                    });
                self.write(stream, &response)
            }
            Lane::PublicText => {
                let answered = self.public(stream, &request, caller);
                self.end(stream, Lane::PublicText, answered)
            }
            Lane::MetaText => {
                let answered = self.meta(stream, &request, caller);
                self.end(stream, Lane::MetaText, answered)
            }
        }
    }
}

impl<T> Connection<'_, T>
where
    T: PublicTranslating + MetaTranslating + EmissionAnswering,
{
    /// Why a request on `lane` from `caller` is refused before it is read: a
    /// lane the socket does not carry, a caller of another login, or an
    /// emission from any process but the paired Nexus.
    fn refusal(&self, lane: &Lane, caller: &Caller) -> Option<Refusal> {
        let carried = match self.endpoint {
            Endpoint::Public => matches!(lane, Lane::PublicText | Lane::EmissionSignal),
            Endpoint::Meta => *lane == Lane::MetaText,
        };
        if !carried {
            return Some(Refusal::LaneUnavailable);
        }
        if caller.user_id.0 != i64::from(nix::unistd::getuid().as_raw()) {
            return Some(Refusal::ForeignLogin);
        }
        if *lane == Lane::EmissionSignal && !self.nexus.is_some_and(|nexus| nexus.matches(caller)) {
            return Some(Refusal::UnpairedEmitter);
        }
        None
    }

    /// Reads the ordinary text into the public query within the translator's
    /// own budget, carries it to the Nexus, and writes back every answer of
    /// its exchange as it comes, until the Nexus ends it.
    fn public(
        &self,
        stream: &UnixStream,
        request: &Frame,
        caller: Caller,
    ) -> Result<(), TranslatorError> {
        let ready = self
            .translator
            .ready()
            .ok_or(TranslatorError::Refused(Refusal::AwaitingSettings))?;
        let mut budget = ready.text_budget().budget();
        let text = request.text().map_err(TranslatorError::Transport)?;
        let query = shared_translator::decode::<T::PublicInput>(text, &mut budget)
            .map_err(|error| TranslatorError::Signal(format!("{error:?}")))?;
        T::public_call(caller, query).call(
            &SocketPaths::<T::Component>::from(self.place).public,
            Patience::Progress(self.admission.deadline),
            FrameCapacity::from(self.admission.frame_capacity),
            |response| {
                let output = T::public_answered(response)?;
                self.write(stream, &Self::text(Lane::PublicText, &output))
            },
        )
    }

    /// Reads the meta text within the bootstrap budget and settles it: the
    /// translator's own part answered here and announced to every open
    /// subscription, the Nexus's carried to its meta socket within one
    /// deadline, and a subscription of the Nexus's followed for as long as
    /// the Nexus keeps it open, the translator's own changes carried into it.
    fn meta(
        &self,
        stream: &UnixStream,
        request: &Frame,
        caller: Caller,
    ) -> Result<(), TranslatorError> {
        let text = request.text().map_err(TranslatorError::Transport)?;
        let mut budget = self.bootstrap_budget.budget();
        let query = shared_translator::decode::<T::MetaInput>(text, &mut budget)
            .map_err(|error| TranslatorError::Signal(format!("{error:?}")))?;
        let socket = SocketPaths::<T::Component>::from(self.place).meta;
        let capacity = FrameCapacity::from(self.admission.frame_capacity);
        match self.translator.settle(query) {
            Settling::Answered(output) => {
                self.write(stream, &Self::text(Lane::MetaText, &output))?;
                if let Some(announced) = self.translator.announced(&output) {
                    self.subscriptions
                        .announce(&Self::text(Lane::MetaText, &announced), &self.admission);
                }
                Ok(())
            }
            Settling::Forwarded(input) => {
                let deadline = Instant::now()
                    .checked_add(self.admission.deadline)
                    .ok_or_else(|| TranslatorError::Signal("meta call deadline overflow".into()))?;
                T::meta_call(caller, input).call(
                    &socket,
                    Patience::Until(deadline),
                    capacity,
                    |response| {
                        let output = self.translator.meta_answered(response)?;
                        self.write(stream, &Self::text(Lane::MetaText, &output))
                    },
                )
            }
            Settling::Subscribed(input) => {
                let mut carried = None;
                let followed = T::meta_call(caller, input).call(
                    &socket,
                    Patience::Progress(self.admission.deadline),
                    capacity,
                    |response| {
                        self.subscriptions
                            .answer(&mut carried, stream, &self.admission, || {
                                let output = self.translator.meta_answered(response)?;
                                Ok(Self::text(Lane::MetaText, &output))
                            })
                    },
                );
                self.subscriptions.withdraw(carried);
                followed
            }
        }
    }

    /// Restores the paired Nexus's emission and answers it with what the
    /// translator observed doing it.
    fn emission(&self, payload: Vec<u8>) -> Result<Frame, TranslatorError> {
        let emission = T::Emission::restored(payload)?;
        let ready = self
            .translator
            .ready()
            .ok_or(TranslatorError::Refused(Refusal::AwaitingSettings))?;
        let outcome = self
            .translator
            .emitted(&ready, emission)
            .signalize()
            .map_err(|error| TranslatorError::Signal(format!("{error:?}")))?;
        Ok(Frame {
            lane: Lane::EmissionSignal,
            kind: Kind::Answered,
            payload: outcome.bytes().to_vec(),
        })
    }

    /// Ends a text answer: `Ended` after its last answer, or `Failed`
    /// carrying what ended it.
    fn end(
        &self,
        stream: &UnixStream,
        lane: Lane,
        answered: Result<(), TranslatorError>,
    ) -> Result<(), TranslatorError> {
        let ending = match answered {
            Ok(()) => Frame {
                lane,
                kind: Kind::Ended,
                payload: Vec::new(),
            },
            Err(error) => Frame {
                lane,
                kind: Kind::Failed,
                payload: error.to_string().into_bytes(),
            },
        };
        self.write(stream, &ending)
    }

    /// An answer on a text lane: the value as its canonical text.
    fn text(lane: Lane, value: &impl datom_codec::Datomizable) -> Frame {
        Frame {
            lane,
            kind: Kind::Answered,
            payload: shared_translator::encode(value).into_bytes(),
        }
    }

    /// The connection again, its every read and write within the admission
    /// deadline from now.
    fn deadline(&self, stream: &UnixStream) -> Result<DeadlineStream, TranslatorError> {
        DeadlineStream::new(
            stream.try_clone().map_err(TranslatorError::Io)?,
            self.admission.deadline,
        )
        .map_err(TranslatorError::Io)
    }

    /// Writes one frame back within the admission deadline.
    fn write(&self, stream: &UnixStream, frame: &Frame) -> Result<(), TranslatorError> {
        let mut output = self.deadline(stream)?;
        output
            .write_lane_frame(frame, self.admission.frame_capacity)
            .map_err(TranslatorError::Transport)
    }

    /// Answers the request on `lane` with a failure carrying `message`.
    fn fail(
        &self,
        output: &mut impl Write,
        lane: Lane,
        message: &str,
    ) -> Result<(), TranslatorError> {
        output
            .write_lane_frame(
                &Frame {
                    lane,
                    kind: Kind::Failed,
                    payload: message.as_bytes().to_vec(),
                },
                self.admission.frame_capacity,
            )
            .map_err(TranslatorError::Transport)
    }
}
