//! Standalone image container returned by `oxideav-bmp`'s framework-free
//! decode API and accepted by the standalone encode API.
//!
//! Defined here (rather than reusing `oxideav_core::VideoFrame`) so the
//! crate can be built with the default `registry` feature off — i.e.
//! without depending on `oxideav-core` at all. When the `registry`
//! feature is on the [`crate::registry`] module provides
//! `From<BmpImage> for oxideav_core::VideoFrame` (and the matching
//! [`BmpPixelFormat`] ↔ `oxideav_core::PixelFormat` mapping) so the
//! trait-side `Decoder` / `Encoder` impls keep working unchanged.

/// Pixel layout used by [`BmpImage`].
///
/// The decoder always produces [`BmpPixelFormat::Rgba`] (palette
/// expansion + BGR→RGB swapping happens at decode time so consumers
/// don't need to know the on-disk quirks). The encoder accepts
/// [`Rgba`](Self::Rgba) (passes alpha through) or [`Rgb24`](Self::Rgb24)
/// (alpha defaulted to `0xFF`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BmpPixelFormat {
    /// 8-bit packed RGBA, 4 bytes per pixel.
    Rgba,
    /// 8-bit packed RGB, 3 bytes per pixel (encode input only).
    Rgb24,
}

/// One image plane: row-major bytes plus the row stride in bytes.
///
/// Mirrors `oxideav_core::VideoPlane` so the registry-side conversion
/// is a trivial field-by-field copy.
#[derive(Debug, Clone)]
pub struct BmpPlane {
    /// Bytes per row in `data`.
    pub stride: usize,
    /// Raw plane bytes, packed `stride` × number of rows.
    pub data: Vec<u8>,
}

/// One decoded BMP frame, framework-free shape.
///
/// `pts` is `None` for the standalone [`crate::decode_bmp`] /
/// [`crate::decode_dib`] entry points. The registry-backed `Decoder`
/// impl still passes `pts` through from the surrounding `Packet`.
#[derive(Debug, Clone)]
pub struct BmpImage {
    /// Picture width in pixels.
    pub width: u32,
    /// Picture height in pixels.
    pub height: u32,
    /// Pixel layout the planes carry. Always [`BmpPixelFormat::Rgba`]
    /// on the decode path.
    pub pixel_format: BmpPixelFormat,
    /// One [`BmpPlane`] per plane. BMP always packs into a single plane
    /// today, so this is always `len() == 1`.
    pub planes: Vec<BmpPlane>,
    /// Optional presentation timestamp. Always `None` from the
    /// standalone decode path.
    pub pts: Option<i64>,
}
