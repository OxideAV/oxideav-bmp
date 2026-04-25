//! BMP + DIB encode.
//!
//! Always writes 32-bit BGRA `BI_RGB` — the simplest layout that
//! preserves the alpha channel (no `BI_BITFIELDS` gymnastics required)
//! and the one modern tooling expects when you ask for "a BMP with
//! transparency".
//!
//! Input `PixelFormat::Rgba` is accepted directly; `Rgb24` is padded
//! with `0xFF` alpha at encode time. Other pixel formats are rejected
//! so the caller gets a clear error instead of a silent conversion.

use oxideav_core::Encoder;
use oxideav_core::{
    CodecId, CodecParameters, Error, Frame, Packet, PixelFormat, Result, TimeBase, VideoFrame,
};

use crate::types::*;

pub fn make_encoder(_params: &CodecParameters) -> Result<Box<dyn Encoder>> {
    Ok(Box::new(BmpEncoder {
        codec_id: CodecId::new(crate::CODEC_ID_STR),
        out_params: CodecParameters::video(CodecId::new(crate::CODEC_ID_STR)),
        pending: None,
        eof: false,
    }))
}

struct BmpEncoder {
    codec_id: CodecId,
    out_params: CodecParameters,
    pending: Option<Vec<u8>>,
    eof: bool,
}

impl Encoder for BmpEncoder {
    fn codec_id(&self) -> &CodecId {
        &self.codec_id
    }
    fn output_params(&self) -> &CodecParameters {
        &self.out_params
    }
    fn send_frame(&mut self, frame: &Frame) -> Result<()> {
        let vf = match frame {
            Frame::Video(v) => v,
            _ => return Err(Error::invalid("BMP encoder: expected video frame")),
        };
        let bytes = encode_bmp(vf)?;
        self.pending = Some(bytes);
        Ok(())
    }
    fn receive_packet(&mut self) -> Result<Packet> {
        match self.pending.take() {
            Some(bytes) => {
                let mut pkt = Packet::new(0, TimeBase::new(1, 1), bytes);
                pkt.flags.keyframe = true;
                Ok(pkt)
            }
            None => {
                if self.eof {
                    Err(Error::Eof)
                } else {
                    Err(Error::NeedMore)
                }
            }
        }
    }
    fn flush(&mut self) -> Result<()> {
        self.eof = true;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Public standalone API
// ---------------------------------------------------------------------------

/// Encode a [`VideoFrame`] into a complete BMP file (with the 14-byte
/// `BITMAPFILEHEADER`). Always produces 32-bit BGRA `BI_RGB`. Rows are
/// written bottom-up per the classic BMP convention.
pub fn encode_bmp(frame: &VideoFrame) -> Result<Vec<u8>> {
    let (pixels, stride) = pack_rgba(frame)?;
    let w = frame.width;
    let h = frame.height;
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

/// Encode a [`VideoFrame`] into a headerless DIB suitable for `.ico`
/// sub-images. `double_height_for_ico_mask` tells the encoder to:
///
/// * Write the height field as 2×`frame.height` (ICO convention).
/// * Append a 1-bit AND mask derived from the frame's alpha channel:
///   alpha == 0 ⇒ 1 (transparent), alpha != 0 ⇒ 0 (opaque).
///
/// When `false`, the output is a plain 32bpp DIB suitable for embedding
/// wherever someone expects a Windows DIB (clipboard, registry blob, …).
pub fn encode_dib(frame: &VideoFrame, double_height_for_ico_mask: bool) -> Result<Vec<u8>> {
    let (pixels, _) = pack_rgba(frame)?;
    let w = frame.width;
    let h = frame.height;
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
        out.extend_from_slice(&build_and_mask_from_alpha(frame)?);
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Internals
// ---------------------------------------------------------------------------

/// Pack the input frame to 32-bit BGRA bottom-up rows (what BMP
/// expects). Row stride is already a multiple of 4 (32 bpp × width).
fn pack_rgba(frame: &VideoFrame) -> Result<(Vec<u8>, usize)> {
    match frame.format {
        PixelFormat::Rgba | PixelFormat::Rgb24 => {}
        other => {
            return Err(Error::invalid(format!(
                "BMP encoder: unsupported pixel format {other:?}"
            )))
        }
    }
    let w = frame.width as usize;
    let h = frame.height as usize;
    if frame.planes.is_empty() {
        return Err(Error::invalid("BMP encoder: empty frame plane"));
    }
    let in_stride = frame.planes[0].stride;
    let in_bpp = match frame.format {
        PixelFormat::Rgba => 4,
        PixelFormat::Rgb24 => 3,
        _ => unreachable!(),
    };
    if frame.planes[0].data.len() < in_stride * h {
        return Err(Error::invalid("BMP encoder: frame plane truncated"));
    }
    let out_stride = w * 4;
    let mut out = vec![0u8; out_stride * h];
    for y in 0..h {
        let src_y = h - 1 - y; // bottom-up
        let src = &frame.planes[0].data[src_y * in_stride..src_y * in_stride + w * in_bpp];
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
fn build_and_mask_from_alpha(frame: &VideoFrame) -> Result<Vec<u8>> {
    let w = frame.width as usize;
    let h = frame.height as usize;
    let stride = row_stride(w, 1);
    let mut mask = vec![0u8; stride * h];
    let in_stride = frame.planes[0].stride;
    let bpp = match frame.format {
        PixelFormat::Rgba => 4,
        PixelFormat::Rgb24 => {
            // No alpha → fully opaque → all-zero AND mask. Short-circuit.
            return Ok(mask);
        }
        other => {
            return Err(Error::invalid(format!(
                "BMP encoder: AND mask needs RGBA input, got {other:?}"
            )))
        }
    };
    for y in 0..h {
        let src_y = h - 1 - y; // match the bottom-up XOR layout
        let src = &frame.planes[0].data[src_y * in_stride..src_y * in_stride + w * bpp];
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
