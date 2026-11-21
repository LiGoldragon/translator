//! The Relay Nexus's translator: the words of its own calls, and nothing else.
//! The sockets, the caller, the serving of each connection, the CLI, the hook
//! client, a lasting emission and the running of a program are the translator
//! library's.
//!
//! A hook's payload is a line of text, an event and a name: `Started alpha`,
//! `Ended alpha`.

use std::io;
use std::os::unix::net::UnixStream;

use relay_signal::emission::{Command, Emittable, Finished, Follow, Line, Lines, Offset, Words};
use relay_signal::signal::{Hook, Name};
use relay_signal::{Emission, Outcome, Relay};
use translator::{Emitting, Framed, Growth, Lasted, Lasting, Named, Translatable, Untranslated, Waited, capture};

/// The Relay Nexus's translator: it is the marker of every body below, and its name is the Nexus's.
pub struct RelayTranslator;

impl Named for RelayTranslator {
    const NAME: &'static str = Relay::NAME;
}

impl Translatable<RelayTranslator> for Hook {
    fn translate(payload: &str) -> Result<Hook, Untranslated> {
        let mut words = payload.split_whitespace();
        let (Some(event), Some(name), None) = (words.next(), words.next(), words.next()) else {
            return Err(Untranslated::Unreadable);
        };
        match event {
            "Started" => Ok(Hook::Started(Name(name.to_owned()))),
            "Ended" => Ok(Hook::Ended(Name(name.to_owned()))),
            other => Err(Untranslated::UnknownEvent(other.to_owned())),
        }
    }
}

impl Emittable<RelayTranslator> for Command {
    /// The words of the call: the program, then each argument. A program that
    /// ended with 0 is done, and what it wrote is its words; any other status is a refusal.
    fn emit(&self, _translator: &RelayTranslator) -> Finished {
        let argv: Vec<String> = std::iter::once(self.program.0.clone()).chain(self.argument_vector.iter().map(|argument| argument.0.clone())).collect();
        let captured = capture(&argv);
        match captured.status {
            0 => Finished::Done(Words(captured.output)),
            status => Finished::Refused(status.into()),
        }
    }
}

impl Lasting<RelayTranslator> for Follow {
    /// Gives the lines the file gains, as they come, from the offset, at most the
    /// count at a time; a file not yet written is waited for.
    fn last(&self, mut connection: Lasted) -> io::Result<()> {
        let mut growth = Growth::begin(&self.file_path.0)?;
        let mut offset = self.offset.0.max(0) as u64;
        let count = self.count.0.max(1) as usize;
        loop {
            if growth.present()? {
                let gained = growth.lines_after(offset, count)?;
                offset = gained.offset;
                let full = gained.lines.len() >= count;
                if !gained.lines.is_empty() {
                    let lines = Lines { line_vector: gained.lines.into_iter().map(Line).collect(), offset: Offset(offset.try_into().unwrap_or(i64::MAX)) };
                    connection.give(&Outcome::Lines(lines))?;
                }
                if full {
                    continue;
                }
            }
            if growth.wait(&connection)? == Waited::Closed {
                return Ok(());
            }
        }
    }
}

impl Emitting<RelayTranslator> for Emission {
    fn emitted(self, mut connection: UnixStream) -> io::Result<()> {
        match self {
            Emission::Command(command) => Outcome::Finished(command.emit(&RelayTranslator)).write_to(&mut connection),
            Emission::Follow(follow) => follow.last(Lasted::new(connection)),
        }
    }
}
