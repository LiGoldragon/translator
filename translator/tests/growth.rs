//! A file followed for growth, and a connection that closes.

use std::io::Write;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

use translator::{Growth, Lasted, Waited};

fn scratch(name: &str) -> PathBuf {
    let directory = std::env::temp_dir().join(format!("tg-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).expect("a scratch directory");
    directory
}

fn connection() -> (Lasted, UnixStream) {
    let (translator, nexus) = UnixStream::pair().expect("a pair");
    (Lasted::new(translator), nexus)
}

#[test]
fn a_file_not_yet_written_is_waited_for_in_its_nearest_ancestor() {
    let directory = scratch("appear");
    let path = directory.join("missing").join("file");
    let (lasted, _nexus) = connection();
    let mut growth = Growth::begin(&path).expect("begun");
    assert!(!growth.present().expect("an ancestor exists"));
    std::fs::create_dir(directory.join("missing")).expect("a directory");
    assert_eq!(growth.wait(&lasted).expect("waited"), Waited::Woken);
    assert!(!growth.present().expect("still not there"));
    std::fs::write(&path, "one\n").expect("written");
    assert_eq!(growth.wait(&lasted).expect("waited"), Waited::Woken);
    assert!(growth.present().expect("there"));
}

#[test]
fn a_path_with_no_ancestor_is_not_found() {
    let (_lasted, _nexus) = connection();
    let mut growth = Growth::begin("relative-without-a-directory/that/exists").expect("begun");
    let error = growth.present().expect_err("no ancestor");
    assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
}

#[test]
fn whole_lines_are_read_from_an_offset_a_count_at_a_time_and_a_last_line_not_ended_waits() {
    let directory = scratch("lines");
    let path = directory.join("file");
    std::fs::write(&path, "one\ntwo\r\nthree\nfou").expect("written");
    let mut growth = Growth::begin(&path).expect("begun");
    assert!(growth.present().expect("there"));
    let first = growth.lines_after(0, 2).expect("read");
    assert_eq!(first.lines, ["one", "two"]);
    assert_eq!(first.offset, 9);
    let second = growth.lines_after(first.offset, 2).expect("read");
    assert_eq!(second.lines, ["three"]);
    assert_eq!(growth.end().expect("a length"), 18);
    std::fs::OpenOptions::new().append(true).open(&path).expect("opened").write_all(b"r\n").expect("appended");
    assert_eq!(growth.lines_after(second.offset, 2).expect("read").lines, ["four"]);
}

#[test]
fn a_wait_ends_when_the_nexus_closes_the_connection() {
    let directory = scratch("close");
    let path = directory.join("file");
    std::fs::write(&path, "").expect("written");
    let (lasted, nexus) = connection();
    let mut growth = Growth::begin(&path).expect("begun");
    assert!(growth.present().expect("there"));
    let closing = thread::spawn(move || {
        thread::sleep(Duration::from_millis(100));
        drop(nexus);
    });
    assert_eq!(growth.wait(&lasted).expect("waited"), Waited::Closed);
    closing.join().expect("joined");
}
