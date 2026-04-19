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
pub const BITMAPV4HEADER_SIZE: u32 = 108;
pub const BITMAPV5HEADER_SIZE: u32 = 124;

/// `biCompression` constants we recognise.
pub const BI_RGB: u32 = 0;
pub const BI_RLE8: u32 = 1;
pub const BI_RLE4: u32 = 2;
pub const BI_BITFIELDS: u32 = 3;
pub const BI_JPEG: u32 = 4;
pub const BI_PNG: u32 = 5;

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
