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
//! Not supported: `BI_RLE4` / `BI_RLE8` / `BI_JPEG` / `BI_PNG`
//! compression (rare in practice, and the latter two defeat the point
//! of wrapping in a BMP in the first place).
//!
//! Encode side always writes 32-bit BGRA `BI_RGB` — simplest layout
//! that preserves alpha without any `BI_BITFIELDS` negotiation.
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
pub use encoder::{encode_bmp, encode_bmp_plane, encode_dib, encode_dib_plane};
#[cfg(feature = "registry")]
pub use decoder::{decode_bmp_videoframe, decode_dib_videoframe};
#[cfg(feature = "registry")]
pub use encoder::{encode_bmp_videoframe, encode_dib_videoframe};
pub use error::{BmpError, Result};
pub use image::{BmpImage, BmpPixelFormat, BmpPlane};
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
            pts: None,
        };
        (image, w, h)
    }

    #[test]
    fn roundtrip_32bpp_rgba() {
        let (src, w, h) = rgba_checker(16, 12);
        let bytes = encode_bmp(&src).unwrap();
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
}
