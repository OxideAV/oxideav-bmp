//! BMP + DIB decode. Always produces an `Rgba` [`VideoFrame`] — palette
//! lookup and BGR→RGB swapping happen at decode time so consumers don't
//! need to know the on-disk quirks.
//!
//! Supports (enough to cover every common icon / texture / historical
//! artifact you'd meet in the wild):
//!
//! * 1-bit monochrome + palette
//! * 4-bit indexed + palette
//! * 8-bit indexed + palette
//! * 24-bit `BI_RGB` (BGR)
//! * 32-bit `BI_RGB` (BGRA; the `A` byte is often 0 in older files —
//!   we keep it as-is, callers who need "treat all-zero alpha as opaque"
//!   handle that themselves)
//! * 16/32-bit `BI_BITFIELDS` with masks read from the header tail (v3)
//!   or body (v4/v5). Unusual mask combos are expanded via the mask
//!   shift-and-scale routine below.
//!
//! Not supported: RLE4 / RLE8 / JPEG / PNG compression types. A BMP
//! that uses those is almost always better represented in its native
//! container anyway.

use oxideav_codec::Decoder;
use oxideav_core::{
    CodecId, CodecParameters, Error, Frame, Packet, PixelFormat, Result, TimeBase, VideoFrame,
    VideoPlane,
};

use crate::types::*;

/// Factory registered with the codec registry. Consumes one packet per
/// whole BMP file and produces one `Rgba` frame. BMP is a single-image
/// format, so `flush()` just drains the one pending frame.
pub fn make_decoder(_params: &CodecParameters) -> Result<Box<dyn Decoder>> {
    Ok(Box::new(BmpDecoder {
        codec_id: CodecId::new(crate::CODEC_ID_STR),
        pending: None,
        eof: false,
    }))
}

struct BmpDecoder {
    codec_id: CodecId,
    pending: Option<VideoFrame>,
    eof: bool,
}

impl Decoder for BmpDecoder {
    fn codec_id(&self) -> &CodecId {
        &self.codec_id
    }
    fn send_packet(&mut self, packet: &Packet) -> Result<()> {
        let frame = decode_bmp(&packet.data)?;
        self.pending = Some(frame);
        Ok(())
    }
    fn receive_frame(&mut self) -> Result<Frame> {
        match self.pending.take() {
            Some(f) => Ok(Frame::Video(f)),
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

/// Decode a complete BMP file (`BM` signature + file header + DIB +
/// pixels) into an `Rgba` [`VideoFrame`].
pub fn decode_bmp(input: &[u8]) -> Result<VideoFrame> {
    if input.len() < (BITMAPFILEHEADER_SIZE + BITMAPINFOHEADER_SIZE) as usize {
        return Err(Error::invalid("BMP: input shorter than header"));
    }
    let magic = read_u16_le(input, 0);
    if magic != BMP_MAGIC {
        return Err(Error::invalid("BMP: missing 'BM' signature"));
    }
    let pixel_offset = read_u32_le(input, 10) as usize;
    let dib = &input[BITMAPFILEHEADER_SIZE as usize..];
    decode_dib_with_offset(dib, input, pixel_offset)
}

/// Decode a headerless DIB (`BITMAPINFOHEADER` + pixels, no
/// `BITMAPFILEHEADER`) into an `Rgba` [`VideoFrame`]. Used by
/// `oxideav-ico`.
///
/// When `dib_height_is_doubled_for_mask` is true, the incoming
/// `biHeight` is 2× the real height (XOR mask + AND mask layout from
/// `.ico` / `.cur`). The returned frame dimensions are halved on the
/// height axis and the AND mask following the XOR pixels is read into
/// the alpha channel — a 1-bit in the AND mask maps to `alpha = 0`
/// (transparent), a 0-bit keeps whatever the XOR mask wrote.
pub fn decode_dib(input: &[u8], dib_height_is_doubled_for_mask: bool) -> Result<VideoFrame> {
    let (header, _header_bytes) = parse_dib_header(input)?;
    // For a "pure" DIB, pixel data starts right after the header (plus
    // any bitfields masks and color table). Compute the offset the same
    // way `decode_bmp` does via `pixel_offset` so the two paths share
    // the pixel-decode.
    let color_table_bytes = (header.palette_entries() * 4) as u32;
    let masks_bytes =
        if header.compression == BI_BITFIELDS && header.header_size == BITMAPINFOHEADER_SIZE {
            12
        } else {
            0
        };
    let pixel_start = (header.header_size + masks_bytes + color_table_bytes) as usize;
    if dib_height_is_doubled_for_mask {
        decode_dib_with_mask(&header, input, pixel_start)
    } else {
        decode_dib_payload(&header, input, pixel_start)
    }
}

// ---------------------------------------------------------------------------
// Internals
// ---------------------------------------------------------------------------

fn decode_dib_with_offset(
    dib: &[u8],
    whole_file: &[u8],
    pixel_offset: usize,
) -> Result<VideoFrame> {
    let (header, _) = parse_dib_header(dib)?;
    decode_dib_payload(&header, whole_file, pixel_offset)
}

fn parse_dib_header(input: &[u8]) -> Result<(DibHeader, usize)> {
    if input.len() < BITMAPINFOHEADER_SIZE as usize {
        return Err(Error::invalid("BMP: DIB header truncated"));
    }
    let header_size = read_u32_le(input, 0);
    if header_size < BITMAPINFOHEADER_SIZE {
        return Err(Error::invalid(format!(
            "BMP: unsupported header size {header_size}"
        )));
    }
    if input.len() < header_size as usize {
        return Err(Error::invalid("BMP: header size exceeds input"));
    }
    let width = read_i32_le(input, 4);
    let height = read_i32_le(input, 8);
    let planes = read_u16_le(input, 12);
    let bpp = read_u16_le(input, 14);
    let compression = read_u32_le(input, 16);
    let image_size = read_u32_le(input, 20);
    let x_pels_per_meter = read_i32_le(input, 24);
    let y_pels_per_meter = read_i32_le(input, 28);
    let clr_used = read_u32_le(input, 32);
    let clr_important = read_u32_le(input, 36);

    if width <= 0 {
        return Err(Error::invalid("BMP: non-positive width"));
    }
    if planes != 1 {
        return Err(Error::invalid(format!("BMP: planes={planes} (must be 1)")));
    }

    let (mask_r, mask_g, mask_b, mask_a) = if compression == BI_BITFIELDS {
        if header_size >= BITMAPV4HEADER_SIZE {
            // v4/v5 store the masks in the header body.
            (
                Some(read_u32_le(input, 40)),
                Some(read_u32_le(input, 44)),
                Some(read_u32_le(input, 48)),
                Some(read_u32_le(input, 52)),
            )
        } else {
            // v3 places them in the 12 bytes immediately following
            // the 40-byte header.
            if input.len() < (BITMAPINFOHEADER_SIZE + 12) as usize {
                return Err(Error::invalid(
                    "BMP: BI_BITFIELDS needs 12 bytes of masks after header",
                ));
            }
            (
                Some(read_u32_le(input, 40)),
                Some(read_u32_le(input, 44)),
                Some(read_u32_le(input, 48)),
                None,
            )
        }
    } else {
        (None, None, None, None)
    };

    Ok((
        DibHeader {
            header_size,
            width,
            height,
            planes,
            bpp,
            compression,
            image_size,
            x_pels_per_meter,
            y_pels_per_meter,
            clr_used,
            clr_important,
            mask_r,
            mask_g,
            mask_b,
            mask_a,
        },
        header_size as usize,
    ))
}

fn decode_dib_payload(h: &DibHeader, whole: &[u8], pixel_offset: usize) -> Result<VideoFrame> {
    // Reject compressions we don't handle before we go any further.
    match h.compression {
        BI_RGB | BI_BITFIELDS => {}
        BI_RLE4 | BI_RLE8 => return Err(Error::invalid("BMP: RLE compression not supported")),
        BI_JPEG => return Err(Error::invalid("BMP: embedded JPEG not supported")),
        BI_PNG => return Err(Error::invalid("BMP: embedded PNG not supported")),
        c => return Err(Error::invalid(format!("BMP: unknown compression {c}"))),
    }

    let width = h.absolute_width();
    let height = h.absolute_height();
    if width == 0 || height == 0 {
        return Err(Error::invalid("BMP: zero dimension"));
    }

    let palette = read_palette(h, whole, pixel_offset)?;
    let rows = decode_pixels(h, whole, pixel_offset, &palette)?;

    // Flip if needed so output is always top-down (consumer-friendly).
    let rows = if h.is_top_down() {
        rows
    } else {
        rows.into_iter().rev().collect()
    };
    let mut flat = Vec::with_capacity(width as usize * height as usize * 4);
    for row in rows {
        flat.extend_from_slice(&row);
    }

    Ok(VideoFrame {
        format: PixelFormat::Rgba,
        width,
        height,
        pts: None,
        time_base: TimeBase::new(1, 1),
        planes: vec![VideoPlane {
            stride: width as usize * 4,
            data: flat,
        }],
    })
}

fn decode_dib_with_mask(h: &DibHeader, whole: &[u8], pixel_offset: usize) -> Result<VideoFrame> {
    // Height in the DIB is doubled to cover the AND mask; actual
    // pixel height is the real image size.
    let mut xor_header = *h;
    xor_header.height = h.height / 2;
    let mut frame = decode_dib_payload(&xor_header, whole, pixel_offset)?;

    // The AND mask is 1bpp, bottom-up, width-padded to 4 bytes, placed
    // immediately after the XOR pixel array.
    let xor_stride = row_stride(xor_header.absolute_width() as usize, h.bpp as usize);
    let xor_bytes = xor_stride * xor_header.absolute_height() as usize;
    let and_start = pixel_offset + xor_bytes;
    let and_stride = row_stride(xor_header.absolute_width() as usize, 1);
    let and_bytes = and_stride * xor_header.absolute_height() as usize;
    if whole.len() < and_start + and_bytes {
        // Some icons lie about the AND mask size. Warn-by-ignore: if
        // there's no AND mask we just keep the XOR alpha as-is.
        return Ok(frame);
    }
    let and = &whole[and_start..and_start + and_bytes];

    let w = xor_header.absolute_width() as usize;
    let abs_h = xor_header.absolute_height() as usize;
    // AND mask is bottom-up regardless of the XOR flip: the convention
    // for ICO is fixed. Apply it row-by-row, remembering that
    // `decode_dib_payload` has already flipped the XOR to top-down.
    for y in 0..abs_h {
        let src_row = abs_h - 1 - y; // bottom-up
        let row = &and[src_row * and_stride..src_row * and_stride + and_stride];
        for x in 0..w {
            let byte = row[x / 8];
            let bit = (byte >> (7 - (x % 8))) & 1;
            if bit == 1 {
                // AND-mask bit set ⇒ transparent.
                let rgba_off = y * w * 4 + x * 4;
                frame.planes[0].data[rgba_off + 3] = 0;
            }
        }
    }
    Ok(frame)
}

fn read_palette(h: &DibHeader, whole: &[u8], _pixel_offset: usize) -> Result<Vec<[u8; 4]>> {
    let entries = h.palette_entries();
    if entries == 0 {
        return Ok(Vec::new());
    }
    // Palette sits between the header (+ bitfields masks) and the pixel
    // array. For a file BMP we're scanning from the start of the file;
    // for a DIB we're scanning from the DIB start. The caller has
    // already accounted for that in `_pixel_offset`; the palette bytes
    // are `entries * 4` before it.
    let palette_end = _pixel_offset;
    let palette_start = palette_end
        .checked_sub(entries * 4)
        .ok_or_else(|| Error::invalid("BMP: palette extends past pixel offset"))?;
    if whole.len() < palette_end {
        return Err(Error::invalid("BMP: palette truncated"));
    }
    let mut out = Vec::with_capacity(entries);
    for e in 0..entries {
        let off = palette_start + e * 4;
        // On-disk order is B, G, R, reserved.
        out.push([whole[off + 2], whole[off + 1], whole[off], 0xFF]);
    }
    Ok(out)
}

fn decode_pixels(
    h: &DibHeader,
    whole: &[u8],
    pixel_offset: usize,
    palette: &[[u8; 4]],
) -> Result<Vec<Vec<u8>>> {
    let width = h.absolute_width() as usize;
    let height = h.absolute_height() as usize;
    let stride = h.row_stride();
    if whole.len() < pixel_offset + stride * height {
        return Err(Error::invalid("BMP: pixel array truncated"));
    }
    let pixels = &whole[pixel_offset..pixel_offset + stride * height];
    let mut rows: Vec<Vec<u8>> = Vec::with_capacity(height);

    match h.bpp {
        1 => {
            for y in 0..height {
                let row = &pixels[y * stride..y * stride + stride];
                let mut out = Vec::with_capacity(width * 4);
                for x in 0..width {
                    let byte = row[x / 8];
                    let bit = (byte >> (7 - (x % 8))) & 1;
                    let rgba = palette
                        .get(bit as usize)
                        .copied()
                        .unwrap_or([0, 0, 0, 0xFF]);
                    out.extend_from_slice(&rgba);
                }
                rows.push(out);
            }
        }
        4 => {
            for y in 0..height {
                let row = &pixels[y * stride..y * stride + stride];
                let mut out = Vec::with_capacity(width * 4);
                for x in 0..width {
                    let byte = row[x / 2];
                    let idx = if x & 1 == 0 { byte >> 4 } else { byte & 0x0F };
                    let rgba = palette
                        .get(idx as usize)
                        .copied()
                        .unwrap_or([0, 0, 0, 0xFF]);
                    out.extend_from_slice(&rgba);
                }
                rows.push(out);
            }
        }
        8 => {
            for y in 0..height {
                let row = &pixels[y * stride..y * stride + width];
                let mut out = Vec::with_capacity(width * 4);
                for &idx in row {
                    let rgba = palette
                        .get(idx as usize)
                        .copied()
                        .unwrap_or([0, 0, 0, 0xFF]);
                    out.extend_from_slice(&rgba);
                }
                rows.push(out);
            }
        }
        16 => {
            // Default BI_RGB mapping is 5-5-5 with the high bit
            // reserved. BI_BITFIELDS lets the file declare its own
            // layout (e.g. 5-6-5). We honour either.
            let (mr, mg, mb, ma) = if h.compression == BI_BITFIELDS {
                (
                    h.mask_r.unwrap_or(0x7C00),
                    h.mask_g.unwrap_or(0x03E0),
                    h.mask_b.unwrap_or(0x001F),
                    h.mask_a.unwrap_or(0),
                )
            } else {
                (0x7C00, 0x03E0, 0x001F, 0)
            };
            let (rs, rn) = shift_len(mr);
            let (gs, gn) = shift_len(mg);
            let (bs, bn) = shift_len(mb);
            let (as_, an) = shift_len(ma);
            for y in 0..height {
                let row = &pixels[y * stride..y * stride + width * 2];
                let mut out = Vec::with_capacity(width * 4);
                for x in 0..width {
                    let v = u16::from_le_bytes([row[x * 2], row[x * 2 + 1]]) as u32;
                    let r = expand(((v & mr) >> rs) as u8, rn);
                    let g = expand(((v & mg) >> gs) as u8, gn);
                    let b = expand(((v & mb) >> bs) as u8, bn);
                    let a = if an > 0 {
                        expand(((v & ma) >> as_) as u8, an)
                    } else {
                        0xFF
                    };
                    out.extend_from_slice(&[r, g, b, a]);
                }
                rows.push(out);
            }
        }
        24 => {
            for y in 0..height {
                let row = &pixels[y * stride..y * stride + width * 3];
                let mut out = Vec::with_capacity(width * 4);
                for x in 0..width {
                    let b = row[x * 3];
                    let g = row[x * 3 + 1];
                    let r = row[x * 3 + 2];
                    out.extend_from_slice(&[r, g, b, 0xFF]);
                }
                rows.push(out);
            }
        }
        32 => {
            // Default BI_RGB for 32bpp is BGRA. BI_BITFIELDS may declare
            // otherwise; handle both.
            if h.compression == BI_BITFIELDS
                && (h.mask_r.is_some() || h.mask_g.is_some() || h.mask_b.is_some())
            {
                let mr = h.mask_r.unwrap_or(0x00FF_0000);
                let mg = h.mask_g.unwrap_or(0x0000_FF00);
                let mb = h.mask_b.unwrap_or(0x0000_00FF);
                let ma = h.mask_a.unwrap_or(0);
                let (rs, rn) = shift_len(mr);
                let (gs, gn) = shift_len(mg);
                let (bs, bn) = shift_len(mb);
                let (as_, an) = shift_len(ma);
                for y in 0..height {
                    let row = &pixels[y * stride..y * stride + width * 4];
                    let mut out = Vec::with_capacity(width * 4);
                    for x in 0..width {
                        let v = u32::from_le_bytes([
                            row[x * 4],
                            row[x * 4 + 1],
                            row[x * 4 + 2],
                            row[x * 4 + 3],
                        ]);
                        let r = expand(((v & mr) >> rs) as u8, rn);
                        let g = expand(((v & mg) >> gs) as u8, gn);
                        let b = expand(((v & mb) >> bs) as u8, bn);
                        let a = if an > 0 {
                            expand(((v & ma) >> as_) as u8, an)
                        } else {
                            0xFF
                        };
                        out.extend_from_slice(&[r, g, b, a]);
                    }
                    rows.push(out);
                }
            } else {
                for y in 0..height {
                    let row = &pixels[y * stride..y * stride + width * 4];
                    let mut out = Vec::with_capacity(width * 4);
                    for x in 0..width {
                        let b = row[x * 4];
                        let g = row[x * 4 + 1];
                        let r = row[x * 4 + 2];
                        let a = row[x * 4 + 3];
                        out.extend_from_slice(&[r, g, b, a]);
                    }
                    rows.push(out);
                }
            }
        }
        other => {
            return Err(Error::invalid(format!(
                "BMP: unsupported bit depth {other}"
            )))
        }
    }
    Ok(rows)
}

/// Locate a channel mask's bit position + bit length so we can scale
/// it into a full 0..=255 byte.
fn shift_len(mask: u32) -> (u32, u32) {
    if mask == 0 {
        return (0, 0);
    }
    let shift = mask.trailing_zeros();
    let len = 32 - mask.leading_zeros() - shift;
    (shift, len)
}

/// Scale an `n`-bit value up to a full 8-bit byte by repeating the
/// high bits. `n=0` returns 0. `n>=8` truncates to the low 8.
fn expand(v: u8, n: u32) -> u8 {
    match n {
        0 => 0,
        1 => {
            if v & 1 != 0 {
                0xFF
            } else {
                0
            }
        }
        2..=7 => {
            let shift = 8u32.saturating_sub(n);
            let hi = (v as u32) << shift;
            // Repeat top bits into the low gap so 0b11111 at 5 bits
            // maps to 0xFF, not 0xF8.
            (hi | (hi >> n)) as u8
        }
        _ => v,
    }
}
