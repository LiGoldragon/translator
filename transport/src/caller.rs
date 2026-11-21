//! The caller of a connection: its process and the user that owns it, read
//! from the kernel's peer credentials.

use std::io;
use std::os::unix::net::UnixStream;

use datom_codec::{Composing, Datomizable};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize, Datomizable, Composing)]
pub struct ProcessId(pub i64);
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize, Datomizable, Composing)]
pub struct UserId(pub i64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize, Datomizable, Composing)]
pub struct Caller {
    pub process_id: ProcessId,
    pub user_id: UserId,
}

impl Caller {
    /// The caller at the other end of a connection.
    pub fn of(stream: &UnixStream) -> io::Result<Caller> {
        let credentials = rustix::net::sockopt::socket_peercred(stream)?;
        Ok(Caller { process_id: ProcessId(credentials.pid.as_raw_nonzero().get().into()), user_id: UserId(credentials.uid.as_raw().into()) })
    }

    /// The translator as a caller: its process, and the user that owns it.
    pub fn itself() -> io::Result<Caller> {
        use std::os::unix::fs::MetadataExt;
        let user_id = std::fs::metadata("/proc/self")?.uid();
        Ok(Caller { process_id: ProcessId(std::process::id().into()), user_id: UserId(user_id.into()) })
    }
}
