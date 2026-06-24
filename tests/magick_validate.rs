//! Integration tests that cross-validate BMP encoder output with the
//! `magick` CLI validator as an opaque black-box process.
//!
//! Each test encodes a BMP variant, writes it to a temp dir, and then
//! invokes `magick identify` to check it is a well-formed BMP, plus
//! `magick convert` to convert it to PNG and back-decode the PNG to
//! verify pixel values survive the trip.
//!
//! Skipped automatically when the `magick` binary is not on `PATH` (CI
//! typically ships it; dev machines may not).

use oxideav_bmp::{
    decode_bmp, encode_bmp, encode_bmp_with_options, BmpEncodeOptions, BmpImage, BmpPalette,
    BmpPixelFormat, BmpPlane, EncodedBmpFormat,
};
use std::path::{Path, PathBuf};
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

fn magick_identify(path: &Path) -> String {
    let out = Command::new("magick")
        .args(["identify", path.to_str().unwrap()])
        .output()
        .expect("magick identify failed");
    String::from_utf8_lossy(&out.stdout).to_string()
}

fn magick_pixel_rgba(path: &Path, x: u32, y: u32) -> [u8; 4] {
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
// RLE8 with a delta-skip: the cells the delta jumps over must resolve to
// colour index 0 — independently corroborated against magick. (Our
// encoder never emits deltas, so this hand-builds the stream.)
// ---------------------------------------------------------------------------

#[test]
fn magick_rle8_delta_skip_fills_index0() {
    if !magick_available() {
        return;
    }

    // 4×2, four-entry palette. Index 0 is an opaque magenta the skipped
    // cells must take; index 1 = green, index 2 = blue.
    fn le_u32(v: u32) -> [u8; 4] {
        v.to_le_bytes()
    }
    let off: u32 = 14 + 40 + 4 * 4; // file + info header + 4 RGBQUADs
                                    // RLE: idx1 at (0,bottom); delta +2x,+1y → (3,top); idx2; end.
    let rle: [u8; 10] = [
        0x01, 0x01, // run 1 × idx1
        0x00, 0x02, 0x02, 0x01, // delta +2 x, +1 y
        0x01, 0x02, // run 1 × idx2
        0x00, 0x01, // end of bitmap
    ];
    let file_size = off + rle.len() as u32;
    let mut bmp = Vec::new();
    // BITMAPFILEHEADER — magick validates bfSize against the actual file
    // length, so it must be correct (not the 0 informational sentinel).
    bmp.extend_from_slice(b"BM");
    bmp.extend_from_slice(&le_u32(file_size));
    bmp.extend_from_slice(&0u16.to_le_bytes());
    bmp.extend_from_slice(&0u16.to_le_bytes());
    bmp.extend_from_slice(&le_u32(off));
    // BITMAPINFOHEADER (positive height → bottom-up, required for RLE)
    bmp.extend_from_slice(&le_u32(40));
    bmp.extend_from_slice(&4i32.to_le_bytes()); // width
    bmp.extend_from_slice(&2i32.to_le_bytes()); // height
    bmp.extend_from_slice(&1u16.to_le_bytes()); // planes
    bmp.extend_from_slice(&8u16.to_le_bytes()); // bpp
    bmp.extend_from_slice(&le_u32(1)); // BI_RLE8
    bmp.extend_from_slice(&le_u32(rle.len() as u32)); // image size
    bmp.extend_from_slice(&0i32.to_le_bytes());
    bmp.extend_from_slice(&0i32.to_le_bytes());
    bmp.extend_from_slice(&le_u32(4)); // clr_used
    bmp.extend_from_slice(&le_u32(0));
    // Palette (BGRA): idx0 magenta, idx1 green, idx2 blue, idx3 grey.
    bmp.extend_from_slice(&[180, 10, 200, 0]); // magenta R200 G10 B180
    bmp.extend_from_slice(&[0, 255, 0, 0]); // green
    bmp.extend_from_slice(&[255, 0, 0, 0]); // blue
    bmp.extend_from_slice(&[128, 128, 128, 0]); // grey
    bmp.extend_from_slice(&rle);

    let path = tmp_path("test_rle8_delta.bmp");
    std::fs::write(&path, &bmp).unwrap();

    let info = magick_identify(&path);
    assert!(info.contains("BMP"), "magick did not recognise: {info}");

    // A skipped cell on the bottom row (image y = 1, e.g. x = 2) must be
    // index-0 magenta in BOTH our decoder and magick.
    let ours = decode_bmp(&bmp).expect("delta RLE8 must decode");
    // Bottom output row is y = 1 (top-down output); x = 2 is skipped.
    let px = &ours.planes[0].data[16 + 2 * 4..16 + 2 * 4 + 4];
    assert_eq!(px, &[200, 10, 180, 255], "our skipped cell must be index 0");

    let m = magick_pixel_rgba(&path, 2, 1);
    assert_eq!(
        &m[..3],
        &[200, 10, 180],
        "magick's skipped cell must also be index 0 (magenta)"
    );
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

// ---------------------------------------------------------------------------
// Minimal palette: biClrUsed-trimmed colour table must read correctly.
// ---------------------------------------------------------------------------

#[test]
fn magick_minimal_palette_8bpp() {
    if !magick_available() {
        return;
    }

    let w = 8u32;
    let h = 8u32;
    let palette = BmpPalette {
        entries: vec![[255, 0, 0], [0, 0, 255]], // 2-entry table; red=0, blue=1
    };
    // Force the uncompressed indexed path (alternate rows ⇒ poor RLE) so
    // we exercise an explicit biClrUsed=2 colour table, not an RLE stream.
    let mut data = Vec::with_capacity(w as usize * h as usize);
    for _ in 0..h {
        for x in 0..w {
            data.push((x & 1) as u8);
        }
    }
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
    let (bytes, fmt) = encode_bmp_with_options(
        &src,
        BmpEncodeOptions {
            minimal_palette: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(fmt, EncodedBmpFormat::Indexed8);

    // biClrUsed (file offset 46) records the 2-entry table.
    let clr_used = u32::from_le_bytes([bytes[46], bytes[47], bytes[48], bytes[49]]);
    assert_eq!(clr_used, 2);

    let path = tmp_path("test_min_palette_8bpp.bmp");
    std::fs::write(&path, &bytes).unwrap();

    let info = magick_identify(&path);
    assert!(info.contains("BMP"), "not BMP: {info}");

    // (0,0) index 0 → red; (1,0) index 1 → blue. magick must resolve both
    // against the trimmed table.
    let px0 = magick_pixel_rgba(&path, 0, 0);
    assert_eq!(px0[0], 255, "px0 R (red): {px0:?}");
    assert_eq!(px0[2], 0, "px0 B: {px0:?}");
    let px1 = magick_pixel_rgba(&path, 1, 0);
    assert_eq!(px1[2], 255, "px1 B (blue): {px1:?}");
    assert_eq!(px1[0], 0, "px1 R: {px1:?}");
}

// ---------------------------------------------------------------------------
// Top-down DIB: magick must read the negative-biHeight file correctly.
// ---------------------------------------------------------------------------

#[test]
fn magick_top_down_rgba() {
    if !magick_available() {
        return;
    }

    let w = 8u32;
    let h = 6u32;
    // Top row red, bottom row blue — easy to tell flip mistakes apart.
    let mut data = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for _ in 0..w {
            if y < h / 2 {
                data.extend_from_slice(&[255u8, 0, 0, 255]);
            } else {
                data.extend_from_slice(&[0u8, 0, 255, 255]);
            }
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
    let (bytes, _) = encode_bmp_with_options(
        &src,
        BmpEncodeOptions {
            top_down: true,
            ..Default::default()
        },
    )
    .unwrap();
    let path = tmp_path("test_top_down.bmp");
    std::fs::write(&path, &bytes).unwrap();

    let info = magick_identify(&path);
    assert!(info.contains("BMP"), "not recognised as BMP: {info}");

    // magick must see the top row as red, bottom row as blue — proving
    // it honoured the negative biHeight.
    let top = magick_pixel_rgba(&path, 0, 0);
    let bottom = magick_pixel_rgba(&path, 0, h - 1);
    assert_eq!(top[0], 255, "top should be red: {top:?}");
    assert_eq!(top[2], 0, "top blue chan should be 0: {top:?}");
    assert_eq!(bottom[2], 255, "bottom should be blue: {bottom:?}");
    assert_eq!(bottom[0], 0, "bottom red chan should be 0: {bottom:?}");
}
