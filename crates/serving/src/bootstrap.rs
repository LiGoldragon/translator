//! The frame a translator is started with.

use shared_translator::{FrameCapacity, FrameReading, Restorable, Signal};

use crate::generated::TranslatorBootstrap;

pub trait BootstrapReading: Sized {
    /// Reads the one bootstrap frame the pair launcher writes on standard
    /// input, then closes standard input.
    fn read_standard_input() -> Result<Self, Box<dyn std::error::Error>>;
}

impl BootstrapReading for TranslatorBootstrap {
    fn read_standard_input() -> Result<Self, Box<dyn std::error::Error>> {
        let body = std::io::stdin().read_frame(FrameCapacity::default())?;
        let bootstrap = Signal::<Self>::from(Vec::<u8>::from(body)).restore()?;
        nix::unistd::close(0)?;
        Ok(bootstrap)
    }
}
