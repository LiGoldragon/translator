//! The identifier of one run of a Nexus, which its pair launcher draws.

use nexus_serving::InstanceId;

pub trait FreshInstance: Sized {
    /// A new identifier from the kernel's randomness: sixteen bytes, written
    /// as thirty-two hexadecimal digits.
    fn fresh() -> Result<Self, Box<dyn std::error::Error>>;
}

impl FreshInstance for InstanceId {
    fn fresh() -> Result<Self, Box<dyn std::error::Error>> {
        let mut bytes = [0_u8; 16];
        let mut filled = 0;
        while filled < bytes.len() {
            let count = rustix::rand::getrandom(
                &mut bytes[filled..],
                rustix::rand::GetRandomFlags::empty(),
            )?;
            if count == 0 {
                return Err("kernel randomness returned no bytes".into());
            }
            filled += count;
        }
        let mut identifier = String::with_capacity(bytes.len() * 2);
        for byte in bytes {
            use std::fmt::Write as _;
            write!(&mut identifier, "{byte:02x}")?;
        }
        Ok(Self(identifier))
    }
}
