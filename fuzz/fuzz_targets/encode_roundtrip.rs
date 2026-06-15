#![no_main]

//! Encoder-roundtrip fuzz target for `oxideav-bmp`.
//!
//! The two existing harnesses (`decode`, `rle_stream`) feed arbitrary bytes
//! to the *decoder* surface. This third harness comes from the other side:
//! the fuzzer drives the *encoder* with attacker-controlled input pixels +
//! palette + encode-options, and then immediately decodes the encoder's
//! output. The contract under test is two-pronged:
//!
//!   1. **No-panic.** Neither `encode_bmp_with_options` nor the follow-up
//!      `decode_bmp` may panic, integer-overflow (in debug builds), index
//!      out of bounds, or OOM-abort on any input the fuzzer produces.
//!      Errors are fine; crashes are not.
//!
//!   2. **Lossless roundtrip for the direct-colour formats.** For
//!      `Rgba` and `Rgb24` input, the decoder reproduces every pixel byte
//!      the encoder wrote: alpha (or the synthesised `0xFF` for Rgb24) and
//!      the three colour channels both survive the BGR↔RGB swap and the
//!      bottom-up / top-down row flip. Mismatch is an assertion failure
//!      that surfaces to libfuzzer as a crash.
//!
//! Indexed and 16-bit `Rgb565` paths are panic-checked but not
//! byte-compared: the decoder always materialises `Rgba`, so an indexed
//! roundtrip compares a 1 B/px index stream against a 4 B/px expanded
//! image — a different shape. We assert the decoded dimensions instead.
//!
//! ## Wire framing of the fuzz input
//!
//! The fuzzer's bytes are sliced into a small header + pixel payload:
//!
//!   * byte 0 — format selector (low 3 bits): 0=Rgba, 1=Rgb24, 2=Rgb565,
//!     3=Indexed8, 4=Indexed4, 5=Indexed1, 6=Rgb555, 7 wraps to Rgb24.
//!   * byte 1 — encode options: bit 0 = `top_down`, bit 1 =
//!     `minimal_palette`.
//!   * byte 2 — width in pixels, clamped to 1..=64. The cap keeps memory
//!     bounded at roughly `64×64×4 = 16 KiB` per iteration so the harness
//!     does not OOM the fuzz worker, and keeps the encoder's per-row
//!     padding maths well-exercised across the [1, 64] range.
//!   * byte 3 — height in pixels, clamped to 1..=64.
//!   * bytes 4..N — pixel payload, sized / cycled to fill the
//!     `width × height × bytes_per_pixel` plane.
//!   * trailing bytes — palette entries for indexed formats: chunks of
//!     three bytes become `[R, G, B]` until the palette size cap for the
//!     selected format is hit (256 / 16 / 2). A short tail is padded
//!     with `[0, 0, 0]` so the palette is always large enough for any
//!     index value the pixel bytes can carry.
//!
//! ## Why bounded geometry
//!
//! BMP's worst-case allocation is `width × height × 4` for the decoder's
//! Rgba output buffer. Without an in-fuzz cap the fuzzer would happily
//! pick `width = height = u32::MAX` from the first few bytes and OOM the
//! worker before reaching any interesting state. A 64×64 cap keeps each
//! iteration under 16 KiB of plane data and lets libfuzzer mutate the
//! pixel bytes through millions of variations per second.

use libfuzzer_sys::fuzz_target;
use oxideav_bmp::{
    decode_bmp, encode_bmp_with_options, BmpEncodeOptions, BmpImage, BmpPalette, BmpPixelFormat,
    BmpPlane, EncodedBmpFormat,
};

/// Maximum picture dimension in pixels. See module docs for rationale.
const MAX_DIM: u32 = 64;

/// Bytes per pixel as the encoder's plane API expects them.
///
/// Note that `Indexed1` consumes one byte per pixel on input (the encoder
/// packs it into the on-disk MSB-first layout); the on-disk stream is
/// 1 bit per pixel but the in-memory plane is one full byte per index.
fn bytes_per_pixel(format: BmpPixelFormat) -> usize {
    match format {
        BmpPixelFormat::Rgba => 4,
        BmpPixelFormat::Rgb24 => 3,
        BmpPixelFormat::Rgb555 | BmpPixelFormat::Rgb565 => 2,
        BmpPixelFormat::Indexed8 | BmpPixelFormat::Indexed4 | BmpPixelFormat::Indexed1 => 1,
    }
}

/// Maximum palette entry count for an indexed format.
fn palette_cap(format: BmpPixelFormat) -> usize {
    match format {
        BmpPixelFormat::Indexed8 => 256,
        BmpPixelFormat::Indexed4 => 16,
        BmpPixelFormat::Indexed1 => 2,
        _ => 0,
    }
}

/// Map the format selector byte onto a [`BmpPixelFormat`]. Code 6
/// selects `Rgb555` and code 7 wraps back to `Rgb24` to keep the
/// distribution roughly even across the seven encodable formats.
fn pick_format(byte: u8) -> BmpPixelFormat {
    match byte & 0b111 {
        0 => BmpPixelFormat::Rgba,
        1 => BmpPixelFormat::Rgb24,
        2 => BmpPixelFormat::Rgb565,
        3 => BmpPixelFormat::Indexed8,
        4 => BmpPixelFormat::Indexed4,
        5 => BmpPixelFormat::Indexed1,
        6 => BmpPixelFormat::Rgb555,
        _ => BmpPixelFormat::Rgb24,
    }
}

/// Coerce a clamped index value into the legal range for the format. The
/// encoder accepts any byte for `Indexed8` (the full range is valid), so
/// no masking is needed there; `Indexed4` and `Indexed1` take only the
/// low nibble / low bit respectively, but the encoder already does the
/// mask so the harness can pass arbitrary bytes through unchanged.
fn mask_index_byte(byte: u8, _format: BmpPixelFormat) -> u8 {
    byte
}

/// Build the pixel plane by cycling the fuzz-provided pixel bytes to fill
/// `width × height × bytes_per_pixel`. A zero-length input yields a zero
/// plane (the encoder still runs against an all-zero pixel grid). The
/// returned plane carries the natural unpadded stride; the encoder's
/// internal 4-byte row padding is independent of this in-memory layout.
fn make_plane(
    pixel_bytes: &[u8],
    width: u32,
    height: u32,
    format: BmpPixelFormat,
) -> Option<BmpPlane> {
    let bpp = bytes_per_pixel(format);
    let stride = (width as usize).checked_mul(bpp)?;
    let total = stride.checked_mul(height as usize)?;
    let mut data = vec![0u8; total];
    if !pixel_bytes.is_empty() {
        for (i, slot) in data.iter_mut().enumerate() {
            *slot = mask_index_byte(pixel_bytes[i % pixel_bytes.len()], format);
        }
    }
    Some(BmpPlane { stride, data })
}

/// Build a palette from the trailing fuzz bytes, three bytes per entry,
/// padded with `[0, 0, 0]` so the table never has fewer entries than the
/// pixel data could index. Returns an empty palette for non-indexed
/// formats; the encoder ignores `palette` in those modes.
fn make_palette(tail: &[u8], format: BmpPixelFormat) -> Option<BmpPalette> {
    let cap = palette_cap(format);
    if cap == 0 {
        return None;
    }
    let mut entries = Vec::with_capacity(cap);
    let mut chunks = tail.chunks_exact(3);
    for chunk in chunks.by_ref() {
        if entries.len() == cap {
            break;
        }
        entries.push([chunk[0], chunk[1], chunk[2]]);
    }
    while entries.len() < cap {
        entries.push([0, 0, 0]);
    }
    Some(BmpPalette { entries })
}

/// Decode the encoder's bytes back into an `Rgba` image. The result is
/// compared against the input for `Rgba` / `Rgb24`; for everything else
/// the call is just a no-panic check on the decoder side of the
/// roundtrip pair.
fn check_roundtrip(
    encoded: &[u8],
    written_format: EncodedBmpFormat,
    src: &BmpImage,
    options: BmpEncodeOptions,
) {
    let decoded = decode_bmp(encoded).expect("encoder output failed to decode");
    assert_eq!(decoded.width, src.width, "decoded width mismatch");
    assert_eq!(decoded.height, src.height, "decoded height mismatch");
    assert_eq!(decoded.pixel_format, BmpPixelFormat::Rgba);
    assert_eq!(decoded.planes.len(), 1);

    // Lossless byte-level comparison applies only to the two direct-colour
    // formats; the indexed and 16-bit paths transform the input shape.
    match src.pixel_format {
        BmpPixelFormat::Rgba => {
            assert_eq!(written_format, EncodedBmpFormat::Rgb32);
            assert_eq!(
                decoded.planes[0].data, src.planes[0].data,
                "Rgba roundtrip diverged (top_down={}, minimal_palette={})",
                options.top_down, options.minimal_palette,
            );
        }
        BmpPixelFormat::Rgb24 => {
            assert_eq!(written_format, EncodedBmpFormat::Rgb24);
            // The decoder synthesises alpha = 0xFF for Rgb24 input.
            let src_stride = src.planes[0].stride;
            let dec_stride = decoded.planes[0].stride;
            let w = src.width as usize;
            let h = src.height as usize;
            for y in 0..h {
                for x in 0..w {
                    let s = &src.planes[0].data[y * src_stride + x * 3..][..3];
                    let d = &decoded.planes[0].data[y * dec_stride + x * 4..][..4];
                    assert_eq!(d[0], s[0], "R diverged at ({x},{y})");
                    assert_eq!(d[1], s[1], "G diverged at ({x},{y})");
                    assert_eq!(d[2], s[2], "B diverged at ({x},{y})");
                    assert_eq!(d[3], 0xFF, "alpha not synthesised at ({x},{y})");
                }
            }
        }
        // Rgb565 quantises 8-bit → 5/6/5-bit, so the roundtrip is lossy
        // by design; only check that the decoded buffer is the right shape.
        // Indexed formats expand 1 B/px palette indices to 4 B/px Rgba, so
        // the same applies — comparing the streams byte-for-byte would
        // compare apples to oranges.
        _ => {
            assert_eq!(
                decoded.planes[0].data.len(),
                (src.width as usize) * (src.height as usize) * 4,
                "decoded Rgba buffer wrong size",
            );
        }
    }
}

fuzz_target!(|data: &[u8]| {
    if data.len() < 4 {
        return;
    }
    let format = pick_format(data[0]);
    let opts_byte = data[1];
    let width = (data[2] as u32).clamp(1, MAX_DIM);
    let height = (data[3] as u32).clamp(1, MAX_DIM);

    let bpp = bytes_per_pixel(format);
    let plane_bytes = (width as usize) * (height as usize) * bpp;
    let tail = &data[4..];
    let (pixel_bytes, palette_tail) = if tail.len() >= plane_bytes {
        tail.split_at(plane_bytes)
    } else {
        (tail, &[][..])
    };

    let plane = match make_plane(pixel_bytes, width, height, format) {
        Some(p) => p,
        None => return,
    };
    let palette = make_palette(palette_tail, format);

    let image = BmpImage {
        width,
        height,
        pixel_format: format,
        planes: vec![plane],
        palette,
        pts: None,
    };

    let options = BmpEncodeOptions {
        top_down: opts_byte & 0b01 != 0,
        minimal_palette: opts_byte & 0b10 != 0,
    };

    let (bytes, written_format) = match encode_bmp_with_options(&image, options) {
        Ok(pair) => pair,
        Err(_) => return,
    };

    // The first two bytes are always the `BM` signature when encode
    // returns success; a successful encode that produced a non-BMP blob
    // would be a contract bug worth surfacing.
    assert_eq!(&bytes[..2], b"BM", "encoder emitted non-BMP signature");

    check_roundtrip(&bytes, written_format, &image, options);
});
