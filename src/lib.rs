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
//! (`BI_BITFIELDS`), 8-bit indexed (with optional `BI_RLE8`), 4-bit
//! indexed (with optional `BI_RLE4`), 2-bit indexed (Windows CE,
//! 4-entry palette, always uncompressed), and 1-bit indexed
//! (monochrome, always uncompressed). RLE is chosen automatically when
//! it produces a smaller file than uncompressed indexed.
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
pub mod metadata;
#[cfg(feature = "registry")]
pub mod registry;
pub mod types;

/// Codec id for BMP image frames.
pub const CODEC_ID_STR: &str = "bmp";

pub use decoder::{decode_bmp, decode_bmp_with_metadata, decode_dib, decode_dib_with_metadata};
#[cfg(feature = "registry")]
pub use decoder::{decode_bmp_videoframe, decode_dib_videoframe};
pub use encoder::{
    encode_bmp, encode_bmp_plane, encode_bmp_plane_with_options, encode_bmp_with_calibrated_rgb,
    encode_bmp_with_icc_profile, encode_bmp_with_linked_icc_profile, encode_bmp_with_options,
    BmpEncodeOptions, EncodedBmpFormat,
};
#[cfg(feature = "registry")]
pub use encoder::{encode_bmp_videoframe, encode_dib_videoframe};
pub use encoder::{encode_dib, encode_dib_plane};
pub use error::{BmpError, Result};
pub use image::{BmpImage, BmpPalette, BmpPixelFormat, BmpPlane};
pub use metadata::{
    BmpColorSpace, BmpIccProfileRef, BmpMetadata, BmpOs2Halftone, BmpOs2Header2, BmpRenderingIntent,
};
pub use types::{
    row_stride, BitmapFileHeader, BitmapInfoHeader, DibHeader, DibHeaderKind,
    BITMAPCOREHEADER_SIZE, BITMAPFILEHEADER_SIZE, BITMAPINFOHEADER_SIZE, BITMAPV2INFOHEADER_SIZE,
    BITMAPV3INFOHEADER_SIZE, BITMAPV4HEADER_SIZE, BITMAPV5HEADER_SIZE, BI_ALPHABITFIELDS,
    BI_BITFIELDS, BI_CMYK, BI_CMYKRLE4, BI_CMYKRLE8, BI_RGB, BMP_MAGIC, LCS_CALIBRATED_RGB,
    LCS_GM_ABS_COLORIMETRIC, LCS_GM_BUSINESS, LCS_GM_GRAPHICS, LCS_GM_IMAGES, LCS_S_RGB,
    LCS_WINDOWS_COLOR_SPACE, OS22XBITMAPHEADER_SIZE, OS2_COLOR_ENCODING_RGB,
    OS2_HALFTONE_ERROR_DIFFUSION, OS2_HALFTONE_NONE, OS2_HALFTONE_PANDA, OS2_HALFTONE_SUPER_CIRCLE,
    OS2_RECORDING_BOTTOM_UP, OS2_UNITS_PELS_PER_METER, PROFILE_EMBEDDED, PROFILE_LINKED,
};

#[cfg(feature = "registry")]
pub use registry::{register, register_codecs, register_containers};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{BI_RLE4, BI_RLE8};

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

    #[test]
    fn roundtrip_16bpp_rgb555() {
        let w = 8u32;
        let h = 6u32;
        // 5-5-5: high bit reserved, R bits 14..10, G bits 9..5, B bits 4..0.
        // 0x7C00 = pure red (R=0b11111, G=0, B=0).
        let pixel = 0x7C00u16.to_le_bytes();
        let stride = w as usize * 2;
        let mut data = Vec::with_capacity(stride * h as usize);
        for _ in 0..(h as usize * w as usize) {
            data.extend_from_slice(&pixel);
        }
        let src = BmpImage {
            width: w,
            height: h,
            pixel_format: BmpPixelFormat::Rgb555,
            planes: vec![BmpPlane { stride, data }],
            palette: None,
            pts: None,
        };
        let (bytes, fmt) = encode_bmp(&src).unwrap();
        assert_eq!(fmt, EncodedBmpFormat::Rgb16Rgb);
        assert_eq!(&bytes[..2], b"BM");

        // A 16-bpp BI_RGB bitmap needs only the plain 40-byte
        // BITMAPINFOHEADER (no BI_BITFIELDS mask block).
        let hdr_size = u32::from_le_bytes([bytes[14], bytes[15], bytes[16], bytes[17]]);
        assert_eq!(hdr_size, 40);

        // bit-count at offset 28 = 16.
        let bit_count = u16::from_le_bytes([bytes[28], bytes[29]]);
        assert_eq!(bit_count, 16);

        // compression at offset 30 = BI_RGB (0).
        let compression = u32::from_le_bytes([bytes[30], bytes[31], bytes[32], bytes[33]]);
        assert_eq!(compression, BI_RGB);

        // bfOffBits at offset 10 = 14 + 40 (no colour table, no masks).
        let off_bits = u32::from_le_bytes([bytes[10], bytes[11], bytes[12], bytes[13]]);
        assert_eq!(off_bits, 14 + 40);

        // Decode it back — each pixel should be opaque red.
        let back = decode_bmp(&bytes).unwrap();
        assert_eq!(back.width, w);
        assert_eq!(back.height, h);
        assert_eq!(back.planes[0].data[0], 255); // R
        assert_eq!(back.planes[0].data[1], 0); // G
        assert_eq!(back.planes[0].data[2], 0); // B
        assert_eq!(back.planes[0].data[3], 255); // A (opaque, no alpha in 5-5-5)
    }

    #[test]
    fn rgb555_top_down_roundtrips() {
        // A vertical gradient so a row-order bug would surface: row y has
        // green level y in the 5-5-5 green field (bits 9..5).
        let w = 4u32;
        let h = 5u32;
        let stride = w as usize * 2;
        let mut data = Vec::with_capacity(stride * h as usize);
        for y in 0..h as u16 {
            let word: u16 = (y & 0x1F) << 5; // green only
            for _ in 0..w {
                data.extend_from_slice(&word.to_le_bytes());
            }
        }
        let src = BmpImage {
            width: w,
            height: h,
            pixel_format: BmpPixelFormat::Rgb555,
            planes: vec![BmpPlane {
                stride,
                data: data.clone(),
            }],
            palette: None,
            pts: None,
        };
        let (bytes, fmt) = encode_bmp_with_options(
            &src,
            BmpEncodeOptions {
                top_down: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(fmt, EncodedBmpFormat::Rgb16Rgb);

        // biHeight at offset 22 must be negative (top-down).
        let stored_h = i32::from_le_bytes([bytes[22], bytes[23], bytes[24], bytes[25]]);
        assert_eq!(stored_h, -(h as i32));

        let back = decode_bmp(&bytes).unwrap();
        assert_eq!((back.width, back.height), (w, h));
        // Row 0 (top) carries green level 0; the green channel rises with y.
        // expand(0,5)=0, expand(1,5) = 0b00001 → repeated to 0x08, etc.
        // Just assert monotonic non-decreasing green down the rows and a
        // zero top row, which a flipped order would violate.
        let g_at = |y: usize| -> u8 {
            let row = &back.planes[0].data[y * w as usize * 4..];
            row[1]
        };
        assert_eq!(g_at(0), 0);
        for y in 1..h as usize {
            assert!(g_at(y) >= g_at(y - 1), "green must rise down the rows");
        }
        assert!(g_at(h as usize - 1) > 0);
    }

    #[test]
    fn rgb555_truncated_plane_errors() {
        // Plane shorter than width*height*2 must be rejected, not panic.
        let w = 8u32;
        let h = 8u32;
        let stride = w as usize * 2;
        let src = BmpImage {
            width: w,
            height: h,
            pixel_format: BmpPixelFormat::Rgb555,
            planes: vec![BmpPlane {
                stride,
                data: vec![0u8; stride * (h as usize - 2)], // too short
            }],
            palette: None,
            pts: None,
        };
        assert!(encode_bmp(&src).is_err());
    }

    #[test]
    fn rgb555_dib_roundtrips() {
        // Headerless DIB path (the .ico consumer surface) accepts Rgb555.
        let w = 4u32;
        let h = 4u32;
        let stride = w as usize * 2;
        let word = 0x001Fu16.to_le_bytes(); // pure blue in 5-5-5
        let mut data = Vec::with_capacity(stride * h as usize);
        for _ in 0..(h as usize * w as usize) {
            data.extend_from_slice(&word);
        }
        let plane = BmpPlane { stride, data };
        let dib = encode_dib_plane(&plane, BmpPixelFormat::Rgb555, None, w, h, false).unwrap();
        // Headerless: first DWORD is the 40-byte header size.
        let hdr_size = u32::from_le_bytes([dib[0], dib[1], dib[2], dib[3]]);
        assert_eq!(hdr_size, 40);
        let back = decode_dib(&dib, false).unwrap();
        assert_eq!((back.width, back.height), (w, h));
        assert_eq!(back.planes[0].data[0], 0); // R
        assert_eq!(back.planes[0].data[1], 0); // G
        assert_eq!(back.planes[0].data[2], 255); // B
        assert_eq!(back.planes[0].data[3], 255); // A
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

    // -----------------------------------------------------------------------
    // Top-down DIB encoder
    // -----------------------------------------------------------------------

    #[test]
    fn encode_top_down_rgba_negative_height_and_roundtrip() {
        let (src, w, h) = rgba_checker(8, 6);
        let (bytes, fmt) = encode_bmp_with_options(
            &src,
            BmpEncodeOptions {
                top_down: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(fmt, EncodedBmpFormat::Rgb32);
        assert_eq!(&bytes[..2], b"BM");
        // V3 header at file offset 14; biHeight is i32 at offset 22.
        let stored_h = i32::from_le_bytes([bytes[22], bytes[23], bytes[24], bytes[25]]);
        assert_eq!(stored_h, -(h as i32), "top-down DIB must encode -biHeight");

        // Decode should produce the same top-down Rgba pixels we put in.
        let back = decode_bmp(&bytes).unwrap();
        assert_eq!(back.width, w);
        assert_eq!(back.height, h);
        assert_eq!(back.planes[0].data, src.planes[0].data);
    }

    #[test]
    fn encode_top_down_rgb24_roundtrip() {
        let src = rgb24_checker(8, 6);
        let (bytes, fmt) = encode_bmp_with_options(
            &src,
            BmpEncodeOptions {
                top_down: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(fmt, EncodedBmpFormat::Rgb24);
        let stored_h = i32::from_le_bytes([bytes[22], bytes[23], bytes[24], bytes[25]]);
        assert_eq!(stored_h, -6);
        let back = decode_bmp(&bytes).unwrap();
        // (0,0) should be red.
        assert_eq!(&back.planes[0].data[..4], &[255u8, 0, 0, 255]);
    }

    #[test]
    fn encode_top_down_indexed8_skips_rle() {
        // Run-heavy image — bottom-up would emit RLE8, top-down must
        // fall back to uncompressed since RLE + negative height is
        // disallowed by the BMP spec.
        let w = 32u32;
        let h = 16u32;
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
        let (bytes, fmt) = encode_bmp_with_options(
            &src,
            BmpEncodeOptions {
                top_down: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            fmt,
            EncodedBmpFormat::Indexed8,
            "top-down must force uncompressed indexed"
        );
        let stored_h = i32::from_le_bytes([bytes[22], bytes[23], bytes[24], bytes[25]]);
        assert_eq!(stored_h, -(h as i32));
        // Roundtrip: (0,0) in top-down output → first row of source →
        // y=0 → index 0 → red.
        let back = decode_bmp(&bytes).unwrap();
        assert_eq!(&back.planes[0].data[..4], &[255u8, 0, 0, 255]);
    }

    // -----------------------------------------------------------------------
    // OS/2 BITMAPCOREHEADER (12-byte) decode
    // -----------------------------------------------------------------------

    /// Hand-assemble a 4×2 RGB 24-bit BMP that uses the OS/2 1.x
    /// `BITMAPCOREHEADER` instead of the V3 header. Rows are bottom-up
    /// per the OS/2 spec (no negative-height support in this header).
    fn build_os2_24bpp_bmp(w: u32, h: u32, pixels_top_down_rgb: &[(u8, u8, u8)]) -> Vec<u8> {
        let row_stride = (w as usize * 3).div_ceil(4) * 4;
        // Pixel array, bottom-up BGR.
        let mut pixel_array = vec![0u8; row_stride * h as usize];
        for y in 0..h as usize {
            let src_y = h as usize - 1 - y;
            let row = &mut pixel_array[y * row_stride..y * row_stride + row_stride];
            for x in 0..w as usize {
                let (r, g, b) = pixels_top_down_rgb[src_y * w as usize + x];
                row[x * 3] = b;
                row[x * 3 + 1] = g;
                row[x * 3 + 2] = r;
            }
        }
        // BITMAPFILEHEADER (14 B) + BITMAPCOREHEADER (12 B) + pixels.
        let pixel_offset = 14u32 + BITMAPCOREHEADER_SIZE;
        let file_size = pixel_offset + pixel_array.len() as u32;
        let mut out = Vec::with_capacity(file_size as usize);
        // 'BM'
        out.extend_from_slice(b"BM");
        out.extend_from_slice(&file_size.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&pixel_offset.to_le_bytes());
        // BITMAPCOREHEADER.
        out.extend_from_slice(&BITMAPCOREHEADER_SIZE.to_le_bytes());
        out.extend_from_slice(&(w as u16).to_le_bytes());
        out.extend_from_slice(&(h as u16).to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes()); // planes
        out.extend_from_slice(&24u16.to_le_bytes()); // bcBitCount
        out.extend_from_slice(&pixel_array);
        out
    }

    /// Hand-assemble a 4×4 OS/2 1.x 4-bit indexed BMP. Palette
    /// entries are 3-byte RGBTRIPLE (not RGBQUAD).
    fn build_os2_4bpp_bmp(w: u32, h: u32, indices_top_down: &[u8], palette: &[[u8; 3]]) -> Vec<u8> {
        assert!(palette.len() <= 16);
        let row_stride = (w as usize * 4).div_ceil(32) * 4;
        let mut pixel_array = vec![0u8; row_stride * h as usize];
        for y in 0..h as usize {
            let src_y = h as usize - 1 - y;
            let row = &mut pixel_array[y * row_stride..];
            for x in 0..w as usize {
                let idx = indices_top_down[src_y * w as usize + x] & 0x0F;
                if x & 1 == 0 {
                    row[x / 2] = idx << 4;
                } else {
                    row[x / 2] |= idx;
                }
            }
        }
        let palette_bytes = 16 * 3; // OS/2 always emits 2^bpp entries × RGBTRIPLE.
        let pixel_offset = 14u32 + BITMAPCOREHEADER_SIZE + palette_bytes;
        let file_size = pixel_offset + pixel_array.len() as u32;
        let mut out = Vec::with_capacity(file_size as usize);
        out.extend_from_slice(b"BM");
        out.extend_from_slice(&file_size.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&pixel_offset.to_le_bytes());
        // BITMAPCOREHEADER.
        out.extend_from_slice(&BITMAPCOREHEADER_SIZE.to_le_bytes());
        out.extend_from_slice(&(w as u16).to_le_bytes());
        out.extend_from_slice(&(h as u16).to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&4u16.to_le_bytes());
        // Palette: 3 bytes/entry, on-disk order B, G, R, padded to 16
        // entries even if the caller passes fewer.
        for i in 0..16 {
            if let Some(rgb) = palette.get(i) {
                out.push(rgb[2]); // B
                out.push(rgb[1]); // G
                out.push(rgb[0]); // R
            } else {
                out.extend_from_slice(&[0, 0, 0]);
            }
        }
        out.extend_from_slice(&pixel_array);
        out
    }

    #[test]
    fn decode_os2_bitmapcoreheader_24bpp() {
        // 4×2 image: top row = red, bottom row = blue.
        let w = 4u32;
        let h = 2u32;
        let mut pixels = Vec::with_capacity((w * h) as usize);
        pixels.resize(w as usize, (255u8, 0, 0));
        pixels.resize(2 * w as usize, (0u8, 0, 255));
        let bytes = build_os2_24bpp_bmp(w, h, &pixels);
        let img = decode_bmp(&bytes).unwrap();
        assert_eq!(img.width, w);
        assert_eq!(img.height, h);
        // Output is top-down Rgba: row 0 should be red.
        assert_eq!(&img.planes[0].data[..4], &[255, 0, 0, 255]);
        // Row 1, x=0: blue.
        let stride = img.planes[0].stride;
        assert_eq!(&img.planes[0].data[stride..stride + 4], &[0, 0, 255, 255]);
    }

    #[test]
    fn decode_os2_bitmapcoreheader_4bpp_indexed() {
        // 4×2 image: top row index 0 (red), bottom row index 1 (green).
        let w = 4u32;
        let h = 2u32;
        let mut indices = Vec::with_capacity((w * h) as usize);
        indices.resize(w as usize, 0u8);
        indices.resize(2 * w as usize, 1u8);
        let palette: &[[u8; 3]] = &[[255, 0, 0], [0, 255, 0]];
        let bytes = build_os2_4bpp_bmp(w, h, &indices, palette);
        let img = decode_bmp(&bytes).unwrap();
        assert_eq!(img.width, w);
        assert_eq!(img.height, h);
        // Row 0 → red, row 1 → green (output is top-down).
        assert_eq!(&img.planes[0].data[..4], &[255, 0, 0, 255]);
        let stride = img.planes[0].stride;
        assert_eq!(&img.planes[0].data[stride..stride + 4], &[0, 255, 0, 255]);
    }

    // -----------------------------------------------------------------------
    // Windows CE 2-bit/pixel indexed (V3 BITMAPINFOHEADER)
    // -----------------------------------------------------------------------

    /// Hand-assemble a `w×h` 2-bit indexed BMP with a 40-byte
    /// `BITMAPINFOHEADER` and a 4-entry RGBQUAD colour table. Four pixels
    /// pack per byte, the left-most pixel in the two most-significant
    /// bits, each a 2-bit index. Rows are bottom-up.
    fn build_v3_2bpp_bmp(w: u32, h: u32, indices_top_down: &[u8], palette: &[[u8; 3]]) -> Vec<u8> {
        assert!(palette.len() <= 4);
        let row_stride = (w as usize * 2).div_ceil(32) * 4;
        let mut pixel_array = vec![0u8; row_stride * h as usize];
        for y in 0..h as usize {
            let src_y = h as usize - 1 - y;
            let row = &mut pixel_array[y * row_stride..];
            for x in 0..w as usize {
                let idx = indices_top_down[src_y * w as usize + x] & 0x03;
                let shift = 6 - 2 * (x % 4);
                row[x / 4] |= idx << shift;
            }
        }
        // biClrUsed = 0 → 2^2 = 4 RGBQUAD entries.
        let palette_entries = 4usize;
        let palette_bytes = (palette_entries * 4) as u32;
        let header_size = 40u32;
        let pixel_offset = 14 + header_size + palette_bytes;
        let file_size = pixel_offset + pixel_array.len() as u32;
        let mut out = Vec::with_capacity(file_size as usize);
        out.extend_from_slice(b"BM");
        out.extend_from_slice(&file_size.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&pixel_offset.to_le_bytes());
        out.extend_from_slice(&header_size.to_le_bytes()); // biSize @0
        out.extend_from_slice(&(w as i32).to_le_bytes()); // biWidth @4
        out.extend_from_slice(&(h as i32).to_le_bytes()); // biHeight @8
        out.extend_from_slice(&1u16.to_le_bytes()); // biPlanes @12
        out.extend_from_slice(&2u16.to_le_bytes()); // biBitCount @14
        out.extend_from_slice(&0u32.to_le_bytes()); // biCompression @16 (BI_RGB)
        out.extend_from_slice(&0u32.to_le_bytes()); // biSizeImage @20
        out.extend_from_slice(&0i32.to_le_bytes()); // biXPelsPerM @24
        out.extend_from_slice(&0i32.to_le_bytes()); // biYPelsPerM @28
        out.extend_from_slice(&0u32.to_le_bytes()); // biClrUsed @32
        out.extend_from_slice(&0u32.to_le_bytes()); // biClrImportant @36
        for i in 0..palette_entries {
            if let Some(rgb) = palette.get(i) {
                out.push(rgb[2]); // B
                out.push(rgb[1]); // G
                out.push(rgb[0]); // R
                out.push(0);
            } else {
                out.extend_from_slice(&[0, 0, 0, 0]);
            }
        }
        out.extend_from_slice(&pixel_array);
        out
    }

    #[test]
    fn decode_v3_2bpp_windows_ce() {
        // 4×2: top row indices 0,1,2,3 ; bottom row 3,2,1,0.
        let w = 4u32;
        let h = 2u32;
        let indices = [0u8, 1, 2, 3, 3, 2, 1, 0];
        let palette: &[[u8; 3]] = &[[10, 20, 30], [40, 50, 60], [70, 80, 90], [255, 128, 0]];
        let bytes = build_v3_2bpp_bmp(w, h, &indices, palette);
        let img = decode_bmp(&bytes).unwrap();
        assert_eq!(img.width, w);
        assert_eq!(img.height, h);
        let s = img.planes[0].stride;
        // Top-down row 0: idx 0,1,2,3.
        assert_eq!(&img.planes[0].data[0..4], &[10, 20, 30, 255]);
        assert_eq!(&img.planes[0].data[4..8], &[40, 50, 60, 255]);
        assert_eq!(&img.planes[0].data[8..12], &[70, 80, 90, 255]);
        assert_eq!(&img.planes[0].data[12..16], &[255, 128, 0, 255]);
        // Row 1: idx 3,2,1,0.
        assert_eq!(&img.planes[0].data[s..s + 4], &[255, 128, 0, 255]);
        assert_eq!(&img.planes[0].data[s + 12..s + 16], &[10, 20, 30, 255]);
    }

    #[test]
    fn decode_v3_2bpp_top_down_negative_height() {
        // Negative biHeight → top-down rows; reverse the source rows so
        // the decoded top-down output matches the bottom-up case.
        let w = 4u32;
        let mut bytes = build_v3_2bpp_bmp(
            w,
            2,
            &[3, 2, 1, 0, 0, 1, 2, 3],
            &[[10, 20, 30], [40, 50, 60], [70, 80, 90], [255, 128, 0]],
        );
        // Patch biHeight (@14 + 8 = offset 22) to -2.
        bytes[22..26].copy_from_slice(&(-2i32).to_le_bytes());
        // Swap the two pixel rows. pixel offset = 14 + 40 + 4*4 = 70,
        // row stride = ((4*2 + 31)/32)*4 = 4.
        let po = 14 + 40 + 4 * 4;
        let stride = 4;
        let (r0, r1) = bytes[po..po + 2 * stride].split_at(stride);
        let swapped: Vec<u8> = r1.iter().chain(r0.iter()).copied().collect();
        bytes[po..po + 2 * stride].copy_from_slice(&swapped);
        let img = decode_bmp(&bytes).unwrap();
        assert_eq!(img.width, w);
        assert_eq!(img.height, 2);
        // Top-down row 0 now: idx 3,2,1,0.
        assert_eq!(&img.planes[0].data[0..4], &[255, 128, 0, 255]);
        assert_eq!(&img.planes[0].data[12..16], &[10, 20, 30, 255]);
    }

    // -----------------------------------------------------------------------
    // Truncated OS/2 2.x OS22XBITMAPHEADER (biSize 16..40)
    // -----------------------------------------------------------------------

    /// Hand-assemble a `w×h` 8-bit indexed BMP whose DIB header is a
    /// *truncated* OS/2 2.x `OS22XBITMAPHEADER` of `header_size` bytes
    /// (16..40). Fields past the truncation point are simply not written
    /// — the decoder reads them as zero. Unlike the 12-byte OS/2 1.x
    /// `BITMAPCOREHEADER`, this header uses 4-byte signed width/height
    /// and 4-byte `RGBQUAD` palette entries. Rows are bottom-up.
    fn build_truncated_os22x_8bpp(
        header_size: u32,
        w: u32,
        h: u32,
        indices_top_down: &[u8],
        palette: &[[u8; 3]],
    ) -> Vec<u8> {
        assert!((16..40).contains(&header_size));
        assert!(palette.len() <= 256);
        let row_stride = (w as usize).div_ceil(4) * 4;
        let mut pixel_array = vec![0u8; row_stride * h as usize];
        for y in 0..h as usize {
            let src_y = h as usize - 1 - y;
            let row = &mut pixel_array[y * row_stride..];
            for x in 0..w as usize {
                row[x] = indices_top_down[src_y * w as usize + x];
            }
        }
        // Truncated headers carry biClrUsed = 0 → 2^bpp = 256 RGBQUADs.
        let palette_entries = 256usize;
        let palette_bytes = (palette_entries * 4) as u32;
        let pixel_offset = 14 + header_size + palette_bytes;
        let file_size = pixel_offset + pixel_array.len() as u32;
        let mut out = Vec::with_capacity(file_size as usize);
        out.extend_from_slice(b"BM");
        out.extend_from_slice(&file_size.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&pixel_offset.to_le_bytes());
        // Truncated OS22XBITMAPHEADER — write only the first
        // `header_size` bytes of the 40-byte INFO layout.
        let mut hdr = Vec::with_capacity(40);
        hdr.extend_from_slice(&header_size.to_le_bytes()); // biSize @0
        hdr.extend_from_slice(&(w as i32).to_le_bytes()); // biWidth @4
        hdr.extend_from_slice(&(h as i32).to_le_bytes()); // biHeight @8
        hdr.extend_from_slice(&1u16.to_le_bytes()); // biPlanes @12
        hdr.extend_from_slice(&8u16.to_le_bytes()); // biBitCount @14
        hdr.extend_from_slice(&0u32.to_le_bytes()); // biCompression @16
        hdr.extend_from_slice(&0u32.to_le_bytes()); // biSizeImage @20
        hdr.extend_from_slice(&0i32.to_le_bytes()); // biXPelsPerM @24
        hdr.extend_from_slice(&0i32.to_le_bytes()); // biYPelsPerM @28
        hdr.extend_from_slice(&0u32.to_le_bytes()); // biClrUsed @32
        hdr.extend_from_slice(&0u32.to_le_bytes()); // biClrImportant @36
        out.extend_from_slice(&hdr[..header_size as usize]);
        // Colour table: 256 RGBQUAD (B, G, R, 0) entries.
        for i in 0..palette_entries {
            if let Some(rgb) = palette.get(i) {
                out.push(rgb[2]); // B
                out.push(rgb[1]); // G
                out.push(rgb[0]); // R
                out.push(0);
            } else {
                out.extend_from_slice(&[0, 0, 0, 0]);
            }
        }
        out.extend_from_slice(&pixel_array);
        out
    }

    #[test]
    fn decode_truncated_os22x_16byte_header() {
        // 4×2 image: top row index 0 (red), bottom row index 1 (green).
        // The canonical `pal8os2v2-16.bmp` shape: a 16-byte header with
        // every field past biBitCount assumed zero.
        let w = 4u32;
        let h = 2u32;
        let mut indices = Vec::with_capacity((w * h) as usize);
        indices.resize(w as usize, 0u8);
        indices.resize(2 * w as usize, 1u8);
        let palette: &[[u8; 3]] = &[[255, 0, 0], [0, 255, 0]];
        let bytes = build_truncated_os22x_8bpp(16, w, h, &indices, palette);
        let img = decode_bmp(&bytes).unwrap();
        assert_eq!(img.width, w);
        assert_eq!(img.height, h);
        // Output is top-down Rgba: row 0 red, row 1 green.
        assert_eq!(&img.planes[0].data[..4], &[255, 0, 0, 255]);
        let stride = img.planes[0].stride;
        assert_eq!(&img.planes[0].data[stride..stride + 4], &[0, 255, 0, 255]);
    }

    #[test]
    fn decode_truncated_os22x_intermediate_sizes() {
        // Every truncation point in 16..40 must decode the same pixels —
        // the partially-present trailing fields are all zero here anyway.
        let w = 2u32;
        let h = 2u32;
        let indices = [0u8, 1, 1, 0];
        let palette: &[[u8; 3]] = &[[10, 20, 30], [200, 100, 50]];
        for hs in [16u32, 20, 24, 28, 32, 36] {
            let bytes = build_truncated_os22x_8bpp(hs, w, h, &indices, palette);
            let img =
                decode_bmp(&bytes).unwrap_or_else(|e| panic!("header_size={hs} must decode: {e}"));
            assert_eq!(img.width, w, "header_size={hs}");
            assert_eq!(img.height, h, "header_size={hs}");
            // Top-down row 0: index 0 (10,20,30), index 1 (200,100,50).
            assert_eq!(&img.planes[0].data[..4], &[10, 20, 30, 255], "hs={hs}");
            assert_eq!(&img.planes[0].data[4..8], &[200, 100, 50, 255], "hs={hs}");
        }
    }

    #[test]
    fn decode_truncated_os22x_top_down_negative_height() {
        // The OS/2 2.x header uses 4-byte signed height, so a negative
        // biHeight means top-down (unlike the 12-byte CORE header, which
        // is u16 and bottom-up only).
        let w = 2u32;
        let mut bytes =
            build_truncated_os22x_8bpp(16, w, 2, &[0, 1, 1, 0], &[[1, 2, 3], [4, 5, 6]]);
        // Patch biHeight (@14 + 8 = offset 22) to -2 and reverse the two
        // pixel rows so the decoded top-down output is unchanged.
        let neg = (-2i32).to_le_bytes();
        bytes[22..26].copy_from_slice(&neg);
        // Swap the two rows in the pixel array. Pixel offset = 14 + 16 +
        // 256*4. Row stride = 4 (2 px padded to 4).
        let po = 14 + 16 + 256 * 4;
        let stride = 4;
        let (r0, r1) = bytes[po..po + 2 * stride].split_at(stride);
        let swapped: Vec<u8> = r1.iter().chain(r0.iter()).copied().collect();
        bytes[po..po + 2 * stride].copy_from_slice(&swapped);
        let img = decode_bmp(&bytes).unwrap();
        assert_eq!(img.width, w);
        assert_eq!(img.height, 2);
        assert_eq!(&img.planes[0].data[..4], &[1, 2, 3, 255]);
        assert_eq!(&img.planes[0].data[4..8], &[4, 5, 6, 255]);
    }

    #[test]
    fn truncated_os22x_rejects_bitfields_compression() {
        // A truncated OS/2 2.x header has no room for the appended mask
        // block, so a BI_BITFIELDS / Huffman-1D (value 3) declaration is
        // rejected rather than silently mis-decoded.
        let w = 2u32;
        let mut bytes =
            build_truncated_os22x_8bpp(20, w, 2, &[0, 1, 1, 0], &[[1, 2, 3], [4, 5, 6]]);
        // biCompression lives at offset 14 + 16 = 30 on a >=20-byte
        // header (header byte offset 16).
        bytes[30..34].copy_from_slice(&3u32.to_le_bytes());
        assert!(decode_bmp(&bytes).is_err());
    }

    // -----------------------------------------------------------------------
    // Full 64-byte OS/2 2.x OS22XBITMAPHEADER trailing-field metadata
    // -----------------------------------------------------------------------

    /// Build an 8-bpp BMP carrying a full 64-byte OS/2 2.x
    /// `OS22XBITMAPHEADER`: the 40-byte BITMAPINFOHEADER prefix plus the
    /// 24 trailing bytes (units / padding / recording / rendering /
    /// size1 / size2 / colour-encoding / identifier) at offsets 40..64.
    #[allow(clippy::too_many_arguments)]
    fn build_os22x_full_8bpp(
        w: u32,
        h: u32,
        indices_top_down: &[u8],
        palette: &[[u8; 3]],
        units: u16,
        recording: u16,
        rendering: u16,
        size1: u32,
        size2: u32,
        color_encoding: u32,
        identifier: u32,
    ) -> Vec<u8> {
        let row_stride = (w as usize).div_ceil(4) * 4;
        let mut pixel_array = vec![0u8; row_stride * h as usize];
        for y in 0..h as usize {
            let src_y = h as usize - 1 - y;
            let row = &mut pixel_array[y * row_stride..];
            for x in 0..w as usize {
                row[x] = indices_top_down[src_y * w as usize + x];
            }
        }
        let palette_entries = 256usize;
        let palette_bytes = (palette_entries * 4) as u32;
        let header_size = 64u32;
        let pixel_offset = 14 + header_size + palette_bytes;
        let file_size = pixel_offset + pixel_array.len() as u32;
        let mut out = Vec::with_capacity(file_size as usize);
        out.extend_from_slice(b"BM");
        out.extend_from_slice(&file_size.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&pixel_offset.to_le_bytes());
        // 40-byte BITMAPINFOHEADER prefix.
        out.extend_from_slice(&header_size.to_le_bytes()); // biSize @0
        out.extend_from_slice(&(w as i32).to_le_bytes()); // biWidth @4
        out.extend_from_slice(&(h as i32).to_le_bytes()); // biHeight @8
        out.extend_from_slice(&1u16.to_le_bytes()); // biPlanes @12
        out.extend_from_slice(&8u16.to_le_bytes()); // biBitCount @14
        out.extend_from_slice(&0u32.to_le_bytes()); // biCompression @16
        out.extend_from_slice(&0u32.to_le_bytes()); // biSizeImage @20
        out.extend_from_slice(&0i32.to_le_bytes()); // biXPelsPerM @24
        out.extend_from_slice(&0i32.to_le_bytes()); // biYPelsPerM @28
        out.extend_from_slice(&0u32.to_le_bytes()); // biClrUsed @32
        out.extend_from_slice(&0u32.to_le_bytes()); // biClrImportant @36
                                                    // 24-byte OS/2 2.x trailing block.
        out.extend_from_slice(&units.to_le_bytes()); // usUnits @40
        out.extend_from_slice(&0u16.to_le_bytes()); // padding @42
        out.extend_from_slice(&recording.to_le_bytes()); // usRecording @44
        out.extend_from_slice(&rendering.to_le_bytes()); // usRendering @46
        out.extend_from_slice(&size1.to_le_bytes()); // cSize1 @48
        out.extend_from_slice(&size2.to_le_bytes()); // cSize2 @52
        out.extend_from_slice(&color_encoding.to_le_bytes()); // ulColorEncoding @56
        out.extend_from_slice(&identifier.to_le_bytes()); // ulIdentifier @60
                                                          // Colour table: 256 RGBQUAD (B, G, R, 0) entries.
        for i in 0..palette_entries {
            if let Some(rgb) = palette.get(i) {
                out.push(rgb[2]);
                out.push(rgb[1]);
                out.push(rgb[0]);
                out.push(0);
            } else {
                out.extend_from_slice(&[0, 0, 0, 0]);
            }
        }
        out.extend_from_slice(&pixel_array);
        out
    }

    #[test]
    fn os22x_full_64byte_header_decodes_pixels() {
        // A 64-byte OS/2 2.x header decodes the same pixels as the other
        // header generations; the trailing block must not perturb the
        // colour table / pixel offsets.
        let w = 2u32;
        let h = 2u32;
        let indices = [0u8, 1, 1, 0];
        let palette: &[[u8; 3]] = &[[10, 20, 30], [200, 100, 50]];
        let bytes = build_os22x_full_8bpp(w, h, &indices, palette, 0, 0, 0, 0, 0, 0, 0);
        let img = decode_bmp(&bytes).unwrap();
        assert_eq!((img.width, img.height), (w, h));
        assert_eq!(&img.planes[0].data[..4], &[10, 20, 30, 255]);
        assert_eq!(&img.planes[0].data[4..8], &[200, 100, 50, 255]);
    }

    #[test]
    fn os22x_full_64byte_header_default_trailing_fields() {
        // All-zero trailing fields = the documented defaults.
        let bytes = build_os22x_full_8bpp(
            2,
            2,
            &[0, 1, 1, 0],
            &[[10, 20, 30], [200, 100, 50]],
            0,
            0,
            0,
            0,
            0,
            0,
            0,
        );
        let (_img, md) = decode_bmp_with_metadata(&bytes).unwrap();
        assert_eq!(md.header_size, 64);
        let h2 = md.os2_header2.expect("64-byte header carries os2_header2");
        assert_eq!(h2.units, 0);
        assert!(h2.units_is_pels_per_meter());
        assert_eq!(h2.recording, 0);
        assert!(h2.is_bottom_up());
        assert_eq!(h2.halftone, BmpOs2Halftone::None);
        assert_eq!(h2.halftone_size1, 0);
        assert_eq!(h2.halftone_size2, 0);
        assert_eq!(h2.color_encoding, 0);
        assert!(h2.color_encoding_is_rgb());
        assert_eq!(h2.identifier, 0);
        // The colour-space tail stays None — a 64-byte header is below the
        // 108-byte V4 threshold.
        assert_eq!(md.color_space, None);
        assert_eq!(md.rendering_intent, None);
    }

    #[test]
    fn os22x_full_64byte_header_error_diffusion_halftone() {
        // usRendering = 1 (error diffusion); size1 = 75% error damping.
        let bytes = build_os22x_full_8bpp(
            2,
            2,
            &[0, 1, 1, 0],
            &[[10, 20, 30], [200, 100, 50]],
            0,
            0,
            1,
            75,
            0,
            0,
            0xDEAD_BEEF,
        );
        let (_img, md) = decode_bmp_with_metadata(&bytes).unwrap();
        let h2 = md.os2_header2.unwrap();
        assert_eq!(h2.halftone, BmpOs2Halftone::ErrorDiffusion);
        assert_eq!(h2.halftone_size1, 75);
        assert_eq!(h2.identifier, 0xDEAD_BEEF);
    }

    #[test]
    fn os22x_full_64byte_header_panda_and_supercircle_halftone() {
        for (rendering, expected) in [
            (2u16, BmpOs2Halftone::Panda),
            (3u16, BmpOs2Halftone::SuperCircle),
        ] {
            let bytes = build_os22x_full_8bpp(
                2,
                2,
                &[0, 1, 1, 0],
                &[[10, 20, 30], [200, 100, 50]],
                0,
                0,
                rendering,
                16,
                32,
                0,
                0,
            );
            let (_img, md) = decode_bmp_with_metadata(&bytes).unwrap();
            let h2 = md.os2_header2.unwrap();
            assert_eq!(h2.halftone, expected, "rendering={rendering}");
            assert_eq!(h2.halftone_size1, 16);
            assert_eq!(h2.halftone_size2, 32);
        }
    }

    #[test]
    fn os22x_full_64byte_header_nonstandard_values_passthrough() {
        // Non-zero units / recording / colour-encoding / unknown halftone
        // are surfaced verbatim, and the convenience predicates report the
        // value differs from the documented default.
        let bytes = build_os22x_full_8bpp(
            2,
            2,
            &[0, 1, 1, 0],
            &[[10, 20, 30], [200, 100, 50]],
            7,
            9,
            42,
            0,
            0,
            5,
            0,
        );
        let (_img, md) = decode_bmp_with_metadata(&bytes).unwrap();
        let h2 = md.os2_header2.unwrap();
        assert_eq!(h2.units, 7);
        assert!(!h2.units_is_pels_per_meter());
        assert_eq!(h2.recording, 9);
        assert!(!h2.is_bottom_up());
        assert_eq!(h2.halftone, BmpOs2Halftone::Unknown(42));
        assert_eq!(h2.color_encoding, 5);
        assert!(!h2.color_encoding_is_rgb());
    }

    #[test]
    fn non_os22x_headers_have_no_os2_header2() {
        // The 12-byte CORE header, the truncated OS/2 2.x form, and a
        // plain V3 header all report `os2_header2 = None`.
        let core = build_os2_24bpp_bmp(2, 1, &[(1, 2, 3), (4, 5, 6)]);
        assert_eq!(decode_bmp_with_metadata(&core).unwrap().1.os2_header2, None);

        let trunc = build_truncated_os22x_8bpp(16, 2, 2, &[0, 1, 1, 0], &[[1, 2, 3], [4, 5, 6]]);
        assert_eq!(
            decode_bmp_with_metadata(&trunc).unwrap().1.os2_header2,
            None
        );
    }

    // -----------------------------------------------------------------------
    // Minimal palette (biClrUsed-limited colour table)
    // -----------------------------------------------------------------------

    #[test]
    fn minimal_palette_8bit_shrinks_table_and_roundtrips() {
        let w = 8u32;
        let h = 8u32;
        // Only 4 distinct indices in use; a 4-entry palette.
        let palette = four_color_palette();
        let (data, stride) = indexed_checker(w, h);
        let src = BmpImage {
            width: w,
            height: h,
            pixel_format: BmpPixelFormat::Indexed8,
            planes: vec![BmpPlane {
                stride,
                data: data.clone(),
            }],
            palette: Some(palette.clone()),
            pts: None,
        };

        // Full-palette (default) output.
        let (full_bytes, _) = encode_bmp(&src).unwrap();
        // Minimal-palette output: same pixels, smaller colour table.
        let (min_bytes, fmt) = encode_bmp_with_options(
            &src,
            BmpEncodeOptions {
                minimal_palette: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(
            fmt == EncodedBmpFormat::Indexed8 || fmt == EncodedBmpFormat::Rle8,
            "unexpected format {fmt:?}"
        );

        // The minimal table writes 4 entries vs the full 256 — the file
        // should be (256-4)*4 = 1008 bytes smaller in the uncompressed
        // case, and at least smaller in every case.
        assert!(
            min_bytes.len() < full_bytes.len(),
            "minimal palette ({}) should be smaller than full ({})",
            min_bytes.len(),
            full_bytes.len()
        );

        // biClrUsed (offset 14+32 = 46) must record the 4-entry count.
        let clr_used =
            u32::from_le_bytes([min_bytes[46], min_bytes[47], min_bytes[48], min_bytes[49]]);
        assert_eq!(clr_used, 4, "biClrUsed must record the partial-table size");

        // The decoder honours biClrUsed and produces identical pixels.
        let back_full = decode_bmp(&full_bytes).unwrap();
        let back_min = decode_bmp(&min_bytes).unwrap();
        assert_eq!(back_full.planes[0].data, back_min.planes[0].data);
        // Pixel (0,0) = index 0 → red.
        assert_eq!(&back_min.planes[0].data[..4], &[255u8, 0, 0, 255]);
    }

    #[test]
    fn minimal_palette_4bit_shrinks_table_and_roundtrips() {
        let w = 8u32;
        let h = 8u32;
        let palette = four_color_palette(); // 4 entries
        let (data, stride) = indexed_checker(w, h);
        let src = BmpImage {
            width: w,
            height: h,
            pixel_format: BmpPixelFormat::Indexed4,
            planes: vec![BmpPlane { stride, data }],
            palette: Some(palette),
            pts: None,
        };

        let (full_bytes, _) = encode_bmp(&src).unwrap();
        let (min_bytes, _) = encode_bmp_with_options(
            &src,
            BmpEncodeOptions {
                minimal_palette: true,
                ..Default::default()
            },
        )
        .unwrap();
        // 4-bit full table is 16 entries; minimal is 4 → (16-4)*4 = 48 B
        // smaller in the uncompressed case.
        assert!(
            min_bytes.len() < full_bytes.len(),
            "minimal 4-bit palette ({}) should be smaller than full ({})",
            min_bytes.len(),
            full_bytes.len()
        );
        let clr_used =
            u32::from_le_bytes([min_bytes[46], min_bytes[47], min_bytes[48], min_bytes[49]]);
        assert_eq!(clr_used, 4);

        let back = decode_bmp(&min_bytes).unwrap();
        assert_eq!(back.width, w);
        assert_eq!(back.height, h);
        assert_eq!(&back.planes[0].data[..4], &[255u8, 0, 0, 255]);
    }

    #[test]
    fn minimal_palette_full_table_keeps_clr_used_zero() {
        // A palette that already fills the whole 2^bpp space must keep the
        // classic clr_used = 0 sentinel even with minimal_palette set.
        let w = 4u32;
        let h = 4u32;
        let entries: Vec<[u8; 3]> = (0..16).map(|i| [i as u8 * 16, 0, 0]).collect();
        let palette = BmpPalette { entries };
        let (data, stride) = indexed_checker(w, h);
        let src = BmpImage {
            width: w,
            height: h,
            pixel_format: BmpPixelFormat::Indexed4,
            planes: vec![BmpPlane { stride, data }],
            palette: Some(palette),
            pts: None,
        };
        let (bytes, _) = encode_bmp_with_options(
            &src,
            BmpEncodeOptions {
                minimal_palette: true,
                ..Default::default()
            },
        )
        .unwrap();
        let clr_used = u32::from_le_bytes([bytes[46], bytes[47], bytes[48], bytes[49]]);
        assert_eq!(
            clr_used, 0,
            "a full 16-entry table keeps the clr_used=0 sentinel"
        );
        let back = decode_bmp(&bytes).unwrap();
        assert_eq!(back.width, w);
        assert_eq!(back.height, h);
    }

    #[test]
    fn minimal_palette_top_down_roundtrips() {
        // minimal_palette + top_down together: RLE is skipped (top-down),
        // the colour table is trimmed, and decode still matches.
        let w = 8u32;
        let h = 6u32;
        let palette = four_color_palette();
        let (data, stride) = indexed_checker(w, h);
        let src = BmpImage {
            width: w,
            height: h,
            pixel_format: BmpPixelFormat::Indexed8,
            planes: vec![BmpPlane { stride, data }],
            palette: Some(palette),
            pts: None,
        };
        let (bytes, fmt) = encode_bmp_with_options(
            &src,
            BmpEncodeOptions {
                top_down: true,
                minimal_palette: true,
            },
        )
        .unwrap();
        assert_eq!(fmt, EncodedBmpFormat::Indexed8);
        let stored_h = i32::from_le_bytes([bytes[22], bytes[23], bytes[24], bytes[25]]);
        assert_eq!(stored_h, -(h as i32));
        let clr_used = u32::from_le_bytes([bytes[46], bytes[47], bytes[48], bytes[49]]);
        assert_eq!(clr_used, 4);
        let back = decode_bmp(&bytes).unwrap();
        // top-down: row 0 of output = source row 0 → index 0 → red.
        assert_eq!(&back.planes[0].data[..4], &[255u8, 0, 0, 255]);
    }

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

    // --- Fuzz-derived regression tests (round 124) ---------------------
    //
    // Each of these crafts a malformed header that previously aborted the
    // process (integer-overflow panic or an exabyte allocation). They must
    // now return `Err` instead. Inputs are hand-built byte streams,
    // authored from the published BMP / Windows GDI header layout.

    /// 14-byte BITMAPFILEHEADER + 40-byte BITMAPINFOHEADER builder.
    fn raw_bmp(
        width: i32,
        height: i32,
        bpp: u16,
        compression: u32,
        pixel_offset: u32,
        clr_used: u32,
        tail: &[u8],
    ) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(b"BM");
        v.extend_from_slice(&0u32.to_le_bytes()); // file size
        v.extend_from_slice(&0u16.to_le_bytes()); // reserved1
        v.extend_from_slice(&0u16.to_le_bytes()); // reserved2
        v.extend_from_slice(&pixel_offset.to_le_bytes());
        v.extend_from_slice(&40u32.to_le_bytes()); // header size
        v.extend_from_slice(&width.to_le_bytes());
        v.extend_from_slice(&height.to_le_bytes());
        v.extend_from_slice(&1u16.to_le_bytes()); // planes
        v.extend_from_slice(&bpp.to_le_bytes());
        v.extend_from_slice(&compression.to_le_bytes());
        v.extend_from_slice(&0u32.to_le_bytes()); // image size
        v.extend_from_slice(&0i32.to_le_bytes()); // x ppm
        v.extend_from_slice(&0i32.to_le_bytes()); // y ppm
        v.extend_from_slice(&clr_used.to_le_bytes());
        v.extend_from_slice(&0u32.to_le_bytes()); // clr important
        v.extend_from_slice(tail);
        v
    }

    #[test]
    fn decode_dib_huge_clr_used_does_not_overflow() {
        // `clr_used = u32::MAX` with bpp=8: the old `palette_entries() as
        // u32 * entry_size` wrapped and panicked. Now it's a clean Err.
        let mut dib = Vec::new();
        dib.extend_from_slice(&40u32.to_le_bytes());
        dib.extend_from_slice(&2i32.to_le_bytes()); // width
        dib.extend_from_slice(&2i32.to_le_bytes()); // height
        dib.extend_from_slice(&1u16.to_le_bytes()); // planes
        dib.extend_from_slice(&8u16.to_le_bytes()); // bpp
        dib.extend_from_slice(&0u32.to_le_bytes()); // compression
        dib.extend_from_slice(&0u32.to_le_bytes()); // image size
        dib.extend_from_slice(&0i32.to_le_bytes());
        dib.extend_from_slice(&0i32.to_le_bytes());
        dib.extend_from_slice(&0xFFFF_FFFFu32.to_le_bytes()); // clr_used
        dib.extend_from_slice(&0u32.to_le_bytes());
        assert!(decode_dib(&dib, false).is_err());
    }

    #[test]
    fn rle8_giant_dimensions_rejected_not_oom() {
        // i32::MAX × i32::MAX RLE8 grid would have asked the allocator for
        // exabytes. The byte-ceiling check rejects it as inconsistent.
        let input = raw_bmp(i32::MAX, i32::MAX, 8, BI_RLE8, 1078, 256, &[0u8; 8]);
        assert!(decode_bmp(&input).is_err());
    }

    #[test]
    fn rle4_giant_dimensions_rejected_not_oom() {
        let input = raw_bmp(i32::MAX, i32::MAX, 4, BI_RLE4, 1078, 16, &[0u8; 8]);
        assert!(decode_bmp(&input).is_err());
    }

    #[test]
    fn rle8_pixel_offset_past_eof_rejected() {
        // pixel_offset beyond the buffer: the bare slice used to panic.
        let mut input = raw_bmp(2, 2, 8, BI_RLE8, 1078, 256, &[0u8; 64]);
        let off = (input.len() as u32) + 4096;
        input[10..14].copy_from_slice(&off.to_le_bytes());
        assert!(decode_bmp(&input).is_err());
    }

    #[test]
    fn zero_bpp_huge_height_rejected_not_oom() {
        // bpp=0 (BI_RGB) gives a zero row stride, so the truncation check
        // passed for any height and `decode_pixels` reserved a 134M-row
        // vector → OOM-abort. bpp is now validated up front.
        let input = raw_bmp(4, 134_283_268, 0, BI_RGB, 66, 3, &[0u8; 64]);
        assert!(decode_bmp(&input).is_err());
    }

    // -----------------------------------------------------------------------
    // CMYK family (BI_CMYK / BI_CMYKRLE8 / BI_CMYKRLE4) — recognised but
    // unsupported. The WMF-defined CMYK→RGB conversion is outside this
    // crate's BMP docs, so a CMYK bitmap is rejected with a distinct,
    // named error rather than the generic "unknown compression" path.
    // -----------------------------------------------------------------------

    #[test]
    fn cmyk_compression_constants_match_spec() {
        // The Wikipedia BMP compression-ID table: 11 / 12 / 13.
        assert_eq!(BI_CMYK, 11);
        assert_eq!(BI_CMYKRLE8, 12);
        assert_eq!(BI_CMYKRLE4, 13);
    }

    #[test]
    fn bi_cmyk_rejected_with_named_error() {
        // 32-bpp uncompressed CMYK body — enough bytes for a 1×1 grid.
        let input = raw_bmp(1, 1, 32, BI_CMYK, 54, 0, &[0u8; 4]);
        let err = decode_bmp(&input).expect_err("BI_CMYK must be rejected");
        let msg = format!("{err}");
        assert!(
            msg.contains("CMYK") && msg.contains("BI_CMYK"),
            "error should name the CMYK family: {msg}"
        );
    }

    #[test]
    fn bi_cmykrle8_rejected_with_named_error() {
        let input = raw_bmp(1, 1, 8, BI_CMYKRLE8, 1078, 256, &[0u8; 8]);
        let err = decode_bmp(&input).expect_err("BI_CMYKRLE8 must be rejected");
        let msg = format!("{err}");
        assert!(
            msg.contains("CMYK") && msg.contains("BI_CMYKRLE8"),
            "error should name BI_CMYKRLE8: {msg}"
        );
    }

    #[test]
    fn bi_cmykrle4_rejected_with_named_error() {
        let input = raw_bmp(1, 1, 4, BI_CMYKRLE4, 1078, 16, &[0u8; 8]);
        let err = decode_bmp(&input).expect_err("BI_CMYKRLE4 must be rejected");
        let msg = format!("{err}");
        assert!(
            msg.contains("CMYK") && msg.contains("BI_CMYKRLE4"),
            "error should name BI_CMYKRLE4: {msg}"
        );
    }

    #[test]
    fn cmyk_family_never_panics_on_truncated_body() {
        // Regression-style: a CMYK declaration with an empty / short body
        // must still error cleanly (no panic, no OOM) rather than reaching
        // a pixel decode path that doesn't exist for this family.
        for comp in [BI_CMYK, BI_CMYKRLE8, BI_CMYKRLE4] {
            let bpp = if comp == BI_CMYKRLE4 { 4 } else { 8 };
            let input = raw_bmp(64, 64, bpp, comp, 54, 0, &[]);
            assert!(
                decode_bmp(&input).is_err(),
                "CMYK compression {comp} must error, not decode"
            );
        }
    }

    // -----------------------------------------------------------------------
    // 1-bit indexed encode (monochrome)
    // -----------------------------------------------------------------------

    /// Two-entry palette: black + white.
    fn mono_palette() -> BmpPalette {
        BmpPalette {
            entries: vec![[0, 0, 0], [255, 255, 255]],
        }
    }

    /// Build a 1-bit indexed plane with a column-striped pattern:
    /// `x & 1 == 0` → index 0, else index 1. One byte per pixel.
    fn mono_stripes(w: u32, h: u32) -> Vec<u8> {
        let mut data = Vec::with_capacity((w * h) as usize);
        for _ in 0..h {
            for x in 0..w {
                data.push((x & 1) as u8);
            }
        }
        data
    }

    #[test]
    fn roundtrip_1bit_indexed() {
        let w = 16u32;
        let h = 8u32;
        let palette = mono_palette();
        let data = mono_stripes(w, h);
        let stride = w as usize;
        let src = BmpImage {
            width: w,
            height: h,
            pixel_format: BmpPixelFormat::Indexed1,
            planes: vec![BmpPlane { stride, data }],
            palette: Some(palette.clone()),
            pts: None,
        };
        let (bytes, fmt) = encode_bmp(&src).unwrap();
        assert_eq!(fmt, EncodedBmpFormat::Indexed1);
        assert_eq!(&bytes[..2], b"BM");

        // V3 header: bpp field at offset 14 + 14 = 28.
        let bpp = u16::from_le_bytes([bytes[28], bytes[29]]);
        assert_eq!(bpp, 1);

        // Compression at offset 30 = BI_RGB (0).
        let compression = u32::from_le_bytes([bytes[30], bytes[31], bytes[32], bytes[33]]);
        assert_eq!(compression, BI_RGB);

        let back = decode_bmp(&bytes).unwrap();
        assert_eq!(back.width, w);
        assert_eq!(back.height, h);
        // (0,0) → idx 0 → black, (1,0) → idx 1 → white, both opaque.
        assert_eq!(&back.planes[0].data[..4], &[0u8, 0, 0, 255]);
        assert_eq!(&back.planes[0].data[4..8], &[255u8, 255, 255, 255]);
        // (2,0) → idx 0 → black again.
        assert_eq!(&back.planes[0].data[8..12], &[0u8, 0, 0, 255]);
    }

    #[test]
    fn indexed1_without_palette_errors() {
        let w = 8u32;
        let h = 1u32;
        let src = BmpImage {
            width: w,
            height: h,
            pixel_format: BmpPixelFormat::Indexed1,
            planes: vec![BmpPlane {
                stride: w as usize,
                data: vec![0u8; w as usize],
            }],
            palette: None,
            pts: None,
        };
        assert!(encode_bmp(&src).is_err());
    }

    #[test]
    fn indexed1_packs_msb_first() {
        // Width-padded 1-bpp rows: 12-pixel width packs into 2 bytes + 2
        // pad bytes (32-bit alignment). Pattern 1,0,1,0,1,0,1,0,1,0,1,0
        // → bytes 0b10101010 + 0b10100000 + 00 + 00 in row 0.
        // With minimal_palette set, the colour table shrinks to 2
        // entries (8 bytes) so we can locate the pixel array deterministically.
        let w = 12u32;
        let h = 1u32;
        let palette = mono_palette();
        let mut data = Vec::with_capacity(w as usize);
        for x in 0..w {
            data.push(((x + 1) & 1) as u8); // 1,0,1,0,...
        }
        let src = BmpImage {
            width: w,
            height: h,
            pixel_format: BmpPixelFormat::Indexed1,
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
        assert_eq!(fmt, EncodedBmpFormat::Indexed1);
        // pixel_offset = 14 (file hdr) + 40 (V3) + 2 entries × 4 = 62.
        let pixel_offset = u32::from_le_bytes([bytes[10], bytes[11], bytes[12], bytes[13]]);
        assert_eq!(pixel_offset, 62);
        // Row 0 = 4 bytes (4-byte-aligned, even for 12 px / 1 bpp).
        assert_eq!(bytes[pixel_offset as usize], 0b1010_1010);
        // Last 4 bits (pixels 8-11) live in the high nibble of byte 1.
        assert_eq!(bytes[pixel_offset as usize + 1] & 0xF0, 0b1010_0000);
        // Padding bytes are zero.
        assert_eq!(bytes[pixel_offset as usize + 2], 0);
        assert_eq!(bytes[pixel_offset as usize + 3], 0);
    }

    #[test]
    fn indexed1_top_down_roundtrips() {
        let w = 16u32;
        let h = 4u32;
        let palette = mono_palette();
        let data = mono_stripes(w, h);
        let src = BmpImage {
            width: w,
            height: h,
            pixel_format: BmpPixelFormat::Indexed1,
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
                top_down: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(fmt, EncodedBmpFormat::Indexed1);
        // biHeight at offset 22 should be -(h as i32).
        let bi_height = i32::from_le_bytes([bytes[22], bytes[23], bytes[24], bytes[25]]);
        assert_eq!(bi_height, -(h as i32));
        let back = decode_bmp(&bytes).unwrap();
        assert_eq!(back.width, w);
        assert_eq!(back.height, h);
        // Stripes preserved: (0,0) black, (1,0) white.
        assert_eq!(&back.planes[0].data[..4], &[0u8, 0, 0, 255]);
        assert_eq!(&back.planes[0].data[4..8], &[255u8, 255, 255, 255]);
    }

    #[test]
    fn indexed1_minimal_palette_one_entry_clamped() {
        // A 1-bpp palette with a single entry must still produce a
        // valid file — `written_palette_entries` clamps to `[1, 2]`,
        // so we write exactly 1 colour table entry and record
        // `biClrUsed = 1`.
        let w = 8u32;
        let h = 1u32;
        let palette = BmpPalette {
            entries: vec![[42, 99, 200]],
        };
        // All-zero pixel data → every pixel resolves to entry 0.
        let src = BmpImage {
            width: w,
            height: h,
            pixel_format: BmpPixelFormat::Indexed1,
            planes: vec![BmpPlane {
                stride: w as usize,
                data: vec![0u8; w as usize],
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
        assert_eq!(fmt, EncodedBmpFormat::Indexed1);
        // biClrUsed at offset 14 + 32 = 46.
        let clr_used = u32::from_le_bytes([bytes[46], bytes[47], bytes[48], bytes[49]]);
        assert_eq!(clr_used, 1);
        let back = decode_bmp(&bytes).unwrap();
        // First pixel resolves to the single palette entry (R=42, G=99, B=200).
        assert_eq!(&back.planes[0].data[..4], &[42u8, 99, 200, 255]);
    }

    fn quad_palette() -> BmpPalette {
        // Four distinct, easily-checkable colours for the 2-bpp tests.
        BmpPalette {
            entries: vec![
                [0, 0, 0],   // idx 0 → black
                [255, 0, 0], // idx 1 → red
                [0, 255, 0], // idx 2 → green
                [0, 0, 255], // idx 3 → blue
            ],
        }
    }

    #[test]
    fn roundtrip_2bit_indexed() {
        // 8×4 image; each row is the repeating index pattern 0,1,2,3.
        let w = 8u32;
        let h = 4u32;
        let palette = quad_palette();
        let mut data = Vec::with_capacity((w * h) as usize);
        for _ in 0..h {
            for x in 0..w {
                data.push((x % 4) as u8);
            }
        }
        let src = BmpImage {
            width: w,
            height: h,
            pixel_format: BmpPixelFormat::Indexed2,
            planes: vec![BmpPlane {
                stride: w as usize,
                data,
            }],
            palette: Some(palette),
            pts: None,
        };
        let (bytes, fmt) = encode_bmp(&src).unwrap();
        assert_eq!(fmt, EncodedBmpFormat::Indexed2);
        assert_eq!(&bytes[..2], b"BM");
        // V3 header: bpp at offset 28, compression at 30.
        let bpp = u16::from_le_bytes([bytes[28], bytes[29]]);
        assert_eq!(bpp, 2);
        let compression = u32::from_le_bytes([bytes[30], bytes[31], bytes[32], bytes[33]]);
        assert_eq!(compression, BI_RGB);

        let back = decode_bmp(&bytes).unwrap();
        assert_eq!(back.width, w);
        assert_eq!(back.height, h);
        // (0,0)→idx0 black, (1,0)→idx1 red, (2,0)→idx2 green, (3,0)→idx3 blue.
        assert_eq!(&back.planes[0].data[0..4], &[0u8, 0, 0, 255]);
        assert_eq!(&back.planes[0].data[4..8], &[255u8, 0, 0, 255]);
        assert_eq!(&back.planes[0].data[8..12], &[0u8, 255, 0, 255]);
        assert_eq!(&back.planes[0].data[12..16], &[0u8, 0, 255, 255]);
    }

    #[test]
    fn indexed2_packs_four_per_byte_msb_first() {
        // 8-pixel width packs into 2 bytes + 2 pad bytes (32-bit aligned).
        // Indices 0,1,2,3,3,2,1,0 → byte0 = 00 01 10 11 = 0b0001_1011,
        // byte1 = 11 10 01 00 = 0b1110_0100. minimal_palette → 4 entries.
        let w = 8u32;
        let h = 1u32;
        let palette = quad_palette();
        let data: Vec<u8> = vec![0, 1, 2, 3, 3, 2, 1, 0];
        let src = BmpImage {
            width: w,
            height: h,
            pixel_format: BmpPixelFormat::Indexed2,
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
        assert_eq!(fmt, EncodedBmpFormat::Indexed2);
        // pixel_offset = 14 + 40 + 4 entries × 4 = 70.
        let pixel_offset = u32::from_le_bytes([bytes[10], bytes[11], bytes[12], bytes[13]]);
        assert_eq!(pixel_offset, 70);
        let po = pixel_offset as usize;
        assert_eq!(bytes[po], 0b0001_1011);
        assert_eq!(bytes[po + 1], 0b1110_0100);
        // Row is padded to 4 bytes; trailing two are zero.
        assert_eq!(bytes[po + 2], 0);
        assert_eq!(bytes[po + 3], 0);
    }

    #[test]
    fn indexed2_without_palette_errors() {
        let src = BmpImage {
            width: 4,
            height: 1,
            pixel_format: BmpPixelFormat::Indexed2,
            planes: vec![BmpPlane {
                stride: 4,
                data: vec![0u8; 4],
            }],
            palette: None,
            pts: None,
        };
        assert!(encode_bmp(&src).is_err());
    }

    #[test]
    fn indexed2_top_down_roundtrips() {
        let w = 8u32;
        let h = 3u32;
        let palette = quad_palette();
        let mut data = Vec::with_capacity((w * h) as usize);
        for _ in 0..h {
            for x in 0..w {
                data.push((x % 4) as u8);
            }
        }
        let src = BmpImage {
            width: w,
            height: h,
            pixel_format: BmpPixelFormat::Indexed2,
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
                top_down: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(fmt, EncodedBmpFormat::Indexed2);
        // biHeight negative for top-down.
        let bi_height = i32::from_le_bytes([bytes[22], bytes[23], bytes[24], bytes[25]]);
        assert_eq!(bi_height, -(h as i32));
        let back = decode_bmp(&bytes).unwrap();
        assert_eq!(back.width, w);
        assert_eq!(back.height, h);
        // Row 0: idx0 black, idx1 red.
        assert_eq!(&back.planes[0].data[0..4], &[0u8, 0, 0, 255]);
        assert_eq!(&back.planes[0].data[4..8], &[255u8, 0, 0, 255]);
    }

    #[test]
    fn indexed2_minimal_palette_clamps() {
        // A 2-bpp palette with a single entry clamps to `[1, 4]`; we write
        // exactly 1 colour table entry and record `biClrUsed = 1`.
        let w = 4u32;
        let h = 1u32;
        let palette = BmpPalette {
            entries: vec![[10, 20, 30]],
        };
        let src = BmpImage {
            width: w,
            height: h,
            pixel_format: BmpPixelFormat::Indexed2,
            planes: vec![BmpPlane {
                stride: w as usize,
                data: vec![0u8; w as usize],
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
        assert_eq!(fmt, EncodedBmpFormat::Indexed2);
        // biClrUsed at offset 14 + 32 = 46.
        let clr_used = u32::from_le_bytes([bytes[46], bytes[47], bytes[48], bytes[49]]);
        assert_eq!(clr_used, 1);
        let back = decode_bmp(&bytes).unwrap();
        assert_eq!(&back.planes[0].data[..4], &[10u8, 20, 30, 255]);
    }

    #[test]
    fn rle8_small_consistent_dims_still_decode() {
        // Sanity floor: a legitimately-tiny RLE8 grid must NOT be caught
        // by the ceiling guard. 1×1, palette of 1 black entry, one
        // encoded run of a single index + end-of-bitmap.
        let mut input = raw_bmp(1, 1, 8, BI_RLE8, 0, 1, &[]);
        // pixel_offset = 14 + 40 + 1*4 (one palette entry) = 58.
        let off = 14 + 40 + 4;
        input[10..14].copy_from_slice(&(off as u32).to_le_bytes());
        // one palette entry (BGRA) then RLE: [1,0] run of idx0, [0,1] end.
        input.extend_from_slice(&[0x10, 0x20, 0x30, 0x00]);
        input.extend_from_slice(&[1, 0, 0, 1]);
        let img = decode_bmp(&input).expect("tiny RLE8 must decode");
        assert_eq!((img.width, img.height), (1, 1));
    }

    // -----------------------------------------------------------------------
    // BI_ALPHABITFIELDS (compression value 6) — V3 header with R/G/B/A masks
    // -----------------------------------------------------------------------
    //
    // V3 `BITMAPINFOHEADER` (40 B) with `biCompression = BI_ALPHABITFIELDS`
    // (= 6) carries 16 bytes of masks (R, G, B, A) immediately after the
    // header, where `BI_BITFIELDS` (3) would have carried only 12 bytes
    // (R, G, B). The Windows CE / NT 5.0+ variant exists because some
    // producers want to declare a per-mask alpha channel without bumping
    // the header to V4 (BITMAPV4HEADER, 108 B). On V4/V5 headers the
    // distinction collapses since the alpha mask is already part of the
    // fixed header layout — we treat both compression values identically
    // for header_size >= 108.

    /// Build a V3 BMP with `BI_ALPHABITFIELDS`, 32-bpp, the four R/G/B/A
    /// masks supplied verbatim, and a single 0xAABBCCDD payload word.
    fn raw_alpha_bitfields_32bpp(masks: [u32; 4], pixel: u32) -> Vec<u8> {
        // pixel_offset = 14 (file header) + 40 (info header) + 16 (masks) = 70
        let mut input = raw_bmp(1, 1, 32, BI_ALPHABITFIELDS, 70, 0, &[]);
        for m in masks {
            input.extend_from_slice(&m.to_le_bytes());
        }
        input.extend_from_slice(&pixel.to_le_bytes());
        input
    }

    #[test]
    fn alpha_bitfields_v3_32bpp_decodes() {
        // Canonical BGRA-style mask layout: R=0x00FF0000, G=0x0000FF00,
        // B=0x000000FF, A=0xFF000000. Payload 0xAABBCCDD decomposes
        // (little-endian on disk) to a u32 word = 0xAABBCCDD.
        //   R = (0xAABBCCDD & 0x00FF0000) >> 16 = 0xBB
        //   G = (0xAABBCCDD & 0x0000FF00) >>  8 = 0xCC
        //   B = (0xAABBCCDD & 0x000000FF)       = 0xDD
        //   A = (0xAABBCCDD & 0xFF000000) >> 24 = 0xAA
        let bytes = raw_alpha_bitfields_32bpp(
            [0x00FF_0000, 0x0000_FF00, 0x0000_00FF, 0xFF00_0000],
            0xAABB_CCDD,
        );
        let img = decode_bmp(&bytes).expect("BI_ALPHABITFIELDS 32bpp must decode");
        assert_eq!((img.width, img.height), (1, 1));
        assert_eq!(&img.planes[0].data[..4], &[0xBB, 0xCC, 0xDD, 0xAA]);
    }

    #[test]
    fn alpha_bitfields_v3_32bpp_alpha_zero_means_transparent() {
        // Same masks but payload alpha nibble is zero → output alpha is 0.
        // The compression code path is exercised end-to-end (header parse,
        // 16-byte mask tail, mask-driven 32-bpp expansion).
        let bytes = raw_alpha_bitfields_32bpp(
            [0x00FF_0000, 0x0000_FF00, 0x0000_00FF, 0xFF00_0000],
            0x00FF_FFFF,
        );
        let img = decode_bmp(&bytes).expect("must decode");
        assert_eq!(img.planes[0].data[3], 0x00);
    }

    #[test]
    fn alpha_bitfields_v3_truncated_masks_rejected() {
        // Compression = BI_ALPHABITFIELDS but the 4 mask words don't all
        // fit after the 40-byte header. The parser must reject rather than
        // index past the buffer.
        let mut input = raw_bmp(1, 1, 32, BI_ALPHABITFIELDS, 70, 0, &[]);
        // Only 12 of the 16 mask bytes — enough for BI_BITFIELDS but
        // short for BI_ALPHABITFIELDS.
        input.extend_from_slice(&0x00FF_0000u32.to_le_bytes());
        input.extend_from_slice(&0x0000_FF00u32.to_le_bytes());
        input.extend_from_slice(&0x0000_00FFu32.to_le_bytes());
        // No alpha mask, no pixel.
        assert!(decode_bmp(&input).is_err());
    }

    #[test]
    fn alpha_bitfields_v3_zero_alpha_mask_yields_opaque() {
        // Alpha mask = 0 means "no alpha" even with BI_ALPHABITFIELDS:
        // the 16-byte mask tail is read in full but the alpha slot is
        // zeroed, so output alpha falls back to the canonical 0xFF.
        let bytes = raw_alpha_bitfields_32bpp(
            [0x00FF_0000, 0x0000_FF00, 0x0000_00FF, 0x0000_0000],
            0x0011_2233,
        );
        let img = decode_bmp(&bytes).expect("must decode");
        assert_eq!(&img.planes[0].data[..4], &[0x11, 0x22, 0x33, 0xFF]);
    }

    // -----------------------------------------------------------------------
    // V2 (52 B) `BITMAPV2INFOHEADER` + V3 (56 B) `BITMAPV3INFOHEADER` —
    // Adobe-published intermediate DIB header variants.
    // -----------------------------------------------------------------------
    //
    // V2 extends `BITMAPINFOHEADER` (40 B) by 12 bytes of in-header R/G/B
    // bit masks at offsets 40, 44, 48; V3 extends V2 by a 4-byte alpha
    // mask at offset 52. The mask block sits in the same byte slots V4 /
    // V5 use, so a 52-byte `BI_BITFIELDS` BMP — or a 56-byte one with
    // alpha — should decode through the same mask-driven 16/32-bpp path
    // as the V4 / V5 case, just without the colour-space tail. Per the
    // Wikipedia survey of header generations, this lets the decoder
    // accept files written by readers that adopted the in-header masks
    // without committing to the full 108-byte V4 colour-space block.

    /// Build a `BITMAPV{2,3}INFOHEADER` BMP with `BI_BITFIELDS`,
    /// 32-bpp, the supplied R/G/B (and optional alpha) masks living
    /// **inside** the header at offsets 40 / 44 / 48 / 52, and a single
    /// 4-byte pixel payload directly after the header.
    fn raw_v2_or_v3_info_32bpp(
        header_size: u32,
        masks_rgb: [u32; 3],
        mask_a: Option<u32>,
        pixel: u32,
    ) -> Vec<u8> {
        assert!(header_size == 52 || header_size == 56);
        // pixel_offset = 14 (file header) + header_size; no trailing
        // mask tail (masks ride inside the header body).
        let pixel_offset = BITMAPFILEHEADER_SIZE + header_size;
        let mut v = Vec::new();
        v.extend_from_slice(b"BM");
        v.extend_from_slice(&0u32.to_le_bytes()); // file size — leave 0, decoder ignores
        v.extend_from_slice(&0u16.to_le_bytes()); // reserved1
        v.extend_from_slice(&0u16.to_le_bytes()); // reserved2
        v.extend_from_slice(&pixel_offset.to_le_bytes());
        // DIB header body
        v.extend_from_slice(&header_size.to_le_bytes());
        v.extend_from_slice(&1i32.to_le_bytes()); // width
        v.extend_from_slice(&1i32.to_le_bytes()); // height (bottom-up)
        v.extend_from_slice(&1u16.to_le_bytes()); // planes
        v.extend_from_slice(&32u16.to_le_bytes()); // bpp
        v.extend_from_slice(&BI_BITFIELDS.to_le_bytes());
        v.extend_from_slice(&0u32.to_le_bytes()); // biSizeImage
        v.extend_from_slice(&0i32.to_le_bytes()); // x ppm
        v.extend_from_slice(&0i32.to_le_bytes()); // y ppm
        v.extend_from_slice(&0u32.to_le_bytes()); // clr used
        v.extend_from_slice(&0u32.to_le_bytes()); // clr important
                                                  // In-header masks
        v.extend_from_slice(&masks_rgb[0].to_le_bytes()); // R @ 40
        v.extend_from_slice(&masks_rgb[1].to_le_bytes()); // G @ 44
        v.extend_from_slice(&masks_rgb[2].to_le_bytes()); // B @ 48
        if header_size == 56 {
            // Alpha mask @ 52; if caller didn't pass one assume zero
            // ("no alpha", opaque per `BI_BITFIELDS` convention).
            v.extend_from_slice(&mask_a.unwrap_or(0).to_le_bytes());
        }
        // Pixel payload (one 4-byte BGRA word).
        v.extend_from_slice(&pixel.to_le_bytes());
        v
    }

    #[test]
    fn v2_info_header_52b_bitfields_32bpp_decodes() {
        // V2 has only R/G/B in-header masks (no alpha slot). Canonical
        // BGRA-style masks; pixel 0xAABBCCDD splits as:
        //   R = (… & 0x00FF0000) >> 16 = 0xBB
        //   G = (… & 0x0000FF00) >>  8 = 0xCC
        //   B = (… & 0x000000FF)       = 0xDD
        //   A → opaque (no alpha mask present on V2)
        let bytes = raw_v2_or_v3_info_32bpp(
            52,
            [0x00FF_0000, 0x0000_FF00, 0x0000_00FF],
            None,
            0xAABB_CCDD,
        );
        let img = decode_bmp(&bytes).expect("V2 BITMAPV2INFOHEADER must decode");
        assert_eq!((img.width, img.height), (1, 1));
        assert_eq!(&img.planes[0].data[..4], &[0xBB, 0xCC, 0xDD, 0xFF]);
    }

    #[test]
    fn v3_info_header_56b_bitfields_32bpp_decodes_with_alpha() {
        // V3 adds the in-header alpha mask at offset 52. Same canonical
        // BGRA-style layout as V4, but on a 56-byte header (no colour-
        // space tail). Payload alpha nibble survives.
        let bytes = raw_v2_or_v3_info_32bpp(
            56,
            [0x00FF_0000, 0x0000_FF00, 0x0000_00FF],
            Some(0xFF00_0000),
            0xAABB_CCDD,
        );
        let img = decode_bmp(&bytes).expect("V3 BITMAPV3INFOHEADER must decode");
        assert_eq!((img.width, img.height), (1, 1));
        assert_eq!(&img.planes[0].data[..4], &[0xBB, 0xCC, 0xDD, 0xAA]);
    }

    #[test]
    fn v3_info_header_56b_zero_alpha_mask_yields_opaque() {
        // Alpha mask = 0 on V3 means "no alpha" — same convention as
        // V3 `BI_ALPHABITFIELDS` and V4 / V5 with an explicit zero mask.
        let bytes = raw_v2_or_v3_info_32bpp(
            56,
            [0x00FF_0000, 0x0000_FF00, 0x0000_00FF],
            Some(0),
            0x0011_2233,
        );
        let img = decode_bmp(&bytes).expect("must decode");
        assert_eq!(&img.planes[0].data[..4], &[0x11, 0x22, 0x33, 0xFF]);
    }

    #[test]
    fn v2_info_header_52b_metadata_reports_header_size() {
        // Metadata path: the 52-byte header pre-dates the V4 colour-
        // space tail (cs_type / endpoints / gamma at offset 56+), so
        // every colour-management field stays `None`. The classic V3
        // tail (DPI + colour-count) is still readable since V2 inherits
        // every byte 24..40 from `BITMAPINFOHEADER`.
        let bytes = raw_v2_or_v3_info_32bpp(
            52,
            [0x00FF_0000, 0x0000_FF00, 0x0000_00FF],
            None,
            0x0000_00FF,
        );
        let (_img, md) = decode_bmp_with_metadata(&bytes).expect("decode + metadata");
        assert_eq!(md.header_size, 52);
        assert!(md.color_space.is_none());
        assert!(md.endpoints.is_none());
        assert!(md.gamma_rgb.is_none());
        assert!(md.rendering_intent.is_none());
        assert_eq!(md.pixels_per_meter_x, Some(0));
        assert_eq!(md.colors_used, Some(0));
    }

    // -----------------------------------------------------------------------
    // V4 / V5 colour-space metadata (`bV4CSType`, `bV5Intent`,
    // `bV5ProfileData`, `bV5ProfileSize`) + embedded ICC profile roundtrip
    // -----------------------------------------------------------------------
    //
    // The V4 header (108 bytes) introduces the `bV4CSType` field plus the
    // `CIEXYZTRIPLE` endpoints + RGB gamma triple. The V5 header (124 bytes)
    // extends V4 with `bV5Intent`, `bV5ProfileData`, `bV5ProfileSize`, and a
    // reserved u32. A V5 BMP with `bV5CSType = PROFILE_EMBEDDED` carries an
    // ICC profile blob after the pixel array at offset
    // `BITMAPFILEHEADER_SIZE + bV5ProfileData`. These tests build such
    // headers by hand against the published BMP / Windows GDI byte layout.

    /// Hand-assemble a 1×1 BMP with a V5 header declaring sRGB. Useful as
    /// a minimal smoke-test that the V5 parser reads the right offsets.
    fn build_v5_srgb_32bpp(pixel: u32, intent: u32) -> Vec<u8> {
        // pixel_offset = 14 (file hdr) + 124 (V5) = 138; one 4-byte BGRA word.
        let pixel_bytes = 4u32;
        let file_size = BITMAPFILEHEADER_SIZE + BITMAPV5HEADER_SIZE + pixel_bytes;
        let pixel_offset = BITMAPFILEHEADER_SIZE + BITMAPV5HEADER_SIZE;
        let mut out = Vec::with_capacity(file_size as usize);
        out.extend_from_slice(b"BM");
        out.extend_from_slice(&file_size.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&pixel_offset.to_le_bytes());
        // BITMAPV5HEADER body:
        out.extend_from_slice(&BITMAPV5HEADER_SIZE.to_le_bytes());
        out.extend_from_slice(&1i32.to_le_bytes()); // width
        out.extend_from_slice(&1i32.to_le_bytes()); // height (positive: bottom-up)
        out.extend_from_slice(&1u16.to_le_bytes()); // planes
        out.extend_from_slice(&32u16.to_le_bytes()); // bpp
        out.extend_from_slice(&BI_RGB.to_le_bytes()); // compression
        out.extend_from_slice(&pixel_bytes.to_le_bytes()); // image size
        out.extend_from_slice(&0i32.to_le_bytes()); // x_pels/m
        out.extend_from_slice(&0i32.to_le_bytes()); // y_pels/m
        out.extend_from_slice(&0u32.to_le_bytes()); // clr_used
        out.extend_from_slice(&0u32.to_le_bytes()); // clr_important
                                                    // R / G / B / A masks (zeroed for BI_RGB).
        out.extend_from_slice(&[0u8; 16]);
        // bV5CSType = LCS_sRGB.
        out.extend_from_slice(&LCS_S_RGB.to_le_bytes());
        // CIEXYZTRIPLE endpoints (zero) + RGB gamma (zero).
        out.extend_from_slice(&[0u8; 36]);
        out.extend_from_slice(&[0u8; 12]);
        // V5 tail: intent + profile data + profile size + reserved.
        out.extend_from_slice(&intent.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        // The 1×1 pixel.
        out.extend_from_slice(&pixel.to_le_bytes());
        out
    }

    #[test]
    fn v5_srgb_header_parses_and_decodes() {
        let bytes = build_v5_srgb_32bpp(0x80FF0011, 0);
        let (img, md) = decode_bmp_with_metadata(&bytes).expect("V5 sRGB must decode");
        assert_eq!((img.width, img.height), (1, 1));
        // 0x80FF0011 LE on disk = [0x11, 0x00, 0xFF, 0x80] = BGRA →
        // B=0x11, G=0x00, R=0xFF. The high byte is 0x80, but this V5 header
        // declares BI_RGB with a *zero* in-header alpha mask, so the alpha
        // sample is not valid (the high byte is the reserved DWORD slot per
        // the BI_RGB definition) and the pixel decodes opaque (A=0xFF), not
        // A=0x80. A V5 BI_RGB bitmap that genuinely wants alpha sets a
        // non-zero bV5AlphaMask (covered by `v5_bi_rgb_nonzero_alpha_mask_*`).
        assert_eq!(&img.planes[0].data[..4], &[0xFF, 0x00, 0x11, 0xFF]);
        assert_eq!(md.header_size, BITMAPV5HEADER_SIZE);
        assert_eq!(md.color_space, Some(BmpColorSpace::SRgb));
        assert_eq!(md.rendering_intent, Some(BmpRenderingIntent::Unspecified));
        assert!(md.endpoints.is_some());
        assert!(md.gamma_rgb.is_some());
        assert_eq!(md.profile_data_offset, Some(0));
        assert_eq!(md.profile_size, Some(0));
        assert!(md.icc_profile.is_none());
    }

    #[test]
    fn v5_intent_perceptual_round_trips() {
        // Same fixture, different intent — verify enum mapping is correct.
        let bytes = build_v5_srgb_32bpp(0x00FF00FF, LCS_GM_IMAGES);
        let (_, md) = decode_bmp_with_metadata(&bytes).expect("V5 must decode");
        assert_eq!(md.rendering_intent, Some(BmpRenderingIntent::Perceptual));
    }

    /// Take a `build_v5_srgb_32bpp` byte buffer and overwrite the four
    /// in-header R/G/B/A masks (offsets 40..56 in the DIB body, i.e. file
    /// offsets 54..70) with the given quadruple. Used to exercise the
    /// V5 BI_RGB in-header alpha-mask path.
    fn with_v5_masks(mut bytes: Vec<u8>, masks_rgba: [u32; 4]) -> Vec<u8> {
        // file header (14) + DIB body offset 40 = 54.
        let base = 14 + 40;
        for (i, m) in masks_rgba.iter().enumerate() {
            bytes[base + i * 4..base + i * 4 + 4].copy_from_slice(&m.to_le_bytes());
        }
        bytes
    }

    #[test]
    fn v5_bi_rgb_zero_alpha_mask_is_opaque() {
        // A V5 header declaring BI_RGB with an all-zero in-header alpha mask
        // has no valid alpha sample: the high byte is the reserved DWORD
        // slot, so even a non-zero high byte decodes opaque. This is the
        // documented zero-alpha-mask → opaque convention applied to the
        // V4/V5 BI_RGB layout (matching BI_ALPHABITFIELDS / V3-zero-alpha).
        let bytes = build_v5_srgb_32bpp(0x00FF0011, 0); // high byte 0x00
        let img = decode_bmp(&bytes).expect("V5 BI_RGB must decode");
        assert_eq!(&img.planes[0].data[..4], &[0xFF, 0x00, 0x11, 0xFF]);
        // A non-zero reserved byte must ALSO decode opaque under a zero mask.
        let bytes = build_v5_srgb_32bpp(0x7FFF0011, 0); // high byte 0x7F
        let img = decode_bmp(&bytes).expect("V5 BI_RGB must decode");
        assert_eq!(&img.planes[0].data[..4], &[0xFF, 0x00, 0x11, 0xFF]);
    }

    #[test]
    fn v5_bi_rgb_canonical_alpha_mask_honoured() {
        // V5 BI_RGB with the canonical ARGB high-byte alpha mask
        // (0xFF000000): the alpha sample is valid and the high byte carries
        // opacity. R/G/B keep the default BGRA byte order (the R/G/B masks
        // are not valid under BI_RGB, only the alpha mask is).
        let bytes = with_v5_masks(build_v5_srgb_32bpp(0x80FF0011, 0), [0, 0, 0, 0xFF00_0000]);
        // 0x80FF0011 LE = [0x11, 0x00, 0xFF, 0x80]: B=0x11 G=0x00 R=0xFF,
        // alpha from the high byte via the 0xFF000000 mask = 0x80.
        let img = decode_bmp(&bytes).expect("V5 BI_RGB alpha-mask must decode");
        assert_eq!(&img.planes[0].data[..4], &[0xFF, 0x00, 0x11, 0x80]);
    }

    #[test]
    fn v5_bi_rgb_noncanonical_alpha_mask_honoured() {
        // The alpha mask need not sit in the high byte: a writer may park
        // alpha in the low byte (0x000000FF) while R/G/B still occupy the
        // BGRA byte order. The decoder must extract alpha through the mask,
        // not from a fixed byte position.
        let bytes = with_v5_masks(build_v5_srgb_32bpp(0x0011_2233, 0), [0, 0, 0, 0x0000_00FF]);
        // 0x00112233 LE = [0x33, 0x22, 0x11, 0x00]. With the low-byte alpha
        // mask, alpha = 0x33; B=src[0]=0x33, G=src[1]=0x22, R=src[2]=0x11.
        let img = decode_bmp(&bytes).expect("V5 BI_RGB low-byte alpha must decode");
        assert_eq!(&img.planes[0].data[..4], &[0x11, 0x22, 0x33, 0x33]);
    }

    #[test]
    fn v3_bi_rgb_32bpp_still_reads_reserved_byte_as_alpha() {
        // Guard the deliberate asymmetry: a plain 40-byte BITMAPINFOHEADER
        // (V3) has no in-header alpha-mask slot, so the decoder keeps its
        // historical "high byte = alpha" behaviour there (this is what the
        // crate's own 32-bit BGRA encoder relies on for a lossless RGBA
        // round-trip). The opaque-fallback correction is scoped to V4/V5,
        // which carry an explicit alpha-mask slot to disambiguate.
        let (src, _, _) = rgba_checker(4, 4); // includes a quadrant with A=128
        let (bytes, fmt) = encode_bmp(&src).unwrap();
        assert_eq!(fmt, EncodedBmpFormat::Rgb32);
        let back = decode_bmp(&bytes).expect("V3 BI_RGB must decode");
        assert_eq!(back.planes[0].data, src.planes[0].data);
    }

    #[test]
    fn v3_decode_with_metadata_has_no_v4_v5_fields() {
        // Sanity floor: a plain V3 (40-byte) bitmap must report `None` for
        // every V4 / V5 field. The `decode_bmp` and
        // `decode_bmp_with_metadata` entry points share the same pixel
        // pipeline so the image must match the V3 reference too.
        let (src, _, _) = rgba_checker(4, 4);
        let (bytes, _) = encode_bmp(&src).unwrap();
        let (img, md) = decode_bmp_with_metadata(&bytes).expect("V3 must decode");
        assert_eq!(img.planes[0].data, src.planes[0].data);
        assert_eq!(md.header_size, BITMAPINFOHEADER_SIZE);
        assert!(md.color_space.is_none());
        assert!(md.rendering_intent.is_none());
        assert!(md.endpoints.is_none());
        assert!(md.gamma_rgb.is_none());
        assert!(md.profile_data_offset.is_none());
        assert!(md.profile_size.is_none());
        assert!(md.icc_profile.is_none());
    }

    #[test]
    fn v4_rgb565_metadata_surfaces_srgb() {
        // The standard 16-bit RGB565 encode path emits a V4 header with
        // CSType = LCS_sRGB; verify the metadata path reads that back.
        // Smallest-possible 1×1 fixture so the test doesn't depend on
        // bit-pattern details.
        let src = BmpImage {
            width: 1,
            height: 1,
            pixel_format: BmpPixelFormat::Rgb565,
            planes: vec![BmpPlane {
                stride: 2,
                data: vec![0xE0, 0x07], // green
            }],
            palette: None,
            pts: None,
        };
        let (bytes, _) = encode_bmp(&src).unwrap();
        let (_, md) = decode_bmp_with_metadata(&bytes).expect("V4 must decode");
        assert_eq!(md.header_size, BITMAPV4HEADER_SIZE);
        assert_eq!(md.color_space, Some(BmpColorSpace::SRgb));
        // V4 doesn't carry an intent — surfaced as `None` (not as
        // `Some(Unspecified)`) so callers can tell apart "V4 header" from
        // "V5 header that set intent = 0".
        assert!(md.rendering_intent.is_none());
        assert!(md.icc_profile.is_none());
    }

    #[test]
    fn v5_with_embedded_icc_profile_roundtrips() {
        // Encoder side: V5 + ICC profile blob.
        // Decoder side: pulls the ICC blob back out of the metadata.
        let (src, w, h) = rgba_checker(4, 4);
        let icc = b"fakeICCprofileblob_v4.3\0\0\0".to_vec();
        let bytes =
            encode_bmp_with_icc_profile(&src, &icc, LCS_GM_IMAGES, BmpEncodeOptions::default())
                .expect("V5 + ICC encode must succeed");
        // Verify on-disk header is V5.
        let header_size = u32::from_le_bytes([bytes[14], bytes[15], bytes[16], bytes[17]]);
        assert_eq!(header_size, BITMAPV5HEADER_SIZE);
        let cs_type = u32::from_le_bytes([
            bytes[14 + 56],
            bytes[14 + 57],
            bytes[14 + 58],
            bytes[14 + 59],
        ]);
        assert_eq!(cs_type, PROFILE_EMBEDDED);

        let (img, md) = decode_bmp_with_metadata(&bytes).expect("V5 + ICC must decode");
        assert_eq!(img.width, w);
        assert_eq!(img.height, h);
        assert_eq!(img.planes[0].data, src.planes[0].data);
        assert_eq!(md.color_space, Some(BmpColorSpace::ProfileEmbedded));
        assert_eq!(md.rendering_intent, Some(BmpRenderingIntent::Perceptual));
        assert_eq!(md.icc_profile.as_deref(), Some(icc.as_slice()));
        assert_eq!(md.profile_size, Some(icc.len() as u32));
    }

    #[test]
    fn v5_embedded_icc_top_down_preserves_profile() {
        // Same encode path but with top_down — biHeight goes negative
        // and the ICC blob still rounds-trips bit-for-bit.
        let (src, _, h) = rgba_checker(2, 2);
        let icc = vec![0xAB, 0xCD, 0xEF, 0x12, 0x34, 0x56];
        let bytes = encode_bmp_with_icc_profile(
            &src,
            &icc,
            0,
            BmpEncodeOptions {
                top_down: true,
                ..Default::default()
            },
        )
        .expect("V5 top-down must succeed");
        // biHeight at offset 22 should be negative.
        let bi_height = i32::from_le_bytes([bytes[22], bytes[23], bytes[24], bytes[25]]);
        assert_eq!(bi_height, -(h as i32));
        let (_, md) = decode_bmp_with_metadata(&bytes).expect("decode V5 top-down");
        assert_eq!(md.icc_profile.as_deref(), Some(icc.as_slice()));
        assert_eq!(md.rendering_intent, Some(BmpRenderingIntent::Unspecified));
    }

    #[test]
    fn v5_embedded_icc_rgb24_path() {
        // The V5 + ICC path also supports 24-bit BGR. Smaller payload
        // exercises a different `pack_*` helper end-to-end.
        let src = rgb24_checker(4, 2);
        let icc = b"icc:rgb24-test".to_vec();
        let bytes =
            encode_bmp_with_icc_profile(&src, &icc, LCS_GM_BUSINESS, BmpEncodeOptions::default())
                .expect("V5 + RGB24 ICC encode");
        let (img, md) = decode_bmp_with_metadata(&bytes).expect("decode");
        assert_eq!((img.width, img.height), (4, 2));
        assert_eq!(md.icc_profile.as_deref(), Some(icc.as_slice()));
        assert_eq!(md.rendering_intent, Some(BmpRenderingIntent::Saturation));
        // Pixel (0,0) is red per `rgb24_checker`.
        assert_eq!(&img.planes[0].data[..4], &[255u8, 0, 0, 255]);
    }

    #[test]
    fn v5_embedded_icc_rgb565_path() {
        // V5 + ICC path now also supports 16-bit BI_BITFIELDS 5-6-5.
        // The V5 header carries `biCompression = BI_BITFIELDS` with the
        // canonical R=0xF800 / G=0x07E0 / B=0x001F masks in the
        // four-mask region (offsets 40..56 of the DIB header), and the
        // embedded ICC blob still lives at `bV5ProfileData`
        // immediately after the pixel array.
        let w = 4u32;
        let h = 2u32;
        // Build a 5-6-5 image: row 0 = red (0xF800), row 1 = green
        // (0x07E0). Stored host-LE in the source plane so the encoder's
        // `pack_rgb565` byte-copy keeps the wire format intact.
        let mut data = Vec::with_capacity((w * h * 2) as usize);
        for y in 0..h {
            let pixel = if y == 0 { 0xF800u16 } else { 0x07E0u16 };
            for _ in 0..w {
                data.extend_from_slice(&pixel.to_le_bytes());
            }
        }
        let src = BmpImage {
            width: w,
            height: h,
            pixel_format: BmpPixelFormat::Rgb565,
            planes: vec![BmpPlane {
                stride: w as usize * 2,
                data,
            }],
            palette: None,
            pts: None,
        };
        let icc = b"icc:rgb565-test".to_vec();
        let bytes =
            encode_bmp_with_icc_profile(&src, &icc, LCS_GM_GRAPHICS, BmpEncodeOptions::default())
                .expect("V5 + RGB565 ICC encode");

        // Header is V5 (124 bytes), bpp = 16, biCompression = BI_BITFIELDS.
        let hdr_size = u32::from_le_bytes([bytes[14], bytes[15], bytes[16], bytes[17]]);
        assert_eq!(hdr_size, BITMAPV5HEADER_SIZE);
        let bpp = u16::from_le_bytes([bytes[28], bytes[29]]);
        assert_eq!(bpp, 16);
        let compression = u32::from_le_bytes([bytes[30], bytes[31], bytes[32], bytes[33]]);
        assert_eq!(compression, BI_BITFIELDS);
        // Mask region at offsets 14 + 40..56.
        let r_mask = u32::from_le_bytes([bytes[54], bytes[55], bytes[56], bytes[57]]);
        let g_mask = u32::from_le_bytes([bytes[58], bytes[59], bytes[60], bytes[61]]);
        let b_mask = u32::from_le_bytes([bytes[62], bytes[63], bytes[64], bytes[65]]);
        let a_mask = u32::from_le_bytes([bytes[66], bytes[67], bytes[68], bytes[69]]);
        assert_eq!(
            (r_mask, g_mask, b_mask, a_mask),
            (0xF800, 0x07E0, 0x001F, 0)
        );
        // bV5CSType at 14 + 56 = 70..74 = PROFILE_EMBEDDED.
        let cs_type = u32::from_le_bytes([bytes[70], bytes[71], bytes[72], bytes[73]]);
        assert_eq!(cs_type, PROFILE_EMBEDDED);

        let (img, md) = decode_bmp_with_metadata(&bytes).expect("V5 + ICC must decode");
        assert_eq!((img.width, img.height), (w, h));
        assert_eq!(md.color_space, Some(BmpColorSpace::ProfileEmbedded));
        assert_eq!(
            md.rendering_intent,
            Some(BmpRenderingIntent::RelativeColorimetric)
        );
        assert_eq!(md.icc_profile.as_deref(), Some(icc.as_slice()));
        // Top row decodes back to red, second row to green. 5-6-5 →
        // 8-bit expansion: F800 → R=255 G=0 B=0, 07E0 → R=0 G=255 B=0.
        assert_eq!(&img.planes[0].data[..4], &[255, 0, 0, 255]);
        let stride = img.planes[0].stride;
        assert_eq!(&img.planes[0].data[stride..stride + 4], &[0, 255, 0, 255]);
    }

    #[test]
    fn v5_embedded_icc_rgb565_top_down_roundtrips() {
        // Same 16-bpp BI_BITFIELDS V5 + ICC path with `top_down = true`.
        // `biHeight` goes negative and the ICC blob still rounds-trips.
        let w = 2u32;
        let h = 2u32;
        // Single colour everywhere = 0xF81F (pure magenta in 5-6-5).
        let mut data = Vec::with_capacity((w * h * 2) as usize);
        for _ in 0..(w as usize * h as usize) {
            data.extend_from_slice(&0xF81Fu16.to_le_bytes());
        }
        let src = BmpImage {
            width: w,
            height: h,
            pixel_format: BmpPixelFormat::Rgb565,
            planes: vec![BmpPlane {
                stride: w as usize * 2,
                data,
            }],
            palette: None,
            pts: None,
        };
        let icc = vec![0u8, 1, 2, 3, 4, 5, 6, 7];
        let bytes = encode_bmp_with_icc_profile(
            &src,
            &icc,
            0,
            BmpEncodeOptions {
                top_down: true,
                ..Default::default()
            },
        )
        .expect("V5 + RGB565 top-down ICC encode");
        // biHeight at file offset 22 negative.
        let bi_height = i32::from_le_bytes([bytes[22], bytes[23], bytes[24], bytes[25]]);
        assert_eq!(bi_height, -(h as i32));
        let (img, md) = decode_bmp_with_metadata(&bytes).expect("decode V5 top-down rgb565");
        assert_eq!((img.width, img.height), (w, h));
        assert_eq!(md.icc_profile.as_deref(), Some(icc.as_slice()));
        assert_eq!(md.rendering_intent, Some(BmpRenderingIntent::Unspecified));
        // Magenta (0xF81F): R=31, G=0, B=31 → 8-bit (255, 0, 255).
        assert_eq!(&img.planes[0].data[..4], &[255, 0, 255, 255]);
    }

    #[test]
    fn v5_embedded_icc_rgb555_uses_plain_bi_rgb() {
        // PROFILE_EMBEDDED + Rgb555: the encoder writes a 124-byte V5
        // header with plain `BI_RGB` 5-5-5 (no bitfields mask block; the
        // four-mask region stays zero) and parks the ICC blob in the
        // trailing slot. The decoder reads 16-bit BI_RGB as 5-5-5.
        let w = 2u32;
        let h = 2u32;
        // 0x7C00 = pure red, 0x03E0 = pure green in 5-5-5.
        let mut data = Vec::with_capacity((w * h * 2) as usize);
        for y in 0..h {
            let pixel = if y == 0 { 0x7C00u16 } else { 0x03E0u16 };
            for _ in 0..w {
                data.extend_from_slice(&pixel.to_le_bytes());
            }
        }
        let src = BmpImage {
            width: w,
            height: h,
            pixel_format: BmpPixelFormat::Rgb555,
            planes: vec![BmpPlane {
                stride: w as usize * 2,
                data,
            }],
            palette: None,
            pts: None,
        };
        let icc = b"icc:rgb555-test".to_vec();
        let bytes =
            encode_bmp_with_icc_profile(&src, &icc, LCS_GM_IMAGES, BmpEncodeOptions::default())
                .expect("V5 + RGB555 ICC encode");
        let hdr_size = u32::from_le_bytes([bytes[14], bytes[15], bytes[16], bytes[17]]);
        assert_eq!(hdr_size, BITMAPV5HEADER_SIZE);
        let bpp = u16::from_le_bytes([bytes[28], bytes[29]]);
        assert_eq!(bpp, 16);
        let compression = u32::from_le_bytes([bytes[30], bytes[31], bytes[32], bytes[33]]);
        assert_eq!(compression, BI_RGB, "Rgb555 → plain BI_RGB, not bitfields");
        // Four-mask region (file offsets 54..70) is all zero for BI_RGB.
        let masks = u128::from_le_bytes(bytes[54..70].try_into().unwrap());
        assert_eq!(masks, 0);
        let cs_type = u32::from_le_bytes([bytes[70], bytes[71], bytes[72], bytes[73]]);
        assert_eq!(cs_type, PROFILE_EMBEDDED);

        let (img, md) = decode_bmp_with_metadata(&bytes).expect("V5 + ICC 5-5-5 must decode");
        assert_eq!((img.width, img.height), (w, h));
        assert_eq!(md.color_space, Some(BmpColorSpace::ProfileEmbedded));
        assert_eq!(md.rendering_intent, Some(BmpRenderingIntent::Perceptual));
        assert_eq!(md.icc_profile.as_deref(), Some(icc.as_slice()));
        // Top row red, second row green. 5-5-5 → 8-bit: 0x7C00 → (255,0,0),
        // 0x03E0 → (0,255,0).
        assert_eq!(&img.planes[0].data[..4], &[255, 0, 0, 255]);
        let stride = img.planes[0].stride;
        assert_eq!(&img.planes[0].data[stride..stride + 4], &[0, 255, 0, 255]);
    }

    #[test]
    fn v5_linked_icc_rgb555_path() {
        // PROFILE_LINKED + Rgb555: 124-byte V5 header, plain BI_RGB 5-5-5,
        // path blob in the trailing slot. The decoder surfaces
        // `ProfileLinked` and the profile pointer without auto-loading.
        let w = 2u32;
        let h = 1u32;
        let mut data = Vec::with_capacity((w * h * 2) as usize);
        for _ in 0..(w as usize * h as usize) {
            data.extend_from_slice(&0x001Fu16.to_le_bytes()); // pure blue 5-5-5
        }
        let src = BmpImage {
            width: w,
            height: h,
            pixel_format: BmpPixelFormat::Rgb555,
            planes: vec![BmpPlane {
                stride: w as usize * 2,
                data,
            }],
            palette: None,
            pts: None,
        };
        let path = b"/profiles/disp.icc";
        let bytes = encode_bmp_with_linked_icc_profile(&src, path, 0, BmpEncodeOptions::default())
            .expect("V5 + RGB555 linked ICC encode");
        let compression = u32::from_le_bytes([bytes[30], bytes[31], bytes[32], bytes[33]]);
        assert_eq!(compression, BI_RGB);
        let cs_type = u32::from_le_bytes([bytes[70], bytes[71], bytes[72], bytes[73]]);
        assert_eq!(cs_type, PROFILE_LINKED);
        let (img, md) = decode_bmp_with_metadata(&bytes).expect("decode V5 linked 5-5-5");
        assert_eq!((img.width, img.height), (w, h));
        assert_eq!(md.color_space, Some(BmpColorSpace::ProfileLinked));
        // 0x001F = pure blue: B=31 → 8-bit (0,0,255).
        assert_eq!(&img.planes[0].data[..4], &[0, 0, 255, 255]);
    }

    #[test]
    fn v5_linked_icc_rgb565_path() {
        // PROFILE_LINKED + Rgb565: encoder writes a 124-byte V5 header
        // with `biCompression = BI_BITFIELDS`, 5-6-5 mask quadruple in
        // the four-mask region, and a path blob in the trailing slot.
        // The decoder side surfaces `BmpColorSpace::ProfileLinked` plus
        // the `profile_data_offset` / `profile_size` pointer without
        // auto-loading the path.
        let w = 4u32;
        let h = 2u32;
        let mut data = Vec::with_capacity((w * h * 2) as usize);
        for _ in 0..(w as usize * h as usize) {
            data.extend_from_slice(&0x001Fu16.to_le_bytes()); // blue
        }
        let src = BmpImage {
            width: w,
            height: h,
            pixel_format: BmpPixelFormat::Rgb565,
            planes: vec![BmpPlane {
                stride: w as usize * 2,
                data,
            }],
            palette: None,
            pts: None,
        };
        let path = b"sRGB-v4-rgb565.icc\0".to_vec();
        let bytes = encode_bmp_with_linked_icc_profile(
            &src,
            &path,
            LCS_GM_IMAGES,
            BmpEncodeOptions::default(),
        )
        .expect("V5 + RGB565 linked encode");
        let cs_type = u32::from_le_bytes([bytes[70], bytes[71], bytes[72], bytes[73]]);
        assert_eq!(cs_type, PROFILE_LINKED);
        // bV5ProfileSize at file offset 14 + 116 = 130..134.
        let profile_size = u32::from_le_bytes([bytes[130], bytes[131], bytes[132], bytes[133]]);
        assert_eq!(profile_size as usize, path.len());
        let (img, md) = decode_bmp_with_metadata(&bytes).expect("decode linked rgb565");
        assert_eq!((img.width, img.height), (w, h));
        assert_eq!(md.color_space, Some(BmpColorSpace::ProfileLinked));
        assert_eq!(md.rendering_intent, Some(BmpRenderingIntent::Perceptual));
        // Decoder must NOT auto-load the linked path.
        assert!(md.icc_profile.is_none());
        // Pure-blue 5-6-5: B=31 → 255.
        assert_eq!(&img.planes[0].data[..4], &[0, 0, 255, 255]);
    }

    #[test]
    fn v5_embedded_icc_indexed_without_palette_errors() {
        // V5 + ICC indexed input requires a palette; without one the
        // encoder must return an error rather than emit a header with
        // no colour table.
        let w = 4u32;
        let h = 4u32;
        let (data, stride) = indexed_checker(w, h);
        let src = BmpImage {
            width: w,
            height: h,
            pixel_format: BmpPixelFormat::Indexed8,
            planes: vec![BmpPlane { stride, data }],
            palette: None,
            pts: None,
        };
        let err = encode_bmp_with_icc_profile(&src, b"icc", 0, BmpEncodeOptions::default())
            .expect_err("indexed-without-palette must be rejected");
        assert!(matches!(err, BmpError::InvalidData(_)), "got {err:?}");
    }

    #[test]
    fn v5_truncated_icc_does_not_panic() {
        // Construct a V5 header that lies: bV5ProfileSize claims 4096
        // bytes but the file is only 200 bytes long. The decoder must
        // surface declared `profile_size` / `profile_data_offset` while
        // leaving `icc_profile = None` (the slice falls past EOF).
        // Start from a valid V5 + ICC blob, then hack the size up.
        let (src, _, _) = rgba_checker(2, 2);
        let icc = vec![0u8; 8];
        let mut bytes =
            encode_bmp_with_icc_profile(&src, &icc, 0, BmpEncodeOptions::default()).unwrap();
        // bV5ProfileSize lives at file offset 14 + 116 = 130.
        let bogus = 4096u32;
        bytes[130..134].copy_from_slice(&bogus.to_le_bytes());
        let (_, md) = decode_bmp_with_metadata(&bytes).expect("decode must not panic");
        assert_eq!(md.color_space, Some(BmpColorSpace::ProfileEmbedded));
        assert_eq!(md.profile_size, Some(bogus));
        assert!(md.icc_profile.is_none(), "out-of-range ICC must be None");
    }

    #[test]
    fn v5_with_linked_icc_profile_path_surfaces() {
        // Encoder writes a V5 header with bV5CSType = PROFILE_LINKED and
        // a path-string blob in the trailing slot. The decoder
        // distinguishes PROFILE_LINKED from PROFILE_EMBEDDED, surfaces
        // the offset / size pointing at the blob, and (per spec) does
        // not auto-load the blob — `icc_profile` stays `None`.
        let (src, w, h) = rgba_checker(4, 4);
        let path = b"C:\\Windows\\System32\\spool\\drivers\\color\\sRGB.icm\0".to_vec();
        let bytes = encode_bmp_with_linked_icc_profile(
            &src,
            &path,
            LCS_GM_GRAPHICS,
            BmpEncodeOptions::default(),
        )
        .expect("V5 linked encode must succeed");
        // On-disk header is V5.
        let header_size = u32::from_le_bytes([bytes[14], bytes[15], bytes[16], bytes[17]]);
        assert_eq!(header_size, BITMAPV5HEADER_SIZE);
        // bV5CSType at file offset 14 + 56 = 70 must be PROFILE_LINKED.
        let cs_type = u32::from_le_bytes([bytes[70], bytes[71], bytes[72], bytes[73]]);
        assert_eq!(cs_type, PROFILE_LINKED);

        let (img, md) = decode_bmp_with_metadata(&bytes).expect("V5 linked must decode");
        assert_eq!((img.width, img.height), (w, h));
        assert_eq!(img.planes[0].data, src.planes[0].data);
        assert_eq!(md.color_space, Some(BmpColorSpace::ProfileLinked));
        assert_eq!(
            md.rendering_intent,
            Some(BmpRenderingIntent::RelativeColorimetric)
        );
        // Linked-path blob is surfaced via offset+size; the decoder does
        // not load it into `icc_profile` (that slot is reserved for the
        // PROFILE_EMBEDDED case).
        assert!(md.icc_profile.is_none());
        assert_eq!(md.profile_size, Some(path.len() as u32));
        // The path bytes themselves live where the offset points, after
        // the pixel array. Pull them back to confirm round-trip.
        let off = md.profile_data_offset.expect("offset present") as usize;
        let size = md.profile_size.unwrap() as usize;
        let dib_start = BITMAPFILEHEADER_SIZE as usize;
        let actual = &bytes[dib_start + off..dib_start + off + size];
        assert_eq!(actual, path.as_slice());
    }

    #[test]
    fn v5_linked_icc_top_down_roundtrips() {
        // Linked-profile path under top-down ordering: biHeight goes
        // negative and the path blob still round-trips bit-for-bit.
        let (src, _, h) = rgba_checker(2, 2);
        let path = b"file:///tmp/profile.icc\0".to_vec();
        let bytes = encode_bmp_with_linked_icc_profile(
            &src,
            &path,
            0,
            BmpEncodeOptions {
                top_down: true,
                ..Default::default()
            },
        )
        .expect("V5 linked top-down");
        // biHeight at offset 22 is negative.
        let bi_height = i32::from_le_bytes([bytes[22], bytes[23], bytes[24], bytes[25]]);
        assert_eq!(bi_height, -(h as i32));
        let (_, md) = decode_bmp_with_metadata(&bytes).expect("decode V5 linked top-down");
        assert_eq!(md.color_space, Some(BmpColorSpace::ProfileLinked));
        assert_eq!(md.rendering_intent, Some(BmpRenderingIntent::Unspecified));
        let off = md.profile_data_offset.unwrap() as usize;
        let size = md.profile_size.unwrap() as usize;
        let dib_start = BITMAPFILEHEADER_SIZE as usize;
        assert_eq!(&bytes[dib_start + off..dib_start + off + size], path);
    }

    #[test]
    fn v5_linked_icc_rgb24_path() {
        // 24-bit BGR + linked-profile path.
        let src = rgb24_checker(4, 2);
        let path = b"sRGB-v4.icc\0".to_vec();
        let bytes = encode_bmp_with_linked_icc_profile(
            &src,
            &path,
            LCS_GM_IMAGES,
            BmpEncodeOptions::default(),
        )
        .expect("V5 + RGB24 linked encode");
        let (img, md) = decode_bmp_with_metadata(&bytes).expect("decode");
        assert_eq!((img.width, img.height), (4, 2));
        assert_eq!(md.color_space, Some(BmpColorSpace::ProfileLinked));
        assert_eq!(md.rendering_intent, Some(BmpRenderingIntent::Perceptual));
        assert_eq!(md.profile_size, Some(path.len() as u32));
        // Pixel (0,0) is red per `rgb24_checker`.
        assert_eq!(&img.planes[0].data[..4], &[255u8, 0, 0, 255]);
    }

    #[test]
    fn v5_linked_icc_indexed_without_palette_errors() {
        // Same constraint as the embedded path: indexed input is now
        // accepted, but the caller still has to supply a palette. A
        // missing palette is a contract violation, not an unsupported
        // format.
        let w = 4u32;
        let h = 4u32;
        let (data, stride) = indexed_checker(w, h);
        let src = BmpImage {
            width: w,
            height: h,
            pixel_format: BmpPixelFormat::Indexed8,
            planes: vec![BmpPlane { stride, data }],
            palette: None,
            pts: None,
        };
        let err = encode_bmp_with_linked_icc_profile(&src, b"path", 0, BmpEncodeOptions::default())
            .expect_err("indexed-without-palette must be rejected");
        assert!(matches!(err, BmpError::InvalidData(_)), "got {err:?}");
    }

    #[test]
    fn v5_linked_icc_empty_path_still_valid() {
        // Empty path blob is structurally valid — the V5 header records
        // `profile_size = 0` and the decoder surfaces that cleanly. The
        // BMP itself is still a perfectly-decodable image.
        let (src, w, h) = rgba_checker(2, 2);
        let bytes = encode_bmp_with_linked_icc_profile(&src, &[], 0, BmpEncodeOptions::default())
            .expect("empty linked path must encode");
        let (img, md) = decode_bmp_with_metadata(&bytes).expect("must decode");
        assert_eq!((img.width, img.height), (w, h));
        assert_eq!(md.color_space, Some(BmpColorSpace::ProfileLinked));
        assert_eq!(md.profile_size, Some(0));
        assert!(md.icc_profile.is_none());
    }

    #[test]
    fn alpha_bitfields_v3_16bpp_5551() {
        // 5-5-5-1 packing as a 16-bpp BI_ALPHABITFIELDS variant. Payload
        // bit layout (LE u16):
        //   bit 15      = alpha
        //   bits 14..10 = R
        //   bits  9..5  = G
        //   bits  4..0  = B
        // Masks: A=0x8000, R=0x7C00, G=0x03E0, B=0x001F.
        // Payload 0xFC1F (= 0b1111_1100_0001_1111) → A=1, R=31, G=0, B=31.
        let mut input = raw_bmp(1, 1, 16, BI_ALPHABITFIELDS, 70, 0, &[]);
        input.extend_from_slice(&0x7C00u32.to_le_bytes()); // R
        input.extend_from_slice(&0x03E0u32.to_le_bytes()); // G
        input.extend_from_slice(&0x001Fu32.to_le_bytes()); // B
        input.extend_from_slice(&0x8000u32.to_le_bytes()); // A
                                                           // 16-bpp row stride is `ceil(1*16/32)*4 = 4`, so pad the 2-byte
                                                           // pixel word to 4 bytes.
        input.extend_from_slice(&[0x1F, 0xFC, 0x00, 0x00]);
        let img = decode_bmp(&input).expect("16-bpp BI_ALPHABITFIELDS decode");
        assert_eq!((img.width, img.height), (1, 1));
        // 5-bit → 8-bit expansion: 31 -> 0xFF, 0 -> 0x00. Alpha 1-bit:
        // 1 expanded to 0xFF.
        assert_eq!(&img.planes[0].data[..4], &[0xFF, 0x00, 0xFF, 0xFF]);
    }

    // -----------------------------------------------------------------------
    // V5 + ICC profile encode for indexed formats (round 231)
    //
    // Layout: [BITMAPFILEHEADER 14] [BITMAPV5HEADER 124] [colour table]
    //         [packed indexed pixels] [profile blob]. `bfOffBits` skips
    //         the V5 header + palette to point at the pixels; the
    //         decoder side already accepts arbitrary header sizes ahead
    //         of the palette so the roundtrip works without any decoder
    //         change.
    // -----------------------------------------------------------------------

    #[test]
    fn v5_embedded_icc_indexed8_roundtrips() {
        // Indexed8 + embedded ICC: V5 header carries `biClrUsed = 0`
        // (full 256-entry table by default) and the ICC blob rides at
        // `bV5ProfileData` after the pixel array. Decoding back to RGBA
        // resolves indices against the palette as usual; the metadata
        // path also surfaces the ICC bytes verbatim.
        let w = 8u32;
        let h = 4u32;
        let palette = four_color_palette();
        let (data, stride) = indexed_checker(w, h);
        let src = BmpImage {
            width: w,
            height: h,
            pixel_format: BmpPixelFormat::Indexed8,
            planes: vec![BmpPlane { stride, data }],
            palette: Some(palette),
            pts: None,
        };
        let icc = b"icc:indexed8-v5".to_vec();
        let bytes =
            encode_bmp_with_icc_profile(&src, &icc, LCS_GM_IMAGES, BmpEncodeOptions::default())
                .expect("V5 + ICC + Indexed8 encode");
        // Header is V5, bpp = 8, BI_RGB, biClrUsed = 0 (full table).
        let hdr_size = u32::from_le_bytes([bytes[14], bytes[15], bytes[16], bytes[17]]);
        assert_eq!(hdr_size, BITMAPV5HEADER_SIZE);
        let bpp = u16::from_le_bytes([bytes[28], bytes[29]]);
        assert_eq!(bpp, 8);
        let compression = u32::from_le_bytes([bytes[30], bytes[31], bytes[32], bytes[33]]);
        assert_eq!(compression, BI_RGB);
        let clr_used = u32::from_le_bytes([bytes[46], bytes[47], bytes[48], bytes[49]]);
        assert_eq!(clr_used, 0, "full table → biClrUsed = 0 sentinel");
        // bV5CSType at 14 + 56 = 70.
        let cs_type = u32::from_le_bytes([bytes[70], bytes[71], bytes[72], bytes[73]]);
        assert_eq!(cs_type, PROFILE_EMBEDDED);
        // bfOffBits at 10..14 skips header + 256×4 colour table.
        let off_bits = u32::from_le_bytes([bytes[10], bytes[11], bytes[12], bytes[13]]);
        assert_eq!(
            off_bits,
            BITMAPFILEHEADER_SIZE + BITMAPV5HEADER_SIZE + 256 * 4
        );

        let (img, md) = decode_bmp_with_metadata(&bytes).expect("decode V5 + ICC + Indexed8");
        assert_eq!((img.width, img.height), (w, h));
        assert_eq!(md.color_space, Some(BmpColorSpace::ProfileEmbedded));
        assert_eq!(md.rendering_intent, Some(BmpRenderingIntent::Perceptual));
        assert_eq!(md.icc_profile.as_deref(), Some(icc.as_slice()));
        // Pixel (0,0) is palette entry 0 (red).
        assert_eq!(&img.planes[0].data[..4], &[255, 0, 0, 255]);
        // Pixel (1,0) is palette entry 1 (green).
        assert_eq!(&img.planes[0].data[4..8], &[0, 255, 0, 255]);
    }

    #[test]
    fn v5_embedded_icc_indexed8_minimal_palette() {
        // `minimal_palette = true` shrinks the on-disk colour table to
        // the supplied entry count and records `biClrUsed = entries`.
        // The pixel array starts immediately after the trimmed table,
        // not 1024 bytes in.
        let w = 4u32;
        let h = 2u32;
        let palette = four_color_palette();
        let (data, stride) = indexed_checker(w, h);
        let src = BmpImage {
            width: w,
            height: h,
            pixel_format: BmpPixelFormat::Indexed8,
            planes: vec![BmpPlane { stride, data }],
            palette: Some(palette),
            pts: None,
        };
        let icc = b"icc:minimal-table".to_vec();
        let bytes = encode_bmp_with_icc_profile(
            &src,
            &icc,
            0,
            BmpEncodeOptions {
                minimal_palette: true,
                ..Default::default()
            },
        )
        .expect("V5 + ICC + minimal palette");
        let clr_used = u32::from_le_bytes([bytes[46], bytes[47], bytes[48], bytes[49]]);
        assert_eq!(clr_used, 4, "minimal_palette → 4 entries recorded");
        let off_bits = u32::from_le_bytes([bytes[10], bytes[11], bytes[12], bytes[13]]);
        assert_eq!(
            off_bits,
            BITMAPFILEHEADER_SIZE + BITMAPV5HEADER_SIZE + 4 * 4,
            "pixels start after the 4-entry trimmed table",
        );
        // Roundtrip still works.
        let (img, md) = decode_bmp_with_metadata(&bytes).expect("decode minimal-palette V5");
        assert_eq!((img.width, img.height), (w, h));
        assert_eq!(md.icc_profile.as_deref(), Some(icc.as_slice()));
        assert_eq!(&img.planes[0].data[..4], &[255, 0, 0, 255]);
    }

    #[test]
    fn v5_embedded_icc_indexed8_top_down_roundtrips() {
        // `top_down = true` writes `biHeight` negative and orders rows
        // top-first. The ICC blob still rides at `bV5ProfileData` after
        // the pixel array.
        let w = 4u32;
        let h = 4u32;
        let palette = four_color_palette();
        let (data, stride) = indexed_checker(w, h);
        let src = BmpImage {
            width: w,
            height: h,
            pixel_format: BmpPixelFormat::Indexed8,
            planes: vec![BmpPlane { stride, data }],
            palette: Some(palette),
            pts: None,
        };
        let icc = vec![0xDEu8, 0xAD, 0xBE, 0xEF];
        let bytes = encode_bmp_with_icc_profile(
            &src,
            &icc,
            LCS_GM_GRAPHICS,
            BmpEncodeOptions {
                top_down: true,
                ..Default::default()
            },
        )
        .expect("V5 + ICC + Indexed8 + top_down");
        let bi_height = i32::from_le_bytes([bytes[22], bytes[23], bytes[24], bytes[25]]);
        assert_eq!(bi_height, -(h as i32));
        let (img, md) = decode_bmp_with_metadata(&bytes).expect("decode top-down V5 indexed");
        assert_eq!((img.width, img.height), (w, h));
        assert_eq!(md.icc_profile.as_deref(), Some(icc.as_slice()));
        assert_eq!(
            md.rendering_intent,
            Some(BmpRenderingIntent::RelativeColorimetric)
        );
        // Row 0 in the picture is row 0 in the source (top-down).
        assert_eq!(&img.planes[0].data[..4], &[255, 0, 0, 255]);
    }

    #[test]
    fn v5_embedded_icc_indexed4_path() {
        // 4-bpp indexed: hi-nibble = left pixel. V5 header records
        // `bpp = 4`, the 16-entry table sits between the header and the
        // pixel array, and the ICC blob lives at `bV5ProfileData`.
        let w = 8u32;
        let h = 4u32;
        let palette = four_color_palette();
        let (data, stride) = indexed_checker(w, h);
        let src = BmpImage {
            width: w,
            height: h,
            pixel_format: BmpPixelFormat::Indexed4,
            planes: vec![BmpPlane { stride, data }],
            palette: Some(palette),
            pts: None,
        };
        let icc = b"icc:idx4".to_vec();
        let bytes =
            encode_bmp_with_icc_profile(&src, &icc, LCS_GM_BUSINESS, BmpEncodeOptions::default())
                .expect("V5 + ICC + Indexed4");
        let bpp = u16::from_le_bytes([bytes[28], bytes[29]]);
        assert_eq!(bpp, 4);
        // 16-entry default table → off_bits = 14 + 124 + 64.
        let off_bits = u32::from_le_bytes([bytes[10], bytes[11], bytes[12], bytes[13]]);
        assert_eq!(
            off_bits,
            BITMAPFILEHEADER_SIZE + BITMAPV5HEADER_SIZE + 16 * 4
        );
        let (img, md) = decode_bmp_with_metadata(&bytes).expect("decode V5 + Indexed4");
        assert_eq!((img.width, img.height), (w, h));
        assert_eq!(md.color_space, Some(BmpColorSpace::ProfileEmbedded));
        assert_eq!(md.rendering_intent, Some(BmpRenderingIntent::Saturation));
        assert_eq!(md.icc_profile.as_deref(), Some(icc.as_slice()));
        // Pixel (0,0) is palette entry 0 (red).
        assert_eq!(&img.planes[0].data[..4], &[255, 0, 0, 255]);
    }

    #[test]
    fn v5_embedded_icc_indexed1_path() {
        // 1-bpp indexed (monochrome). Default table is 2 × 4 = 8 bytes;
        // pixels start at `14 + 124 + 8 = 146`.
        let w = 8u32;
        let h = 2u32;
        let palette = BmpPalette {
            entries: vec![[0, 0, 0], [255, 255, 255]],
        };
        // 1-bit data: alternate 0/1 horizontally.
        let stride = w as usize;
        let data: Vec<u8> = (0..(w * h)).map(|i| (i & 1) as u8).collect();
        let src = BmpImage {
            width: w,
            height: h,
            pixel_format: BmpPixelFormat::Indexed1,
            planes: vec![BmpPlane { stride, data }],
            palette: Some(palette),
            pts: None,
        };
        let icc = b"icc:mono".to_vec();
        let bytes = encode_bmp_with_icc_profile(&src, &icc, 0, BmpEncodeOptions::default())
            .expect("V5 + ICC + Indexed1");
        let bpp = u16::from_le_bytes([bytes[28], bytes[29]]);
        assert_eq!(bpp, 1);
        let off_bits = u32::from_le_bytes([bytes[10], bytes[11], bytes[12], bytes[13]]);
        assert_eq!(
            off_bits,
            BITMAPFILEHEADER_SIZE + BITMAPV5HEADER_SIZE + 2 * 4
        );
        let (img, md) = decode_bmp_with_metadata(&bytes).expect("decode V5 + Indexed1");
        assert_eq!((img.width, img.height), (w, h));
        assert_eq!(md.icc_profile.as_deref(), Some(icc.as_slice()));
        // First pixel is index 0 (black).
        assert_eq!(&img.planes[0].data[..4], &[0, 0, 0, 255]);
        // Second pixel is index 1 (white).
        assert_eq!(&img.planes[0].data[4..8], &[255, 255, 255, 255]);
    }

    #[test]
    fn v5_linked_icc_indexed8_path() {
        // PROFILE_LINKED + Indexed8: same layout as the embedded variant
        // but `bV5CSType = PROFILE_LINKED` and the trailing blob is a
        // path-string rather than an ICC profile. The decoder surfaces
        // the path offset / size and leaves `icc_profile = None`.
        let w = 4u32;
        let h = 4u32;
        let palette = four_color_palette();
        let (data, stride) = indexed_checker(w, h);
        let src = BmpImage {
            width: w,
            height: h,
            pixel_format: BmpPixelFormat::Indexed8,
            planes: vec![BmpPlane { stride, data }],
            palette: Some(palette),
            pts: None,
        };
        let path = b"C:\\ICC\\Indexed8.icm\0".to_vec();
        let bytes = encode_bmp_with_linked_icc_profile(
            &src,
            &path,
            LCS_GM_IMAGES,
            BmpEncodeOptions::default(),
        )
        .expect("V5 + PROFILE_LINKED + Indexed8");
        let cs_type = u32::from_le_bytes([bytes[70], bytes[71], bytes[72], bytes[73]]);
        assert_eq!(cs_type, PROFILE_LINKED);
        let (img, md) = decode_bmp_with_metadata(&bytes).expect("decode linked V5 indexed");
        assert_eq!((img.width, img.height), (w, h));
        assert_eq!(md.color_space, Some(BmpColorSpace::ProfileLinked));
        assert_eq!(md.rendering_intent, Some(BmpRenderingIntent::Perceptual));
        assert!(md.icc_profile.is_none());
        assert_eq!(md.profile_size, Some(path.len() as u32));
        // Pull the path blob back at the surfaced offset.
        let off = md.profile_data_offset.unwrap() as usize;
        let size = md.profile_size.unwrap() as usize;
        let dib_start = BITMAPFILEHEADER_SIZE as usize;
        assert_eq!(&bytes[dib_start + off..dib_start + off + size], path);
        // Pixels survive too.
        assert_eq!(&img.planes[0].data[..4], &[255, 0, 0, 255]);
    }

    #[test]
    fn v5_linked_icc_indexed4_minimal_palette_path() {
        // Indexed4 + minimal_palette + linked profile in one go. The
        // 4-entry table shrinks the colour-table region to 16 bytes;
        // pixel-offset, profile-offset, and decoded pixels all reflect
        // the trimmed layout.
        let w = 8u32;
        let h = 2u32;
        let palette = four_color_palette();
        let (data, stride) = indexed_checker(w, h);
        let src = BmpImage {
            width: w,
            height: h,
            pixel_format: BmpPixelFormat::Indexed4,
            planes: vec![BmpPlane { stride, data }],
            palette: Some(palette),
            pts: None,
        };
        let path = b"linked-trimmed.icm".to_vec();
        let bytes = encode_bmp_with_linked_icc_profile(
            &src,
            &path,
            0,
            BmpEncodeOptions {
                minimal_palette: true,
                ..Default::default()
            },
        )
        .expect("V5 + linked + Indexed4 + minimal");
        let clr_used = u32::from_le_bytes([bytes[46], bytes[47], bytes[48], bytes[49]]);
        assert_eq!(clr_used, 4);
        let off_bits = u32::from_le_bytes([bytes[10], bytes[11], bytes[12], bytes[13]]);
        assert_eq!(
            off_bits,
            BITMAPFILEHEADER_SIZE + BITMAPV5HEADER_SIZE + 4 * 4
        );
        let (img, md) = decode_bmp_with_metadata(&bytes).expect("decode minimal linked V5 idx4");
        assert_eq!((img.width, img.height), (w, h));
        assert_eq!(md.color_space, Some(BmpColorSpace::ProfileLinked));
        assert_eq!(md.profile_size, Some(path.len() as u32));
        let off = md.profile_data_offset.unwrap() as usize;
        let dib_start = BITMAPFILEHEADER_SIZE as usize;
        assert_eq!(
            &bytes[dib_start + off..dib_start + off + path.len()],
            path.as_slice(),
        );
        assert_eq!(&img.planes[0].data[..4], &[255, 0, 0, 255]);
    }

    #[test]
    fn v5_icc_profile_ref_discriminates_embedded_linked_and_absent() {
        // Typed accessor `BmpMetadata::icc_profile_ref` returns a
        // single `BmpIccProfileRef` view of the V5 trailing-slot
        // bytes. Cover the three live outcomes the decoder produces:
        //   1. PROFILE_EMBEDDED + readable blob   → Embedded(&[u8])
        //   2. PROFILE_LINKED   + readable blob   → Linked(&[u8])
        //   3. No PROFILE_* (V3 / sRGB / V4 / V5  → None
        //      LCS_* variants).
        // Plus the lying-offset path: PROFILE_EMBEDDED with a
        // bV5ProfileSize that walks past EOF must surface as
        // Declared{cs_type, profile_data_offset, profile_size} — the
        // header's claims are kept so a caller can investigate
        // without the bytes themselves being available.

        // (1) Embedded.
        let (src, _, _) = rgba_checker(4, 4);
        let icc = b"ICC-PROFILE-EMBEDDED-TEST".to_vec();
        let bytes_embedded =
            encode_bmp_with_icc_profile(&src, &icc, LCS_GM_IMAGES, BmpEncodeOptions::default())
                .expect("V5 + embedded encode");
        let (_, md_embedded) = decode_bmp_with_metadata(&bytes_embedded).expect("decode embedded");
        match md_embedded.icc_profile_ref() {
            BmpIccProfileRef::Embedded(bytes) => assert_eq!(bytes, icc.as_slice()),
            other => panic!("expected Embedded(...), got {other:?}"),
        }

        // (2) Linked.
        let path = b"/usr/share/color/icc/sRGB.icc\0".to_vec();
        let bytes_linked = encode_bmp_with_linked_icc_profile(
            &src,
            &path,
            LCS_GM_GRAPHICS,
            BmpEncodeOptions::default(),
        )
        .expect("V5 + linked encode");
        let (_, md_linked) = decode_bmp_with_metadata(&bytes_linked).expect("decode linked");
        match md_linked.icc_profile_ref() {
            BmpIccProfileRef::Linked(bytes) => assert_eq!(bytes, path.as_slice()),
            other => panic!("expected Linked(...), got {other:?}"),
        }
        // Linked-path bytes are also surfaced via the named field, in
        // parallel to `icc_profile` for the embedded variant.
        assert_eq!(
            md_linked.linked_profile_path.as_deref(),
            Some(path.as_slice())
        );
        // Embedded slot remains empty for the linked variant — the
        // discriminator on `cs_type` is what decides which slot the
        // trailing bytes go into.
        assert!(md_linked.icc_profile.is_none());

        // (3) Absent — a plain V3 BMP has no V5 colour-management
        // tail at all, so the accessor reports None.
        let (bytes_v3, _) = encode_bmp(&src).expect("V3 encode");
        let (_, md_v3) = decode_bmp_with_metadata(&bytes_v3).expect("decode V3");
        assert_eq!(md_v3.color_space, None);
        assert_eq!(md_v3.icc_profile_ref(), BmpIccProfileRef::None);

        // (4) Lying offset / size: take the embedded-blob output and
        // overwrite bV5ProfileSize with a wildly out-of-range value.
        // bV5ProfileSize lives at file offset 14 + 116 = 130 (the
        // existing tests already document this offset). The accessor
        // must report Declared{...} with the bogus size echoed back.
        let mut bytes_bad = bytes_embedded.clone();
        let bogus = 0xFFFF_0000u32;
        bytes_bad[130..134].copy_from_slice(&bogus.to_le_bytes());
        let (_, md_bad) = decode_bmp_with_metadata(&bytes_bad).expect("decode lying-size");
        match md_bad.icc_profile_ref() {
            BmpIccProfileRef::Declared {
                cs_type,
                profile_size,
                ..
            } => {
                assert_eq!(cs_type, PROFILE_EMBEDDED);
                assert_eq!(profile_size, bogus);
            }
            other => panic!("expected Declared{{..}}, got {other:?}"),
        }
        // The named field also stays None for the bytes-unavailable case.
        assert!(md_bad.icc_profile.is_none());
    }

    // -----------------------------------------------------------------------
    // V3+ device-resolution + palette-count metadata
    // (`biXPelsPerMeter`, `biYPelsPerMeter`, `biClrUsed`, `biClrImportant`).
    // -----------------------------------------------------------------------
    //
    // V3 `BITMAPINFOHEADER` (40 bytes, Windows 3.0) was the first BMP
    // header generation to carry the four fields above; V4 and V5 inherit
    // them at the same byte offsets. The OS/2 `BITMAPCOREHEADER` (12 B)
    // pre-dates every one of them. `BmpMetadata` surfaces all four
    // verbatim and offers a `dpi_*` convenience that converts the
    // pels-per-metre field to dots-per-inch.

    /// Hand-assemble a 1×1 V3 BMP whose `biXPelsPerMeter` /
    /// `biYPelsPerMeter` / `biClrUsed` / `biClrImportant` fields take
    /// caller-supplied values. Used to verify the V3 metadata path picks
    /// the bytes up at the documented offsets.
    fn build_v3_with_resolution(
        x_pels_per_meter: i32,
        y_pels_per_meter: i32,
        clr_used: u32,
        clr_important: u32,
    ) -> Vec<u8> {
        let pixel_bytes = 4u32;
        let file_size = BITMAPFILEHEADER_SIZE + BITMAPINFOHEADER_SIZE + pixel_bytes;
        let pixel_offset = BITMAPFILEHEADER_SIZE + BITMAPINFOHEADER_SIZE;
        let mut out = Vec::with_capacity(file_size as usize);
        out.extend_from_slice(b"BM");
        out.extend_from_slice(&file_size.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&pixel_offset.to_le_bytes());
        // BITMAPINFOHEADER body (40 B):
        out.extend_from_slice(&BITMAPINFOHEADER_SIZE.to_le_bytes());
        out.extend_from_slice(&1i32.to_le_bytes()); // width
        out.extend_from_slice(&1i32.to_le_bytes()); // height (positive: bottom-up)
        out.extend_from_slice(&1u16.to_le_bytes()); // planes
        out.extend_from_slice(&32u16.to_le_bytes()); // bpp
        out.extend_from_slice(&BI_RGB.to_le_bytes()); // compression
        out.extend_from_slice(&pixel_bytes.to_le_bytes()); // image size
        out.extend_from_slice(&x_pels_per_meter.to_le_bytes());
        out.extend_from_slice(&y_pels_per_meter.to_le_bytes());
        out.extend_from_slice(&clr_used.to_le_bytes());
        out.extend_from_slice(&clr_important.to_le_bytes());
        // One BGRA pixel.
        out.extend_from_slice(&0xFF112233u32.to_le_bytes());
        out
    }

    #[test]
    fn v3_metadata_surfaces_resolution_and_palette_fields() {
        // 2835 pels/m ≈ 72 DPI (the encoder's own default); 5669 pels/m
        // ≈ 144 DPI exactly. clr_used/clr_important pass through verbatim.
        let bytes = build_v3_with_resolution(2835, 5669, 16, 8);
        let (_, md) = decode_bmp_with_metadata(&bytes).expect("V3 must decode");
        assert_eq!(md.header_size, BITMAPINFOHEADER_SIZE);
        assert_eq!(md.pixels_per_meter_x, Some(2835));
        assert_eq!(md.pixels_per_meter_y, Some(5669));
        assert_eq!(md.colors_used, Some(16));
        assert_eq!(md.colors_important, Some(8));
        // Conversion to DPI (rounded to nearest integer; 0.0254 m / in).
        assert_eq!(md.dpi_x(), Some(72));
        assert_eq!(md.dpi_y(), Some(144));
    }

    #[test]
    fn v3_metadata_zero_resolution_returns_none_dpi() {
        // The "unknown / unspecified" sentinel (`0` pels/m) on the V3+
        // resolution fields must surface as None from `dpi_*` so callers
        // don't see a nonsensical 0 DPI. The raw field stays as `Some(0)`
        // — the sentinel is distinguishable from "header didn't carry
        // the field" (`None` for OS/2 `BITMAPCOREHEADER`).
        let bytes = build_v3_with_resolution(0, 0, 0, 0);
        let (_, md) = decode_bmp_with_metadata(&bytes).expect("V3 must decode");
        assert_eq!(md.pixels_per_meter_x, Some(0));
        assert_eq!(md.pixels_per_meter_y, Some(0));
        assert_eq!(md.colors_used, Some(0));
        assert_eq!(md.colors_important, Some(0));
        assert_eq!(md.dpi_x(), None);
        assert_eq!(md.dpi_y(), None);
    }

    #[test]
    fn v3_metadata_rejects_negative_pels_per_meter() {
        // Negative pixels-per-metre is semantically meaningless (the
        // field is documented as a target-device resolution, not a
        // signed offset). The raw value is still passed through so
        // callers can investigate, but `dpi_*` returns None so a
        // misencoded file doesn't generate a "DPI = some-large-negative"
        // report downstream.
        let bytes = build_v3_with_resolution(-2835, -5669, 0, 0);
        let (_, md) = decode_bmp_with_metadata(&bytes).expect("V3 must decode");
        assert_eq!(md.pixels_per_meter_x, Some(-2835));
        assert_eq!(md.pixels_per_meter_y, Some(-5669));
        assert_eq!(md.dpi_x(), None);
        assert_eq!(md.dpi_y(), None);
    }

    #[test]
    fn os2_bitmapcoreheader_metadata_has_no_resolution_fields() {
        // The OS/2 12-byte BITMAPCOREHEADER pre-dates the
        // biXPelsPerMeter / biYPelsPerMeter / biClrUsed / biClrImportant
        // fields entirely. The metadata path must surface all four as
        // None — distinguishable from V3+ headers that set the fields
        // to zero. Build a minimal 1×1 24-bpp BITMAPCOREHEADER fixture
        // by hand.
        let pixel_offset = BITMAPFILEHEADER_SIZE + BITMAPCOREHEADER_SIZE;
        // Row stride for a 1px-wide 24-bpp row, padded to 4 bytes.
        let row = 4u32;
        let file_size = pixel_offset + row;
        let mut out = Vec::with_capacity(file_size as usize);
        out.extend_from_slice(b"BM");
        out.extend_from_slice(&file_size.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&pixel_offset.to_le_bytes());
        // BITMAPCOREHEADER body (12 B): size + u16 width + u16 height +
        // u16 planes + u16 bcBitCount.
        out.extend_from_slice(&BITMAPCOREHEADER_SIZE.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&24u16.to_le_bytes());
        // One BGR padded pixel.
        out.extend_from_slice(&[0x11, 0x22, 0x33, 0x00]);

        let (_, md) = decode_bmp_with_metadata(&out).expect("OS/2 V1 must decode");
        assert_eq!(md.header_size, BITMAPCOREHEADER_SIZE);
        assert!(md.pixels_per_meter_x.is_none());
        assert!(md.pixels_per_meter_y.is_none());
        assert!(md.colors_used.is_none());
        assert!(md.colors_important.is_none());
        assert!(md.dpi_x().is_none());
        assert!(md.dpi_y().is_none());
    }

    #[test]
    fn v5_metadata_inherits_resolution_fields() {
        // V4 / V5 headers carry biXPelsPerMeter etc. at the same byte
        // offsets as V3, so a V5 fixture must also surface them.
        // `build_v5_srgb_32bpp` writes zeros to those slots, so use a
        // 96-DPI value (3780 pels/m) injected via in-place byte edit.
        let mut bytes = build_v5_srgb_32bpp(0x11223344, 0);
        // biXPelsPerMeter sits at file offset 14 + 24 = 38; biYPelsPerMeter
        // at 14 + 28 = 42; biClrUsed at 14 + 32 = 46; biClrImportant at
        // 14 + 36 = 50 (matching the V3 layout at the same offsets in the
        // DIB body).
        let x = 3780i32; // ≈ 96 DPI
        let y = 3780i32;
        bytes[38..42].copy_from_slice(&x.to_le_bytes());
        bytes[42..46].copy_from_slice(&y.to_le_bytes());
        bytes[46..50].copy_from_slice(&5u32.to_le_bytes());
        bytes[50..54].copy_from_slice(&3u32.to_le_bytes());
        let (_, md) = decode_bmp_with_metadata(&bytes).expect("V5 must decode");
        assert_eq!(md.header_size, BITMAPV5HEADER_SIZE);
        assert_eq!(md.pixels_per_meter_x, Some(x));
        assert_eq!(md.pixels_per_meter_y, Some(y));
        assert_eq!(md.colors_used, Some(5));
        assert_eq!(md.colors_important, Some(3));
        assert_eq!(md.dpi_x(), Some(96));
        assert_eq!(md.dpi_y(), Some(96));
    }

    // -----------------------------------------------------------------------
    // V4 calibrated-RGB encode (bV4CSType = LCS_CALIBRATED_RGB)
    // -----------------------------------------------------------------------

    // sRGB-ish CIE endpoints (FXPT2DOT30) and gamma triple, picked as
    // distinctive non-zero values so the roundtrip is meaningful.
    const CAL_ENDPOINTS: [i32; 9] = [
        0x0002_8511,
        0x0001_5476,
        0x0000_0000, // red x/y/z
        0x0001_2A68,
        0x0002_6666,
        0x0000_570A, // green x/y/z
        0x0000_6332,
        0x0000_6666,
        0x0001_E51E, // blue x/y/z
    ];
    const CAL_GAMMA: [u32; 3] = [0x0002_3333, 0x0002_3333, 0x0002_3333]; // ~2.2 in 16.16

    #[test]
    fn v4_calibrated_rgba_roundtrips() {
        let (src, w, h) = rgba_checker(4, 4);
        let bytes = encode_bmp_with_calibrated_rgb(
            &src,
            CAL_ENDPOINTS,
            CAL_GAMMA,
            BmpEncodeOptions::default(),
        )
        .expect("V4 calibrated encode must succeed");
        // On-disk header is V4 (108 B) with bV4CSType = LCS_CALIBRATED_RGB.
        let header_size = u32::from_le_bytes([bytes[14], bytes[15], bytes[16], bytes[17]]);
        assert_eq!(header_size, BITMAPV4HEADER_SIZE);
        let cs_type = u32::from_le_bytes([
            bytes[14 + 56],
            bytes[14 + 57],
            bytes[14 + 58],
            bytes[14 + 59],
        ]);
        assert_eq!(cs_type, LCS_CALIBRATED_RGB);

        let (img, md) = decode_bmp_with_metadata(&bytes).expect("V4 calibrated must decode");
        assert_eq!((img.width, img.height), (w, h));
        assert_eq!(img.planes[0].data, src.planes[0].data);
        assert_eq!(md.header_size, BITMAPV4HEADER_SIZE);
        assert_eq!(md.color_space, Some(BmpColorSpace::Calibrated));
        assert_eq!(md.endpoints, Some(CAL_ENDPOINTS));
        assert_eq!(md.gamma_rgb, Some(CAL_GAMMA));
        // V4 carries no rendering intent / ICC.
        assert!(md.rendering_intent.is_none());
        assert!(md.icc_profile.is_none());
    }

    #[test]
    fn v4_calibrated_rgb24_path() {
        let src = rgb24_checker(4, 2);
        let bytes = encode_bmp_with_calibrated_rgb(
            &src,
            CAL_ENDPOINTS,
            CAL_GAMMA,
            BmpEncodeOptions::default(),
        )
        .expect("V4 calibrated RGB24 encode");
        let (img, md) = decode_bmp_with_metadata(&bytes).expect("decode");
        assert_eq!((img.width, img.height), (4, 2));
        assert_eq!(md.color_space, Some(BmpColorSpace::Calibrated));
        assert_eq!(md.endpoints, Some(CAL_ENDPOINTS));
        assert_eq!(md.gamma_rgb, Some(CAL_GAMMA));
        // Pixel (0,0) is red per `rgb24_checker`.
        assert_eq!(&img.planes[0].data[..4], &[255u8, 0, 0, 255]);
    }

    #[test]
    fn v4_calibrated_rgb565_carries_masks_in_header() {
        // 16-bit input → BI_BITFIELDS with the canonical 5-6-5 masks
        // living in the V4 four-mask region; calibrated endpoints + gamma
        // still survive the roundtrip.
        let src = BmpImage {
            width: 1,
            height: 1,
            pixel_format: BmpPixelFormat::Rgb565,
            planes: vec![BmpPlane {
                stride: 2,
                data: vec![0xE0, 0x07], // green
            }],
            palette: None,
            pts: None,
        };
        let bytes = encode_bmp_with_calibrated_rgb(
            &src,
            CAL_ENDPOINTS,
            CAL_GAMMA,
            BmpEncodeOptions::default(),
        )
        .expect("V4 calibrated 5-6-5 encode");
        // biCompression at DIB offset 16 (file offset 30) must be BI_BITFIELDS.
        let compression = u32::from_le_bytes([bytes[30], bytes[31], bytes[32], bytes[33]]);
        assert_eq!(compression, BI_BITFIELDS);
        // R mask at DIB offset 40 (file offset 54) must be 0xF800.
        let r_mask = u32::from_le_bytes([bytes[54], bytes[55], bytes[56], bytes[57]]);
        assert_eq!(r_mask, 0xF800);
        let (_, md) = decode_bmp_with_metadata(&bytes).expect("decode 5-6-5");
        assert_eq!(md.color_space, Some(BmpColorSpace::Calibrated));
        assert_eq!(md.endpoints, Some(CAL_ENDPOINTS));
        assert_eq!(md.gamma_rgb, Some(CAL_GAMMA));
    }

    #[test]
    fn v4_calibrated_rgb555_uses_plain_bi_rgb() {
        // 16-bit `Rgb555` input → plain `BI_RGB` 5-5-5 (high bit reserved,
        // NO bitfields mask block); the V4 four-mask region stays zero.
        // Calibrated endpoints + gamma still round-trip. Pixel 0xF800 is
        // pure red in 5-5-5 (R in bits 14..10): R=31 → 8-bit 255.
        let src = BmpImage {
            width: 1,
            height: 1,
            pixel_format: BmpPixelFormat::Rgb555,
            planes: vec![BmpPlane {
                stride: 2,
                data: vec![0x00, 0x7C], // 0x7C00 = pure red 5-5-5
            }],
            palette: None,
            pts: None,
        };
        let bytes = encode_bmp_with_calibrated_rgb(
            &src,
            CAL_ENDPOINTS,
            CAL_GAMMA,
            BmpEncodeOptions::default(),
        )
        .expect("V4 calibrated 5-5-5 encode");
        // Header is V4; biCompression is plain BI_RGB (no bitfields).
        let header_size = u32::from_le_bytes([bytes[14], bytes[15], bytes[16], bytes[17]]);
        assert_eq!(header_size, BITMAPV4HEADER_SIZE);
        let bpp = u16::from_le_bytes([bytes[28], bytes[29]]);
        assert_eq!(bpp, 16);
        let compression = u32::from_le_bytes([bytes[30], bytes[31], bytes[32], bytes[33]]);
        assert_eq!(compression, BI_RGB);
        // The four-mask region (file offsets 54..70) is all zero for BI_RGB.
        let masks = u128::from_le_bytes(bytes[54..70].try_into().unwrap());
        assert_eq!(masks, 0, "BI_RGB 5-5-5 writes no mask quadruple");
        let (img, md) = decode_bmp_with_metadata(&bytes).expect("decode 5-5-5");
        assert_eq!((img.width, img.height), (1, 1));
        assert_eq!(&img.planes[0].data[..4], &[255, 0, 0, 255]);
        assert_eq!(md.color_space, Some(BmpColorSpace::Calibrated));
        assert_eq!(md.endpoints, Some(CAL_ENDPOINTS));
        assert_eq!(md.gamma_rgb, Some(CAL_GAMMA));
    }

    #[test]
    fn v4_calibrated_top_down_negative_height() {
        let (src, _, h) = rgba_checker(2, 2);
        let bytes = encode_bmp_with_calibrated_rgb(
            &src,
            CAL_ENDPOINTS,
            CAL_GAMMA,
            BmpEncodeOptions {
                top_down: true,
                ..Default::default()
            },
        )
        .expect("V4 calibrated top-down encode");
        // biHeight at file offset 22 should be negative.
        let bi_height = i32::from_le_bytes([bytes[22], bytes[23], bytes[24], bytes[25]]);
        assert_eq!(bi_height, -(h as i32));
        let (img, md) = decode_bmp_with_metadata(&bytes).expect("decode top-down");
        // Decode is always top-down Rgba, so the plane matches the source.
        assert_eq!(img.planes[0].data, src.planes[0].data);
        assert_eq!(md.color_space, Some(BmpColorSpace::Calibrated));
    }

    #[test]
    fn v4_calibrated_indexed8_roundtrips() {
        let (idx, stride) = indexed_checker(4, 4);
        let src = BmpImage {
            width: 4,
            height: 4,
            pixel_format: BmpPixelFormat::Indexed8,
            planes: vec![BmpPlane { stride, data: idx }],
            palette: Some(four_color_palette()),
            pts: None,
        };
        let bytes = encode_bmp_with_calibrated_rgb(
            &src,
            CAL_ENDPOINTS,
            CAL_GAMMA,
            BmpEncodeOptions::default(),
        )
        .expect("V4 calibrated indexed8 encode");
        // Header is V4; biCompression is BI_RGB (indexed never RLE here).
        let header_size = u32::from_le_bytes([bytes[14], bytes[15], bytes[16], bytes[17]]);
        assert_eq!(header_size, BITMAPV4HEADER_SIZE);
        let compression = u32::from_le_bytes([bytes[30], bytes[31], bytes[32], bytes[33]]);
        assert_eq!(compression, BI_RGB);
        // bfOffBits = 14 + 108 + 256*4 (full table by default).
        let off_bits = u32::from_le_bytes([bytes[10], bytes[11], bytes[12], bytes[13]]);
        assert_eq!(off_bits, 14 + 108 + 256 * 4);
        let (img, md) = decode_bmp_with_metadata(&bytes).expect("decode indexed8");
        assert_eq!((img.width, img.height), (4, 4));
        // index 0 → red, per four_color_palette.
        assert_eq!(&img.planes[0].data[..4], &[255u8, 0, 0, 255]);
        assert_eq!(md.color_space, Some(BmpColorSpace::Calibrated));
        assert_eq!(md.endpoints, Some(CAL_ENDPOINTS));
    }

    #[test]
    fn v4_calibrated_indexed8_minimal_palette() {
        let (idx, stride) = indexed_checker(4, 4);
        let src = BmpImage {
            width: 4,
            height: 4,
            pixel_format: BmpPixelFormat::Indexed8,
            planes: vec![BmpPlane { stride, data: idx }],
            palette: Some(four_color_palette()),
            pts: None,
        };
        let bytes = encode_bmp_with_calibrated_rgb(
            &src,
            CAL_ENDPOINTS,
            CAL_GAMMA,
            BmpEncodeOptions {
                minimal_palette: true,
                ..Default::default()
            },
        )
        .expect("V4 calibrated indexed8 minimal-palette encode");
        // Only the 4 supplied entries are written; bfOffBits trims.
        let off_bits = u32::from_le_bytes([bytes[10], bytes[11], bytes[12], bytes[13]]);
        assert_eq!(off_bits, 14 + 108 + 4 * 4);
        // biClrUsed at DIB offset 32 (file offset 46) records the count.
        let clr_used = u32::from_le_bytes([bytes[46], bytes[47], bytes[48], bytes[49]]);
        assert_eq!(clr_used, 4);
        let (img, _) = decode_bmp_with_metadata(&bytes).expect("decode minimal-palette");
        assert_eq!(&img.planes[0].data[..4], &[255u8, 0, 0, 255]);
    }

    #[test]
    fn v4_calibrated_indexed_without_palette_errors() {
        let (idx, stride) = indexed_checker(2, 2);
        let src = BmpImage {
            width: 2,
            height: 2,
            pixel_format: BmpPixelFormat::Indexed8,
            planes: vec![BmpPlane { stride, data: idx }],
            palette: None,
            pts: None,
        };
        let err = encode_bmp_with_calibrated_rgb(
            &src,
            CAL_ENDPOINTS,
            CAL_GAMMA,
            BmpEncodeOptions::default(),
        )
        .unwrap_err();
        let _ = err;
    }

    #[test]
    fn v4_calibrated_zero_endpoints_still_tags_calibrated() {
        // A caller that only wants the LCS_CALIBRATED_RGB tag without
        // asserting primaries passes all-zero endpoints + gamma.
        let (src, _, _) = rgba_checker(2, 2);
        let bytes =
            encode_bmp_with_calibrated_rgb(&src, [0; 9], [0; 3], BmpEncodeOptions::default())
                .expect("zero-endpoint calibrated encode");
        let (_, md) = decode_bmp_with_metadata(&bytes).expect("decode");
        assert_eq!(md.color_space, Some(BmpColorSpace::Calibrated));
        assert_eq!(md.endpoints, Some([0; 9]));
        assert_eq!(md.gamma_rgb, Some([0; 3]));
    }

    // ---- bfOffBits recovery -------------------------------------------
    //
    // `bfOffBits` is the spec source of truth for where the pixel array
    // begins, but minimal/corrupt writers leave it zero (or point it
    // inside the header / colour table). The decoder recovers the
    // canonical layout (file header → DIB header → masks → colour table
    // → pixels) when the stored value cannot be where the pixels start.

    /// Overwrite the 4-byte little-endian `bfOffBits` field (file bytes
    /// 10..14) of an encoded BMP.
    fn set_off_bits(bytes: &mut [u8], value: u32) {
        bytes[10..14].copy_from_slice(&value.to_le_bytes());
    }

    fn read_off_bits(bytes: &[u8]) -> u32 {
        u32::from_le_bytes([bytes[10], bytes[11], bytes[12], bytes[13]])
    }

    #[test]
    fn zero_off_bits_recovers_pixels_32bpp() {
        let (src, _, _) = rgba_checker(7, 5);
        let (mut bytes, _) = encode_bmp(&src).unwrap();
        // Reference decode with the writer's correct bfOffBits.
        let want = decode_bmp(&bytes).unwrap();
        // A minimal writer that left bfOffBits unset (0) must still decode.
        set_off_bits(&mut bytes, 0);
        let got = decode_bmp(&bytes).expect("zero bfOffBits recovers");
        assert_eq!(got.planes[0].data, want.planes[0].data);
    }

    #[test]
    fn zero_off_bits_recovers_pixels_indexed8() {
        // Indexed paths carry a colour table between the header and the
        // pixels, so the recovered offset must skip both the header and
        // the full RGBQUAD table.
        let palette = BmpPalette {
            entries: (0..=255u32)
                .map(|i| [i as u8, (255 - i) as u8, (i.wrapping_mul(3)) as u8])
                .collect(),
        };
        let w = 9u32;
        let h = 6u32;
        let stride = w as usize;
        let mut idx = vec![0u8; stride * h as usize];
        for (n, b) in idx.iter_mut().enumerate() {
            *b = (n % 256) as u8;
        }
        let src = BmpImage {
            width: w,
            height: h,
            pixel_format: BmpPixelFormat::Indexed8,
            planes: vec![BmpPlane { stride, data: idx }],
            palette: Some(palette),
            pts: None,
        };
        // Force the uncompressed path so the on-disk layout is the
        // canonical header → 256-entry colour table → pixels.
        let (mut bytes, _) = encode_bmp_with_options(
            &src,
            BmpEncodeOptions {
                top_down: true,
                ..Default::default()
            },
        )
        .unwrap();
        let want = decode_bmp(&bytes).unwrap();
        set_off_bits(&mut bytes, 0);
        let got = decode_bmp(&bytes).expect("zero bfOffBits recovers (indexed8)");
        assert_eq!(got.planes[0].data, want.planes[0].data);
    }

    #[test]
    fn off_bits_pointing_into_header_is_recovered() {
        // A value that lands inside the DIB header / colour table cannot
        // be the pixel start; recovery moves it forward to the canonical
        // position rather than reading header bytes as pixels.
        let (src, _, _) = rgba_checker(4, 4);
        let (mut bytes, _) = encode_bmp(&src).unwrap();
        let want = decode_bmp(&bytes).unwrap();
        // 20 points 6 bytes into the 40-byte DIB header.
        set_off_bits(&mut bytes, 20);
        let got = decode_bmp(&bytes).expect("early bfOffBits recovers");
        assert_eq!(got.planes[0].data, want.planes[0].data);
    }

    #[test]
    fn larger_off_bits_gap_is_honoured() {
        // A writer is allowed to leave a gap between the colour table and
        // the pixel array; a `bfOffBits` at or past the canonical offset
        // must be honoured verbatim (not clamped back to canonical).
        let (src, _, _) = rgba_checker(4, 4);
        let (orig, _) = encode_bmp(&src).unwrap();
        let canonical = read_off_bits(&orig) as usize;
        let want = decode_bmp(&orig).unwrap();
        // Insert an 8-byte gap right before the pixel array and bump
        // bfOffBits to match.
        let mut bytes = Vec::with_capacity(orig.len() + 8);
        bytes.extend_from_slice(&orig[..canonical]);
        bytes.extend_from_slice(&[0u8; 8]);
        bytes.extend_from_slice(&orig[canonical..]);
        set_off_bits(&mut bytes, (canonical + 8) as u32);
        let got = decode_bmp(&bytes).expect("gap honoured");
        assert_eq!(got.planes[0].data, want.planes[0].data);
    }

    #[test]
    fn zero_off_bits_recovers_metadata_path() {
        // The `_with_metadata` entry point shares the same offset
        // resolution, so it recovers too.
        let (src, _, _) = rgba_checker(5, 3);
        let (mut bytes, _) = encode_bmp(&src).unwrap();
        let (want, _) = decode_bmp_with_metadata(&bytes).unwrap();
        set_off_bits(&mut bytes, 0);
        let (got, _) = decode_bmp_with_metadata(&bytes).expect("metadata path recovers");
        assert_eq!(got.planes[0].data, want.planes[0].data);
    }
}
