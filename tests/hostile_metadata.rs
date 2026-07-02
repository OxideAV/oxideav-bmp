//! Adversarial tests for the V4 / V5 metadata surfaces (Round 383).
//!
//! `tests/malformed_inputs.rs` mangles the pixel-decode path
//! (`decode_bmp` / `decode_dib`); this suite points the same
//! structural-mutation style at the *metadata* entry points —
//! `decode_bmp_with_metadata` / `decode_dib_with_metadata` — whose
//! colour-space tail parsing and ICC-blob slicing maths
//! (`input[base + bV5ProfileData ..][.. bV5ProfileSize]`, both fields
//! attacker-controlled u32s) the pixel path never reaches.
//!
//! Fixtures are built exclusively through the public colour-management
//! encoders (`encode_bmp_with_icc_profile`,
//! `encode_bmp_with_linked_icc_profile`, `encode_bmp_with_calibrated_rgb`)
//! and then mutated byte-wise. The invariants:
//!
//!   * no truncation, bit-flip, or field patch may panic, index out of
//!     bounds, or OOM-abort — every call returns a `Result`;
//!   * a declared profile slice that falls outside the input yields
//!     `Ok` with the blob field `None` (the declared offset / size stay
//!     inspectable), never `Err`-by-panic;
//!   * extreme-but-wellformed colour-management values (i32::MIN/MAX
//!     endpoints, u32::MAX gamma, undefined intent codes) round-trip
//!     verbatim.

use oxideav_bmp::{
    decode_bmp_with_metadata, decode_dib_with_metadata, encode_bmp_with_calibrated_rgb,
    encode_bmp_with_icc_profile, encode_bmp_with_linked_icc_profile, BmpColorSpace,
    BmpEncodeOptions, BmpImage, BmpPalette, BmpPixelFormat, BmpPlane, BmpRenderingIntent,
    BITMAPFILEHEADER_SIZE, PROFILE_EMBEDDED, PROFILE_LINKED,
};

// ---------------------------------------------------------------------------
// Fixture builders (public encoder API only)
// ---------------------------------------------------------------------------

fn rgba_image(w: u32, h: u32) -> BmpImage {
    let mut data = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            data.extend_from_slice(&[
                ((x * 29) & 0xFF) as u8,
                ((y * 31) & 0xFF) as u8,
                ((x ^ y) & 0xFF) as u8,
                0xFF - ((x & 0xFF) as u8),
            ]);
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

fn indexed8_image(w: u32, h: u32) -> BmpImage {
    let data: Vec<u8> = (0..w * h).map(|i| (i & 0x0F) as u8).collect();
    let entries: Vec<[u8; 3]> = (0..16u8).map(|i| [i * 16, 255 - i * 16, i]).collect();
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

/// A fake-but-plausible ICC blob (the decoder treats it opaquely).
fn icc_blob(len: usize) -> Vec<u8> {
    (0..len).map(|i| ((i * 7) ^ (i >> 3)) as u8).collect()
}

/// The four V4/V5 fixture files this suite mutates. Names keep failure
/// output readable.
fn metadata_fixtures() -> Vec<(&'static str, Vec<u8>)> {
    let opts = BmpEncodeOptions::default();
    vec![
        (
            "v5_embedded_rgba",
            encode_bmp_with_icc_profile(&rgba_image(7, 5), &icc_blob(64), 4, opts).unwrap(),
        ),
        (
            "v5_embedded_indexed8",
            encode_bmp_with_icc_profile(&indexed8_image(9, 4), &icc_blob(17), 1, opts).unwrap(),
        ),
        (
            "v5_linked_rgba",
            encode_bmp_with_linked_icc_profile(
                &rgba_image(4, 6),
                b"m:\\profiles\\p.icc\0",
                2,
                opts,
            )
            .unwrap(),
        ),
        (
            "v4_calibrated_rgba",
            encode_bmp_with_calibrated_rgb(
                &rgba_image(6, 3),
                [1 << 30, 0, 0, 0, 1 << 30, 0, 0, 0, 1 << 30],
                [0x0001_0000; 3],
                opts,
            )
            .unwrap(),
        ),
    ]
}

/// File-absolute offsets of the V5 colour-management fields.
const FILE_OFF_CS_TYPE: usize = BITMAPFILEHEADER_SIZE as usize + 56;
const FILE_OFF_PROFILE_DATA: usize = BITMAPFILEHEADER_SIZE as usize + 112;
const FILE_OFF_PROFILE_SIZE: usize = BITMAPFILEHEADER_SIZE as usize + 116;

fn patch_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

// ---------------------------------------------------------------------------
// Truncation sweeps
// ---------------------------------------------------------------------------

/// Every prefix of every V4/V5 fixture must decode without panicking
/// through the BMP-file metadata entry point, and the untruncated file
/// must succeed.
#[test]
fn metadata_truncation_sweep_never_panics() {
    for (name, full) in metadata_fixtures() {
        for len in 0..full.len() {
            let _ = decode_bmp_with_metadata(&full[..len]);
        }
        let (image, _) = decode_bmp_with_metadata(&full)
            .unwrap_or_else(|e| panic!("{name}: full decode: {e:?}"));
        assert!(image.width > 0, "{name}: decoded a zero-width image");
    }
}

/// Same sweep through the header-less DIB metadata entry point, under
/// both mask framings. Stripping the 14-byte file header off an
/// encoder-produced BMP yields a self-consistent DIB (every V5 offset
/// is DIB-relative).
#[test]
fn dib_metadata_truncation_sweep_never_panics() {
    for (name, full) in metadata_fixtures() {
        let dib = &full[BITMAPFILEHEADER_SIZE as usize..];
        for len in 0..dib.len() {
            let _ = decode_dib_with_metadata(&dib[..len], false);
            let _ = decode_dib_with_metadata(&dib[..len], true);
        }
        let (_, meta) = decode_dib_with_metadata(dib, false)
            .unwrap_or_else(|e| panic!("{name}: full DIB decode: {e:?}"));
        assert!(meta.header_size >= 108, "{name}: expected a V4/V5 header");
    }
}

// ---------------------------------------------------------------------------
// Bit-flip sweep
// ---------------------------------------------------------------------------

/// Flipping any single bit anywhere in a V4/V5 file (header, palette,
/// pixels, ICC blob) must never panic the metadata decoder.
#[test]
fn single_bit_flip_v5_metadata_never_panics() {
    for (_name, full) in metadata_fixtures() {
        for byte_idx in 0..full.len() {
            for bit in 0..8 {
                let mut mutated = full.clone();
                mutated[byte_idx] ^= 1 << bit;
                let _ = decode_bmp_with_metadata(&mutated);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Hostile bV5ProfileData / bV5ProfileSize probes
// ---------------------------------------------------------------------------

/// Attacker-controlled profile offset / size pairs. Wherever the
/// declared slice escapes the input, the decode must still succeed
/// (pixels are unaffected) with the blob reported as absent; wherever
/// it stays in bounds the decode must succeed with *some* blob. No
/// combination may panic or wrap.
#[test]
fn hostile_profile_offset_size_probes() {
    let full = metadata_fixtures().remove(0).1; // v5_embedded_rgba
    let file_len = full.len() as u32;
    let dib_len = file_len - BITMAPFILEHEADER_SIZE;

    let hostile: &[(u32, u32)] = &[
        (u32::MAX, u32::MAX),         // both saturated — offset+size wraps
        (u32::MAX, 1),                // offset past EOF
        (u32::MAX - 3, 8),            // offset+size wraps u32 exactly
        (0, u32::MAX),                // size covers "everything and more"
        (dib_len, 1),                 // first byte past EOF
        (dib_len.wrapping_sub(1), 2), // straddles EOF by one byte
        (0, 0),                       // zero-size at header start
        (0, 4),                       // in-bounds but inside the header
        (124, dib_len),               // pixel array claimed as profile, one byte too long
    ];
    for &(offset, size) in hostile {
        let mut mutated = full.clone();
        patch_u32(&mut mutated, FILE_OFF_PROFILE_DATA, offset);
        patch_u32(&mut mutated, FILE_OFF_PROFILE_SIZE, size);

        let (_, meta) = decode_bmp_with_metadata(&mutated).unwrap_or_else(|e| {
            panic!("profile ({offset:#x},{size:#x}): pixels must still decode: {e:?}")
        });
        assert_eq!(meta.profile_data_offset, Some(offset));
        assert_eq!(meta.profile_size, Some(size));

        let in_bounds = (offset as u64 + size as u64) <= dib_len as u64;
        if !in_bounds {
            assert!(
                meta.icc_profile.is_none(),
                "profile ({offset:#x},{size:#x}): out-of-bounds slice must not produce a blob",
            );
        } else if size > 0 {
            let blob = meta
                .icc_profile
                .unwrap_or_else(|| panic!("profile ({offset:#x},{size:#x}): in-bounds blob lost"));
            assert_eq!(blob.len(), size as usize);
        }

        // The header-less DIB framing re-runs the same slicing maths
        // with base 0 instead of 14.
        let dib = &mutated[BITMAPFILEHEADER_SIZE as usize..];
        let _ = decode_dib_with_metadata(dib, false);
        let _ = decode_dib_with_metadata(dib, true);
    }
}

/// The same hostile pairs with the colour space flipped to
/// `PROFILE_LINKED` — the path-bytestring slot shares the slicing maths
/// but fills `linked_profile_path` instead.
#[test]
fn hostile_linked_path_offset_size_probes() {
    let full = metadata_fixtures().remove(0).1; // v5_embedded_rgba
    let dib_len = full.len() as u32 - BITMAPFILEHEADER_SIZE;

    for &(offset, size) in &[
        (u32::MAX, u32::MAX),
        (u32::MAX - 3, 8),
        (dib_len, 1),
        (0, u32::MAX),
    ] {
        let mut mutated = full.clone();
        patch_u32(&mut mutated, FILE_OFF_CS_TYPE, PROFILE_LINKED);
        patch_u32(&mut mutated, FILE_OFF_PROFILE_DATA, offset);
        patch_u32(&mut mutated, FILE_OFF_PROFILE_SIZE, size);
        let (_, meta) = decode_bmp_with_metadata(&mutated)
            .unwrap_or_else(|e| panic!("linked ({offset:#x},{size:#x}): {e:?}"));
        assert_eq!(meta.color_space, Some(BmpColorSpace::ProfileLinked));
        assert!(
            meta.linked_profile_path.is_none(),
            "linked ({offset:#x},{size:#x}): out-of-bounds path must not materialise",
        );
        assert!(meta.icc_profile.is_none());
    }
}

/// Every value of the low `bV5CSType` byte (and a spread of full-word
/// values) must parse without panicking, and unknown tags must surface
/// as `BmpColorSpace::Unknown` rather than being dropped.
#[test]
fn cs_type_word_probe_never_panics() {
    let full = metadata_fixtures().remove(0).1; // v5_embedded_rgba
    for low in 0..=255u32 {
        let mut mutated = full.clone();
        patch_u32(&mut mutated, FILE_OFF_CS_TYPE, low);
        let _ = decode_bmp_with_metadata(&mutated);
    }
    for word in [
        0x0000_0001,
        0x4C43_5320, // ASCII-ish garbage
        PROFILE_EMBEDDED ^ 1,
        PROFILE_LINKED ^ 0x2000_0000,
        u32::MAX,
    ] {
        let mut mutated = full.clone();
        patch_u32(&mut mutated, FILE_OFF_CS_TYPE, word);
        let (_, meta) = decode_bmp_with_metadata(&mutated)
            .unwrap_or_else(|e| panic!("cs_type {word:#010x}: {e:?}"));
        assert_eq!(
            meta.color_space,
            Some(BmpColorSpace::Unknown(word)),
            "cs_type {word:#010x} must surface as Unknown",
        );
    }
}

// ---------------------------------------------------------------------------
// Extreme-value round-trips (well-formed, hostile-magnitude)
// ---------------------------------------------------------------------------

/// i32::MIN / i32::MAX endpoints and u32::MAX gamma survive the V4
/// calibrated round-trip verbatim.
#[test]
fn calibrated_extreme_endpoints_roundtrip() {
    let endpoints = [
        i32::MIN,
        i32::MAX,
        -1,
        0,
        1,
        i32::MIN + 1,
        i32::MAX - 1,
        0x5555_5555,
        -0x5555_5556,
    ];
    let gamma = [u32::MAX, 0, 0x8000_0000];
    let encoded = encode_bmp_with_calibrated_rgb(
        &rgba_image(3, 3),
        endpoints,
        gamma,
        BmpEncodeOptions::default(),
    )
    .expect("extreme endpoints must encode");
    let (_, meta) = decode_bmp_with_metadata(&encoded).expect("extreme endpoints must decode");
    assert_eq!(meta.color_space, Some(BmpColorSpace::Calibrated));
    assert_eq!(meta.endpoints, Some(endpoints));
    assert_eq!(meta.gamma_rgb, Some(gamma));
}

/// Undefined `bV5Intent` codes are written verbatim and surface as
/// `BmpRenderingIntent::Unknown`, never as an error.
#[test]
fn undefined_rendering_intent_codes_survive() {
    for intent in [3u32, 5, 16, 0x8000_0000, u32::MAX] {
        let encoded = encode_bmp_with_icc_profile(
            &rgba_image(2, 2),
            &icc_blob(8),
            intent,
            BmpEncodeOptions::default(),
        )
        .expect("undefined intent must encode");
        let (_, meta) = decode_bmp_with_metadata(&encoded).expect("undefined intent must decode");
        assert_eq!(
            meta.rendering_intent,
            Some(BmpRenderingIntent::Unknown(intent))
        );
    }
}

/// Zero-length and multi-KiB profile blobs both round-trip: the
/// boundary sizes of the trailing-slot maths.
#[test]
fn profile_blob_size_boundaries_roundtrip() {
    for len in [0usize, 1, 3, 4096] {
        let blob = icc_blob(len);
        let encoded =
            encode_bmp_with_icc_profile(&rgba_image(2, 2), &blob, 0, BmpEncodeOptions::default())
                .expect("boundary blob must encode");
        let (_, meta) = decode_bmp_with_metadata(&encoded).expect("boundary blob must decode");
        assert_eq!(meta.profile_size, Some(len as u32));
        assert_eq!(
            meta.icc_profile.as_deref().unwrap_or(&[]),
            &blob[..],
            "blob of {len} bytes diverged",
        );
    }
}
