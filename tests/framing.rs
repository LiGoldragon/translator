use std::io::Cursor;
use translator::{FrameCapacity, FrameReading, FrameWriting, Framable};

#[test]
fn typed_frame_roundtrip() {
    let body = b"hello".to_vec();
    let frame_body = translator::FrameBody::from(body.clone());
    let mut bytes = Vec::new();
    bytes.write_frame(&frame_body, FrameCapacity::default()).unwrap();
    assert_eq!(&bytes[..4], &[0, 0, 0, 5]);
    let read = Cursor::new(bytes).read_frame(FrameCapacity::default()).unwrap();
    assert_eq!(Vec::<u8>::from(read), body);
}

#[test]
fn oversized_frame_is_refused() {
    let body = translator::FrameBody::from(vec![0; 4]);
    let result = body.framed(FrameCapacity::from(3));
    assert!(matches!(result, Err(translator::FrameError::BodyTooLarge { .. })));
}

#[test]
fn eof_is_reported() {
    let result = Cursor::new(vec![0, 0]).read_frame(FrameCapacity::default());
    assert!(matches!(result, Err(translator::FrameError::Io(_))));
}

#[test]
fn bad_archive_is_refused() {
    let signal = translator::Signal::<u32>::from(vec![0xff, 0x00, 0xff]);
    assert!(translator::Restorable::restore(&signal).is_err());
}
