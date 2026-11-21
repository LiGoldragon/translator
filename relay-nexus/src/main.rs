//! relay-nexus '<place>': the Relay Nexus, started in its place; it makes its
//! one socket, relay.sock, and serves it until it is ended.

use std::os::unix::net::UnixListener;
use std::process::ExitCode;

use relay_nexus::Nexus;
use relay_signal::Relay;
use translator_transport::{Place, Placed};

fn main() -> ExitCode {
    let place = match Place::from_arguments() {
        Ok(place) => place,
        Err(unsent) => {
            eprintln!("{unsent:?}");
            return ExitCode::from(2);
        }
    };
    let path = place.path::<Relay>(Placed::NexusSocket);
    let listener = place.prepare().and_then(|()| {
        let _ = std::fs::remove_file(&path);
        UnixListener::bind(&path)
    });
    match listener {
        Ok(listener) => {
            Nexus::new(&place).listen(listener);
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
