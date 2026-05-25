//! Criterion benchmarks for the BMP decoder hot paths.
//!
//! Round 129 (depth-mode benchmarks): oxideav-bmp has hit the per-codec
//! saturation point — every bit depth has a write + read path, RLE4 /
//! RLE8 / BI_BITFIELDS / OS/2 BITMAPCOREHEADER all decode, top-down DIB
//! output is supported, the fuzz harness has been exercised for 7.6M
//! iterations across rounds 123/124. Per the workspace "saturated →
//! fuzz/bench/profile" memo this round wires up `criterion` benches
//! mirroring the cinepak / tta / flac shape so future optimisation
//! rounds can A/B-test changes to the decoder hot paths.
//!
//! This file covers the **decoder**; sibling files cover `encode` and
//! `roundtrip`.
//!
//! Each scenario is self-contained: the bench encodes a fresh BMP on
//! the fly with the public encoder API and then iterates `decode_bmp`
//! on the encoded bytes. No fixture files are committed.
//!
//!   - **decode_rgba_320x240**: 320×240 32-bit BGRA `BI_RGB` decode —
//!     the natural-image baseline matching the bulk of the round-122
//!     PSNR fixtures. Exercises the direct-colour fast path.
//!   - **decode_rgb24_640x480**: 640×480 24-bit BGR `BI_RGB` decode —
//!     a larger VGA fixture that stresses the per-row 4-byte padding
//!     fix-up.
//!   - **decode_rgb565_320x240**: 320×240 16-bit `BI_BITFIELDS` 5-6-5
//!     decode — exercises the mask-derived per-pixel unpack path.
//!   - **decode_indexed8_320x240**: 320×240 8-bit indexed `BI_RGB`
//!     decode — exercises the 256-entry palette expansion path.
//!   - **decode_indexed4_320x240**: 320×240 4-bit indexed `BI_RGB`
//!     decode — exercises the packed-nibble palette expansion path.
//!   - **decode_rle8_320x240**: 320×240 8-bit `BI_RLE8` decode — the
//!     run-length / absolute-mode escape walk.
//!   - **decode_rle4_320x240**: 320×240 4-bit `BI_RLE4` decode — the
//!     packed-nibble run-length path.
//!   - **decode_dib_rgba_320x240**: 320×240 headerless DIB decode (the
//!     `.ico` sub-image path, plain mode).
//!   - **decode_dib_ico_rgba_64x64**: 64×64 doubled-height DIB +
//!     1-bit AND mask decode — the `.ico` sub-image path with mask.
//!
//! Run with:
//!     cargo bench -p oxideav-bmp --bench decode

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};

use oxideav_bmp::{
    decode_bmp, decode_dib, encode_bmp, encode_dib, BmpEncodeOptions, BmpImage, BmpPalette,
    BmpPixelFormat, BmpPlane,
};

/// Cheap deterministic xorshift32 — synthesises "natural-ish" per-pixel
/// values so the bench inputs aren't trivially compressible / branch-
/// predictable.
fn xorshift_byte(state: &mut u32) -> u8 {
    *state ^= *state << 13;
    *state ^= *state >> 17;
    *state ^= *state << 5;
    (*state & 0xff) as u8
}

fn build_rgba_image(width: u32, height: u32) -> BmpImage {
    let mut data = vec![0u8; (width as usize) * (height as usize) * 4];
    let mut state: u32 = 0x1234_5678;
    for r in 0..height as usize {
        for c in 0..width as usize {
            let base_y = ((r * 255) / (height as usize).max(1)) as u32;
            let base_x = ((c * 255) / (width as usize).max(1)) as u32;
            let idx = (r * width as usize + c) * 4;
            data[idx] = (((base_x + base_y) / 2).min(255)) as u8;
            data[idx + 1] = base_y.min(255) as u8;
            data[idx + 2] = base_x.min(255) as u8;
            data[idx + 3] = 0xff;
            // Stir in some xorshift so the encoder doesn't trivially
            // collapse identical pixels.
            data[idx] = data[idx].wrapping_add(xorshift_byte(&mut state) & 0x07);
        }
    }
    BmpImage {
        width,
        height,
        pixel_format: BmpPixelFormat::Rgba,
        planes: vec![BmpPlane {
            stride: (width as usize) * 4,
            data,
        }],
        palette: None,
        pts: None,
    }
}

fn build_rgb24_image(width: u32, height: u32) -> BmpImage {
    let mut data = vec![0u8; (width as usize) * (height as usize) * 3];
    let mut state: u32 = 0x2345_6789;
    for r in 0..height as usize {
        for c in 0..width as usize {
            let base_y = ((r * 255) / (height as usize).max(1)) as u32;
            let base_x = ((c * 255) / (width as usize).max(1)) as u32;
            let idx = (r * width as usize + c) * 3;
            data[idx] = (((base_x + base_y) / 2).min(255)) as u8;
            data[idx + 1] = base_y.min(255) as u8;
            data[idx + 2] = base_x.min(255) as u8;
            data[idx] = data[idx].wrapping_add(xorshift_byte(&mut state) & 0x07);
        }
    }
    BmpImage {
        width,
        height,
        pixel_format: BmpPixelFormat::Rgb24,
        planes: vec![BmpPlane {
            stride: (width as usize) * 3,
            data,
        }],
        palette: None,
        pts: None,
    }
}

fn build_rgb565_image(width: u32, height: u32) -> BmpImage {
    let mut data = vec![0u8; (width as usize) * (height as usize) * 2];
    for r in 0..height as usize {
        for c in 0..width as usize {
            let base_y = ((r * 31) / (height as usize).max(1)) as u16;
            let base_x = ((c * 31) / (width as usize).max(1)) as u16;
            let g = (base_y + base_x) & 0x3f;
            // 5R | 6G | 5B → little-endian u16
            let px: u16 = (base_x << 11) | (g << 5) | base_y;
            let idx = (r * width as usize + c) * 2;
            data[idx] = (px & 0xff) as u8;
            data[idx + 1] = (px >> 8) as u8;
        }
    }
    BmpImage {
        width,
        height,
        pixel_format: BmpPixelFormat::Rgb565,
        planes: vec![BmpPlane {
            stride: (width as usize) * 2,
            data,
        }],
        palette: None,
        pts: None,
    }
}

fn build_indexed8_image(width: u32, height: u32) -> BmpImage {
    let mut data = vec![0u8; (width as usize) * (height as usize)];
    let mut state: u32 = 0x3456_789a;
    for byte in data.iter_mut() {
        *byte = xorshift_byte(&mut state);
    }
    // Full 256-entry palette so the encoder does not have to clamp.
    let entries: Vec<[u8; 3]> = (0..256u16)
        .map(|i| [i as u8, (i ^ 0x55) as u8, (i ^ 0xaa) as u8])
        .collect();
    BmpImage {
        width,
        height,
        pixel_format: BmpPixelFormat::Indexed8,
        planes: vec![BmpPlane {
            stride: width as usize,
            data,
        }],
        palette: Some(BmpPalette { entries }),
        pts: None,
    }
}

fn build_indexed4_image(width: u32, height: u32) -> BmpImage {
    // The Indexed4 plane is byte-per-pixel (the encoder packs nibbles).
    let mut data = vec![0u8; (width as usize) * (height as usize)];
    let mut state: u32 = 0x4567_89ab;
    for byte in data.iter_mut() {
        *byte = xorshift_byte(&mut state) & 0x0f;
    }
    let entries: Vec<[u8; 3]> = (0..16u8)
        .map(|i| [i * 0x11, (i * 0x11) ^ 0x55, (i * 0x11) ^ 0xaa])
        .collect();
    BmpImage {
        width,
        height,
        pixel_format: BmpPixelFormat::Indexed4,
        planes: vec![BmpPlane {
            stride: width as usize,
            data,
        }],
        palette: Some(BmpPalette { entries }),
        pts: None,
    }
}

/// Indexed8 fixture that compresses well under RLE8 — long horizontal
/// runs so the encoder's auto-picker chooses RLE8 over uncompressed.
fn build_rle_friendly_indexed8(width: u32, height: u32) -> BmpImage {
    let mut data = vec![0u8; (width as usize) * (height as usize)];
    for r in 0..height as usize {
        let run_color = (r % 16) as u8;
        for c in 0..width as usize {
            data[r * width as usize + c] = run_color;
        }
    }
    let entries: Vec<[u8; 3]> = (0..256u16)
        .map(|i| [i as u8, (i ^ 0x55) as u8, (i ^ 0xaa) as u8])
        .collect();
    BmpImage {
        width,
        height,
        pixel_format: BmpPixelFormat::Indexed8,
        planes: vec![BmpPlane {
            stride: width as usize,
            data,
        }],
        palette: Some(BmpPalette { entries }),
        pts: None,
    }
}

fn build_rle_friendly_indexed4(width: u32, height: u32) -> BmpImage {
    let mut data = vec![0u8; (width as usize) * (height as usize)];
    for r in 0..height as usize {
        let run_color = (r % 16) as u8;
        for c in 0..width as usize {
            data[r * width as usize + c] = run_color;
        }
    }
    let entries: Vec<[u8; 3]> = (0..16u8)
        .map(|i| [i * 0x11, (i * 0x11) ^ 0x55, (i * 0x11) ^ 0xaa])
        .collect();
    BmpImage {
        width,
        height,
        pixel_format: BmpPixelFormat::Indexed4,
        planes: vec![BmpPlane {
            stride: width as usize,
            data,
        }],
        palette: Some(BmpPalette { entries }),
        pts: None,
    }
}

fn encode_to_bytes(image: &BmpImage) -> Vec<u8> {
    encode_bmp(image).expect("encode_bmp").0
}

fn bench_decode_rgba_320x240(c: &mut Criterion) {
    let image = build_rgba_image(320, 240);
    let bytes = encode_to_bytes(&image);
    let mut g = c.benchmark_group("decode_rgba_320x240");
    g.throughput(Throughput::Bytes((320 * 240 * 4) as u64));
    g.bench_function(BenchmarkId::from_parameter("rgba/320x240"), |b| {
        b.iter(|| decode_bmp(criterion::black_box(&bytes)).expect("decode"));
    });
    g.finish();
}

fn bench_decode_rgb24_640x480(c: &mut Criterion) {
    let image = build_rgb24_image(640, 480);
    let bytes = encode_to_bytes(&image);
    let mut g = c.benchmark_group("decode_rgb24_640x480");
    g.throughput(Throughput::Bytes((640 * 480 * 3) as u64));
    g.sample_size(20);
    g.bench_function(BenchmarkId::from_parameter("rgb24/640x480"), |b| {
        b.iter(|| decode_bmp(criterion::black_box(&bytes)).expect("decode"));
    });
    g.finish();
}

fn bench_decode_rgb565_320x240(c: &mut Criterion) {
    let image = build_rgb565_image(320, 240);
    let bytes = encode_to_bytes(&image);
    let mut g = c.benchmark_group("decode_rgb565_320x240");
    g.throughput(Throughput::Bytes((320 * 240 * 2) as u64));
    g.bench_function(BenchmarkId::from_parameter("rgb565/320x240"), |b| {
        b.iter(|| decode_bmp(criterion::black_box(&bytes)).expect("decode"));
    });
    g.finish();
}

fn bench_decode_indexed8_320x240(c: &mut Criterion) {
    // Random data — encoder's RLE picker should fall back to BI_RGB
    // because RLE wouldn't shrink noise.
    let image = build_indexed8_image(320, 240);
    let bytes = encode_to_bytes(&image);
    let mut g = c.benchmark_group("decode_indexed8_320x240");
    g.throughput(Throughput::Bytes((320 * 240) as u64));
    g.bench_function(BenchmarkId::from_parameter("indexed8/320x240"), |b| {
        b.iter(|| decode_bmp(criterion::black_box(&bytes)).expect("decode"));
    });
    g.finish();
}

fn bench_decode_indexed4_320x240(c: &mut Criterion) {
    let image = build_indexed4_image(320, 240);
    let bytes = encode_to_bytes(&image);
    let mut g = c.benchmark_group("decode_indexed4_320x240");
    g.throughput(Throughput::Bytes((320 * 240) as u64));
    g.bench_function(BenchmarkId::from_parameter("indexed4/320x240"), |b| {
        b.iter(|| decode_bmp(criterion::black_box(&bytes)).expect("decode"));
    });
    g.finish();
}

fn bench_decode_rle8_320x240(c: &mut Criterion) {
    let image = build_rle_friendly_indexed8(320, 240);
    let bytes = encode_to_bytes(&image);
    let mut g = c.benchmark_group("decode_rle8_320x240");
    g.throughput(Throughput::Bytes((320 * 240) as u64));
    g.bench_function(BenchmarkId::from_parameter("rle8/320x240"), |b| {
        b.iter(|| decode_bmp(criterion::black_box(&bytes)).expect("decode"));
    });
    g.finish();
}

fn bench_decode_rle4_320x240(c: &mut Criterion) {
    let image = build_rle_friendly_indexed4(320, 240);
    let bytes = encode_to_bytes(&image);
    let mut g = c.benchmark_group("decode_rle4_320x240");
    g.throughput(Throughput::Bytes((320 * 240) as u64));
    g.bench_function(BenchmarkId::from_parameter("rle4/320x240"), |b| {
        b.iter(|| decode_bmp(criterion::black_box(&bytes)).expect("decode"));
    });
    g.finish();
}

fn bench_decode_dib_rgba_320x240(c: &mut Criterion) {
    let image = build_rgba_image(320, 240);
    let dib = encode_dib(&image, /* doubled */ false).expect("encode_dib");
    let mut g = c.benchmark_group("decode_dib_rgba_320x240");
    g.throughput(Throughput::Bytes((320 * 240 * 4) as u64));
    g.bench_function(BenchmarkId::from_parameter("dib/rgba/320x240"), |b| {
        b.iter(|| decode_dib(criterion::black_box(&dib), false).expect("decode_dib"));
    });
    g.finish();
}

fn bench_decode_dib_ico_rgba_64x64(c: &mut Criterion) {
    let image = build_rgba_image(64, 64);
    let dib = encode_dib(&image, /* doubled */ true).expect("encode_dib doubled");
    let mut g = c.benchmark_group("decode_dib_ico_rgba_64x64");
    g.throughput(Throughput::Bytes((64 * 64 * 4) as u64));
    g.bench_function(BenchmarkId::from_parameter("dib/ico/rgba/64x64"), |b| {
        b.iter(|| decode_dib(criterion::black_box(&dib), true).expect("decode_dib doubled"));
    });
    g.finish();
}

// Acknowledge `BmpEncodeOptions` as part of the public surface (used by
// the encode bench, kept in scope here for the import block to stay
// in sync across benches).
#[allow(dead_code)]
fn _unused_options_marker() -> BmpEncodeOptions {
    BmpEncodeOptions::default()
}

criterion_group!(
    benches,
    bench_decode_rgba_320x240,
    bench_decode_rgb24_640x480,
    bench_decode_rgb565_320x240,
    bench_decode_indexed8_320x240,
    bench_decode_indexed4_320x240,
    bench_decode_rle8_320x240,
    bench_decode_rle4_320x240,
    bench_decode_dib_rgba_320x240,
    bench_decode_dib_ico_rgba_64x64,
);
criterion_main!(benches);
