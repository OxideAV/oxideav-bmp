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
    DibHeader, BITMAPV4HEADER_SIZE, LCS_CALIBRATED_RGB, LCS_GM_ABS_COLORIMETRIC, LCS_GM_BUSINESS,
    LCS_GM_GRAPHICS, LCS_GM_IMAGES, LCS_S_RGB, LCS_WINDOWS_COLOR_SPACE, PROFILE_EMBEDDED,
    PROFILE_LINKED,
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

/// Header-derived colour-space metadata for a decoded BMP.
///
/// V3 / OS/2 headers don't carry any of this; the decoder fills every
/// field with `None` / the sentinel zeroes when it sees one of those.
/// V4 fills `color_space` / `endpoints` / `gamma_rgb`. V5 additionally
/// fills `rendering_intent`; for the [`BmpColorSpace::ProfileEmbedded`]
/// variant the embedded ICC profile bytes are pulled into
/// [`Self::icc_profile`].
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
        }
    }
}
