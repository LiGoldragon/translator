//! The meta connections following a subscription of the Nexus's, which the
//! translator's own setting changes are carried into.
//!
//! Hand-written: it holds sockets and a lock, which Ethos cannot name.

use std::os::unix::net::UnixStream;
use std::sync::{Mutex, MutexGuard, PoisonError};

use nexus_exchange::lane::{Frame, LaneWriting};
use nexus_exchange::{DeadlineStream, ValidAdmission};

use crate::error::TranslatorError;

/// Every meta connection following a subscription of the Nexus's. Each
/// answer written to one, the Nexus's and the translator's own, is written
/// under one lock, so a change the translator announces lands between two
/// whole answers and after the subscription's first.
#[derive(Default)]
pub struct MetaSubscriptions {
    open: Mutex<OpenSubscriptions>,
}

#[derive(Default)]
struct OpenSubscriptions {
    next: SubscriptionNumber,
    carried: Vec<CarriedSubscription>,
}

/// One subscription's place among those open.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct SubscriptionNumber(u64);

struct CarriedSubscription {
    number: SubscriptionNumber,
    stream: UnixStream,
}

pub(crate) trait SubscriptionCarrying {
    /// Writes one answer of a subscription to its client: `answer` is
    /// composed and written with every other write to an open subscription
    /// held back, and the client is carried from its first answer on, its
    /// number kept in `carried`, so an announcement reaches it after that
    /// answer and never before it.
    fn answer(
        &self,
        carried: &mut Option<SubscriptionNumber>,
        client: &UnixStream,
        admission: &ValidAdmission,
        answer: impl FnOnce() -> Result<Frame, TranslatorError>,
    ) -> Result<(), TranslatorError>;

    /// The subscription is no longer carried: its exchange has ended.
    fn withdraw(&self, carried: Option<SubscriptionNumber>);

    /// Writes `frame` to every open subscription, each within the admission
    /// deadline; one that cannot be written is no longer carried.
    fn announce(&self, frame: &Frame, admission: &ValidAdmission);
}

impl MetaSubscriptions {
    fn open(&self) -> MutexGuard<'_, OpenSubscriptions> {
        self.open.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl SubscriptionCarrying for MetaSubscriptions {
    fn answer(
        &self,
        carried: &mut Option<SubscriptionNumber>,
        client: &UnixStream,
        admission: &ValidAdmission,
        answer: impl FnOnce() -> Result<Frame, TranslatorError>,
    ) -> Result<(), TranslatorError> {
        let mut open = self.open();
        let frame = answer()?;
        if carried.is_none() {
            let stream = client.try_clone().map_err(TranslatorError::Io)?;
            let number = open.next;
            open.next = SubscriptionNumber(number.0 + 1);
            open.carried.push(CarriedSubscription { number, stream });
            *carried = Some(number);
        }
        client.written(&frame, admission)
    }

    fn withdraw(&self, carried: Option<SubscriptionNumber>) {
        if let Some(number) = carried {
            self.open()
                .carried
                .retain(|subscription| subscription.number != number);
        }
    }

    fn announce(&self, frame: &Frame, admission: &ValidAdmission) {
        self.open()
            .carried
            .retain(|subscription| subscription.stream.written(frame, admission).is_ok());
    }
}

/// A client connection one whole lane frame is written to.
trait Written {
    /// Writes `frame` within the admission deadline from now.
    fn written(&self, frame: &Frame, admission: &ValidAdmission) -> Result<(), TranslatorError>;
}

impl Written for UnixStream {
    fn written(&self, frame: &Frame, admission: &ValidAdmission) -> Result<(), TranslatorError> {
        let mut output = DeadlineStream::new(
            self.try_clone().map_err(TranslatorError::Io)?,
            admission.deadline,
        )
        .map_err(TranslatorError::Io)?;
        output
            .write_lane_frame(frame, admission.frame_capacity)
            .map_err(TranslatorError::Transport)
    }
}
