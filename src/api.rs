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

/// Decode with explicit limits / strictness.
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

/// [`encode`] appended to `out`, after whatever it already holds.
///
/// The encode requests [`encoded_size_bound`] bytes of spare capacity
/// once (`Vec::reserve`, which may give more) and writes each pixel row
/// straight into its place: when `out` already has that much spare
/// capacity, the encode does not allocate. On error `out` keeps the
/// length and contents it had. Every error but one is raised before
/// `out` is touched; the exception is a `Pal8` / `Indexed4` image whose
/// uncompressed file is over the 4 GiB the BMP size fields can record
/// and whose RLE stream does not bring it under, which is found after
/// the RLE probe and truncates `out` back.
pub fn encode_into(image: &BmpImage, opts: &EncodeOptions, out: &mut Vec<u8>) -> Result<()> {
    encoder::encode_image_into(image, opts, out).map(|_| ())
}

/// The spare capacity [`encode_into`] requests, and so needs to encode
/// `image` without allocating.
///
/// This is the file's exact size for every layout but one: a `Pal8` /
/// `Indexed4` image written with the plain V3 header (no
/// [`EncodeOptions::bitfields`], [`EncodeOptions::calibrated_rgb`] or V5
/// profile option) and RLE allowed (bottom-up with
/// [`EncodeOptions::rle`] set). There the RLE probe runs, and the bound
/// is the smaller of the uncompressed file and 4 GiB (2^32 bytes), plus
/// `2 × width + 2` bytes, the most the probe writes past its budget
/// before it gives up; the file itself is never larger than the
/// uncompressed one. The V4 calibrated-RGB and V5 profile paths never
/// use RLE and return exact sizes.
///
/// It fails as [`encode`] does for everything decided before encoding.
/// A bound for an RLE candidate whose uncompressed file is over the
/// 4 GiB the BMP size fields can record does not mean the encode will
/// succeed: whether the RLE stream brings the file under is only known
/// once the probe has run during the encode.
pub fn encoded_size_bound(image: &BmpImage, opts: &EncodeOptions) -> Result<usize> {
    encoder::encoded_size_bound(image, opts)
}

/// [`encode_into`] with `profile` as the image's ICC profile, in place
/// of [`crate::Metadata::icc`]: the profile is borrowed and written once,
/// straight into `out`, so a caller that holds it elsewhere need not
/// copy it into the image first. [`EncodeOptions`] decide the header as
/// they do for [`encode_into`]: with the defaults the file is a V5
/// `PROFILE_EMBEDDED` bitmap at the default rendering intent, the bytes
/// the deprecated `encode_bmp_with_icc_profile` writes at
/// `LCS_GM_IMAGES`, while [`EncodeOptions::embed_icc`] `= false` writes
/// no profile and [`EncodeOptions::linked_icc`] links one instead.
/// With [`encoded_size_bound_with_icc_profile`] bytes of spare capacity
/// in `out`, the encode does not allocate. On error `out` keeps the
/// length and contents it had.
pub fn encode_into_with_icc_profile(
    image: &BmpImage,
    opts: &EncodeOptions,
    profile: &[u8],
    out: &mut Vec<u8>,
) -> Result<()> {
    encoder::encode_image_into_with_icc(image, opts, profile, out).map(|_| ())
}

/// The spare capacity [`encode_into_with_icc_profile`] requests, and so
/// needs to encode without allocating: [`encoded_size_bound`] for the
/// image carrying `profile`. With the default options it is the file's
/// exact size.
pub fn encoded_size_bound_with_icc_profile(
    image: &BmpImage,
    opts: &EncodeOptions,
    profile: &[u8],
) -> Result<usize> {
    encoder::encoded_size_bound_with_icc(image, opts, profile)
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
