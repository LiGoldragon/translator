//! A translator's sockets and the serving of each connection, written once.
//! The text sockets take a datom and write back each output as a datom; the
//! hook socket takes the harness's payloads, one at a time and in order; the
//! emission socket takes each emission on its own connection.

use std::io::{self, Read, Write};
use std::marker::PhantomData;
use std::net::Shutdown;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::thread::{self, JoinHandle};

use datom_codec::{Composing, Datomizable};
use protos::{Compactable, Protosizable};
use rustix::event::{PollFd, PollFlags};
use transport::{Caller, Framed, Named, Place, Placed, Unsent, actualize};

use crate::Translatable;

/// How a text connection was served.
#[derive(Debug)]
pub enum Served {
    /// The Nexus's outputs were written back.
    Answered,
    /// The text was not an Input.
    Unreadable(datom_codec::Error),
    /// No Nexus listens, or its connection ended before it answered.
    NexusDown,
}

/// What a Nexus's translator does with an emission that arrives on its own
/// connection: for each emission the body writes the outcome on it, or, where
/// the emission lasts, outcomes until the connection closes. `T` is the
/// translator's marker type, which lets the impl live in its crate.
pub trait Emitting<T>: Framed + Send + 'static {
    fn emitted(self, connection: UnixStream) -> io::Result<()>;
}

/// A Nexus's translator: its Place and nothing else. Every socket, the two of its
/// Nexus and its own, is found in the Runtime of that Place by the Nexus's name.
/// `N` is the translator's marker type and carries the name.
pub struct Translator<N: Named> {
    place: Place,
    lanes: Vec<JoinHandle<()>>,
    marker: PhantomData<fn() -> N>,
}

fn listen(path: &Path) -> io::Result<UnixListener> {
    let _ = std::fs::remove_file(path);
    UnixListener::bind(path)
}

/// One datom in, each output out as a datom line, until the Nexus or the caller
/// closes. A text with nothing in it asks for the Ethos of the Nexus's Input and Output.
fn serve_text<I, C, O>(mut text: UnixStream, nexus: &Path, ethos: &str, called: fn(Caller, I) -> C) -> io::Result<Served>
where
    I: Composing,
    C: Framed,
    O: Framed + Datomizable,
{
    let caller = Caller::of(&text)?;
    let mut datom = String::new();
    text.read_to_string(&mut datom)?;
    if datom.trim().is_empty() {
        text.write_all(ethos.as_bytes())?;
        return Ok(Served::Answered);
    }
    let input: I = match actualize(&datom) {
        Ok(input) => input,
        Err(error) => {
            let _ = writeln!(text, "Unreadable");
            return Ok(Served::Unreadable(error));
        }
    };
    let Ok(mut connection) = UnixStream::connect(nexus) else {
        let _ = writeln!(text, "NexusDown");
        return Ok(Served::NexusDown);
    };
    called(caller, input).write_to(&mut connection)?;
    if let (Ok(hanging), Ok(closing)) = (text.try_clone(), connection.try_clone()) {
        thread::spawn(move || {
            let mut waiting = [PollFd::new(&hanging, PollFlags::empty())];
            let _ = rustix::event::poll(&mut waiting, None);
            let _ = closing.shutdown(Shutdown::Both);
        });
    }
    let mut written = false;
    while let Ok(Some(output)) = O::read_from(&mut connection) {
        writeln!(text, "{}", output.datomize(vec![]).protosize().compact())?;
        written = true;
    }
    let _ = text.shutdown(Shutdown::Both);
    Ok(if written { Served::Answered } else { Served::NexusDown })
}

/// The harness's payload, read into a type of the Nexus and sent in; the Nexus's
/// answer is waited for, so that the hook client ends when the Nexus has taken it.
fn serve_hook<H, T, C, O>(mut hook: UnixStream, nexus: &Path, called: fn(Caller, H) -> C) -> io::Result<()>
where
    H: Translatable<T>,
    C: Framed,
    O: Framed,
{
    let caller = Caller::of(&hook)?;
    let mut payload = String::new();
    hook.read_to_string(&mut payload)?;
    let hook = H::translate(&payload).map_err(|untranslated| io::Error::new(io::ErrorKind::InvalidData, format!("{untranslated:?}")))?;
    let mut connection = UnixStream::connect(nexus)?;
    called(caller, hook).write_to(&mut connection)?;
    O::read_from(&mut connection)?;
    Ok(())
}

impl<N: Named + 'static> Translator<N> {
    pub fn new(place: Place) -> Translator<N> {
        Translator { place, lanes: Vec::new(), marker: PhantomData }
    }

    /// Of the one datom the program was started with, its place.
    pub fn from_arguments() -> Result<Translator<N>, Unsent> {
        Ok(Translator::new(Place::from_arguments()?))
    }

    pub fn place(&self) -> &Place {
        &self.place
    }

    /// Makes the directories of the place.
    pub fn prepare(&self) -> io::Result<()> {
        self.place.prepare()
    }

    fn listener(&self, placed: Placed) -> io::Result<UnixListener> {
        listen(&self.place.path::<N>(placed))
    }

    /// A text lane: the socket `text` takes a datom of the Nexus's input `I`; it
    /// is sent in as the call `C`, and each output `O` is written back as a datom.
    /// `ethos` is what a text with nothing in it is answered with.
    pub fn text<I, C, O>(&mut self, text: Placed, nexus: Placed, ethos: &'static str, called: fn(Caller, I) -> C) -> io::Result<()>
    where
        I: Composing + 'static,
        C: Framed + 'static,
        O: Framed + Datomizable + 'static,
    {
        let listener = self.listener(text)?;
        let nexus = self.place.path::<N>(nexus);
        self.lanes.push(thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let nexus = nexus.clone();
                thread::spawn(move || match serve_text::<I, C, O>(stream, &nexus, ethos, called) {
                    Ok(Served::Answered) => {}
                    Ok(served) => eprintln!("{served:?}"),
                    Err(error) => eprintln!("{error}"),
                });
            }
        }));
        Ok(())
    }

    /// The hook socket: each payload of the harness is read into `H` by its body,
    /// sent to the Nexus's own socket as the call `C`, and its answer `O` is waited for.
    /// Payloads are taken one at a time and in order.
    pub fn hooks<H, C, O>(&mut self, called: fn(Caller, H) -> C) -> io::Result<()>
    where
        H: Translatable<N> + Send + 'static,
        C: Framed + 'static,
        O: Framed + 'static,
    {
        let listener = self.listener(Placed::HookSocket)?;
        let nexus = self.place.path::<N>(Placed::NexusSocket);
        self.lanes.push(thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                if let Err(error) = serve_hook::<H, N, C, O>(stream, &nexus, called) {
                    eprintln!("{error}");
                }
            }
        }));
        Ok(())
    }

    /// The emission socket: each emission arrives on a connection of its own and
    /// is taken by its body.
    pub fn emissions<E: Emitting<N>>(&mut self) -> io::Result<()> {
        let listener = self.listener(Placed::EmissionSocket)?;
        self.lanes.push(thread::spawn(move || {
            for mut stream in listener.incoming().flatten() {
                thread::spawn(move || {
                    if let Ok(Some(emission)) = E::read_from(&mut stream) {
                        let _ = emission.emitted(stream);
                    }
                });
            }
        }));
        Ok(())
    }

    /// Tells the Nexus that the translator listens, with the call `C` the Nexus
    /// takes it as; the answer is read. A Nexus that is down is told nothing: it
    /// bears what it holds at its own start.
    pub fn listening<C: Framed, O: Framed>(&self, call: &C) -> Option<O> {
        let mut nexus = UnixStream::connect(self.place.path::<N>(Placed::NexusSocket)).ok()?;
        call.write_to(&mut nexus).ok()?;
        O::read_from(&mut nexus).ok().flatten()
    }

    /// Serves until every socket ends, which is when the program does.
    pub fn serve(self) {
        for lane in self.lanes {
            let _ = lane.join();
        }
    }
}
