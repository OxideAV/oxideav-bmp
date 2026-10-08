//! Decode-side limits / strictness ([`DecodeOptions`]) and the encoder
//! knobs ([`EncodeOptions`]).

use crate::encoder::BmpBitfields;
use crate::error::{BmpError, Result};
use crate::types::LCS_GM_IMAGES;

/// Limits and strictness for [`crate::decode_with`] / [`crate::decode_dib_with`].
///
/// Every limit is checked against the header **before** any pixel
/// buffer is allocated, so a hostile header fails with
/// [`BmpError::LimitExceeded`] instead of committing memory. The
/// defaults are: no dimension / pixel-count limit, decoded plane capped
/// at [`DecodeOptions::DEFAULT_MAX_BYTES`] (1 GiB), `strict = false`,
/// `copy_icc = true`.
///
/// `strict` turns two tolerances off:
///
/// * `bfOffBits` recovery — a pixel offset of `0`, or one that points
///   inside the header / colour table, is normally replaced by the
///   canonical header → masks → colour table → pixels position; strict
///   mode rejects it as `InvalidData`;
/// * the two `BITMAPFILEHEADER` reserved words, documented as "must be
///   zero", are normally ignored; strict mode rejects a non-zero value.
///
/// `copy_icc` selects whether a V5 header's embedded ICC profile is
/// copied out of the file into [`crate::Metadata::icc`]. It is by
/// default. The profile is as large as the file says, up to the whole
/// file, so a caller that wants the pixels, or that reads the profile
/// where it lies in the file, turns it off: `metadata.icc` is then
/// `None`, and nothing profile-sized is allocated. [`crate::info`]
/// still reports `has_icc`, and the header's `bV5ProfileData` /
/// `bV5ProfileSize` ([`crate::DibHeader`]) locate the profile.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct DecodeOptions {
    /// Reject images wider than this (pixels).
    pub max_width: Option<u32>,
    /// Reject images taller than this (pixels).
    pub max_height: Option<u32>,
    /// Reject images with more than this many pixels (`width × height`).
    pub max_pixels: Option<u64>,
    /// Reject images whose decoded plane would exceed this many bytes
    /// (`width × bytes_per_pixel × height` of the native layout).
    pub max_bytes: Option<u64>,
    /// Enforce the file-header rules (see the type docs).
    pub strict: bool,
    /// Copy an embedded ICC profile into the decoded image's metadata
    /// (see the type docs). Default `true`.
    pub copy_icc: bool,
}

impl DecodeOptions {
    /// Default [`Self::max_bytes`]: 1 GiB of decoded plane.
    pub const DEFAULT_MAX_BYTES: u64 = 1 << 30;

    /// The defaults (see the type docs).
    pub fn new() -> Self {
        Self::default()
    }

    /// Set (or lift with `None`) the width limit.
    pub fn with_max_width(mut self, max_width: impl Into<Option<u32>>) -> Self {
        self.max_width = max_width.into();
        self
    }

    /// Set (or lift with `None`) the height limit.
    pub fn with_max_height(mut self, max_height: impl Into<Option<u32>>) -> Self {
        self.max_height = max_height.into();
        self
    }

    /// Set (or lift with `None`) the pixel-count limit.
    pub fn with_max_pixels(mut self, max_pixels: impl Into<Option<u64>>) -> Self {
        self.max_pixels = max_pixels.into();
        self
    }

    /// Set (or lift with `None`) the decoded-bytes limit.
    pub fn with_max_bytes(mut self, max_bytes: impl Into<Option<u64>>) -> Self {
        self.max_bytes = max_bytes.into();
        self
    }

    /// Set strict mode.
    pub fn with_strict(mut self, strict: bool) -> Self {
        self.strict = strict;
        self
    }

    /// Set whether an embedded ICC profile is copied into the decoded
    /// image (see the type docs).
    pub fn with_copy_icc(mut self, copy_icc: bool) -> Self {
        self.copy_icc = copy_icc;
        self
    }

    /// Lift every limit (`max_*` all `None`).
    pub fn unlimited(mut self) -> Self {
        self.max_width = None;
        self.max_height = None;
        self.max_pixels = None;
        self.max_bytes = None;
        self
    }

    /// Check a header's geometry against the limits. `bytes` is the
    /// decoded plane size the native layout implies.
    pub(crate) fn check(&self, width: u32, height: u32, bytes: u64) -> Result<()> {
        if let Some(m) = self.max_width {
            if width > m {
                return Err(BmpError::limit(format!(
                    "BMP: width {width} exceeds max_width {m}"
                )));
            }
        }
        if let Some(m) = self.max_height {
            if height > m {
                return Err(BmpError::limit(format!(
                    "BMP: height {height} exceeds max_height {m}"
                )));
            }
        }
        let pixels = u64::from(width) * u64::from(height);
        if let Some(m) = self.max_pixels {
            if pixels > m {
                return Err(BmpError::limit(format!(
                    "BMP: {pixels} pixels exceed max_pixels {m}"
                )));
            }
        }
        if let Some(m) = self.max_bytes {
            if bytes > m {
                return Err(BmpError::limit(format!(
                    "BMP: decoded plane of {bytes} bytes exceeds max_bytes {m}"
                )));
            }
        }
        Ok(())
    }
}

impl Default for DecodeOptions {
    fn default() -> Self {
        Self {
            max_width: None,
            max_height: None,
            max_pixels: None,
            max_bytes: Some(Self::DEFAULT_MAX_BYTES),
            strict: false,
            copy_icc: true,
        }
    }
}

/// V4 `LCS_CALIBRATED_RGB` colour description: the CIE XYZ endpoints
/// (`CIEXYZTRIPLE`, nine Q2.30 fixed-point values in R·xyz, G·xyz,
/// B·xyz order) and the per-channel Q16.16 gamma triple, written
/// verbatim into the header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct CalibratedRgb {
    /// `bV4Endpoints`: R·x, R·y, R·z, G·x, G·y, G·z, B·x, B·y, B·z.
    pub endpoints: [i32; 9],
    /// `bV4GammaRed`, `bV4GammaGreen`, `bV4GammaBlue` (Q16.16).
    pub gamma: [u32; 3],
}

impl CalibratedRgb {
    /// Pair the endpoints with the gamma triple.
    pub const fn new(endpoints: [i32; 9], gamma: [u32; 3]) -> Self {
        Self { endpoints, gamma }
    }
}

/// Encoder options for [`crate::encode`] / [`crate::encode_rgb8`] /
/// [`crate::encode_rgba8`] / [`crate::encode_to`].
///
/// The defaults write the classic layout: bottom-up rows, a full
/// `2^bpp` colour table for indexed input, RLE when it is smaller, a
/// V3 `BITMAPINFOHEADER` (V4 for `Rgb565`), and — when the image
/// carries [`crate::Metadata::icc`] — a V5 header embedding that
/// profile.
///
/// Three header families are mutually exclusive, because a file has one
/// DIB header: [`bitfields`](Self::bitfields) (V3 with a mask tail),
/// [`calibrated_rgb`](Self::calibrated_rgb) (V4 `LCS_CALIBRATED_RGB`),
/// and the V5 profile modes ([`embed_icc`](Self::embed_icc) with an
/// ICC profile present, or [`linked_icc`](Self::linked_icc)). Asking
/// for more than one is [`BmpError::Unsupported`].
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct EncodeOptions {
    /// Emit a top-down DIB (rows stored top-to-bottom, encoded
    /// `biHeight` negative). Top-down is legal for uncompressed
    /// bitmaps only, so it also disables RLE. Default `false`.
    pub top_down: bool,
    /// Write only as many colour-table entries as the palette carries
    /// (recording the count in `biClrUsed`) instead of a zero-padded
    /// `2^bpp` table. Indexed layouts only. Default `false`.
    pub minimal_palette: bool,
    /// Allow `BI_RLE8` / `BI_RLE4` for `Pal8` / `Indexed4` input when
    /// the compressed stream is smaller than the raw pixel array.
    /// Default `true`. (Ignored, i.e. uncompressed, under
    /// [`top_down`](Self::top_down) or when writing a headerless DIB.)
    pub rle: bool,
    /// Explicit `BI_BITFIELDS` / `BI_ALPHABITFIELDS` masks (V3 header +
    /// mask tail) for `Rgba` / `Rgb24` / `Bgra` / `Bgr24` input: the
    /// pixels are quantised into the given 16- or 32-bit word layout.
    /// `None` (default) writes the format's native layout.
    pub bitfields: Option<BmpBitfields>,
    /// Embed [`crate::Metadata::icc`] as a V5 `PROFILE_EMBEDDED`
    /// header when the image carries one. Default `true`; `false`
    /// drops the profile and writes the plain header.
    pub embed_icc: bool,
    /// Write a V5 `PROFILE_LINKED` header whose profile slot holds this
    /// path bytestring verbatim (the caller chooses the encoding and
    /// terminator). `None` by default. Takes precedence over
    /// [`embed_icc`](Self::embed_icc).
    pub linked_icc: Option<Vec<u8>>,
    /// `bV5Intent` for the two V5 profile modes. Default
    /// `LCS_GM_IMAGES` (perceptual, the value for pictures).
    pub rendering_intent: u32,
    /// Write a V4 `LCS_CALIBRATED_RGB` header with these endpoints and
    /// gamma. `None` by default.
    pub calibrated_rgb: Option<CalibratedRgb>,
}

impl EncodeOptions {
    /// The defaults (see the type docs).
    pub fn new() -> Self {
        Self::default()
    }

    /// Set [`Self::top_down`].
    pub fn with_top_down(mut self, top_down: bool) -> Self {
        self.top_down = top_down;
        self
    }

    /// Set [`Self::minimal_palette`].
    pub fn with_minimal_palette(mut self, minimal_palette: bool) -> Self {
        self.minimal_palette = minimal_palette;
        self
    }

    /// Set [`Self::rle`].
    pub fn with_rle(mut self, rle: bool) -> Self {
        self.rle = rle;
        self
    }

    /// Set (or clear) [`Self::bitfields`].
    pub fn with_bitfields(mut self, bitfields: impl Into<Option<BmpBitfields>>) -> Self {
        self.bitfields = bitfields.into();
        self
    }

    /// Set [`Self::embed_icc`].
    pub fn with_embed_icc(mut self, embed_icc: bool) -> Self {
        self.embed_icc = embed_icc;
        self
    }

    /// Set (or clear) [`Self::linked_icc`].
    pub fn with_linked_icc(mut self, path: impl Into<Option<Vec<u8>>>) -> Self {
        self.linked_icc = path.into();
        self
    }

    /// Set [`Self::rendering_intent`].
    pub fn with_rendering_intent(mut self, intent: u32) -> Self {
        self.rendering_intent = intent;
        self
    }

    /// Set (or clear) [`Self::calibrated_rgb`].
    pub fn with_calibrated_rgb(mut self, calibrated: impl Into<Option<CalibratedRgb>>) -> Self {
        self.calibrated_rgb = calibrated.into();
        self
    }
}

impl Default for EncodeOptions {
    fn default() -> Self {
        Self {
            top_down: false,
            minimal_palette: false,
            rle: true,
            bitfields: None,
            embed_icc: true,
            linked_icc: None,
            rendering_intent: LCS_GM_IMAGES,
            calibrated_rgb: None,
        }
    }
}

/// The pre-contract name of [`EncodeOptions`] (same `top_down` /
/// `minimal_palette` fields; build it with `EncodeOptions::default()`
/// and the `with_*` setters).
#[deprecated(note = "use oxideav_bmp::EncodeOptions (IMAGE_CRATE_API)")]
pub type BmpEncodeOptions = EncodeOptions;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits_fire_in_order() {
        let o = DecodeOptions::default()
            .with_max_width(10u32)
            .with_max_height(10u32)
            .with_max_pixels(50u64)
            .with_max_bytes(100u64);
        assert!(o.check(5, 5, 75).is_ok());
        assert!(matches!(o.check(11, 1, 1), Err(BmpError::LimitExceeded(_))));
        assert!(matches!(o.check(1, 11, 1), Err(BmpError::LimitExceeded(_))));
        assert!(matches!(o.check(8, 8, 1), Err(BmpError::LimitExceeded(_))));
        assert!(matches!(
            o.check(5, 5, 101),
            Err(BmpError::LimitExceeded(_))
        ));
        assert!(o.unlimited().check(u32::MAX, u32::MAX, u64::MAX).is_ok());
    }

    #[test]
    fn encode_defaults() {
        let o = EncodeOptions::default();
        assert!(!o.top_down);
        assert!(!o.minimal_palette);
        assert!(o.rle);
        assert!(o.bitfields.is_none());
        assert!(o.embed_icc);
        assert!(o.linked_icc.is_none());
        assert_eq!(o.rendering_intent, LCS_GM_IMAGES);
        assert!(o.calibrated_rgb.is_none());
    }
}
