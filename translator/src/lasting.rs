//! A lasting emission, as the translator holds it: one connection that stays
//! open, an outcome written on it each time what it asked for happens, until
//! the Nexus closes it.

use std::fs::File;
use std::io::{self, BufRead, BufReader, Seek, SeekFrom};
use std::os::fd::{AsFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;

use rustix::event::{PollFd, PollFlags};
use rustix::fs::inotify;

use crate::{Caller, Framed};

/// What ended a wait: what was watched happened, or the Nexus closed the connection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Waited {
    Woken,
    Closed,
}

/// The connection of a lasting emission, as the translator holds it.
#[derive(Debug)]
pub struct Lasted {
    stream: UnixStream,
}

impl Lasted {
    pub fn new(stream: UnixStream) -> Lasted {
        Lasted { stream }
    }

    pub fn caller(&self) -> io::Result<Caller> {
        Caller::of(&self.stream)
    }

    /// One outcome, written as it happens.
    pub fn give<O: Framed>(&mut self, outcome: &O) -> io::Result<()> {
        outcome.write_to(&mut self.stream)
    }

    /// Waits until what is watched can be read, or the Nexus closes the
    /// connection; the Nexus writes nothing after its emission, so a connection
    /// that can be read has ended.
    pub fn wait<W: AsFd>(&self, watching: &W) -> io::Result<Waited> {
        let mut waiting = [PollFd::new(watching, PollFlags::IN), PollFd::new(&self.stream, PollFlags::IN)];
        rustix::event::poll(&mut waiting, None)?;
        Ok(if waiting[1].revents().is_empty() { Waited::Woken } else { Waited::Closed })
    }
}

/// The body of a lasting emission, written for a type of the Nexus's contract
/// by the Nexus's translator, which `T` marks.
pub trait Lasting<T> {
    /// Gives an outcome on the connection each time, until the Nexus closes it.
    fn last(&self, connection: Lasted) -> io::Result<()>;
}

/// The whole lines a file gained, and the offset after the last of them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Lines {
    pub lines: Vec<String>,
    pub offset: u64,
}

/// A file watched for growth by the kernel's inotify. A file not yet written is
/// waited for: its nearest existing ancestor is watched until each missing part
/// appears.
#[derive(Debug)]
pub struct Growth {
    path: PathBuf,
    watching: OwnedFd,
    appearing: Option<(PathBuf, i32)>,
    followed: bool,
}

impl Growth {
    pub fn begin(path: impl Into<PathBuf>) -> io::Result<Growth> {
        Ok(Growth { path: path.into(), watching: inotify::init(inotify::CreateFlags::CLOEXEC)?, appearing: None, followed: false })
    }

    /// True once the file is there and watched. False while it is waited for: an
    /// ancestor is then watched, and a wait ends when something appears in it.
    /// A path with no ancestor at all is NotFound.
    pub fn present(&mut self) -> io::Result<bool> {
        while !self.followed {
            if inotify::add_watch(&self.watching, &self.path, inotify::WatchFlags::MODIFY).is_ok() {
                self.followed = true;
                if let Some((_, watch)) = self.appearing.take() {
                    inotify::remove_watch(&self.watching, watch)?;
                }
                break;
            }
            let Some(nearest) = self.path.ancestors().skip(1).find(|ancestor| ancestor.is_dir()).map(std::path::Path::to_owned) else {
                return Err(io::Error::new(io::ErrorKind::NotFound, "no ancestor of the file exists"));
            };
            if self.appearing.as_ref().is_some_and(|(directory, _)| *directory == nearest) {
                break;
            }
            if let Some((_, watch)) = self.appearing.take() {
                inotify::remove_watch(&self.watching, watch)?;
            }
            let watch = inotify::add_watch(&self.watching, &nearest, inotify::WatchFlags::CREATE | inotify::WatchFlags::MOVED_TO)?;
            self.appearing = Some((nearest, watch));
        }
        Ok(self.followed)
    }

    /// The file's length: where a reader that wants only what comes begins.
    pub fn end(&self) -> io::Result<u64> {
        Ok(std::fs::metadata(&self.path)?.len())
    }

    /// The whole lines after the offset, at most the count of them; a last line
    /// not yet ended is left for the next reading.
    pub fn lines_after(&self, offset: u64, count: usize) -> io::Result<Lines> {
        let mut file = BufReader::new(File::open(&self.path)?);
        file.seek(SeekFrom::Start(offset))?;
        let mut gained = Lines { lines: Vec::new(), offset };
        let mut line = Vec::new();
        while gained.lines.len() < count {
            line.clear();
            let read = file.read_until(b'\n', &mut line)?;
            if read == 0 || line.last() != Some(&b'\n') {
                break;
            }
            gained.offset += read as u64;
            gained.lines.push(String::from_utf8_lossy(&line).trim_end_matches(['\n', '\r']).to_owned());
        }
        Ok(gained)
    }

    /// Waits until the kernel tells of a write, or of the file's appearing, or the
    /// Nexus closes the connection.
    pub fn wait(&mut self, connection: &Lasted) -> io::Result<Waited> {
        let waited = connection.wait(&self.watching)?;
        if waited == Waited::Woken {
            let mut events = [0u8; 4096];
            rustix::io::read(&self.watching, &mut events)?;
        }
        Ok(waited)
    }
}
