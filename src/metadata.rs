//! Colour-space + ICC metadata extracted from V4 / V5 BMP headers.
//!
//! The BMP V4 header (108 bytes total, introduced with Windows 95 / NT
//! 4.0) added a `bV4CSType` field plus a `CIEXYZTRIPLE` of red/green/blue
//! endpoints and per-channel gamma values, all aimed at letting callers
//! describe the colour space of the bitmap without bumping the format.
//! The V5 header (124 bytes) extended that with a rendering intent and a
//! pair of fields that point at an external ICC profile (`PROFILE_LINKED`)
//! or an embedded one (`PROFILE_EMBEDDED`) carried after the pixel array.
//!
//! Decoder consumers can keep using [`crate::decode_bmp`] /
//! [`crate::decode_dib`] when they only want pixels. Callers that want
//! the V4/V5 colour-space tail too use [`crate::decode_bmp_with_metadata`]
//! / [`crate::decode_dib_with_metadata`] instead — those return both the
//! same [`crate::BmpImage`] *and* the parsed [`BmpMetadata`] alongside it,
//! so the metadata path stays additive to the existing API surface.

use crate::types::{
    DibHeader, Os2Header2Raw, BITMAPINFOHEADER_SIZE, BITMAPV4HEADER_SIZE, LCS_CALIBRATED_RGB,
    LCS_GM_ABS_COLORIMETRIC, LCS_GM_BUSINESS, LCS_GM_GRAPHICS, LCS_GM_IMAGES, LCS_S_RGB,
    LCS_WINDOWS_COLOR_SPACE, OS2_HALFTONE_ERROR_DIFFUSION, OS2_HALFTONE_NONE, OS2_HALFTONE_PANDA,
    OS2_HALFTONE_SUPER_CIRCLE, PROFILE_EMBEDDED, PROFILE_LINKED,
};

/// Colour-space declaration carried by a V4 / V5 BMP header.
///
/// The five enumerated variants cover every legal value of the V4
/// `bV4CSType` field; V5 additionally allows the two `Profile*` variants.
/// Any other on-disk value is surfaced as [`Unknown`](Self::Unknown) so a
/// caller can tell apart "decoder didn't recognise the tag" from "header
/// didn't carry one".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BmpColorSpace {
    /// `LCS_CALIBRATED_RGB` (0): the V4 `endpoints` + `gamma` describe
    /// the colour space directly. Use those fields on
    /// [`BmpMetadata::endpoints`] / [`BmpMetadata::gamma_rgb`].
    Calibrated,
    /// `LCS_sRGB`: bitmap is in the sRGB colour space.
    SRgb,
    /// `LCS_WINDOWS_COLOR_SPACE`: bitmap is in the current Windows
    /// default colour space.
    Windows,
    /// `PROFILE_LINKED` (V5 only): the V5 `profile_data` field points
    /// at a file-path bytestring identifying an external ICC profile.
    ProfileLinked,
    /// `PROFILE_EMBEDDED` (V5 only): an ICC profile blob lives after
    /// the pixel array; [`BmpMetadata::icc_profile`] carries the bytes.
    ProfileEmbedded,
    /// A `bV4CSType` value the BMP spec doesn't define; passed through
    /// verbatim so a caller can investigate without losing data.
    Unknown(u32),
}

impl BmpColorSpace {
    /// Map a raw `bV4CSType` / `bV5CSType` value to a [`BmpColorSpace`].
    pub fn from_raw(value: u32) -> Self {
        match value {
            LCS_CALIBRATED_RGB => BmpColorSpace::Calibrated,
            LCS_S_RGB => BmpColorSpace::SRgb,
            LCS_WINDOWS_COLOR_SPACE => BmpColorSpace::Windows,
            PROFILE_LINKED => BmpColorSpace::ProfileLinked,
            PROFILE_EMBEDDED => BmpColorSpace::ProfileEmbedded,
            other => BmpColorSpace::Unknown(other),
        }
    }
}

/// V5 rendering intent (`bV5Intent`).
///
/// Maps the four documented values plus an [`Unspecified`](Self::Unspecified)
/// "no intent set" variant (value 0) and a passthrough for unknown values
/// (so a non-zero on-disk value never silently collapses to a default).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BmpRenderingIntent {
    /// `bV5Intent = 0`: the header carries no intent (V4 headers always
    /// land here since the field doesn't exist there).
    Unspecified,
    /// `LCS_GM_BUSINESS`: saturation (graphics / business charts).
    Saturation,
    /// `LCS_GM_GRAPHICS`: relative colorimetric (proofing).
    RelativeColorimetric,
    /// `LCS_GM_IMAGES`: perceptual (photographs).
    Perceptual,
    /// `LCS_GM_ABS_COLORIMETRIC`: absolute colorimetric.
    AbsoluteColorimetric,
    /// Anything else the V5 spec doesn't define.
    Unknown(u32),
}

impl BmpRenderingIntent {
    /// Map a raw `bV5Intent` value to a [`BmpRenderingIntent`].
    pub fn from_raw(value: u32) -> Self {
        match value {
            0 => BmpRenderingIntent::Unspecified,
            LCS_GM_BUSINESS => BmpRenderingIntent::Saturation,
            LCS_GM_GRAPHICS => BmpRenderingIntent::RelativeColorimetric,
            LCS_GM_IMAGES => BmpRenderingIntent::Perceptual,
            LCS_GM_ABS_COLORIMETRIC => BmpRenderingIntent::AbsoluteColorimetric,
            other => BmpRenderingIntent::Unknown(other),
        }
    }
}

/// Typed view of the V5 ICC profile reference.
///
/// Discriminates the three V5 colour-management outcomes a caller cares
/// about without forcing it to match `bV5CSType` and read `icc_profile`
/// / `profile_data_offset` / `profile_size` by hand:
///
/// * [`BmpIccProfileRef::None`] — the BMP doesn't declare an ICC
///   profile (V3 / V4 / OS-2 headers, or a V5 header whose `bV5CSType`
///   is `LCS_CALIBRATED_RGB` / `LCS_sRGB` / `LCS_WINDOWS_COLOR_SPACE`).
/// * [`BmpIccProfileRef::Embedded`] — the V5 header carries
///   `PROFILE_EMBEDDED` and the ICC profile bytes were successfully
///   sliced out of the input buffer.
/// * [`BmpIccProfileRef::Linked`] — the V5 header carries
///   `PROFILE_LINKED` and the trailing slot held a caller-encoded path
///   bytestring that was successfully sliced out of the input buffer.
/// * [`BmpIccProfileRef::Declared`] — the V5 header declared either
///   PROFILE_* variant but the trailing-slot offset/size lay past EOF
///   or had size `0`; the declared fields are still surfaced so the
///   caller can investigate, but the bytes are unavailable.
///
/// The variant returned by [`BmpMetadata::icc_profile_ref`] is borrowed
/// from `self`; callers that need owned bytes can `to_vec()` the slice
/// or use the existing `icc_profile` / `linked_profile_path` fields
/// directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BmpIccProfileRef<'a> {
    /// No V5 ICC profile reference is present.
    None,
    /// `bV5CSType = PROFILE_EMBEDDED` and the embedded ICC bytes were
    /// sliced out of the input. `bytes` is the embedded profile.
    Embedded(&'a [u8]),
    /// `bV5CSType = PROFILE_LINKED` and the linked path bytestring was
    /// sliced out of the input. `path_bytes` is verbatim — the BMP
    /// spec leaves path encoding system-dependent (typically null-
    /// terminated ANSI on Windows), so the caller decides how to
    /// interpret it.
    Linked(&'a [u8]),
    /// The V5 header declared one of the PROFILE_* variants but the
    /// trailing-slot reference could not be resolved against the input
    /// buffer (truncated file, lying offset, or zero size). The
    /// declared `cs_type` plus `profile_data_offset` / `profile_size`
    /// are still surfaced on [`BmpMetadata`] so the caller can decide
    /// whether to treat the BMP as invalid or fall back to a default
    /// colour space.
    Declared {
        /// Whichever of [`PROFILE_EMBEDDED`](crate::PROFILE_EMBEDDED)
        /// / [`PROFILE_LINKED`](crate::PROFILE_LINKED) the V5 header
        /// declared.
        cs_type: u32,
        /// The DIB-relative `bV5ProfileData` offset the V5 header
        /// declared.
        profile_data_offset: u32,
        /// The `bV5ProfileSize` value the V5 header declared.
        profile_size: u32,
    },
}

/// OS/2 2.x halftoning algorithm (`usRendering`, offset 46 of a full
/// 64-byte `OS22XBITMAPHEADER`).
///
/// Maps the four documented values; any other on-disk value is surfaced
/// as [`Unknown`](Self::Unknown) so a caller can tell "decoder didn't
/// recognise the algorithm" apart from "header declared no halftoning".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BmpOs2Halftone {
    /// `0`: no halftoning (the most common value).
    None,
    /// `1`: error diffusion. The halftoning `size1` parameter is the
    /// percentage of error damping (100 = no damping, 0 = errors not
    /// diffused).
    ErrorDiffusion,
    /// `2`: PANDA (Processing Algorithm for Noncoded Document
    /// Acquisition). `size1` / `size2` are the X / Y dimensions, in
    /// pixels, of the halftoning pattern.
    Panda,
    /// `3`: super-circle. `size1` / `size2` are the X / Y dimensions, in
    /// pixels, of the halftoning pattern.
    SuperCircle,
    /// A `usRendering` value the OS/2 2.x header documentation doesn't
    /// define; passed through verbatim.
    Unknown(u16),
}

impl BmpOs2Halftone {
    /// Map a raw `usRendering` value to a [`BmpOs2Halftone`].
    pub fn from_raw(value: u16) -> Self {
        match value {
            OS2_HALFTONE_NONE => BmpOs2Halftone::None,
            OS2_HALFTONE_ERROR_DIFFUSION => BmpOs2Halftone::ErrorDiffusion,
            OS2_HALFTONE_PANDA => BmpOs2Halftone::Panda,
            OS2_HALFTONE_SUPER_CIRCLE => BmpOs2Halftone::SuperCircle,
            other => BmpOs2Halftone::Unknown(other),
        }
    }
}

/// Typed view of the trailing 24 bytes of a full 64-byte OS/2 2.x
/// `OS22XBITMAPHEADER` (`BITMAPINFOHEADER2` in IBM's documentation).
///
/// Present on [`BmpMetadata::os2_header2`] only when the decoded DIB
/// header is exactly 64 bytes — the full IBM form. The truncated OS/2
/// 2.x forms (`biSize` 16..40) and every Windows header generation have
/// no room for these fields, so `os2_header2` is `None` for them. The
/// only documented value for `units` / `recording` / `color_encoding`
/// is `0`; the raw values are surfaced via the dedicated accessors so a
/// non-standard write is distinguishable from the default.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BmpOs2Header2 {
    /// `usUnits` (offset 40): resolution units for the
    /// pixels-per-X-axis fields. The only documented value is `0`,
    /// meaning pixels per metre.
    pub units: u16,
    /// `usRecording` (offset 44): the direction in which bits fill the
    /// bitmap. The only documented value is `0`, meaning a lower-left
    /// origin (left-to-right, then bottom-to-top). A Windows-style
    /// upper-left origin is instead expressed by a negative `biHeight`.
    pub recording: u16,
    /// `usRendering` (offset 46): the halftoning algorithm, mapped to a
    /// [`BmpOs2Halftone`].
    pub halftone: BmpOs2Halftone,
    /// `cSize1` (offset 48): halftoning parameter 1. Its meaning depends
    /// on [`halftone`](Self::halftone) — error-damping percentage for
    /// error diffusion, or the pattern X dimension for PANDA /
    /// super-circle.
    pub halftone_size1: u32,
    /// `cSize2` (offset 52): halftoning parameter 2 — the pattern Y
    /// dimension for PANDA / super-circle; unused for the other
    /// algorithms.
    pub halftone_size2: u32,
    /// `ulColorEncoding` (offset 56): the colour encoding of each
    /// colour-table entry. The only documented value is `0`, meaning RGB.
    pub color_encoding: u32,
    /// `ulIdentifier` (offset 60): an application-defined identifier.
    /// Not used for image rendering; surfaced verbatim.
    pub identifier: u32,
}

impl BmpOs2Header2 {
    fn from_raw(raw: Os2Header2Raw) -> Self {
        Self {
            units: raw.units,
            recording: raw.recording,
            halftone: BmpOs2Halftone::from_raw(raw.rendering),
            halftone_size1: raw.size1,
            halftone_size2: raw.size2,
            color_encoding: raw.color_encoding,
            identifier: raw.identifier,
        }
    }

    /// `true` when `units` is the documented default (`0`, pixels per
    /// metre).
    pub fn units_is_pels_per_meter(&self) -> bool {
        self.units == crate::types::OS2_UNITS_PELS_PER_METER
    }

    /// `true` when `recording` is the documented default (`0`, lower-left
    /// origin / bottom-up fill).
    pub fn is_bottom_up(&self) -> bool {
        self.recording == crate::types::OS2_RECORDING_BOTTOM_UP
    }

    /// `true` when `color_encoding` is the documented default (`0`, RGB).
    pub fn color_encoding_is_rgb(&self) -> bool {
        self.color_encoding == crate::types::OS2_COLOR_ENCODING_RGB
    }
}

/// Header-derived colour-space metadata for a decoded BMP.
///
/// V3 / OS/2 headers don't carry any of this; the decoder fills every
/// field with `None` / the sentinel zeroes when it sees one of those.
/// V4 fills `color_space` / `endpoints` / `gamma_rgb`. V5 additionally
/// fills `rendering_intent`; for the [`BmpColorSpace::ProfileEmbedded`]
/// variant the embedded ICC profile bytes are pulled into
/// [`Self::icc_profile`], and for the [`BmpColorSpace::ProfileLinked`]
/// variant the path bytestring is pulled into
/// [`Self::linked_profile_path`]. The typed accessor
/// [`Self::icc_profile_ref`] returns a single
/// [`BmpIccProfileRef`] discriminated view of whichever variant is
/// present.
#[derive(Debug, Clone)]
pub struct BmpMetadata {
    /// Reported DIB header size. 12 = OS/2 `BITMAPCOREHEADER`, 40 = V3
    /// `BITMAPINFOHEADER`, 108 = V4 `BITMAPV4HEADER`, 124 = V5
    /// `BITMAPV5HEADER`.
    pub header_size: u32,
    /// Parsed `bV4CSType` / `bV5CSType`. `None` for V3 / OS/2.
    pub color_space: Option<BmpColorSpace>,
    /// V4+ endpoints (R/G/B × X/Y/Z). `None` for V3 / OS/2; the raw
    /// `i32` values are passed through verbatim — they are documented
    /// as fixed-point Q2.30 but the decoder doesn't reinterpret them.
    pub endpoints: Option<[i32; 9]>,
    /// V4+ gamma triple (R / G / B), each a u32 fixed-point Q16.16.
    /// `None` for V3 / OS/2.
    pub gamma_rgb: Option<[u32; 3]>,
    /// V5 rendering intent. `None` for V3 / V4 / OS/2.
    pub rendering_intent: Option<BmpRenderingIntent>,
    /// V5 `bV5ProfileData`. For [`BmpColorSpace::ProfileLinked`] this
    /// is the offset (from the start of the DIB) of a file-path
    /// bytestring; for [`BmpColorSpace::ProfileEmbedded`] this is the
    /// offset of the embedded ICC blob itself.
    pub profile_data_offset: Option<u32>,
    /// V5 `bV5ProfileSize` — byte length of the linked path or the
    /// embedded ICC blob.
    pub profile_size: Option<u32>,
    /// Embedded ICC profile bytes when [`color_space`](Self::color_space)
    /// is [`BmpColorSpace::ProfileEmbedded`]. Decoded from
    /// `whole[BITMAPFILEHEADER_SIZE + profile_data_offset..][..profile_size]`
    /// for [`crate::decode_bmp_with_metadata`] and from
    /// `dib[profile_data_offset..][..profile_size]` for
    /// [`crate::decode_dib_with_metadata`]. `None` for every other CS
    /// type (and for `ProfileEmbedded` cases where the bytes lie about
    /// the offset / size and the slice falls past EOF — the metadata
    /// fields are still populated so callers can inspect what was
    /// declared).
    pub icc_profile: Option<Vec<u8>>,
    /// Linked-path bytestring when [`color_space`](Self::color_space)
    /// is [`BmpColorSpace::ProfileLinked`]. The path blob sits in the
    /// trailing slot at the same DIB-relative `bV5ProfileData` /
    /// `bV5ProfileSize` location used by the embedded variant — only
    /// the `bV5CSType` discriminator distinguishes the two on the
    /// wire. `None` for every other colour-space variant, and for
    /// `ProfileLinked` cases where the declared offset / size slip
    /// past EOF or have size `0` (the declared fields stay populated
    /// so the caller can investigate). The decoder never auto-loads
    /// the file the path points at: this slot is the path bytestring
    /// verbatim and its encoding is system-dependent per the BMP
    /// spec (typically null-terminated ANSI on Windows).
    pub linked_profile_path: Option<Vec<u8>>,
    /// Horizontal resolution of the target device, in pixels per
    /// metre (`biXPelsPerMeter`). The BMP spec documents this as a
    /// signed `LONG`; this field passes the raw value through
    /// unchanged so callers that depend on the exact wire bytes can
    /// roundtrip them. Convert to dots-per-inch with
    /// [`Self::dpi_x`].
    ///
    /// `None` for OS/2 `BITMAPCOREHEADER` bitmaps (which pre-date
    /// the resolution fields entirely). `Some(0)` is the documented
    /// "resolution unknown / not specified" sentinel for V3+ and is
    /// surfaced verbatim — it is *not* collapsed to `None` so callers
    /// can distinguish "header doesn't carry the field" from "header
    /// carries the field but the encoder didn't set it".
    pub pixels_per_meter_x: Option<i32>,
    /// Vertical resolution of the target device, in pixels per metre
    /// (`biYPelsPerMeter`). Same shape and conventions as
    /// [`Self::pixels_per_meter_x`].
    pub pixels_per_meter_y: Option<i32>,
    /// Number of palette entries the bitmap actually uses
    /// (`biClrUsed`). Per the BMP spec a value of `0` for an indexed
    /// bitmap means "all `2^biBitCount` colours are used"; this
    /// field passes the raw value through unchanged so callers can
    /// distinguish the sentinel from an explicit count. For
    /// non-indexed depths (16 / 24 / 32 bpp), the value still has a
    /// meaning the spec acknowledges — it specifies the size of the
    /// colour-table-as-display-optimisation-hint that may sit between
    /// the header and the pixel array.
    ///
    /// `None` for OS/2 `BITMAPCOREHEADER` bitmaps (no `biClrUsed`
    /// field).
    pub colors_used: Option<u32>,
    /// Number of palette entries the spec considers "important" for
    /// rendering the bitmap (`biClrImportant`). A value of `0`
    /// means "all colours are important" per the
    /// `BITMAPINFOHEADER` documentation; this field passes the raw
    /// value through unchanged so callers can distinguish that
    /// sentinel from explicit values. `None` for OS/2
    /// `BITMAPCOREHEADER` bitmaps.
    pub colors_important: Option<u32>,
    /// The trailing 24-byte block of a full 64-byte OS/2 2.x
    /// `OS22XBITMAPHEADER` (units / recording / halftoning /
    /// colour-encoding / app-id). `Some` only when the decoded DIB
    /// header was exactly 64 bytes — the full IBM form. `None` for every
    /// Windows header generation, the 12-byte OS/2 1.x
    /// `BITMAPCOREHEADER`, and the truncated OS/2 2.x forms (`biSize`
    /// 16..40) which have no room for these fields.
    pub os2_header2: Option<BmpOs2Header2>,
}

impl BmpMetadata {
    /// Build a [`BmpMetadata`] from a parsed [`DibHeader`] *without*
    /// populating the embedded ICC profile bytes. Used by both the
    /// `decode_bmp_with_metadata` and `decode_dib_with_metadata` paths,
    /// which then fill `icc_profile` from the input slice the caller
    /// passed in.
    pub(crate) fn from_header(header: &DibHeader) -> Self {
        // V4+ headers always carry the cs_type / endpoints / gamma
        // fields (even if the cs_type says "embedded profile", at which
        // point the endpoint + gamma fields are documented as
        // undefined-but-present). V3 / OS/2 leave them empty.
        let color_space = header.cs_type.map(BmpColorSpace::from_raw);
        let rendering_intent = header.intent.map(BmpRenderingIntent::from_raw);
        // V3 (BITMAPINFOHEADER) is the first header generation to carry
        // biXPelsPerMeter / biYPelsPerMeter / biClrUsed / biClrImportant.
        // V4 / V5 inherit the same offsets unchanged. OS/2
        // BITMAPCOREHEADER (12 B) predates all four fields, so the
        // header parser fills DibHeader with zero placeholders there;
        // surface those as `None` to keep "header doesn't carry the
        // field" distinguishable from "header carries the field with
        // value zero".
        let carries_v3_tail = header.header_size >= BITMAPINFOHEADER_SIZE;
        Self {
            header_size: header.header_size,
            color_space,
            endpoints: if header.header_size >= BITMAPV4HEADER_SIZE {
                header.endpoints
            } else {
                None
            },
            gamma_rgb: if header.header_size >= BITMAPV4HEADER_SIZE {
                header.gamma_rgb
            } else {
                None
            },
            rendering_intent,
            profile_data_offset: header.profile_data_offset,
            profile_size: header.profile_size,
            icc_profile: None,
            linked_profile_path: None,
            pixels_per_meter_x: if carries_v3_tail {
                Some(header.x_pels_per_meter)
            } else {
                None
            },
            pixels_per_meter_y: if carries_v3_tail {
                Some(header.y_pels_per_meter)
            } else {
                None
            },
            colors_used: if carries_v3_tail {
                Some(header.clr_used)
            } else {
                None
            },
            colors_important: if carries_v3_tail {
                Some(header.clr_important)
            } else {
                None
            },
            os2_header2: header.os2_header2.map(BmpOs2Header2::from_raw),
        }
    }

    /// Convert [`Self::pixels_per_meter_x`] to dots per inch (DPI),
    /// rounded to the nearest integer. Uses the SI definition of
    /// one inch = 0.0254 metres exactly.
    ///
    /// Returns `None` when the metadata has no horizontal resolution
    /// (OS/2 `BITMAPCOREHEADER`) or when the recorded value is `0`
    /// (the documented "resolution unknown" sentinel for V3+
    /// headers). Negative pixels-per-metre is semantically invalid;
    /// `dpi_x` returns `None` in that case so callers don't see a
    /// nonsensical negative DPI.
    pub fn dpi_x(&self) -> Option<u32> {
        Self::pels_per_meter_to_dpi(self.pixels_per_meter_x?)
    }

    /// Convert [`Self::pixels_per_meter_y`] to dots per inch (DPI),
    /// rounded to the nearest integer. See [`Self::dpi_x`] for the
    /// conventions used.
    pub fn dpi_y(&self) -> Option<u32> {
        Self::pels_per_meter_to_dpi(self.pixels_per_meter_y?)
    }

    /// Shared conversion helper: pixels-per-metre → dots-per-inch,
    /// rounded to the nearest integer using one inch = 0.0254 m.
    /// Returns `None` for the documented "unknown" sentinel (`0`)
    /// and for negative inputs (semantically invalid).
    fn pels_per_meter_to_dpi(pels_per_meter: i32) -> Option<u32> {
        if pels_per_meter <= 0 {
            return None;
        }
        // One inch is 0.0254 metres exactly, so DPI = pels/m * 0.0254.
        // Use integer arithmetic with rounding so the conversion stays
        // bit-exact across platforms: dpi = (pels_per_meter * 254 + 5000) / 10000.
        // The numerator fits in i64 because pels_per_meter is bounded
        // by i32::MAX (~ 2.1e9) and 254 * 2.1e9 ≈ 5.3e11.
        let num = i64::from(pels_per_meter) * 254 + 5000;
        let dpi = num / 10000;
        // dpi is at most i32::MAX * 254 / 10000 ≈ 54.5M, fits in u32.
        Some(dpi as u32)
    }

    /// Typed accessor that returns the V5 ICC profile reference as a
    /// single discriminated [`BmpIccProfileRef`] view.
    ///
    /// Saves callers from matching on `color_space` and then reading
    /// `icc_profile` / `linked_profile_path` / `profile_data_offset` /
    /// `profile_size` by hand: the accessor returns
    /// [`BmpIccProfileRef::Embedded`] / [`BmpIccProfileRef::Linked`]
    /// when the trailing-slot bytes are present, or
    /// [`BmpIccProfileRef::Declared`] when the V5 header declared a
    /// PROFILE_* variant but the trailing-slot reference couldn't be
    /// resolved (truncated file, lying offset, zero size). For every
    /// other colour-space variant (and for V3 / V4 / OS-2 headers
    /// where `color_space` is `None` or non-PROFILE) the accessor
    /// returns [`BmpIccProfileRef::None`].
    pub fn icc_profile_ref(&self) -> BmpIccProfileRef<'_> {
        match self.color_space {
            Some(BmpColorSpace::ProfileEmbedded) => match self.icc_profile.as_deref() {
                Some(bytes) => BmpIccProfileRef::Embedded(bytes),
                None => BmpIccProfileRef::Declared {
                    cs_type: PROFILE_EMBEDDED,
                    profile_data_offset: self.profile_data_offset.unwrap_or(0),
                    profile_size: self.profile_size.unwrap_or(0),
                },
            },
            Some(BmpColorSpace::ProfileLinked) => match self.linked_profile_path.as_deref() {
                Some(bytes) => BmpIccProfileRef::Linked(bytes),
                None => BmpIccProfileRef::Declared {
                    cs_type: PROFILE_LINKED,
                    profile_data_offset: self.profile_data_offset.unwrap_or(0),
                    profile_size: self.profile_size.unwrap_or(0),
                },
            },
            _ => BmpIccProfileRef::None,
        }
    }
}
