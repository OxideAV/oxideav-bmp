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

/// DIB header generation, discriminated by the `biSize` value stored in
/// the first DWORD of every DIB header.
///
/// The Bitmap Header Types page documents four basic header types —
/// `BITMAPCOREHEADER`, `BITMAPINFOHEADER`, `BITMAPV4HEADER`,
/// `BITMAPV5HEADER` — "differentiated by the Size member, which is the
/// first DWORD in each of the structures". V5 is an extended V4, which
/// is an extended INFO; the CORE header shares only the `Size` member
/// (its body is `WORD`-based and laid out differently). The two
/// Adobe-published intermediates (52 / 56 bytes, surveyed in the staged
/// Wikipedia file-format page) slot between INFO and V4 and share the
/// 40-byte INFO prefix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DibHeaderKind {
    /// 12-byte OS/2 `BITMAPCOREHEADER` — `WORD` width / height, no
    /// compression field, `RGBTRIPLE` colour table.
    Core,
    /// 40-byte `BITMAPINFOHEADER` (the baseline modern header).
    Info,
    /// 52-byte `BITMAPV2INFOHEADER` — INFO + in-header R/G/B masks.
    V2Info,
    /// 56-byte `BITMAPV3INFOHEADER` — V2 + in-header alpha mask.
    V3Info,
    /// 108-byte `BITMAPV4HEADER` — adds the colour-space tail
    /// (`bV4CSType`, endpoints, gamma).
    V4,
    /// 124-byte `BITMAPV5HEADER` — adds rendering intent + ICC profile
    /// offset / size.
    V5,
}

impl DibHeaderKind {
    /// Map a `biSize` value to the header generation it declares.
    /// Returns `None` for sizes that match none of the six known
    /// generations (e.g. the OS/2 2.x 64-byte variant, which the
    /// decoder tolerates by reading only the 40-byte INFO prefix).
    pub fn from_size(size: u32) -> Option<Self> {
        match size {
            BITMAPCOREHEADER_SIZE => Some(Self::Core),
            BITMAPINFOHEADER_SIZE => Some(Self::Info),
            BITMAPV2INFOHEADER_SIZE => Some(Self::V2Info),
            BITMAPV3INFOHEADER_SIZE => Some(Self::V3Info),
            BITMAPV4HEADER_SIZE => Some(Self::V4),
            BITMAPV5HEADER_SIZE => Some(Self::V5),
            _ => None,
        }
    }

    /// On-disk byte size of this header generation (the canonical
    /// `biSize` value).
    pub fn size(&self) -> u32 {
        match self {
            Self::Core => BITMAPCOREHEADER_SIZE,
            Self::Info => BITMAPINFOHEADER_SIZE,
            Self::V2Info => BITMAPV2INFOHEADER_SIZE,
            Self::V3Info => BITMAPV3INFOHEADER_SIZE,
            Self::V4 => BITMAPV4HEADER_SIZE,
            Self::V5 => BITMAPV5HEADER_SIZE,
        }
    }

    /// `true` for every generation that shares the 40-byte
    /// `BITMAPINFOHEADER` field layout as a prefix (INFO / V2 / V3 /
    /// V4 / V5). `false` only for [`DibHeaderKind::Core`], whose
    /// `WORD`-based body has "only the Size member in common with
    /// other bitmap header structures" per the header-types page.
    pub fn has_info_prefix(&self) -> bool {
        !matches!(self, Self::Core)
    }
}

/// Typed view of the 40-byte `BITMAPINFOHEADER` — the eleven-field
/// structure that opens every V3-and-later DIB header.
///
/// On-disk layout (little-endian, no padding), field-for-field from
/// the `BITMAPINFOHEADER (wingdi.h)` structure page:
///
/// ```text
///   off  0  DWORD biSize          — bytes required by the structure; does
///                                   NOT include the colour table or the
///                                   colour masks appended after it
///   off  4  LONG  biWidth         — width in pixels
///   off  8  LONG  biHeight        — height in pixels; positive = bottom-up
///                                   DIB, negative = top-down DIB. Compressed
///                                   formats must use a positive height.
///   off 12  WORD  biPlanes        — "must be set to 1"
///   off 14  WORD  biBitCount      — bits per pixel
///   off 16  DWORD biCompression   — BI_RGB / BI_RLE8 / BI_RLE4 /
///                                   BI_BITFIELDS / BI_JPEG / BI_PNG …
///   off 20  DWORD biSizeImage     — image size in bytes; may be 0 for
///                                   uncompressed RGB bitmaps
///   off 24  LONG  biXPelsPerMeter — horizontal device resolution
///   off 28  LONG  biYPelsPerMeter — vertical device resolution
///   off 32  DWORD biClrUsed       — colour-table entries actually used;
///                                   0 = the 2^biBitCount maximum
///   off 36  DWORD biClrImportant  — important colours; 0 = all
/// ```
///
/// The extended generations (V2 / V3 / V4 / V5) keep these eleven
/// fields at the same offsets and append masks / colour-space fields
/// after them, so this struct doubles as the typed prefix view for any
/// `biSize >= 40` header. The full extended parse (masks, colour-space
/// tail, ICC slot) stays on [`DibHeader`]; this struct is the narrow
/// 1:1 mirror for probe / dispatcher / encoder consumers, the same
/// shape [`BitmapFileHeader`] provides for the 14-byte file header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BitmapInfoHeader {
    /// `biSize` — declared structure size. 40 for the plain INFO
    /// header; 52 / 56 / 108 / 124 for the extended generations that
    /// carry this layout as a prefix. Surfaced as-read.
    pub header_size: u32,
    /// `biWidth` — signed width in pixels. Negative widths are not
    /// meaningful; the decoder rejects them, this view surfaces the
    /// raw value.
    pub width: i32,
    /// `biHeight` — signed height. Positive = bottom-up (origin at the
    /// lower-left corner), negative = top-down (origin upper-left).
    pub height: i32,
    /// `biPlanes` — documented as "must be set to 1". Surfaced raw;
    /// see [`Self::planes_is_valid`].
    pub planes: u16,
    /// `biBitCount` — bits per pixel.
    pub bit_count: u16,
    /// `biCompression` — see [`BI_RGB`], [`BI_RLE8`], [`BI_RLE4`],
    /// [`BI_BITFIELDS`], [`BI_JPEG`], [`BI_PNG`], [`BI_ALPHABITFIELDS`].
    pub compression: u32,
    /// `biSizeImage` — image size in bytes; 0 is legal for
    /// uncompressed RGB bitmaps (the stride formula reconstructs it).
    pub image_size: u32,
    /// `biXPelsPerMeter` — horizontal resolution of the target device,
    /// pixels per metre.
    pub x_pels_per_meter: i32,
    /// `biYPelsPerMeter` — vertical resolution, pixels per metre.
    pub y_pels_per_meter: i32,
    /// `biClrUsed` — number of colour-table entries actually used by
    /// the bitmap. 0 means the full `2^biBitCount` table for indexed
    /// depths.
    pub clr_used: u32,
    /// `biClrImportant` — number of colour indices considered
    /// important; 0 means all colours are important.
    pub clr_important: u32,
}

impl BitmapInfoHeader {
    /// Size of the on-disk structure — always 40 bytes.
    pub const SIZE: usize = BITMAPINFOHEADER_SIZE as usize;

    /// Read the eleven fields out of the first 40 bytes of `buf`
    /// **without** validating the `biSize` discriminator. Returns
    /// `None` if `buf` is shorter than 40 bytes. Probe / fuzz
    /// consumers that want the raw field view use this; callers that
    /// want the `biSize` discrimination enforced use [`Self::parse`].
    pub fn from_bytes(buf: &[u8]) -> Option<Self> {
        if buf.len() < Self::SIZE {
            return None;
        }
        Some(Self {
            header_size: read_u32_le(buf, 0),
            width: read_i32_le(buf, 4),
            height: read_i32_le(buf, 8),
            planes: read_u16_le(buf, 12),
            bit_count: read_u16_le(buf, 14),
            compression: read_u32_le(buf, 16),
            image_size: read_u32_le(buf, 20),
            x_pels_per_meter: read_i32_le(buf, 24),
            y_pels_per_meter: read_i32_le(buf, 28),
            clr_used: read_u32_le(buf, 32),
            clr_important: read_u32_le(buf, 36),
        })
    }

    /// Parse + validate the INFO header at the start of `buf`.
    ///
    /// Validation is the `biSize` discrimination the header-types page
    /// prescribes ("differentiated by the Size member, which is the
    /// first DWORD"):
    ///
    /// * `buf` must hold at least the 40-byte structure;
    /// * `biSize == 12` is rejected with a dedicated message — that's
    ///   the `BITMAPCOREHEADER` layout, whose `WORD`-based body would
    ///   read back as garbage through this struct's field offsets;
    /// * `biSize < 40` (and ≠ 12) is rejected as unsupported;
    /// * `biSize >= 40` is accepted — the extended generations
    ///   (52 / 56 / 108 / 124, plus odd in-the-wild sizes such as the
    ///   OS/2 2.x 64-byte variant) all carry this 40-byte layout as a
    ///   prefix, matching the decoder's leniency.
    ///
    /// Everything else (width sign, planes, bit depth, compression) is
    /// surfaced as-read for the caller to judge — the decoder applies
    /// its own semantic checks downstream.
    pub fn parse(buf: &[u8]) -> crate::error::Result<Self> {
        let h = Self::from_bytes(buf)
            .ok_or_else(|| crate::error::BmpError::invalid("BMP: DIB header truncated"))?;
        if h.header_size == BITMAPCOREHEADER_SIZE {
            return Err(crate::error::BmpError::invalid(
                "BMP: biSize=12 is BITMAPCOREHEADER, not BITMAPINFOHEADER",
            ));
        }
        if h.header_size < BITMAPINFOHEADER_SIZE {
            return Err(crate::error::BmpError::invalid(format!(
                "BMP: unsupported header size {}",
                h.header_size
            )));
        }
        Ok(h)
    }

    /// Render the 40-byte on-disk representation. Inverse of
    /// [`Self::from_bytes`] / [`Self::parse`].
    pub fn to_bytes(&self) -> [u8; Self::SIZE] {
        let mut out = [0u8; Self::SIZE];
        out[0..4].copy_from_slice(&self.header_size.to_le_bytes());
        out[4..8].copy_from_slice(&self.width.to_le_bytes());
        out[8..12].copy_from_slice(&self.height.to_le_bytes());
        out[12..14].copy_from_slice(&self.planes.to_le_bytes());
        out[14..16].copy_from_slice(&self.bit_count.to_le_bytes());
        out[16..20].copy_from_slice(&self.compression.to_le_bytes());
        out[20..24].copy_from_slice(&self.image_size.to_le_bytes());
        out[24..28].copy_from_slice(&self.x_pels_per_meter.to_le_bytes());
        out[28..32].copy_from_slice(&self.y_pels_per_meter.to_le_bytes());
        out[32..36].copy_from_slice(&self.clr_used.to_le_bytes());
        out[36..40].copy_from_slice(&self.clr_important.to_le_bytes());
        out
    }

    /// The header generation `biSize` declares, or `None` when the
    /// size matches no known generation (the decoder still accepts
    /// such headers if `biSize >= 40`, reading the INFO prefix only).
    pub fn kind(&self) -> Option<DibHeaderKind> {
        DibHeaderKind::from_size(self.header_size)
    }

    /// `true` when `biPlanes` carries the documented "must be set
    /// to 1" value. Informational — `parse` does not reject other
    /// values, mirroring the reserved-word treatment on
    /// [`BitmapFileHeader`].
    pub fn planes_is_valid(&self) -> bool {
        self.planes == 1
    }

    /// `true` for a top-down DIB (negative `biHeight`, origin at the
    /// upper-left corner per the structure page).
    pub fn is_top_down(&self) -> bool {
        self.height < 0
    }

    /// Width in pixels with the (illegal) sign stripped.
    pub fn absolute_width(&self) -> u32 {
        self.width.unsigned_abs()
    }

    /// Height in pixels regardless of the bottom-up / top-down sign.
    pub fn absolute_height(&self) -> u32 {
        self.height.unsigned_abs()
    }

    /// Minimum row stride in bytes for an uncompressed bitmap with
    /// these dimensions: the structure page's
    /// `((((biWidth * biBitCount) + 31) & ~31) >> 3)` formula, shared
    /// with [`row_stride`].
    pub fn row_stride(&self) -> usize {
        row_stride(self.absolute_width() as usize, self.bit_count as usize)
    }

    /// Colour-table entry count implied by `biClrUsed` + `biBitCount`
    /// for the indexed depths (1 / 4 / 8 bpp): `biClrUsed` when
    /// non-zero, otherwise the `2^biBitCount` maximum the structure
    /// page documents for a zero `biClrUsed`. Non-indexed depths
    /// return 0 (any optional optimal palette is not consumed by the
    /// pixel pipeline).
    pub fn palette_entries(&self) -> usize {
        if matches!(self.bit_count, 1 | 4 | 8) {
            if self.clr_used == 0 {
                1usize << self.bit_count
            } else {
                self.clr_used as usize
            }
        } else {
            0
        }
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

    // -----------------------------------------------------------------
    // BitmapInfoHeader (typed 40-byte BITMAPINFOHEADER view)
    // -----------------------------------------------------------------

    // A canonical 40-byte BITMAPINFOHEADER: 7×3, bottom-up, 1 plane,
    // 24 bpp, BI_RGB, image_size 0 (legal for uncompressed RGB),
    // 2835 pels/m (~72 DPI) both axes, no colour table.
    fn canonical_info_header() -> BitmapInfoHeader {
        BitmapInfoHeader {
            header_size: BITMAPINFOHEADER_SIZE,
            width: 7,
            height: 3,
            planes: 1,
            bit_count: 24,
            compression: BI_RGB,
            image_size: 0,
            x_pels_per_meter: 2835,
            y_pels_per_meter: 2835,
            clr_used: 0,
            clr_important: 0,
        }
    }

    #[test]
    fn info_header_size_constant_matches_struct_size() {
        assert_eq!(BitmapInfoHeader::SIZE, 40);
        assert_eq!(BitmapInfoHeader::SIZE, BITMAPINFOHEADER_SIZE as usize);
    }

    #[test]
    fn info_header_to_bytes_lays_down_documented_offsets() {
        // Field-for-field check of the on-disk little-endian layout:
        // biSize @0, biWidth @4, biHeight @8, biPlanes @12,
        // biBitCount @14, biCompression @16, biSizeImage @20,
        // biXPelsPerMeter @24, biYPelsPerMeter @28, biClrUsed @32,
        // biClrImportant @36.
        let h = BitmapInfoHeader {
            header_size: 40,
            width: 0x0102_0304,
            height: -2,
            planes: 1,
            bit_count: 32,
            compression: BI_BITFIELDS,
            image_size: 0x0A0B_0C0D,
            x_pels_per_meter: 2835,
            y_pels_per_meter: -2835,
            clr_used: 5,
            clr_important: 7,
        };
        let b = h.to_bytes();
        assert_eq!(&b[0..4], &[40, 0, 0, 0]);
        assert_eq!(&b[4..8], &[0x04, 0x03, 0x02, 0x01]);
        assert_eq!(&b[8..12], &(-2i32).to_le_bytes());
        assert_eq!(&b[12..14], &[1, 0]);
        assert_eq!(&b[14..16], &[32, 0]);
        assert_eq!(&b[16..20], &[3, 0, 0, 0]); // BI_BITFIELDS = 3
        assert_eq!(&b[20..24], &[0x0D, 0x0C, 0x0B, 0x0A]);
        assert_eq!(&b[24..28], &2835i32.to_le_bytes());
        assert_eq!(&b[28..32], &(-2835i32).to_le_bytes());
        assert_eq!(&b[32..36], &[5, 0, 0, 0]);
        assert_eq!(&b[36..40], &[7, 0, 0, 0]);
    }

    #[test]
    fn info_header_from_bytes_roundtrips() {
        let original = canonical_info_header();
        let parsed = BitmapInfoHeader::from_bytes(&original.to_bytes()).unwrap();
        assert_eq!(parsed, original);
        // And the byte-level inverse holds too.
        assert_eq!(parsed.to_bytes(), original.to_bytes());
    }

    #[test]
    fn info_header_from_bytes_too_short_returns_none() {
        // 39 bytes — one short of the documented 40.
        assert!(BitmapInfoHeader::from_bytes(&[0u8; 39]).is_none());
        assert!(BitmapInfoHeader::from_bytes(&[]).is_none());
        // Exactly 40 reads fine (zeroed fields are surfaced raw).
        assert!(BitmapInfoHeader::from_bytes(&[0u8; 40]).is_some());
    }

    #[test]
    fn info_header_from_bytes_ignores_trailing_bytes() {
        // A real DIB has masks / colour table / pixels after byte 40;
        // `from_bytes` only consumes the structure itself.
        let mut buf = Vec::from(canonical_info_header().to_bytes());
        buf.extend_from_slice(&[0xEE; 128]);
        let h = BitmapInfoHeader::from_bytes(&buf).unwrap();
        assert_eq!(h, canonical_info_header());
    }

    #[test]
    fn info_header_parse_canonical_succeeds() {
        let h = BitmapInfoHeader::parse(&canonical_info_header().to_bytes()).unwrap();
        assert_eq!(h.kind(), Some(DibHeaderKind::Info));
        assert!(h.planes_is_valid());
        assert!(!h.is_top_down());
    }

    #[test]
    fn info_header_parse_rejects_short_buffer() {
        let err = BitmapInfoHeader::parse(&[0u8; 20]).unwrap_err();
        match err {
            BmpError::InvalidData(msg) => assert!(msg.contains("DIB header truncated")),
            other => panic!("unexpected error variant: {other:?}"),
        }
    }

    #[test]
    fn info_header_parse_rejects_core_header_size() {
        // biSize = 12 declares the WORD-based BITMAPCOREHEADER layout;
        // reading it through the INFO offsets would produce garbage,
        // so the discrimination must reject it with a pointed message.
        let mut h = canonical_info_header();
        h.header_size = BITMAPCOREHEADER_SIZE;
        let err = BitmapInfoHeader::parse(&h.to_bytes()).unwrap_err();
        match err {
            BmpError::InvalidData(msg) => assert!(msg.contains("BITMAPCOREHEADER")),
            other => panic!("unexpected error variant: {other:?}"),
        }
    }

    #[test]
    fn info_header_parse_rejects_sub_40_sizes() {
        // Anything below 40 that isn't the CORE 12 matches no known
        // generation and can't even hold the declared structure.
        for bad in [0u32, 1, 11, 13, 16, 39] {
            let mut h = canonical_info_header();
            h.header_size = bad;
            let err = BitmapInfoHeader::parse(&h.to_bytes()).unwrap_err();
            match err {
                BmpError::InvalidData(msg) => {
                    assert!(msg.contains("unsupported header size"), "size {bad}: {msg}")
                }
                other => panic!("unexpected error variant: {other:?}"),
            }
        }
    }

    #[test]
    fn info_header_parse_accepts_extended_generation_sizes() {
        // V2 (52) / V3 (56) / V4 (108) / V5 (124) all carry the
        // 40-byte INFO prefix, so the typed prefix parse accepts them.
        for (size, kind) in [
            (BITMAPV2INFOHEADER_SIZE, DibHeaderKind::V2Info),
            (BITMAPV3INFOHEADER_SIZE, DibHeaderKind::V3Info),
            (BITMAPV4HEADER_SIZE, DibHeaderKind::V4),
            (BITMAPV5HEADER_SIZE, DibHeaderKind::V5),
        ] {
            let mut h = canonical_info_header();
            h.header_size = size;
            let parsed = BitmapInfoHeader::parse(&h.to_bytes()).unwrap();
            assert_eq!(parsed.header_size, size);
            assert_eq!(parsed.kind(), Some(kind));
        }
    }

    #[test]
    fn info_header_parse_accepts_unknown_size_at_least_40() {
        // The OS/2 2.x 64-byte variant matches no documented
        // generation; the decoder reads its 40-byte prefix, so the
        // typed parse stays equally lenient — `kind()` reports the
        // non-recognition.
        let mut h = canonical_info_header();
        h.header_size = 64;
        let parsed = BitmapInfoHeader::parse(&h.to_bytes()).unwrap();
        assert_eq!(parsed.kind(), None);
    }

    #[test]
    fn info_header_planes_predicate_is_informational() {
        // biPlanes "must be set to 1" per the structure page, but the
        // typed parse surfaces other values rather than rejecting —
        // semantic checks belong to the decoder.
        let mut h = canonical_info_header();
        h.planes = 3;
        let parsed = BitmapInfoHeader::parse(&h.to_bytes()).unwrap();
        assert_eq!(parsed.planes, 3);
        assert!(!parsed.planes_is_valid());
    }

    #[test]
    fn info_header_top_down_and_absolute_dimensions() {
        let mut h = canonical_info_header();
        h.height = -3;
        assert!(h.is_top_down());
        assert_eq!(h.absolute_height(), 3);
        assert_eq!(h.absolute_width(), 7);
        h.height = 3;
        assert!(!h.is_top_down());
        assert_eq!(h.absolute_height(), 3);
    }

    #[test]
    fn info_header_row_stride_matches_documented_formula() {
        // stride = ((((biWidth * biBitCount) + 31) & ~31) >> 3),
        // per the structure page's surface-stride formula. 7 px at
        // 24 bpp = 21 bytes of pixels → padded to 24.
        let h = canonical_info_header();
        assert_eq!(h.row_stride(), 24);
        let mut h1 = h;
        h1.bit_count = 1;
        assert_eq!(h1.row_stride(), 4); // 7 bits → 1 byte → padded to 4
        let mut h32 = h;
        h32.width = 10;
        h32.bit_count = 32;
        assert_eq!(h32.row_stride(), 40);
    }

    #[test]
    fn info_header_palette_entries_follow_clr_used_rules() {
        // The colour-table remarks: for indexed depths a zero
        // biClrUsed means the 2^biBitCount maximum, non-zero gives the
        // actual entry count; non-indexed depths carry no table the
        // pixel pipeline consumes.
        let mut h = canonical_info_header();
        h.bit_count = 8;
        assert_eq!(h.palette_entries(), 256);
        h.clr_used = 17;
        assert_eq!(h.palette_entries(), 17);
        h.bit_count = 4;
        h.clr_used = 0;
        assert_eq!(h.palette_entries(), 16);
        h.bit_count = 1;
        assert_eq!(h.palette_entries(), 2);
        h.bit_count = 24;
        assert_eq!(h.palette_entries(), 0);
        h.bit_count = 32;
        h.clr_used = 256; // optional optimal palette — not consumed
        assert_eq!(h.palette_entries(), 0);
    }

    #[test]
    fn info_header_matches_storage_doc_worked_example() {
        // The Bitmap Storage page's hexadecimal listing of
        // Redbrick.bmp places the BITMAPINFOHEADER at file bytes
        // 0x0E..0x36: a 32×32, 1-plane, 4-bpp, BI_RGB bitmap with
        // every remaining field zero. Reconstruct those bytes and
        // confirm the typed parse reads the documented values.
        let mut dib = [0u8; 40];
        dib[0] = 0x28; // biSize = 40
        dib[4] = 0x20; // biWidth = 32
        dib[8] = 0x20; // biHeight = 32
        dib[12] = 0x01; // biPlanes = 1
        dib[14] = 0x04; // biBitCount = 4
        let h = BitmapInfoHeader::parse(&dib).unwrap();
        assert_eq!(h.kind(), Some(DibHeaderKind::Info));
        assert_eq!(h.width, 32);
        assert_eq!(h.height, 32);
        assert!(h.planes_is_valid());
        assert_eq!(h.bit_count, 4);
        assert_eq!(h.compression, BI_RGB);
        assert_eq!(h.image_size, 0);
        assert_eq!(h.palette_entries(), 16);
        // 32 px at 4 bpp = 16 bytes/row, already DWORD-aligned. The
        // listing's colour-index array spans 0x76..=0x275 = 512 bytes
        // = 16 × 32 rows.
        assert_eq!(h.row_stride(), 16);
        assert_eq!(h.row_stride() * h.absolute_height() as usize, 512);
    }

    #[test]
    fn info_header_struct_supports_equality_and_copy() {
        let a = canonical_info_header();
        let b = a;
        assert_eq!(a, b);
        let mut c = a;
        c.bit_count = 8;
        assert_ne!(a, c);
    }

    #[test]
    fn dib_header_kind_from_size_covers_all_generations() {
        assert_eq!(DibHeaderKind::from_size(12), Some(DibHeaderKind::Core));
        assert_eq!(DibHeaderKind::from_size(40), Some(DibHeaderKind::Info));
        assert_eq!(DibHeaderKind::from_size(52), Some(DibHeaderKind::V2Info));
        assert_eq!(DibHeaderKind::from_size(56), Some(DibHeaderKind::V3Info));
        assert_eq!(DibHeaderKind::from_size(108), Some(DibHeaderKind::V4));
        assert_eq!(DibHeaderKind::from_size(124), Some(DibHeaderKind::V5));
        for unknown in [0u32, 11, 13, 39, 41, 64, 109, 125, u32::MAX] {
            assert_eq!(DibHeaderKind::from_size(unknown), None, "size {unknown}");
        }
    }

    #[test]
    fn dib_header_kind_size_roundtrips_through_from_size() {
        for kind in [
            DibHeaderKind::Core,
            DibHeaderKind::Info,
            DibHeaderKind::V2Info,
            DibHeaderKind::V3Info,
            DibHeaderKind::V4,
            DibHeaderKind::V5,
        ] {
            assert_eq!(DibHeaderKind::from_size(kind.size()), Some(kind));
        }
    }

    #[test]
    fn dib_header_kind_info_prefix_excludes_core_only() {
        // Per the header-types page, INFO/V4/V5 nest (V5 extends V4
        // extends INFO) while CORE shares only the Size member.
        assert!(!DibHeaderKind::Core.has_info_prefix());
        for kind in [
            DibHeaderKind::Info,
            DibHeaderKind::V2Info,
            DibHeaderKind::V3Info,
            DibHeaderKind::V4,
            DibHeaderKind::V5,
        ] {
            assert!(kind.has_info_prefix(), "{kind:?}");
        }
    }

    #[test]
    fn info_header_decoder_agreement_on_full_bmp() {
        // The typed view and the full decoder must agree on a real
        // file: encode a 3×2 RGBA image, then read the DIB prefix at
        // byte 14 through BitmapInfoHeader and cross-check against
        // what the decode path produced.
        let image = crate::image::BmpImage {
            width: 3,
            height: 2,
            pixel_format: crate::image::BmpPixelFormat::Rgba,
            planes: vec![crate::image::BmpPlane {
                stride: 12,
                data: vec![0x40; 24],
            }],
            palette: None,
            pts: None,
        };
        let (bytes, _) = crate::encoder::encode_bmp(&image).unwrap();
        let h = BitmapInfoHeader::parse(&bytes[BitmapFileHeader::SIZE..]).unwrap();
        assert_eq!(h.kind(), Some(DibHeaderKind::Info));
        assert_eq!(h.width, 3);
        assert_eq!(h.height, 2);
        assert_eq!(h.bit_count, 32);
        assert_eq!(h.compression, BI_RGB);
        assert!(h.planes_is_valid());
        let decoded = crate::decoder::decode_bmp(&bytes).unwrap();
        assert_eq!(decoded.width, h.absolute_width());
        assert_eq!(decoded.height, h.absolute_height());
    }
}
