//! What the Nexus and its translator share: the place they start in, the
//! sockets and files named under it, and the framing of Signal on a stream.

use std::io::{self, Read, Write};
use std::path::PathBuf;

use datom_codec::{Actualizing, Budget, Composing, Datomizable, Potential};
use protos::ReaderBudget;

#[derive(Clone, Debug, PartialEq, Eq, Datomizable, Composing)]
pub struct PlaceName(pub String);
#[derive(Clone, Debug, PartialEq, Eq, Datomizable, Composing)]
pub struct Runtime(pub String);
#[derive(Clone, Debug, PartialEq, Eq, Datomizable, Composing)]
pub struct State(pub String);

/// What every Nexus starts with: its sockets in Runtime, its memory in State.
#[derive(Clone, Debug, PartialEq, Eq, Datomizable, Composing)]
pub struct Place {
    pub place_name: PlaceName,
    pub runtime: Runtime,
    pub state: State,
}

/// What a client knows besides its datom: the place it reaches.
#[derive(Clone, Debug, PartialEq, Eq, Datomizable, Composing)]
pub struct CliConfiguration {
    pub place_name: PlaceName,
}

/// Each socket and file of the Claude Harness Nexus, by what it is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Placed {
    NexusSocket,
    HookSocket,
    EmissionSocket,
    Memory,
    Trace,
}

impl Placed {
    fn file_name(self) -> String {
        let version = env!("CARGO_PKG_VERSION");
        match self {
            Placed::NexusSocket => "claude-harness.sock".into(),
            Placed::HookSocket => "claude-harness-hook.sock".into(),
            Placed::EmissionSocket => "claude-harness-emission.sock".into(),
            Placed::Memory => format!("claude-harness-memory-{version}.signal"),
            Placed::Trace => format!("claude-harness-trace-{version}.signal"),
        }
    }
}

#[derive(Debug)]
pub enum Unplaced {
    Usage,
    Unreadable(String),
    Invalid(String),
}

fn budget() -> Budget {
    Budget { remaining: 4_096, reader: ReaderBudget::try_from(4_096).expect("a reader budget"), depth: 0, maximum_depth: 64 }
}

fn actualize<T: Composing>(text: &str) -> Result<T, Unplaced> {
    Potential::<T>::from(text).actualize(&mut budget()).map_err(|error| Unplaced::Invalid(format!("{error:?}")))
}

impl Place {
    /// The one datom a program of the Nexus is started with.
    pub fn from_arguments() -> Result<Place, Unplaced> {
        let mut arguments = std::env::args().skip(1);
        match (arguments.next(), arguments.next()) {
            (Some(datom), None) => actualize(&datom),
            _ => Err(Unplaced::Usage),
        }
    }

    /// The place a client reaches, from its configuration and the XDG runtime root.
    pub fn from_configuration() -> Result<Place, Unplaced> {
        let home = std::env::var("HOME").map_err(|_| Unplaced::Usage)?;
        let configuration_root = std::env::var("XDG_CONFIG_HOME").unwrap_or_else(|_| format!("{home}/.config"));
        let path = format!("{configuration_root}/nexus/claude-harness-nexus.datom");
        let text = std::fs::read_to_string(&path).map_err(|error| Unplaced::Unreadable(format!("{path}: {error}")))?;
        let configuration: CliConfiguration = actualize(&text)?;
        let runtime_root = std::env::var("XDG_RUNTIME_DIR").map_err(|_| Unplaced::Usage)?;
        let state_root = std::env::var("XDG_STATE_HOME").unwrap_or_else(|_| format!("{home}/.local/state"));
        let name = configuration.place_name.0.clone();
        Ok(Place {
            runtime: Runtime(format!("{runtime_root}/nexus/{name}")),
            state: State(format!("{state_root}/nexus/{name}")),
            place_name: configuration.place_name,
        })
    }

    pub fn path(&self, placed: Placed) -> PathBuf {
        let root = match placed {
            Placed::NexusSocket | Placed::HookSocket | Placed::EmissionSocket => &self.runtime.0,
            Placed::Memory | Placed::Trace => &self.state.0,
        };
        PathBuf::from(root).join(placed.file_name())
    }

    pub fn prepare(&self) -> io::Result<()> {
        std::fs::create_dir_all(&self.runtime.0)?;
        std::fs::create_dir_all(&self.state.0)
    }
}

type HighSerializer<'a> = rkyv::api::high::HighSerializer<rkyv::util::AlignedVec, rkyv::ser::allocator::ArenaHandle<'a>, rkyv::rancor::Error>;
type HighValidator<'a> = rkyv::api::high::HighValidator<'a, rkyv::rancor::Error>;
type HighDeserializer = rkyv::api::high::HighDeserializer<rkyv::rancor::Error>;

fn unreadable(error: rkyv::rancor::Error) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error)
}

/// One Signal value on a stream: its length, then its archive.
pub trait Framed: Sized {
    fn write_to<W: Write>(&self, stream: &mut W) -> io::Result<()>;
    /// None is a stream that ended before a frame began.
    fn read_from<R: Read>(stream: &mut R) -> io::Result<Option<Self>>;
}

impl<T> Framed for T
where
    T: rkyv::Archive + for<'a> rkyv::Serialize<HighSerializer<'a>>,
    T::Archived: for<'a> rkyv::bytecheck::CheckBytes<HighValidator<'a>> + rkyv::Deserialize<T, HighDeserializer>,
{
    fn write_to<W: Write>(&self, stream: &mut W) -> io::Result<()> {
        let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(self).map_err(unreadable)?;
        let length = u32::try_from(bytes.len()).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "a frame over four gigabytes"))?;
        stream.write_all(&length.to_le_bytes())?;
        stream.write_all(&bytes)?;
        stream.flush()
    }

    fn read_from<R: Read>(stream: &mut R) -> io::Result<Option<Self>> {
        let mut length = [0u8; 4];
        match stream.read_exact(&mut length) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
            Err(error) => return Err(error),
        }
        let mut bytes = vec![0u8; u32::from_le_bytes(length) as usize];
        stream.read_exact(&mut bytes)?;
        rkyv::from_bytes::<T, rkyv::rancor::Error>(&bytes).map(Some).map_err(unreadable)
    }
}
