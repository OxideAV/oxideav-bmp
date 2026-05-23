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
pub use encoder::{
    encode_bmp, encode_bmp_plane, encode_bmp_plane_with_options, encode_bmp_with_options,
    encode_dib, encode_dib_plane, BmpEncodeOptions, EncodedBmpFormat,
};
#[cfg(feature = "registry")]
pub use encoder::{encode_bmp_videoframe, encode_dib_videoframe};
pub use error::{BmpError, Result};
pub use image::{BmpImage, BmpPalette, BmpPixelFormat, BmpPlane};
pub use types::{
    row_stride, DibHeader, BITMAPCOREHEADER_SIZE, BITMAPFILEHEADER_SIZE, BITMAPINFOHEADER_SIZE,
    BITMAPV4HEADER_SIZE, BITMAPV5HEADER_SIZE, BI_BITFIELDS, BI_RGB, BMP_MAGIC,
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
}
