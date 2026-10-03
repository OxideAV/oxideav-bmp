//! Criterion benchmarks for the BMP encoder hot paths.
//!
//! Round 129 (depth-mode benchmarks): the encoder grew uncompressed
//! 32/24/16-bit and indexed-8 / indexed-4 paths plus the RLE auto-
//! picker (encode-twice + pick-smaller) across rounds 118..r122. The
//! `minimal_palette` and `top_down` options each tweak the output
//! shape too. These benches make those costs measurable so future
//! "Lever N+1" picker tweaks can be A/B-compared against the round-
//! 129 baseline.
//!
//! Scenarios:
//!
//!   - **encode_rgba_320x240**: 320×240 32-bit BGRA `BI_RGB` baseline
//!     encode — the natural-image path that the decode bench mirrors.
//!   - **encode_rgb24_640x480**: 640×480 24-bit BGR `BI_RGB` encode —
//!     a larger VGA case that stresses the 4-byte row-padding fix-up.
//!   - **encode_rgb565_320x240**: 320×240 16-bit `BI_BITFIELDS` 5-6-5
//!     encode — exercises the V4 header + bitfields mask write path.
//!   - **encode_indexed8_random_320x240**: 320×240 8-bit indexed
//!     encode on random data — RLE auto-picker should fall back to
//!     BI_RGB. Measures the "try RLE → fall back" cost.
//!   - **encode_indexed8_rle_friendly_320x240**: 320×240 8-bit
//!     indexed encode on row-constant data — RLE auto-picker should
//!     choose BI_RLE8. Measures the "RLE chosen" path.
//!   - **encode_indexed4_rle_friendly_320x240**: 320×240 4-bit
//!     indexed encode on row-constant data — same shape for RLE4.
//!   - **encode_indexed8_minimal_palette_320x240**: 320×240 8-bit
//!     indexed encode with `EncodeOptions::minimal_palette` —
//!     covers the `biClrUsed`-aware short-table write path.
//!   - **encode_rgba_top_down_320x240**: 320×240 32-bit BGRA encode
//!     with `top_down = true` — exercises the negative-biHeight path.
//!   - **encode_dib_rgba_320x240**: 320×240 RGBA headerless DIB
//!     encode (plain mode, no .ico mask).
//!   - **encode_dib_ico_rgba_64x64**: 64×64 RGBA doubled-height DIB
//!     + 1-bit AND mask encode (the .ico sub-image path).
//!
//! Run with:
//!     cargo bench -p oxideav-bmp --bench encode

// The pre-contract entry points exercised here are the deprecated
// wrappers (IMAGE_CRATE_API migration); this file is their regression
// gate until they are removed.
#![allow(deprecated)]

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};

use oxideav_bmp::{
    encode_bmp, encode_dib, encode_with_report, BmpImage, BmpPixelFormat, EncodeOptions, Palette,
    Plane,
};

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
            data[idx] = data[idx].wrapping_add(xorshift_byte(&mut state) & 0x07);
        }
    }
    BmpImage::new(
        width,
        height,
        BmpPixelFormat::Rgba,
        vec![Plane::new((width as usize) * 4, data)],
    )
    .unwrap()
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
    BmpImage::new(
        width,
        height,
        BmpPixelFormat::Rgb24,
        vec![Plane::new((width as usize) * 3, data)],
    )
    .unwrap()
}

fn build_rgb565_image(width: u32, height: u32) -> BmpImage {
    let mut data = vec![0u8; (width as usize) * (height as usize) * 2];
    for r in 0..height as usize {
        for c in 0..width as usize {
            let base_y = ((r * 31) / (height as usize).max(1)) as u16;
            let base_x = ((c * 31) / (width as usize).max(1)) as u16;
            let g = (base_y + base_x) & 0x3f;
            let px: u16 = (base_x << 11) | (g << 5) | base_y;
            let idx = (r * width as usize + c) * 2;
            data[idx] = (px & 0xff) as u8;
            data[idx + 1] = (px >> 8) as u8;
        }
    }
    BmpImage::new(
        width,
        height,
        BmpPixelFormat::Rgb565,
        vec![Plane::new((width as usize) * 2, data)],
    )
    .unwrap()
}

fn build_indexed8_random(width: u32, height: u32) -> BmpImage {
    let mut data = vec![0u8; (width as usize) * (height as usize)];
    let mut state: u32 = 0x3456_789a;
    for byte in data.iter_mut() {
        *byte = xorshift_byte(&mut state);
    }
    let entries: Vec<[u8; 3]> = (0..256u16)
        .map(|i| [i as u8, (i ^ 0x55) as u8, (i ^ 0xaa) as u8])
        .collect();
    BmpImage::new(
        width,
        height,
        BmpPixelFormat::Pal8,
        vec![Plane::new(width as usize, data)],
    )
    .unwrap()
    .with_palette(Some(Palette::from_rgb(&entries)))
}

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
    BmpImage::new(
        width,
        height,
        BmpPixelFormat::Pal8,
        vec![Plane::new(width as usize, data)],
    )
    .unwrap()
    .with_palette(Some(Palette::from_rgb(&entries)))
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
    BmpImage::new(
        width,
        height,
        BmpPixelFormat::Indexed4,
        vec![Plane::new(width as usize, data)],
    )
    .unwrap()
    .with_palette(Some(Palette::from_rgb(&entries)))
}

fn bench_encode_rgba_320x240(c: &mut Criterion) {
    let image = build_rgba_image(320, 240);
    let mut g = c.benchmark_group("encode_rgba_320x240");
    g.throughput(Throughput::Bytes((320 * 240 * 4) as u64));
    g.bench_function(BenchmarkId::from_parameter("rgba/320x240"), |b| {
        b.iter(|| encode_bmp(criterion::black_box(&image)).expect("encode_bmp"));
    });
    g.finish();
}

fn bench_encode_rgb24_640x480(c: &mut Criterion) {
    let image = build_rgb24_image(640, 480);
    let mut g = c.benchmark_group("encode_rgb24_640x480");
    g.throughput(Throughput::Bytes((640 * 480 * 3) as u64));
    g.sample_size(20);
    g.bench_function(BenchmarkId::from_parameter("rgb24/640x480"), |b| {
        b.iter(|| encode_bmp(criterion::black_box(&image)).expect("encode_bmp"));
    });
    g.finish();
}

fn bench_encode_rgb565_320x240(c: &mut Criterion) {
    let image = build_rgb565_image(320, 240);
    let mut g = c.benchmark_group("encode_rgb565_320x240");
    g.throughput(Throughput::Bytes((320 * 240 * 2) as u64));
    g.bench_function(BenchmarkId::from_parameter("rgb565/320x240"), |b| {
        b.iter(|| encode_bmp(criterion::black_box(&image)).expect("encode_bmp"));
    });
    g.finish();
}

fn bench_encode_indexed8_random_320x240(c: &mut Criterion) {
    let image = build_indexed8_random(320, 240);
    let mut g = c.benchmark_group("encode_indexed8_random_320x240");
    g.throughput(Throughput::Bytes((320 * 240) as u64));
    g.sample_size(20);
    g.bench_function(
        BenchmarkId::from_parameter("indexed8/random/320x240"),
        |b| {
            b.iter(|| encode_bmp(criterion::black_box(&image)).expect("encode_bmp"));
        },
    );
    g.finish();
}

fn bench_encode_indexed8_rle_friendly_320x240(c: &mut Criterion) {
    let image = build_rle_friendly_indexed8(320, 240);
    let mut g = c.benchmark_group("encode_indexed8_rle_friendly_320x240");
    g.throughput(Throughput::Bytes((320 * 240) as u64));
    g.sample_size(20);
    g.bench_function(BenchmarkId::from_parameter("indexed8/rle/320x240"), |b| {
        b.iter(|| encode_bmp(criterion::black_box(&image)).expect("encode_bmp"));
    });
    g.finish();
}

fn bench_encode_indexed4_rle_friendly_320x240(c: &mut Criterion) {
    let image = build_rle_friendly_indexed4(320, 240);
    let mut g = c.benchmark_group("encode_indexed4_rle_friendly_320x240");
    g.throughput(Throughput::Bytes((320 * 240) as u64));
    g.sample_size(20);
    g.bench_function(BenchmarkId::from_parameter("indexed4/rle/320x240"), |b| {
        b.iter(|| encode_bmp(criterion::black_box(&image)).expect("encode_bmp"));
    });
    g.finish();
}

fn bench_encode_indexed8_minimal_palette_320x240(c: &mut Criterion) {
    let image = build_rle_friendly_indexed8(320, 240);
    let opts = EncodeOptions::default().with_minimal_palette(true);
    let mut g = c.benchmark_group("encode_indexed8_minimal_palette_320x240");
    g.throughput(Throughput::Bytes((320 * 240) as u64));
    g.sample_size(20);
    g.bench_function(
        BenchmarkId::from_parameter("indexed8/min-pal/320x240"),
        |b| {
            b.iter(|| encode_with_report(criterion::black_box(&image), &opts).expect("encode_bmp"));
        },
    );
    g.finish();
}

fn bench_encode_rgba_top_down_320x240(c: &mut Criterion) {
    let image = build_rgba_image(320, 240);
    let opts = EncodeOptions::default().with_top_down(true);
    let mut g = c.benchmark_group("encode_rgba_top_down_320x240");
    g.throughput(Throughput::Bytes((320 * 240 * 4) as u64));
    g.bench_function(BenchmarkId::from_parameter("rgba/top-down/320x240"), |b| {
        b.iter(|| encode_with_report(criterion::black_box(&image), &opts).expect("encode_bmp"));
    });
    g.finish();
}

fn bench_encode_dib_rgba_320x240(c: &mut Criterion) {
    let image = build_rgba_image(320, 240);
    let mut g = c.benchmark_group("encode_dib_rgba_320x240");
    g.throughput(Throughput::Bytes((320 * 240 * 4) as u64));
    g.bench_function(BenchmarkId::from_parameter("dib/rgba/320x240"), |b| {
        b.iter(|| encode_dib(criterion::black_box(&image), false).expect("encode_dib"));
    });
    g.finish();
}

fn bench_encode_dib_ico_rgba_64x64(c: &mut Criterion) {
    let image = build_rgba_image(64, 64);
    let mut g = c.benchmark_group("encode_dib_ico_rgba_64x64");
    g.throughput(Throughput::Bytes((64 * 64 * 4) as u64));
    g.bench_function(BenchmarkId::from_parameter("dib/ico/rgba/64x64"), |b| {
        b.iter(|| encode_dib(criterion::black_box(&image), true).expect("encode_dib doubled"));
    });
    g.finish();
}

criterion_group!(
    benches,
    bench_encode_rgba_320x240,
    bench_encode_rgb24_640x480,
    bench_encode_rgb565_320x240,
    bench_encode_indexed8_random_320x240,
    bench_encode_indexed8_rle_friendly_320x240,
    bench_encode_indexed4_rle_friendly_320x240,
    bench_encode_indexed8_minimal_palette_320x240,
    bench_encode_rgba_top_down_320x240,
    bench_encode_dib_rgba_320x240,
    bench_encode_dib_ico_rgba_64x64,
);
criterion_main!(benches);
