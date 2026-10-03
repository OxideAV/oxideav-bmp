#![no_main]

//! Metadata-path fuzz target for `oxideav-bmp`.
//!
//! The existing `decode` target feeds arbitrary bytes to `decode` /
//! `decode_dib`. The *metadata* entry points — `BmpMetadata::from_bmp`
//! / `BmpMetadata::from_dib` and the `color` / `metadata` the decoder
//! stamps on the image
//! — are independent public surfaces with their own attacker-controlled
//! offset / slicing maths that the pixel-only path never reaches:
//!
//!   * The V4 colour-space tail: `bV4CSType` (offset 56), a
//!     `CIEXYZTRIPLE` of nine `i32` endpoints (offsets 60..96), and a
//!     three-`u32` gamma triple (offsets 96..108).
//!   * The V5 colour-management tail: `bV5Intent` (offset 108),
//!     `bV5ProfileData` (offset 112), `bV5ProfileSize` (offset 116).
//!   * The trailing ICC / linked-path blob slice: a V5 header that
//!     declares `PROFILE_EMBEDDED` or `PROFILE_LINKED` makes the decoder
//!     slice `input[base + bV5ProfileData ..][.. bV5ProfileSize]` out of
//!     the buffer, where both the offset and the size are fully
//!     attacker-controlled `u32` fields. The slice base differs between
//!     the two entry points (14 bytes for the BMP-file path, 0 for the
//!     header-less DIB path), so the same declared offset reaches
//!     different bytes — exercising both keeps both slicing maths covered.
//!
//! The contract under test is the same panic-free one the other three
//! targets share: every call must *return* a `Result` — a malformed
//! header yields `Err`, a well-formed one yields
//! `Ok((BmpImage, BmpMetadata))` — and neither path may panic,
//! integer-overflow (in a debug build), index out of bounds, or
//! OOM-abort on any input the fuzzer produces. The returned values are
//! intentionally discarded; this harness asserts only that the calls
//! come back.
//!
//! Both header-less DIB modes are fuzzed (`mask = false` plain DIB and
//! `mask = true` for the `.ico` / `.cur` doubled-height XOR+AND layout),
//! mirroring the `decode` target so the metadata path's colour-space
//! tail parsing is exercised under both DIB framings as well as the full
//! BMP-file framing.

use libfuzzer_sys::fuzz_target;
use oxideav_bmp::{decode, decode_dib, BmpMetadata};

fuzz_target!(|data: &[u8]| {
    // Header-only metadata parse (the V5 profile-slot slicing).
    let _ = BmpMetadata::from_bmp(data);
    let _ = BmpMetadata::from_dib(data);
    // The contract view: `color` / `metadata.icc` on the decoded image.
    if let Ok(img) = decode(data) {
        let _ = (img.color, img.metadata.icc.as_ref().map(Vec::len));
    }
    let _ = decode_dib(data, false).map(|img| img.metadata);
    let _ = decode_dib(data, true).map(|img| img.metadata);
});
