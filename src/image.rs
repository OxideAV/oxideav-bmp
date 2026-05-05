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
///
/// Additional encode-only formats:
/// * [`Rgb565`](Self::Rgb565) — 16-bit RGB 5-6-5, emit as BI_BITFIELDS V4.
/// * [`Indexed8`](Self::Indexed8) — 8-bit palette index; the caller must
///   supply a [`BmpPalette`] alongside the plane.
/// * [`Indexed4`](Self::Indexed4) — 4-bit palette index (hi-nibble = left
///   pixel); caller must supply a [`BmpPalette`] of up to 16 entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BmpPixelFormat {
    /// 8-bit packed RGBA, 4 bytes per pixel.
    Rgba,
    /// 8-bit packed RGB, 3 bytes per pixel (encode input only).
    Rgb24,
    /// 16-bit RGB 5-6-5, 2 bytes per pixel (encode input only).
    /// Emitted with a `BI_BITFIELDS` BITMAPV4HEADER and canonical masks
    /// R=0xF800, G=0x07E0, B=0x001F.
    Rgb565,
    /// 8-bit palette index, 1 byte per pixel (encode input only).
    /// Must be paired with a [`BmpPalette`] that owns the colour table.
    Indexed8,
    /// 4-bit palette index, hi-nibble = left pixel (encode input only).
    /// Must be paired with a [`BmpPalette`] of up to 16 entries.
    Indexed4,
}

/// A colour palette for use with indexed BMP formats.
///
/// Each entry is `[R, G, B]` (24-bit sRGB). Up to 256 entries for 8-bit
/// mode; up to 16 for 4-bit mode. The encoder writes the entries in the
/// BMP on-disk order (B, G, R, 0x00).
#[derive(Debug, Clone, Default)]
pub struct BmpPalette {
    /// Colour entries in `[R, G, B]` order.
    pub entries: Vec<[u8; 3]>,
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
    /// Colour table for indexed pixel formats ([`BmpPixelFormat::Indexed8`]
    /// and [`BmpPixelFormat::Indexed4`]). `None` for direct-colour
    /// formats and on the decode path (which always produces `Rgba`).
    pub palette: Option<BmpPalette>,
    /// Optional presentation timestamp. Always `None` from the
    /// standalone decode path.
    pub pts: Option<i64>,
}
