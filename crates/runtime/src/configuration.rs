//! The one observed configuration subscription. Socket events and unavailable
//! text arrivals request a dial; concurrent arrivals share its observation.
use crate::{ConfigurationFollowing, frame_capacity};
use datom_codec::Budget;
use nexus::{SignalReceiving, SignalSubmitting};
use nix::sys::inotify::{AddWatchFlags, InitFlags, Inotify};
use std::{future::Future, io, os::unix::ffi::OsStrExt, path::Path, sync::Arc};
use tokio::{
    io::unix::AsyncFd,
    net::UnixStream,
    sync::{mpsc, watch},
};
use translator::{Call, Caller, DatomReadingBudget};

pub trait Budgeting {
    fn reading_budget(&self) -> Option<Budget>;
}
impl Budgeting for DatomReadingBudget {
    fn reading_budget(&self) -> Option<Budget> {
        if self.reading_steps.0 <= 0 || self.reading_depth.0 <= 0 {
            return None;
        }
        Some(Budget {
            remaining: self.reading_steps.0,
            reader: protos::ReaderBudget::try_from(usize::try_from(self.reading_steps.0).ok()?)
                .ok()?,
            depth: 0,
            maximum_depth: self.reading_depth.0,
        })
    }
}

pub(crate) struct Observation {
    budget: Option<DatomReadingBudget>,
}
#[derive(Clone)]
pub(crate) struct BudgetHandle {
    observed: watch::Receiver<Arc<Observation>>,
    dial: mpsc::UnboundedSender<Arc<Observation>>,
}
/// A read-only subscription to the currently observed Nexus budget.
/// This is a channel resource, not another configured value.
pub struct ConfigurationObservation {
    observed: watch::Receiver<Arc<Observation>>,
}
pub trait ObservingConfiguration {
    fn budget(&self) -> Option<DatomReadingBudget>;
    fn changed(
        &mut self,
    ) -> impl Future<Output = Result<Option<DatomReadingBudget>, watch::error::RecvError>> + Send;
}
impl ObservingConfiguration for ConfigurationObservation {
    fn budget(&self) -> Option<DatomReadingBudget> {
        self.observed.borrow().budget.clone()
    }
    async fn changed(&mut self) -> Result<Option<DatomReadingBudget>, watch::error::RecvError> {
        self.observed.changed().await?;
        Ok(self.budget())
    }
}
pub(crate) trait BudgetObserving {
    fn observation(&self) -> ConfigurationObservation;
    fn current(&self) -> impl Future<Output = Option<Budget>> + Send;
}
impl BudgetObserving for BudgetHandle {
    fn observation(&self) -> ConfigurationObservation {
        ConfigurationObservation {
            observed: self.observed.clone(),
        }
    }
    async fn current(&self) -> Option<Budget> {
        let mut observed = self.observed.clone();
        let before = observed.borrow().clone();
        if let Some(budget) = &before.budget {
            return budget.reading_budget();
        }
        self.dial.send(before.clone()).ok()?;
        loop {
            observed.changed().await.ok()?;
            let next = observed.borrow().clone();
            if !Arc::ptr_eq(&before, &next) {
                return next.budget.as_ref()?.reading_budget();
            }
        }
    }
}

pub(crate) struct ConfigurationWatch {
    socket: std::path::PathBuf,
    watcher: AsyncFd<Inotify>,
    requests: mpsc::UnboundedReceiver<Arc<Observation>>,
    observed: watch::Sender<Arc<Observation>>,
}
pub(crate) trait ConfigurationObserving: Sized {
    fn create(socket: std::path::PathBuf) -> io::Result<(Self, BudgetHandle)>;
    fn publish(&self, budget: Option<DatomReadingBudget>);
    fn socket_event(
        watcher: &AsyncFd<Inotify>,
        path: &Path,
    ) -> impl Future<Output = io::Result<()>> + Send;
    fn connect<C: ConfigurationFollowing>(
        &self,
        caller: Caller,
    ) -> impl Future<Output = Option<UnixStream>> + Send;
    fn run<C: ConfigurationFollowing>(
        self,
        caller: Caller,
    ) -> impl Future<Output = io::Result<()>> + Send;
}
impl ConfigurationObserving for ConfigurationWatch {
    fn create(socket: std::path::PathBuf) -> io::Result<(Self, BudgetHandle)> {
        let watcher = Inotify::init(InitFlags::IN_NONBLOCK | InitFlags::IN_CLOEXEC)
            .map_err(io::Error::from)?;
        watcher
            .add_watch(
                socket.parent().ok_or(io::ErrorKind::InvalidInput)?,
                AddWatchFlags::IN_CREATE | AddWatchFlags::IN_MOVED_TO,
            )
            .map_err(io::Error::from)?;
        let watcher = AsyncFd::new(watcher)?;
        let (observed, receiver) = watch::channel(Arc::new(Observation { budget: None }));
        let (dial, requests) = mpsc::unbounded_channel();
        Ok((
            Self {
                socket,
                watcher,
                requests,
                observed,
            },
            BudgetHandle {
                observed: receiver,
                dial,
            },
        ))
    }
    fn publish(&self, budget: Option<DatomReadingBudget>) {
        self.observed.send_replace(Arc::new(Observation { budget }));
    }
    async fn socket_event(watcher: &AsyncFd<Inotify>, path: &Path) -> io::Result<()> {
        loop {
            let mut ready = watcher.readable().await?;
            match ready.try_io(|inner| inner.get_ref().read_events().map_err(io::Error::from)) {
                Ok(Ok(events)) => {
                    if events.iter().any(|event| {
                        event.name.as_ref().is_some_and(|name| {
                            path.file_name()
                                .is_some_and(|expected| name.as_bytes() == expected.as_bytes())
                        })
                    }) {
                        return Ok(());
                    }
                }
                Ok(Err(error)) => return Err(error),
                Err(_) => {}
            }
        }
    }
    async fn connect<C: ConfigurationFollowing>(&self, caller: Caller) -> Option<UnixStream> {
        let attempt = async {
            let mut stream = UnixStream::connect(&self.socket).await.ok()?;
            stream
                .submit(
                    Call {
                        caller,
                        input: C::subscribe_configuration(),
                    },
                    frame_capacity(),
                )
                .await
                .ok()?;
            let output = stream
                .receive::<C::MetaOutput>(frame_capacity())
                .await
                .ok()?;
            let budget = C::snapshot(&output)?;
            budget.reading_budget()?;
            Some((stream, budget))
        }
        .await;
        match attempt {
            Some((stream, budget)) => {
                self.publish(Some(budget));
                Some(stream)
            }
            None => {
                self.publish(None);
                None
            }
        }
    }
    async fn run<C: ConfigurationFollowing>(mut self, caller: Caller) -> io::Result<()> {
        let mut subscription = self.connect::<C>(caller.clone()).await;
        loop {
            enum Event<T> {
                Output(T),
                Socket(io::Result<()>),
                Request(Option<Arc<Observation>>),
            }
            let event = if let Some(stream) = &mut subscription {
                tokio::select! {
                    output = stream.receive::<C::MetaOutput>(frame_capacity()) => Event::Output(output),
                    event = Self::socket_event(&self.watcher, &self.socket) => Event::Socket(event),
                    request = self.requests.recv() => Event::Request(request),
                }
            } else {
                tokio::select! {
                    event = Self::socket_event(&self.watcher, &self.socket) => Event::Socket(event),
                    request = self.requests.recv() => Event::Request(request),
                }
            };
            match event {
                Event::Output(output) => {
                    let next = output
                        .ok()
                        .and_then(|output| {
                            self.observed
                                .borrow()
                                .budget
                                .as_ref()
                                .and_then(|budget| C::after(budget, &output))
                        })
                        .filter(|budget| budget.reading_budget().is_some());
                    if let Some(budget) = next {
                        self.publish(Some(budget));
                    } else {
                        subscription = None;
                        self.publish(None);
                    }
                }
                Event::Socket(event) => {
                    event?;
                    drop(subscription.take());
                    self.publish(None);
                    subscription = self.connect::<C>(caller.clone()).await;
                }
                Event::Request(Some(observed)) => {
                    let current = self.observed.borrow().clone();
                    if subscription.is_none() && Arc::ptr_eq(&current, &observed) {
                        subscription = self.connect::<C>(caller.clone()).await;
                    }
                }
                Event::Request(None) => return Ok(()),
            }
        }
    }
}
