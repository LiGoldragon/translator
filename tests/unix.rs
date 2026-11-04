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
    let peer = std::thread::spawn(move || {
        writer.write_all(&[0]).unwrap();
        std::thread::sleep(Duration::from_millis(60));
    });
    assert!(matches!(read_frame_after_idle(reader, Duration::from_millis(10), 64), Err(Error::Io(_))));
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
fn accept_retries_only_recoverable_failures() {
    let mut attempts = 0;
    let mut waits = Vec::new();
    let value = accept_with_retry_using(
        || { attempts += 1; if attempts == 1 { Err(std::io::Error::from_raw_os_error(libc::EMFILE)) } else { Ok(7) } },
        |delay| waits.push(delay),
        Duration::from_millis(3),
    ).unwrap();
    assert_eq!(value, 7);
    assert_eq!(waits, [Duration::from_millis(3)]);
    assert_eq!(accept_with_retry_using(|| Err::<(), _>(std::io::Error::from_raw_os_error(libc::EBADF)), |_| {}, Duration::from_millis(3)).unwrap_err().raw_os_error(), Some(libc::EBADF));
}

#[test]
fn current_process_peer_is_live_and_matches_kernel_credentials() {
    let (left, right) = std::os::unix::net::UnixStream::pair().unwrap();
    let credentials = peer_credentials(&left).unwrap();
    let peer = LivePeer::capture(crate_identity(&credentials)).unwrap();
    assert!(peer.matches(&credentials));
    assert!(peer.matches_caller(&translator::Caller {
        process_id: credentials.process_id.clone(),
        user_id: credentials.user_id.clone(),
    }));
    drop(right);
}

fn crate_identity(credentials: &PeerCredentials) -> translator::PeerIdentity {
    translator::PeerIdentity {
        user_id: credentials.user_id.clone(),
        process_id: credentials.process_id.clone(),
    }
}
