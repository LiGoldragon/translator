//! The translator, started with its Nexus's place: the text socket takes a
//! datom typed at the CLI and writes back each output as a datom; the hook
//! socket takes Claude Code's payloads, one at a time and in order; the
//! emission socket takes each emission on its own connection.

use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::process::ExitCode;
use std::thread;

use claude_harness_process::*;
use claude_harness_translator::{Translatable, caller, follow, itself};
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

/// One datom in, each output out as a datom line, until the Nexus or the caller
/// closes: the ordinary text socket and the meta one, each to its own Nexus socket.
fn serve_text<I, C, O>(mut text: UnixStream, place: &Place, nexus_socket: Placed, called: fn(Caller, I) -> C)
where
    I: datom_codec::Composing,
    C: Framed,
    O: Framed + Datomizable,
{
    let Ok(caller) = caller(&text) else { return };
    let mut datom = String::new();
    if text.read_to_string(&mut datom).is_err() {
        return;
    }
    let input: I = match transport::actualize(&datom) {
        Ok(input) => input,
        Err(unplaced) => {
            let _ = writeln!(text, "Unreadable");
            eprintln!("{unplaced:?}");
            return;
        }
    };
    let Ok(mut nexus) = UnixStream::connect(place.path(nexus_socket)) else {
        let _ = writeln!(text, "NexusDown");
        return;
    };
    if called(caller, input).write_to(&mut nexus).is_err() {
        return;
    }
    if let (Ok(hanging), Ok(closing)) = (text.try_clone(), nexus.try_clone()) {
        thread::spawn(move || {
            let mut waiting = [PollFd::new(&hanging, PollFlags::empty())];
            let _ = rustix::event::poll(&mut waiting, None);
            let _ = closing.shutdown(std::net::Shutdown::Both);
        });
    }
    while let Ok(Some(output)) = O::read_from(&mut nexus) {
        if writeln!(text, "{}", output.datomize(vec![]).protosize().textualize()).is_err() {
            break;
        }
    }
    let _ = text.shutdown(std::net::Shutdown::Both);
}

/// Once its sockets listen, the translator tells its Nexus so, and the Nexus
/// gives each record that has a process and no living one its process again.
/// A Nexus that is down is told nothing: it bears them at its own start.
fn listening(place: &Place) {
    let (Ok(caller), Ok(mut nexus)) = (itself(), UnixStream::connect(place.path(Placed::NexusSocket))) else { return };
    let _ = (Call { caller, input: Input::Listening }).write_to(&mut nexus);
}

fn serve_texts(texts: UnixListener, place: &Place, serve: fn(UnixStream, &Place)) {
    let place = place.clone();
    thread::spawn(move || {
        for stream in texts.incoming().flatten() {
            let place = place.clone();
            thread::spawn(move || serve(stream, &place));
        }
    });
}

fn main() -> ExitCode {
    let place = match Place::from_arguments() {
        Ok(place) => place,
        Err(unplaced) => {
            eprintln!("{unplaced:?}");
            return ExitCode::from(2);
        }
    };
    let listened = place.prepare().and_then(|()| {
        Ok((listen(&place, Placed::TextSocket)?, listen(&place, Placed::MetaTextSocket)?, listen(&place, Placed::HookSocket)?, listen(&place, Placed::EmissionSocket)?))
    });
    let (texts, meta_texts, hooks, emissions) = match listened {
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
    listening(&place);
    serve_texts(texts, &place, |text, place| serve_text::<Input, Call, Output>(text, place, Placed::NexusSocket, |caller, input| Call { caller, input }));
    serve_texts(meta_texts, &place, |text, place| {
        serve_text::<MetaInput, MetaCall, MetaOutput>(text, place, Placed::MetaSocket, |caller, meta_input| MetaCall { caller, meta_input })
    });
    for stream in hooks.incoming().flatten() {
        report(stream, &place);
    }
    ExitCode::SUCCESS
}
