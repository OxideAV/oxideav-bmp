//! BMP + DIB decode. Always produces an [`BmpImage`] tagged
//! [`BmpPixelFormat::Rgba`] — palette lookup and BGR→RGB swapping
//! happen at decode time so consumers don't need to know the on-disk
//! quirks.
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
//! * 8-bit indexed `BI_RLE8` — decoded bottom-up, then flipped to top-down.
//! * 4-bit indexed `BI_RLE4` — same. Delta codes + absolute mode are
//!   both supported.
//!
//! Not supported: `BI_JPEG` / `BI_PNG` embedded payloads (those defeat
//! the purpose of BMP wrapping).
//!
//! With the default `registry` feature on, the gated `BmpDecoder` trait
//! impl wraps [`decode_bmp`] for the `oxideav_core::Decoder` surface.

use crate::error::{BmpError as Error, Result};
use crate::image::{BmpImage, BmpPixelFormat, BmpPlane};
use crate::types::*;

#[cfg(feature = "registry")]
use oxideav_core::Decoder;
#[cfg(feature = "registry")]
use oxideav_core::{CodecId, CodecParameters, Frame, Packet, VideoFrame, VideoPlane};

/// Factory registered with the codec registry. Consumes one packet per
/// whole BMP file and produces one `Rgba` frame. BMP is a single-image
/// format, so `flush()` just drains the one pending frame.
#[cfg(feature = "registry")]
pub fn make_decoder(_params: &CodecParameters) -> oxideav_core::Result<Box<dyn Decoder>> {
    Ok(Box::new(BmpDecoder {
        codec_id: CodecId::new(crate::CODEC_ID_STR),
        pending: None,
        eof: false,
    }))
}

#[cfg(feature = "registry")]
struct BmpDecoder {
    codec_id: CodecId,
    pending: Option<VideoFrame>,
    eof: bool,
}

#[cfg(feature = "registry")]
impl Decoder for BmpDecoder {
    fn codec_id(&self) -> &CodecId {
        &self.codec_id
    }
    fn send_packet(&mut self, packet: &Packet) -> oxideav_core::Result<()> {
        let image = decode_bmp(&packet.data)?;
        self.pending = Some(image_to_video_frame(image));
        Ok(())
    }
    fn receive_frame(&mut self) -> oxideav_core::Result<Frame> {
        match self.pending.take() {
            Some(f) => Ok(Frame::Video(f)),
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

#[cfg(feature = "registry")]
fn image_to_video_frame(image: BmpImage) -> VideoFrame {
    VideoFrame {
        pts: image.pts,
        planes: image
            .planes
            .into_iter()
            .map(|p| VideoPlane {
                stride: p.stride,
                data: p.data,
            })
            .collect(),
    }
}

// ---------------------------------------------------------------------------
// Public standalone API
// ---------------------------------------------------------------------------

/// Decode a complete BMP file (`BM` signature + file header + DIB +
/// pixels) into an `Rgba` [`BmpImage`].
pub fn decode_bmp(input: &[u8]) -> Result<BmpImage> {
    // Smallest legal DIB is the OS/2 1.x BITMAPCOREHEADER (12 B) on top
    // of the 14-byte BITMAPFILEHEADER. Larger DIB variants are checked
    // again inside `parse_dib_header` after reading the size field.
    if input.len() < (BITMAPFILEHEADER_SIZE + BITMAPCOREHEADER_SIZE) as usize {
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
/// `BITMAPFILEHEADER`) into an `Rgba` [`BmpImage`]. Used by
/// `oxideav-ico`.
///
/// When `dib_height_is_doubled_for_mask` is true, the incoming
/// `biHeight` is 2× the real height (XOR mask + AND mask layout from
/// `.ico` / `.cur`). The returned image dimensions are halved on the
/// height axis and the AND mask following the XOR pixels is read into
/// the alpha channel — a 1-bit in the AND mask maps to `alpha = 0`
/// (transparent), a 0-bit keeps whatever the XOR mask wrote.
pub fn decode_dib(input: &[u8], dib_height_is_doubled_for_mask: bool) -> Result<BmpImage> {
    let (header, _header_bytes) = parse_dib_header(input)?;
    // For a "pure" DIB, pixel data starts right after the header (plus
    // any bitfields masks and color table). Compute the offset the same
    // way `decode_bmp` does via `pixel_offset` so the two paths share
    // the pixel-decode. OS/2 BITMAPCOREHEADER uses 3-byte palette
    // entries (RGBTRIPLE), every other variant uses 4-byte RGBQUAD.
    //
    // `palette_entries()` is bounded only by the attacker-supplied
    // `clr_used` (up to `u32::MAX`), so this product must be done in
    // `usize` — the old `as u32` multiply overflowed (`clr_used =
    // 0xFFFF_FFFF` * 4 wraps) and aborted the process. `read_palette`
    // re-validates the entry count against the actual byte count, so a
    // wildly-large `pixel_start` simply fails the bounds checks there.
    let entry_size = palette_entry_bytes(&header);
    let color_table_bytes = header.palette_entries().saturating_mul(entry_size);
    let masks_bytes =
        if header.compression == BI_BITFIELDS && header.header_size == BITMAPINFOHEADER_SIZE {
            12usize
        } else {
            0usize
        };
    let pixel_start = (header.header_size as usize)
        .saturating_add(masks_bytes)
        .saturating_add(color_table_bytes);
    if dib_height_is_doubled_for_mask {
        decode_dib_with_mask(&header, input, pixel_start)
    } else {
        decode_dib_payload(&header, input, pixel_start)
    }
}

/// Compatibility wrapper around [`decode_bmp`] returning an
/// `oxideav_core::VideoFrame`. Only available with the default
/// `registry` feature; intended for `oxideav-core`-using consumers
/// (e.g. `oxideav-ico`) that haven't migrated to the standalone
/// [`BmpImage`] shape.
#[cfg(feature = "registry")]
pub fn decode_bmp_videoframe(input: &[u8]) -> oxideav_core::Result<VideoFrame> {
    Ok(image_to_video_frame(decode_bmp(input)?))
}

/// Compatibility wrapper around [`decode_dib`] returning an
/// `oxideav_core::VideoFrame`. Only available with the default
/// `registry` feature; intended for `oxideav-core`-using consumers
/// (e.g. `oxideav-ico`) that haven't migrated to the standalone
/// [`BmpImage`] shape.
#[cfg(feature = "registry")]
pub fn decode_dib_videoframe(
    input: &[u8],
    dib_height_is_doubled_for_mask: bool,
) -> oxideav_core::Result<VideoFrame> {
    Ok(image_to_video_frame(decode_dib(
        input,
        dib_height_is_doubled_for_mask,
    )?))
}

// ---------------------------------------------------------------------------
// Internals
// ---------------------------------------------------------------------------

fn decode_dib_with_offset(dib: &[u8], whole_file: &[u8], pixel_offset: usize) -> Result<BmpImage> {
    let (header, _) = parse_dib_header(dib)?;
    decode_dib_payload(&header, whole_file, pixel_offset)
}

fn parse_dib_header(input: &[u8]) -> Result<(DibHeader, usize)> {
    if input.len() < 4 {
        return Err(Error::invalid("BMP: DIB header truncated"));
    }
    let header_size = read_u32_le(input, 0);

    // OS/2 1.x BITMAPCOREHEADER is the only sub-40-byte variant we
    // accept. The layout is fundamentally different from the V3+
    // headers (u16 width/height, no compression field, 3-byte palette
    // entries) so we promote it to a synthesised DibHeader here.
    if header_size == BITMAPCOREHEADER_SIZE {
        return parse_bitmapcoreheader(input);
    }

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

/// Parse a 12-byte OS/2 `BITMAPCOREHEADER` into a [`DibHeader`].
///
/// The OS/2 1.x header is the only legitimate sub-40-byte DIB header.
/// Layout (all little-endian):
/// ```text
///   off  0  u32  bcSize       (always 12)
///   off  4  u16  bcWidth      (unsigned — no top-down support)
///   off  6  u16  bcHeight
///   off  8  u16  bcPlanes     (must be 1)
///   off 10  u16  bcBitCount   (1/4/8/24)
/// ```
/// There is no compression / image-size / DPI / clr_used field; we
/// fill them with zero. Colour-table entries that follow are 3-byte
/// `RGBTRIPLE` not 4-byte `RGBQUAD`; the palette reader honours
/// `header_size == 12` to switch entry stride.
fn parse_bitmapcoreheader(input: &[u8]) -> Result<(DibHeader, usize)> {
    if input.len() < BITMAPCOREHEADER_SIZE as usize {
        return Err(Error::invalid("BMP: BITMAPCOREHEADER truncated"));
    }
    let width = read_u16_le(input, 4) as i32;
    let height = read_u16_le(input, 6) as i32;
    let planes = read_u16_le(input, 8);
    let bpp = read_u16_le(input, 10);
    if width <= 0 {
        return Err(Error::invalid("BMP: BITMAPCOREHEADER zero width"));
    }
    if planes != 1 {
        return Err(Error::invalid(format!(
            "BMP: BITMAPCOREHEADER planes={planes} (must be 1)"
        )));
    }
    Ok((
        DibHeader {
            header_size: BITMAPCOREHEADER_SIZE,
            width,
            height,
            planes,
            bpp,
            compression: BI_RGB,
            image_size: 0,
            x_pels_per_meter: 0,
            y_pels_per_meter: 0,
            clr_used: 0,
            clr_important: 0,
            mask_r: None,
            mask_g: None,
            mask_b: None,
            mask_a: None,
        },
        BITMAPCOREHEADER_SIZE as usize,
    ))
}

fn decode_dib_payload(h: &DibHeader, whole: &[u8], pixel_offset: usize) -> Result<BmpImage> {
    // Reject compressions we don't handle before we go any further.
    match h.compression {
        BI_RGB | BI_BITFIELDS | BI_RLE4 | BI_RLE8 => {}
        BI_JPEG => return Err(Error::invalid("BMP: embedded JPEG not supported")),
        BI_PNG => return Err(Error::invalid("BMP: embedded PNG not supported")),
        c => return Err(Error::invalid(format!("BMP: unknown compression {c}"))),
    }

    let width = h.absolute_width();
    let height = h.absolute_height();
    if width == 0 || height == 0 {
        return Err(Error::invalid("BMP: zero dimension"));
    }

    // Validate bpp before any per-row allocation. The per-bpp decode
    // arms below already reject unknown depths, but only *after*
    // `decode_pixels` has sized its row vector with
    // `Vec::with_capacity(height)`. A header with `bpp = 0` (legal only
    // for BI_JPEG / BI_PNG, both rejected above) yields a zero row
    // stride, so the "pixel array truncated" length check passes for any
    // height — then `with_capacity(height)` tries to reserve an
    // attacker-chosen 134-million-element vector and OOM-aborts. Reject
    // unsupported depths here so a non-zero stride always bounds the
    // height against the available bytes.
    if !matches!(h.bpp, 1 | 4 | 8 | 16 | 24 | 32) {
        return Err(Error::invalid(format!(
            "BMP: unsupported bit depth {}",
            h.bpp
        )));
    }

    let palette = read_palette(h, whole, pixel_offset)?;

    // RLE-compressed bitmaps have a special decode path.
    if h.compression == BI_RLE8 {
        if h.bpp != 8 {
            return Err(Error::invalid("BMP: BI_RLE8 requires bpp=8"));
        }
        let rle_data = rle_input(whole, pixel_offset, width as u64, height as u64)?;
        // decode_rle8 returns rows in bottom-up order (row 0 = bottom of
        // image). Reverse to produce the top-down output the caller expects.
        let rows = decode_rle8(rle_data, width as usize, height as usize, &palette)?;
        let mut flat = Vec::with_capacity(width as usize * height as usize * 4);
        for row in rows.into_iter().rev() {
            flat.extend_from_slice(&row);
        }
        return Ok(BmpImage {
            width,
            height,
            pixel_format: BmpPixelFormat::Rgba,
            planes: vec![BmpPlane {
                stride: width as usize * 4,
                data: flat,
            }],
            palette: None,
            pts: None,
        });
    }
    if h.compression == BI_RLE4 {
        if h.bpp != 4 {
            return Err(Error::invalid("BMP: BI_RLE4 requires bpp=4"));
        }
        let rle_data = rle_input(whole, pixel_offset, width as u64, height as u64)?;
        // Same: bottom-up → reverse to top-down.
        let rows = decode_rle4(rle_data, width as usize, height as usize, &palette)?;
        let mut flat = Vec::with_capacity(width as usize * height as usize * 4);
        for row in rows.into_iter().rev() {
            flat.extend_from_slice(&row);
        }
        return Ok(BmpImage {
            width,
            height,
            pixel_format: BmpPixelFormat::Rgba,
            planes: vec![BmpPlane {
                stride: width as usize * 4,
                data: flat,
            }],
            palette: None,
            pts: None,
        });
    }

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

    Ok(BmpImage {
        width,
        height,
        pixel_format: BmpPixelFormat::Rgba,
        planes: vec![BmpPlane {
            stride: width as usize * 4,
            data: flat,
        }],
        palette: None,
        pts: None,
    })
}

fn decode_dib_with_mask(h: &DibHeader, whole: &[u8], pixel_offset: usize) -> Result<BmpImage> {
    // Height in the DIB is doubled to cover the AND mask; actual
    // pixel height is the real image size.
    let mut xor_header = *h;
    xor_header.height = h.height / 2;
    let mut image = decode_dib_payload(&xor_header, whole, pixel_offset)?;

    // The AND mask is 1bpp, bottom-up, width-padded to 4 bytes, placed
    // immediately after the XOR pixel array. The XOR decode above already
    // proved the pixel array fits in `whole`, but the mask offsets are
    // still derived from attacker-supplied dimensions, so saturate the
    // additions: any overflow lands past `whole.len()` and falls through
    // to the "no AND mask" early-return below rather than wrapping into a
    // small in-bounds index.
    let xor_stride = row_stride(xor_header.absolute_width() as usize, h.bpp as usize);
    let xor_bytes = xor_stride.saturating_mul(xor_header.absolute_height() as usize);
    let and_start = pixel_offset.saturating_add(xor_bytes);
    let and_stride = row_stride(xor_header.absolute_width() as usize, 1);
    let and_bytes = and_stride.saturating_mul(xor_header.absolute_height() as usize);
    if whole.len() < and_start.saturating_add(and_bytes) {
        // Some icons lie about the AND mask size. Warn-by-ignore: if
        // there's no AND mask we just keep the XOR alpha as-is.
        return Ok(image);
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
                image.planes[0].data[rgba_off + 3] = 0;
            }
        }
    }
    Ok(image)
}

/// Bytes-per-palette-entry for a parsed DIB header.
///
/// V3+ headers store 4-byte `RGBQUAD` (B, G, R, reserved). The OS/2 1.x
/// `BITMAPCOREHEADER` stores 3-byte `RGBTRIPLE` (B, G, R). This is the
/// only place that difference matters for the decode pipeline.
fn palette_entry_bytes(h: &DibHeader) -> usize {
    if h.header_size == BITMAPCOREHEADER_SIZE {
        3
    } else {
        4
    }
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
    // are `entries * entry_size` before it. Entry stride is 4 (RGBQUAD)
    // for V3+ and 3 (RGBTRIPLE) for OS/2 BITMAPCOREHEADER.
    let entry_size = palette_entry_bytes(h);
    let palette_end = _pixel_offset;
    let palette_start = palette_end
        .checked_sub(entries * entry_size)
        .ok_or_else(|| Error::invalid("BMP: palette extends past pixel offset"))?;
    if whole.len() < palette_end {
        return Err(Error::invalid("BMP: palette truncated"));
    }
    let mut out = Vec::with_capacity(entries);
    for e in 0..entries {
        let off = palette_start + e * entry_size;
        // On-disk order is B, G, R, (reserved for RGBQUAD only).
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
    // `stride`, `height` and `pixel_offset` are all bounded only by the
    // attacker-supplied header, so size the pixel array with saturating
    // arithmetic. An overflowing `stride * height` would otherwise wrap
    // to a small value, pass the bounds check, then panic on the slice;
    // saturating to `usize::MAX` keeps the truncation check sound.
    let pixel_bytes = stride.saturating_mul(height);
    let pixel_end = pixel_offset.saturating_add(pixel_bytes);
    if whole.len() < pixel_end {
        return Err(Error::invalid("BMP: pixel array truncated"));
    }
    let pixels = &whole[pixel_offset..pixel_end];
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

// ---------------------------------------------------------------------------
// RLE decoders
// ---------------------------------------------------------------------------

/// Slice the RLE stream out of `whole` at `pixel_offset`, after proving
/// the header's `width × height` grid can actually be backed by the
/// available bytes.
///
/// Two attacker-controlled hazards motivate this guard:
///
/// 1. `pixel_offset` comes straight from the file header — a value past
///    the end of the buffer would panic the bare `&whole[pixel_offset..]`
///    slice. We reject it instead.
/// 2. Unlike the uncompressed path (which sizes the pixel array as
///    `stride × height` and bounds-checks it before reading), the RLE
///    decoders pre-allocate the *whole* `width × height` grid up front.
///    A 40-byte header claiming `0x7FFF_FFFF × 0x7FFF_FFFF` would ask the
///    allocator for exabytes and abort the process. The on-disk RLE
///    opcodes can each emit at most 255 pixels, and the smallest opcode
///    that emits any pixel is two bytes (an encoded run / a one-byte
///    absolute run is still framed in pairs), so an `n`-byte stream can
///    decode to no more than `n × 255` pixels. If the claimed grid is
///    larger than that ceiling it can never be filled, so the stream is
///    inconsistent — reject it rather than allocate on the attacker's
///    word. This caps the allocation at the input's own size.
fn rle_input(whole: &[u8], pixel_offset: usize, width: u64, height: u64) -> Result<&[u8]> {
    if pixel_offset > whole.len() {
        return Err(Error::invalid("BMP RLE: pixel offset past end of input"));
    }
    let rle_data = &whole[pixel_offset..];
    let pixels = width.saturating_mul(height);
    let ceiling = (rle_data.len() as u64).saturating_mul(255);
    if pixels > ceiling {
        return Err(Error::invalid(
            "BMP RLE: declared dimensions exceed what the RLE stream can encode",
        ));
    }
    Ok(rle_data)
}

/// Decode a BI_RLE8 stream into bottom-up RGBA rows.
///
/// The stream encodes 8-bit indices; the caller provides the palette.
/// Output rows are in bottom-up order (row 0 = bottom of image) to
/// match the caller's `rev()` flip in `decode_dib_payload`.
fn decode_rle8(
    data: &[u8],
    width: usize,
    height: usize,
    palette: &[[u8; 4]],
) -> Result<Vec<Vec<u8>>> {
    let mut rows: Vec<Vec<u8>> = vec![vec![0u8; width * 4]; height];
    let mut x = 0usize;
    // RLE8 bitmaps are bottom-up: row 0 in the stream is the bottom row.
    let mut y = 0usize;
    let mut i = 0usize;

    macro_rules! put_pixel {
        ($idx:expr) => {
            if x < width && y < height {
                let rgba = palette
                    .get($idx as usize)
                    .copied()
                    .unwrap_or([0, 0, 0, 0xFF]);
                let off = x * 4;
                rows[y][off..off + 4].copy_from_slice(&rgba);
                x += 1;
            }
        };
    }

    while i + 1 < data.len() {
        let b0 = data[i];
        let b1 = data[i + 1];
        i += 2;

        if b0 != 0 {
            // Encoded run: b0 pixels of palette index b1.
            for _ in 0..b0 {
                put_pixel!(b1);
            }
        } else {
            match b1 {
                0x00 => {
                    // End of line.
                    x = 0;
                    y += 1;
                }
                0x01 => {
                    // End of bitmap.
                    break;
                }
                0x02 => {
                    // Delta: move cursor.
                    if i + 2 > data.len() {
                        return Err(Error::invalid("BMP RLE8: delta truncated"));
                    }
                    x += data[i] as usize;
                    y += data[i + 1] as usize;
                    i += 2;
                }
                count => {
                    // Absolute mode: `count` pixels follow.
                    let count = count as usize;
                    if i + count > data.len() {
                        return Err(Error::invalid("BMP RLE8: absolute run truncated"));
                    }
                    for k in 0..count {
                        put_pixel!(data[i + k]);
                    }
                    i += count;
                    // Padded to word boundary.
                    if count & 1 != 0 {
                        i += 1;
                    }
                }
            }
        }
    }
    Ok(rows)
}

/// Decode a BI_RLE4 stream into bottom-up RGBA rows.
fn decode_rle4(
    data: &[u8],
    width: usize,
    height: usize,
    palette: &[[u8; 4]],
) -> Result<Vec<Vec<u8>>> {
    let mut rows: Vec<Vec<u8>> = vec![vec![0u8; width * 4]; height];
    let mut x = 0usize;
    let mut y = 0usize;
    let mut i = 0usize;

    macro_rules! put_pixel {
        ($idx:expr) => {
            if x < width && y < height {
                let rgba = palette
                    .get(($idx & 0x0F) as usize)
                    .copied()
                    .unwrap_or([0, 0, 0, 0xFF]);
                let off = x * 4;
                rows[y][off..off + 4].copy_from_slice(&rgba);
                x += 1;
            }
        };
    }

    while i + 1 < data.len() {
        let b0 = data[i];
        let b1 = data[i + 1];
        i += 2;

        if b0 != 0 {
            // Encoded run: b0 pixels alternating between hi/lo nibble of b1.
            let hi = b1 >> 4;
            let lo = b1 & 0x0F;
            for k in 0..b0 {
                if k & 1 == 0 {
                    put_pixel!(hi);
                } else {
                    put_pixel!(lo);
                }
            }
        } else {
            match b1 {
                0x00 => {
                    // End of line.
                    x = 0;
                    y += 1;
                }
                0x01 => {
                    // End of bitmap.
                    break;
                }
                0x02 => {
                    // Delta.
                    if i + 2 > data.len() {
                        return Err(Error::invalid("BMP RLE4: delta truncated"));
                    }
                    x += data[i] as usize;
                    y += data[i + 1] as usize;
                    i += 2;
                }
                count => {
                    // Absolute mode: `count` nibbles follow in packed bytes.
                    let count = count as usize;
                    let packed_bytes = count.div_ceil(2);
                    if i + packed_bytes > data.len() {
                        return Err(Error::invalid("BMP RLE4: absolute run truncated"));
                    }
                    for k in 0..count {
                        let byte = data[i + k / 2];
                        let nib = if k & 1 == 0 { byte >> 4 } else { byte & 0x0F };
                        put_pixel!(nib);
                    }
                    i += packed_bytes;
                    // Padded to word boundary (in bytes).
                    if packed_bytes & 1 != 0 {
                        i += 1;
                    }
                }
            }
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
