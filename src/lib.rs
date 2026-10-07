//! Pure-Rust BMP (Windows bitmap) decoder + encoder + container.
//!
//! The crate follows the OxideAV image-crate contract
//! (`IMAGE_CRATE_API.md`): the root exposes [`probe`], [`info`],
//! [`decode`] / [`decode_with`] / [`decode_rgb8`] / [`decode_rgba8`] /
//! [`decode_from`], [`encode`] / [`encode_rgb8`] / [`encode_rgba8`] /
//! [`encode_to`], the native-layout [`BmpImage`] and the shared
//! [`RgbImage`] / [`RgbaImage`] / [`Plane`] / [`Palette`] /
//! [`ColorInfo`] / [`Metadata`] / [`ImageInfo`] records, all usable
//! with `default-features = false` (no `oxideav-core`).
//!
//! ```no_run
//! # fn main() -> Result<(), oxideav_bmp::Error> {
//! let bytes = std::fs::read("in.bmp")?;
//! if oxideav_bmp::probe(&bytes) {
//!     let info = oxideav_bmp::info(&bytes)?;          // header only
//!     let img = oxideav_bmp::decode(&bytes)?;         // native layout
//!     let rgba: Vec<u8> = img.to_rgba8();             // tightly packed RGBA
//!     let (w, h) = (img.width(), img.height());
//!     let out = oxideav_bmp::encode_rgba8(w, h, &rgba, &oxideav_bmp::EncodeOptions::default())?;
//!     std::fs::write("out.bmp", out)?;
//!     let _ = info;
//! }
//! # Ok(()) }
//! ```
//!
//! Decode handles 1 / 2 / 4 / 8-bit indexed (`BI_RGB`, `BI_RLE4`,
//! `BI_RLE8`), 16-bit 5-5-5 / 5-6-5 and arbitrary `BI_BITFIELDS` masks,
//! 24-bit and 32-bit (with or without an alpha mask), bottom-up and
//! top-down rows, and every header generation from the OS/2
//! `BITMAPCOREHEADER` to `BITMAPV5HEADER` (colour-space tag and
//! embedded ICC profile surfaced as [`BmpImage::color`] /
//! [`BmpImage::metadata`]). `BI_JPEG` / `BI_PNG` payloads and the CMYK
//! compressions are rejected.
//!
//! Encode writes 32-bit BGRA, 24-bit BGR, 16-bit 5-5-5 / 5-6-5, 8 / 4 /
//! 2 / 1-bit indexed (RLE8 / RLE4 when smaller), explicit-mask
//! `BI_BITFIELDS` / `BI_ALPHABITFIELDS`, V4 calibrated-RGB and V5
//! embedded / linked ICC headers — all selected through
//! [`EncodeOptions`] fields. [`encode_into`] appends the file to a
//! caller's buffer instead of returning a new one; with
//! [`encoded_size_bound`] bytes of spare capacity it does not allocate.
//!
//! The headerless "DIB" helpers ([`decode_dib`] / [`encode_dib`]) read
//! and write the same `BITMAPINFOHEADER` + pixel array without the
//! 14-byte `BITMAPFILEHEADER` and, on request, the 1-bpp AND mask that
//! `.ico` / `.cur` sub-images carry; `oxideav-ico` uses them.
//!
//! ## Standalone vs registry-integrated
//!
//! The default `registry` Cargo feature pulls in `oxideav-core` and
//! adds [`register`] (`&mut RuntimeContext`), [`make_decoder`] /
//! [`make_encoder`], and the `BmpImage` ⇄ `VideoFrame` bridge. The
//! framework decoder hands the pipeline `Rgba` frames (the container
//! declares `Rgba`); the standalone [`decode`] returns the native
//! layout.

mod api;
#[cfg(feature = "registry")]
pub mod container;
pub mod decoder;
pub mod encoder;
pub mod error;
pub mod image;
pub mod metadata;
mod options;
#[cfg(feature = "registry")]
pub mod registry;
pub mod types;

/// Codec id for BMP image frames.
pub const CODEC_ID_STR: &str = "bmp";

// ---- The image-crate contract (IMAGE_CRATE_API) ---------------------------
// Root vocabulary, identical across every oxideav image crate; works
// with `default-features = false`.
pub use api::{
    decode, decode_from, decode_rgb8, decode_rgba8, decode_with, encode, encode_rgb8, encode_rgba8,
    encode_to, encode_with_report, info, probe,
};
pub use error::{BmpError, Error, Result};
pub use image::{
    BmpImage, BmpPixelFormat, ColorInfo, ColorRange, ImageInfo, Metadata, Palette, PixelFormat,
    Plane, RgbImage, RgbaImage,
};
pub use options::{CalibratedRgb, DecodeOptions, EncodeOptions};

// ---- BMP-specific depth (the contract is a floor, not a ceiling) ----------
pub use api::{encode_into, encoded_size_bound};
pub use decoder::{decode_dib, decode_dib_with};
pub use encoder::{encode_dib, BmpBitfields, EncodedBmpFormat};
pub use metadata::{
    BmpColorSpace, BmpIccProfileRef, BmpMetadata, BmpOs2Halftone, BmpOs2Header2, BmpRenderingIntent,
};
pub use types::{
    row_stride, BitmapFileHeader, BitmapInfoHeader, BmpFileMagic, DibHeader, DibHeaderKind,
    BITMAPCOREHEADER_SIZE, BITMAPFILEHEADER_SIZE, BITMAPINFOHEADER_SIZE, BITMAPV2INFOHEADER_SIZE,
    BITMAPV3INFOHEADER_SIZE, BITMAPV4HEADER_SIZE, BITMAPV5HEADER_SIZE, BI_ALPHABITFIELDS,
    BI_BITFIELDS, BI_CMYK, BI_CMYKRLE4, BI_CMYKRLE8, BI_JPEG, BI_PNG, BI_RGB, BI_RLE4, BI_RLE8,
    BMP_MAGIC, LCS_CALIBRATED_RGB, LCS_GM_ABS_COLORIMETRIC, LCS_GM_BUSINESS, LCS_GM_GRAPHICS,
    LCS_GM_IMAGES, LCS_S_RGB, LCS_WINDOWS_COLOR_SPACE, OS22XBITMAPHEADER_SIZE,
    OS2_COLOR_ENCODING_RGB, OS2_HALFTONE_ERROR_DIFFUSION, OS2_HALFTONE_NONE, OS2_HALFTONE_PANDA,
    OS2_HALFTONE_SUPER_CIRCLE, OS2_MAGIC_BA, OS2_MAGIC_CI, OS2_MAGIC_CP, OS2_MAGIC_IC,
    OS2_MAGIC_PT, OS2_RECORDING_BOTTOM_UP, OS2_UNITS_PELS_PER_METER, PROFILE_EMBEDDED,
    PROFILE_LINKED,
};

// ---- Deprecated pre-contract names (one release) --------------------------
#[allow(deprecated)]
pub use decoder::{decode_bmp, decode_bmp_with_metadata, decode_dib_with_metadata};
#[allow(deprecated)]
pub use encoder::{
    encode_bmp, encode_bmp_bitfields, encode_bmp_plane, encode_bmp_plane_bitfields,
    encode_bmp_plane_with_options, encode_bmp_with_calibrated_rgb, encode_bmp_with_icc_profile,
    encode_bmp_with_linked_icc_profile, encode_bmp_with_options, encode_dib_plane,
};
#[allow(deprecated)]
pub use image::{BmpPalette, BmpPlane};
#[allow(deprecated)]
pub use options::BmpEncodeOptions;

// ---- Registry-gated framework surface -------------------------------------
#[cfg(feature = "registry")]
pub use registry::{
    __oxideav_entry, from_color_signal, from_core_pixel_format, make_decoder, make_encoder,
    register, register_codecs, register_containers, to_color_signal, to_core_pixel_format,
    BmpDecoder, BmpEncoder,
};
#[cfg(feature = "registry")]
#[allow(deprecated)]
pub use registry::{
    decode_bmp_videoframe, decode_dib_videoframe, encode_bmp_videoframe, encode_dib_videoframe,
};

#[cfg(test)]
mod tests;
