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

/// Typed view of the 14-byte `BITMAPFILEHEADER` that prefixes every
/// real `.bmp` file (the headerless DIB variant consumed by `.ico`
/// strips this off — see [`crate::decode_dib`]).
///
/// On-disk layout (little-endian, no padding):
///
/// ```text
///   off 0  WORD   bfType        — must equal [`BMP_MAGIC`] (`0x4D42` = ASCII "BM")
///   off 2  DWORD  bfSize        — file size in bytes (may legally be 0; some
///                                 writers leave it blank and the field is
///                                 never authoritative — the real file end
///                                 is whichever happens first between the
///                                 reported size and the actual byte stream)
///   off 6  WORD   bfReserved1   — reserved, "must be zero" per the docs but
///                                 surviving files in the wild leak random
///                                 bytes here; we surface the raw value.
///   off 8  WORD   bfReserved2   — same as `bfReserved1`.
///   off 10 DWORD  bfOffBits     — byte offset from the start of the file to
///                                 the first byte of the pixel array.
/// ```
///
/// The struct is a faithful 1:1 mirror of the on-disk fields. Callers
/// that want validated semantics use [`BitmapFileHeader::parse`] which
/// checks the magic + buffer length; for raw byte-pump uses the
/// [`BitmapFileHeader::from_bytes`] entry point reads the five fields
/// without enforcing anything else.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BitmapFileHeader {
    /// `bfType` — the two-byte signature, always `0x4D42` (`"BM"`) on
    /// a real BMP. Returned as-read so a caller debugging a malformed
    /// file can see what was actually there.
    pub file_type: u16,
    /// `bfSize` — file size in bytes per the writer. Treated as
    /// informational only by the decoder; some writers leave it zero.
    pub file_size: u32,
    /// `bfReserved1` — should be zero. Surfaced raw.
    pub reserved1: u16,
    /// `bfReserved2` — should be zero. Surfaced raw.
    pub reserved2: u16,
    /// `bfOffBits` — byte offset from the start of the file to the
    /// pixel array. The DIB header (and any masks / palette) live
    /// between byte 14 and byte `pixel_offset`.
    pub pixel_offset: u32,
}

impl BitmapFileHeader {
    /// Size of the on-disk header — always 14 bytes.
    pub const SIZE: usize = BITMAPFILEHEADER_SIZE as usize;

    /// Read the five header fields out of the first 14 bytes of `buf`
    /// **without** validating the magic. Returns `None` if `buf` is
    /// shorter than 14 bytes so callers that want to use this in
    /// fuzz / probe paths don't have to do the length check themselves.
    pub fn from_bytes(buf: &[u8]) -> Option<Self> {
        if buf.len() < Self::SIZE {
            return None;
        }
        Some(Self {
            file_type: read_u16_le(buf, 0),
            file_size: read_u32_le(buf, 2),
            reserved1: read_u16_le(buf, 6),
            reserved2: read_u16_le(buf, 8),
            pixel_offset: read_u32_le(buf, 10),
        })
    }

    /// Parse + validate the file header at the start of `buf`. Checks
    /// the buffer length and the [`BMP_MAGIC`] signature; everything
    /// else is surfaced as-is (the reserved words, `file_size`, and
    /// `pixel_offset` are caller-validated against the rest of the
    /// stream).
    ///
    /// Returns `Err` if `buf` is shorter than [`Self::SIZE`] or the
    /// `bfType` field is not `0x4D42`. The error messages match the
    /// strings the inline parse used historically so external matchers
    /// keep working.
    pub fn parse(buf: &[u8]) -> crate::error::Result<Self> {
        let h = Self::from_bytes(buf)
            .ok_or_else(|| crate::error::BmpError::invalid("BMP: input shorter than header"))?;
        if h.file_type != BMP_MAGIC {
            return Err(crate::error::BmpError::invalid(
                "BMP: missing 'BM' signature",
            ));
        }
        Ok(h)
    }

    /// `true` when the `bfType` field carries the canonical `"BM"`
    /// signature. Returns `false` for the three OS/2-era alternates
    /// (`BA` array, `CI` colour icon, `CP` colour pointer, `IC` icon,
    /// `PT` pointer) — none of which our decoder handles, but a
    /// caller probing a multi-image OS/2 archive can branch on this.
    pub fn has_canonical_magic(&self) -> bool {
        self.file_type == BMP_MAGIC
    }

    /// `true` when the reserved field words are both zero, matching
    /// the documented "must be zero" requirement. Many real-world
    /// writers leave these dirty; this helper is purely informational
    /// and the decoder does not reject non-zero values.
    pub fn reserved_is_clean(&self) -> bool {
        self.reserved1 == 0 && self.reserved2 == 0
    }

    /// Render the 14-byte on-disk representation. Inverse of
    /// [`Self::from_bytes`] / [`Self::parse`]; encoder paths can call
    /// this to lay down a deterministic header rather than open-coding
    /// the byte writes.
    pub fn to_bytes(&self) -> [u8; Self::SIZE] {
        let mut out = [0u8; Self::SIZE];
        out[0..2].copy_from_slice(&self.file_type.to_le_bytes());
        out[2..6].copy_from_slice(&self.file_size.to_le_bytes());
        out[6..8].copy_from_slice(&self.reserved1.to_le_bytes());
        out[8..10].copy_from_slice(&self.reserved2.to_le_bytes());
        out[10..14].copy_from_slice(&self.pixel_offset.to_le_bytes());
        out
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::BmpError;

    // A canonical 14-byte BITMAPFILEHEADER for a 14 + 40 + 4-byte-pixel
    // toy BMP: BM, file_size = 0x12_34_56_78 (arbitrary), reserved
    // words zero, pixel offset = 14 + 40 = 54 (0x36).
    fn canonical_header_bytes() -> [u8; 14] {
        [
            b'B', b'M', // bfType
            0x78, 0x56, 0x34, 0x12, // bfSize = 0x1234_5678
            0x00, 0x00, // bfReserved1
            0x00, 0x00, // bfReserved2
            0x36, 0x00, 0x00, 0x00, // bfOffBits = 54
        ]
    }

    #[test]
    fn file_header_size_constant_matches_struct_size() {
        // The on-disk header is exactly 14 bytes — both the public
        // const and the associated SIZE must agree.
        assert_eq!(BitmapFileHeader::SIZE, 14);
        assert_eq!(BITMAPFILEHEADER_SIZE, 14);
        assert_eq!(BitmapFileHeader::SIZE, BITMAPFILEHEADER_SIZE as usize);
    }

    #[test]
    fn file_header_from_bytes_canonical() {
        let h = BitmapFileHeader::from_bytes(&canonical_header_bytes()).unwrap();
        assert_eq!(h.file_type, 0x4D42);
        assert_eq!(h.file_size, 0x1234_5678);
        assert_eq!(h.reserved1, 0);
        assert_eq!(h.reserved2, 0);
        assert_eq!(h.pixel_offset, 54);
    }

    #[test]
    fn file_header_from_bytes_too_short_returns_none() {
        // 13 bytes — one short of the documented 14. None, not a
        // panic, not a corrupt read.
        let buf = [0u8; 13];
        assert!(BitmapFileHeader::from_bytes(&buf).is_none());
        // 0-length buffer also gracefully None.
        assert!(BitmapFileHeader::from_bytes(&[]).is_none());
    }

    #[test]
    fn file_header_from_bytes_accepts_exact_14_bytes() {
        // No requirement for tail bytes — exactly 14 must parse.
        let h = BitmapFileHeader::from_bytes(&canonical_header_bytes()).unwrap();
        assert_eq!(h.pixel_offset, 54);
    }

    #[test]
    fn file_header_from_bytes_ignores_trailing_bytes() {
        // A real file has the DIB header and pixels after byte 14;
        // `from_bytes` only consumes the first 14.
        let mut buf = Vec::from(canonical_header_bytes());
        buf.extend_from_slice(&[0xFF; 200]);
        let h = BitmapFileHeader::from_bytes(&buf).unwrap();
        assert_eq!(h.file_type, BMP_MAGIC);
        assert_eq!(h.pixel_offset, 54);
    }

    #[test]
    fn file_header_from_bytes_reads_reserved_words_verbatim() {
        // The "must be zero" requirement is not enforced — the typed
        // accessor must surface the raw bytes so a caller debugging a
        // dirty writer can see them.
        let mut bytes = canonical_header_bytes();
        bytes[6] = 0xCA;
        bytes[7] = 0xFE;
        bytes[8] = 0xBE;
        bytes[9] = 0xEF;
        let h = BitmapFileHeader::from_bytes(&bytes).unwrap();
        assert_eq!(h.reserved1, 0xFECA);
        assert_eq!(h.reserved2, 0xEFBE);
    }

    #[test]
    fn file_header_from_bytes_accepts_non_canonical_magic() {
        // `from_bytes` is the unchecked variant; a `BA` array header
        // (which a multi-image OS/2 archive would carry) reads as-is.
        let mut bytes = canonical_header_bytes();
        bytes[0] = b'B';
        bytes[1] = b'A';
        let h = BitmapFileHeader::from_bytes(&bytes).unwrap();
        assert_eq!(h.file_type, 0x4142); // 'BA' little-endian
        assert!(!h.has_canonical_magic());
    }

    #[test]
    fn file_header_parse_canonical_succeeds() {
        let h = BitmapFileHeader::parse(&canonical_header_bytes()).unwrap();
        assert_eq!(h.file_type, BMP_MAGIC);
        assert_eq!(h.pixel_offset, 54);
        assert!(h.has_canonical_magic());
    }

    #[test]
    fn file_header_parse_rejects_short_buffer() {
        let buf = [0u8; 8];
        let err = BitmapFileHeader::parse(&buf).unwrap_err();
        match err {
            BmpError::InvalidData(msg) => assert!(msg.contains("shorter than header")),
            other => panic!("unexpected error variant: {other:?}"),
        }
    }

    #[test]
    fn file_header_parse_rejects_wrong_magic() {
        // 'MZ' (DOS .exe signature) is a common misroute. The parse
        // path must reject it before any caller starts trusting the
        // remaining fields.
        let mut bytes = canonical_header_bytes();
        bytes[0] = b'M';
        bytes[1] = b'Z';
        let err = BitmapFileHeader::parse(&bytes).unwrap_err();
        match err {
            BmpError::InvalidData(msg) => assert!(msg.contains("'BM' signature")),
            other => panic!("unexpected error variant: {other:?}"),
        }
    }

    #[test]
    fn file_header_parse_rejects_all_zero_buffer() {
        // Garbage / zeroed input is the exact case the magic check
        // exists for. Must be rejected.
        let buf = [0u8; 14];
        assert!(BitmapFileHeader::parse(&buf).is_err());
    }

    #[test]
    fn file_header_parse_rejects_zero_byte_at_offset_one() {
        // 'B' then a garbage byte instead of 'M'. Some malformed
        // writers truncate the signature; the parse path must catch
        // this rather than treat the file as half-valid.
        let mut bytes = canonical_header_bytes();
        bytes[1] = 0x00;
        assert!(BitmapFileHeader::parse(&bytes).is_err());
    }

    #[test]
    fn file_header_parse_rejects_os2_array_signature() {
        // `BA` is the OS/2 array-of-bitmaps signature. We don't
        // handle the array container, so the parse path must reject
        // (rather than silently fall through into a DIB read).
        let mut bytes = canonical_header_bytes();
        bytes[1] = b'A';
        assert!(BitmapFileHeader::parse(&bytes).is_err());
    }

    #[test]
    fn file_header_has_canonical_magic_distinguishes_alt_sigs() {
        let mut h = BitmapFileHeader::from_bytes(&canonical_header_bytes()).unwrap();
        assert!(h.has_canonical_magic());
        // Any non-BM value flips it false.
        h.file_type = 0;
        assert!(!h.has_canonical_magic());
        h.file_type = 0x4142; // 'BA'
        assert!(!h.has_canonical_magic());
        h.file_type = 0x4943; // 'CI'
        assert!(!h.has_canonical_magic());
    }

    #[test]
    fn file_header_reserved_is_clean_for_canonical() {
        let h = BitmapFileHeader::from_bytes(&canonical_header_bytes()).unwrap();
        assert!(h.reserved_is_clean());
    }

    #[test]
    fn file_header_reserved_is_clean_detects_dirty_writers() {
        let mut bytes = canonical_header_bytes();
        bytes[6] = 0xFF;
        let h = BitmapFileHeader::from_bytes(&bytes).unwrap();
        assert!(!h.reserved_is_clean());
        // Either reserved word being non-zero flips the predicate.
        let mut bytes = canonical_header_bytes();
        bytes[9] = 0x01;
        let h = BitmapFileHeader::from_bytes(&bytes).unwrap();
        assert!(!h.reserved_is_clean());
    }

    #[test]
    fn file_header_to_bytes_roundtrips() {
        // Build → from_bytes → to_bytes → must be byte-identical.
        let original = canonical_header_bytes();
        let h = BitmapFileHeader::from_bytes(&original).unwrap();
        assert_eq!(h.to_bytes(), original);
    }

    #[test]
    fn file_header_to_bytes_roundtrips_with_dirty_reserved() {
        let mut bytes = canonical_header_bytes();
        bytes[6] = 0x12;
        bytes[7] = 0x34;
        bytes[8] = 0x56;
        bytes[9] = 0x78;
        let h = BitmapFileHeader::from_bytes(&bytes).unwrap();
        assert_eq!(h.to_bytes(), bytes);
    }

    #[test]
    fn file_header_to_bytes_emits_canonical_layout() {
        // A freshly-constructed header at standard sizes must lay
        // down the canonical 14-byte sequence encoders rely on.
        let h = BitmapFileHeader {
            file_type: BMP_MAGIC,
            file_size: 14 + 40 + 4,
            reserved1: 0,
            reserved2: 0,
            pixel_offset: 14 + 40,
        };
        let bytes = h.to_bytes();
        assert_eq!(&bytes[0..2], b"BM");
        // Total = 58 = 0x3A
        assert_eq!(&bytes[2..6], &[0x3A, 0x00, 0x00, 0x00]);
        assert_eq!(&bytes[6..10], &[0, 0, 0, 0]);
        // Pixel offset = 54 = 0x36
        assert_eq!(&bytes[10..14], &[0x36, 0x00, 0x00, 0x00]);
    }

    #[test]
    fn file_header_pixel_offset_above_4gb_roundtrips_as_u32() {
        // bfOffBits is u32. A pathological writer can store the full
        // 32-bit range; the typed parser must surface it verbatim
        // (the decoder validates it against the actual buffer length
        // separately).
        let mut bytes = canonical_header_bytes();
        bytes[10] = 0xFF;
        bytes[11] = 0xFF;
        bytes[12] = 0xFF;
        bytes[13] = 0xFF;
        let h = BitmapFileHeader::from_bytes(&bytes).unwrap();
        assert_eq!(h.pixel_offset, 0xFFFF_FFFF);
        // Round-tripping preserves the high-bit pattern.
        assert_eq!(h.to_bytes()[10..14], [0xFF, 0xFF, 0xFF, 0xFF]);
    }

    #[test]
    fn file_header_file_size_zero_is_legal() {
        // Some writers (notably old Paint variants) leave bfSize as 0.
        // The typed parser surfaces it as-is and `parse` does not
        // reject — only the magic + buffer length are validated.
        let mut bytes = canonical_header_bytes();
        bytes[2] = 0;
        bytes[3] = 0;
        bytes[4] = 0;
        bytes[5] = 0;
        let h = BitmapFileHeader::parse(&bytes).unwrap();
        assert_eq!(h.file_size, 0);
    }

    #[test]
    fn file_header_struct_supports_equality_and_copy() {
        // Confirms the derived bounds we documented in the doc-comment.
        let a = BitmapFileHeader::from_bytes(&canonical_header_bytes()).unwrap();
        let b = a;
        assert_eq!(a, b);
        let mut c = a;
        c.pixel_offset = 999;
        assert_ne!(a, c);
    }

    #[test]
    fn file_header_buffer_at_minimum_full_bmp_length_parses() {
        // The smallest legal complete BMP is 14 + 12 (CORE header)
        // bytes. Confirm the file-header parse succeeds at the
        // minimum sensible total buffer length.
        let mut buf = vec![0u8; 14 + 12];
        buf[0] = b'B';
        buf[1] = b'M';
        buf[10] = 26; // bfOffBits = end-of-header
        let h = BitmapFileHeader::parse(&buf).unwrap();
        assert_eq!(h.pixel_offset, 26);
    }
}
