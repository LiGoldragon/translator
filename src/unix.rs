//! Policy-free Unix transport mechanics shared by Nexus translators.

use crate::{Caller, GroupId, PeerIdentity, ProcessId, UserId};
use std::io::{Read, Write};
use std::os::fd::{AsFd, OwnedFd};
use std::time::{Duration, Instant};

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

pub fn read_frame_after_idle(
    mut stream: std::os::unix::net::UnixStream,
    progress_timeout: Duration,
    limit: usize,
) -> Result<Option<Frame>, Error> {
    validate_limit(limit)?;
    stream.set_read_timeout(None)?;
    let mut prefix = [0_u8; 4];
    match stream.read(&mut prefix[..1]) {
        Ok(0) => return Ok(None),
        Ok(1) => {}
        Ok(_) => return Err(Error::InvalidLength),
        Err(error) => return Err(Error::Io(error)),
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

#[derive(Clone, Copy, Debug)]
pub struct Admission {
    pub deadline: Duration,
    pub frame_capacity: usize,
    pub retry_delay: Duration,
}

impl Admission {
    pub fn new(deadline: Duration, frame_capacity: usize, retry_delay: Duration) -> Result<Self, &'static str> {
        if deadline.is_zero() || Instant::now().checked_add(deadline).is_none() {
            return Err("frame deadline must be positive and representable");
        }
        validate_limit(frame_capacity).map_err(|_| "frame capacity is outside the shared wire bound")?;
        if retry_delay.is_zero() || Instant::now().checked_add(retry_delay).is_none() {
            return Err("retry delay must be positive and representable");
        }
        Ok(Self { deadline, frame_capacity, retry_delay })
    }
}

fn recoverable_accept(error: &std::io::Error) -> bool {
    matches!(error.kind(), std::io::ErrorKind::Interrupted | std::io::ErrorKind::WouldBlock | std::io::ErrorKind::ConnectionAborted)
        || matches!(error.raw_os_error(), Some(libc::EMFILE) | Some(libc::ENFILE) | Some(libc::ENOBUFS) | Some(libc::ENOMEM))
}

pub fn accept_with_retry(
    listener: &std::os::unix::net::UnixListener,
    retry_delay: Duration,
) -> std::io::Result<(std::os::unix::net::UnixStream, std::os::unix::net::SocketAddr)> {
    accept_with_retry_using(|| listener.accept(), |delay| std::thread::sleep(delay), retry_delay)
}

pub fn accept_with_retry_using<T>(
    mut accept: impl FnMut() -> std::io::Result<T>,
    mut wait: impl FnMut(Duration),
    retry_delay: Duration,
) -> std::io::Result<T> {
    loop {
        match accept() {
            Ok(value) => return Ok(value),
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) if recoverable_accept(&error) => { wait(retry_delay); }
            Err(error) => return Err(error),
        }
    }
}
