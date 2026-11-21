//! The framing of Signal on a stream.

use std::io::{self, Read, Write};

/// The largest frame a reader accepts; a length beyond it is refused before any
/// memory is spent on it.
pub const MAXIMUM_FRAME: u32 = 16 * 1024 * 1024;

type HighSerializer<'a> = rkyv::api::high::HighSerializer<rkyv::util::AlignedVec, rkyv::ser::allocator::ArenaHandle<'a>, rkyv::rancor::Error>;
type HighValidator<'a> = rkyv::api::high::HighValidator<'a, rkyv::rancor::Error>;
type HighDeserializer = rkyv::api::high::HighDeserializer<rkyv::rancor::Error>;

fn unreadable(error: rkyv::rancor::Error) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error)
}

/// One Signal value on a stream: its length, then its archive.
pub trait Framed: Sized {
    fn write_to<W: Write>(&self, stream: &mut W) -> io::Result<()>;
    /// None is a stream that ended before a frame began.
    fn read_from<R: Read>(stream: &mut R) -> io::Result<Option<Self>>;
}

impl<T> Framed for T
where
    T: rkyv::Archive + for<'a> rkyv::Serialize<HighSerializer<'a>>,
    T::Archived: for<'a> rkyv::bytecheck::CheckBytes<HighValidator<'a>> + rkyv::Deserialize<T, HighDeserializer>,
{
    fn write_to<W: Write>(&self, stream: &mut W) -> io::Result<()> {
        let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(self).map_err(unreadable)?;
        let length = u32::try_from(bytes.len()).ok().filter(|length| *length <= MAXIMUM_FRAME).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "a frame over the maximum"))?;
        stream.write_all(&length.to_le_bytes())?;
        stream.write_all(&bytes)?;
        stream.flush()
    }

    fn read_from<R: Read>(stream: &mut R) -> io::Result<Option<Self>> {
        let mut length = [0u8; 4];
        match stream.read_exact(&mut length) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
            Err(error) => return Err(error),
        }
        let length = u32::from_le_bytes(length);
        if length > MAXIMUM_FRAME {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "a frame over the maximum"));
        }
        let mut bytes = vec![0u8; length as usize];
        stream.read_exact(&mut bytes)?;
        rkyv::from_bytes::<T, rkyv::rancor::Error>(&bytes).map(Some).map_err(unreadable)
    }
}
