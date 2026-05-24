#![no_main]

//! Decode arbitrary fuzz-supplied bytes through the BMP decoder. The
//! decoder must always return a `Result` and never panic / abort / OOM,
//! regardless of how malformed the input is.
//!
//! The contract under test is purely that the calls *return*: a malformed
//! stream yields `Err(BmpError::…)`, a well-formed one yields
//! `Ok(BmpImage)`, and neither path may panic, integer-overflow (in a
//! debug build), index out of bounds, or pre-allocate an attacker-claimed
//! `width * height * 4` (or RLE) pixel buffer that exceeds what the input
//! could possibly back. The return values are intentionally discarded.
//!
//! Three entry points are fuzzed off the same input bytes because they
//! are independent public surfaces with distinct offset / allocation
//! maths:
//!
//!   * [`decode_bmp`]              — full file: `BM` signature + 14-byte
//!     BITMAPFILEHEADER + DIB + pixels, with `bfOffBits` (the pixel
//!     offset) read from the file header.
//!   * [`decode_dib`] (mask=false) — header-less DIB: pixel offset is
//!     *computed* from the header + bitfield masks + colour-table size,
//!     so `clr_used` drives the arithmetic directly.
//!   * [`decode_dib`] (mask=true)  — the `.ico` / `.cur` doubled-height
//!     XOR+AND layout, which halves the height and walks a trailing
//!     1bpp AND mask.

use libfuzzer_sys::fuzz_target;
use oxideav_bmp::{decode_bmp, decode_dib};

fuzz_target!(|data: &[u8]| {
    let _ = decode_bmp(data);
    let _ = decode_dib(data, false);
    let _ = decode_dib(data, true);
});
