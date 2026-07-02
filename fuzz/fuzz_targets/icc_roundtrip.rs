#![no_main]

//! V4 / V5 colour-management encoder round-trip target.
//!
//! The `encode_roundtrip` harness drives the baseline
//! `encode_bmp_with_options` surface; this one drives the three
//! colour-management encode entry points that write V4 / V5 headers
//! with a colour-space tail and (for V5) a trailing profile blob:
//!
//!   * [`encode_bmp_with_icc_profile`]        — V5, `PROFILE_EMBEDDED`,
//!     ICC blob after the pixel array at `bV5ProfileData`;
//!   * [`encode_bmp_with_linked_icc_profile`] — V5, `PROFILE_LINKED`,
//!     path bytestring in the same trailing slot;
//!   * [`encode_bmp_with_calibrated_rgb`]     — V4, `LCS_CALIBRATED_RGB`,
//!     nine fuzzer-picked `i32` endpoints + three `u32` gamma values.
//!
//! Contract per iteration:
//!
//!   1. **No-panic** on either side, for every format × options × blob
//!      combination the framing below can produce (encode `Err` is
//!      accepted and ends the iteration).
//!   2. **Decode must succeed** on every encode success — the encoder
//!      is never allowed to emit bytes its own decoder rejects.
//!   3. **Metadata round-trip.** Embedded blobs come back byte-exact in
//!      `BmpMetadata::icc_profile`, linked paths in
//!      `BmpMetadata::linked_profile_path`, calibrated endpoints /
//!      gamma verbatim in `endpoints` / `gamma_rgb`, and the decoded
//!      colour-space tag matches the entry point used.
//!   4. **Pixel round-trip for `Rgba`** (32-bpp `BI_RGB` on all three
//!      paths keeps every byte).
//!
//! ## Wire framing
//!
//!   * byte 0 — path selector (`% 3`): 0 = embedded ICC, 1 = linked
//!     ICC, 2 = calibrated RGB.
//!   * byte 1 — format selector (`% 7`): 0=Rgba, 1=Rgb24, 2=Rgb555,
//!     3=Rgb565, 4=Indexed8, 5=Indexed4, 6=Indexed1 (the set all three
//!     entry points document).
//!   * byte 2 — options: bit 0 = `top_down`, bit 1 = `minimal_palette`.
//!   * byte 3 — width, clamped to 1..=32; byte 4 — height, clamped to
//!     1..=32 (V4/V5 headers are large; small geometry keeps the
//!     encoder's offset maths hot without burning the budget on pixel
//!     copying).
//!   * byte 5 — rendering intent (raw u32 from the byte so the
//!     unspecified/undefined values are exercised too).
//!   * byte 6 — blob length (0..=255) for the ICC / linked paths.
//!   * following bytes — blob bytes (embedded / linked), or the
//!     48-byte endpoints + gamma block (calibrated), then pixels, then
//!     palette entries (3 B each) for indexed formats.

use libfuzzer_sys::fuzz_target;
use oxideav_bmp::{
    decode_bmp_with_metadata, encode_bmp_with_calibrated_rgb, encode_bmp_with_icc_profile,
    encode_bmp_with_linked_icc_profile, BmpColorSpace, BmpEncodeOptions, BmpImage, BmpPalette,
    BmpPixelFormat, BmpPlane,
};

const MAX_DIM: u32 = 32;

fn pick_format(byte: u8) -> BmpPixelFormat {
    match byte % 7 {
        0 => BmpPixelFormat::Rgba,
        1 => BmpPixelFormat::Rgb24,
        2 => BmpPixelFormat::Rgb555,
        3 => BmpPixelFormat::Rgb565,
        4 => BmpPixelFormat::Indexed8,
        5 => BmpPixelFormat::Indexed4,
        _ => BmpPixelFormat::Indexed1,
    }
}

fn bytes_per_pixel(format: BmpPixelFormat) -> usize {
    match format {
        BmpPixelFormat::Rgba => 4,
        BmpPixelFormat::Rgb24 => 3,
        BmpPixelFormat::Rgb555 | BmpPixelFormat::Rgb565 => 2,
        _ => 1,
    }
}

fn palette_cap(format: BmpPixelFormat) -> usize {
    match format {
        BmpPixelFormat::Indexed8 => 256,
        BmpPixelFormat::Indexed4 => 16,
        BmpPixelFormat::Indexed1 => 2,
        _ => 0,
    }
}

/// Cycle `src` to fill a fresh `len`-byte buffer (zeros when `src` is
/// empty).
fn cycled(src: &[u8], len: usize) -> Vec<u8> {
    let mut out = vec![0u8; len];
    if !src.is_empty() {
        for (i, slot) in out.iter_mut().enumerate() {
            *slot = src[i % src.len()];
        }
    }
    out
}

fuzz_target!(|data: &[u8]| {
    if data.len() < 7 {
        return;
    }
    let path_selector = data[0] % 3;
    let format = pick_format(data[1]);
    let options = BmpEncodeOptions {
        top_down: data[2] & 1 != 0,
        minimal_palette: data[2] & 2 != 0,
    };
    let width = 1 + (data[3] as u32) % MAX_DIM;
    let height = 1 + (data[4] as u32) % MAX_DIM;
    let intent = data[5] as u32;
    let blob_len = data[6] as usize;
    let tail = &data[7..];

    // Carve the path-specific block off the tail, then pixels + palette.
    let (blob, endpoints, gamma, rest) = match path_selector {
        0 | 1 => {
            let blob = cycled(&tail[..blob_len.min(tail.len())], blob_len);
            let rest = &tail[blob_len.min(tail.len())..];
            (blob, [0i32; 9], [0u32; 3], rest)
        }
        _ => {
            let block = cycled(&tail[..48.min(tail.len())], 48);
            let mut endpoints = [0i32; 9];
            for (i, e) in endpoints.iter_mut().enumerate() {
                *e = i32::from_le_bytes(block[i * 4..i * 4 + 4].try_into().unwrap());
            }
            let mut gamma = [0u32; 3];
            for (i, g) in gamma.iter_mut().enumerate() {
                *g = u32::from_le_bytes(block[36 + i * 4..40 + i * 4].try_into().unwrap());
            }
            let rest = &tail[48.min(tail.len())..];
            (Vec::new(), endpoints, gamma, rest)
        }
    };

    let bpp = bytes_per_pixel(format);
    let stride = width as usize * bpp;
    let pixel_len = stride * height as usize;
    let pixel_src = &rest[..pixel_len.min(rest.len())];
    let plane = BmpPlane {
        stride,
        data: cycled(pixel_src, pixel_len),
    };

    let palette = {
        let cap = palette_cap(format);
        if cap == 0 {
            None
        } else {
            let pal_src = &rest[pixel_len.min(rest.len())..];
            let mut entries = Vec::with_capacity(cap);
            let mut chunks = pal_src.chunks_exact(3);
            for chunk in chunks.by_ref().take(cap) {
                entries.push([chunk[0], chunk[1], chunk[2]]);
            }
            while entries.len() < cap {
                entries.push([0, 0, 0]);
            }
            Some(BmpPalette { entries })
        }
    };

    let image = BmpImage {
        width,
        height,
        pixel_format: format,
        planes: vec![plane],
        palette,
        pts: None,
    };

    let encoded = match path_selector {
        0 => encode_bmp_with_icc_profile(&image, &blob, intent, options),
        1 => encode_bmp_with_linked_icc_profile(&image, &blob, intent, options),
        _ => encode_bmp_with_calibrated_rgb(&image, endpoints, gamma, options),
    };
    let encoded = match encoded {
        Ok(bytes) => bytes,
        // Encoder rejection is a legal outcome; the contract is only
        // that it never panics and never emits undecodable bytes.
        Err(_) => return,
    };

    let (decoded, meta) =
        decode_bmp_with_metadata(&encoded).expect("colour-management encoder output must decode");
    assert_eq!(decoded.width, width, "decoded width mismatch");
    assert_eq!(decoded.height, height, "decoded height mismatch");
    assert_eq!(decoded.pixel_format, BmpPixelFormat::Rgba);

    match path_selector {
        0 => {
            assert_eq!(
                meta.color_space,
                Some(BmpColorSpace::ProfileEmbedded),
                "embedded-profile output must tag PROFILE_EMBEDDED",
            );
            assert_eq!(
                meta.icc_profile.as_deref().unwrap_or(&[]),
                &blob[..],
                "embedded ICC blob diverged on the way back",
            );
        }
        1 => {
            assert_eq!(
                meta.color_space,
                Some(BmpColorSpace::ProfileLinked),
                "linked-profile output must tag PROFILE_LINKED",
            );
            assert_eq!(
                meta.linked_profile_path.as_deref().unwrap_or(&[]),
                &blob[..],
                "linked profile path diverged on the way back",
            );
        }
        _ => {
            assert_eq!(
                meta.color_space,
                Some(BmpColorSpace::Calibrated),
                "calibrated output must tag LCS_CALIBRATED_RGB",
            );
            assert_eq!(meta.endpoints, Some(endpoints), "endpoints diverged");
            assert_eq!(meta.gamma_rgb, Some(gamma), "gamma triple diverged");
        }
    }

    // 32-bpp BI_RGB keeps all four bytes on every path.
    if format == BmpPixelFormat::Rgba {
        assert_eq!(
            decoded.planes[0].data, image.planes[0].data,
            "Rgba pixels diverged through the V4/V5 colour-management path",
        );
    }
});
