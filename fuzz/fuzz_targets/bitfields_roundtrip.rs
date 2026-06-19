#![no_main]

//! Explicit-mask `BI_BITFIELDS` / `BI_ALPHABITFIELDS` encoder fuzz target.
//!
//! Drives [`encode_bmp_bitfields`] with fuzzer-controlled pixels, a mask
//! set, and encode options, then decodes the output. The contract:
//!
//!   1. **No-panic.** Neither `encode_bmp_bitfields` (including the
//!      `BmpBitfields::validate` rejection path) nor the follow-up
//!      `decode_bmp` may panic, integer-overflow, index out of bounds, or
//!      OOM-abort on any input. Errors are fine; crashes are not.
//!
//!   2. **Lossless round-trip for the byte-aligned 32-bpp preset.** When
//!      the selected mask set is `BmpBitfields::BGRA8888` every channel
//!      keeps its full 8 bits and alpha survives, so the decoded `Rgba`
//!      plane must equal the source plane byte for byte.
//!
//!   3. **Colour-exact, alpha-opaque for `BGRX8888`.** The three-mask
//!      32-bpp preset drops alpha; the three colour channels are bit-exact
//!      and the decoder reads alpha = `0xFF`.
//!
//! ## Wire framing
//!
//!   * byte 0 — mask-preset selector (`byte % 6`): 0=RGB565, 1=RGB555,
//!     2=ARGB1555, 3=BGRA8888, 4=BGRX8888, 5=an arbitrary mask built from
//!     the next bytes (exercises `validate`'s reject path).
//!   * byte 1 — encode options: bit 0 = `top_down`.
//!   * byte 2 — width, clamped to 1..=64.
//!   * byte 3 — height, clamped to 1..=64.
//!   * bytes 4..N — `Rgba` pixel payload (4 B/px), cycled to fill the
//!     `width × height × 4` plane. For the arbitrary-mask case bytes
//!     4..20 seed the four candidate masks first, and the pixel payload
//!     follows.

use libfuzzer_sys::fuzz_target;
use oxideav_bmp::{
    decode_bmp, encode_bmp_bitfields, BmpBitfields, BmpEncodeOptions, BmpImage, BmpPixelFormat,
    BmpPlane,
};

const MAX_DIM: u32 = 64;

/// Build a `width × height × 4` `Rgba` plane by cycling `pixel_bytes`.
fn make_plane(pixel_bytes: &[u8], width: u32, height: u32) -> Option<BmpPlane> {
    let stride = (width as usize).checked_mul(4)?;
    let total = stride.checked_mul(height as usize)?;
    let mut data = vec![0u8; total];
    if !pixel_bytes.is_empty() {
        for (i, slot) in data.iter_mut().enumerate() {
            *slot = pixel_bytes[i % pixel_bytes.len()];
        }
    }
    Some(BmpPlane { stride, data })
}

fuzz_target!(|data: &[u8]| {
    if data.len() < 4 {
        return;
    }
    let selector = data[0] % 6;
    let top_down = data[1] & 0b01 != 0;
    let width = (data[2] as u32).clamp(1, MAX_DIM);
    let height = (data[3] as u32).clamp(1, MAX_DIM);

    let mut rest = &data[4..];
    let masks = match selector {
        0 => BmpBitfields::RGB565,
        1 => BmpBitfields::RGB555,
        2 => BmpBitfields::ARGB1555,
        3 => BmpBitfields::BGRA8888,
        4 => BmpBitfields::BGRX8888,
        _ => {
            // Arbitrary masks from the next 16 bytes (if present). Many of
            // these fail `validate`; that exercises the rejection path.
            if rest.len() < 16 {
                return;
            }
            let rd = |s: &[u8]| u32::from_le_bytes([s[0], s[1], s[2], s[3]]);
            let m = BmpBitfields {
                bpp: if data[1] & 0b10 != 0 { 32 } else { 16 },
                r: rd(&rest[0..4]),
                g: rd(&rest[4..8]),
                b: rd(&rest[8..12]),
                a: rd(&rest[12..16]),
            };
            rest = &rest[16..];
            m
        }
    };

    let plane = match make_plane(rest, width, height) {
        Some(p) => p,
        None => return,
    };
    let image = BmpImage {
        width,
        height,
        pixel_format: BmpPixelFormat::Rgba,
        planes: vec![plane],
        palette: None,
        pts: None,
    };
    let options = BmpEncodeOptions {
        top_down,
        minimal_palette: false,
    };

    let bytes = match encode_bmp_bitfields(&image, masks, options) {
        Ok(b) => b,
        Err(_) => return,
    };
    assert_eq!(&bytes[..2], b"BM", "encoder emitted non-BMP signature");

    let decoded = decode_bmp(&bytes).expect("bitfields output failed to decode");
    assert_eq!(decoded.width, width, "decoded width mismatch");
    assert_eq!(decoded.height, height, "decoded height mismatch");
    assert_eq!(decoded.pixel_format, BmpPixelFormat::Rgba);

    // Byte-aligned 32-bpp presets have an exact contract.
    if masks == BmpBitfields::BGRA8888 {
        assert_eq!(
            decoded.planes[0].data, image.planes[0].data,
            "BGRA8888 round-trip diverged (top_down={top_down})",
        );
    } else if masks == BmpBitfields::BGRX8888 {
        let stride = decoded.planes[0].stride;
        let src_stride = image.planes[0].stride;
        for y in 0..height as usize {
            for x in 0..width as usize {
                let d = &decoded.planes[0].data[y * stride + x * 4..][..4];
                let s = &image.planes[0].data[y * src_stride + x * 4..][..4];
                assert_eq!(&d[..3], &s[..3], "BGRX8888 colour diverged at ({x},{y})");
                assert_eq!(d[3], 0xFF, "BGRX8888 alpha not opaque at ({x},{y})");
            }
        }
    }
});
