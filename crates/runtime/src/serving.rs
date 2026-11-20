//! Three listeners and one exchange per accepted connection. Quiet clients
//! and Nexus streams are observed concurrently, without idle timers.
use crate::configuration::{BudgetObserving, ConfigurationObserving};
use crate::{
    BudgetHandle, ConfigurationFollowing, ConfigurationWatch, Emittable, SocketLocating,
    SocketPaths, frame_capacity,
};
use datom_codec::{Actualizing, Budget, Datomizable, Potential};
use nexus::{SignalReceiving, SignalSubmitting, TransmissionError, WireValue};
use nexus_client::{Served, TextReply, TextRequest, TextSocket};
use protos::{Compactable, Protosizable};
use std::{future::Future, io, marker::PhantomData, os::unix::fs::MetadataExt, path::PathBuf};
use tokio::{
    io::{AsyncReadExt, AsyncWrite},
    net::{UnixListener, UnixStream},
    task::JoinSet,
};
use translator::{Call, Caller, OutputMode, ProcessId, Translator, UserId};

#[derive(Debug)]
pub enum RuntimeError {
    Io(io::Error),
    Transmission(TransmissionError),
    Credentials,
    Task(tokio::task::JoinError),
}
impl From<io::Error> for RuntimeError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}
impl From<TransmissionError> for RuntimeError {
    fn from(error: TransmissionError) -> Self {
        Self::Transmission(error)
    }
}

/// The socket path belongs to this listener only while its inode is unchanged.
struct BoundSocket {
    listener: UnixListener,
    path: PathBuf,
    device: u64,
    inode: u64,
}
impl Drop for BoundSocket {
    fn drop(&mut self) {
        if std::fs::symlink_metadata(&self.path)
            .is_ok_and(|metadata| metadata.dev() == self.device && metadata.ino() == self.inode)
        {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}
trait Binding: Sized {
    fn bind(path: PathBuf) -> io::Result<Self>;
}
impl Binding for BoundSocket {
    fn bind(path: PathBuf) -> io::Result<Self> {
        let listener = UnixListener::bind(&path)?;
        let metadata = std::fs::symlink_metadata(&path)?;
        Ok(Self {
            listener,
            path,
            device: metadata.dev(),
            inode: metadata.ino(),
        })
    }
}

/// Running OS resources, separate from the Place-only Translator datum.
pub struct BoundTranslator<C> {
    ordinary: BoundSocket,
    meta: BoundSocket,
    emission: BoundSocket,
    paths: SocketPaths,
    configuration: ConfigurationWatch,
    budget: BudgetHandle,
    component: PhantomData<fn() -> C>,
}

pub trait BindingTranslator<C: ConfigurationFollowing> {
    fn bind(self) -> Result<BoundTranslator<C>, RuntimeError>;
}
pub trait ConfigurationWatching {
    fn configuration(&self) -> crate::ConfigurationObservation;
}
impl<C> ConfigurationWatching for BoundTranslator<C> {
    fn configuration(&self) -> crate::ConfigurationObservation {
        self.budget.observation()
    }
}
pub trait Serving {
    fn serve(self) -> impl Future<Output = Result<(), RuntimeError>> + Send;
}
impl<C: ConfigurationFollowing> BindingTranslator<C> for Translator {
    fn bind(self) -> Result<BoundTranslator<C>, RuntimeError> {
        let paths = self.socket_paths::<C>();
        std::fs::create_dir_all(&self.place.runtime.0)?;
        let ordinary = BoundSocket::bind(paths.ordinary_text.clone())?;
        let meta = BoundSocket::bind(paths.meta_text.clone())?;
        let emission = BoundSocket::bind(paths.emission.clone())?;
        let (configuration, budget) = ConfigurationWatch::create(paths.meta_nexus.clone())?;
        Ok(BoundTranslator {
            ordinary,
            meta,
            emission,
            paths,
            configuration,
            budget,
            component: PhantomData,
        })
    }
}

#[derive(Clone)]
struct Context {
    paths: SocketPaths,
    budget: BudgetHandle,
}
trait Handling {
    fn text<C: ConfigurationFollowing>(
        self,
        stream: UnixStream,
        socket: TextSocket,
    ) -> impl Future<Output = Result<(), RuntimeError>> + Send;
    fn emission<C: ConfigurationFollowing>(
        stream: UnixStream,
    ) -> impl Future<Output = Result<(), RuntimeError>> + Send;
}
impl Handling for Context {
    async fn text<C: ConfigurationFollowing>(
        self,
        mut stream: UnixStream,
        socket: TextSocket,
    ) -> Result<(), RuntimeError> {
        let peer = stream.peer_cred()?;
        let caller = Caller {
            process_id: ProcessId(peer.pid().map(i64::from).ok_or(RuntimeError::Credentials)?),
            user_id: UserId(i64::from(peer.uid())),
        };
        let request = stream.receive::<TextRequest>(frame_capacity()).await?;
        let (mut input, mut output) = stream.into_split();
        // Losing the caller closes even a quiet budget wait or Nexus read.
        let mut byte = [0];
        tokio::select! {
            result = self.answer::<C, _>(request, caller, socket, &mut output) => result,
            _ = input.read(&mut byte) => Ok(()),
        }
    }
    async fn emission<C: ConfigurationFollowing>(
        mut stream: UnixStream,
    ) -> Result<(), RuntimeError> {
        let emission = stream.receive::<C::Emission>(frame_capacity()).await?;
        let outcome = emission.emit().await;
        stream.submit(outcome, frame_capacity()).await?;
        Ok(())
    }
}
trait Answering {
    fn answer<C: ConfigurationFollowing, W: AsyncWrite + Unpin + Send>(
        &self,
        request: TextRequest,
        caller: Caller,
        socket: TextSocket,
        output: &mut W,
    ) -> impl Future<Output = Result<(), RuntimeError>> + Send;
    fn relay<
        I: WireValue + datom_codec::Composing,
        O: WireValue + Datomizable,
        W: AsyncWrite + Unpin + Send,
    >(
        &self,
        text: String,
        caller: Caller,
        path: &std::path::Path,
        budget: Budget,
        mode: fn(&O) -> OutputMode,
        output: &mut W,
    ) -> impl Future<Output = Result<(), RuntimeError>> + Send;
}
impl Answering for Context {
    async fn answer<C: ConfigurationFollowing, W: AsyncWrite + Unpin + Send>(
        &self,
        request: TextRequest,
        caller: Caller,
        socket: TextSocket,
        output: &mut W,
    ) -> Result<(), RuntimeError> {
        let Some(budget) = self.budget.current().await else {
            output
                .submit(TextReply::Served(Served::NexusDown), frame_capacity())
                .await?;
            return Ok(());
        };
        match request {
            TextRequest::Describe => {
                let text = match socket {
                    TextSocket::Ordinary => C::describe(),
                    TextSocket::Meta => C::describe_meta(),
                };
                output
                    .submit(TextReply::Datom(text), frame_capacity())
                    .await?;
                output
                    .submit(TextReply::Served(Served::Answered), frame_capacity())
                    .await?;
                Ok(())
            }
            TextRequest::Datom(text) => match socket {
                TextSocket::Ordinary => {
                    self.relay::<C::Input, C::Output, _>(
                        text,
                        caller,
                        &self.paths.ordinary_nexus,
                        budget,
                        C::output_mode,
                        output,
                    )
                    .await
                }
                TextSocket::Meta => {
                    self.relay::<C::MetaInput, C::MetaOutput, _>(
                        text,
                        caller,
                        &self.paths.meta_nexus,
                        budget,
                        C::meta_output_mode,
                        output,
                    )
                    .await
                }
            },
        }
    }
    async fn relay<
        I: WireValue + datom_codec::Composing,
        O: WireValue + Datomizable,
        W: AsyncWrite + Unpin + Send,
    >(
        &self,
        text: String,
        caller: Caller,
        path: &std::path::Path,
        mut budget: Budget,
        mode: fn(&O) -> OutputMode,
        output: &mut W,
    ) -> Result<(), RuntimeError> {
        let input = match Potential::<I>::from(text).actualize(&mut budget) {
            Ok(input) => input,
            Err(error) => {
                output
                    .submit(
                        TextReply::Served(Served::Unreadable(error)),
                        frame_capacity(),
                    )
                    .await?;
                return Ok(());
            }
        };
        let mut nexus = match UnixStream::connect(path).await {
            Ok(nexus) => nexus,
            Err(_) => {
                output
                    .submit(TextReply::Served(Served::NexusDown), frame_capacity())
                    .await?;
                return Ok(());
            }
        };
        if nexus
            .submit(Call { caller, input }, frame_capacity())
            .await
            .is_err()
        {
            output
                .submit(TextReply::Served(Served::NexusDown), frame_capacity())
                .await?;
            return Ok(());
        }
        let first = match nexus.receive::<O>(frame_capacity()).await {
            Ok(first) => first,
            Err(_) => {
                output
                    .submit(TextReply::Served(Served::NexusDown), frame_capacity())
                    .await?;
                return Ok(());
            }
        };
        let streaming = mode(&first);
        output
            .submit(
                TextReply::Datom(first.datomize(vec![]).protosize().compact()),
                frame_capacity(),
            )
            .await?;
        if streaming == OutputMode::Single {
            output
                .submit(TextReply::Served(Served::Answered), frame_capacity())
                .await?;
            return Ok(());
        }
        loop {
            match nexus.receive::<O>(frame_capacity()).await {
                Ok(reply) if mode(&reply) == OutputMode::Streaming => {
                    output
                        .submit(
                            TextReply::Datom(reply.datomize(vec![]).protosize().compact()),
                            frame_capacity(),
                        )
                        .await?;
                }
                _ => {
                    output
                        .submit(TextReply::Served(Served::NexusDown), frame_capacity())
                        .await?;
                    return Ok(());
                }
            }
        }
    }
}
impl<C: ConfigurationFollowing> Serving for BoundTranslator<C> {
    async fn serve(self) -> Result<(), RuntimeError> {
        let Self {
            ordinary,
            meta,
            emission,
            paths,
            configuration,
            budget,
            component: _,
        } = self;
        let context = Context { paths, budget };
        let mut connections = JoinSet::new();
        let caller = Caller {
            process_id: ProcessId(i64::from(std::process::id())),
            user_id: UserId(i64::from(rustix::process::getuid().as_raw())),
        };
        let configuration = configuration.run::<C>(caller);
        tokio::pin!(configuration);
        loop {
            tokio::select! {
                result = ordinary.listener.accept() => {
                    let (stream, _) = result?; let context = context.clone();
                    connections.spawn(async move { context.text::<C>(stream, TextSocket::Ordinary).await });
                }
                result = meta.listener.accept() => {
                    let (stream, _) = result?; let context = context.clone();
                    connections.spawn(async move { context.text::<C>(stream, TextSocket::Meta).await });
                }
                result = emission.listener.accept() => {
                    let (stream, _) = result?;
                    connections.spawn(Context::emission::<C>(stream));
                }
                result = &mut configuration => { result?; return Ok(()); }
                result = connections.join_next(), if !connections.is_empty() => {
                    if let Some(Err(error)) = result { return Err(RuntimeError::Task(error)); }
                }
            }
        }
    }
}
