//! Adversarial tests: hostile bitfield masks, integer-overflow header
//! probes, and RLE4-focused stream attacks (Round 383).
//!
//! Complements `tests/malformed_inputs.rs` (structural mutations of
//! encoder output) and `tests/hostile_metadata.rs` (the V4/V5 tail)
//! with three families the earlier suites only graze:
//!
//!   * **mask matrices** — every single-bit R/G/B/A mask position at
//!     16 and 32 bpp, plus overlapping / non-contiguous / all-ones /
//!     wider-than-bpp masks, through the raw `BI_BITFIELDS` and
//!     `BI_ALPHABITFIELDS` paths and the V4 in-header mask slots;
//!   * **integer-overflow probes** — header fields chosen so naive
//!     `width × height`, `width × bpp`, `offset + size`, or
//!     `clr_used × 4` arithmetic wraps (i32 / u32 / usize), asserting
//!     the decoder returns fast without panicking or allocating the
//!     claimed buffer;
//!   * **RLE4 stream attacks** — delta-escape cursor overflows,
//!     absolute-mode row overruns, and a full truncation × byte-value
//!     mutation grid over a hand-built RLE4 payload (the existing RLE
//!     tests are RLE8-heavy).
//!
//! Everything is built from public API + raw byte assembly; the
//! decoder contract is "return a `Result`, never panic, never
//! allocate what the input cannot back".

use oxideav_bmp::types::{BI_RLE4, BI_RLE8};
use oxideav_bmp::{
    decode_bmp, decode_dib, encode_bmp, BmpImage, BmpPixelFormat, BmpPlane, BITMAPFILEHEADER_SIZE,
    BITMAPINFOHEADER_SIZE, BITMAPV4HEADER_SIZE, BI_ALPHABITFIELDS, BI_BITFIELDS,
};

// ---------------------------------------------------------------------------
// Raw-file assembly helpers
// ---------------------------------------------------------------------------

/// Assemble a complete BMP file from raw header fields: 14-byte file
/// header + 40-byte BITMAPINFOHEADER + `tail` (mask section, palette,
/// pixel array — whatever the test wants after the header). `off_bits`
/// is written verbatim so tests can lie about the pixel offset.
#[allow(clippy::too_many_arguments)]
fn raw_bmp_v3(
    width: i32,
    height: i32,
    bpp: u16,
    compression: u32,
    size_image: u32,
    clr_used: u32,
    off_bits: u32,
    tail: &[u8],
) -> Vec<u8> {
    let file_len = (BITMAPFILEHEADER_SIZE + BITMAPINFOHEADER_SIZE) as usize + tail.len();
    let mut out = Vec::with_capacity(file_len);
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&(file_len as u32).to_le_bytes());
    out.extend_from_slice(&[0u8; 4]);
    out.extend_from_slice(&off_bits.to_le_bytes());
    out.extend_from_slice(&BITMAPINFOHEADER_SIZE.to_le_bytes());
    out.extend_from_slice(&width.to_le_bytes());
    out.extend_from_slice(&height.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // planes
    out.extend_from_slice(&bpp.to_le_bytes());
    out.extend_from_slice(&compression.to_le_bytes());
    out.extend_from_slice(&size_image.to_le_bytes());
    out.extend_from_slice(&0i32.to_le_bytes()); // x ppm
    out.extend_from_slice(&0i32.to_le_bytes()); // y ppm
    out.extend_from_slice(&clr_used.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // clr important
    out.extend_from_slice(tail);
    out
}

/// The honest pixel offset for a V3 file whose mask/palette section is
/// `pre_pixel` bytes long.
fn v3_off_bits(pre_pixel: usize) -> u32 {
    BITMAPFILEHEADER_SIZE + BITMAPINFOHEADER_SIZE + pre_pixel as u32
}

/// A `BI_BITFIELDS` V3 file: three-mask section + one padded pixel row
/// per `height`. `bpp` must be 16 or 32.
fn bitfields_bmp(width: i32, height: i32, bpp: u16, masks: [u32; 3]) -> Vec<u8> {
    let mut tail = Vec::new();
    for m in masks {
        tail.extend_from_slice(&m.to_le_bytes());
    }
    let row = ((width.unsigned_abs() as usize * bpp as usize).div_ceil(8) + 3) & !3;
    tail.extend(std::iter::repeat(0xA5u8).take(row * height.unsigned_abs() as usize));
    raw_bmp_v3(
        width,
        height,
        bpp,
        BI_BITFIELDS,
        0,
        0,
        v3_off_bits(12),
        &tail,
    )
}

// ---------------------------------------------------------------------------
// Mask matrices
// ---------------------------------------------------------------------------

/// Every single-bit mask position for R/G/B at 32 bpp and 16 bpp. All
/// masks are valid (contiguous, disjoint), so decode must *succeed* —
/// this pins the full shift/width extraction range, including bit 31
/// (the sign-bit corner) and bit 0.
#[test]
fn single_bit_mask_matrix_decodes() {
    for bpp in [32u16, 16] {
        let bit_span = bpp as u32;
        for r in 0..bit_span {
            let g = (r + 1) % bit_span;
            let b = (r + 2) % bit_span;
            let file = bitfields_bmp(3, 2, bpp, [1 << r, 1 << g, 1 << b]);
            let decoded = decode_bmp(&file)
                .unwrap_or_else(|e| panic!("bpp={bpp} single-bit masks r={r} g={g} b={b}: {e:?}"));
            assert_eq!((decoded.width, decoded.height), (3, 2));
        }
    }
}

/// Hostile mask sets: overlapping, non-contiguous, all-ones, and (for
/// 16 bpp) masks entirely above the pixel width. Decoder may accept or
/// reject each — it must not panic, shift out of range, or divide by
/// zero.
#[test]
fn hostile_mask_sets_do_not_panic() {
    let hostile: &[[u32; 3]] = &[
        [u32::MAX, u32::MAX, u32::MAX],          // full overlap, 32 bits wide
        [0xAAAA_AAAA, 0x5555_5555, 0xFFFF_0000], // non-contiguous
        [0x8000_0000, 0x8000_0000, 0x8000_0000], // sign bit, all overlapping
        [0xFFFF_FF00, 0x00FF_FFFF, 0x0FFF_FFF0], // >8-bit widths, overlapping
        [0, 0, u32::MAX],                        // two dead channels
        [1, 1, 1],                               // triple overlap at bit 0
        [0x00F0_0000, 0x0000_F000, 0x0000_00F0], // valid-looking but 16bpp-hostile
    ];
    for &masks in hostile {
        for bpp in [16u16, 32] {
            let file = bitfields_bmp(4, 3, bpp, masks);
            let _ = decode_bmp(&file); // must not panic
        }
    }
}

/// `BI_ALPHABITFIELDS` carries a fourth mask word. Run the hostile
/// alpha values against valid colour masks and vice versa.
#[test]
fn alphabitfields_hostile_alpha_does_not_panic() {
    let colour: [u32; 3] = [0x00FF_0000, 0x0000_FF00, 0x0000_00FF];
    for alpha in [
        0u32,
        u32::MAX,
        0xFF00_0000,
        0x00FF_0000, // overlaps red
        0x8000_0000,
        0xAAAA_AAAA,
    ] {
        let mut tail = Vec::new();
        for m in colour {
            tail.extend_from_slice(&m.to_le_bytes());
        }
        tail.extend_from_slice(&alpha.to_le_bytes());
        let row = 4 * 4; // 4 px × 32 bpp
        tail.extend(std::iter::repeat(0x3Cu8).take(row * 2));
        let file = raw_bmp_v3(4, 2, 32, BI_ALPHABITFIELDS, 0, 0, v3_off_bits(16), &tail);
        let _ = decode_bmp(&file); // must not panic
    }
}

/// V4 headers carry the four masks *inside* the header (offsets
/// 40..56). Patch an encoder-produced V4 RGB565 file with hostile
/// in-header masks: no combination may panic.
#[test]
fn v4_inheader_hostile_masks_do_not_panic() {
    let img = BmpImage {
        width: 6,
        height: 4,
        pixel_format: BmpPixelFormat::Rgb565,
        planes: vec![BmpPlane {
            stride: 12,
            data: (0..48u8).collect(),
        }],
        palette: None,
        pts: None,
    };
    let baseline = encode_bmp(&img).unwrap().0;
    assert_eq!(
        u32::from_le_bytes(baseline[14..18].try_into().unwrap()),
        BITMAPV4HEADER_SIZE,
        "encoder is expected to emit a V4 header for RGB565",
    );
    let mask_base = BITMAPFILEHEADER_SIZE as usize + 40;
    for masks in [
        [u32::MAX; 4],
        [0u32; 4],
        [0x8000_0000, 0x4000_0000, 0x2000_0000, 0x1000_0000],
        [0xFFFF_FFFF, 0x0000_0001, 0x8000_0001, 0x7FFF_FFFE],
    ] {
        let mut mutated = baseline.clone();
        for (i, m) in masks.iter().enumerate() {
            mutated[mask_base + i * 4..mask_base + i * 4 + 4].copy_from_slice(&m.to_le_bytes());
        }
        let _ = decode_bmp(&mutated); // must not panic
    }
}

// ---------------------------------------------------------------------------
// Integer-overflow probes
// ---------------------------------------------------------------------------

/// Header dimension pairs chosen so naive area / stride arithmetic
/// wraps at i32, u32, or 32-bit usize width. All must return without
/// panicking and without allocating the claimed pixel buffer; a 64-byte
/// input can never back any of these.
#[test]
fn dimension_overflow_probes_return_fast() {
    let hostile: &[(i32, i32, u16)] = &[
        (i32::MAX, 1, 32),              // stride = width×4 wraps i32
        (i32::MAX, i32::MAX, 1),        // area wraps everything
        (46341, 46341, 8),              // 46341² > i32::MAX by a hair
        (65536, 65536, 24),             // area == 2^32 exactly (u32 wrap to 0)
        (0x2000_0000, 2, 32),           // width×4 == 2^31, ×height wraps u32
        (1, i32::MAX, 32),              // per-row fine, total wraps
        (i32::MIN + 1, 4, 32),          // negative width magnitude near 2^31
        (4, i32::MIN + 1, 32),          // top-down height magnitude near 2^31
        (0x00FF_FFFF, 0x0000_0100, 16), // area == 2^32 - 256, stride padding wraps
    ];
    for &(w, h, bpp) in hostile {
        let file = raw_bmp_v3(w, h, bpp, 0, 0, 0, v3_off_bits(0), &[0xEE; 16]);
        let r = decode_bmp(&file);
        assert!(
            r.is_err(),
            "({w},{h})@{bpp}bpp: a 70-byte file cannot back this geometry",
        );
    }
}

/// `biClrUsed = u32::MAX` at every legal depth: the colour-table size
/// `clr_used × 4` wraps u32. Must never panic or allocate 16 GiB.
#[test]
fn clr_used_saturated_all_depths_do_not_panic() {
    for bpp in [1u16, 2, 4, 8, 16, 24, 32] {
        for clr_used in [u32::MAX, u32::MAX / 4 + 1, 0x4000_0000] {
            let file = raw_bmp_v3(2, 2, bpp, 0, 0, clr_used, v3_off_bits(0), &[0x55; 64]);
            let _ = decode_bmp(&file); // must not panic
        }
    }
}

/// Hostile `bfOffBits` values: saturated, zero, inside the file header,
/// exactly at the header boundary, one past EOF, and values whose
/// `offset + row` arithmetic wraps u32.
#[test]
fn off_bits_extremes_do_not_panic() {
    let tail = [0x77u8; 32];
    for off_bits in [
        u32::MAX,
        u32::MAX - 3,
        0,
        1,
        13,
        14,
        (BITMAPFILEHEADER_SIZE + BITMAPINFOHEADER_SIZE + 33), // one past EOF
    ] {
        let file = raw_bmp_v3(2, 2, 32, 0, 0, 0, off_bits, &tail);
        let _ = decode_bmp(&file); // must not panic
    }
}

/// `biSizeImage = u32::MAX` on both RLE flavours — the size claim must
/// not drive an allocation (the actual stream is 2 bytes of EOB).
#[test]
fn size_image_saturated_rle_does_not_allocate() {
    for (compression, bpp) in [(BI_RLE8, 8u16), (BI_RLE4, 4)] {
        let mut tail = vec![0u8; 4 * (1usize << bpp)]; // full colour table
        tail.extend_from_slice(&[0x00, 0x01]); // EOB immediately
        let pre = tail.len() - 2;
        let file = raw_bmp_v3(4, 4, bpp, compression, u32::MAX, 0, v3_off_bits(pre), &tail);
        let _ = decode_bmp(&file); // must not panic / OOM
    }
}

// ---------------------------------------------------------------------------
// RLE4 stream attacks
// ---------------------------------------------------------------------------

/// A minimal valid RLE4 BMP: 16-entry colour table + caller-supplied
/// RLE stream.
fn rle4_bmp(width: i32, height: i32, stream: &[u8]) -> Vec<u8> {
    let mut tail = vec![0u8; 4 * 16];
    for (i, chunk) in tail.chunks_exact_mut(4).enumerate() {
        chunk[0] = i as u8 * 16;
        chunk[1] = 255 - i as u8 * 16;
        chunk[2] = i as u8;
    }
    let pre = tail.len();
    tail.extend_from_slice(stream);
    raw_bmp_v3(width, height, 4, BI_RLE4, 0, 0, v3_off_bits(pre), &tail)
}

/// Delta escapes that push the cursor past the right edge, past the
/// top row, and both at once — repeatedly, so any unchecked cursor
/// accumulation wraps.
#[test]
fn rle4_delta_beyond_edges_does_not_panic() {
    let mut runaway = Vec::new();
    for _ in 0..64 {
        runaway.extend_from_slice(&[0x00, 0x02, 0xFF, 0xFF]); // delta +255,+255
    }
    runaway.extend_from_slice(&[0x00, 0x01]); // EOB
    let streams: &[&[u8]] = &[
        &[0x00, 0x02, 0xFF, 0x00, 0x00, 0x01], // dx past right edge
        &[0x00, 0x02, 0x00, 0xFF, 0x00, 0x01], // dy past top
        &[0x00, 0x02, 0xFF, 0xFF, 0x00, 0x01], // both
        &[0x00, 0x02, 0xFF, 0xFF],             // delta then EOF (no EOB)
        &runaway,                              // 64 accumulated deltas
        &[0x00, 0x02],                         // escape truncated mid-args
        &[0x00, 0x02, 0x10],                   // one arg then EOF
    ];
    for stream in streams {
        let _ = decode_bmp(&rle4_bmp(8, 4, stream)); // must not panic
    }
}

/// Absolute-mode runs that overrun the row, straddle the word-pad, or
/// truncate mid-run.
#[test]
fn rle4_absolute_mode_overruns_do_not_panic() {
    // count=255 nibbles at a 8-px row: massively past the row end.
    let mut overrun = vec![0x00, 0xFF];
    overrun.extend(std::iter::repeat(0x12u8).take(128)); // 255 nibbles = 128 B
    overrun.extend_from_slice(&[0x00, 0x01]);

    // count=5 (odd nibble count, 3 data bytes, needs 1 pad byte) but the
    // pad byte is missing at EOF.
    let missing_pad = [0x00u8, 0x05, 0x12, 0x34, 0x50];

    // count=3 declared, zero data bytes follow.
    let truncated_data = [0x00u8, 0x03];

    for stream in [&overrun[..], &missing_pad[..], &truncated_data[..]] {
        let _ = decode_bmp(&rle4_bmp(8, 4, stream)); // must not panic
    }
}

/// Encoded runs that exceed the row width at every starting column:
/// run count 255 preceded by 0..w positioning pixels.
#[test]
fn rle4_encoded_run_overruns_every_column_do_not_panic() {
    for lead in 0..8u8 {
        let mut stream = Vec::new();
        if lead > 0 {
            stream.extend_from_slice(&[lead, 0x34]); // position the cursor
        }
        stream.extend_from_slice(&[0xFF, 0xAB]); // run of 255 nibbles
        stream.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]); // EOL, EOB
        let _ = decode_bmp(&rle4_bmp(8, 4, &stream)); // must not panic
    }
}

/// Full truncation sweep + byte-value grid over a hand-built RLE4
/// stream that exercises every opcode class (encoded run, absolute
/// run + pad, delta, EOL, EOB).
#[test]
fn rle4_truncation_and_mutation_grid_does_not_panic() {
    let stream: &[u8] = &[
        0x04, 0x12, // encoded run: 4 nibbles
        0x00, 0x04, 0xAB, 0xCD, // absolute: 4 nibbles (word-aligned)
        0x00, 0x02, 0x01, 0x01, // delta +1,+1
        0x03, 0x77, // encoded run: 3 nibbles
        0x00, 0x00, // EOL
        0x05, 0x9F, // encoded run on next row
        0x00, 0x01, // EOB
    ];
    let file = rle4_bmp(8, 4, stream);
    let stream_start = file.len() - stream.len();

    for cut in 0..file.len() {
        let _ = decode_bmp(&file[..cut]);
    }
    for idx in stream_start..file.len() {
        for value in [0x00u8, 0x01, 0x02, 0x03, 0xFF, 0x80, 0x10] {
            let mut mutated = file.clone();
            mutated[idx] = value;
            let _ = decode_bmp(&mutated);
        }
    }
}

/// The doubled-height ICO framing on top of hostile RLE4: RLE +
/// AND-mask interaction is undefined, so any `Result` is fine — the
/// decoder just must not panic walking the mask rows.
#[test]
fn rle4_ico_doubled_height_does_not_panic() {
    let stream: &[u8] = &[0x08, 0x12, 0x00, 0x00, 0x08, 0x34, 0x00, 0x01];
    let file = rle4_bmp(8, 4, stream);
    let dib = &file[BITMAPFILEHEADER_SIZE as usize..];
    let _ = decode_dib(dib, true);
    let _ = decode_dib(dib, false);
}
