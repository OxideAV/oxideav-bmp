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

//!
//! Contract surface (IMAGE_CRATE_API): `probe` is total, `info` is
//! header-only, `decode` / `decode_with` return the native layout and
//! `decode_rgb8` / `decode_rgba8` exercise every `to_rgb8` / `to_rgba8`
//! kernel on whatever the decoder produced. A tight `DecodeOptions`
//! limit set runs alongside the defaults so the limit checks see the
//! same hostile headers.

use libfuzzer_sys::fuzz_target;
use oxideav_bmp::{
    decode, decode_dib, decode_dib_with, decode_rgb8, decode_rgba8, decode_with, info, probe,
    DecodeOptions,
};

fuzz_target!(|data: &[u8]| {
    let _ = probe(data);
    let _ = info(data);
    if let Ok(img) = decode(data) {
        let rgba = img.to_rgba8();
        assert_eq!(rgba.len(), img.width as usize * img.height as usize * 4);
        let rgb = img.to_rgb8();
        assert_eq!(rgb.len(), img.width as usize * img.height as usize * 3);
    }
    let tight = DecodeOptions::default()
        .with_max_width(512u32)
        .with_max_height(512u32)
        .with_max_pixels(1u64 << 16)
        .with_max_bytes(1u64 << 20)
        .with_strict(true);
    let _ = decode_with(data, &tight);
    let _ = decode_rgb8(data);
    let _ = decode_rgba8(data);
    let _ = decode_dib(data, false);
    let _ = decode_dib(data, true);
    let _ = decode_dib_with(data, true, &tight);
});
