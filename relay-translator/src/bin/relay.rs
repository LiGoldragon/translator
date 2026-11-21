//! The CLI of the Relay Nexus.

use relay_translator::RelayTranslator;
use translator::{Calling, TextSocket};

fn main() -> std::process::ExitCode {
    RelayTranslator::call(TextSocket::Ordinary).into()
}
