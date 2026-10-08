//! The image-crate contract root vocabulary (`IMAGE_CRATE_API.md`):
//! `probe` / `info` / `decode*` / `encode*`, framework-free.

use std::io::{Read, Write};

use crate::decoder;
use crate::encoder::{self, EncodedBmpFormat};
use crate::error::Result;
use crate::image::{BmpImage, ImageInfo, RgbImage, RgbaImage};
use crate::options::{DecodeOptions, EncodeOptions};
use crate::types::BMP_MAGIC;

/// `true` when `bytes` starts with the `BM` file signature. Allocation-
/// free, never panics, `false` on short input.
pub fn probe(bytes: &[u8]) -> bool {
    bytes.len() >= 2 && u16::from_le_bytes([bytes[0], bytes[1]]) == BMP_MAGIC
}

/// Header-only description: dimensions, the layout [`decode`] would
/// return, colour signalling, ICC presence. No pixel is touched.
pub fn info(bytes: &[u8]) -> Result<ImageInfo> {
    decoder::info_file(bytes)
}

/// Decode a complete BMP file into its native layout with
/// [`DecodeOptions::default`].
pub fn decode(bytes: &[u8]) -> Result<BmpImage> {
    decoder::decode_file(bytes, &DecodeOptions::default())
}

/// Decode with explicit limits / strictness / profile copy
/// ([`DecodeOptions`]).
pub fn decode_with(bytes: &[u8], opts: &DecodeOptions) -> Result<BmpImage> {
    decoder::decode_file(bytes, opts)
}

/// Decode straight to tightly packed 8-bit RGB (alpha dropped).
pub fn decode_rgb8(bytes: &[u8]) -> Result<RgbImage> {
    let img = decode(bytes)?;
    Ok(RgbImage::new(img.width, img.height, img.to_rgb8()))
}

/// Decode straight to tightly packed 8-bit RGBA (opaque where the
/// file has no alpha).
pub fn decode_rgba8(bytes: &[u8]) -> Result<RgbaImage> {
    let img = decode(bytes)?;
    Ok(RgbaImage::new(img.width, img.height, img.to_rgba8()))
}

/// Read `r` to its end and [`decode`] the bytes.
pub fn decode_from<R: Read>(mut r: R) -> Result<BmpImage> {
    let mut bytes = Vec::new();
    r.read_to_end(&mut bytes)?;
    decode(&bytes)
}

/// Encode `image` as a complete BMP file. The on-disk variant follows
/// the image's [`crate::PixelFormat`] (see [`crate::encoder`]); every
/// layout the type can express is writable, so the only
/// [`crate::Error::Unsupported`] cases are mutually exclusive header
/// options.
pub fn encode(image: &BmpImage, opts: &EncodeOptions) -> Result<Vec<u8>> {
    Ok(encoder::encode_image(image, opts)?.0)
}

/// [`encode`] that also reports which on-disk variant was written
/// (the RLE-or-raw choice is made per image).
pub fn encode_with_report(
    image: &BmpImage,
    opts: &EncodeOptions,
) -> Result<(Vec<u8>, EncodedBmpFormat)> {
    encoder::encode_image(image, opts)
}

/// Encode tightly packed 8-bit RGB as a 24-bit `BI_RGB` bitmap.
pub fn encode_rgb8(width: u32, height: u32, rgb: &[u8], opts: &EncodeOptions) -> Result<Vec<u8>> {
    let img = BmpImage::packed(
        width,
        height,
        crate::image::BmpPixelFormat::Rgb24,
        width as usize * 3,
        rgb.to_vec(),
    )?;
    encode(&img, opts)
}

/// Encode tightly packed 8-bit RGBA as a 32-bit `BI_RGB` bitmap; the
/// alpha byte is stored in the fourth byte of every pixel (a V3 header
/// cannot declare it, so readers that ignore the byte see the colours
/// only).
pub fn encode_rgba8(width: u32, height: u32, rgba: &[u8], opts: &EncodeOptions) -> Result<Vec<u8>> {
    let img = BmpImage::packed(
        width,
        height,
        crate::image::BmpPixelFormat::Rgba,
        width as usize * 4,
        rgba.to_vec(),
    )?;
    encode(&img, opts)
}

/// [`encode`] into a writer.
pub fn encode_to<W: Write>(image: &BmpImage, opts: &EncodeOptions, mut w: W) -> Result<()> {
    let bytes = encode(image, opts)?;
    w.write_all(&bytes)?;
    Ok(())
}
