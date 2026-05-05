//! Pure-Rust BMP (Windows bitmap) codec + container.
//!
//! Handles 1/4/8/16/24/32-bit `BI_RGB` bitmaps plus 16/32-bit
//! `BI_BITFIELDS`, bottom-up and top-down row orders, and v3 / v4 / v5
//! `BITMAPINFOHEADER` variants. Always decodes to an `Rgba`
//! [`BmpImage`] so consumers don't have to care about palette lookup
//! or channel order.
//!
//! Also exposes a headerless "DIB" API ([`decode_dib`] / [`encode_dib`])
//! used by `oxideav-ico` for icon sub-images. The DIB path reads + writes
//! the same `BITMAPINFOHEADER` + pixel array but skips the 14-byte
//! `BITMAPFILEHEADER` and, on request, appends the 1-bpp AND mask that
//! `.ico` / `.cur` sub-images require.
//!
//! Decode supports `BI_RLE8` and `BI_RLE4` in addition to the uncompressed
//! formats. `BI_JPEG` and `BI_PNG` embedded payloads are rejected.
//!
//! Encode side supports 32-bit BGRA, 24-bit BGR, 16-bit RGB 5-6-5
//! (`BI_BITFIELDS`), 8-bit indexed (with optional `BI_RLE8`), and 4-bit
//! indexed (with optional `BI_RLE4`). RLE is chosen automatically when it
//! produces a smaller file than uncompressed indexed.
//!
//! ## Standalone vs registry-integrated
//!
//! The crate's default `registry` Cargo feature pulls in `oxideav-core`
//! and exposes the framework `Decoder` / `Encoder` trait surface plus
//! a [`registry::register`] entry point. Disable the feature
//! (`default-features = false`) for an `oxideav-core`-free build that
//! still exposes the standalone [`decode_bmp`] / [`encode_bmp`] /
//! [`decode_dib`] / [`encode_dib`] API plus crate-local [`BmpImage`] /
//! [`BmpPixelFormat`] / [`BmpError`] types.

#[cfg(feature = "registry")]
pub mod container;
pub mod decoder;
pub mod encoder;
pub mod error;
pub mod image;
#[cfg(feature = "registry")]
pub mod registry;
pub mod types;

/// Codec id for BMP image frames.
pub const CODEC_ID_STR: &str = "bmp";

pub use decoder::{decode_bmp, decode_dib};
#[cfg(feature = "registry")]
pub use decoder::{decode_bmp_videoframe, decode_dib_videoframe};
pub use encoder::{encode_bmp, encode_bmp_plane, encode_dib, encode_dib_plane, EncodedBmpFormat};
#[cfg(feature = "registry")]
pub use encoder::{encode_bmp_videoframe, encode_dib_videoframe};
pub use error::{BmpError, Result};
pub use image::{BmpImage, BmpPalette, BmpPixelFormat, BmpPlane};
pub use types::{
    row_stride, DibHeader, BITMAPFILEHEADER_SIZE, BITMAPINFOHEADER_SIZE, BITMAPV4HEADER_SIZE,
    BITMAPV5HEADER_SIZE, BI_BITFIELDS, BI_RGB, BMP_MAGIC,
};

#[cfg(feature = "registry")]
pub use registry::{register, register_codecs, register_containers};

#[cfg(test)]
mod tests {
    use super::*;

    fn rgba_checker(w: u32, h: u32) -> (BmpImage, u32, u32) {
        // 2×2 red/green/blue/white grid, tiled to w×h. Dumb but
        // easy to eyeball on roundtrip.
        let mut data = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                let q = ((x & 1) + 2 * (y & 1)) as usize;
                let rgba = [
                    [255, 0, 0, 255],
                    [0, 255, 0, 255],
                    [0, 0, 255, 200],
                    [255, 255, 255, 128],
                ][q];
                data.extend_from_slice(&rgba);
            }
        }
        let image = BmpImage {
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
        (image, w, h)
    }

    #[test]
    fn roundtrip_32bpp_rgba() {
        let (src, w, h) = rgba_checker(16, 12);
        let (bytes, fmt) = encode_bmp(&src).unwrap();
        assert_eq!(fmt, EncodedBmpFormat::Rgb32);
        assert_eq!(&bytes[..2], b"BM");
        let back = decode_bmp(&bytes).unwrap();
        // Width can be derived from the Rgba plane stride (4 bytes/pixel).
        assert_eq!(back.planes[0].stride / 4, w as usize);
        assert_eq!(
            back.planes[0].data.len() / back.planes[0].stride,
            h as usize
        );
        assert_eq!(back.planes[0].data, src.planes[0].data);
    }

    #[test]
    fn dib_ico_roundtrip_with_and_mask() {
        let (src, w, h) = rgba_checker(8, 8);
        let dib = encode_dib(&src, /* doubled */ true).unwrap();
        // First 4 bytes are the header size = 40.
        assert_eq!(dib[0], 40);
        // The stored height should be 16 (2×8).
        let stored_h = i32::from_le_bytes([dib[8], dib[9], dib[10], dib[11]]);
        assert_eq!(stored_h, 16);
        // Fully opaque quadrants have AND mask = 0; the q=3 (RGBA
        // 255,255,255,128) has alpha != 0 → mask bit = 0 too. With no
        // fully-transparent pixels the whole AND mask should be zero.
        let back = decode_dib(&dib, /* doubled */ true).unwrap();
        assert_eq!(back.planes[0].stride / 4, w as usize);
        assert_eq!(
            back.planes[0].data.len() / back.planes[0].stride,
            h as usize
        );
        assert_eq!(back.planes[0].data, src.planes[0].data);
    }

    #[test]
    fn decode_rejects_bad_magic() {
        let mut bytes = vec![0u8; 64];
        bytes[0] = b'X';
        assert!(decode_bmp(&bytes).is_err());
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    /// Build a simple 4×4 RGB24 image: top-left red, top-right green,
    /// bottom-left blue, bottom-right white (2×2 block each).
    fn rgb24_checker(w: u32, h: u32) -> BmpImage {
        let mut data = Vec::with_capacity((w * h * 3) as usize);
        for y in 0..h {
            for x in 0..w {
                let q = ((x & 1) + 2 * (y & 1)) as usize;
                let rgb = [[255u8, 0, 0], [0, 255, 0], [0, 0, 255], [200, 200, 200]][q];
                data.extend_from_slice(&rgb);
            }
        }
        BmpImage {
            width: w,
            height: h,
            pixel_format: BmpPixelFormat::Rgb24,
            planes: vec![BmpPlane {
                stride: w as usize * 3,
                data,
            }],
            palette: None,
            pts: None,
        }
    }

    /// Build a 4-entry palette: red, green, blue, white.
    fn four_color_palette() -> BmpPalette {
        BmpPalette {
            entries: vec![[255, 0, 0], [0, 255, 0], [0, 0, 255], [255, 255, 255]],
        }
    }

    /// Build an indexed image with entries 0-3 tiled.
    fn indexed_checker(w: u32, h: u32) -> (Vec<u8>, usize) {
        let stride = w as usize;
        let mut data = Vec::with_capacity(stride * h as usize);
        for y in 0..h {
            for x in 0..w {
                let idx = ((x & 1) + 2 * (y & 1)) as u8;
                data.push(idx);
            }
        }
        (data, stride)
    }

    // -----------------------------------------------------------------------
    // 24-bit BGR roundtrip
    // -----------------------------------------------------------------------

    #[test]
    fn roundtrip_24bpp_rgb24() {
        let src = rgb24_checker(8, 6);
        let (bytes, fmt) = encode_bmp(&src).unwrap();
        assert_eq!(fmt, EncodedBmpFormat::Rgb24);
        assert_eq!(&bytes[..2], b"BM");
        // V3 header: bpp at offset 28 (14 file header + 14 into DIB).
        let bpp = u16::from_le_bytes([bytes[28], bytes[29]]);
        assert_eq!(bpp, 24);
        let back = decode_bmp(&bytes).unwrap();
        assert_eq!(back.width, 8);
        assert_eq!(back.height, 6);
        // Verify spot pixel: (0,0) should be [255, 0, 0, 255] (red, opaque).
        assert_eq!(&back.planes[0].data[..4], &[255u8, 0, 0, 255]);
    }

    // -----------------------------------------------------------------------
    // 16-bit RGB565 roundtrip
    // -----------------------------------------------------------------------

    #[test]
    fn roundtrip_16bpp_rgb565() {
        let w = 8u32;
        let h = 6u32;
        // Build a simple RGB565 image: all pixels = 0xF800 (red in 5-6-5).
        let pixel = 0xF800u16.to_le_bytes();
        let stride = w as usize * 2;
        let mut data = Vec::with_capacity(stride * h as usize);
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
        let (bytes, fmt) = encode_bmp(&src).unwrap();
        assert_eq!(fmt, EncodedBmpFormat::Rgb16Bitfields);
        assert_eq!(&bytes[..2], b"BM");

        // V4 header: size field at offset 14 should be 108 (BITMAPV4HEADER).
        let hdr_size = u32::from_le_bytes([bytes[14], bytes[15], bytes[16], bytes[17]]);
        assert_eq!(hdr_size, 108);

        // Compression field at offset 30 should be BI_BITFIELDS (3).
        let compression = u32::from_le_bytes([bytes[30], bytes[31], bytes[32], bytes[33]]);
        assert_eq!(compression, BI_BITFIELDS);

        // R mask at offset 54 = 0xF800.
        let r_mask = u32::from_le_bytes([bytes[54], bytes[55], bytes[56], bytes[57]]);
        assert_eq!(r_mask, 0xF800);

        // Decode it back — should be (255, 0, 0, 255) for each pixel.
        let back = decode_bmp(&bytes).unwrap();
        assert_eq!(back.width, w);
        assert_eq!(back.height, h);
        // Red pixel in RGB565 = 0xF800 → R=0b11111→255, G=0, B=0.
        assert_eq!(back.planes[0].data[0], 255); // R
        assert_eq!(back.planes[0].data[1], 0); // G
        assert_eq!(back.planes[0].data[2], 0); // B
        assert_eq!(back.planes[0].data[3], 255); // A (opaque, no alpha in 5-6-5)
    }

    // -----------------------------------------------------------------------
    // 8-bit indexed uncompressed roundtrip
    // -----------------------------------------------------------------------

    #[test]
    fn roundtrip_8bit_indexed() {
        let w = 8u32;
        let h = 8u32;
        let palette = four_color_palette();
        let (data, stride) = indexed_checker(w, h);
        let src = BmpImage {
            width: w,
            height: h,
            pixel_format: BmpPixelFormat::Indexed8,
            planes: vec![BmpPlane { stride, data }],
            palette: Some(palette.clone()),
            pts: None,
        };
        let (bytes, fmt) = encode_bmp(&src).unwrap();
        // Small solid-colour checker: RLE may or may not win; test both paths.
        assert!(
            fmt == EncodedBmpFormat::Indexed8 || fmt == EncodedBmpFormat::Rle8,
            "unexpected format {fmt:?}"
        );
        assert_eq!(&bytes[..2], b"BM");

        let back = decode_bmp(&bytes).unwrap();
        assert_eq!(back.width, w);
        assert_eq!(back.height, h);

        // Verify pixel (0, 0): index 0 → red.
        assert_eq!(&back.planes[0].data[..4], &[255u8, 0, 0, 255]);
    }

    // -----------------------------------------------------------------------
    // 4-bit indexed uncompressed roundtrip
    // -----------------------------------------------------------------------

    #[test]
    fn roundtrip_4bit_indexed() {
        let w = 8u32;
        let h = 8u32;
        let palette = four_color_palette();
        let (data, stride) = indexed_checker(w, h);
        let src = BmpImage {
            width: w,
            height: h,
            pixel_format: BmpPixelFormat::Indexed4,
            planes: vec![BmpPlane { stride, data }],
            palette: Some(palette.clone()),
            pts: None,
        };
        let (bytes, fmt) = encode_bmp(&src).unwrap();
        assert!(
            fmt == EncodedBmpFormat::Indexed4 || fmt == EncodedBmpFormat::Rle4,
            "unexpected format {fmt:?}"
        );
        assert_eq!(&bytes[..2], b"BM");

        let back = decode_bmp(&bytes).unwrap();
        assert_eq!(back.width, w);
        assert_eq!(back.height, h);

        // Pixel (0, 0): index 0 → red.
        assert_eq!(&back.planes[0].data[..4], &[255u8, 0, 0, 255]);
    }

    // -----------------------------------------------------------------------
    // RLE8: force a run-heavy image and verify RLE is chosen + decodes.
    // -----------------------------------------------------------------------

    #[test]
    fn roundtrip_rle8_run_heavy() {
        let w = 32u32;
        let h = 16u32;
        // Solid-colour rows → very RLE-friendly.
        let mut data = Vec::with_capacity(w as usize * h as usize);
        for y in 0..h {
            for _ in 0..w {
                data.push((y % 4) as u8);
            }
        }
        let palette = BmpPalette {
            entries: vec![[255, 0, 0], [0, 255, 0], [0, 0, 255], [128, 128, 128]],
        };
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
        // Solid rows should compress well; expect RLE8.
        assert_eq!(
            fmt,
            EncodedBmpFormat::Rle8,
            "expected RLE8 for run-heavy image"
        );

        // Verify compressed output is smaller than raw.
        let raw_pixel_bytes = row_stride(w as usize, 8) * h as usize;
        let encoded_size =
            bytes.len() - (BITMAPFILEHEADER_SIZE + BITMAPINFOHEADER_SIZE + 256 * 4) as usize;
        assert!(
            encoded_size < raw_pixel_bytes,
            "RLE8 payload {encoded_size} should be smaller than raw {raw_pixel_bytes}"
        );

        let back = decode_bmp(&bytes).unwrap();
        assert_eq!(back.width, w);
        assert_eq!(back.height, h);
        // Row 0 (bottom of image, y=h-1) → index (h-1) % 4 = 3 → [128,128,128,255]
        // Row h-1 (top of image, y=0) → index 0 → [255,0,0,255]
        assert_eq!(&back.planes[0].data[..4], &[255u8, 0, 0, 255]);
    }

    // -----------------------------------------------------------------------
    // RLE4: force a run-heavy image and verify RLE4 is chosen + decodes.
    // -----------------------------------------------------------------------

    #[test]
    fn roundtrip_rle4_run_heavy() {
        let w = 32u32;
        let h = 16u32;
        // Solid rows of index 0 and 1 alternating.
        let mut data = Vec::with_capacity(w as usize * h as usize);
        for y in 0..h {
            for _ in 0..w {
                data.push((y % 2) as u8);
            }
        }
        let palette = BmpPalette {
            entries: vec![[255, 0, 0], [0, 0, 255]],
        };
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
        assert_eq!(
            fmt,
            EncodedBmpFormat::Rle4,
            "expected RLE4 for run-heavy image"
        );

        let back = decode_bmp(&bytes).unwrap();
        assert_eq!(back.width, w);
        assert_eq!(back.height, h);
        // Top row (y=0 in output) → index 0 → red.
        assert_eq!(&back.planes[0].data[..4], &[255u8, 0, 0, 255]);
    }

    // -----------------------------------------------------------------------
    // Indexed without palette → error
    // -----------------------------------------------------------------------

    #[test]
    fn indexed8_without_palette_errors() {
        let src = BmpImage {
            width: 4,
            height: 4,
            pixel_format: BmpPixelFormat::Indexed8,
            planes: vec![BmpPlane {
                stride: 4,
                data: vec![0u8; 16],
            }],
            palette: None,
            pts: None,
        };
        assert!(encode_bmp(&src).is_err());
    }

    // -----------------------------------------------------------------------
    // File-size sanity: 24-bit should be smaller than 32-bit for same image.
    // -----------------------------------------------------------------------

    #[test]
    fn rgb24_smaller_than_rgba() {
        let (src_rgba, _, _) = rgba_checker(16, 16);
        let src_rgb24 = rgb24_checker(16, 16);
        let (bytes_rgba, _) = encode_bmp(&src_rgba).unwrap();
        let (bytes_rgb24, _) = encode_bmp(&src_rgb24).unwrap();
        assert!(
            bytes_rgb24.len() < bytes_rgba.len(),
            "24-bit ({}) should be smaller than 32-bit ({})",
            bytes_rgb24.len(),
            bytes_rgba.len()
        );
    }

    // -----------------------------------------------------------------------
    // V4 header dimensions in encoded 16-bit file.
    // -----------------------------------------------------------------------

    #[test]
    fn rgb565_header_dimensions() {
        let w = 10u32;
        let h = 7u32;
        let stride = w as usize * 2;
        let data = vec![0u8; stride * h as usize];
        let src = BmpImage {
            width: w,
            height: h,
            pixel_format: BmpPixelFormat::Rgb565,
            planes: vec![BmpPlane { stride, data }],
            palette: None,
            pts: None,
        };
        let (bytes, _) = encode_bmp(&src).unwrap();
        // Width at offset 18 = w.
        let enc_w = i32::from_le_bytes([bytes[18], bytes[19], bytes[20], bytes[21]]);
        let enc_h = i32::from_le_bytes([bytes[22], bytes[23], bytes[24], bytes[25]]);
        assert_eq!(enc_w, w as i32);
        assert_eq!(enc_h, h as i32);
    }
}
