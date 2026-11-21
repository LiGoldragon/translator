//! The place a Nexus and its translator start in, and the sockets and files
//! named under it.

use std::io;
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

/// What a Nexus adds to the library: its name, which names its sockets, its
/// files and its clients' configuration.
pub trait Named {
    const NAME: &'static str;
}

/// Each socket and file of a Nexus, by what it is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Placed {
    NexusSocket,
    MetaSocket,
    TextSocket,
    MetaTextSocket,
    HookSocket,
    EmissionSocket,
    /// A contract the Nexus serves beside its own: `<name>-<contract>.sock`.
    ContractSocket(&'static str),
    /// The text socket of such a contract: `<name>-<contract>-translator.sock`.
    ContractTextSocket(&'static str),
    /// A file the translator writes beside the sockets: `<name>-<file>`.
    Beside(&'static str),
    /// The memory, named for the version of the types it holds.
    Memory(&'static str),
    /// The trace, named for the version of the Nexus that wrote it.
    Trace(&'static str),
}

impl Placed {
    fn file_name(self, name: &str) -> String {
        match self {
            Placed::NexusSocket => format!("{name}.sock"),
            Placed::MetaSocket => format!("{name}-meta.sock"),
            Placed::TextSocket => format!("{name}-translator.sock"),
            Placed::MetaTextSocket => format!("{name}-meta-translator.sock"),
            Placed::HookSocket => format!("{name}-hook.sock"),
            Placed::EmissionSocket => format!("{name}-emission.sock"),
            Placed::ContractSocket(contract) => format!("{name}-{contract}.sock"),
            Placed::ContractTextSocket(contract) => format!("{name}-{contract}-translator.sock"),
            Placed::Beside(file) => format!("{name}-{file}"),
            Placed::Memory(version) => format!("{name}-memory-{version}.signal"),
            Placed::Trace(version) => format!("{name}-trace-{version}.signal"),
        }
    }

    fn in_state(self) -> bool {
        matches!(self, Placed::Memory(_) | Placed::Trace(_))
    }
}

/// Why nothing was sent, or why a program could not begin: what the person is told.
#[derive(Debug, PartialEq, Eq)]
pub enum Unsent {
    Usage,
    Missing(String),
    Unreadable { path: String, reason: String },
    Invalid { path: String, reason: String },
}

fn budget() -> Budget {
    Budget { remaining: 4_096, reader: ReaderBudget::try_from(4_096).expect("a reader budget"), depth: 0, maximum_depth: 64 }
}

/// Reads a datom typed at a client into its type.
pub fn actualize<T: Composing>(text: &str) -> Result<T, datom_codec::Error> {
    Potential::<T>::from(text).actualize(&mut budget())
}

/// An absolute path named by a standard variable; a relative or empty one is no path.
fn absolute(variable: &str) -> Option<String> {
    std::env::var(variable).ok().filter(|path| PathBuf::from(path).is_absolute())
}

fn home() -> Result<String, Unsent> {
    absolute("HOME").ok_or_else(|| Unsent::Missing("HOME".into()))
}

impl Place {
    /// The one datom a program of the Nexus is started with.
    pub fn from_arguments() -> Result<Place, Unsent> {
        let mut arguments = std::env::args().skip(1);
        match (arguments.next(), arguments.next()) {
            (Some(datom), None) => actualize(&datom).map_err(|error| Unsent::Invalid { path: "the place".into(), reason: format!("{error:?}") }),
            _ => Err(Unsent::Usage),
        }
    }

    /// The place a client reaches: its configuration names it, and the standard
    /// variables, nothing else, say where its sockets and memory are.
    pub fn from_configuration<N: Named>() -> Result<Place, Unsent> {
        let configuration_root = match absolute("XDG_CONFIG_HOME") {
            Some(root) => root,
            None => format!("{}/.config", home()?),
        };
        let path = format!("{configuration_root}/nexus/{}-nexus.datom", N::NAME);
        let text = std::fs::read_to_string(&path).map_err(|error| match error.kind() {
            io::ErrorKind::NotFound => Unsent::Missing(path.clone()),
            _ => Unsent::Unreadable { path: path.clone(), reason: error.to_string() },
        })?;
        let configuration: CliConfiguration =
            actualize(&text).map_err(|error| Unsent::Invalid { path: path.clone(), reason: format!("{error:?}") })?;
        let runtime_root = absolute("XDG_RUNTIME_DIR").ok_or_else(|| Unsent::Missing("XDG_RUNTIME_DIR".into()))?;
        let state_root = match absolute("XDG_STATE_HOME") {
            Some(root) => root,
            None => format!("{}/.local/state", home()?),
        };
        let name = configuration.place_name.0.clone();
        Ok(Place {
            runtime: Runtime(format!("{runtime_root}/nexus/{name}")),
            state: State(format!("{state_root}/nexus/{name}")),
            place_name: configuration.place_name,
        })
    }

    pub fn path<N: Named>(&self, placed: Placed) -> PathBuf {
        let root = if placed.in_state() { &self.state.0 } else { &self.runtime.0 };
        PathBuf::from(root).join(placed.file_name(N::NAME))
    }

    pub fn prepare(&self) -> io::Result<()> {
        std::fs::create_dir_all(&self.runtime.0)?;
        std::fs::create_dir_all(&self.state.0)
    }
}
