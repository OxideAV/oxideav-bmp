//! BMP / DIB structural types.
//!
//! BMP on-disk layout:
//! ```text
//!   [BITMAPFILEHEADER (14 B)]      ─── stripped out for DIB-only variants (ICO)
//!   [BITMAPINFOHEADER (40 B)] ──┐
//!   [optional extra header      ├── "DIB" — the Device-Independent Bitmap
//!    bytes, v2–v5]              │
//!   [optional color masks       │
//!    (BI_BITFIELDS, 12 or 16 B)]│
//!   [optional color table       │
//!    (4 B × num_colors)]        │
//!   [pixel array (bottom-up,    │
//!    4-byte row padding)]      ─┘
//! ```
//!
//! `oxideav-ico` consumes the DIB variant directly — same pixel + palette
//! layout, just without the 14-byte file header and with `height` set to
//! 2× the real height to cover both the XOR mask (actual pixels) and the
//! 1bpp AND mask underneath.

/// `BM` ASCII as a little-endian u16.
pub const BMP_MAGIC: u16 = 0x4D42;

/// Size of `BITMAPFILEHEADER` on disk.
pub const BITMAPFILEHEADER_SIZE: u32 = 14;
/// Size of `BITMAPINFOHEADER` (v3) on disk. Every DIB we handle starts
/// with this; v4/v5 extend it but we only read the extra bytes when the
/// compression field is `BI_BITFIELDS`.
pub const BITMAPINFOHEADER_SIZE: u32 = 40;
/// `BITMAPV2INFOHEADER` (52 B) — Adobe-published intermediate header
/// that extends `BITMAPINFOHEADER` by 12 bytes of in-header R/G/B
/// bit masks (at offsets 40, 44, 48). Documented by Wikipedia as
/// undocumented-by-Microsoft but accepted by some readers; layout
/// matches the V4 / V5 mask-block prefix so a 52-byte
/// `BI_BITFIELDS` header can be read with the same in-header mask
/// reads V4 / V5 already use, just without the colour-space tail
/// that V4 adds at offset 56+.
pub const BITMAPV2INFOHEADER_SIZE: u32 = 52;
/// `BITMAPV3INFOHEADER` (56 B) — extends `BITMAPV2INFOHEADER` by a
/// 4-byte alpha bit mask at offset 52 (same slot V4 / V5 use). Per
/// the Wikipedia survey of header generations, this is the first
/// header to carry an integrated alpha channel; V4 (108 B) and V5
/// (124 B) inherit the same mask layout and add the colour-space
/// tail on top.
pub const BITMAPV3INFOHEADER_SIZE: u32 = 56;
pub const BITMAPV4HEADER_SIZE: u32 = 108;
pub const BITMAPV5HEADER_SIZE: u32 = 124;
/// Size of the OS/2 1.x `BITMAPCOREHEADER` (a.k.a. OS21XBITMAPHEADER).
/// Decoder-only: 12-byte header with `u16` width/height, no compression
/// field, and 3-byte (RGBTRIPLE) colour-table entries.
pub const BITMAPCOREHEADER_SIZE: u32 = 12;

/// `biCompression` constants we recognise.
pub const BI_RGB: u32 = 0;
pub const BI_RLE8: u32 = 1;
pub const BI_RLE4: u32 = 2;
pub const BI_BITFIELDS: u32 = 3;
pub const BI_JPEG: u32 = 4;
pub const BI_PNG: u32 = 5;
/// `BI_ALPHABITFIELDS` — like `BI_BITFIELDS` but four masks (R/G/B/A)
/// follow a V3 `BITMAPINFOHEADER` (16 bytes total) instead of three
/// (12 bytes). Documented for Windows CE 5.0+ and accepted by recent
/// Windows builds. V4/V5 headers already carry all four masks in the
/// header body, so for those header sizes this is treated identically
/// to `BI_BITFIELDS`.
pub const BI_ALPHABITFIELDS: u32 = 6;

// ---------------------------------------------------------------------------
// V4 / V5 colour-space (`bV4CSType` / `bV5CSType`) constants
// ---------------------------------------------------------------------------
//
// The V4 header introduced the `CSType` field at byte offset 56 of the DIB
// header body (offset 70 from the start of the file when paired with a 14
// byte file header). The V5 header inherits the same slot and adds the
// `Intent` / `ProfileData` / `ProfileSize` tail for ICC-managed colour.
//
// On disk the values are little-endian u32 words. Some constants are the
// FOURCC packing of an ASCII tag (`b' B'b'G'b'R'b's'` for sRGB); the
// numeric value below matches the byte ordering Windows GDI documents.

/// `LCS_CALIBRATED_RGB`: the V4 endpoint + gamma fields define the
/// colour space directly (the classic non-ICC variant). Encoded as the
/// integer 0.
pub const LCS_CALIBRATED_RGB: u32 = 0x0000_0000;
/// `LCS_sRGB`: the bitmap is in the sRGB colour space. On-disk bytes
/// are `b'B' b'G' b'R' b's'`, which on a little-endian machine reads
/// back as the u32 value `0x7352_4742`.
pub const LCS_S_RGB: u32 = 0x7352_4742;
/// `LCS_WINDOWS_COLOR_SPACE`: bitmap is in the current Windows default
/// colour space. On-disk bytes are `b' ' b'n' b'i' b'W'`, decoding
/// little-endian to `0x5769_6E20`.
pub const LCS_WINDOWS_COLOR_SPACE: u32 = 0x5769_6E20;
/// `PROFILE_LINKED` (V5 only): the V5 header points at a file path to
/// an external ICC profile via `bV5ProfileData`. On-disk bytes are
/// `b'K' b'N' b'I' b'L'`, little-endian = `0x4C49_4E4B`.
pub const PROFILE_LINKED: u32 = 0x4C49_4E4B;
/// `PROFILE_EMBEDDED` (V5 only): an ICC profile blob follows the
/// pixel array at `BITMAPFILEHEADER_SIZE + bV5ProfileData`, with the
/// length given by `bV5ProfileSize`. On-disk bytes are
/// `b'D' b'E' b'B' b'M'`, little-endian = `0x4D42_4544`.
pub const PROFILE_EMBEDDED: u32 = 0x4D42_4544;

/// V5 rendering intent: saturation (graphics / business charts).
/// Maps to ICC perceptual-intent slot "saturation".
pub const LCS_GM_BUSINESS: u32 = 1;
/// V5 rendering intent: relative colorimetric (proofing).
pub const LCS_GM_GRAPHICS: u32 = 2;
/// V5 rendering intent: perceptual (photographs).
pub const LCS_GM_IMAGES: u32 = 4;
/// V5 rendering intent: absolute colorimetric (match exact colour).
pub const LCS_GM_ABS_COLORIMETRIC: u32 = 8;

/// Parsed `BITMAPINFOHEADER` (plus the extra masks read from a v4/v5
/// header when present). All integer fields kept in their native BMP
/// types so callers can roundtrip byte-for-byte.
#[derive(Debug, Clone, Copy)]
pub struct DibHeader {
    /// Reported header size — 40 for v3, 108 for v4, 124 for v5.
    pub header_size: u32,
    /// Signed width. Negative widths are illegal but we reject them.
    pub width: i32,
    /// Signed height. Positive = bottom-up, negative = top-down.
    /// `absolute_height()` gives the pixel count.
    pub height: i32,
    pub planes: u16,
    /// Bits per pixel: 1 / 4 / 8 / 16 / 24 / 32. (2 is legal but rare
    /// enough we reject it; 0 is BI_JPEG / BI_PNG which we also reject
    /// at the decoder boundary.)
    pub bpp: u16,
    pub compression: u32,
    /// Size of the pixel array in bytes. `0` is allowed for `BI_RGB`
    /// (the spec says the decoder can compute it from row stride × height).
    pub image_size: u32,
    pub x_pels_per_meter: i32,
    pub y_pels_per_meter: i32,
    /// Palette entry count. `0` means `2^bpp` for indexed depths
    /// (1 / 4 / 8) and zero for non-indexed depths.
    pub clr_used: u32,
    pub clr_important: u32,
    /// For `BI_BITFIELDS` these are the R / G / B masks pulled from
    /// the bytes immediately after the info header (v3) or from the
    /// matching fields in a v4/v5 header. `A` is only set when a v4+
    /// header includes an alpha mask.
    pub mask_r: Option<u32>,
    pub mask_g: Option<u32>,
    pub mask_b: Option<u32>,
    pub mask_a: Option<u32>,
    /// V4+ `bV4CSType` / `bV5CSType`. `None` for V3 / OS/2 headers
    /// where the field doesn't exist. See [`LCS_CALIBRATED_RGB`],
    /// [`LCS_S_RGB`], [`LCS_WINDOWS_COLOR_SPACE`], [`PROFILE_LINKED`],
    /// [`PROFILE_EMBEDDED`].
    pub cs_type: Option<u32>,
    /// V4+ `CIEXYZTRIPLE` of red / green / blue endpoints, packed as
    /// 9 × i32 fixed-point (Q2.30 in the documented layout). Always
    /// `None` for V3 / OS/2; populated for V4 / V5 (even if the
    /// `cs_type` says the endpoints aren't authoritative).
    pub endpoints: Option<[i32; 9]>,
    /// V4+ gamma triple (R/G/B), each a u32 fixed-point Q16.16.
    pub gamma_rgb: Option<[u32; 3]>,
    /// V5 `bV5Intent` (rendering intent). `None` for V3 / V4 / OS/2.
    /// Values: 0 = unspecified, [`LCS_GM_BUSINESS`] (saturation),
    /// [`LCS_GM_GRAPHICS`] (relative colorimetric),
    /// [`LCS_GM_IMAGES`] (perceptual),
    /// [`LCS_GM_ABS_COLORIMETRIC`] (absolute).
    pub intent: Option<u32>,
    /// V5 `bV5ProfileData` — offset (from the start of the DIB header)
    /// of an external file path (`PROFILE_LINKED`) or an embedded ICC
    /// profile blob (`PROFILE_EMBEDDED`). `None` for V3 / V4 / OS/2.
    pub profile_data_offset: Option<u32>,
    /// V5 `bV5ProfileSize` — byte length of the profile blob / path
    /// pointed at by [`profile_data_offset`](Self::profile_data_offset).
    /// `None` for V3 / V4 / OS/2.
    pub profile_size: Option<u32>,
}

impl DibHeader {
    pub fn absolute_width(&self) -> u32 {
        self.width.unsigned_abs()
    }
    pub fn absolute_height(&self) -> u32 {
        self.height.unsigned_abs()
    }
    pub fn is_top_down(&self) -> bool {
        self.height < 0
    }
    /// Row stride in bytes — 4-byte aligned per the spec.
    pub fn row_stride(&self) -> usize {
        row_stride(self.absolute_width() as usize, self.bpp as usize)
    }
    /// Palette size in entries. Each entry is 4 bytes (BGRA, though
    /// the alpha byte is always 0 for classic BMP).
    pub fn palette_entries(&self) -> usize {
        if matches!(self.bpp, 1 | 4 | 8) {
            if self.clr_used == 0 {
                1usize << self.bpp
            } else {
                self.clr_used as usize
            }
        } else {
            0
        }
    }
}

/// Row-alignment helper. Rows are padded up to the nearest 4-byte
/// boundary. Extracted so the decoder and encoder agree byte-for-byte.
pub fn row_stride(width_px: usize, bpp: usize) -> usize {
    // Round `(w * bpp)` bits up to the next multiple of 32 bits, then
    // convert to bytes. `div_ceil` keeps clippy happy.
    (width_px * bpp).div_ceil(32) * 4
}

/// Minimum number of little-endian bytes in `buf` starting at `off`
/// needed to read a u32. Small helper that keeps parse sites noise-free.
#[inline]
pub fn read_u32_le(buf: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]])
}
#[inline]
pub fn read_i32_le(buf: &[u8], off: usize) -> i32 {
    i32::from_le_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]])
}
#[inline]
pub fn read_u16_le(buf: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([buf[off], buf[off + 1]])
}
