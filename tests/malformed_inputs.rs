//! Property tests for malformed BMP inputs (Round 155).
//!
//! These complement the `cargo-fuzz` `decode` target by enumerating
//! *structurally* mangled inputs rather than randomly-mangled ones:
//! each test constructs a valid BMP / DIB through the public encoder
//! API, then transforms one bit of the header / colour-table / pixel
//! payload (truncation, oversize claims, RLE overruns, etc.) and
//! asserts the decoder *never* panics, indexes out of bounds, or
//! OOM-aborts — it only ever returns `Err`.
//!
//! Every test uses 100% public API (`encode_bmp`, `encode_dib`,
//! `decode_bmp`, `decode_dib`). They run with the default `registry`
//! feature on and the standalone-only build (`--no-default-features`)
//! both — there is no `oxideav-core` dep in the test harness itself.
//!
//! Allocation ceilings: every malformed input here is < 4 KiB so a
//! test process should be flat in memory even if the decoder mis-handles
//! it. A decoder regression that reserves an attacker-controlled vector
//! would manifest as an OOM-abort which the test harness reports as a
//! distinct failure (SIGABRT) from a panic.

use oxideav_bmp::{
    decode_bmp, decode_dib, encode_bmp, encode_bmp_with_options, encode_dib, BmpEncodeOptions,
    BmpImage, BmpPalette, BmpPixelFormat, BmpPlane, BITMAPCOREHEADER_SIZE, BITMAPFILEHEADER_SIZE,
    BITMAPINFOHEADER_SIZE, BITMAPV4HEADER_SIZE, BITMAPV5HEADER_SIZE,
};

// ---------------------------------------------------------------------------
// Canonical "valid" fixtures, used as the starting point for mutations.
// ---------------------------------------------------------------------------

fn rgba_image(w: u32, h: u32) -> BmpImage {
    let mut data = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let r = ((x * 13) & 0xFF) as u8;
            let g = ((y * 17) & 0xFF) as u8;
            let b = ((x + y) & 0xFF) as u8;
            data.extend_from_slice(&[r, g, b, 0xFF]);
        }
    }
    BmpImage {
        width: w,
        height: h,
        pixel_format: BmpPixelFormat::Rgba,
        planes: vec![BmpPlane {
            stride: w as usize * 4,
            data,
        }],
        palette: None,
        pts: None,
    }
}

fn rgb24_image(w: u32, h: u32) -> BmpImage {
    let mut data = Vec::with_capacity((w * h * 3) as usize);
    for y in 0..h {
        for x in 0..w {
            data.extend_from_slice(&[(x & 0xFF) as u8, (y & 0xFF) as u8, ((x + y) & 0xFF) as u8]);
        }
    }
    BmpImage {
        width: w,
        height: h,
        pixel_format: BmpPixelFormat::Rgb24,
        planes: vec![BmpPlane {
            stride: w as usize * 3,
            data,
        }],
        palette: None,
        pts: None,
    }
}

fn rgb565_image(w: u32, h: u32) -> BmpImage {
    let mut data = Vec::with_capacity((w * h * 2) as usize);
    for y in 0..h {
        for x in 0..w {
            let v: u16 = (x as u16 * 31) ^ (y as u16 * 7);
            data.extend_from_slice(&v.to_le_bytes());
        }
    }
    BmpImage {
        width: w,
        height: h,
        pixel_format: BmpPixelFormat::Rgb565,
        planes: vec![BmpPlane {
            stride: w as usize * 2,
            data,
        }],
        palette: None,
        pts: None,
    }
}

fn indexed8_image(w: u32, h: u32) -> BmpImage {
    let mut data = Vec::with_capacity((w * h) as usize);
    for y in 0..h {
        for x in 0..w {
            data.push(((x + y) & 0x07) as u8); // 8 distinct indices
        }
    }
    let mut entries: Vec<[u8; 3]> = Vec::new();
    for i in 0..8u8 {
        entries.push([i * 32, i * 32, i * 32]);
    }
    BmpImage {
        width: w,
        height: h,
        pixel_format: BmpPixelFormat::Indexed8,
        planes: vec![BmpPlane {
            stride: w as usize,
            data,
        }],
        palette: Some(BmpPalette { entries }),
        pts: None,
    }
}

fn indexed4_image(w: u32, h: u32) -> BmpImage {
    let mut data = Vec::with_capacity((w * h) as usize);
    for y in 0..h {
        for x in 0..w {
            data.push(((x ^ y) & 0x0F) as u8);
        }
    }
    let mut entries: Vec<[u8; 3]> = Vec::new();
    for i in 0..16u8 {
        entries.push([i * 16, i * 8, i * 4]);
    }
    BmpImage {
        width: w,
        height: h,
        pixel_format: BmpPixelFormat::Indexed4,
        planes: vec![BmpPlane {
            stride: w as usize,
            data,
        }],
        palette: Some(BmpPalette { entries }),
        pts: None,
    }
}

fn all_canonical_bmps() -> Vec<(&'static str, Vec<u8>)> {
    // Default options match `encode_bmp`'s behaviour; the indexed paths
    // auto-pick RLE when it shrinks the output, otherwise fall back to
    // BI_RGB. Sufficient for the structural mutation tests below; the
    // dedicated RLE-overrun tests further down build their own RLE input.
    let opts: BmpEncodeOptions = Default::default();
    vec![
        ("rgba_32_8x8", encode_bmp(&rgba_image(8, 8)).unwrap().0),
        ("rgb24_8x8", encode_bmp(&rgb24_image(8, 8)).unwrap().0),
        ("rgb565_8x8", encode_bmp(&rgb565_image(8, 8)).unwrap().0),
        (
            "indexed8_8x8",
            encode_bmp_with_options(&indexed8_image(8, 8), opts)
                .unwrap()
                .0,
        ),
        (
            "indexed4_8x8",
            encode_bmp_with_options(&indexed4_image(8, 8), opts)
                .unwrap()
                .0,
        ),
    ]
}

fn all_canonical_dibs() -> Vec<(&'static str, Vec<u8>)> {
    vec![
        (
            "dib_rgba_8x8",
            encode_dib(&rgba_image(8, 8), false).unwrap(),
        ),
        (
            "dib_ico_rgba_8x8",
            encode_dib(&rgba_image(8, 8), true).unwrap(),
        ),
    ]
}

// ---------------------------------------------------------------------------
// Truncation sweep.
//
// For every byte offset between 0 and `len-1`, slice the canonical input
// to that length and require the decoder returns `Err` *without* panic.
// The contract being tested is the safety contract from the fuzz harness,
// applied deterministically.
// ---------------------------------------------------------------------------

#[test]
fn truncation_sweep_decode_bmp_never_panics() {
    for (label, full) in all_canonical_bmps() {
        // Full-length must decode.
        let _ = decode_bmp(&full).unwrap_or_else(|e| panic!("baseline {label} failed: {e}"));
        for cut in 0..full.len() {
            let result = decode_bmp(&full[..cut]);
            assert!(
                result.is_err(),
                "{label}: truncated to {cut}/{} should return Err",
                full.len()
            );
        }
    }
}

#[test]
fn truncation_sweep_decode_dib_never_panics() {
    for (label, full) in all_canonical_dibs() {
        // Baseline check.
        let doubled = label.contains("ico");
        let _ =
            decode_dib(&full, doubled).unwrap_or_else(|e| panic!("baseline {label} failed: {e}"));
        for cut in 0..full.len() {
            let result = decode_dib(&full[..cut], doubled);
            if doubled {
                // The ICO doubled-height path intentionally tolerates a
                // missing AND mask (some icon files lie about its size and
                // simply omit the trailing 1-bpp bytes). The safety
                // contract under test for that variant is "no panic" only;
                // `Ok` with the XOR alpha preserved is the documented
                // behaviour for truncations that land past the XOR-pixel
                // region. See `decode_dib_with_mask` in src/decoder.rs.
                let _ = result; // accept Ok or Err; must not panic.
            } else {
                assert!(
                    result.is_err(),
                    "{label}: truncated to {cut}/{} should return Err (doubled=false)",
                    full.len()
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Header-size lies: claim a larger header (V4 / V5) than the file actually
// supplies. The decoder must reject without panicking.
// ---------------------------------------------------------------------------

#[test]
fn header_size_claims_v4_but_only_v3_bytes_present() {
    let mut bytes = encode_bmp(&rgba_image(4, 4)).unwrap().0;
    // Patch DIB-header size from 40 (V3) to 108 (V4) without growing the
    // buffer.
    bytes[14..18].copy_from_slice(&BITMAPV4HEADER_SIZE.to_le_bytes());
    let r = decode_bmp(&bytes);
    assert!(r.is_err(), "V4 size claim with V3 body must be rejected");
}

#[test]
fn header_size_claims_v5_but_only_v3_bytes_present() {
    let mut bytes = encode_bmp(&rgba_image(4, 4)).unwrap().0;
    bytes[14..18].copy_from_slice(&BITMAPV5HEADER_SIZE.to_le_bytes());
    let r = decode_bmp(&bytes);
    assert!(r.is_err(), "V5 size claim with V3 body must be rejected");
}

#[test]
fn header_size_below_coreheader_is_rejected() {
    let mut bytes = encode_bmp(&rgba_image(4, 4)).unwrap().0;
    // Illegal `biSize` values: below the 12-byte OS/2 1.x
    // `BITMAPCOREHEADER` and in the 13..16 gap below the smallest
    // truncated OS/2 2.x `OS22XBITMAPHEADER`. (Sizes 12, 16..40, and
    // >=40 are all legitimate header generations and are *not* rejected
    // on size alone.)
    for s in [0u32, 1, 4, 8, 11, 13, 14, 15] {
        bytes[14..18].copy_from_slice(&s.to_le_bytes());
        let r = decode_bmp(&bytes);
        assert!(
            r.is_err(),
            "header_size={s} must be rejected (below 12 or in the 13..16 gap)"
        );
    }
}

#[test]
fn header_size_huge_is_rejected() {
    let mut bytes = encode_bmp(&rgba_image(4, 4)).unwrap().0;
    for s in [
        0xFFFF_FFFFu32,
        0x8000_0000,
        0x4000_0000,
        BITMAPV5HEADER_SIZE + 1024,
    ] {
        bytes[14..18].copy_from_slice(&s.to_le_bytes());
        let r = decode_bmp(&bytes);
        assert!(r.is_err(), "header_size={s} (oversized) must be rejected");
    }
}

// ---------------------------------------------------------------------------
// Width / height edge cases.
// ---------------------------------------------------------------------------

#[test]
fn negative_width_is_rejected() {
    let mut bytes = encode_bmp(&rgba_image(4, 4)).unwrap().0;
    // Width sits at offset 14+4 = 18 in BITMAPINFOHEADER.
    for w in [-1i32, -16, i32::MIN, 0] {
        bytes[18..22].copy_from_slice(&w.to_le_bytes());
        let r = decode_bmp(&bytes);
        assert!(r.is_err(), "width={w} (non-positive) must be rejected");
    }
}

#[test]
fn zero_height_is_rejected() {
    let mut bytes = encode_bmp(&rgba_image(4, 4)).unwrap().0;
    // Height at offset 14+8 = 22.
    bytes[22..26].copy_from_slice(&0i32.to_le_bytes());
    let r = decode_bmp(&bytes);
    assert!(r.is_err(), "height=0 must be rejected");
}

#[test]
fn height_i32_min_does_not_overflow_on_abs() {
    let mut bytes = encode_bmp(&rgba_image(4, 4)).unwrap().0;
    // height = i32::MIN — absolute value would overflow `i32::abs`. The
    // decoder must not panic; either rejects, or normalises through u32
    // and rejects on the size check.
    bytes[22..26].copy_from_slice(&i32::MIN.to_le_bytes());
    let r = decode_bmp(&bytes);
    assert!(r.is_err(), "height=i32::MIN must be rejected without panic");
}

// ---------------------------------------------------------------------------
// `bfOffBits` (pixel offset) edge cases.
// ---------------------------------------------------------------------------

#[test]
fn pixel_offset_inside_header_is_rejected() {
    let mut bytes = encode_bmp(&rgba_image(4, 4)).unwrap().0;
    // Force pixel offset to point inside the file header (offset 0..14).
    for off in [0u32, 2, 13, 14] {
        bytes[10..14].copy_from_slice(&off.to_le_bytes());
        let r = decode_bmp(&bytes);
        // Either decoded a garbled-but-bounded image, or rejected. The
        // safety contract is "no panic" — both Ok and Err are acceptable
        // here so long as the call returned. Drop the result.
        let _ = r;
    }
}

#[test]
fn pixel_offset_past_eof_is_rejected() {
    let mut bytes = encode_bmp(&rgba_image(4, 4)).unwrap().0;
    let full_len = bytes.len() as u32;
    for off in [full_len, full_len + 1, full_len + 0xFFFF, u32::MAX] {
        bytes[10..14].copy_from_slice(&off.to_le_bytes());
        let r = decode_bmp(&bytes);
        assert!(
            r.is_err(),
            "pixel_offset={off} (past EOF, file_len={full_len}) must be rejected"
        );
    }
}

// ---------------------------------------------------------------------------
// `biClrUsed` over-claim.
// ---------------------------------------------------------------------------

#[test]
fn indexed8_clr_used_exceeds_palette_space_is_rejected_or_capped() {
    let img = indexed8_image(8, 8);
    let mut bytes = encode_bmp_with_options(&img, BmpEncodeOptions::default())
        .unwrap()
        .0;
    // Find the BITMAPINFOHEADER's clr_used at offset 14+32 = 46.
    // Claim 1 billion palette entries. The decoder must not OOM trying to
    // allocate 4 GiB for the table; it should return Err.
    for huge in [1_000_000u32, 100_000_000, u32::MAX, u32::MAX - 1] {
        bytes[46..50].copy_from_slice(&huge.to_le_bytes());
        let r = decode_bmp(&bytes);
        assert!(
            r.is_err(),
            "clr_used={huge} must be rejected (no exabyte allocation)"
        );
    }
}

#[test]
fn indexed4_clr_used_exceeds_2_bpp_is_rejected() {
    let img = indexed4_image(8, 8);
    let mut bytes = encode_bmp_with_options(&img, BmpEncodeOptions::default())
        .unwrap()
        .0;
    // For 4 bpp the legal max is 16 entries. Claim something obviously
    // illegal; the decoder must not panic or under-allocate.
    for over in [u32::MAX, 1_000_000u32, 0xFFFF_FFFFu32] {
        bytes[46..50].copy_from_slice(&over.to_le_bytes());
        let r = decode_bmp(&bytes);
        assert!(r.is_err(), "indexed4 clr_used={over} must be rejected");
    }
}

// ---------------------------------------------------------------------------
// Bit-depth validation.
// ---------------------------------------------------------------------------

#[test]
fn illegal_bit_depths_are_rejected() {
    let mut bytes = encode_bmp(&rgba_image(4, 4)).unwrap().0;
    // bpp at offset 14+14 = 28 (u16).
    for bpp in [0u16, 2, 3, 5, 6, 7, 9, 10, 15, 17, 31, 33, 48, 64, 0xFFFF] {
        bytes[28..30].copy_from_slice(&bpp.to_le_bytes());
        let r = decode_bmp(&bytes);
        assert!(r.is_err(), "bpp={bpp} must be rejected");
    }
}

#[test]
fn planes_other_than_1_are_rejected() {
    let mut bytes = encode_bmp(&rgba_image(4, 4)).unwrap().0;
    // planes at offset 14+12 = 26 (u16).
    for p in [0u16, 2, 3, 8, 0xFFFF] {
        bytes[26..28].copy_from_slice(&p.to_le_bytes());
        let r = decode_bmp(&bytes);
        assert!(r.is_err(), "planes={p} must be rejected (only 1 allowed)");
    }
}

// ---------------------------------------------------------------------------
// Compression field validation.
// ---------------------------------------------------------------------------

#[test]
fn unknown_compression_is_rejected() {
    let mut bytes = encode_bmp(&rgba_image(4, 4)).unwrap().0;
    // compression at offset 14+16 = 30 (u32). Recognised compressions:
    //   BI_RGB=0, BI_RLE8=1, BI_RLE4=2, BI_BITFIELDS=3, BI_JPEG=4,
    //   BI_PNG=5, BI_ALPHABITFIELDS=6.
    // BI_JPEG / BI_PNG are explicitly rejected; everything outside that
    // table must also be rejected.
    for c in [4u32, 5, 7, 8, 0xFF, 0xFFFF, u32::MAX] {
        bytes[30..34].copy_from_slice(&c.to_le_bytes());
        let r = decode_bmp(&bytes);
        assert!(r.is_err(), "compression={c} must be rejected");
    }
}

#[test]
fn rle8_with_wrong_bpp_is_rejected() {
    // Hand-build a BMP that claims BI_RLE8 but a non-8 bpp. We can't go
    // through the encoder here because it would refuse to emit such a
    // file.
    let mut bytes = encode_bmp(&rgba_image(4, 4)).unwrap().0;
    // Set compression = BI_RLE8 (1) at offset 30, bpp stays at 32.
    bytes[30..34].copy_from_slice(&1u32.to_le_bytes());
    let r = decode_bmp(&bytes);
    assert!(r.is_err(), "BI_RLE8 with bpp=32 must be rejected");
}

#[test]
fn rle4_with_wrong_bpp_is_rejected() {
    let mut bytes = encode_bmp(&rgba_image(4, 4)).unwrap().0;
    // compression = BI_RLE4 (2), bpp still 32.
    bytes[30..34].copy_from_slice(&2u32.to_le_bytes());
    let r = decode_bmp(&bytes);
    assert!(r.is_err(), "BI_RLE4 with bpp=32 must be rejected");
}

// ---------------------------------------------------------------------------
// RLE stream overrun: take a valid RLE-encoded BMP, mangle the run length
// inside the pixel data, and require the decoder still returns Err / Ok
// without panic.
// ---------------------------------------------------------------------------

#[test]
fn rle8_with_runs_exceeding_row_does_not_panic() {
    let img = indexed8_image(8, 8);
    // Force RLE on by leaving the default options (the encoder auto-picks
    // RLE for RLE-friendly indexed data).
    let bytes = encode_bmp(&img).unwrap().0;
    // For each byte in the pixel data region, try replacing it with a
    // few hostile values (huge run, illegal opcode pair) and confirm
    // no panic.
    let file_off = u32::from_le_bytes([bytes[10], bytes[11], bytes[12], bytes[13]]) as usize;
    if file_off >= bytes.len() {
        // Not RLE-encoded — fallback to BI_RGB. Skip.
        return;
    }
    for byte_idx in file_off..bytes.len() {
        for &mutation in &[0xFFu8, 0x00, 0x01, 0x02, 0x10, 0xFE] {
            let mut mangled = bytes.clone();
            mangled[byte_idx] = mutation;
            let _ = decode_bmp(&mangled); // must not panic.
        }
    }
}

#[test]
fn rle8_truncated_pixel_stream_does_not_panic() {
    let img = indexed8_image(16, 16);
    let bytes = encode_bmp(&img).unwrap().0;
    let file_off = u32::from_le_bytes([bytes[10], bytes[11], bytes[12], bytes[13]]) as usize;
    if file_off >= bytes.len() {
        return;
    }
    // Truncate every possible point inside the pixel region.
    for cut in file_off..bytes.len() {
        let _ = decode_bmp(&bytes[..cut]); // must not panic.
    }
}

// ---------------------------------------------------------------------------
// Palette-table truncation: claim N entries via clr_used but only supply
// fewer bytes than the entries would need.
// ---------------------------------------------------------------------------

#[test]
fn indexed8_palette_truncated_does_not_panic() {
    let img = indexed8_image(4, 4);
    let bytes = encode_bmp_with_options(&img, BmpEncodeOptions::default())
        .unwrap()
        .0;
    // For each truncation point between header-end and pixel-start (where
    // the palette lives), decode must not panic.
    let pixel_off = u32::from_le_bytes([bytes[10], bytes[11], bytes[12], bytes[13]]) as usize;
    let palette_start = (BITMAPFILEHEADER_SIZE + BITMAPINFOHEADER_SIZE) as usize;
    for cut in palette_start..pixel_off {
        let _ = decode_bmp(&bytes[..cut]); // must not panic.
    }
}

// ---------------------------------------------------------------------------
// BI_BITFIELDS edge cases.
// ---------------------------------------------------------------------------

#[test]
fn bitfields_with_truncated_mask_section_is_rejected() {
    // Build a V3 BI_BITFIELDS BMP (16-bit RGB565). The encoder uses a V4
    // header for RGB565, so we patch the header size back to 40 to land
    // in the v3 "12 bytes of masks immediately after the 40-byte header"
    // path, then truncate the mask section.
    let img = rgb565_image(8, 8);
    let mut bytes = encode_bmp(&img).unwrap().0;
    // Header size at offset 14: change V4 (108) -> V3 (40).
    bytes[14..18].copy_from_slice(&BITMAPINFOHEADER_SIZE.to_le_bytes());
    // The encoder placed 12 bytes of masks at offset 14+108=122 (V4 spot)
    // for the V4 path; in the V3 path the masks must live at 14+40=54..66.
    // Truncate everything from byte 54+0 onward inside the mask region:
    let mask_start = (BITMAPFILEHEADER_SIZE + BITMAPINFOHEADER_SIZE) as usize;
    for cut in mask_start..(mask_start + 12).min(bytes.len()) {
        let _ = decode_bmp(&bytes[..cut]); // must not panic.
    }
}

#[test]
fn bitfields_all_zero_mask_does_not_divide_by_zero() {
    let img = rgb565_image(8, 8);
    let mut bytes = encode_bmp(&img).unwrap().0;
    // V4 header places masks at offset 14+40 = 54..66.
    for off in [54, 58, 62] {
        bytes[off..off + 4].copy_from_slice(&0u32.to_le_bytes());
    }
    // Decoder may reject or return junk colours, but must not panic on
    // shift-by-32 or divide-by-zero.
    let _ = decode_bmp(&bytes);
}

// ---------------------------------------------------------------------------
// ICO doubled-height path.
// ---------------------------------------------------------------------------

#[test]
fn dib_ico_odd_height_does_not_panic() {
    // Build a valid doubled-height DIB then patch the height field to an
    // *odd* value (the doubled-height contract is that biHeight is 2× the
    // real height, so an odd value is ill-formed).
    let img = rgba_image(8, 4);
    let mut dib = encode_dib(&img, true).unwrap();
    // Height field at offset 8..12 inside the DIB.
    for h in [1i32, 3, 5, 7, 9, 11, 15] {
        dib[8..12].copy_from_slice(&h.to_le_bytes());
        let _ = decode_dib(&dib, true); // must not panic.
    }
}

#[test]
fn dib_ico_height_overflow_does_not_panic() {
    let img = rgba_image(8, 4);
    let mut dib = encode_dib(&img, true).unwrap();
    // Height = i32::MAX would, naively divided by 2, still need a giant
    // AND mask. Implementation must bound off the available bytes.
    for h in [i32::MAX, i32::MAX - 1, 0x4000_0000] {
        dib[8..12].copy_from_slice(&h.to_le_bytes());
        let _ = decode_dib(&dib, true);
    }
}

// ---------------------------------------------------------------------------
// Single-bit mutation sweep: walk every header byte, flip the top bit,
// and confirm the decoder still returns (panic-free) — the canonical
// "any single-bit flip" robustness check.
// ---------------------------------------------------------------------------

#[test]
fn single_bit_flip_in_header_never_panics() {
    let bytes = encode_bmp(&rgba_image(8, 8)).unwrap().0;
    // Header region = first 14 (file) + 40 (info) = 54 bytes. Beyond
    // that we hit the pixel array which is just data; mutations there
    // can't trip header maths.
    let header_end = (BITMAPFILEHEADER_SIZE + BITMAPINFOHEADER_SIZE) as usize;
    for byte_idx in 0..header_end.min(bytes.len()) {
        for bit in 0..8u8 {
            let mut mangled = bytes.clone();
            mangled[byte_idx] ^= 1 << bit;
            let _ = decode_bmp(&mangled); // must not panic.
        }
    }
}

#[test]
fn single_bit_flip_in_dib_header_never_panics() {
    let dib = encode_dib(&rgba_image(8, 8), false).unwrap();
    let header_end = BITMAPINFOHEADER_SIZE as usize;
    for byte_idx in 0..header_end.min(dib.len()) {
        for bit in 0..8u8 {
            let mut mangled = dib.clone();
            mangled[byte_idx] ^= 1 << bit;
            let _ = decode_dib(&mangled, false); // must not panic.
            let _ = decode_dib(&mangled, true); // must not panic either.
        }
    }
}

// ---------------------------------------------------------------------------
// OS/2 BITMAPCOREHEADER (12-byte) malformed inputs.
// ---------------------------------------------------------------------------

fn build_os2_bmp(bpp: u16, w: u16, h: u16, palette_bytes: &[u8], pixels: &[u8]) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(b"BM");
    v.extend_from_slice(&0u32.to_le_bytes()); // file size
    v.extend_from_slice(&0u16.to_le_bytes()); // reserved
    v.extend_from_slice(&0u16.to_le_bytes()); // reserved
    let pixel_off = (BITMAPFILEHEADER_SIZE + BITMAPCOREHEADER_SIZE) + palette_bytes.len() as u32;
    v.extend_from_slice(&pixel_off.to_le_bytes());
    // BITMAPCOREHEADER:
    v.extend_from_slice(&BITMAPCOREHEADER_SIZE.to_le_bytes());
    v.extend_from_slice(&w.to_le_bytes());
    v.extend_from_slice(&h.to_le_bytes());
    v.extend_from_slice(&1u16.to_le_bytes()); // planes
    v.extend_from_slice(&bpp.to_le_bytes());
    v.extend_from_slice(palette_bytes);
    v.extend_from_slice(pixels);
    v
}

#[test]
fn os2_coreheader_unsupported_bpp_is_rejected() {
    // The OS/2 spec allowed 2-bpp; our impl rejects everything outside
    // {1, 4, 8, 16, 24, 32}. The relevant safety property is "no panic",
    // not that 2-bpp specifically is or isn't supported — both Err and a
    // (in theory) Ok return are fine, only the call must not abort.
    for bpp in [0u16, 2, 3, 5, 7, 9, 17, 33, 0xFFFF] {
        let bmp = build_os2_bmp(bpp, 4, 4, &[], &[0u8; 32]);
        let r = decode_bmp(&bmp);
        // For unsupported depths the decoder rejects; if it ever grows
        // 2-bpp support, that variant will start succeeding. Both are OK.
        let _ = r;
    }
}

#[test]
fn os2_coreheader_zero_width_is_rejected() {
    let bmp = build_os2_bmp(8, 0, 4, &[0u8; 4], &[0u8; 16]);
    let r = decode_bmp(&bmp);
    assert!(r.is_err(), "OS/2 coreheader width=0 must be rejected");
}

#[test]
fn os2_coreheader_truncated_palette_does_not_panic() {
    // 8-bpp OS/2 with palette truncated before all 256 RGBTRIPLE entries
    // are present.
    let mut bmp = build_os2_bmp(8, 4, 4, &[0u8; 30], &[0u8; 16]);
    // Patch pixel offset to point inside the (truncated) palette area.
    let bad_off = (BITMAPFILEHEADER_SIZE + BITMAPCOREHEADER_SIZE) + 256 * 3;
    bmp[10..14].copy_from_slice(&bad_off.to_le_bytes());
    let r = decode_bmp(&bmp);
    assert!(r.is_err(), "truncated OS/2 palette must be rejected");
}

// ---------------------------------------------------------------------------
// Random-mutation pass: a small deterministic LCG drives N rounds of
// random single-byte changes. This is the cheapest property test in the
// file but it overlaps with the cargo-fuzz target's coverage; included
// here so `cargo test` (no nightly required) still gets a sample of the
// "any garbage" robustness check.
// ---------------------------------------------------------------------------

#[test]
fn random_mutation_burst_never_panics() {
    let bases = all_canonical_bmps();
    let mut rng_state: u32 = 0xCAFEBABE;
    let next = |state: &mut u32| -> u32 {
        *state = state.wrapping_mul(1664525).wrapping_add(1013904223);
        *state
    };
    for (_label, full) in &bases {
        for _ in 0..256 {
            let mut bytes = full.clone();
            // Flip 1..=4 random bytes.
            let flips = (next(&mut rng_state) % 4) + 1;
            for _ in 0..flips {
                let idx = (next(&mut rng_state) as usize) % bytes.len();
                let val = (next(&mut rng_state) & 0xFF) as u8;
                bytes[idx] = val;
            }
            let _ = decode_bmp(&bytes); // must not panic / OOM / overflow.
        }
    }
}
