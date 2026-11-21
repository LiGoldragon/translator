//! The running of a program: its status, and what it wrote.

use translator::{Captured, capture, run};

fn shell(script: &str) -> Vec<String> {
    vec!["/bin/sh".into(), "-c".into(), script.into()]
}

#[test]
fn a_program_that_ends_well_is_zero() {
    assert_eq!(run(&shell("exit 0")), 0);
}

#[test]
fn a_program_that_refuses_gives_its_status() {
    assert_eq!(run(&shell("exit 3")), 3);
}

#[test]
fn a_program_ended_by_a_signal_is_128_and_the_signal() {
    assert_eq!(run(&shell("kill -9 $$")), 137);
}

#[test]
fn a_program_that_cannot_be_run_is_127() {
    assert_eq!(run(&["/nonexistent/program".into()]), 127);
    assert_eq!(run(&[]), 127);
}

#[test]
fn what_a_program_wrote_is_read_back() {
    assert_eq!(capture(&shell("echo hello; exit 2")), Captured { status: 2, output: "hello\n".into() });
}

#[test]
fn a_program_reads_nothing() {
    assert_eq!(capture(&shell("cat")), Captured { status: 0, output: String::new() });
}
