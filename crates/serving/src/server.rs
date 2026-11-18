//! A translator's two sockets, each accepted on a thread of its own and each
//! connection served on a thread of its own.

use std::sync::Arc;

use nexus_exchange::{Named, RetryingAccept, ValidAdmission};
use nexus_serving::{OwnedListener, PairedPeer, PeerMatching, SocketBinding, SocketPaths};
use shared_translator::placement::Place;

use crate::budget::ValidTextBudget;
use crate::connection::{Connection, ConnectionServing};
use crate::error::TranslatorError;
use crate::generated::{Endpoint, TranslatorBootstrap};
use crate::subscriptions::MetaSubscriptions;
use crate::translating::{EmissionAnswering, MetaTranslating, PublicTranslating, Translator};

pub trait TranslatorServing {
    /// Binds both translator sockets of the place the bootstrap names, pins
    /// the Nexus it names, and serves the meta socket on a thread of its own
    /// and the ordinary socket on this one. It returns only when the ordinary
    /// listener fails; the meta listener failing ends the process.
    fn serve(self, bootstrap: &TranslatorBootstrap) -> Result<(), TranslatorError>;
}

impl<T> TranslatorServing for T
where
    T: PublicTranslating + MetaTranslating + EmissionAnswering,
{
    fn serve(self, bootstrap: &TranslatorBootstrap) -> Result<(), TranslatorError> {
        let admission = ValidAdmission::try_from(&bootstrap.admission)
            .map_err(|message| TranslatorError::Signal(message.into()))?;
        let bootstrap_budget = ValidTextBudget::try_from(&bootstrap.text_limits)
            .map_err(|_| TranslatorError::Signal("invalid bootstrap text budget".into()))?;
        let paths = SocketPaths::<T::Component>::from(&bootstrap.place);
        let public = OwnedListener::bind(paths.translator).map_err(TranslatorError::Io)?;
        let meta = OwnedListener::bind(paths.meta_translator).map_err(TranslatorError::Io)?;
        let nexus =
            PairedPeer::capture(bootstrap.peer_identity.clone()).map_err(TranslatorError::Io)?;
        let listening = Listening {
            translator: Arc::new(self),
            place: Arc::new(bootstrap.place.clone()),
            admission,
            bootstrap_budget,
            nexus: Arc::new(nexus),
            subscriptions: Arc::new(MetaSubscriptions::default()),
        };
        let meta_listening = listening.clone();
        std::thread::spawn(move || {
            if let Err(error) = meta_listening.accept(&meta, Endpoint::Meta) {
                let title = T::Component::title().0;
                eprintln!("{title} translator meta listener failed permanently: {error}");
                std::process::exit(1);
            }
        });
        listening.accept(&public, Endpoint::Public)
    }
}

/// What every connection of a translator is served under.
struct Listening<T> {
    translator: Arc<T>,
    place: Arc<Place>,
    admission: ValidAdmission,
    bootstrap_budget: ValidTextBudget,
    nexus: Arc<PairedPeer>,
    subscriptions: Arc<MetaSubscriptions>,
}

impl<T> Clone for Listening<T> {
    fn clone(&self) -> Self {
        Self {
            translator: Arc::clone(&self.translator),
            place: Arc::clone(&self.place),
            admission: self.admission,
            bootstrap_budget: self.bootstrap_budget,
            nexus: Arc::clone(&self.nexus),
            subscriptions: Arc::clone(&self.subscriptions),
        }
    }
}

impl<T> Listening<T>
where
    T: PublicTranslating + MetaTranslating + EmissionAnswering,
{
    /// Accepts every connection to `listener`, each served on a thread of its
    /// own, until accepting fails for good.
    fn accept(&self, listener: &OwnedListener, endpoint: Endpoint) -> Result<(), TranslatorError> {
        loop {
            let (mut stream, _) = listener
                .listener()
                .accept_with_retry(self.admission.retry_delay)
                .map_err(TranslatorError::Io)?;
            let listening = self.clone();
            let endpoint = endpoint.clone();
            std::thread::spawn(move || {
                let connection = Connection {
                    translator: listening.translator.as_ref(),
                    place: listening.place.as_ref(),
                    endpoint,
                    admission: listening.admission,
                    bootstrap_budget: listening.bootstrap_budget,
                    nexus: Some(listening.nexus.as_ref()),
                    subscriptions: listening.subscriptions.as_ref(),
                };
                if let Err(error) = connection.serve(&mut stream) {
                    let title = <T as Translator>::Component::title().0;
                    eprintln!("{title} translator request failed: {error}");
                }
            });
        }
    }
}
