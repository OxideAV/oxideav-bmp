#![no_main]

//! Header-less DIB encoder round-trip target — the `.ico` / `.cur`
//! shared surface.
//!
//! `encode_roundtrip` exercises the full-file encoder; this harness
//! drives [`encode_dib`], the header-less flavour whose two layouts
//! (`double_height_for_ico_mask` false / true) are what `oxideav-ico`
//! embeds in icon directories:
//!
//!   * `mask = false` — plain DIB (40-byte header for most formats,
//!     V4 for 5-6-5), no file header, computed pixel offset;
//!   * `mask = true`  — the ICO convention: `biHeight` stores double
//!     the visible height and (for the 32-bpp path) a 1-bpp AND mask
//!     derived from alpha trails the XOR pixel array.
//!
//! Contract per iteration:
//!
//!   1. **No-panic** on either side (encoder `Err` is accepted and ends
//!      the iteration).
//!   2. **Decode must succeed** on every encode success, with the
//!      *matching* `dib_height_is_doubled_for_mask` flag — the encoder
//!      must never emit a DIB its own decoder rejects.
//!   3. **Geometry round-trip** — decoded width / height equal the
//!      encoder inputs for both layouts (the doubled stored height must
//!      halve back exactly).
//!   4. **Pixel round-trip for `Rgba` + `mask = false`** — the plain
//!      32-bpp BGRA layout keeps every byte.
//!   5. **Cross-flag no-panic** — the same bytes are also decoded with
//!      the *opposite* mask flag (an `.ico` payload read as a plain
//!      DIB and vice versa is exactly the confusion a hostile file
//!      engineers), where any `Result` is acceptable but panics are
//!      not.
//!
//! ## Wire framing
//!
//!   * byte 0 — format selector (`% 8`): 0=Rgba, 1=Rgb24, 2=Rgb555,
//!     3=Rgb565, 4=Indexed8, 5=Indexed4, 6=Indexed2, 7=Indexed1.
//!   * byte 1 — bit 0 = `double_height_for_ico_mask`.
//!   * byte 2 — width, clamped to 1..=64; byte 3 — height, clamped to
//!     1..=64.
//!   * bytes 4.. — pixel payload (cycled to fill the plane), then
//!     palette entries (3 B each) for indexed formats.

use libfuzzer_sys::fuzz_target;
use oxideav_bmp::{decode_dib, encode_dib, BmpImage, BmpPalette, BmpPixelFormat, BmpPlane};

const MAX_DIM: u32 = 64;

fn pick_format(byte: u8) -> BmpPixelFormat {
    match byte % 8 {
        0 => BmpPixelFormat::Rgba,
        1 => BmpPixelFormat::Rgb24,
        2 => BmpPixelFormat::Rgb555,
        3 => BmpPixelFormat::Rgb565,
        4 => BmpPixelFormat::Indexed8,
        5 => BmpPixelFormat::Indexed4,
        6 => BmpPixelFormat::Indexed2,
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
        BmpPixelFormat::Indexed2 => 4,
        BmpPixelFormat::Indexed1 => 2,
        _ => 0,
    }
}

fuzz_target!(|data: &[u8]| {
    if data.len() < 4 {
        return;
    }
    let format = pick_format(data[0]);
    let ico_mask = data[1] & 1 != 0;
    let width = 1 + (data[2] as u32) % MAX_DIM;
    let height = 1 + (data[3] as u32) % MAX_DIM;
    let tail = &data[4..];

    let bpp = bytes_per_pixel(format);
    let stride = width as usize * bpp;
    let pixel_len = stride * height as usize;
    let mut pixels = vec![0u8; pixel_len];
    if !tail.is_empty() {
        for (i, slot) in pixels.iter_mut().enumerate() {
            *slot = tail[i % tail.len()];
        }
    }

    let palette = {
        let cap = palette_cap(format);
        if cap == 0 {
            None
        } else {
            let pal_src = &tail[pixel_len.min(tail.len())..];
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
        planes: vec![BmpPlane {
            stride,
            data: pixels,
        }],
        palette,
        pts: None,
    };

    let encoded = match encode_dib(&image, ico_mask) {
        Ok(bytes) => bytes,
        Err(_) => return,
    };

    // Matching-flag decode must succeed and round-trip the geometry.
    let decoded = decode_dib(&encoded, ico_mask).expect("encode_dib output must decode back");
    assert_eq!(decoded.width, width, "decoded width mismatch");
    assert_eq!(decoded.height, height, "decoded height mismatch");
    assert_eq!(decoded.pixel_format, BmpPixelFormat::Rgba);

    // Plain 32-bpp BGRA keeps every byte.
    if format == BmpPixelFormat::Rgba && !ico_mask {
        assert_eq!(
            decoded.planes[0].data, image.planes[0].data,
            "Rgba pixels diverged through the plain-DIB path",
        );
    }

    // Opposite-flag decode: any Result is fine, panicking is not.
    let _ = decode_dib(&encoded, !ico_mask);
});
