//! The Relay Nexus's own part: it takes each call, answers it, and remembers
//! what it was told. It holds no text and no path of a program: what must be
//! done outside it is an emission, sent to its translator, and what returns is
//! an outcome it reads in its own terms.

use std::io;
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::{Arc, Mutex};
use std::thread;

use relay_signal::emission::{Command, Finished, Follow};
use relay_signal::signal::{Answerable, Answering, Call, Hook, Notice, Output};
use relay_signal::{Emission, Outcome, Relay};
use translator_transport::{EmissionSocket, Framed, Place, Unobserved};

/// What the Nexus remembers: what the hook socket said, and each line a followed
/// file gained, in the order they came.
type Memory = Arc<Mutex<Vec<Notice>>>;

pub struct Nexus {
    memory: Memory,
    emissions: EmissionSocket,
}

fn remember(memory: &Memory, notice: Notice) {
    memory.lock().expect("the memory is not poisoned").push(notice);
}

impl Nexus {
    pub fn new(place: &Place) -> Nexus {
        Nexus { memory: Memory::default(), emissions: EmissionSocket::of::<Relay>(place) }
    }

    /// One call on one connection, one output back.
    fn serve(&self, mut connection: UnixStream) -> io::Result<()> {
        if let Some(Call { input, .. }) = Call::read_from(&mut connection)? {
            input.answer(self).write_to(&mut connection)?;
        }
        Ok(())
    }

    /// Takes each connection of the listener on a thread of its own, until the listener ends.
    pub fn listen(self, listener: UnixListener) {
        let nexus = Arc::new(self);
        for connection in listener.incoming().flatten() {
            let nexus = Arc::clone(&nexus);
            thread::spawn(move || {
                if let Err(error) = nexus.serve(connection) {
                    eprintln!("{error}");
                }
            });
        }
    }
}

impl Answering for Nexus {
    /// Attempts the program through the translator; its outcome is the answer.
    fn run(&self, command: &Command) -> Result<Finished, Output> {
        match self.emissions.send::<Emission, Outcome>(&Emission::Command(command.clone())) {
            Ok(Outcome::Finished(finished)) => Ok(finished),
            Ok(Outcome::Lines(_)) => Err(Output::Unobserved(Unobserved::Unanswered)),
            Err(unobserved) => Err(Output::Unobserved(unobserved)),
        }
    }

    /// A lasting emission: the connection stays open, and each line the file gains
    /// is remembered as it comes, until the translator closes it.
    fn follow(&self, follow: &Follow) -> Result<(), Output> {
        let mut held = self.emissions.hold::<Emission, Outcome>(&Emission::Follow(follow.clone())).map_err(Output::Unobserved)?;
        let memory = Arc::clone(&self.memory);
        thread::spawn(move || {
            while let Ok(Outcome::Lines(lines)) = held.outcome() {
                for line in lines.line_vector {
                    remember(&memory, Notice::Lined(line));
                }
            }
        });
        Ok(())
    }

    fn report(&self, hook: &Hook) -> Result<(), Output> {
        remember(&self.memory, Notice::Hooked(hook.clone()));
        Ok(())
    }

    fn seen(&self) -> Result<Vec<Notice>, Output> {
        Ok(self.memory.lock().expect("the memory is not poisoned").clone())
    }

    /// The translator tells that it listens.
    fn listening(&self) -> Result<(), Output> {
        Ok(())
    }
}
