//! The translator, started with its Nexus's place: the hook socket takes
//! Claude Code's payloads, one at a time and in order; the emission socket
//! takes each emission on its own connection.

use std::io::Read;
use std::os::unix::net::{UnixListener, UnixStream};
use std::process::ExitCode;
use std::thread;

use claude_harness_process::*;
use claude_harness_translator::{Translatable, caller, follow};
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

fn main() -> ExitCode {
    let place = match Place::from_arguments() {
        Ok(place) => place,
        Err(unplaced) => {
            eprintln!("{unplaced:?}");
            return ExitCode::from(2);
        }
    };
    let (hooks, emissions) = match place.prepare().and_then(|()| Ok((listen(&place, Placed::HookSocket)?, listen(&place, Placed::EmissionSocket)?))) {
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
    for stream in hooks.incoming().flatten() {
        report(stream, &place);
    }
    ExitCode::SUCCESS
}
