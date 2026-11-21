//! The translator, started with its Nexus's place: the text socket takes a
//! datom typed at the CLI and writes back each output as a datom; the hook
//! socket takes Claude Code's payloads, one at a time and in order; the
//! emission socket takes each emission on its own connection.

use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::process::ExitCode;
use std::thread;

use claude_harness_process::*;
use claude_harness_translator::{Translatable, caller, follow};
use datom_codec::Datomizable;
use protos::{Protosizable, Textualizable};
use rustix::event::{PollFd, PollFlags};
use transport::{Framed, Place, Placed};

fn listen(place: &Place, placed: Placed) -> std::io::Result<UnixListener> {
    let path = place.path(placed);
    let _ = std::fs::remove_file(&path);
    UnixListener::bind(path)
}

fn report(mut hook_stream: UnixStream, place: &Place) {
    let Ok(caller) = caller(&hook_stream) else { return };
    let mut payload = String::new();
    if hook_stream.read_to_string(&mut payload).is_err() {
        return;
    }
    let hook = match Hook::translate(&payload) {
        Ok(hook) => hook,
        Err(untranslated) => {
            eprintln!("{untranslated:?}");
            return;
        }
    };
    let Ok(mut nexus) = UnixStream::connect(place.path(Placed::NexusSocket)) else {
        eprintln!("NexusDown");
        return;
    };
    if (Call { caller, input: Input::Report(hook) }).write_to(&mut nexus).is_ok() {
        let _ = Output::read_from(&mut nexus);
    }
}

/// One datom in, each output out as a datom line, until the Nexus or the caller closes.
fn serve_text(mut text: UnixStream, place: &Place) {
    let Ok(caller) = caller(&text) else { return };
    let mut datom = String::new();
    if text.read_to_string(&mut datom).is_err() {
        return;
    }
    let input: Input = match transport::actualize(&datom) {
        Ok(input) => input,
        Err(unplaced) => {
            let _ = writeln!(text, "Unreadable");
            eprintln!("{unplaced:?}");
            return;
        }
    };
    let Ok(mut nexus) = UnixStream::connect(place.path(Placed::NexusSocket)) else {
        let _ = writeln!(text, "NexusDown");
        return;
    };
    if (Call { caller, input }).write_to(&mut nexus).is_err() {
        return;
    }
    if let (Ok(hanging), Ok(closing)) = (text.try_clone(), nexus.try_clone()) {
        thread::spawn(move || {
            let mut waiting = [PollFd::new(&hanging, PollFlags::empty())];
            let _ = rustix::event::poll(&mut waiting, None);
            let _ = closing.shutdown(std::net::Shutdown::Both);
        });
    }
    while let Ok(Some(output)) = Output::read_from(&mut nexus) {
        if writeln!(text, "{}", output.datomize(vec![]).protosize().textualize()).is_err() {
            return;
        }
    }
}

fn main() -> ExitCode {
    let place = match Place::from_arguments() {
        Ok(place) => place,
        Err(unplaced) => {
            eprintln!("{unplaced:?}");
            return ExitCode::from(2);
        }
    };
    let listened = place.prepare().and_then(|()| Ok((listen(&place, Placed::TextSocket)?, listen(&place, Placed::HookSocket)?, listen(&place, Placed::EmissionSocket)?)));
    let (texts, hooks, emissions) = match listened {
        Ok(listeners) => listeners,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
    };
    thread::spawn(move || {
        for mut stream in emissions.incoming().flatten() {
            thread::spawn(move || {
                if let Ok(Some(emission)) = Follow::read_from(&mut stream) {
                    let _ = follow(stream, emission);
                }
            });
        }
    });
    let serving = place.clone();
    thread::spawn(move || {
        for stream in texts.incoming().flatten() {
            let place = serving.clone();
            thread::spawn(move || serve_text(stream, &place));
        }
    });
    for stream in hooks.incoming().flatten() {
        report(stream, &place);
    }
    ExitCode::SUCCESS
}
