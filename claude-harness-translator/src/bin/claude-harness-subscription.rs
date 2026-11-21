//! The subscription CLI: signal-harness, as the Claude Harness Nexus serves it.

fn main() -> std::process::ExitCode {
    claude_harness_translator::call(transport::Placed::SubscriptionTextSocket)
}
