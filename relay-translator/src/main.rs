//! relay-translator '<place>': the Relay Nexus's translator, started in its place.
//! Its text socket takes a datom typed at the CLI; its hook socket takes the
//! payloads relay-hook brings; its emission socket takes what the Nexus sends.

use std::process::ExitCode;

use relay_signal::signal::{Call, Hook, Input, Output};
use relay_signal::{ETHOS, Emission};
use relay_translator::RelayTranslator;
use translator::{Caller, Placed, Translator};

fn translate() -> std::io::Result<Translator<RelayTranslator>> {
    let mut translator = Translator::<RelayTranslator>::from_arguments().map_err(|unsent| std::io::Error::other(format!("{unsent:?}")))?;
    translator.prepare()?;
    translator.text::<Input, Call, Output>(Placed::TextSocket, Placed::NexusSocket, ETHOS, |caller, input| Call { caller, input })?;
    translator.hooks::<Hook, Call, Output>(|caller, hook| Call { caller, input: Input::Report(hook) })?;
    translator.emissions::<Emission>()?;
    if let Ok(caller) = Caller::itself() {
        let _: Option<Output> = translator.listening(&Call { caller, input: Input::Listening });
    }
    Ok(translator)
}

fn main() -> ExitCode {
    match translate() {
        Ok(translator) => {
            translator.serve();
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
