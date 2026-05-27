#![no_main]

//! Focused fuzz target for the BI_RLE8 / BI_RLE4 state machines.
//!
//! The existing `decode` target feeds arbitrary bytes into the full
//! `decode_bmp` surface, which means a large fraction of every fuzz
//! iteration is spent satisfying the `BM` signature, the 14-byte
//! BITMAPFILEHEADER, and the 40-byte BITMAPINFOHEADER before the
//! fuzzer ever reaches the RLE pixel payload. That dilutes coverage of
//! the RLE state machines themselves — the two `decode_rle{8,4}`
//! routines in `decoder.rs` that walk a stream of:
//!
//!   * encoded runs (`count, index`),
//!   * the EOL / EOB / delta escapes (`0x00, 0x00 / 0x01 / 0x02 dx dy`),
//!   * absolute mode (`0x00, n>=3, n bytes, optional word-pad`).
//!
//! Past `cargo-fuzz` runs already shook out a handful of header-driven
//! DoS paths (RLE wall-clock allocation, `bpp == 0` colour-table
//! oversize, `biClrUsed` blowup); the open question is whether the RLE
//! cursor / delta / absolute-mode arithmetic itself is panic-free under
//! every byte sequence. This harness narrows the input so the fuzzer
//! spends all of its budget on that question.
//!
//! ## Wire framing
//!
//! Each fuzz input is interpreted as `(header_byte, width_byte,
//! height_byte, rle_payload...)`:
//!
//!   * `header_byte` low bit picks `BI_RLE8` (0) vs `BI_RLE4` (1).
//!     The remaining bits are unused.
//!   * `width_byte`  → `width  = max(1, byte)`  (1..=255 px).
//!   * `height_byte` → `height = max(1, byte)`  (1..=255 px).
//!   * remaining bytes become the on-disk RLE pixel array.
//!
//! The harness then assembles a syntactically valid BMP around the
//! payload: signature + file header + v3 BITMAPINFOHEADER claiming the
//! chosen compression / bpp / dimensions + a maximal colour table
//! (256 × `RGBQUAD` for RLE8, 16 × `RGBQUAD` for RLE4) so palette
//! lookups never go out of bounds for any byte the fuzzer feeds. The
//! result is fed through the public `decode_bmp` exactly like a real
//! file would be, so any panic / index OOB / debug-mode integer
//! overflow / OOM-abort surfaces as a crash.
//!
//! Inputs shorter than 3 bytes have no payload to fuzz so are returned
//! immediately; the framing-only edge case is already covered by the
//! generic `decode` target.

use libfuzzer_sys::fuzz_target;
use oxideav_bmp::decode_bmp;

const BITMAPFILEHEADER_SIZE: usize = 14;
const BITMAPINFOHEADER_SIZE: usize = 40;
const BI_RLE8: u32 = 1;
const BI_RLE4: u32 = 2;

fn put_u16(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn put_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn put_i32(out: &mut Vec<u8>, v: i32) {
    out.extend_from_slice(&v.to_le_bytes());
}

/// Build a BMP whose pixel payload is `rle_payload`, declaring the
/// given compression / bit-depth / dimensions and carrying a maximal
/// colour table. Returns the on-wire bytes.
fn build_bmp(compression: u32, bpp: u16, width: u32, height: u32, rle_payload: &[u8]) -> Vec<u8> {
    let palette_entries: u32 = match bpp {
        4 => 16,
        8 => 256,
        _ => 0,
    };
    let palette_bytes: u32 = palette_entries * 4;
    let pixel_offset: u32 =
        BITMAPFILEHEADER_SIZE as u32 + BITMAPINFOHEADER_SIZE as u32 + palette_bytes;
    let file_size: u32 = pixel_offset.saturating_add(rle_payload.len() as u32);

    let mut out: Vec<u8> = Vec::with_capacity(file_size as usize);
    // BITMAPFILEHEADER (14 B).
    out.extend_from_slice(b"BM");
    put_u32(&mut out, file_size);
    put_u16(&mut out, 0);
    put_u16(&mut out, 0);
    put_u32(&mut out, pixel_offset);
    // BITMAPINFOHEADER (40 B).
    put_u32(&mut out, BITMAPINFOHEADER_SIZE as u32);
    put_i32(&mut out, width as i32);
    put_i32(&mut out, height as i32);
    put_u16(&mut out, 1); // planes
    put_u16(&mut out, bpp);
    put_u32(&mut out, compression);
    put_u32(&mut out, rle_payload.len() as u32); // biSizeImage
    put_i32(&mut out, 2835); // x ppm (72 dpi)
    put_i32(&mut out, 2835); // y ppm
    put_u32(&mut out, 0); // biClrUsed = 0 sentinel
    put_u32(&mut out, 0); // biClrImportant

    // Maximal colour table — every palette index the fuzzer can name
    // (0..=255 for RLE8, 0..=15 for RLE4) resolves to a real entry, so
    // a missing entry never short-circuits the RLE walk.
    for i in 0..palette_entries {
        let b = (i & 0xFF) as u8;
        out.extend_from_slice(&[b, b ^ 0x55, b ^ 0xAA, 0]);
    }
    out.extend_from_slice(rle_payload);
    out
}

fuzz_target!(|data: &[u8]| {
    if data.len() < 3 {
        return;
    }
    let header_byte = data[0];
    let width = data[1].max(1) as u32;
    let height = data[2].max(1) as u32;
    let payload = &data[3..];

    let (compression, bpp) = if header_byte & 1 == 0 {
        (BI_RLE8, 8u16)
    } else {
        (BI_RLE4, 4u16)
    };

    let bmp = build_bmp(compression, bpp, width, height, payload);
    let _ = decode_bmp(&bmp);
});
