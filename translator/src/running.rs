//! Running a program: the words of the call are the body's, the running is the library's.

use std::os::unix::process::ExitStatusExt;
use std::process::{Command, Stdio};

/// The status a program that could not be run is given, as a shell gives it.
const NOT_RUN: i32 = 127;

/// What a program wrote and how it ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Captured {
    pub status: i32,
    pub output: String,
}

fn status(status: std::process::ExitStatus) -> i32 {
    status.code().unwrap_or_else(|| 128 + status.signal().unwrap_or(0))
}

/// Runs the program with its arguments, reading nothing, and gives its status:
/// 0 is done, a signal is 128 and its number, and 127 is a program that could not be run.
pub fn run(argv: &[String]) -> i32 {
    let Some((program, arguments)) = argv.split_first() else { return NOT_RUN };
    Command::new(program).args(arguments).stdin(Stdio::null()).status().map_or(NOT_RUN, status)
}

/// The same, with what the program wrote to its output read back.
pub fn capture(argv: &[String]) -> Captured {
    let Some((program, arguments)) = argv.split_first() else { return Captured { status: NOT_RUN, output: String::new() } };
    match Command::new(program).args(arguments).stdin(Stdio::null()).output() {
        Ok(output) => Captured { status: status(output.status), output: String::from_utf8_lossy(&output.stdout).into_owned() },
        Err(_) => Captured { status: NOT_RUN, output: String::new() },
    }
}
