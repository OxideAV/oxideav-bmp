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
//! indexed (with optional `BI_RLE4`), and 1-bit indexed (monochrome,
//! always uncompressed). RLE is chosen automatically when it produces
//! a smaller file than uncompressed indexed.
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
    encode_bmp, encode_bmp_plane, encode_bmp_plane_with_options, encode_bmp_with_icc_profile,
    encode_bmp_with_linked_icc_profile, encode_bmp_with_options, BmpEncodeOptions,
    EncodedBmpFormat,
};
#[cfg(feature = "registry")]
pub use encoder::{encode_bmp_videoframe, encode_dib_videoframe};
pub use encoder::{encode_dib, encode_dib_plane};
pub use error::{BmpError, Result};
pub use image::{BmpImage, BmpPalette, BmpPixelFormat, BmpPlane};
pub use metadata::{BmpColorSpace, BmpMetadata, BmpRenderingIntent};
pub use types::{
    row_stride, DibHeader, BITMAPCOREHEADER_SIZE, BITMAPFILEHEADER_SIZE, BITMAPINFOHEADER_SIZE,
    BITMAPV4HEADER_SIZE, BITMAPV5HEADER_SIZE, BI_ALPHABITFIELDS, BI_BITFIELDS, BI_RGB, BMP_MAGIC,
    LCS_CALIBRATED_RGB, LCS_GM_ABS_COLORIMETRIC, LCS_GM_BUSINESS, LCS_GM_GRAPHICS, LCS_GM_IMAGES,
    LCS_S_RGB, LCS_WINDOWS_COLOR_SPACE, PROFILE_EMBEDDED, PROFILE_LINKED,
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
        // B=0x11, G=0x00, R=0xFF, A=0x80, surfaced as RGBA.
        assert_eq!(&img.planes[0].data[..4], &[0xFF, 0x00, 0x11, 0x80]);
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
}
