#![cfg(feature = "unix")]

use std::io::{Cursor, Write};
use std::time::Duration;
use translator::unix::*;

#[test]
fn frame_limit_is_checked_before_body_allocation() {
    let mut bytes = Cursor::new((129_u32).to_be_bytes().to_vec());
    assert!(matches!(read_frame(&mut bytes, 128), Err(Error::TooLarge)));
}

#[test]
fn idle_is_unbounded_and_progress_is_bounded() {
    let (reader, mut writer) = std::os::unix::net::UnixStream::pair().unwrap();
    let peer = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(50));
        write_frame(&mut writer, &Frame { lane: 3, kind: 1, payload: vec![1] }, 64).unwrap();
    });
    let began = std::time::Instant::now();
    assert!(read_frame_after_idle(reader, Duration::from_millis(10), 64).unwrap().is_some());
    assert!(began.elapsed() >= Duration::from_millis(40));
    peer.join().unwrap();

    let (reader, mut writer) = std::os::unix::net::UnixStream::pair().unwrap();
    let (stalled, released) = std::sync::mpsc::channel::<()>();
    let peer = std::thread::spawn(move || {
        writer.write_all(&[0]).unwrap();
        released.recv().unwrap();
        drop(writer);
    });
    assert!(matches!(read_frame_after_idle(reader, Duration::from_millis(10), 64), Err(Error::Io(_))));
    stalled.send(()).unwrap();
    peer.join().unwrap();
}

#[test]
fn incomplete_submission_reports_written_bytes() {
    struct Stops { limit: usize, bytes: Vec<u8> }
    impl Write for Stops {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if self.bytes.len() == self.limit { return Err(std::io::ErrorKind::BrokenPipe.into()); }
            let count = bytes.len().min(self.limit - self.bytes.len());
            self.bytes.extend_from_slice(&bytes[..count]);
            Ok(count)
        }
        fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
    }
    let mut output = Stops { limit: 5, bytes: Vec::new() };
    let error = write_frame_witness(&mut output, &Frame { lane: 3, kind: 0, payload: vec![1] }, 64).unwrap_err();
    assert_eq!(error.written, 5);
}

#[test]
fn witnessed_submission_retries_an_interrupted_write() {
    struct InterruptedOnce {
        interrupted: bool,
        bytes: Vec<u8>,
    }
    impl Write for InterruptedOnce {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if !self.interrupted {
                self.interrupted = true;
                return Err(std::io::ErrorKind::Interrupted.into());
            }
            self.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
    }

    let frame = Frame { lane: 3, kind: 0, payload: vec![1] };
    let mut expected = Vec::new();
    write_frame(&mut expected, &frame, 64).unwrap();
    let mut output = InterruptedOnce { interrupted: false, bytes: Vec::new() };
    write_frame_witness(&mut output, &frame, 64).unwrap();
    assert_eq!(output.bytes, expected);
}

#[test]
fn admission_is_constructed_once_and_read_through_its_access_kind() {
    let admission = Admission::new(Duration::from_millis(7), 64, 3).unwrap();
    assert_eq!(admission.deadline(), Duration::from_millis(7));
    assert_eq!(admission.frame_capacity(), 64);
    assert_eq!(admission.connection_capacity(), 3);

    assert!(Admission::new(Duration::ZERO, 64, 3).is_err());
    assert!(Admission::new(Duration::from_millis(7), 1, 3).is_err());
    assert!(Admission::new(Duration::from_millis(7), 64, 0).is_err());
}

/// A directory of its own for one test, under the test's temporary root.
fn directory(name: &str) -> std::path::PathBuf {
    let directory = std::env::temp_dir().join(format!("translator-unix-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    directory
}

#[test]
fn an_awaited_socket_is_connected_once_it_appears_in_a_directory_made_after_the_wait() {
    let root = directory("appears");
    let socket = root.join("place").join("nexus-meta.sock");
    let binding = {
        let socket = socket.clone();
        std::thread::spawn(move || {
            // The wait is placed first; the directory and the socket come after.
            std::thread::sleep(Duration::from_millis(50));
            std::fs::create_dir(socket.parent().unwrap()).unwrap();
            let listener = socket.as_path().bind_appearing().unwrap();
            let (mut accepted, _) = listener.accept().unwrap();
            accepted.write_all(b"k").unwrap();
        })
    };
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let mut stream = socket.as_path().connect_when_present(deadline).expect("connected once it appeared");
    let mut byte = [0_u8; 1];
    std::io::Read::read_exact(&mut stream, &mut byte).unwrap();
    assert_eq!(&byte, b"k");
    binding.join().unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn the_deadline_bounds_a_socket_that_never_appears() {
    let root = directory("never");
    let socket = root.join("nexus-meta.sock");
    let began = std::time::Instant::now();
    let refused = socket.as_path().connect_when_present(began + Duration::from_millis(60)).unwrap_err();
    assert_eq!(refused, AwaitRefusal::Elapsed);
    assert!(began.elapsed() >= Duration::from_millis(60));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_directory_that_cannot_be_watched_is_refused() {
    // Root reads every directory, so only another login meets the refusal.
    if rustix::process::geteuid().is_root() {
        return;
    }
    let root = directory("unwatchable");
    let closed = root.join("closed");
    std::fs::create_dir(&closed).unwrap();
    std::fs::set_permissions(&closed, std::os::unix::fs::PermissionsExt::from_mode(0o000)).unwrap();
    let refused = closed
        .join("nexus-meta.sock")
        .as_path()
        .connect_when_present(std::time::Instant::now() + Duration::from_secs(10))
        .unwrap_err();
    std::fs::set_permissions(&closed, std::os::unix::fs::PermissionsExt::from_mode(0o700)).unwrap();
    assert!(matches!(refused, AwaitRefusal::Unwatchable(_)), "{refused:?}");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn an_appearing_socket_listens_at_its_path_and_refuses_one_already_there() {
    let root = directory("binding");
    let socket = root.join("nexus.sock");
    let listener = socket.as_path().bind_appearing().unwrap();
    let client = std::os::unix::net::UnixStream::connect(&socket).unwrap();
    let _ = listener.accept().unwrap();
    drop(client);
    assert!(socket.as_path().bind_appearing().is_err());
    let left: Vec<_> = std::fs::read_dir(&root).unwrap().map(|entry| entry.unwrap().file_name()).collect();
    assert_eq!(left, [std::ffi::OsString::from("nexus.sock")]);
    drop(listener);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn current_process_peer_is_live_and_matches_kernel_credentials() {
    let (left, right) = std::os::unix::net::UnixStream::pair().unwrap();
    let credentials = peer_credentials(&left).unwrap();
    let peer = LivePeer::capture(crate_identity(&credentials)).unwrap();
    assert!(peer.matches(&credentials));
    assert!(peer.matches_caller(&translator::Caller {
        process_id: credentials.process_id,
        user_id: credentials.user_id,
    }));
    drop(right);
}

fn crate_identity(credentials: &PeerCredentials) -> translator::PeerIdentity {
    translator::PeerIdentity {
        user_id: credentials.user_id,
        process_id: credentials.process_id,
    }
}
