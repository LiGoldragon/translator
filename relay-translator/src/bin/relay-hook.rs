//! The hook client of the Relay Nexus: run with a payload on its input.

use relay_translator::RelayTranslator;
use translator::Hooking;

fn main() -> std::process::ExitCode {
    RelayTranslator::hook()
}
