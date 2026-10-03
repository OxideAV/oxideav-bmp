//! Crate-local error type used by `oxideav-bmp`'s standalone (no
//! `oxideav-core`) public API.
//!
//! When the `registry` feature is enabled, [`BmpError`] gains a
//! `From<BmpError> for oxideav_core::Error` impl (defined in
//! [`crate::registry`]) so the trait-side surface (`Decoder` /
//! `Encoder`) can keep returning `oxideav_core::Result<T>` while the
//! underlying decode/encode functions stay framework-free.

use core::fmt;

/// `Result` alias scoped to `oxideav-bmp`.
pub type Result<T> = core::result::Result<T, BmpError>;

/// Contract alias: `oxideav_bmp::Error` is [`BmpError`].
pub type Error = BmpError;

/// Error variants returned by `oxideav-bmp`'s standalone API.
///
/// The enum does not implement `Clone` / `PartialEq` (the `Io` variant
/// carries a `std::io::Error`); match on the variant or on `Display`.
#[derive(Debug)]
#[non_exhaustive]
pub enum BmpError {
    /// The byte stream is malformed (bad magic, truncated header,
    /// pixel array runs past the end of the file, …).
    InvalidData(String),
    /// The byte stream uses a feature this codec doesn't implement
    /// (embedded JPEG / PNG payloads, the CMYK compressions, OS/2
    /// container wrappers) or the encoder was asked for a layout /
    /// option combination it cannot write.
    Unsupported(String),
    /// A [`crate::DecodeOptions`] limit (dimensions, pixel count,
    /// decoded bytes) was exceeded. Raised before any pixel buffer is
    /// allocated.
    LimitExceeded(String),
    /// An I/O error from [`crate::decode_from`] / [`crate::encode_to`].
    Io(std::io::Error),
}

impl BmpError {
    /// Construct a [`BmpError::InvalidData`] from a stringy message.
    pub fn invalid(msg: impl Into<String>) -> Self {
        Self::InvalidData(msg.into())
    }

    /// Construct a [`BmpError::Unsupported`] from a stringy message.
    pub fn unsupported(msg: impl Into<String>) -> Self {
        Self::Unsupported(msg.into())
    }

    /// Construct a [`BmpError::LimitExceeded`] from a stringy message.
    pub fn limit(msg: impl Into<String>) -> Self {
        Self::LimitExceeded(msg.into())
    }
}

impl From<std::io::Error> for BmpError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl fmt::Display for BmpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidData(s) => write!(f, "invalid data: {s}"),
            Self::Unsupported(s) => write!(f, "unsupported: {s}"),
            Self::LimitExceeded(s) => write!(f, "limit exceeded: {s}"),
            Self::Io(e) => write!(f, "io: {e}"),
        }
    }
}

impl std::error::Error for BmpError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            _ => None,
        }
    }
}
