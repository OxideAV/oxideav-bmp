//! Pure-Rust BMP (Windows bitmap) codec + container.
//!
//! Handles 1/4/8/16/24/32-bit `BI_RGB` bitmaps plus 16/32-bit
//! `BI_BITFIELDS`, bottom-up and top-down row orders, and v3 / v4 / v5
//! `BITMAPINFOHEADER` variants. Always decodes to an `Rgba`
//! [`VideoFrame`] so consumers don't have to care about palette lookup
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

pub mod container;
pub mod decoder;
pub mod encoder;
pub mod types;

use oxideav_codec::{CodecInfo, CodecRegistry};
use oxideav_container::ContainerRegistry;
use oxideav_core::{CodecCapabilities, CodecId, PixelFormat};

/// Codec id for BMP image frames.
pub const CODEC_ID_STR: &str = "bmp";

pub fn register_codecs(reg: &mut CodecRegistry) {
    let caps = CodecCapabilities::video("bmp_sw")
        .with_intra_only(true)
        .with_lossless(true)
        .with_max_size(65535, 65535)
        .with_pixel_formats(vec![PixelFormat::Rgba, PixelFormat::Rgb24]);
    reg.register(
        CodecInfo::new(CodecId::new(CODEC_ID_STR))
            .capabilities(caps)
            .decoder(decoder::make_decoder)
            .encoder(encoder::make_encoder),
    );
}

pub fn register_containers(reg: &mut ContainerRegistry) {
    container::register(reg);
}

pub fn register(codecs: &mut CodecRegistry, containers: &mut ContainerRegistry) {
    register_codecs(codecs);
    register_containers(containers);
}

pub use decoder::{decode_bmp, decode_dib};
pub use encoder::{encode_bmp, encode_dib};
pub use types::{
    row_stride, DibHeader, BITMAPFILEHEADER_SIZE, BITMAPINFOHEADER_SIZE, BITMAPV4HEADER_SIZE,
    BITMAPV5HEADER_SIZE, BI_BITFIELDS, BI_RGB, BMP_MAGIC,
};

#[cfg(test)]
mod tests {
    use super::*;
    use oxideav_core::{PixelFormat, TimeBase, VideoFrame, VideoPlane};

    fn rgba_checker(w: u32, h: u32) -> VideoFrame {
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
        VideoFrame {
            format: PixelFormat::Rgba,
            width: w,
            height: h,
            pts: None,
            time_base: TimeBase::new(1, 1),
            planes: vec![VideoPlane {
                stride: w as usize * 4,
                data,
            }],
        }
    }

    #[test]
    fn roundtrip_32bpp_rgba() {
        let src = rgba_checker(16, 12);
        let bytes = encode_bmp(&src).unwrap();
        assert_eq!(&bytes[..2], b"BM");
        let back = decode_bmp(&bytes).unwrap();
        assert_eq!(back.width, src.width);
        assert_eq!(back.height, src.height);
        assert_eq!(back.format, PixelFormat::Rgba);
        assert_eq!(back.planes[0].data, src.planes[0].data);
    }

    #[test]
    fn dib_ico_roundtrip_with_and_mask() {
        let src = rgba_checker(8, 8);
        let dib = encode_dib(&src, /* doubled */ true).unwrap();
        // First 4 bytes are the header size = 40.
        assert_eq!(dib[0], 40);
        // The stored height should be 16 (2×8).
        let h = i32::from_le_bytes([dib[8], dib[9], dib[10], dib[11]]);
        assert_eq!(h, 16);
        // Fully opaque quadrants have AND mask = 0; the q=3 (RGBA
        // 255,255,255,128) has alpha != 0 → mask bit = 0 too. With no
        // fully-transparent pixels the whole AND mask should be zero.
        let back = decode_dib(&dib, /* doubled */ true).unwrap();
        assert_eq!(back.width, src.width);
        assert_eq!(back.height, src.height);
        assert_eq!(back.planes[0].data, src.planes[0].data);
    }

    #[test]
    fn decode_rejects_bad_magic() {
        let mut bytes = vec![0u8; 64];
        bytes[0] = b'X';
        assert!(decode_bmp(&bytes).is_err());
    }
}
