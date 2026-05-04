//! BMP + DIB encode.
//!
//! Always writes 32-bit BGRA `BI_RGB` — the simplest layout that
//! preserves the alpha channel (no `BI_BITFIELDS` gymnastics required)
//! and the one modern tooling expects when you ask for "a BMP with
//! transparency".
//!
//! Input [`BmpPixelFormat::Rgba`] is accepted directly;
//! [`BmpPixelFormat::Rgb24`] is padded with `0xFF` alpha at encode
//! time. Other pixel formats are not representable in [`BmpImage`].

use crate::error::{BmpError as Error, Result};
use crate::image::{BmpImage, BmpPixelFormat, BmpPlane};
use crate::types::*;

#[cfg(feature = "registry")]
use oxideav_core::Encoder;
#[cfg(feature = "registry")]
use oxideav_core::{CodecId, CodecParameters, Frame, Packet, PixelFormat, TimeBase};

#[cfg(feature = "registry")]
pub fn make_encoder(params: &CodecParameters) -> oxideav_core::Result<Box<dyn Encoder>> {
    let mut out_params = CodecParameters::video(CodecId::new(crate::CODEC_ID_STR));
    out_params.width = params.width;
    out_params.height = params.height;
    out_params.pixel_format = params.pixel_format;
    Ok(Box::new(BmpEncoder {
        codec_id: CodecId::new(crate::CODEC_ID_STR),
        out_params,
        pending: None,
        eof: false,
    }))
}

#[cfg(feature = "registry")]
struct BmpEncoder {
    codec_id: CodecId,
    out_params: CodecParameters,
    pending: Option<Vec<u8>>,
    eof: bool,
}

#[cfg(feature = "registry")]
impl Encoder for BmpEncoder {
    fn codec_id(&self) -> &CodecId {
        &self.codec_id
    }
    fn output_params(&self) -> &CodecParameters {
        &self.out_params
    }
    fn send_frame(&mut self, frame: &Frame) -> oxideav_core::Result<()> {
        let vf = match frame {
            Frame::Video(v) => v,
            _ => {
                return Err(oxideav_core::Error::invalid(
                    "BMP encoder: expected video frame",
                ))
            }
        };
        let format = self.out_params.pixel_format.ok_or_else(|| {
            oxideav_core::Error::invalid("BMP encoder: pixel_format missing in CodecParameters")
        })?;
        let width = self.out_params.width.ok_or_else(|| {
            oxideav_core::Error::invalid("BMP encoder: width missing in CodecParameters")
        })?;
        let height = self.out_params.height.ok_or_else(|| {
            oxideav_core::Error::invalid("BMP encoder: height missing in CodecParameters")
        })?;
        let bmp_format = match format {
            PixelFormat::Rgba => BmpPixelFormat::Rgba,
            PixelFormat::Rgb24 => BmpPixelFormat::Rgb24,
            other => {
                return Err(oxideav_core::Error::invalid(format!(
                    "BMP encoder: unsupported pixel format {other:?}"
                )))
            }
        };
        if vf.planes.is_empty() {
            return Err(oxideav_core::Error::invalid(
                "BMP encoder: empty frame plane",
            ));
        }
        let plane = BmpPlane {
            stride: vf.planes[0].stride,
            data: vf.planes[0].data.clone(),
        };
        let bytes = encode_bmp_plane(&plane, bmp_format, width, height)?;
        self.pending = Some(bytes);
        Ok(())
    }
    fn receive_packet(&mut self) -> oxideav_core::Result<Packet> {
        match self.pending.take() {
            Some(bytes) => {
                let mut pkt = Packet::new(0, TimeBase::new(1, 1), bytes);
                pkt.flags.keyframe = true;
                Ok(pkt)
            }
            None => {
                if self.eof {
                    Err(oxideav_core::Error::Eof)
                } else {
                    Err(oxideav_core::Error::NeedMore)
                }
            }
        }
    }
    fn flush(&mut self) -> oxideav_core::Result<()> {
        self.eof = true;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Public standalone API
// ---------------------------------------------------------------------------

/// Encode a [`BmpImage`] into a complete BMP file (with the 14-byte
/// `BITMAPFILEHEADER`). Always produces 32-bit BGRA `BI_RGB`. Rows
/// are written bottom-up per the classic BMP convention.
pub fn encode_bmp(image: &BmpImage) -> Result<Vec<u8>> {
    if image.planes.is_empty() {
        return Err(Error::invalid("BMP encoder: empty frame plane"));
    }
    encode_bmp_plane(
        &image.planes[0],
        image.pixel_format,
        image.width,
        image.height,
    )
}

/// Encode a single [`BmpPlane`] (width × height pixels in `format`)
/// into a BMP file. Lower-level than [`encode_bmp`] for callers that
/// already have plane bytes laid out without a wrapping [`BmpImage`].
pub fn encode_bmp_plane(
    plane: &BmpPlane,
    format: BmpPixelFormat,
    width: u32,
    height: u32,
) -> Result<Vec<u8>> {
    let (pixels, stride) = pack_rgba(plane, format, width, height)?;
    let w = width;
    let h = height;
    let pixel_bytes = pixels.len() as u32;
    let _ = stride;
    let file_size = BITMAPFILEHEADER_SIZE + BITMAPINFOHEADER_SIZE + pixel_bytes;

    let mut out = Vec::with_capacity(file_size as usize);
    // BITMAPFILEHEADER
    out.extend_from_slice(&BMP_MAGIC.to_le_bytes());
    out.extend_from_slice(&file_size.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes()); // reserved1
    out.extend_from_slice(&0u16.to_le_bytes()); // reserved2
    let pixel_offset = BITMAPFILEHEADER_SIZE + BITMAPINFOHEADER_SIZE;
    out.extend_from_slice(&pixel_offset.to_le_bytes());
    // BITMAPINFOHEADER (v3, 40 bytes) — BI_RGB, 32 bpp.
    write_dib_header_v3(&mut out, w, h, /* negative_for_top_down */ false);
    // Pixel data — already bottom-up in `pixels`.
    out.extend_from_slice(&pixels);
    Ok(out)
}

/// Encode a [`BmpImage`] into a headerless DIB suitable for `.ico`
/// sub-images. `double_height_for_ico_mask` tells the encoder to:
///
/// * Write the height field as 2×`height` (ICO convention).
/// * Append a 1-bit AND mask derived from the frame's alpha channel:
///   alpha == 0 ⇒ 1 (transparent), alpha != 0 ⇒ 0 (opaque).
///
/// When `false`, the output is a plain 32bpp DIB suitable for embedding
/// wherever someone expects a Windows DIB (clipboard, registry blob, …).
pub fn encode_dib(image: &BmpImage, double_height_for_ico_mask: bool) -> Result<Vec<u8>> {
    if image.planes.is_empty() {
        return Err(Error::invalid("BMP encoder: empty frame plane"));
    }
    encode_dib_plane(
        &image.planes[0],
        image.pixel_format,
        image.width,
        image.height,
        double_height_for_ico_mask,
    )
}

/// Encode a `VideoFrame` into a complete BMP file. Compatibility
/// wrapper around [`encode_bmp_plane`] for `oxideav-core`-using
/// callers (e.g. `oxideav-ico`); only available with the default
/// `registry` feature.
#[cfg(feature = "registry")]
pub fn encode_bmp_videoframe(
    frame: &oxideav_core::VideoFrame,
    format: PixelFormat,
    width: u32,
    height: u32,
) -> oxideav_core::Result<Vec<u8>> {
    let bmp_format = match format {
        PixelFormat::Rgba => BmpPixelFormat::Rgba,
        PixelFormat::Rgb24 => BmpPixelFormat::Rgb24,
        other => {
            return Err(oxideav_core::Error::invalid(format!(
                "BMP encoder: unsupported pixel format {other:?}"
            )))
        }
    };
    if frame.planes.is_empty() {
        return Err(oxideav_core::Error::invalid(
            "BMP encoder: empty frame plane",
        ));
    }
    let plane = BmpPlane {
        stride: frame.planes[0].stride,
        data: frame.planes[0].data.clone(),
    };
    Ok(encode_bmp_plane(&plane, bmp_format, width, height)?)
}

/// Encode a `VideoFrame` into a headerless DIB. Compatibility wrapper
/// around [`encode_dib_plane`] for `oxideav-core`-using callers (e.g.
/// `oxideav-ico`); only available with the default `registry` feature.
#[cfg(feature = "registry")]
pub fn encode_dib_videoframe(
    frame: &oxideav_core::VideoFrame,
    format: PixelFormat,
    width: u32,
    height: u32,
    double_height_for_ico_mask: bool,
) -> oxideav_core::Result<Vec<u8>> {
    let bmp_format = match format {
        PixelFormat::Rgba => BmpPixelFormat::Rgba,
        PixelFormat::Rgb24 => BmpPixelFormat::Rgb24,
        other => {
            return Err(oxideav_core::Error::invalid(format!(
                "BMP encoder: unsupported pixel format {other:?}"
            )))
        }
    };
    if frame.planes.is_empty() {
        return Err(oxideav_core::Error::invalid(
            "BMP encoder: empty frame plane",
        ));
    }
    let plane = BmpPlane {
        stride: frame.planes[0].stride,
        data: frame.planes[0].data.clone(),
    };
    Ok(encode_dib_plane(
        &plane,
        bmp_format,
        width,
        height,
        double_height_for_ico_mask,
    )?)
}

/// Encode a single [`BmpPlane`] into a headerless DIB. Lower-level
/// than [`encode_dib`] for callers that already have plane bytes laid
/// out without a wrapping [`BmpImage`].
pub fn encode_dib_plane(
    plane: &BmpPlane,
    format: BmpPixelFormat,
    width: u32,
    height: u32,
    double_height_for_ico_mask: bool,
) -> Result<Vec<u8>> {
    let (pixels, _) = pack_rgba(plane, format, width, height)?;
    let w = width;
    let h = height;
    let mut out = Vec::new();
    // BITMAPINFOHEADER only — no file header.
    write_dib_header_v3(
        &mut out,
        w,
        if double_height_for_ico_mask { h * 2 } else { h },
        /* negative_for_top_down */ false,
    );
    out.extend_from_slice(&pixels);
    if double_height_for_ico_mask {
        out.extend_from_slice(&build_and_mask_from_alpha(plane, format, width, height)?);
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Internals
// ---------------------------------------------------------------------------

/// Pack the input plane to 32-bit BGRA bottom-up rows (what BMP
/// expects). Row stride is already a multiple of 4 (32 bpp × width).
fn pack_rgba(
    plane: &BmpPlane,
    format: BmpPixelFormat,
    width: u32,
    height: u32,
) -> Result<(Vec<u8>, usize)> {
    let w = width as usize;
    let h = height as usize;
    let in_stride = plane.stride;
    let in_bpp = match format {
        BmpPixelFormat::Rgba => 4,
        BmpPixelFormat::Rgb24 => 3,
    };
    if plane.data.len() < in_stride * h {
        return Err(Error::invalid("BMP encoder: frame plane truncated"));
    }
    let out_stride = w * 4;
    let mut out = vec![0u8; out_stride * h];
    for y in 0..h {
        let src_y = h - 1 - y; // bottom-up
        let src = &plane.data[src_y * in_stride..src_y * in_stride + w * in_bpp];
        let dst = &mut out[y * out_stride..y * out_stride + out_stride];
        for x in 0..w {
            let (r, g, b, a) = match in_bpp {
                4 => (src[x * 4], src[x * 4 + 1], src[x * 4 + 2], src[x * 4 + 3]),
                3 => (src[x * 3], src[x * 3 + 1], src[x * 3 + 2], 0xFF),
                _ => unreachable!(),
            };
            dst[x * 4] = b;
            dst[x * 4 + 1] = g;
            dst[x * 4 + 2] = r;
            dst[x * 4 + 3] = a;
        }
    }
    Ok((out, out_stride))
}

fn write_dib_header_v3(out: &mut Vec<u8>, w: u32, stored_height: u32, _top_down: bool) {
    // 40-byte BITMAPINFOHEADER for 32-bit BI_RGB.
    out.extend_from_slice(&BITMAPINFOHEADER_SIZE.to_le_bytes());
    out.extend_from_slice(&(w as i32).to_le_bytes());
    out.extend_from_slice(&(stored_height as i32).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // planes
    out.extend_from_slice(&32u16.to_le_bytes()); // bpp
    out.extend_from_slice(&BI_RGB.to_le_bytes());
    let image_size: u32 = w * stored_height * 4;
    out.extend_from_slice(&image_size.to_le_bytes());
    out.extend_from_slice(&2835i32.to_le_bytes()); // x_pels/m = 72 DPI
    out.extend_from_slice(&2835i32.to_le_bytes()); // y_pels/m
    out.extend_from_slice(&0u32.to_le_bytes()); // clr_used
    out.extend_from_slice(&0u32.to_le_bytes()); // clr_important
}

/// Build the 1-bpp AND mask (bottom-up, 4-byte padded rows) required
/// for a BMP embedded in a `.ico` / `.cur`. A set bit means "the pixel
/// under this one in the XOR mask is TRANSPARENT", matching every
/// `ICO` file you'll find in the wild.
fn build_and_mask_from_alpha(
    plane: &BmpPlane,
    format: BmpPixelFormat,
    width: u32,
    height: u32,
) -> Result<Vec<u8>> {
    let w = width as usize;
    let h = height as usize;
    let stride = row_stride(w, 1);
    let mut mask = vec![0u8; stride * h];
    let in_stride = plane.stride;
    let bpp = match format {
        BmpPixelFormat::Rgba => 4,
        BmpPixelFormat::Rgb24 => {
            // No alpha → fully opaque → all-zero AND mask. Short-circuit.
            return Ok(mask);
        }
    };
    for y in 0..h {
        let src_y = h - 1 - y; // match the bottom-up XOR layout
        let src = &plane.data[src_y * in_stride..src_y * in_stride + w * bpp];
        let dst = &mut mask[y * stride..y * stride + stride];
        for x in 0..w {
            let alpha = src[x * bpp + 3];
            if alpha == 0 {
                dst[x / 8] |= 1 << (7 - (x % 8));
            }
        }
    }
    Ok(mask)
}
