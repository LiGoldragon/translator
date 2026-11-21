//! The clients of a Nexus's translator, written once: the CLI, and the hook
//! client the harness runs. What a Nexus adds is its name.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::process::ExitCode;

use transport::{Named, Place, Placed, Unsent};

/// The text socket a CLI reaches: the ordinary one, or the meta one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextSocket {
    Ordinary,
    Meta,
}

/// How a CLI ended: a reply was printed, the translator could not answer, or
/// nothing was sent.
#[derive(Debug, PartialEq, Eq)]
pub enum Ending {
    Answered,
    Failed,
    Unsent(Unsent),
}

impl From<Ending> for ExitCode {
    /// Answered is 0, Failed is 1, Unsent is 2.
    fn from(ending: Ending) -> ExitCode {
        match ending {
            Ending::Answered => ExitCode::SUCCESS,
            Ending::Failed => ExitCode::FAILURE,
            Ending::Unsent(_) => ExitCode::from(2),
        }
    }
}

/// The CLI: one datom written to the translator, and each reply printed as it
/// arrives, one datom a line, until the translator ends the exchange. Called
/// with no datom, it prints the Ethos of the Nexus's Input and Output, which the
/// translator answers it with.
pub trait Calling: Named + Sized {
    /// The CLI of a Nexus, from its arguments and its configuration.
    fn call(text_socket: TextSocket) -> Ending {
        let mut arguments = std::env::args().skip(1);
        let datom = match (arguments.next(), arguments.next()) {
            (datom, None) => datom.unwrap_or_default(),
            _ => return Self::unsent(Unsent::Usage),
        };
        match Place::from_configuration::<Self>() {
            Ok(place) => Self::exchange(&place, text_socket, &datom, &mut std::io::stdout()),
            Err(unsent) => Self::unsent(unsent),
        }
    }

    fn unsent(unsent: Unsent) -> Ending {
        eprintln!("{unsent:?}");
        Ending::Unsent(unsent)
    }

    /// One exchange: the datom is written, each line of the reply is written to
    /// `out` as it arrives. A reply of no line, or of the one word Unreadable or
    /// NexusDown, which is how the translator says it served nothing, is Failed.
    fn exchange<W: Write>(place: &Place, text_socket: TextSocket, datom: &str, out: &mut W) -> Ending {
        let placed = match text_socket {
            TextSocket::Ordinary => Placed::TextSocket,
            TextSocket::Meta => Placed::MetaTextSocket,
        };
        let Ok(mut stream) = UnixStream::connect(place.path::<Self>(placed)) else {
            eprintln!("TranslatorDown");
            return Ending::Failed;
        };
        if stream.write_all(datom.as_bytes()).is_err() || stream.shutdown(Shutdown::Write).is_err() {
            return Ending::Failed;
        }
        let mut count = 0;
        let mut last = String::new();
        for line in BufReader::new(stream).lines().map_while(Result::ok) {
            if writeln!(out, "{line}").and_then(|()| out.flush()).is_err() {
                break;
            }
            count += 1;
            last = line;
        }
        if count == 0 || (count == 1 && matches!(last.as_str(), "Unreadable" | "NexusDown")) {
            return Ending::Failed;
        }
        Ending::Answered
    }
}

impl<N: Named> Calling for N {}

/// The hook client: the harness runs it at each event with the payload on its
/// input. It writes the payload, as it came, to the translator's hook socket,
/// waits until the translator has reported it, and ends with 0 whatever happened.
pub trait Hooking: Named + Sized {
    fn hook() -> ExitCode {
        let mut payload = Vec::new();
        if std::io::stdin().read_to_end(&mut payload).is_ok()
            && let Ok(place) = Place::from_configuration::<Self>()
        {
            Self::report(&place, &payload);
        }
        ExitCode::SUCCESS
    }

    fn report(place: &Place, payload: &[u8]) {
        if let Ok(mut stream) = UnixStream::connect(place.path::<Self>(Placed::HookSocket))
            && stream.write_all(payload).is_ok()
            && stream.shutdown(Shutdown::Write).is_ok()
        {
            let _ = stream.read_to_end(&mut Vec::new());
        }
    }
}

impl<N: Named> Hooking for N {}
