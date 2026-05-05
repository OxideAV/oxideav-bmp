//! Integration tests that cross-validate BMP encoder output with ImageMagick.
//!
//! Each test encodes a BMP variant, writes it to a temp dir, and then
//! invokes `magick identify` to check it is a well-formed BMP, plus
//! `magick convert` to convert it to PNG and back-decode the PNG to
//! verify pixel values survive the trip.
//!
//! Skipped automatically when ImageMagick is not installed (CI typically
//! has it; dev machines may not).

use oxideav_bmp::{decode_bmp, encode_bmp, BmpImage, BmpPalette, BmpPixelFormat, BmpPlane};
use std::path::PathBuf;
use std::process::Command;

fn magick_available() -> bool {
    Command::new("magick")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn tmp_path(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join("oxideav_bmp_validate");
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

fn magick_identify(path: &PathBuf) -> String {
    let out = Command::new("magick")
        .args(["identify", path.to_str().unwrap()])
        .output()
        .expect("magick identify failed");
    String::from_utf8_lossy(&out.stdout).to_string()
}

fn magick_pixel_rgba(path: &PathBuf, x: u32, y: u32) -> [u8; 4] {
    // Use `magick convert` to extract a single pixel as a PNM-style 8-bit
    // RGBA value.
    let out = Command::new("magick")
        .args([
            path.to_str().unwrap(),
            "-format",
            &format!("%[fx:u.p[{x},{y}].r*255],%[fx:u.p[{x},{y}].g*255],%[fx:u.p[{x},{y}].b*255],%[fx:u.p[{x},{y}].a*255]"),
            "info:",
        ])
        .output()
        .expect("magick pixel probe failed");
    let s = String::from_utf8_lossy(&out.stdout);
    let parts: Vec<f64> = s
        .trim()
        .split(',')
        .map(|v| v.parse().unwrap_or(0.0))
        .collect();
    [
        parts.first().copied().unwrap_or(0.0).round() as u8,
        parts.get(1).copied().unwrap_or(0.0).round() as u8,
        parts.get(2).copied().unwrap_or(0.0).round() as u8,
        parts.get(3).copied().unwrap_or(0.0).round() as u8,
    ]
}

// ---------------------------------------------------------------------------
// 32-bit RGBA
// ---------------------------------------------------------------------------

#[test]
fn magick_32bpp_rgba() {
    if !magick_available() {
        return;
    }

    let w = 8u32;
    let h = 6u32;
    let mut data = Vec::with_capacity((w * h * 4) as usize);
    for _ in 0..h {
        for _ in 0..w {
            data.extend_from_slice(&[255u8, 0, 0, 255]); // solid red RGBA
        }
    }
    let src = BmpImage {
        width: w,
        height: h,
        pixel_format: BmpPixelFormat::Rgba,
        planes: vec![BmpPlane {
            stride: w as usize * 4,
            data,
        }],
        palette: None,
        pts: None,
    };
    let (bytes, _) = encode_bmp(&src).unwrap();
    let path = tmp_path("test_32bpp.bmp");
    std::fs::write(&path, &bytes).unwrap();

    let info = magick_identify(&path);
    assert!(
        info.contains("BMP"),
        "magick did not recognise as BMP: {info}"
    );
    assert!(
        info.contains("8x6") || info.contains("8×6"),
        "wrong dimensions: {info}"
    );

    // Verify top-left pixel via magick.
    let px = magick_pixel_rgba(&path, 0, 0);
    assert_eq!(px[0], 255, "R mismatch: {px:?}");
    assert_eq!(px[1], 0, "G mismatch: {px:?}");
    assert_eq!(px[2], 0, "B mismatch: {px:?}");
}

// ---------------------------------------------------------------------------
// 24-bit BGR
// ---------------------------------------------------------------------------

#[test]
fn magick_24bpp_rgb24() {
    if !magick_available() {
        return;
    }

    let w = 8u32;
    let h = 6u32;
    let mut data = Vec::with_capacity((w * h * 3) as usize);
    for _ in 0..h {
        for _ in 0..w {
            data.extend_from_slice(&[0u8, 255, 0]); // solid green RGB
        }
    }
    let src = BmpImage {
        width: w,
        height: h,
        pixel_format: BmpPixelFormat::Rgb24,
        planes: vec![BmpPlane {
            stride: w as usize * 3,
            data,
        }],
        palette: None,
        pts: None,
    };
    let (bytes, _) = encode_bmp(&src).unwrap();
    let path = tmp_path("test_24bpp.bmp");
    std::fs::write(&path, &bytes).unwrap();

    let info = magick_identify(&path);
    assert!(info.contains("BMP"), "not BMP: {info}");

    let px = magick_pixel_rgba(&path, 0, 0);
    assert_eq!(px[0], 0, "R: {px:?}");
    assert_eq!(px[1], 255, "G: {px:?}");
    assert_eq!(px[2], 0, "B: {px:?}");
}

// ---------------------------------------------------------------------------
// 16-bit RGB565
// ---------------------------------------------------------------------------

#[test]
fn magick_16bpp_rgb565() {
    if !magick_available() {
        return;
    }

    let w = 8u32;
    let h = 6u32;
    // 0x001F = blue in 5-6-5 (B has 5 bits at positions 0-4).
    let pixel = 0x001Fu16.to_le_bytes();
    let stride = w as usize * 2;
    let mut data = Vec::new();
    for _ in 0..(h as usize * w as usize) {
        data.extend_from_slice(&pixel);
    }
    let src = BmpImage {
        width: w,
        height: h,
        pixel_format: BmpPixelFormat::Rgb565,
        planes: vec![BmpPlane { stride, data }],
        palette: None,
        pts: None,
    };
    let (bytes, _) = encode_bmp(&src).unwrap();
    let path = tmp_path("test_16bpp_rgb565.bmp");
    std::fs::write(&path, &bytes).unwrap();

    let info = magick_identify(&path);
    assert!(info.contains("BMP"), "not BMP: {info}");

    // 0x001F → R=0, G=0, B=255.
    let px = magick_pixel_rgba(&path, 0, 0);
    assert_eq!(px[0], 0, "R: {px:?}");
    assert_eq!(px[1], 0, "G: {px:?}");
    assert_eq!(px[2], 255, "B: {px:?}");
}

// ---------------------------------------------------------------------------
// 8-bit indexed
// ---------------------------------------------------------------------------

#[test]
fn magick_8bpp_indexed() {
    if !magick_available() {
        return;
    }

    let w = 8u32;
    let h = 8u32;
    let palette = BmpPalette {
        entries: vec![[255, 0, 0], [0, 0, 255]], // red=0, blue=1
    };
    // Solid red image (all index 0).
    let data = vec![0u8; w as usize * h as usize];
    let src = BmpImage {
        width: w,
        height: h,
        pixel_format: BmpPixelFormat::Indexed8,
        planes: vec![BmpPlane {
            stride: w as usize,
            data,
        }],
        palette: Some(palette),
        pts: None,
    };
    let (bytes, _) = encode_bmp(&src).unwrap();
    let path = tmp_path("test_8bpp_indexed.bmp");
    std::fs::write(&path, &bytes).unwrap();

    let info = magick_identify(&path);
    assert!(info.contains("BMP"), "not BMP: {info}");

    let px = magick_pixel_rgba(&path, 0, 0);
    assert_eq!(px[0], 255, "R (should be red=0): {px:?}");
    assert_eq!(px[1], 0, "G: {px:?}");
    assert_eq!(px[2], 0, "B: {px:?}");
}

// ---------------------------------------------------------------------------
// 4-bit indexed
// ---------------------------------------------------------------------------

#[test]
fn magick_4bpp_indexed() {
    if !magick_available() {
        return;
    }

    let w = 8u32;
    let h = 8u32;
    let palette = BmpPalette {
        entries: vec![[0, 255, 0], [255, 0, 0]], // green=0, red=1
    };
    // Solid green (all index 0).
    let data = vec![0u8; w as usize * h as usize];
    let src = BmpImage {
        width: w,
        height: h,
        pixel_format: BmpPixelFormat::Indexed4,
        planes: vec![BmpPlane {
            stride: w as usize,
            data,
        }],
        palette: Some(palette),
        pts: None,
    };
    let (bytes, _) = encode_bmp(&src).unwrap();
    let path = tmp_path("test_4bpp_indexed.bmp");
    std::fs::write(&path, &bytes).unwrap();

    let info = magick_identify(&path);
    assert!(info.contains("BMP"), "not BMP: {info}");

    let px = magick_pixel_rgba(&path, 0, 0);
    assert_eq!(px[0], 0, "R (should be green): {px:?}");
    assert_eq!(px[1], 255, "G: {px:?}");
    assert_eq!(px[2], 0, "B: {px:?}");
}

// ---------------------------------------------------------------------------
// RLE8: run-heavy image goes through RLE path + magick validates.
// ---------------------------------------------------------------------------

#[test]
fn magick_rle8_encode() {
    if !magick_available() {
        return;
    }

    let w = 32u32;
    let h = 16u32;
    let palette = BmpPalette {
        entries: vec![[255, 0, 0], [0, 255, 0], [0, 0, 255], [128, 128, 128]],
    };
    // Solid red (index 0) — should RLE down nicely.
    let data = vec![0u8; w as usize * h as usize];
    let src = BmpImage {
        width: w,
        height: h,
        pixel_format: BmpPixelFormat::Indexed8,
        planes: vec![BmpPlane {
            stride: w as usize,
            data,
        }],
        palette: Some(palette),
        pts: None,
    };
    let (bytes, fmt) = encode_bmp(&src).unwrap();
    assert_eq!(fmt, oxideav_bmp::EncodedBmpFormat::Rle8);

    let path = tmp_path("test_rle8.bmp");
    std::fs::write(&path, &bytes).unwrap();

    let info = magick_identify(&path);
    assert!(
        info.contains("BMP"),
        "magick does not recognise RLE8 as BMP: {info}"
    );

    // Verify we can decode it back internally and get red pixels.
    let back = decode_bmp(&bytes).unwrap();
    assert_eq!(&back.planes[0].data[..4], &[255u8, 0, 0, 255]);
}

// ---------------------------------------------------------------------------
// RLE4: run-heavy image.
// ---------------------------------------------------------------------------

#[test]
fn magick_rle4_encode() {
    if !magick_available() {
        return;
    }

    let w = 32u32;
    let h = 16u32;
    let palette = BmpPalette {
        entries: vec![[0, 0, 255], [255, 0, 0]], // blue=0, red=1
    };
    // Solid blue (index 0).
    let data = vec![0u8; w as usize * h as usize];
    let src = BmpImage {
        width: w,
        height: h,
        pixel_format: BmpPixelFormat::Indexed4,
        planes: vec![BmpPlane {
            stride: w as usize,
            data,
        }],
        palette: Some(palette),
        pts: None,
    };
    let (bytes, fmt) = encode_bmp(&src).unwrap();
    assert_eq!(fmt, oxideav_bmp::EncodedBmpFormat::Rle4);

    let path = tmp_path("test_rle4.bmp");
    std::fs::write(&path, &bytes).unwrap();

    let info = magick_identify(&path);
    assert!(
        info.contains("BMP"),
        "magick does not recognise RLE4 as BMP: {info}"
    );

    let back = decode_bmp(&bytes).unwrap();
    assert_eq!(&back.planes[0].data[..4], &[0u8, 0, 255, 255]);
}
