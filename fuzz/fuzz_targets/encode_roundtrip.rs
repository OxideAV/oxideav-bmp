#![no_main]

//! Encoder-roundtrip fuzz target for `oxideav-bmp`.
//!
//! The two existing harnesses (`decode`, `rle_stream`) feed arbitrary bytes
//! to the *decoder* surface. This third harness comes from the other side:
//! the fuzzer drives the *encoder* with attacker-controlled input pixels +
//! palette + encode-options, and then immediately decodes the encoder's
//! output. The contract under test is two-pronged:
//!
//!   1. **No-panic.** Neither `encode_with_report` nor the follow-up
//!      `decode` may panic, integer-overflow (in debug builds), index
//!      out of bounds, or OOM-abort on any input the fuzzer produces.
//!      Errors are fine; crashes are not.
//!
//!   2. **Lossless roundtrip.** `decode(encode(img))` reproduces the
//!      picture for every layout: `to_rgba8()` of source and decoded
//!      image match byte for byte, and the native layouts (`Bgra`,
//!      `Bgr24`, `Rgb555`, `Rgb565`, `Pal8`) also come back with the same
//!      `format`, plane bytes and palette. `Rgba` / `Rgb24` input is
//!      re-laid as `Bgra` / `Bgr24` on disk and the sub-byte indexed
//!      selectors come back as `Pal8`, so for those only the pixel values
//!      are compared. Mismatch is an assertion failure that surfaces to
//!      libfuzzer as a crash.
//!
//! ## Wire framing of the fuzz input
//!
//! The fuzzer's bytes are sliced into a small header + pixel payload:
//!
//!   * byte 0 — format selector (`byte % 10`): 0=Rgba, 1=Rgb24, 2=Rgb565,
//!     3=Pal8, 4=Indexed4, 5=Indexed2, 6=Indexed1, 7=Bgra, 8=Bgr24,
//!     9=Rgb555.
//!   * byte 1 — encode options: bit 0 = `top_down`, bit 1 =
//!     `minimal_palette`, bit 2 = disable `rle`.
//!   * byte 2 — width in pixels, clamped to 1..=64. The cap keeps memory
//!     bounded at roughly `64×64×4 = 16 KiB` per iteration so the harness
//!     does not OOM the fuzz worker, and keeps the encoder's per-row
//!     padding maths well-exercised across the [1, 64] range.
//!   * byte 3 — height in pixels, clamped to 1..=64.
//!   * bytes 4..N — pixel payload, sized / cycled to fill the
//!     `width × height × bytes_per_pixel` plane.
//!   * trailing bytes — palette entries for indexed formats: chunks of
//!     three bytes become `[R, G, B]` until the palette size cap for the
//!     selected format is hit (256 / 16 / 4 / 2). A short tail is padded
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
    decode, encode_with_report, BmpImage, BmpPixelFormat, EncodeOptions, EncodedBmpFormat,
    Palette, Plane,
};

/// Maximum picture dimension in pixels. See module docs for rationale.
const MAX_DIM: u32 = 64;

/// Bytes per pixel as the encoder's plane API expects them.
///
/// Note that `Indexed1` consumes one byte per pixel on input (the encoder
/// packs it into the on-disk MSB-first layout); the on-disk stream is
/// 1 bit per pixel but the in-memory plane is one full byte per index.
fn bytes_per_pixel(format: BmpPixelFormat) -> usize {
    format.bytes_per_pixel()
}

/// Maximum palette entry count for an indexed format.
fn palette_cap(format: BmpPixelFormat) -> usize {
    match format {
        BmpPixelFormat::Pal8 => 256,
        BmpPixelFormat::Indexed4 => 16,
        BmpPixelFormat::Indexed2 => 4,
        BmpPixelFormat::Indexed1 => 2,
        _ => 0,
    }
}

/// Map the format selector byte onto a [`BmpPixelFormat`] across all
/// ten encodable layouts.
fn pick_format(byte: u8) -> BmpPixelFormat {
    match byte % 10 {
        0 => BmpPixelFormat::Rgba,
        1 => BmpPixelFormat::Rgb24,
        2 => BmpPixelFormat::Rgb565,
        3 => BmpPixelFormat::Pal8,
        4 => BmpPixelFormat::Indexed4,
        5 => BmpPixelFormat::Indexed2,
        6 => BmpPixelFormat::Indexed1,
        7 => BmpPixelFormat::Bgra,
        8 => BmpPixelFormat::Bgr24,
        _ => BmpPixelFormat::Rgb555,
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
) -> Option<Plane> {
    let bpp = bytes_per_pixel(format);
    let stride = (width as usize).checked_mul(bpp)?;
    let total = stride.checked_mul(height as usize)?;
    let mut data = vec![0u8; total];
    if !pixel_bytes.is_empty() {
        for (i, slot) in data.iter_mut().enumerate() {
            *slot = mask_index_byte(pixel_bytes[i % pixel_bytes.len()], format);
        }
    }
    Some(Plane::new(stride, data))
}

/// Build a palette from the trailing fuzz bytes, three bytes per entry,
/// padded with `[0, 0, 0]` so the table never has fewer entries than the
/// pixel data could index. Returns an empty palette for non-indexed
/// formats; the encoder ignores `palette` in those modes.
fn make_palette(tail: &[u8], format: BmpPixelFormat) -> Option<Palette> {
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
    Some(Palette::from_rgb(&entries))
}

/// Decode the encoder's bytes back and hold the contract's lossless
/// promise: `decode(encode(img))` reproduces the picture exactly.
///
/// * every layout: `to_rgba8()` of the decoded image equals that of the
///   source (the 16-bit words, the BGR bytes and the palette indices
///   are stored verbatim, so no quantisation happens anywhere);
/// * the native layouts (`Bgra`, `Bgr24`, `Rgb555`, `Rgb565`, `Pal8`)
///   additionally come back with the same `format`, plane bytes and
///   (for `Pal8`) palette — `Rgba` / `Rgb24` are re-laid as
///   `Bgra` / `Bgr24` on disk, and the sub-byte indexed selectors come
///   back as `Pal8`, so only the pixel values are compared for those.
fn check_roundtrip(
    encoded: &[u8],
    written_format: EncodedBmpFormat,
    src: &BmpImage,
    options: &EncodeOptions,
) {
    let decoded = decode(encoded).expect("encoder output failed to decode");
    assert_eq!(decoded.width, src.width, "decoded width mismatch");
    assert_eq!(decoded.height, src.height, "decoded height mismatch");
    assert_eq!(decoded.planes.len(), 1);

    let expected_token = match src.format {
        BmpPixelFormat::Rgba | BmpPixelFormat::Bgra => Some(EncodedBmpFormat::Rgb32),
        BmpPixelFormat::Rgb24 | BmpPixelFormat::Bgr24 => Some(EncodedBmpFormat::Rgb24),
        BmpPixelFormat::Rgb555 => Some(EncodedBmpFormat::Rgb16Rgb),
        BmpPixelFormat::Rgb565 => Some(EncodedBmpFormat::Rgb16Bitfields),
        _ => None, // indexed: RLE-or-raw is the encoder's call
    };
    if let Some(t) = expected_token {
        assert_eq!(written_format, t, "unexpected on-disk variant");
    }

    assert_eq!(
        decoded.to_rgba8(),
        src.to_rgba8(),
        "pixels diverged for {:?} (top_down={}, minimal_palette={}, rle={})",
        src.format,
        options.top_down,
        options.minimal_palette,
        options.rle,
    );

    match src.format {
        BmpPixelFormat::Bgra
        | BmpPixelFormat::Bgr24
        | BmpPixelFormat::Rgb555
        | BmpPixelFormat::Rgb565
        | BmpPixelFormat::Pal8 => {
            assert_eq!(decoded.format, src.format, "native layout changed");
            assert_eq!(decoded.planes, src.planes, "native plane diverged");
            if src.format == BmpPixelFormat::Pal8 {
                assert_eq!(decoded.palette, src.palette, "palette diverged");
            }
        }
        BmpPixelFormat::Rgba => assert_eq!(decoded.format, BmpPixelFormat::Bgra),
        BmpPixelFormat::Rgb24 => assert_eq!(decoded.format, BmpPixelFormat::Bgr24),
        _ => assert_eq!(decoded.format, BmpPixelFormat::Pal8),
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

    let image = BmpImage::new(width, height, format, vec![plane]).unwrap().with_palette(palette);

    let options = EncodeOptions::default()
        .with_top_down(opts_byte & 0b01 != 0)
        .with_minimal_palette(opts_byte & 0b10 != 0)
        .with_rle(opts_byte & 0b100 == 0);

    let (bytes, written_format) = match encode_with_report(&image, &options) {
        Ok(pair) => pair,
        Err(_) => return,
    };

    // The first two bytes are always the `BM` signature when encode
    // returns success; a successful encode that produced a non-BMP blob
    // would be a contract bug worth surfacing.
    assert_eq!(&bytes[..2], b"BM", "encoder emitted non-BMP signature");

    check_roundtrip(&bytes, written_format, &image, &options);
});
