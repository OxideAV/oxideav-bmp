#![no_main]

//! Structured DIB-header forge target.
//!
//! The generic `decode` target feeds raw bytes to the decoder, which
//! means the fuzzer has to *discover* the `BM` signature, a plausible
//! `bfOffBits`, and a known `biSize` before any mutation reaches the
//! deep header-validation matrix (dimension sanity, bpp × compression
//! pairing, colour-table maths, mask parsing, colour-space tail,
//! doubled-height ICO layout). This harness spends the entire budget
//! *inside* that matrix instead: the fuzzer's bytes become raw DIB
//! header *fields* wrapped in always-well-formed framing.
//!
//! ## Wire framing of the fuzz input
//!
//!   * byte 0 — header-generation selector (`% 8`):
//!     0 = Core (12 B), 1 = Info (40 B), 2 = V2 (52 B), 3 = V3 (56 B),
//!     4 = V4 (108 B), 5 = V5 (124 B), 6 = OS/2 2.x (64 B),
//!     7 = arbitrary `biSize` pulled from the body bytes (exercises the
//!     unknown-/truncated-OS/2-size tolerance paths, 16..64).
//!   * byte 1 — file-magic selector (`% 8`): 0..=5 map to the six
//!     recognised signatures (`BM` weighted at 0 and 6..=7 so most
//!     iterations get past the magic check), the rest take the raw
//!     word from bytes 2..4.
//!   * bytes 2..4 — `bfReserved` bytes (or the raw magic word for the
//!     arbitrary-magic case).
//!   * bytes 4..6 — little-endian u16 delta added to the honest
//!     `bfOffBits` (14 + declared header size), wrapping. Small values
//!     keep the pixel offset near-plausible; large ones push it past
//!     EOF, before the header, or into wrap-around territory.
//!   * bytes 6.. — the DIB header *body* (everything after `biSize`),
//!     colour table, masks, and pixel array, verbatim. The harness
//!     writes only `biSize` itself; every other field — width, height,
//!     planes, bpp, compression, `biClrUsed`, channel masks, CSType,
//!     endpoints, gamma, intent, `bV5ProfileData` / `bV5ProfileSize` —
//!     comes straight from the fuzzer. The body may be shorter than
//!     the declared `biSize` (truncated-header handling) or longer
//!     (the tail doubles as colour table + pixels).
//!
//! Each forged file is pushed through all six public parse surfaces so
//! the same header bytes exercise the BMP-file offset maths, the
//! header-less DIB offset maths, the doubled-height ICO layout, and
//! the metadata colour-space / ICC-slicing tail:
//!
//!   * `decode_bmp` + `decode_bmp_with_metadata` (full file),
//!   * `decode_dib` / `decode_dib_with_metadata` with `mask = false`,
//!   * `decode_dib` / `decode_dib_with_metadata` with `mask = true`.
//!
//! Contract: every call returns a `Result`. No panic, no
//! index-out-of-bounds, no integer overflow (debug assertions are on
//! in fuzz builds), no attacker-sized allocation. Return values are
//! intentionally discarded.

use libfuzzer_sys::fuzz_target;
use oxideav_bmp::{decode_bmp, decode_bmp_with_metadata, decode_dib, decode_dib_with_metadata};

/// Declared `biSize` for each generation selector. Selector 7 reads an
/// arbitrary size from the body instead.
fn header_size_for(selector: u8, body: &[u8]) -> (u32, usize) {
    match selector {
        0 => (12, 0),
        1 => (40, 0),
        2 => (52, 0),
        3 => (56, 0),
        4 => (108, 0),
        5 => (124, 0),
        6 => (64, 0),
        _ => {
            if body.len() >= 4 {
                (u32::from_le_bytes([body[0], body[1], body[2], body[3]]), 4)
            } else {
                (0, 0)
            }
        }
    }
}

/// Two-byte file signature for the magic selector. Weighted so five of
/// the eight codes produce the canonical `BM` — the OS/2 container
/// signatures and the arbitrary word are rejected at the very first
/// check, so they only need enough weight to keep that path covered.
fn magic_for(selector: u8, raw: [u8; 2]) -> [u8; 2] {
    match selector {
        1 => *b"BA",
        2 => *b"CI",
        3 => *b"CP",
        4 => *b"IC",
        5 => *b"PT",
        6 => raw,
        _ => *b"BM",
    }
}

fuzz_target!(|data: &[u8]| {
    if data.len() < 6 {
        return;
    }
    let gen_selector = data[0] % 8;
    let magic_selector = data[1] % 8;
    let reserved = [data[2], data[3]];
    let off_delta = u16::from_le_bytes([data[4], data[5]]) as u32;
    let body = &data[6..];

    let (dib_size, consumed) = header_size_for(gen_selector, body);
    let body = &body[consumed..];

    // Header-less DIB image: declared biSize + fuzzer-supplied fields.
    let mut dib = Vec::with_capacity(4 + body.len());
    dib.extend_from_slice(&dib_size.to_le_bytes());
    dib.extend_from_slice(body);

    // Full BMP file: 14-byte BITMAPFILEHEADER + the DIB above. bfSize is
    // written honestly; bfOffBits is the honest pixel offset plus a
    // fuzzer-chosen wrapping delta.
    let magic = magic_for(magic_selector, reserved);
    let file_len = (14 + dib.len()) as u32;
    let off_bits = 14u32.wrapping_add(dib_size).wrapping_add(off_delta);
    let mut bmp = Vec::with_capacity(14 + dib.len());
    bmp.extend_from_slice(&magic);
    bmp.extend_from_slice(&file_len.to_le_bytes());
    bmp.push(reserved[0]);
    bmp.push(reserved[1]);
    bmp.push(0);
    bmp.push(0);
    bmp.extend_from_slice(&off_bits.to_le_bytes());
    bmp.extend_from_slice(&dib);

    let _ = decode_bmp(&bmp);
    let _ = decode_bmp_with_metadata(&bmp);
    let _ = decode_dib(&dib, false);
    let _ = decode_dib(&dib, true);
    let _ = decode_dib_with_metadata(&dib, false);
    let _ = decode_dib_with_metadata(&dib, true);
});
