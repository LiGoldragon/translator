//! `nexus-configure '<datom>'`: compiles a place written in datom into the
//! place file every Nexus reads before any socket opens.
//!
//! ```sh
//! nexus-configure 'Compile.{ place.datom flow-nexus.signal }'
//! ```
//!
//! It answers one datom: `Compiled.<signal file>` (exit 0), or `Refused.`
//! with the file at fault and why (exit 2).

use datom_codec::Budget;
use std::process::ExitCode;
use translator::place_file::CompilesPlace;
use translator::placement::{Configure, Configured};

/// The one argument, read as what nexus-configure takes.
trait ReadsArguments {
    const USAGE: &'static str = "usage: nexus-configure 'Compile.{ «datom file» «signal file» }'";
    fn configure(&self) -> Result<Configure, String>;
}

impl ReadsArguments for [String] {
    fn configure(&self) -> Result<Configure, String> {
        let [argument] = self else {
            return Err(Self::USAGE.into());
        };
        let mut budget = Budget {
            remaining: 65_536,
            reader: protos::ReaderBudget::try_from(65_536).expect("a constant reader budget"),
            depth: 0,
            maximum_depth: 64,
        };
        translator::decode::<Configure>(argument, &mut budget)
            .map_err(|error| format!("{}: {error:?}", Self::USAGE))
    }
}

fn main() -> ExitCode {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    let configure = match arguments.configure() {
        Ok(configure) => configure,
        Err(usage) => {
            eprintln!("{usage}");
            return ExitCode::from(2);
        }
    };
    let configured = match configure {
        Configure::Compile(compilation) => compilation.compile(),
    };
    println!("{}", translator::encode(&configured));
    match configured {
        Configured::Compiled(_) => ExitCode::SUCCESS,
        Configured::Refused(_) => ExitCode::from(2),
    }
}
