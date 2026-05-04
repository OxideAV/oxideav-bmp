//! Crate-local error type used by `oxideav-bmp`'s standalone (no
//! `oxideav-core`) public API.
//!
//! When the `registry` feature is enabled, [`BmpError`] gains a
//! `From<BmpError> for oxideav_core::Error` impl (defined in
//! [`crate::registry`]) so the trait-side surface (`Decoder` /
//! `Encoder`) can keep returning `oxideav_core::Result<T>` while the
//! underlying decode/encode functions stay framework-free.

use core::fmt;

/// `Result` alias scoped to `oxideav-bmp`. Standalone (no `oxideav-core`)
/// callers see this; framework callers convert via the gated
/// `From<BmpError> for oxideav_core::Error` impl.
pub type Result<T> = core::result::Result<T, BmpError>;

/// Error variants returned by `oxideav-bmp`'s standalone API.
///
/// The variants mirror the subset of `oxideav_core::Error` the codec
/// can hit. The crate intentionally avoids surfacing transport (`Io`)
/// or framework-specific (`FormatNotFound`, `CodecNotFound`) errors —
/// those originate in callers that are already linking `oxideav-core`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BmpError {
    /// The byte stream is malformed (bad magic, truncated header,
    /// pixel array runs past the end of the file, …).
    InvalidData(String),
    /// The byte stream uses a feature this codec doesn't implement
    /// (RLE4 / RLE8 / embedded JPEG / embedded PNG compression types,
    /// or a pixel format the encoder can't lay out).
    Unsupported(String),
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
}

impl fmt::Display for BmpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidData(s) => write!(f, "invalid data: {s}"),
            Self::Unsupported(s) => write!(f, "unsupported: {s}"),
        }
    }
}

impl std::error::Error for BmpError {}
