//! BMP + DIB encode.
//!
//! Output variants, chosen by the image's [`BmpPixelFormat`]:
//!
//! | Input layout | On disk | Compression | Header |
//! | --- | --- | --- | --- |
//! | `Rgba`, `Bgra` | 32-bit BGRA | `BI_RGB` | V3 |
//! | `Rgb24`, `Bgr24` | 24-bit BGR | `BI_RGB` | V3 |
//! | `Rgb555` | 16-bit RGB 5-5-5 | `BI_RGB` | V3 |
//! | `Rgb565` | 16-bit RGB 5-6-5 | `BI_BITFIELDS` (in-header masks) | V4 |
//! | `Pal8` | 8-bit indexed | `BI_RGB` or `BI_RLE8` (smaller wins) | V3 |
//! | `Indexed4` | 4-bit indexed | `BI_RGB` or `BI_RLE4` (smaller wins) | V3 |
//! | `Indexed2` | 2-bit indexed (Windows CE) | `BI_RGB` | V3 |
//! | `Indexed1` | 1-bit indexed | `BI_RGB` | V3 |
//!
//! [`EncodeOptions`] switches the header family: explicit-mask
//! `BI_BITFIELDS` / `BI_ALPHABITFIELDS` (V3 + mask tail), V4
//! `LCS_CALIBRATED_RGB`, V5 `PROFILE_EMBEDDED` (from the image's ICC
//! profile) or V5 `PROFILE_LINKED`. The indexed layouts need a
//! [`Palette`] on the image. Every option is honoured on every path
//! where it is meaningful (top-down disables RLE, which is illegal for a
//! negative `biHeight`).
//!
//! The contract entry points live at the crate root ([`crate::encode`],
//! [`crate::encode_rgb8`], …); this module keeps the headerless-DIB
//! helpers `oxideav-ico` uses, the [`BmpBitfields`] mask presets and
//! the deprecated pre-contract names.

use crate::error::{BmpError as Error, Result};
use crate::image::{BmpImage, BmpPixelFormat, Palette, Plane};
use crate::options::{CalibratedRgb, EncodeOptions};
use crate::types::*;

// The items that used to live in this module keep their `encoder::` path
// for one release.
#[allow(deprecated)]
pub use crate::options::BmpEncodeOptions;
#[cfg(feature = "registry")]
pub use crate::registry::make_encoder;
#[cfg(feature = "registry")]
#[allow(deprecated)]
pub use crate::registry::{encode_bmp_videoframe, encode_dib_videoframe};

/// Which on-disk variant [`crate::encode_with_report`] actually wrote.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum EncodedBmpFormat {
    /// 32-bit BGRA `BI_RGB`.
    Rgb32,
    /// 24-bit BGR `BI_RGB`.
    Rgb24,
    /// 16-bit `BI_RGB` RGB 5-5-5.
    Rgb16Rgb,
    /// 16-bit `BI_BITFIELDS` RGB 5-6-5.
    Rgb16Bitfields,
    /// 8-bit uncompressed indexed `BI_RGB`.
    Indexed8,
    /// 4-bit uncompressed indexed `BI_RGB`.
    Indexed4,
    /// 2-bit uncompressed indexed `BI_RGB` (Windows CE 4-colour).
    Indexed2,
    /// 8-bit RLE-compressed indexed `BI_RLE8`.
    Rle8,
    /// 4-bit RLE-compressed indexed `BI_RLE4`.
    Rle4,
    /// 1-bit uncompressed indexed `BI_RGB` (monochrome).
    Indexed1,
    /// Explicit-mask 16- or 32-bit `BI_BITFIELDS` (no alpha mask;
    /// [`EncodeOptions::bitfields`]).
    Bitfields,
    /// Explicit-mask 16- or 32-bit `BI_ALPHABITFIELDS` (alpha mask set;
    /// [`EncodeOptions::bitfields`]).
    AlphaBitfields,
}

impl EncodedBmpFormat {
    /// The `biCompression` value the variant carries.
    pub fn compression(self) -> u32 {
        match self {
            Self::Rle8 => BI_RLE8,
            Self::Rle4 => BI_RLE4,
            Self::Rgb16Bitfields | Self::Bitfields => BI_BITFIELDS,
            Self::AlphaBitfields => BI_ALPHABITFIELDS,
            _ => BI_RGB,
        }
    }
}

// ---------------------------------------------------------------------------
// Crate-internal dispatcher behind the root vocabulary
// ---------------------------------------------------------------------------

/// Borrowed view of the source plane: the encoder reads every row
/// straight out of it.
#[derive(Clone, Copy)]
struct PlaneRef<'a> {
    stride: usize,
    data: &'a [u8],
}

impl<'a> From<&'a Plane> for PlaneRef<'a> {
    fn from(plane: &'a Plane) -> Self {
        Self {
            stride: plane.stride,
            data: &plane.data,
        }
    }
}

/// What an encode reads from its source, borrowed: the [`BmpImage`]
/// fields the encoder uses.
#[derive(Clone, Copy)]
struct EncodeSource<'a> {
    plane: PlaneRef<'a>,
    format: BmpPixelFormat,
    palette: Option<&'a Palette>,
    /// The ICC profile a V5 `PROFILE_EMBEDDED` header carries when
    /// [`EncodeOptions::embed_icc`] is set.
    icc: Option<&'a [u8]>,
    width: u32,
    height: u32,
}

impl<'a> EncodeSource<'a> {
    fn from_image(image: &'a BmpImage) -> Result<Self> {
        let plane = image
            .planes
            .first()
            .ok_or_else(|| Error::invalid("BMP encoder: empty frame plane"))?;
        Ok(Self {
            plane: plane.into(),
            format: image.format,
            palette: image.palette.as_ref(),
            icc: image.metadata.icc.as_deref(),
            width: image.width,
            height: image.height,
        })
    }
}

/// [`crate::encode_with_report`]: write `image` as a complete BMP file
/// honouring every [`EncodeOptions`] field.
pub(crate) fn encode_image(
    image: &BmpImage,
    options: &EncodeOptions,
) -> Result<(Vec<u8>, EncodedBmpFormat)> {
    encode_source(&EncodeSource::from_image(image)?, options)
}

/// [`crate::encode_into`]: append `image` to `out`. On error `out` has
/// its length and contents on entry (see [`Plan::write_into`]).
pub(crate) fn encode_image_into(
    image: &BmpImage,
    options: &EncodeOptions,
    out: &mut Vec<u8>,
) -> Result<EncodedBmpFormat> {
    Plan::new(&EncodeSource::from_image(image)?, options)?.write_into(out)
}

/// [`crate::encoded_size_bound`]: the capacity [`encode_image_into`]
/// requests. `Sizes::new` has checked that it is addressable.
pub(crate) fn encoded_size_bound(image: &BmpImage, options: &EncodeOptions) -> Result<usize> {
    let plan = Plan::new(&EncodeSource::from_image(image)?, options)?;
    Ok(plan.sizes.bound as usize)
}

/// [`encode_image`] for a borrowed source: one buffer, reserved once for
/// the plan's [`Sizes::bound`] and shrunk to the file when the RLE stream
/// wins.
fn encode_source(
    src: &EncodeSource<'_>,
    options: &EncodeOptions,
) -> Result<(Vec<u8>, EncodedBmpFormat)> {
    let plan = Plan::new(src, options)?;
    let mut out = Vec::new();
    let format = plan.write_into(&mut out)?;
    if matches!(format, EncodedBmpFormat::Rle8 | EncodedBmpFormat::Rle4) {
        out.shrink_to_fit();
    }
    Ok((out, format))
}

/// The uncompressed token a layout maps to (the V4 / V5 paths never
/// use RLE).
fn plain_token(format: BmpPixelFormat) -> EncodedBmpFormat {
    match format {
        BmpPixelFormat::Rgba | BmpPixelFormat::Bgra => EncodedBmpFormat::Rgb32,
        BmpPixelFormat::Rgb24 | BmpPixelFormat::Bgr24 => EncodedBmpFormat::Rgb24,
        BmpPixelFormat::Rgb555 => EncodedBmpFormat::Rgb16Rgb,
        BmpPixelFormat::Rgb565 => EncodedBmpFormat::Rgb16Bitfields,
        BmpPixelFormat::Pal8 => EncodedBmpFormat::Indexed8,
        BmpPixelFormat::Indexed4 => EncodedBmpFormat::Indexed4,
        BmpPixelFormat::Indexed2 => EncodedBmpFormat::Indexed2,
        BmpPixelFormat::Indexed1 => EncodedBmpFormat::Indexed1,
    }
}

/// Wrap a bare plane + palette into a validated [`BmpImage`] (the
/// deprecated plane-level entry points funnel through this).
#[allow(deprecated)]
fn plane_image(
    plane: &Plane,
    format: BmpPixelFormat,
    palette: Option<&crate::image::BmpPalette>,
    width: u32,
    height: u32,
) -> Result<BmpImage> {
    Ok(BmpImage::new(width, height, format, vec![plane.clone()])?
        .with_palette(palette.map(Palette::from)))
}

// ---------------------------------------------------------------------------
// Deprecated pre-contract names
// ---------------------------------------------------------------------------

/// Encode a [`BmpImage`] with the default options.
#[deprecated(note = "use oxideav_bmp::encode / encode_with_report (IMAGE_CRATE_API)")]
pub fn encode_bmp(image: &BmpImage) -> Result<(Vec<u8>, EncodedBmpFormat)> {
    encode_image(image, &EncodeOptions::default())
}

/// Encode a [`BmpImage`] with explicit options.
#[deprecated(note = "use oxideav_bmp::encode_with_report(image, &opts) (IMAGE_CRATE_API)")]
pub fn encode_bmp_with_options(
    image: &BmpImage,
    options: EncodeOptions,
) -> Result<(Vec<u8>, EncodedBmpFormat)> {
    encode_image(image, &options)
}

/// Encode a bare plane with the default options.
#[deprecated(
    note = "build a BmpImage (BmpImage::new / with_palette) and use oxideav_bmp::encode_with_report (IMAGE_CRATE_API)"
)]
#[allow(deprecated)]
pub fn encode_bmp_plane(
    plane: &Plane,
    format: BmpPixelFormat,
    palette: Option<&crate::image::BmpPalette>,
    width: u32,
    height: u32,
) -> Result<(Vec<u8>, EncodedBmpFormat)> {
    let image = plane_image(plane, format, palette, width, height)?;
    encode_image(&image, &EncodeOptions::default())
}

/// Encode a bare plane with explicit options.
#[deprecated(
    note = "build a BmpImage (BmpImage::new / with_palette) and use oxideav_bmp::encode_with_report (IMAGE_CRATE_API)"
)]
#[allow(deprecated)]
pub fn encode_bmp_plane_with_options(
    plane: &Plane,
    format: BmpPixelFormat,
    palette: Option<&crate::image::BmpPalette>,
    width: u32,
    height: u32,
    options: EncodeOptions,
) -> Result<(Vec<u8>, EncodedBmpFormat)> {
    let image = plane_image(plane, format, palette, width, height)?;
    encode_image(&image, &options)
}

/// Explicit-mask `BI_BITFIELDS` / `BI_ALPHABITFIELDS` encode.
#[deprecated(note = "use oxideav_bmp::encode with EncodeOptions::with_bitfields (IMAGE_CRATE_API)")]
pub fn encode_bmp_bitfields(
    image: &BmpImage,
    masks: BmpBitfields,
    options: EncodeOptions,
) -> Result<Vec<u8>> {
    Ok(encode_image(image, &options.with_bitfields(masks))?.0)
}

/// Plane-level explicit-mask encode.
#[deprecated(
    note = "build a BmpImage and use oxideav_bmp::encode with EncodeOptions::with_bitfields (IMAGE_CRATE_API)"
)]
pub fn encode_bmp_plane_bitfields(
    plane: &Plane,
    format: BmpPixelFormat,
    masks: BmpBitfields,
    width: u32,
    height: u32,
    options: EncodeOptions,
) -> Result<Vec<u8>> {
    let image = plane_image(plane, format, None, width, height)?;
    Ok(encode_image(&image, &options.with_bitfields(masks))?.0)
}

/// V5 `PROFILE_EMBEDDED` encode with the given ICC profile bytes.
#[deprecated(
    note = "put the profile in BmpImage::metadata.icc and use oxideav_bmp::encode (EncodeOptions::embed_icc / with_rendering_intent) (IMAGE_CRATE_API)"
)]
pub fn encode_bmp_with_icc_profile(
    image: &BmpImage,
    icc_profile: &[u8],
    rendering_intent: u32,
    options: EncodeOptions,
) -> Result<Vec<u8>> {
    let mut image = image.clone();
    image.metadata.icc = Some(icc_profile.to_vec());
    let options = options
        .with_embed_icc(true)
        .with_linked_icc(None)
        .with_rendering_intent(rendering_intent);
    Ok(encode_image(&image, &options)?.0)
}

/// V5 `PROFILE_LINKED` encode with the given path bytestring.
#[deprecated(
    note = "use oxideav_bmp::encode with EncodeOptions::with_linked_icc / with_rendering_intent (IMAGE_CRATE_API)"
)]
pub fn encode_bmp_with_linked_icc_profile(
    image: &BmpImage,
    linked_path: &[u8],
    rendering_intent: u32,
    options: EncodeOptions,
) -> Result<Vec<u8>> {
    let options = options
        .with_linked_icc(linked_path.to_vec())
        .with_rendering_intent(rendering_intent);
    Ok(encode_image(image, &options)?.0)
}

/// V4 `LCS_CALIBRATED_RGB` encode.
#[deprecated(
    note = "use oxideav_bmp::encode with EncodeOptions::with_calibrated_rgb (IMAGE_CRATE_API)"
)]
pub fn encode_bmp_with_calibrated_rgb(
    image: &BmpImage,
    endpoints: [i32; 9],
    gamma_rgb: [u32; 3],
    options: EncodeOptions,
) -> Result<Vec<u8>> {
    let options =
        options.with_calibrated_rgb(crate::options::CalibratedRgb::new(endpoints, gamma_rgb));
    Ok(encode_image(image, &options)?.0)
}

// ---------------------------------------------------------------------------
// Explicit-mask BI_BITFIELDS / BI_ALPHABITFIELDS encoder (V3 header)
// ---------------------------------------------------------------------------

/// Per-channel bit masks for an explicit-mask `BI_BITFIELDS` /
/// `BI_ALPHABITFIELDS` BMP.
///
/// `r` / `g` / `b` / `a` are the 32-bit DWORD masks selecting each
/// channel's bits inside the packed pixel word. `bpp` is the on-disk
/// bit depth (16 or 32). When `a` is zero the encoder writes a 12-byte
/// (three-mask) `BI_BITFIELDS` tail and decoders treat every pixel as
/// opaque; a non-zero `a` writes a 16-byte (four-mask)
/// `BI_ALPHABITFIELDS` tail so the alpha channel survives the round-trip.
///
/// The masks for each channel must be a single contiguous run of set
/// bits (the BMP `BI_BITFIELDS` mechanism is shift-and-scale, not an
/// arbitrary bit permutation); the masks must not overlap and must fit
/// inside `bpp` bits. [`BmpBitfields::validate`] enforces this.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BmpBitfields {
    /// On-disk bit depth: 16 or 32.
    pub bpp: u16,
    /// Red-channel DWORD mask.
    pub r: u32,
    /// Green-channel DWORD mask.
    pub g: u32,
    /// Blue-channel DWORD mask.
    pub b: u32,
    /// Alpha-channel DWORD mask. Zero ⇒ no alpha (three-mask
    /// `BI_BITFIELDS`); non-zero ⇒ four-mask `BI_ALPHABITFIELDS`.
    pub a: u32,
}

impl BmpBitfields {
    /// 16-bit RGB 5-6-5: R bits 15..11, G bits 10..5, B bits 4..0, no
    /// alpha. The canonical Windows/`BI_BITFIELDS` 16-bpp layout.
    pub const RGB565: Self = Self {
        bpp: 16,
        r: 0xF800,
        g: 0x07E0,
        b: 0x001F,
        a: 0x0000,
    };
    /// 16-bit RGB 5-5-5: R bits 14..10, G bits 9..5, B bits 4..0, bit 15
    /// unused, no alpha.
    pub const RGB555: Self = Self {
        bpp: 16,
        r: 0x7C00,
        g: 0x03E0,
        b: 0x001F,
        a: 0x0000,
    };
    /// 16-bit ARGB 1-5-5-5: A bit 15, R bits 14..10, G bits 9..5, B bits
    /// 4..0. The 16-bpp four-mask `BI_ALPHABITFIELDS` layout.
    pub const ARGB1555: Self = Self {
        bpp: 16,
        r: 0x7C00,
        g: 0x03E0,
        b: 0x001F,
        a: 0x8000,
    };
    /// 32-bit BGRA 8-8-8-8: B bits 7..0, G bits 15..8, R bits 23..16,
    /// A bits 31..24 — the byte-aligned alpha-carrying layout. Every
    /// channel is a full 8-bit run, so the round-trip is bit-exact.
    pub const BGRA8888: Self = Self {
        bpp: 32,
        r: 0x00FF_0000,
        g: 0x0000_FF00,
        b: 0x0000_00FF,
        a: 0xFF00_0000,
    };
    /// 32-bit BGRX 8-8-8-8: same channel layout as [`Self::BGRA8888`]
    /// but the top byte is unused (no alpha mask) — a three-mask
    /// `BI_BITFIELDS` 32-bpp bitmap. Colour is bit-exact; alpha is
    /// dropped (decoders read every pixel as opaque).
    pub const BGRX8888: Self = Self {
        bpp: 32,
        r: 0x00FF_0000,
        g: 0x0000_FF00,
        b: 0x0000_00FF,
        a: 0x0000_0000,
    };

    /// `true` when an alpha mask is present (a four-mask
    /// `BI_ALPHABITFIELDS` tail is required).
    pub fn has_alpha(&self) -> bool {
        self.a != 0
    }

    /// The `biCompression` value this mask set declares: `BI_ALPHABITFIELDS`
    /// when an alpha mask is present, else `BI_BITFIELDS`.
    pub fn compression(&self) -> u32 {
        if self.has_alpha() {
            BI_ALPHABITFIELDS
        } else {
            BI_BITFIELDS
        }
    }

    /// Validate the mask set: `bpp` must be 16 or 32, each non-zero mask
    /// must be a single contiguous bit run, masks must not overlap, and
    /// every mask bit must fit inside `bpp` bits.
    pub fn validate(&self) -> Result<()> {
        if self.bpp != 16 && self.bpp != 32 {
            return Err(Error::invalid(format!(
                "BMP bitfields: bpp must be 16 or 32, got {}",
                self.bpp
            )));
        }
        let limit: u32 = if self.bpp == 32 {
            0xFFFF_FFFF
        } else {
            (1u32 << self.bpp) - 1
        };
        let channels = [("R", self.r), ("G", self.g), ("B", self.b), ("A", self.a)];
        let mut union: u32 = 0;
        for (name, mask) in channels {
            if mask == 0 {
                continue; // a zero mask means "channel absent"
            }
            if mask & !limit != 0 {
                return Err(Error::invalid(format!(
                    "BMP bitfields: {name} mask {mask:#010x} exceeds {} bits",
                    self.bpp
                )));
            }
            // Contiguous run check: a value whose set bits are contiguous
            // satisfies `m & (m + lsb) == 0` where lsb isolates the low set
            // bit — equivalently `(m >> tz)` is one less than a power of two.
            // A full-width mask (`normalised == u32::MAX`, e.g. a 32-bit
            // `0xFFFF_FFFF` channel) is itself a contiguous run, so the
            // `+ 1` must wrap to 0 rather than overflow-panic: use
            // `wrapping_add` so `0xFFFF_FFFF & 0 == 0` correctly accepts it.
            let normalised = mask >> mask.trailing_zeros();
            if normalised & normalised.wrapping_add(1) != 0 {
                return Err(Error::invalid(format!(
                    "BMP bitfields: {name} mask {mask:#010x} is not a contiguous bit run"
                )));
            }
            if union & mask != 0 {
                return Err(Error::invalid(format!(
                    "BMP bitfields: {name} mask {mask:#010x} overlaps another channel"
                )));
            }
            union |= mask;
        }
        if self.r == 0 && self.g == 0 && self.b == 0 {
            return Err(Error::invalid(
                "BMP bitfields: at least one of R/G/B must be non-zero",
            ));
        }
        Ok(())
    }
}

/// Trailing-zero shift and run-length of a single-run channel mask.
/// `(0, 0)` for a zero mask (absent channel).
fn mask_shift_len(mask: u32) -> (u32, u32) {
    if mask == 0 {
        return (0, 0);
    }
    let shift = mask.trailing_zeros();
    let len = 32 - mask.leading_zeros() - shift;
    (shift, len)
}

/// Requantise an 8-bit sample to an `n`-bit channel run.
///
/// * `n == 8` — identity, so byte-aligned masks are bit-exact.
/// * `n < 8`  — drop the low `8 - n` bits (the inverse of the decoder's
///   `expand`, which scales an `n`-bit sample back up to 8).
/// * `n > 8`  — left-justify the 8 bits into the `n`-bit run and
///   replicate the high bits into the freed low bits, so a full-scale
///   input (`0xFF`) maps to a full-scale `n`-bit value rather than
///   leaving the low bits dark. Only reachable for a 32-bpp mask wider
///   than a byte, which `validate` permits.
fn quantise(sample: u8, n: u32) -> u32 {
    let s = sample as u32;
    match n.cmp(&8) {
        core::cmp::Ordering::Equal => s,
        core::cmp::Ordering::Less => s >> (8 - n),
        core::cmp::Ordering::Greater => {
            // Left-justify the 8 sample bits into the n-bit run, then
            // replicate the high bits downward so full-scale stays
            // full-scale. `extra` is the freed low-bit count (1..=24 for a
            // 9..=32-bit run); replicate by `s >> (8 - extra)` but clamp
            // the shift to 0 once `extra >= 8` (the whole sample fits in
            // the replicated region) to avoid a shift underflow.
            let extra = n - 8;
            let mask = if n >= 32 { u32::MAX } else { (1u32 << n) - 1 };
            let repl = if extra >= 8 { s } else { s >> (8 - extra) };
            ((s << extra) | repl) & mask
        }
    }
}

/// Encode a [`BmpImage`] into a headerless DIB suitable for `.ico`
/// sub-images. `double_height_for_ico_mask` tells the encoder to:
///
/// * Write the height field as 2×`height` (ICO convention).
/// * Append a 1-bit AND mask derived from the frame's alpha channel:
///   alpha == 0 ⇒ 1 (transparent), alpha != 0 ⇒ 0 (opaque).
///
/// When `false`, the output is a plain DIB suitable for embedding
/// wherever someone expects a Windows DIB (clipboard, registry blob, …).
///
/// `Rgba` / `Rgb24` / `Bgra` / `Bgr24` input is written as 32-bit BGRA;
/// the 16-bit and indexed layouts keep their depth. The DIB path is
/// always bottom-up and uncompressed (RLE + ICO mask interaction is
/// undefined) and never writes the V4 / V5 colour fields; the indexed
/// layouts need a palette on the image.
pub fn encode_dib(image: &BmpImage, double_height_for_ico_mask: bool) -> Result<Vec<u8>> {
    if image.planes.is_empty() {
        return Err(Error::invalid("BMP encoder: empty frame plane"));
    }
    if image.format.is_indexed() && image.palette.is_none() {
        return Err(Error::invalid(format!(
            "BMP encoder: {:?} requires a palette",
            image.format
        )));
    }
    encode_dib_impl(
        &image.planes[0],
        image.format,
        image.palette.as_ref(),
        image.width,
        image.height,
        double_height_for_ico_mask,
    )
}

/// Plane-level headerless-DIB encode.
#[deprecated(
    note = "build a BmpImage (BmpImage::new / with_palette) and use oxideav_bmp::encode_dib (IMAGE_CRATE_API)"
)]
#[allow(deprecated)]
pub fn encode_dib_plane(
    plane: &Plane,
    format: BmpPixelFormat,
    palette: Option<&crate::image::BmpPalette>,
    width: u32,
    height: u32,
    double_height_for_ico_mask: bool,
) -> Result<Vec<u8>> {
    let image = plane_image(plane, format, palette, width, height)?;
    encode_dib(&image, double_height_for_ico_mask)
}

/// Headerless DIB: `BITMAPINFOHEADER` (V4 for `Rgb565`) + colour table +
/// pixels (+ AND mask).
fn encode_dib_impl(
    plane: &Plane,
    format: BmpPixelFormat,
    palette: Option<&Palette>,
    width: u32,
    height: u32,
    double_height_for_ico_mask: bool,
) -> Result<Vec<u8>> {
    // DIB path is always bottom-up (the AND-mask convention assumes
    // bottom-up XOR pixels; nothing in `oxideav-ico` requests top-down).
    let opts = &EncodeOptions::default();
    match format {
        BmpPixelFormat::Rgba
        | BmpPixelFormat::Rgb24
        | BmpPixelFormat::Bgra
        | BmpPixelFormat::Bgr24 => {
            // Classic 32-bpp BGRA DIB path (used by oxideav-ico).
            let (pixels, _) = pack_rgba(plane, format, width, height, opts)?;
            let w = width;
            let h = height;
            let mut out = Vec::new();
            write_dib_header_v3(
                &mut out,
                w,
                if double_height_for_ico_mask {
                    (h * 2) as i32
                } else {
                    h as i32
                },
                32,
                BI_RGB,
                0,
            );
            out.extend_from_slice(&pixels);
            if double_height_for_ico_mask {
                out.extend_from_slice(&build_and_mask_from_alpha(plane, format, width, height)?);
            }
            Ok(out)
        }
        BmpPixelFormat::Rgb555 => {
            // 16-bit DIB — no ICO mask support for 16-bit (alpha is
            // meaningless in 5-5-5 anyway). A 16-bpp `BI_RGB` DIB is
            // unambiguously RGB 555 so a plain 40-byte header suffices.
            let (pixels, _) = pack_rgb555(plane, width, height, opts)?;
            let mut out = Vec::new();
            let stored_h = if double_height_for_ico_mask {
                (height * 2) as i32
            } else {
                height as i32
            };
            write_dib_header_v3(&mut out, width, stored_h, 16, BI_RGB, 0);
            out.extend_from_slice(&pixels);
            Ok(out)
        }
        BmpPixelFormat::Rgb565 => {
            // 16-bit DIB — no ICO mask support for 16-bit (alpha is
            // meaningless in 5-6-5 anyway).
            let (pixels, _) = pack_rgb565(plane, width, height, opts)?;
            let mut out = Vec::new();
            let stored_h = if double_height_for_ico_mask {
                (height * 2) as i32
            } else {
                height as i32
            };
            write_dib_header_v4_bitfields(&mut out, width, stored_h);
            out.extend_from_slice(&pixels);
            Ok(out)
        }
        BmpPixelFormat::Pal8 => {
            let pal = palette
                .ok_or_else(|| Error::invalid("BMP encoder: Indexed8 requires a palette"))?;
            let (pixels, _) = pack_indexed(plane, 8, width, height, opts)?;
            let stored_h = if double_height_for_ico_mask {
                (height * 2) as i32
            } else {
                height as i32
            };
            let mut out = Vec::new();
            let entries = written_palette_entries(8, pal, opts);
            write_dib_header_v3_indexed(&mut out, width, stored_h, 8, BI_RGB, pal, entries);
            out.extend_from_slice(&pixels);
            Ok(out)
        }
        BmpPixelFormat::Indexed4 => {
            let pal = palette
                .ok_or_else(|| Error::invalid("BMP encoder: Indexed4 requires a palette"))?;
            let (pixels, _) = pack_indexed(plane, 4, width, height, opts)?;
            let stored_h = if double_height_for_ico_mask {
                (height * 2) as i32
            } else {
                height as i32
            };
            let mut out = Vec::new();
            let entries = written_palette_entries(4, pal, opts);
            write_dib_header_v3_indexed(&mut out, width, stored_h, 4, BI_RGB, pal, entries);
            out.extend_from_slice(&pixels);
            Ok(out)
        }
        BmpPixelFormat::Indexed2 => {
            let pal = palette
                .ok_or_else(|| Error::invalid("BMP encoder: Indexed2 requires a palette"))?;
            let (pixels, _) = pack_indexed(plane, 2, width, height, opts)?;
            let stored_h = if double_height_for_ico_mask {
                (height * 2) as i32
            } else {
                height as i32
            };
            let mut out = Vec::new();
            let entries = written_palette_entries(2, pal, opts);
            write_dib_header_v3_indexed(&mut out, width, stored_h, 2, BI_RGB, pal, entries);
            out.extend_from_slice(&pixels);
            Ok(out)
        }
        BmpPixelFormat::Indexed1 => {
            let pal = palette
                .ok_or_else(|| Error::invalid("BMP encoder: Indexed1 requires a palette"))?;
            let (pixels, _) = pack_indexed(plane, 1, width, height, opts)?;
            let stored_h = if double_height_for_ico_mask {
                (height * 2) as i32
            } else {
                height as i32
            };
            let mut out = Vec::new();
            let entries = written_palette_entries(1, pal, opts);
            write_dib_header_v3_indexed(&mut out, width, stored_h, 1, BI_RGB, pal, entries);
            out.extend_from_slice(&pixels);
            Ok(out)
        }
    }
}

// ---------------------------------------------------------------------------
// The file plan: header, pixel rows, trailing blob
// ---------------------------------------------------------------------------

/// Stored signed `biHeight` for a given output height + layout choice.
/// Bottom-up DIBs encode the absolute height; top-down DIBs encode its
/// negation per the BMP spec's signed-height convention.
fn signed_stored_height(h: u32, top_down: bool) -> i32 {
    if top_down {
        -(h as i32)
    } else {
        h as i32
    }
}

/// Number of colour-table entries actually written on disk for the
/// given bit depth, palette, and options.
///
/// With `minimal_palette` set the table is exactly `palette.entries`
/// long (clamped to the `2^bpp` ceiling so a caller can't overflow the
/// index space); otherwise the classic full `2^bpp` table is emitted.
/// A `minimal_palette` table is never shrunk below 1 entry — a
/// zero-entry colour table is meaningless for an indexed bitmap.
fn written_palette_entries(bpp: u16, palette: &Palette, options: &EncodeOptions) -> usize {
    let full = palette_entry_count(bpp);
    if options.minimal_palette {
        palette.entries.len().clamp(1, full)
    } else {
        full
    }
}

/// The largest file the 32-bit `bfSize` field of the
/// `BITMAPFILEHEADER` can record.
const MAX_FILE: u64 = u32::MAX as u64;

/// One encode, decided before the first byte is written: the header
/// (which carries the RLE choice and the trailing profile blob, see
/// [`Head`]), the pixel rows and the byte counts ([`Sizes`]). Every
/// error but one is raised while building the plan; the exception is an
/// RLE-eligible file too large to write uncompressed, which fails only
/// when its RLE stream does not bring it under the limit.
struct Plan<'a> {
    head: Head<'a>,
    rows: Rows<'a>,
    /// Picture width and height as the header records them.
    width: u32,
    height: u32,
    /// What the uncompressed layout reports.
    format: EncodedBmpFormat,
    sizes: Sizes,
}

/// The byte counts of one encode, checked against the file-size limit
/// before anything is written, so no header field can wrap.
///
/// They are `u64` whatever the target's pointer width, so the limit
/// arithmetic (`limit + 1` included) and every decision are the same on
/// 32- and 64-bit targets. A count becomes a `usize` only to reserve or
/// slice a buffer, and only once [`Sizes::new`] has checked that
/// [`Self::bound`], the largest of them, is addressable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Sizes {
    /// Bytes before the pixel array: the file's `bfOffBits`.
    head: u64,
    /// The raw pixel array.
    raw: u64,
    /// The uncompressed file: head, raw array and profile blob.
    file: u64,
    /// Whether the uncompressed file fits the limit.
    file_fits: bool,
    /// For a layout that may be RLE-compressed: the stream must stay
    /// below this many bytes to be used. It is the raw array (the stream
    /// has to beat it) or, when smaller, the most that keeps the file
    /// within the limit.
    rle_budget: Option<u64>,
    /// What [`Plan::write_into`] reserves: the uncompressed file, or
    /// with an RLE probe the most the probe (which can run [`rle_slack`]
    /// bytes past its budget) and the raw fallback need. Every other
    /// count that is used as a `usize` is at most this.
    bound: u64,
    /// The largest file allowed: [`MAX_FILE`], or less in tests.
    limit: u64,
}

impl Sizes {
    /// `rle_slack` is `Some` when an RLE probe runs (V3 indexed only, so
    /// `tail` is 0 there). A file over `limit` is an error, unless an RLE
    /// stream may still bring it under. `addressable` is the largest
    /// buffer the target can hold (`usize::MAX`); a bound over it is an
    /// error, where `Vec::reserve` would panic.
    fn new(
        head: u64,
        raw: u64,
        tail: u64,
        rle_slack: Option<u64>,
        limit: u64,
        addressable: u64,
    ) -> Result<Self> {
        let overflow = || Error::unsupported("BMP encoder: the file size overflows 64 bits");
        let file = head
            .checked_add(raw)
            .and_then(|n| n.checked_add(tail))
            .ok_or_else(overflow)?;
        let file_fits = file <= limit;
        let (rle_budget, bound) = match rle_slack {
            None if !file_fits => return Err(too_large(file, limit)),
            None => (None, file),
            Some(slack) => {
                let budget = raw.min((limit + 1).saturating_sub(head));
                let fallback = if file_fits { raw } else { 0 };
                let bound = budget
                    .checked_add(slack)
                    .map(|probe| probe.max(fallback))
                    .and_then(|n| n.checked_add(head))
                    .ok_or_else(overflow)?;
                (Some(budget), bound)
            }
        };
        if bound > addressable {
            return Err(Error::unsupported(format!(
                "BMP encoder: the {bound}-byte buffer this encode needs is more than the target can address"
            )));
        }
        Ok(Self {
            head,
            raw,
            file,
            file_fits,
            rle_budget,
            bound,
            limit,
        })
    }

    /// The error for an uncompressed file over the limit whose RLE stream
    /// did not bring it under.
    fn rle_too_large(&self) -> Error {
        Error::unsupported(format!(
            "BMP encoder: a {}-byte uncompressed file is over the {} bytes the BMP size fields can record, and its RLE stream does not bring it under",
            self.file, self.limit
        ))
    }

    /// The sizes of `head` over `rows`.
    fn of(head: &Head<'_>, rows: &Rows<'_>, limit: u64) -> Result<Self> {
        let raw = (rows.stride as u64)
            .checked_mul(rows.height as u64)
            .ok_or_else(|| {
                Error::unsupported("BMP encoder: the pixel array size overflows 64 bits")
            })?;
        let (tail, slack) = match head {
            Head::V5Profile { blob, .. } => (blob.len() as u64, None),
            Head::V3Indexed { rle: Some(_), .. } => (0, Some(rle_slack(rows.width))),
            _ => (0, None),
        };
        Self::new(
            u64::from(head.len()),
            raw,
            tail,
            slack,
            limit,
            usize::MAX as u64,
        )
    }
}

/// The error for a file the 32-bit size fields cannot record.
fn too_large(file: u64, limit: u64) -> Error {
    Error::unsupported(format!(
        "BMP encoder: a {file}-byte file is over the {limit} bytes the BMP size fields can record"
    ))
}

/// The header family of the file, with what its writer needs beyond
/// the geometry and the size of the pixel array.
///
/// Two facts live here so the type keeps them apart: only
/// [`Head::V3Indexed`] can carry an RLE stream, and only
/// [`Head::V5Profile`] has a blob after the pixel array. An RLE file
/// therefore ends with its stream, and [`patch_rle_head`] only ever sees
/// the V3 layout.
enum Head<'a> {
    /// 40-byte `BITMAPINFOHEADER`, direct colour, `BI_RGB`. For 16 bpp
    /// the documented `BI_RGB` layout is always RGB 5-5-5 (high bit
    /// reserved), so no mask block is needed.
    V3 { bpp: u16 },
    /// 40-byte `BITMAPINFOHEADER` (V3) followed by the per-channel masks
    /// as a 12-byte (RGB) or 16-byte (RGBA) tail: the classic Windows
    /// in-file mask layout, distinct from the V4 in-header mask block
    /// the `Rgb565` path emits.
    V3Masks(BmpBitfields),
    /// 40-byte `BITMAPINFOHEADER` followed by the colour table. `rle` is
    /// the RLE stream the pixel array may be replaced with.
    V3Indexed {
        bpp: u16,
        palette: &'a Palette,
        entries: usize,
        rle: Option<Rle>,
    },
    /// 108-byte `BITMAPV4HEADER` with the canonical 5-6-5 masks at
    /// offsets 40..56, so no separate mask tail precedes the pixels.
    V4Rgb565,
    /// 108-byte `BITMAPV4HEADER` with `bV4CSType = LCS_CALIBRATED_RGB`,
    /// the CIE endpoints and gamma triple written verbatim, then the
    /// colour table for indexed input:
    /// `[file header 14 B][V4 header 108 B][colour table][pixels]`.
    V4Calibrated(ColourHeader<'a>, CalibratedRgb),
    /// 124-byte `BITMAPV5HEADER` with `bV5CSType = cs_type`
    /// (`PROFILE_EMBEDDED` or `PROFILE_LINKED`), then the colour table
    /// for indexed input, the pixel array, and the profile blob:
    /// `[file header 14 B][V5 header 124 B][colour table][pixels][blob]`.
    /// `bV5ProfileData` is DIB-relative (`124 + table + pixels`) and
    /// `bfOffBits` skips the header and table but never the blob (ICC
    /// bytes or the linked path), which lives after the pixel array.
    V5Profile {
        header: ColourHeader<'a>,
        cs_type: u32,
        intent: u32,
        blob: &'a [u8],
    },
}

impl Head<'_> {
    /// Bytes before the pixel array (file header, DIB header, mask
    /// tail, colour table): the file's `bfOffBits`.
    fn len(&self) -> u32 {
        BITMAPFILEHEADER_SIZE
            + match self {
                Head::V3 { .. } => BITMAPINFOHEADER_SIZE,
                Head::V3Masks(masks) => {
                    BITMAPINFOHEADER_SIZE + if masks.has_alpha() { 16 } else { 12 }
                }
                Head::V3Indexed { entries, .. } => BITMAPINFOHEADER_SIZE + (entries * 4) as u32,
                Head::V4Rgb565 => BITMAPV4HEADER_SIZE,
                Head::V4Calibrated(header, _) => BITMAPV4HEADER_SIZE + header.table_bytes(),
                Head::V5Profile { header, .. } => BITMAPV5HEADER_SIZE + header.table_bytes(),
            }
    }
}

/// What the V4 / V5 colour headers record about the pixel array: its
/// depth, compression and four-mask region, plus the colour table that
/// follows the header for indexed input.
#[derive(Clone, Copy)]
struct ColourHeader<'a> {
    bpp: u16,
    compression: u32,
    masks: [u32; 4],
    table: Option<(&'a Palette, usize)>,
}

impl ColourHeader<'_> {
    fn table_bytes(&self) -> u32 {
        self.table.map_or(0, |(_, entries)| (entries * 4) as u32)
    }

    /// `biClrUsed`: 0 for direct colour and for a full `2^bpp` table
    /// (the spec's "all entries" sentinel), else the table's length.
    fn clr_used(&self) -> u32 {
        match self.table {
            Some((_, entries)) if entries < palette_entry_count(self.bpp) => entries as u32,
            _ => 0,
        }
    }

    fn write_table(&self, out: &mut Vec<u8>) {
        if let Some((palette, entries)) = self.table {
            write_color_table(out, palette, entries);
        }
    }
}

impl<'a> Plan<'a> {
    /// Plan the file for `src`. Every check an encode makes runs here,
    /// in this order: dimensions, palette, the exclusive header options,
    /// then the chosen path's own checks (mask set, source layout, plane
    /// length).
    fn new(src: &EncodeSource<'a>, options: &'a EncodeOptions) -> Result<Self> {
        if src.width == 0 || src.height == 0 {
            return Err(Error::invalid("BMP encoder: zero dimension"));
        }
        if src.format.is_indexed() && src.palette.is_none() {
            return Err(Error::invalid(format!(
                "BMP encoder: {:?} requires a palette",
                src.format
            )));
        }
        let embedded_icc = if options.embed_icc { src.icc } else { None };
        let v5 = options.linked_icc.is_some() || embedded_icc.is_some();
        let requested = usize::from(options.bitfields.is_some())
            + usize::from(options.calibrated_rgb.is_some())
            + usize::from(v5);
        if requested > 1 {
            return Err(Error::unsupported(
                "BMP encoder: bitfields, calibrated_rgb and the V5 profile modes are mutually exclusive",
            ));
        }
        if let Some(masks) = options.bitfields {
            return Self::bitfields(src, masks, options);
        }
        if let Some(cal) = options.calibrated_rgb {
            let (rows, header) = Self::colour_rows(src, options)?;
            let head = Head::V4Calibrated(header, cal);
            return Self::assemble(src, head, rows);
        }
        // A linked profile takes precedence over an embedded one.
        let profile = match (options.linked_icc.as_deref(), embedded_icc) {
            (Some(path), _) => Some((path, PROFILE_LINKED)),
            (None, Some(icc)) => Some((icc, PROFILE_EMBEDDED)),
            (None, None) => None,
        };
        if let Some((blob, cs_type)) = profile {
            let (rows, header) = Self::colour_rows(src, options)?;
            let head = Head::V5Profile {
                header,
                cs_type,
                intent: options.rendering_intent,
                blob,
            };
            return Self::assemble(src, head, rows);
        }
        Self::plain(src, options)
    }

    fn assemble(src: &EncodeSource<'a>, head: Head<'a>, rows: Rows<'a>) -> Result<Self> {
        let sizes = Sizes::of(&head, &rows, MAX_FILE)?;
        Ok(Self {
            head,
            rows,
            width: src.width,
            height: src.height,
            format: plain_token(src.format),
            sizes,
        })
    }

    /// The same plan checked against a lower file-size limit, so the
    /// over-limit paths can be exercised without 4 GiB files.
    #[cfg(test)]
    fn with_limit(mut self, limit: u64) -> Result<Self> {
        self.sizes = Sizes::of(&self.head, &self.rows, limit)?;
        Ok(self)
    }

    /// The blob after the pixel array: the V5 profile, else nothing.
    fn tail(&self) -> &'a [u8] {
        match self.head {
            Head::V5Profile { blob, .. } => blob,
            _ => &[],
        }
    }

    /// Plain-header path: V3 (V4 for `Rgb565`), RLE where allowed.
    ///
    /// `Pal8` / `Indexed4` try `BI_RLE8` / `BI_RLE4` and keep whichever
    /// of the stream and the raw array is smaller. Under `top_down` RLE
    /// is skipped unconditionally: BMP RLE streams describe a bottom-up
    /// scan with `(end-of-line, delta, end-of-bitmap)` escape codes that
    /// have no defined meaning under a negative `biHeight`, so the
    /// output stays uncompressed and spec-compliant.
    fn plain(src: &EncodeSource<'a>, options: &EncodeOptions) -> Result<Self> {
        let format = src.format;
        let (head, pack, truncated) = match format {
            BmpPixelFormat::Rgba | BmpPixelFormat::Bgra => (
                Head::V3 { bpp: 32 },
                RowPack::bgra32(format),
                "BMP encoder: frame plane truncated",
            ),
            BmpPixelFormat::Rgb24 | BmpPixelFormat::Bgr24 => (
                Head::V3 { bpp: 24 },
                RowPack::bgr24(format),
                "BMP encoder: frame plane truncated (rgb24)",
            ),
            BmpPixelFormat::Rgb555 => (
                Head::V3 { bpp: 16 },
                RowPack::Words16,
                "BMP encoder: frame plane truncated (rgb555)",
            ),
            BmpPixelFormat::Rgb565 => (
                Head::V4Rgb565,
                RowPack::Words16,
                "BMP encoder: frame plane truncated (rgb565)",
            ),
            BmpPixelFormat::Pal8
            | BmpPixelFormat::Indexed4
            | BmpPixelFormat::Indexed2
            | BmpPixelFormat::Indexed1 => {
                let bpp = format.bits_per_pixel();
                let palette = src.palette.ok_or_else(|| {
                    Error::invalid(format!("BMP encoder: {format:?} requires a palette"))
                })?;
                let rle = match format {
                    BmpPixelFormat::Pal8 => Some(Rle::Rle8),
                    BmpPixelFormat::Indexed4 => Some(Rle::Rle4),
                    _ => None,
                };
                let head = Head::V3Indexed {
                    bpp,
                    palette,
                    entries: written_palette_entries(bpp, palette, options),
                    rle: rle.filter(|_| options.rle && !options.top_down),
                };
                (
                    head,
                    RowPack::Indexed {
                        bpp: usize::from(bpp),
                    },
                    "BMP encoder: frame plane truncated (indexed)",
                )
            }
        };
        let rows = Rows::new(
            src.plane,
            pack,
            src.width,
            src.height,
            options.top_down,
            truncated,
        )?;
        Self::assemble(src, head, rows)
    }

    /// Explicit-mask `BI_BITFIELDS` / `BI_ALPHABITFIELDS` path (V3
    /// header + mask tail).
    ///
    /// The source plane must be `Rgba` / `Rgb24` / `Bgra` / `Bgr24`; each
    /// 8-bit channel is requantised down to the width of its mask and
    /// shifted into place. For a byte-aligned 32-bpp mask set
    /// ([`BmpBitfields::BGRA8888`] / [`BmpBitfields::BGRX8888`]) every
    /// channel keeps its full 8 bits so the round-trip is bit-exact.
    fn bitfields(
        src: &EncodeSource<'a>,
        masks: BmpBitfields,
        options: &EncodeOptions,
    ) -> Result<Self> {
        masks.validate()?;
        let pack = RowPack::masks(src.format, masks)?;
        let rows = Rows::new(
            src.plane,
            pack,
            src.width,
            src.height,
            options.top_down,
            "BMP encoder: frame plane truncated (bitfields)",
        )?;
        let mut plan = Self::assemble(src, Head::V3Masks(masks), rows)?;
        plan.format = if masks.has_alpha() {
            EncodedBmpFormat::AlphaBitfields
        } else {
            EncodedBmpFormat::Bitfields
        };
        Ok(plan)
    }

    /// Pixel rows and header fields for the V4 / V5 colour headers.
    /// Direct-colour input keeps its plain layout (32 / 24 / 16 bpp; the
    /// 5-6-5 masks, or the alpha mask for 32-bit input, in the four-mask
    /// region); indexed input is written uncompressed with its colour
    /// table between header and pixels. RLE is never used here: the BMP
    /// spec doesn't define how an RLE pixel stream and a trailing
    /// colour-management blob co-exist on disk, and one header shape per
    /// layout keeps the output deterministic.
    fn colour_rows(
        src: &EncodeSource<'a>,
        options: &EncodeOptions,
    ) -> Result<(Rows<'a>, ColourHeader<'a>)> {
        let format = src.format;
        let (pack, bpp, compression, masks, table, truncated) = match format {
            BmpPixelFormat::Rgba | BmpPixelFormat::Bgra => (
                RowPack::bgra32(format),
                32,
                BI_RGB,
                RGBA32_ALPHA_MASK_V4_V5,
                None,
                "BMP encoder: frame plane truncated",
            ),
            BmpPixelFormat::Rgb24 | BmpPixelFormat::Bgr24 => (
                RowPack::bgr24(format),
                24,
                BI_RGB,
                [0u32; 4],
                None,
                "BMP encoder: frame plane truncated (rgb24)",
            ),
            BmpPixelFormat::Rgb555 => (
                RowPack::Words16,
                16,
                BI_RGB,
                [0u32; 4],
                None,
                "BMP encoder: frame plane truncated (rgb555)",
            ),
            BmpPixelFormat::Rgb565 => (
                RowPack::Words16,
                16,
                BI_BITFIELDS,
                RGB565_MASKS_V5,
                None,
                "BMP encoder: frame plane truncated (rgb565)",
            ),
            BmpPixelFormat::Pal8
            | BmpPixelFormat::Indexed4
            | BmpPixelFormat::Indexed2
            | BmpPixelFormat::Indexed1 => {
                let bpp = format.bits_per_pixel();
                let palette = src.palette.ok_or_else(|| {
                    Error::invalid(format!("BMP encoder: {format:?} requires a palette"))
                })?;
                let entries = written_palette_entries(bpp, palette, options);
                (
                    RowPack::Indexed {
                        bpp: usize::from(bpp),
                    },
                    bpp,
                    BI_RGB,
                    [0u32; 4],
                    Some((palette, entries)),
                    "BMP encoder: frame plane truncated (indexed)",
                )
            }
        };
        let rows = Rows::new(
            src.plane,
            pack,
            src.width,
            src.height,
            options.top_down,
            truncated,
        )?;
        let header = ColourHeader {
            bpp,
            compression,
            masks,
            table,
        };
        Ok((rows, header))
    }

    /// Write everything before the pixel array: `file_size` goes into
    /// `bfSize`, `pixel_bytes` into the headers that record the pixel
    /// array's size. Both fit by [`Sizes`].
    fn write_head(&self, out: &mut Vec<u8>, file_size: u32, pixel_bytes: u32) {
        let pixel_offset = self.head.len();
        write_file_header(out, file_size, pixel_offset);
        let w = self.width;
        let h = signed_stored_height(self.height, self.rows.top_down);
        match &self.head {
            Head::V3 { bpp } => write_dib_header_v3(out, w, h, *bpp, BI_RGB, 0),
            Head::V3Masks(masks) => {
                write_dib_header_v3(out, w, h, masks.bpp, masks.compression(), 0);
                // V3 mask tail: 3 DWORDs (R/G/B) for BI_BITFIELDS, 4 (R/G/B/A) for
                // BI_ALPHABITFIELDS, immediately after the 40-byte header.
                out.extend_from_slice(&masks.r.to_le_bytes());
                out.extend_from_slice(&masks.g.to_le_bytes());
                out.extend_from_slice(&masks.b.to_le_bytes());
                if masks.has_alpha() {
                    out.extend_from_slice(&masks.a.to_le_bytes());
                }
            }
            Head::V3Indexed {
                bpp,
                palette,
                entries,
                ..
            } => write_dib_header_v3_indexed(out, w, h, *bpp, BI_RGB, palette, *entries),
            Head::V4Rgb565 => write_dib_header_v4_bitfields(out, w, h),
            Head::V4Calibrated(header, cal) => {
                write_dib_header_v4_calibrated(
                    out,
                    w,
                    h,
                    header.bpp,
                    pixel_bytes,
                    header.compression,
                    header.masks,
                    header.clr_used(),
                    cal.endpoints,
                    cal.gamma,
                );
                header.write_table(out);
            }
            Head::V5Profile {
                header,
                cs_type,
                intent,
                blob,
            } => {
                // DIB-relative: past the V5 header, the colour table and
                // the pixel array.
                let profile_data = pixel_offset - BITMAPFILEHEADER_SIZE + pixel_bytes;
                let profile_size = blob.len() as u32;
                if header.table.is_some() {
                    write_dib_header_v5_indexed_with_profile(
                        out,
                        w,
                        h,
                        header.bpp,
                        pixel_bytes,
                        header.clr_used(),
                        *cs_type,
                        *intent,
                        profile_data,
                        profile_size,
                    );
                } else {
                    write_dib_header_v5_with_profile(
                        out,
                        w,
                        h,
                        header.bpp,
                        pixel_bytes,
                        header.compression,
                        header.masks,
                        *cs_type,
                        *intent,
                        profile_data,
                        profile_size,
                    );
                }
                header.write_table(out);
            }
        }
    }

    /// Append the file to `out`. The buffer's capacity is requested once,
    /// [`Sizes::bound`] bytes, and every row is packed straight into
    /// its final place; the RLE probe writes its stream into the same
    /// buffer, right after the header. The one error raised here (see
    /// [`Plan`]) truncates `out` back to its length on entry.
    fn write_into(&self, out: &mut Vec<u8>) -> Result<EncodedBmpFormat> {
        let s = self.sizes;
        // Every `as usize` below is at most `bound`, which `Sizes::new`
        // has checked the target can address.
        out.reserve(s.bound as usize);
        let start = out.len();
        // Only a V3 indexed header carries an RLE choice, and it has no
        // blob after the pixels, so an RLE file ends with its stream.
        if let (Head::V3Indexed { rle: Some(rle), .. }, Some(budget)) = (&self.head, s.rle_budget) {
            // `bfSize` is patched once the probe has decided; 0 holds its
            // place when only the RLE stream can fit.
            let provisional = if s.file_fits { s.file as u32 } else { 0 };
            self.write_head(out, provisional, 0);
            if self.rows.rle_into(*rle, budget as usize, out) {
                let size = out.len() - start;
                patch_rle_head(&mut out[start..], size, *rle);
                return Ok(rle.format());
            }
            if !s.file_fits {
                out.truncate(start);
                return Err(s.rle_too_large());
            }
        } else {
            self.write_head(out, s.file as u32, s.raw as u32);
        }
        let pixels = out.len();
        out.resize(pixels + s.raw as usize, 0);
        self.rows.pack_all(&mut out[pixels..]);
        out.extend_from_slice(self.tail());
        Ok(self.format)
    }
}

// ---------------------------------------------------------------------------
// Pixel-packing helpers
// ---------------------------------------------------------------------------

/// The source rows of one encode and how each becomes an on-disk row.
#[derive(Clone, Copy)]
struct Rows<'a> {
    plane: PlaneRef<'a>,
    pack: RowPack,
    width: usize,
    height: usize,
    top_down: bool,
    /// On-disk row pitch: the packed row rounded up to a DWORD.
    stride: usize,
}

impl<'a> Rows<'a> {
    /// `truncated` is the error for a plane holding fewer than `height`
    /// rows of its stride.
    fn new(
        plane: PlaneRef<'a>,
        pack: RowPack,
        width: u32,
        height: u32,
        top_down: bool,
        truncated: &str,
    ) -> Result<Self> {
        let w = width as usize;
        let h = height as usize;
        if plane.data.len() < plane.stride * h {
            return Err(Error::invalid(truncated));
        }
        Ok(Self {
            plane,
            pack,
            width: w,
            height: h,
            top_down,
            stride: row_stride(w, pack.bits_per_pixel()),
        })
    }

    /// Size of the raw pixel array.
    fn len(&self) -> usize {
        self.stride * self.height
    }

    /// The source row stored as on-disk row `y`. Bottom-up DIBs store the
    /// bottom of the picture first, so on-disk row 0 holds the last
    /// source row; top-down DIBs keep the source order.
    fn source(&self, y: usize) -> &'a [u8] {
        let src_y = if self.top_down {
            y
        } else {
            self.height - 1 - y
        };
        let start = src_y * self.plane.stride;
        &self.plane.data[start..start + self.width * self.pack.source_bytes()]
    }

    /// Pack on-disk row `y` into `dst`, one zeroed on-disk row: the
    /// padding is left zero and the sub-byte layouts OR their bits in.
    fn pack(&self, y: usize, dst: &mut [u8]) {
        self.pack.row(self.source(y), dst);
    }

    /// Pack every row into `dst`, a zeroed raw pixel array. A zero width
    /// (possible through the public fields; the headerless DIB writes it)
    /// makes every row empty, and each source row is still sliced.
    fn pack_all(&self, dst: &mut [u8]) {
        for y in 0..self.height {
            let at = y * self.stride;
            self.pack(y, &mut dst[at..at + self.stride]);
        }
    }
}

/// How a source row becomes an on-disk row.
#[derive(Clone, Copy)]
enum RowPack {
    /// 32-bit BGRA from `Rgba` / `Rgb24` / `Bgra` / `Bgr24`: `in_bpp` 4
    /// or 3, `bgr` when the source is already blue-first; 3-byte input
    /// gets an opaque alpha byte.
    Bgra32 { in_bpp: usize, bgr: bool },
    /// 24-bit BGR from the same four layouts.
    Bgr24 { in_bpp: usize, bgr: bool },
    /// Little-endian 16-bit words (5-5-5 / 5-6-5), copied verbatim: the
    /// input already carries the on-disk word layout.
    Words16,
    /// One index byte per pixel packed to `bpp` (1, 2, 4 or 8) bits, the
    /// leftmost pixel in the most-significant bits.
    Indexed { bpp: usize },
    /// Explicit masks: each 8-bit channel requantised into its mask.
    Masks {
        in_bpp: usize,
        bgr: bool,
        layout: MaskLayout,
    },
}

/// `true` for the layouts already stored blue-first (`Bgra`, `Bgr24`).
fn blue_first(format: BmpPixelFormat) -> bool {
    matches!(format, BmpPixelFormat::Bgra | BmpPixelFormat::Bgr24)
}

impl RowPack {
    fn bgra32(format: BmpPixelFormat) -> Self {
        Self::Bgra32 {
            in_bpp: format.bytes_per_pixel(),
            bgr: blue_first(format),
        }
    }

    fn bgr24(format: BmpPixelFormat) -> Self {
        Self::Bgr24 {
            in_bpp: format.bytes_per_pixel(),
            bgr: blue_first(format),
        }
    }

    fn masks(format: BmpPixelFormat, masks: BmpBitfields) -> Result<Self> {
        match format {
            BmpPixelFormat::Rgba
            | BmpPixelFormat::Rgb24
            | BmpPixelFormat::Bgra
            | BmpPixelFormat::Bgr24 => Ok(Self::Masks {
                in_bpp: format.bytes_per_pixel(),
                bgr: blue_first(format),
                layout: MaskLayout::new(masks),
            }),
            other => Err(Error::unsupported(format!(
                "BMP bitfields: source must be Rgba / Rgb24 / Bgra / Bgr24, got {other:?}"
            ))),
        }
    }

    /// Bits per pixel on disk.
    fn bits_per_pixel(&self) -> usize {
        match *self {
            Self::Bgra32 { .. } => 32,
            Self::Bgr24 { .. } => 24,
            Self::Words16 => 16,
            Self::Indexed { bpp } => bpp,
            Self::Masks { layout, .. } => usize::from(layout.bpp),
        }
    }

    /// Bytes per pixel of the source plane.
    fn source_bytes(&self) -> usize {
        match *self {
            Self::Bgra32 { in_bpp, .. }
            | Self::Bgr24 { in_bpp, .. }
            | Self::Masks { in_bpp, .. } => in_bpp,
            Self::Words16 => 2,
            Self::Indexed { .. } => 1,
        }
    }

    /// Pack one source row into `dst`, a zeroed on-disk row.
    fn row(&self, src: &[u8], dst: &mut [u8]) {
        match *self {
            // The format branch is hoisted out of the per-pixel loop and
            // both sides iterate `chunks_exact`, so the compiler drops the
            // per-byte bounds checks (and can vectorise the fixed 3/4-byte
            // shuffle).
            Self::Bgra32 { in_bpp, bgr } => match (in_bpp, bgr) {
                (4, false) => {
                    for (d, s) in dst.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
                        d[0] = s[2];
                        d[1] = s[1];
                        d[2] = s[0];
                        d[3] = s[3];
                    }
                }
                (4, true) => dst[..src.len()].copy_from_slice(src),
                (_, false) => {
                    for (d, s) in dst.chunks_exact_mut(4).zip(src.chunks_exact(3)) {
                        d[0] = s[2];
                        d[1] = s[1];
                        d[2] = s[0];
                        d[3] = 0xFF;
                    }
                }
                (_, true) => {
                    for (d, s) in dst.chunks_exact_mut(4).zip(src.chunks_exact(3)) {
                        d[..3].copy_from_slice(s);
                        d[3] = 0xFF;
                    }
                }
            },
            // The source step is `in_bpp` (3 or 4), the dest step a packed 3.
            Self::Bgr24 { in_bpp, bgr } => {
                let dst = &mut dst[..src.len() / in_bpp * 3];
                if bgr {
                    for (d, s) in dst.chunks_exact_mut(3).zip(src.chunks_exact(in_bpp)) {
                        d.copy_from_slice(&s[..3]);
                    }
                } else {
                    for (d, s) in dst.chunks_exact_mut(3).zip(src.chunks_exact(in_bpp)) {
                        d[0] = s[2];
                        d[1] = s[1];
                        d[2] = s[0];
                    }
                }
            }
            Self::Words16 => dst[..src.len()].copy_from_slice(src),
            Self::Indexed { bpp } => pack_indices(src, bpp, dst),
            Self::Masks {
                in_bpp,
                bgr,
                layout,
            } => layout.pack(src, in_bpp, bgr, dst),
        }
    }
}

/// Pack one row of index bytes to `bpp` bits per pixel.
///
/// For 8-bit: input is 1 byte per pixel (index 0-255).
/// For 4-bit: input is 1 byte per pixel (index 0-15); packing into
///   hi-nibble/lo-nibble is done here.
/// For 2-bit: input is 1 byte per pixel (index 0-3, treated as
///   `byte & 3`); four pixels pack per byte, the left-most pixel in the
///   two most-significant bits.
/// For 1-bit: input is 1 byte per pixel (index 0 or 1, treated as
///   `byte & 1`); packing into MSB-first bytes is done here.
fn pack_indices(src: &[u8], bpp: usize, dst: &mut [u8]) {
    let w = src.len();
    match bpp {
        8 => {
            dst[..w].copy_from_slice(src);
        }
        4 => {
            for x in 0..w {
                let idx = src[x] & 0x0F;
                if x & 1 == 0 {
                    dst[x / 2] = idx << 4;
                } else {
                    dst[x / 2] |= idx;
                }
            }
        }
        2 => {
            // Four 2-bit indices per byte; the left-most pixel sits in
            // the two most-significant bits (Windows CE indexed layout,
            // the encode counterpart of the decoder's 2-bpp unpack).
            for x in 0..w {
                let idx = src[x] & 0x03;
                let shift = 6 - 2 * (x % 4);
                dst[x / 4] |= idx << shift;
            }
        }
        // 1 bpp, the only depth left.
        _ => {
            for x in 0..w {
                let bit = src[x] & 1;
                if bit != 0 {
                    dst[x / 8] |= 1 << (7 - (x % 8));
                }
            }
        }
    }
}

/// A [`BmpBitfields`] set resolved once per encode to the trailing-zero
/// shift and run length of each channel mask.
#[derive(Clone, Copy)]
struct MaskLayout {
    bpp: u16,
    r: (u32, u32),
    g: (u32, u32),
    b: (u32, u32),
    a: (u32, u32),
}

impl MaskLayout {
    fn new(masks: BmpBitfields) -> Self {
        Self {
            bpp: masks.bpp,
            r: mask_shift_len(masks.r),
            g: mask_shift_len(masks.g),
            b: mask_shift_len(masks.b),
            a: mask_shift_len(masks.a),
        }
    }

    /// Quantise one `Rgba` / `Rgb24` / `Bgra` / `Bgr24` row into the
    /// masked word layout.
    fn pack(&self, src: &[u8], in_bpp: usize, bgr: bool, dst: &mut [u8]) {
        let (ri, bi) = if bgr { (2, 0) } else { (0, 2) };
        let out_bpp_bytes = (self.bpp / 8) as usize;
        let (rs, rn) = self.r;
        let (gs, gn) = self.g;
        let (bs, bn) = self.b;
        let (as_, an) = self.a;
        for x in 0..src.len() / in_bpp {
            let r = src[x * in_bpp + ri];
            let g = src[x * in_bpp + 1];
            let b = src[x * in_bpp + bi];
            let a = if in_bpp == 4 {
                src[x * in_bpp + 3]
            } else {
                0xFF
            };
            let mut word: u32 = 0;
            if rn > 0 {
                word |= quantise(r, rn) << rs;
            }
            if gn > 0 {
                word |= quantise(g, gn) << gs;
            }
            if bn > 0 {
                word |= quantise(b, bn) << bs;
            }
            if an > 0 {
                word |= quantise(a, an) << as_;
            }
            let bytes = word.to_le_bytes();
            dst[x * out_bpp_bytes..x * out_bpp_bytes + out_bpp_bytes]
                .copy_from_slice(&bytes[..out_bpp_bytes]);
        }
    }
}

/// The whole pixel array of `plane` in a buffer of its own, with its
/// row stride. Only the headerless DIB path ([`encode_dib_impl`]) uses
/// this; it assembles its output from parts.
fn pack_plane(
    plane: &Plane,
    pack: RowPack,
    width: u32,
    height: u32,
    options: &EncodeOptions,
    truncated: &str,
) -> Result<(Vec<u8>, usize)> {
    let rows = Rows::new(
        plane.into(),
        pack,
        width,
        height,
        options.top_down,
        truncated,
    )?;
    let mut out = vec![0u8; rows.len()];
    rows.pack_all(&mut out);
    Ok((out, rows.stride))
}

/// Pack RGBA or RGB24 input to 32-bit BGRA rows.
fn pack_rgba(
    plane: &Plane,
    format: BmpPixelFormat,
    width: u32,
    height: u32,
    options: &EncodeOptions,
) -> Result<(Vec<u8>, usize)> {
    pack_plane(
        plane,
        RowPack::bgra32(format),
        width,
        height,
        options,
        "BMP encoder: frame plane truncated",
    )
}

/// Pack 16-bit RGB 5-5-5 input to row-padded output rows.
fn pack_rgb555(
    plane: &Plane,
    width: u32,
    height: u32,
    options: &EncodeOptions,
) -> Result<(Vec<u8>, usize)> {
    pack_plane(
        plane,
        RowPack::Words16,
        width,
        height,
        options,
        "BMP encoder: frame plane truncated (rgb555)",
    )
}

/// Pack 16-bit RGB 5-6-5 input to row-padded output rows.
fn pack_rgb565(
    plane: &Plane,
    width: u32,
    height: u32,
    options: &EncodeOptions,
) -> Result<(Vec<u8>, usize)> {
    pack_plane(
        plane,
        RowPack::Words16,
        width,
        height,
        options,
        "BMP encoder: frame plane truncated (rgb565)",
    )
}

/// Pack indexed pixel data (`bpp` 1, 2, 4 or 8) with row padding.
fn pack_indexed(
    plane: &Plane,
    bpp: usize,
    width: u32,
    height: u32,
    options: &EncodeOptions,
) -> Result<(Vec<u8>, usize)> {
    pack_plane(
        plane,
        RowPack::Indexed { bpp },
        width,
        height,
        options,
        "BMP encoder: frame plane truncated (indexed)",
    )
}

// ---------------------------------------------------------------------------
// RLE encoders
// ---------------------------------------------------------------------------

/// The RLE stream an indexed layout may use instead of its raw array.
#[derive(Clone, Copy)]
enum Rle {
    /// `BI_RLE8`, for `Pal8`.
    Rle8,
    /// `BI_RLE4`, for `Indexed4`.
    Rle4,
}

impl Rle {
    fn compression(self) -> u32 {
        match self {
            Self::Rle8 => BI_RLE8,
            Self::Rle4 => BI_RLE4,
        }
    }

    fn format(self) -> EncodedBmpFormat {
        match self {
            Self::Rle8 => EncodedBmpFormat::Rle8,
            Self::Rle4 => EncodedBmpFormat::Rle4,
        }
    }
}

/// How far past the raw array the RLE probe's stream can reach before
/// the probe gives up. It checks the budget at each row boundary, so the
/// stream can pass it by one row: at most two bytes per pixel (a run of
/// one is two bytes; a longer run or an absolute stretch is less per
/// pixel) plus the two-byte end-of-line or end-of-bitmap marker.
fn rle_slack(width: usize) -> u64 {
    2 * width as u64 + 2
}

/// Rewrite the two fields an RLE stream changes in the V3 indexed
/// header at the start of `file`: `bfSize` (bytes 2..6 of the
/// `BITMAPFILEHEADER`) and `biCompression` (bytes 16..20 of the
/// `BITMAPINFOHEADER` that follows it).
fn patch_rle_head(file: &mut [u8], file_size: usize, rle: Rle) {
    let dib = BITMAPFILEHEADER_SIZE as usize;
    debug_assert_eq!(
        file[dib..dib + 4],
        BITMAPINFOHEADER_SIZE.to_le_bytes(),
        "an RLE stream needs the V3 header"
    );
    file[2..6].copy_from_slice(&(file_size as u32).to_le_bytes());
    file[dib + 16..dib + 20].copy_from_slice(&rle.compression().to_le_bytes());
}

impl Rows<'_> {
    /// Append the RLE encoding of on-disk row `y` to `out`, then the
    /// end-of-line marker, or the end-of-bitmap marker after the last
    /// row. The input is what the raw array would hold: the index bytes
    /// (`BI_RLE8`) or their low nibbles (`BI_RLE4`), bottom-up.
    fn rle_row(&self, rle: Rle, y: usize, out: &mut Vec<u8>) {
        let src = self.source(y);
        match rle {
            Rle::Rle8 => encode_rle8_row(out, src),
            Rle::Rle4 => encode_rle4_row(out, src),
        }
        if y + 1 < self.height {
            // End of line
            out.extend_from_slice(&[0x00, 0x00]);
        } else {
            // End of bitmap
            out.extend_from_slice(&[0x00, 0x01]);
        }
    }

    /// The RLE probe, written straight into `out`. Keeps the stream and
    /// returns `true` only when it is strictly smaller than `budget` (the
    /// raw pixel array); otherwise truncates `out` back and returns
    /// `false`. An incompressible image, where RLE only grows the data,
    /// gives up one row past the budget instead of encoding the whole
    /// plane to throw the result away.
    fn rle_into(&self, rle: Rle, budget: usize, out: &mut Vec<u8>) -> bool {
        let start = out.len();
        for y in 0..self.height {
            // Already no smaller than the raw array: RLE has lost, stop.
            if out.len() - start >= budget {
                out.truncate(start);
                return false;
            }
            self.rle_row(rle, y, out);
        }
        if out.len() - start < budget {
            true
        } else {
            out.truncate(start);
            false
        }
    }
}

fn encode_rle8_row(out: &mut Vec<u8>, row: &[u8]) {
    let mut i = 0;
    let n = row.len();
    while i < n {
        // Count run of same byte.
        let val = row[i];
        let mut run = 1usize;
        while i + run < n && run < 255 && row[i + run] == val {
            run += 1;
        }
        if run >= 2 {
            // Encoded run: count, value
            out.push(run as u8);
            out.push(val);
            i += run;
        } else {
            // Absolute mode: find how many non-repeating bytes ahead.
            let start = i;
            let mut abs_len = 1usize;
            while i + abs_len < n && abs_len < 255 {
                // Peek ahead: if next 2+ are a run, break.
                let j = i + abs_len;
                if j + 1 < n && row[j] == row[j + 1] {
                    break;
                }
                abs_len += 1;
            }
            // Absolute mode needs >= 3 bytes to be worthwhile (overhead = 2
            // bytes escape + count + pad). For < 3 just emit single encoded
            // runs of 1.
            if abs_len < 3 {
                out.push(1);
                out.push(val);
                i += 1;
            } else {
                out.push(0x00);
                out.push(abs_len as u8);
                out.extend_from_slice(&row[start..start + abs_len]);
                // Absolute mode payload padded to even length.
                if abs_len & 1 != 0 {
                    out.push(0x00);
                }
                i += abs_len;
            }
        }
    }
}

/// `row` holds one index byte per pixel; only its low nibble is coded,
/// read in place (no unpacked copy of the row).
fn encode_rle4_row(out: &mut Vec<u8>, row: &[u8]) {
    let nibble = |k: usize| row[k] & 0x0F;
    let mut i = 0;
    let n = row.len();
    while i < n {
        let v0 = nibble(i);
        // Count run of pairs (RLE4 encodes pairs of nibbles).
        // A run of the same nibble value.
        let mut run = 1usize;
        while i + run < n && run < 255 && nibble(i + run) == v0 {
            run += 1;
        }
        if run >= 2 {
            // Encoded run: count byte, then two nibbles packed (both same).
            out.push(run as u8);
            out.push((v0 << 4) | v0);
            i += run;
        } else {
            // Absolute mode: collect non-repeating nibbles.
            let start = i;
            let mut abs_len = 1usize;
            while i + abs_len < n && abs_len < 255 {
                let j = i + abs_len;
                if j + 1 < n && nibble(j) == nibble(j + 1) {
                    break;
                }
                abs_len += 1;
            }
            if abs_len < 3 {
                out.push(1);
                let v1 = if i + 1 < n { nibble(i + 1) } else { 0 };
                out.push((v0 << 4) | v1);
                i += 1;
            } else {
                out.push(0x00);
                out.push(abs_len as u8);
                // Pack nibbles into bytes.
                for k in (0..abs_len).step_by(2) {
                    let hi = nibble(start + k);
                    let lo = if k + 1 < abs_len {
                        nibble(start + k + 1)
                    } else {
                        0
                    };
                    out.push((hi << 4) | lo);
                }
                // Absolute mode payload in RLE4 is padded to even number of
                // bytes (i.e. to a 4-nibble boundary → even byte count).
                let packed_bytes = abs_len.div_ceil(2);
                if packed_bytes & 1 != 0 {
                    out.push(0x00);
                }
                i += abs_len;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Header writers
// ---------------------------------------------------------------------------

fn write_file_header(out: &mut Vec<u8>, file_size: u32, pixel_offset: u32) {
    out.extend_from_slice(&BMP_MAGIC.to_le_bytes());
    out.extend_from_slice(&file_size.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes()); // reserved1
    out.extend_from_slice(&0u16.to_le_bytes()); // reserved2
    out.extend_from_slice(&pixel_offset.to_le_bytes());
}

/// Write a 40-byte BITMAPINFOHEADER (V3) for direct-colour or indexed
/// (non-bitfields) BMPs. Palette entries follow immediately for indexed.
///
/// `stored_height` is the signed BMP `biHeight` — positive for bottom-up
/// layouts, negative for top-down. `image_size` 0 is valid for
/// uncompressed `BI_RGB`; pass the actual compressed byte count for RLE.
fn write_dib_header_v3(
    out: &mut Vec<u8>,
    w: u32,
    stored_height: i32,
    bpp: u16,
    compression: u32,
    image_size: u32,
) {
    out.extend_from_slice(&BITMAPINFOHEADER_SIZE.to_le_bytes());
    out.extend_from_slice(&(w as i32).to_le_bytes());
    out.extend_from_slice(&stored_height.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // planes
    out.extend_from_slice(&bpp.to_le_bytes());
    out.extend_from_slice(&compression.to_le_bytes());
    out.extend_from_slice(&image_size.to_le_bytes());
    out.extend_from_slice(&2835i32.to_le_bytes()); // x_pels/m ≈ 72 DPI
    out.extend_from_slice(&2835i32.to_le_bytes()); // y_pels/m
    out.extend_from_slice(&0u32.to_le_bytes()); // clr_used
    out.extend_from_slice(&0u32.to_le_bytes()); // clr_important
}

/// Write a 40-byte V3 header followed immediately by the colour table.
///
/// `entries` is the number of `RGBQUAD` colour-table entries to write
/// (computed by [`written_palette_entries`]). When it equals the full
/// `2^bpp` count the header leaves `biClrUsed = 0` — the classic
/// "full table" sentinel that prior output used. When it is smaller
/// (minimal-palette mode) the exact count is recorded in `biClrUsed`
/// so the decoder's `palette_entries()` reads back only that many
/// entries. Either way the colour table written matches the count the
/// header advertises, so the pixel offset stays correct.
/// `stored_height` is the signed BMP `biHeight`.
fn write_dib_header_v3_indexed(
    out: &mut Vec<u8>,
    w: u32,
    stored_height: i32,
    bpp: u16,
    compression: u32,
    palette: &Palette,
    entries: usize,
) {
    let image_size: u32 = 0; // valid for BI_RGB; RLE size is embedded in the pixel data
    let full = palette_entry_count(bpp);
    // A full table advertises clr_used = 0 (the spec's "all 2^bpp"
    // sentinel); a shorter table advertises its exact length.
    let clr_used = if entries >= full { 0 } else { entries as u32 };
    out.extend_from_slice(&BITMAPINFOHEADER_SIZE.to_le_bytes());
    out.extend_from_slice(&(w as i32).to_le_bytes());
    out.extend_from_slice(&stored_height.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // planes
    out.extend_from_slice(&bpp.to_le_bytes());
    out.extend_from_slice(&compression.to_le_bytes());
    out.extend_from_slice(&image_size.to_le_bytes());
    out.extend_from_slice(&2835i32.to_le_bytes());
    out.extend_from_slice(&2835i32.to_le_bytes());
    out.extend_from_slice(&clr_used.to_le_bytes()); // 0 → full 2^bpp; else exact count
    out.extend_from_slice(&0u32.to_le_bytes()); // clr_important
    write_color_table(out, palette, entries);
}

/// Write `entries` `RGBQUAD` colour-table entries: on-disk order is B,
/// G, R, 0x00, and entries past the end of the palette are zero.
fn write_color_table(out: &mut Vec<u8>, palette: &Palette, entries: usize) {
    for i in 0..entries {
        if let Some(rgb) = palette.entries.get(i) {
            out.push(rgb[2]); // B
            out.push(rgb[1]); // G
            out.push(rgb[0]); // R
            out.push(0x00);
        } else {
            out.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]);
        }
    }
}

/// Write a 108-byte BITMAPV4HEADER for 16-bit BI_BITFIELDS RGB 5-6-5.
///
/// Canonical masks: R=0xF800, G=0x07E0, B=0x001F, A=0x0000.
/// CS type set to LCS_sRGB (0x73524742) with all endpoints + gamma zero.
/// `stored_height` is signed: negative for a top-down DIB.
fn write_dib_header_v4_bitfields(out: &mut Vec<u8>, w: u32, stored_height: i32) {
    let pixel_bytes = row_stride(w as usize, 16) as u32 * stored_height.unsigned_abs();
    // Header size
    out.extend_from_slice(&BITMAPV4HEADER_SIZE.to_le_bytes());
    out.extend_from_slice(&(w as i32).to_le_bytes());
    out.extend_from_slice(&stored_height.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // planes
    out.extend_from_slice(&16u16.to_le_bytes()); // bpp
    out.extend_from_slice(&BI_BITFIELDS.to_le_bytes()); // compression
    out.extend_from_slice(&pixel_bytes.to_le_bytes()); // image size
    out.extend_from_slice(&2835i32.to_le_bytes()); // x_pels/m
    out.extend_from_slice(&2835i32.to_le_bytes()); // y_pels/m
    out.extend_from_slice(&0u32.to_le_bytes()); // clr_used
    out.extend_from_slice(&0u32.to_le_bytes()); // clr_important
                                                // V4 extension: R/G/B/A masks (offsets 40-55)
    out.extend_from_slice(&0xF800u32.to_le_bytes()); // R mask
    out.extend_from_slice(&0x07E0u32.to_le_bytes()); // G mask
    out.extend_from_slice(&0x001Fu32.to_le_bytes()); // B mask
    out.extend_from_slice(&0x0000u32.to_le_bytes()); // A mask (no alpha)
                                                     // CS type: LCS_sRGB = 0x73524742
    out.extend_from_slice(&0x7352_4742u32.to_le_bytes());
    // CIEXYZTRIPLE (9 × i32 = 36 bytes) — all zero for sRGB
    out.extend_from_slice(&[0u8; 36]);
    // GammaRed, GammaGreen, GammaBlue (3 × u32 = 12 bytes) — zero
    out.extend_from_slice(&[0u8; 12]);
    // Total so far: 40 + 4*4 + 4 + 36 + 12 = 40+16+4+36+12 = 108 ✓
}

/// Write a 108-byte `BITMAPV4HEADER` with
/// `bV4CSType = LCS_CALIBRATED_RGB` and caller-supplied CIE endpoints +
/// per-channel gamma. Used by the `LCS_CALIBRATED_RGB` path for both
/// direct-colour and indexed input.
///
/// Layout (offsets relative to the start of the DIB header):
/// * 0..40   classic `BITMAPINFOHEADER` fields (`biClrUsed` = `clr_used`)
/// * 40..56  R / G / B / A masks (`masks`; zeroed for `BI_RGB`,
///   canonical 5-6-5 for the 16-bpp `BI_BITFIELDS` arm)
/// * 56..60  `bV4CSType` = [`LCS_CALIBRATED_RGB`]
/// * 60..96  `bV4Endpoints` CIEXYZTRIPLE (9 × `i32`, packed
///   R.x R.y R.z G.x G.y G.z B.x B.y B.z)
/// * 96..108 `bV4GammaRed` / `bV4GammaGreen` / `bV4GammaBlue`
///   (3 × `u32`, unsigned 16.16 fixed point)
///
/// `image_size` is the byte length of the pixel array (written to the
/// `biSizeImage` slot at offset 20); `compression` and `masks`
/// parameterise the V3-prefix `biCompression` field and the four-mask
/// region the same way [`write_dib_header_v5_with_profile`] does.
/// `stored_height` is signed: negative for a top-down DIB.
#[allow(clippy::too_many_arguments)]
fn write_dib_header_v4_calibrated(
    out: &mut Vec<u8>,
    w: u32,
    stored_height: i32,
    bpp: u16,
    image_size: u32,
    compression: u32,
    masks: [u32; 4],
    clr_used: u32,
    endpoints: [i32; 9],
    gamma_rgb: [u32; 3],
) {
    // Classic V3 prefix (40 B).
    out.extend_from_slice(&BITMAPV4HEADER_SIZE.to_le_bytes());
    out.extend_from_slice(&(w as i32).to_le_bytes());
    out.extend_from_slice(&stored_height.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // planes
    out.extend_from_slice(&bpp.to_le_bytes());
    out.extend_from_slice(&compression.to_le_bytes());
    out.extend_from_slice(&image_size.to_le_bytes());
    out.extend_from_slice(&2835i32.to_le_bytes()); // x_pels/m ≈ 72 DPI
    out.extend_from_slice(&2835i32.to_le_bytes()); // y_pels/m
    out.extend_from_slice(&clr_used.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // clr_important
                                                // V4 tail: R/G/B/A masks (offsets 40..56).
    out.extend_from_slice(&masks[0].to_le_bytes()); // R
    out.extend_from_slice(&masks[1].to_le_bytes()); // G
    out.extend_from_slice(&masks[2].to_le_bytes()); // B
    out.extend_from_slice(&masks[3].to_le_bytes()); // A
                                                    // bV4CSType — calibrated RGB: endpoints + gamma are authoritative.
    out.extend_from_slice(&LCS_CALIBRATED_RGB.to_le_bytes());
    // CIEXYZTRIPLE endpoints (9 × i32 = 36 bytes).
    for v in endpoints {
        out.extend_from_slice(&v.to_le_bytes());
    }
    // Gamma R / G / B (3 × u32 = 12 bytes), unsigned 16.16 fixed point.
    for g in gamma_rgb {
        out.extend_from_slice(&g.to_le_bytes());
    }
    // Total: 40 + 16 + 4 + 36 + 12 = 108 ✓
}

/// Write a 124-byte `BITMAPV5HEADER` with `bV5CSType = PROFILE_EMBEDDED`.
///
/// Layout (offsets given relative to the start of the DIB header):
/// * 0..40   classic `BITMAPINFOHEADER` fields
/// * 40..56  R / G / B / A masks (zeroed — direct-colour `BI_RGB`)
/// * 56..60  `bV5CSType` = [`PROFILE_EMBEDDED`]
/// * 60..96  CIEXYZTRIPLE endpoints (zeroed)
/// * 96..108 R / G / B gamma (zeroed)
/// * 108..112 `bV5Intent`
/// * 112..116 `bV5ProfileData` (DIB-relative byte offset to ICC blob)
/// * 116..120 `bV5ProfileSize`
/// * 120..124 reserved (zero)
///
/// `image_size` is the byte length of the pixel array — written to
/// the classic `biSizeImage` slot at offset 20.
///
/// `compression` and `masks` parameterise the V3-prefix `biCompression`
/// field and the four-mask region at offsets 40..56. Direct-colour
/// `BI_RGB` paths pass `(BI_RGB, [0,0,0,0])`; the 16-bit V5 path passes
/// `(BI_BITFIELDS, [0xF800, 0x07E0, 0x001F, 0])` so the decoder picks
/// up the canonical RGB 5-6-5 mask layout from the header body the same
/// way it would on a V4 header.
#[allow(clippy::too_many_arguments)]
fn write_dib_header_v5_with_profile(
    out: &mut Vec<u8>,
    w: u32,
    stored_height: i32,
    bpp: u16,
    image_size: u32,
    compression: u32,
    masks: [u32; 4],
    cs_type: u32,
    rendering_intent: u32,
    profile_data_offset: u32,
    profile_size: u32,
) {
    // Classic V3 prefix (40 B).
    out.extend_from_slice(&BITMAPV5HEADER_SIZE.to_le_bytes());
    out.extend_from_slice(&(w as i32).to_le_bytes());
    out.extend_from_slice(&stored_height.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // planes
    out.extend_from_slice(&bpp.to_le_bytes());
    out.extend_from_slice(&compression.to_le_bytes()); // direct-colour or BI_BITFIELDS
    out.extend_from_slice(&image_size.to_le_bytes());
    out.extend_from_slice(&2835i32.to_le_bytes()); // x_pels/m ≈ 72 DPI
    out.extend_from_slice(&2835i32.to_le_bytes()); // y_pels/m
    out.extend_from_slice(&0u32.to_le_bytes()); // clr_used
    out.extend_from_slice(&0u32.to_le_bytes()); // clr_important
                                                // V4 tail: R/G/B/A masks. Zeroed for BI_RGB direct-colour;
                                                // canonical 5-6-5 layout for 16-bpp BI_BITFIELDS.
    out.extend_from_slice(&masks[0].to_le_bytes()); // R
    out.extend_from_slice(&masks[1].to_le_bytes()); // G
    out.extend_from_slice(&masks[2].to_le_bytes()); // B
    out.extend_from_slice(&masks[3].to_le_bytes()); // A
                                                    // bV5CSType — caller-selected: PROFILE_EMBEDDED routes the
                                                    // trailing blob through `BmpMetadata::icc_profile`;
                                                    // PROFILE_LINKED keeps the trailing blob as an opaque
                                                    // external-path bytestring that the decoder surfaces via
                                                    // `profile_data_offset` / `profile_size` without
                                                    // auto-loading.
    out.extend_from_slice(&cs_type.to_le_bytes());
    // CIEXYZTRIPLE endpoints (9 × i32 = 36 bytes) — zero; the profile
    // (embedded or linked) is the authoritative description.
    out.extend_from_slice(&[0u8; 36]);
    // Gamma R / G / B (3 × u32 = 12 bytes) — zero.
    out.extend_from_slice(&[0u8; 12]);
    // V5 tail.
    out.extend_from_slice(&rendering_intent.to_le_bytes());
    out.extend_from_slice(&profile_data_offset.to_le_bytes());
    out.extend_from_slice(&profile_size.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // bV5Reserved
                                                // Total: 40 + 16 + 4 + 36 + 12 + 4 + 4 + 4 + 4 = 124 ✓
}

/// Canonical RGB 5-6-5 mask quadruple (R, G, B, A=0) — used by the V5
/// 16-bpp BI_BITFIELDS encode path so its mask region matches the V4
/// 16-bpp encoder and the decoder's `BI_BITFIELDS` reader.
const RGB565_MASKS_V5: [u32; 4] = [0xF800, 0x07E0, 0x001F, 0x0000];

/// Canonical 32-bit alpha mask quadruple `(R=0, G=0, B=0, A=0xFF000000)`
/// for the V4 / V5 `BmpPixelFormat::Rgba` encode paths. The format stays
/// `BI_RGB` (so R / G / B keep the default BGRA byte order and their masks
/// are left zero, since the R / G / B masks are only valid under
/// `BI_BITFIELDS`), but the in-header alpha-mask slot at offset 52 is set
/// to the canonical high-byte mask. The BMP spec treats the alpha sample
/// as valid "whenever the alpha mask is present in the DIB header", so this
/// makes the emitted V4 / V5 32-bit file a spec-correct alpha-carrying
/// bitmap whose alpha the decoder recovers through the mask — rather than a
/// `BI_RGB` file that hides opacity in the reserved DWORD byte where a
/// strict reader would discard it.
const RGBA32_ALPHA_MASK_V4_V5: [u32; 4] = [0, 0, 0, 0xFF00_0000];

/// V5 header writer for the indexed encode paths. Layout is identical
/// to [`write_dib_header_v5_with_profile`] except that `biCompression`
/// is fixed at `BI_RGB`, the four-mask region at offsets 40..56 is
/// zeroed (the colour table that follows the V5 header carries the
/// colour assignment), and `biClrUsed` is written explicitly so a
/// minimal-palette table can be advertised to the decoder.
///
/// The header still finishes with the V5 tail
/// (`bV5Intent` / `bV5ProfileData` / `bV5ProfileSize` / `bV5Reserved`),
/// so the trailing ICC or path blob lives at `bV5ProfileData` after
/// the pixel array exactly like the direct-colour V5 paths.
#[allow(clippy::too_many_arguments)]
fn write_dib_header_v5_indexed_with_profile(
    out: &mut Vec<u8>,
    w: u32,
    stored_height: i32,
    bpp: u16,
    image_size: u32,
    clr_used: u32,
    cs_type: u32,
    rendering_intent: u32,
    profile_data_offset: u32,
    profile_size: u32,
) {
    out.extend_from_slice(&BITMAPV5HEADER_SIZE.to_le_bytes());
    out.extend_from_slice(&(w as i32).to_le_bytes());
    out.extend_from_slice(&stored_height.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // planes
    out.extend_from_slice(&bpp.to_le_bytes());
    out.extend_from_slice(&BI_RGB.to_le_bytes()); // indexed V5 → always uncompressed
    out.extend_from_slice(&image_size.to_le_bytes());
    out.extend_from_slice(&2835i32.to_le_bytes());
    out.extend_from_slice(&2835i32.to_le_bytes());
    out.extend_from_slice(&clr_used.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // clr_important
                                                // V4 mask region zeroed for BI_RGB indexed.
    out.extend_from_slice(&[0u8; 16]);
    out.extend_from_slice(&cs_type.to_le_bytes());
    out.extend_from_slice(&[0u8; 36]); // CIEXYZTRIPLE — zero, profile is authoritative
    out.extend_from_slice(&[0u8; 12]); // Gamma R/G/B — zero
    out.extend_from_slice(&rendering_intent.to_le_bytes());
    out.extend_from_slice(&profile_data_offset.to_le_bytes());
    out.extend_from_slice(&profile_size.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // bV5Reserved
}

// ---------------------------------------------------------------------------
// AND mask (ICO)
// ---------------------------------------------------------------------------

/// Build the 1-bpp AND mask (bottom-up, 4-byte padded rows) required
/// for a BMP embedded in a `.ico` / `.cur`. A set bit means "the pixel
/// under this one in the XOR mask is TRANSPARENT", matching every
/// `ICO` file you'll find in the wild.
fn build_and_mask_from_alpha(
    plane: &Plane,
    format: BmpPixelFormat,
    width: u32,
    height: u32,
) -> Result<Vec<u8>> {
    let w = width as usize;
    let h = height as usize;
    let stride = row_stride(w, 1);
    let mut mask = vec![0u8; stride * h];
    let in_stride = plane.stride;
    let bpp = match format {
        BmpPixelFormat::Rgba | BmpPixelFormat::Bgra => 4,
        // No alpha → fully opaque → all-zero AND mask. Short-circuit.
        _ => return Ok(mask),
    };
    for y in 0..h {
        let src_y = h - 1 - y; // match the bottom-up XOR layout
        let src = &plane.data[src_y * in_stride..src_y * in_stride + w * bpp];
        let dst = &mut mask[y * stride..y * stride + stride];
        for x in 0..w {
            let alpha = src[x * bpp + 3];
            if alpha == 0 {
                dst[x / 8] |= 1 << (7 - (x % 8));
            }
        }
    }
    Ok(mask)
}

// ---------------------------------------------------------------------------
// Utilities
// ---------------------------------------------------------------------------

/// Number of palette entries written for a given bit depth.
fn palette_entry_count(bpp: u16) -> usize {
    match bpp {
        1 => 2,
        2 => 4,
        4 => 16,
        8 => 256,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The largest buffer a 64-bit and a 32-bit target can address: the
    /// sizes must come out the same under both models, except where the
    /// buffer is too large for a 32-bit address space.
    const ADDRESSABLE_64: u64 = u64::MAX;
    const ADDRESSABLE_32: u64 = u32::MAX as u64;

    /// A 1024-wide `Pal8` image: all zeros, or noise RLE cannot shrink.
    fn pal8(height: u32, noise: bool) -> BmpImage {
        let mut state = 0x2545_f491u32;
        let data = (0..1024 * height as usize)
            .map(|_| {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                if noise {
                    (state >> 24) as u8
                } else {
                    0
                }
            })
            .collect();
        BmpImage::packed(1024, height, BmpPixelFormat::Pal8, 1024, data)
            .unwrap()
            .with_palette(Palette::from_rgb(&[[0, 0, 0]]))
    }

    /// An all-zero `Pal8` image of 1024 x 4,194,303 has a 4,294,967,350
    /// byte uncompressed file, over what `bfSize` can record, and a
    /// 50,331,636-byte RLE stream (each row is four 255-pixel runs, one
    /// of 4 pixels and its 2-byte marker: 12 bytes), so a 50,332,714-byte
    /// file. The sizes are computed without wrapping and leave room for
    /// that stream; nothing that large is allocated.
    #[test]
    fn a_four_gib_uncompressed_file_is_sized_without_wrapping() {
        let head: u64 = 14 + 40 + 256 * 4;
        let raw: u64 = 1024 * 4_194_303;
        let slack = Some(rle_slack(1024));
        let sizes = Sizes::new(head, raw, 0, slack, MAX_FILE, ADDRESSABLE_64).unwrap();
        assert_eq!(sizes.file, 4_294_967_350);
        assert!(!sizes.file_fits);
        let budget = sizes.rle_budget.unwrap();
        assert_eq!(
            head + budget - 1,
            MAX_FILE,
            "the stream may fill the file to the limit"
        );
        let stream: u64 = 4_194_303 * 12;
        assert!(stream < budget);
        assert_eq!(head + stream, 50_332_714);
        // Without RLE the same file is refused.
        assert!(Sizes::new(head, raw, 0, None, MAX_FILE, ADDRESSABLE_64).is_err());
        // The largest file that fits, and one byte more.
        assert!(Sizes::new(54, MAX_FILE - 54, 0, None, MAX_FILE, ADDRESSABLE_64).is_ok());
        assert!(Sizes::new(54, MAX_FILE - 53, 0, None, MAX_FILE, ADDRESSABLE_64).is_err());
        // A 32-bit target cannot hold the probe's buffer: an error, not
        // a `Vec::reserve` panic.
        let err = Sizes::new(head, raw, 0, slack, MAX_FILE, ADDRESSABLE_32).unwrap_err();
        assert!(err.to_string().contains("can address"), "{err}");
    }

    /// The limit arithmetic is `u64` on every target: under a 32-bit
    /// address model an ordinary RLE-eligible image gets exactly the
    /// sizes, and so the RLE budget, a 64-bit target gives it (a
    /// `usize` `limit + 1` overflowed there and zeroed the budget).
    #[test]
    fn sizes_are_the_same_under_a_32_bit_address_model() {
        let cases: [(u64, u64, u64, Option<u64>); 6] = [
            // All-zero 1024 x 2 Pal8, full table: a 24-byte RLE stream.
            (1078, 2048, 0, Some(rle_slack(1024))),
            // Indexed4, 16-entry table.
            (118, 512 * 300, 0, Some(rle_slack(1024))),
            // Rgba, V3; and a V5 file with a profile blob.
            (54, 4096 * 1024, 0, None),
            (138, 4096 * 1024, 37, None),
            // The largest raw-fitting RLE candidate a 32-bit target can
            // still reserve for.
            (1078, MAX_FILE - 1078 - 2050, 0, Some(2050)),
            // Exactly the limit, no RLE.
            (54, MAX_FILE - 54, 0, None),
        ];
        for (head, raw, tail, slack) in cases {
            let wide = Sizes::new(head, raw, tail, slack, MAX_FILE, ADDRESSABLE_64).unwrap();
            let narrow = Sizes::new(head, raw, tail, slack, MAX_FILE, ADDRESSABLE_32).unwrap();
            assert_eq!(narrow, wide, "head {head} raw {raw}");
            if slack.is_some() {
                assert_eq!(wide.rle_budget, Some(raw), "the budget is the raw array");
            }
        }
        // The 1024 x 2 case written for real on this target: RLE wins.
        let zeros = pal8(2, false);
        let (bytes, format) = crate::encode_with_report(&zeros, &EncodeOptions::default()).unwrap();
        assert_eq!(format, EncodedBmpFormat::Rle8);
        assert_eq!(bytes.len(), 1078 + 24);
    }

    /// The same decisions at a lower limit, written for real: an RLE
    /// stream that brings an over-limit file under the limit is written
    /// as before, and when nothing brings it under, an error comes back
    /// with `out` as it was.
    #[test]
    fn a_file_over_the_limit_is_written_compressed_or_refused() {
        let opts = EncodeOptions::default();
        let zeros = pal8(300, false);
        let rle_file = crate::encode(&zeros, &opts).unwrap();
        assert_eq!(rle_file.len(), 1078 + 300 * 12, "12 bytes per all-zero row");
        let rle_len = rle_file.len() as u64;
        let raw_file: u64 = 1078 + 1024 * 300;
        let src = EncodeSource::from_image(&zeros).unwrap();
        for limit in [MAX_FILE, raw_file - 1, rle_len] {
            let plan = Plan::new(&src, &opts).unwrap().with_limit(limit).unwrap();
            let mut out = b"head".to_vec();
            let format = plan.write_into(&mut out).unwrap();
            assert_eq!(format, EncodedBmpFormat::Rle8, "limit {limit}");
            assert!(out[4..] == rle_file[..], "limit {limit}");
        }
        let plan = Plan::new(&src, &opts)
            .unwrap()
            .with_limit(rle_len - 1)
            .unwrap();
        let mut out = b"head".to_vec();
        let err = plan.write_into(&mut out).unwrap_err();
        assert!(
            err.to_string()
                .contains("RLE stream does not bring it under"),
            "{err}"
        );
        assert_eq!(out, b"head");

        // Without RLE an over-limit file is refused while planning.
        let raw_opts = opts.clone().with_rle(false);
        let plan = Plan::new(&src, &raw_opts).unwrap();
        assert!(plan.with_limit(raw_file).is_ok());
        let plan = Plan::new(&src, &raw_opts).unwrap();
        let err = plan.with_limit(raw_file - 1).err().unwrap();
        assert!(err.to_string().contains("BMP size fields"), "{err}");

        // Noise: the RLE stream loses, so the raw file has to fit.
        let noise = pal8(300, true);
        let noise_file = crate::encode(&noise, &opts).unwrap();
        assert_eq!(noise_file.len() as u64, raw_file);
        let src = EncodeSource::from_image(&noise).unwrap();
        let plan = Plan::new(&src, &opts)
            .unwrap()
            .with_limit(raw_file)
            .unwrap();
        let mut out = Vec::new();
        assert_eq!(
            plan.write_into(&mut out).unwrap(),
            EncodedBmpFormat::Indexed8
        );
        assert!(out == noise_file);
        let plan = Plan::new(&src, &opts)
            .unwrap()
            .with_limit(raw_file - 1)
            .unwrap();
        let mut out = b"head".to_vec();
        assert!(plan.write_into(&mut out).is_err());
        assert_eq!(out, b"head");
    }
}
