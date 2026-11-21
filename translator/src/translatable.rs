//! What reads the words of the world a Nexus has no Signal contract with.

/// Why a payload was not read into a type of the Nexus.
#[derive(Debug, PartialEq, Eq)]
pub enum Untranslated {
    Unreadable,
    UnknownEvent(String),
    MissingField(&'static str),
}

/// What reads the harness's own words into a type of the Nexus; `T` is the
/// translator's own marker type, which is what lets the body live in the
/// translator's crate and not the Nexus's.
pub trait Translatable<T>: Sized {
    fn translate(text: &str) -> Result<Self, Untranslated>;
}
