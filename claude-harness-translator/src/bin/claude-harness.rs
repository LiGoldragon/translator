//! The CLI of the Claude Harness Nexus.

fn main() -> std::process::ExitCode {
    claude_harness_translator::call(transport::Placed::TextSocket)
}
