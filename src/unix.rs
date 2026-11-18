//! Policy-free Unix transport mechanics shared by Nexus translators.

use crate::{Caller, GroupId, PeerIdentity, ProcessId, UserId};
use std::io::{Read, Write};
use std::os::fd::{AsFd, OwnedFd};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::RecvTimeoutError;
use std::time::{Duration, Instant};

use notify::{RecursiveMode, Watcher as _};

pub use crate::awaiting::{AwaitRefusal, SocketFault};

pub const HEADER_BYTES: usize = 2;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub lane: u8,
    pub kind: u8,
    pub payload: Vec<u8>,
}

#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    InvalidLimit,
    InvalidLength,
    TooLarge,
    PeerCredential(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "Unix translator transport error: {self:?}")
    }
}
impl std::error::Error for Error {}
impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self { Self::Io(error) }
}

pub struct DeadlineStream {
    stream: std::os::unix::net::UnixStream,
    deadline: Instant,
}

impl DeadlineStream {
    pub fn new(stream: std::os::unix::net::UnixStream, timeout: Duration) -> std::io::Result<Self> {
        let deadline = Instant::now().checked_add(timeout)
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "deadline overflow"))?;
        Ok(Self { stream, deadline })
    }
    fn remaining(&self) -> std::io::Result<Duration> {
        self.deadline.checked_duration_since(Instant::now()).filter(|value| !value.is_zero())
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::TimedOut, "attempt deadline elapsed"))
    }
}
impl Read for DeadlineStream {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        self.stream.set_read_timeout(Some(self.remaining()?))?;
        self.stream.read(buffer)
    }
}
impl Write for DeadlineStream {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        self.stream.set_write_timeout(Some(self.remaining()?))?;
        self.stream.write(buffer)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.stream.set_write_timeout(Some(self.remaining()?))?;
        self.stream.flush()
    }
}

fn validate_limit(limit: usize) -> Result<(), Error> {
    if (HEADER_BYTES..=signal::MAXIMUM_SIGNAL_BYTES).contains(&limit) { Ok(()) }
    else { Err(Error::InvalidLimit) }
}

pub fn read_frame(input: &mut impl Read, limit: usize) -> Result<Frame, Error> {
    validate_limit(limit)?;
    let mut prefix = [0_u8; 4];
    input.read_exact(&mut prefix)?;
    let total = usize::try_from(u32::from_be_bytes(prefix)).map_err(|_| Error::InvalidLength)?;
    if total < HEADER_BYTES { return Err(Error::InvalidLength); }
    if total > limit { return Err(Error::TooLarge); }
    let mut body = vec![0_u8; total];
    input.read_exact(&mut body)?;
    Ok(Frame { lane: body[0], kind: body[1], payload: body.split_off(HEADER_BYTES) })
}

fn read_idle_byte(input: &mut impl Read, byte: &mut u8) -> std::io::Result<bool> {
    loop {
        match input.read(std::slice::from_mut(byte)) {
            Ok(0) => return Ok(false),
            Ok(1) => return Ok(true),
            Ok(_) => unreachable!("a one-byte buffer read at most one byte"),
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
}

pub fn read_frame_after_idle(
    mut stream: std::os::unix::net::UnixStream,
    progress_timeout: Duration,
    limit: usize,
) -> Result<Option<Frame>, Error> {
    validate_limit(limit)?;
    stream.set_read_timeout(None)?;
    let mut prefix = [0_u8; 4];
    if !read_idle_byte(&mut stream, &mut prefix[0])? {
        return Ok(None);
    }
    let mut progressing = DeadlineStream::new(stream, progress_timeout)?;
    progressing.read_exact(&mut prefix[1..])?;
    let total = usize::try_from(u32::from_be_bytes(prefix)).map_err(|_| Error::InvalidLength)?;
    if total < HEADER_BYTES { return Err(Error::InvalidLength); }
    if total > limit { return Err(Error::TooLarge); }
    let mut body = vec![0_u8; total];
    progressing.read_exact(&mut body)?;
    Ok(Some(Frame { lane: body[0], kind: body[1], payload: body.split_off(HEADER_BYTES) }))
}

pub fn write_frame(output: &mut impl Write, frame: &Frame, limit: usize) -> Result<(), Error> {
    validate_limit(limit)?;
    let total = frame.payload.len().checked_add(HEADER_BYTES).ok_or(Error::TooLarge)?;
    if total > limit { return Err(Error::TooLarge); }
    let total = u32::try_from(total).map_err(|_| Error::TooLarge)?;
    output.write_all(&total.to_be_bytes())?;
    output.write_all(&[frame.lane, frame.kind])?;
    output.write_all(&frame.payload)?;
    output.flush()?;
    Ok(())
}

#[derive(Debug)]
pub struct IncompleteSubmission {
    pub written: usize,
    pub error: Error,
}

pub fn write_frame_witness(
    output: &mut impl Write,
    frame: &Frame,
    limit: usize,
) -> Result<(), IncompleteSubmission> {
    let mut bytes = Vec::new();
    write_frame(&mut bytes, frame, limit)
        .map_err(|error| IncompleteSubmission { written: 0, error })?;
    let mut written = 0;
    while written < bytes.len() {
        match output.write(&bytes[written..]) {
            Ok(0) => return Err(IncompleteSubmission { written, error: Error::Io(std::io::Error::new(std::io::ErrorKind::WriteZero, "request frame write returned zero")) }),
            Ok(count) => written += count,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(IncompleteSubmission { written, error: Error::Io(error) }),
        }
    }
    output.flush().map_err(|error| IncompleteSubmission { written, error: Error::Io(error) })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PeerCredentials {
    pub user_id: UserId,
    pub group_id: GroupId,
    pub process_id: ProcessId,
}

pub fn peer_credentials(stream: &std::os::unix::net::UnixStream) -> Result<PeerCredentials, Error> {
    let credentials = nix::sys::socket::getsockopt(stream, nix::sys::socket::sockopt::PeerCredentials)
        .map_err(|error| Error::PeerCredential(error.to_string()))?;
    Ok(PeerCredentials {
        user_id: UserId(i64::from(credentials.uid())),
        group_id: GroupId(i64::from(credentials.gid())),
        process_id: ProcessId(i64::from(credentials.pid())),
    })
}

pub struct LivePeer {
    pub identity: PeerIdentity,
    pidfd: OwnedFd,
}

impl LivePeer {
    pub fn capture(identity: PeerIdentity) -> std::io::Result<Self> {
        let pid = i32::try_from(identity.process_id.0).ok()
            .and_then(rustix::process::Pid::from_raw)
            .ok_or_else(|| std::io::Error::other("peer process id is invalid"))?;
        let pidfd = rustix::process::pidfd_open(pid, rustix::process::PidfdFlags::empty())?;
        Ok(Self { identity, pidfd })
    }
    pub fn is_live(&self) -> bool {
        let mut poll = [nix::poll::PollFd::new(self.pidfd.as_fd(), nix::poll::PollFlags::POLLIN)];
        matches!(nix::poll::poll(&mut poll, 0_u8), Ok(0))
    }
    pub fn matches(&self, credentials: &PeerCredentials) -> bool {
        self.identity.user_id == credentials.user_id
            && self.identity.process_id == credentials.process_id
            && self.is_live()
    }
    pub fn matches_caller(&self, caller: &Caller) -> bool {
        self.identity.user_id == caller.user_id
            && self.identity.process_id == caller.process_id
            && self.is_live()
    }
}

#[cfg(test)]
mod tests {
    use super::read_idle_byte;
    use std::io::Read;

    struct InterruptedThenByte(bool);
    impl Read for InterruptedThenByte {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            if !self.0 {
                self.0 = true;
                return Err(std::io::ErrorKind::Interrupted.into());
            }
            buffer[0] = 7;
            Ok(1)
        }
    }

    #[test]
    fn idle_byte_read_retries_interruption_and_keeps_eof_and_errors() {
        let mut byte = 0;
        assert!(read_idle_byte(&mut InterruptedThenByte(false), &mut byte).unwrap());
        assert_eq!(byte, 7);

        assert!(!read_idle_byte(&mut &b""[..], &mut byte).unwrap());

        struct Fails;
        impl Read for Fails {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                Err(std::io::ErrorKind::BrokenPipe.into())
            }
        }
        assert_eq!(
            read_idle_byte(&mut Fails, &mut byte).unwrap_err().kind(),
            std::io::ErrorKind::BrokenPipe
        );
    }
}

/// The admission both processes of a Nexus are bounded by: the deadline
/// every frame is read and written within, the largest frame admitted, and
/// how many connections a listener holds open at once.
#[derive(Clone, Copy, Debug)]
pub struct Admission {
    deadline: Duration,
    frame_capacity: usize,
    connection_capacity: usize,
}

pub trait AdmissionAccess {
    fn deadline(&self) -> Duration;
    fn frame_capacity(&self) -> usize;
    fn connection_capacity(&self) -> usize;
}

impl AdmissionAccess for Admission {
    fn deadline(&self) -> Duration { self.deadline }
    fn frame_capacity(&self) -> usize { self.frame_capacity }
    fn connection_capacity(&self) -> usize { self.connection_capacity }
}

impl Admission {
    pub fn new(deadline: Duration, frame_capacity: usize, connection_capacity: usize) -> Result<Self, &'static str> {
        if deadline.is_zero() || Instant::now().checked_add(deadline).is_none() {
            return Err("frame deadline must be positive and representable");
        }
        validate_limit(frame_capacity).map_err(|_| "frame capacity is outside the shared wire bound")?;
        if connection_capacity == 0 {
            return Err("connection capacity must be positive");
        }
        Ok(Self { deadline, frame_capacity, connection_capacity })
    }
}

/// A listening socket whose path appears only once it listens, so a peer
/// that waits for the path connects once and is accepted.
pub trait AppearingBinding {
    /// Binds and listens under a name of this process's own beside the path,
    /// then renames the socket onto the path, which must not be there. The
    /// path's appearance is the event a peer waits on.
    fn bind_appearing(&self) -> std::io::Result<UnixListener>;
}

impl AppearingBinding for Path {
    fn bind_appearing(&self) -> std::io::Result<UnixListener> {
        static STAGED: AtomicU64 = AtomicU64::new(0);
        let directory = self.parent().filter(|parent| !parent.as_os_str().is_empty()).unwrap_or(Path::new("."));
        let drawn = STAGED.fetch_add(1, Ordering::Relaxed);
        let staged = directory.join(format!(".{}-{drawn}", std::process::id()));
        match std::fs::remove_file(&staged) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        let listener = UnixListener::bind(&staged)?;
        let renamed = rustix::fs::renameat_with(rustix::fs::CWD, &staged, rustix::fs::CWD, self, rustix::fs::RenameFlags::NOREPLACE);
        if let Err(errno) = renamed {
            let _ = std::fs::remove_file(&staged);
            return Err(errno.into());
        }
        Ok(listener)
    }
}

/// A socket awaited at its path: the kernel's word that something was made
/// wakes the wait, which then connects once. Nothing here keeps a clock: the
/// deadline only bounds the wait.
pub trait SocketAwaiting {
    /// Connects once to the socket at this path as soon as it is there. The
    /// watch rests on the path's directory, or, while that is not there, on
    /// its nearest existing ancestor, and moves down at each creation. A
    /// directory there that cannot be watched is refused `Unwatchable`; the
    /// deadline passing first is `Elapsed`; a socket there that refuses the
    /// connection is `Unconnected`.
    fn connect_when_present(&self, deadline: Instant) -> Result<UnixStream, AwaitRefusal>;
}

impl SocketAwaiting for Path {
    fn connect_when_present(&self, deadline: Instant) -> Result<UnixStream, AwaitRefusal> {
        let directory = self.parent().filter(|parent| !parent.as_os_str().is_empty()).unwrap_or(Path::new("."));
        let unwatchable = |place: &Path, error: &dyn std::fmt::Display| {
            AwaitRefusal::Unwatchable(SocketFault(format!("{}: {error}", place.display())))
        };
        let (sender, words) = std::sync::mpsc::channel();
        let mut watcher = notify::recommended_watcher(move |word: notify::Result<notify::Event>| {
            let _ = sender.send(word);
        })
        .map_err(|error| unwatchable(directory, &error))?;
        let mut resting: Option<PathBuf> = None;
        loop {
            let nearest = directory
                .ancestors()
                .find(|ancestor| ancestor.is_dir())
                .ok_or_else(|| unwatchable(directory, &"no ancestor of it is there"))?
                .to_path_buf();
            if resting.as_ref() != Some(&nearest) {
                match watcher.watch(&nearest, RecursiveMode::NonRecursive) {
                    Ok(()) => {
                        if let Some(previous) = resting.replace(nearest) {
                            let _ = watcher.unwatch(&previous);
                        }
                    }
                    // Gone between the look and the watch: look again.
                    Err(error) if matches!(error.kind, notify::ErrorKind::PathNotFound) => continue,
                    Err(error) => return Err(unwatchable(&nearest, &error)),
                }
            }
            if std::fs::symlink_metadata(self).is_ok() {
                return UnixStream::connect(self)
                    .map_err(|error| AwaitRefusal::Unconnected(SocketFault(format!("{}: {error}", self.display()))));
            }
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .filter(|remaining| !remaining.is_zero())
                .ok_or(AwaitRefusal::Elapsed)?;
            match words.recv_timeout(remaining) {
                Ok(Ok(_)) => {}
                Ok(Err(error)) => return Err(unwatchable(&nearest_or(&resting, directory), &error)),
                Err(RecvTimeoutError::Timeout) => return Err(AwaitRefusal::Elapsed),
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(unwatchable(&nearest_or(&resting, directory), &"the watch ended"))
                }
            }
        }
    }
}

fn nearest_or(resting: &Option<PathBuf>, directory: &Path) -> PathBuf {
    resting.clone().unwrap_or_else(|| directory.to_path_buf())
}
