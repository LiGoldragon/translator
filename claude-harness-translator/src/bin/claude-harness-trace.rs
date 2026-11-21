//! Reads the Nexus's archived trace, as Signal, and writes it out as datom,
//! one line for each birth, gift, taking and end.

use std::fs::File;
use std::io::BufReader;
use std::process::ExitCode;

use claude_harness_process::Process;
use datom_codec::Datomizable;
use process::Traced;
use protos::{Protosizable, Textualizable};
use transport::{Framed, Place, Placed};

fn main() -> ExitCode {
    let place = match Place::from_arguments() {
        Ok(place) => place,
        Err(unplaced) => {
            eprintln!("{unplaced:?}");
            return ExitCode::from(2);
        }
    };
    let mut archive = match File::open(place.path(Placed::Trace)) {
        Ok(file) => BufReader::new(file),
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
    };
    loop {
        match Traced::<Process>::read_from(&mut archive) {
            Ok(Some(trace)) => println!("{}", trace.datomize(vec![]).protosize().textualize()),
            Ok(None) => return ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("{error}");
                return ExitCode::FAILURE;
            }
        }
    }
}
