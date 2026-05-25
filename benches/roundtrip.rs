//! Criterion benchmarks for the BMP encoder + decoder roundtrip — the
//! realistic "encode an image, decode it back" path that an end-to-end
//! consumer (e.g. an ICO or thumbnail muxer) exercises.
//!
//! Round 129 (depth-mode benchmarks): pairs each bit-depth's encode
//! path with its decode path so future "Lever N+1" changes can be
//! A/B-compared at the pipeline level (not just one half). Each
//! scenario re-decodes the encoder output, so a perf regression that
//! quietly mis-encodes will show up as a panic rather than a
//! silently-cheaper benchmark number.
//!
//! Scenarios:
//!
//!   - **roundtrip_rgba_320x240**: 320×240 32-bit BGRA encode → decode.
//!   - **roundtrip_rgb24_640x480**: 640×480 24-bit BGR encode → decode.
//!   - **roundtrip_rgb565_320x240**: 320×240 16-bit `BI_BITFIELDS`
//!     5-6-5 encode → decode.
//!   - **roundtrip_indexed8_rle_320x240**: 320×240 8-bit indexed
//!     RLE-friendly encode → RLE8 decode.
//!   - **roundtrip_indexed4_rle_320x240**: 320×240 4-bit indexed
//!     RLE-friendly encode → RLE4 decode.
//!   - **roundtrip_dib_ico_rgba_64x64**: 64×64 RGBA `.ico` sub-image
//!     DIB roundtrip (doubled-height + AND mask).
//!
//! Run with:
//!     cargo bench -p oxideav-bmp --bench roundtrip

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};

use oxideav_bmp::{
    decode_bmp, decode_dib, encode_bmp, encode_dib, BmpImage, BmpPalette, BmpPixelFormat, BmpPlane,
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

fn bench_roundtrip_rgba_320x240(c: &mut Criterion) {
    let image = build_rgba_image(320, 240);
    let mut g = c.benchmark_group("roundtrip_rgba_320x240");
    g.throughput(Throughput::Bytes((320 * 240 * 4) as u64));
    g.bench_function(BenchmarkId::from_parameter("rgba/320x240"), |b| {
        b.iter(|| {
            let (bytes, _) = encode_bmp(criterion::black_box(&image)).expect("encode_bmp");
            decode_bmp(criterion::black_box(&bytes)).expect("decode")
        });
    });
    g.finish();
}

fn bench_roundtrip_rgb24_640x480(c: &mut Criterion) {
    let image = build_rgb24_image(640, 480);
    let mut g = c.benchmark_group("roundtrip_rgb24_640x480");
    g.throughput(Throughput::Bytes((640 * 480 * 3) as u64));
    g.sample_size(20);
    g.bench_function(BenchmarkId::from_parameter("rgb24/640x480"), |b| {
        b.iter(|| {
            let (bytes, _) = encode_bmp(criterion::black_box(&image)).expect("encode_bmp");
            decode_bmp(criterion::black_box(&bytes)).expect("decode")
        });
    });
    g.finish();
}

fn bench_roundtrip_rgb565_320x240(c: &mut Criterion) {
    let image = build_rgb565_image(320, 240);
    let mut g = c.benchmark_group("roundtrip_rgb565_320x240");
    g.throughput(Throughput::Bytes((320 * 240 * 2) as u64));
    g.bench_function(BenchmarkId::from_parameter("rgb565/320x240"), |b| {
        b.iter(|| {
            let (bytes, _) = encode_bmp(criterion::black_box(&image)).expect("encode_bmp");
            decode_bmp(criterion::black_box(&bytes)).expect("decode")
        });
    });
    g.finish();
}

fn bench_roundtrip_indexed8_rle_320x240(c: &mut Criterion) {
    let image = build_rle_friendly_indexed8(320, 240);
    let mut g = c.benchmark_group("roundtrip_indexed8_rle_320x240");
    g.throughput(Throughput::Bytes((320 * 240) as u64));
    g.sample_size(20);
    g.bench_function(BenchmarkId::from_parameter("indexed8/rle/320x240"), |b| {
        b.iter(|| {
            let (bytes, _) = encode_bmp(criterion::black_box(&image)).expect("encode_bmp");
            decode_bmp(criterion::black_box(&bytes)).expect("decode")
        });
    });
    g.finish();
}

fn bench_roundtrip_indexed4_rle_320x240(c: &mut Criterion) {
    let image = build_rle_friendly_indexed4(320, 240);
    let mut g = c.benchmark_group("roundtrip_indexed4_rle_320x240");
    g.throughput(Throughput::Bytes((320 * 240) as u64));
    g.sample_size(20);
    g.bench_function(BenchmarkId::from_parameter("indexed4/rle/320x240"), |b| {
        b.iter(|| {
            let (bytes, _) = encode_bmp(criterion::black_box(&image)).expect("encode_bmp");
            decode_bmp(criterion::black_box(&bytes)).expect("decode")
        });
    });
    g.finish();
}

fn bench_roundtrip_dib_ico_rgba_64x64(c: &mut Criterion) {
    let image = build_rgba_image(64, 64);
    let mut g = c.benchmark_group("roundtrip_dib_ico_rgba_64x64");
    g.throughput(Throughput::Bytes((64 * 64 * 4) as u64));
    g.bench_function(BenchmarkId::from_parameter("dib/ico/rgba/64x64"), |b| {
        b.iter(|| {
            let dib = encode_dib(criterion::black_box(&image), true).expect("encode_dib doubled");
            decode_dib(criterion::black_box(&dib), true).expect("decode_dib doubled")
        });
    });
    g.finish();
}

criterion_group!(
    benches,
    bench_roundtrip_rgba_320x240,
    bench_roundtrip_rgb24_640x480,
    bench_roundtrip_rgb565_320x240,
    bench_roundtrip_indexed8_rle_320x240,
    bench_roundtrip_indexed4_rle_320x240,
    bench_roundtrip_dib_ico_rgba_64x64,
);
criterion_main!(benches);
