//! BMP + DIB encode.
//!
//! Supported output variants:
//!
//! | Format                | Compression  | Header |
//! | --------------------- | ------------ | ------ |
//! | 32-bit BGRA           | `BI_RGB`     | V3     |
//! | 24-bit BGR            | `BI_RGB`     | V3     |
//! | 16-bit RGB 5-6-5      | `BI_BITFIELDS` | V4   |
//! | 8-bit indexed         | `BI_RGB`     | V3     |
//! | 4-bit indexed         | `BI_RGB`     | V3     |
//! | 1-bit indexed         | `BI_RGB`     | V3     |
//! | 8-bit indexed RLE     | `BI_RLE8`    | V3     |
//! | 4-bit indexed RLE     | `BI_RLE4`    | V3     |
//!
//! For RLE variants the encoder first tries the compressed form and falls
//! back to uncompressed indexed when the compressed output is not smaller.
//! BMP has no RLE flavour at 1 bpp, so the [`BmpPixelFormat::Indexed1`]
//! path is always emitted as uncompressed `BI_RGB`.
//!
//! Input [`BmpPixelFormat::Rgba`] is accepted directly;
//! [`BmpPixelFormat::Rgb24`] is written as 24-bit BGR.
//! [`BmpPixelFormat::Rgb565`] is written as 16-bit BI_BITFIELDS (V4 header).
//! [`BmpPixelFormat::Indexed8`] / [`BmpPixelFormat::Indexed4`] /
//! [`BmpPixelFormat::Indexed1`] require a [`BmpPalette`] in the
//! accompanying [`BmpImage`]; optional RLE is chosen automatically when
//! it compresses (8/4-bit only).

use crate::error::{BmpError as Error, Result};
use crate::image::{BmpImage, BmpPalette, BmpPixelFormat, BmpPlane};
use crate::types::*;

#[cfg(feature = "registry")]
use oxideav_core::Encoder;
#[cfg(feature = "registry")]
use oxideav_core::{CodecId, CodecParameters, Frame, Packet, PixelFormat, TimeBase};

/// Options that tune the BMP encoder beyond the format-picking that
/// [`encode_bmp`] / [`encode_bmp_plane`] derive from
/// [`BmpPixelFormat`]. Pass via [`encode_bmp_with_options`] or
/// [`encode_bmp_plane_with_options`].
///
/// Defaults match the classic BMP convention: rows bottom-up,
/// `biHeight` positive. Setting [`top_down`](Self::top_down) inverts
/// the layout: rows are written top-down (no in-encoder flip) and
/// the encoded `biHeight` field is the negative of the height per
/// the BMP spec's signed-height convention. Top-down output is
/// compatible with `BI_RGB` only (uncompressed direct-colour /
/// uncompressed indexed and 16-bit `BI_BITFIELDS`); RLE-compressed
/// payloads with negative heights are explicitly disallowed by the
/// spec, so requesting top-down on `Indexed8` / `Indexed4` forces
/// the uncompressed fall-back.
#[derive(Debug, Clone, Copy, Default)]
pub struct BmpEncodeOptions {
    /// Emit a top-down DIB (rows stored top-to-bottom, encoded
    /// `biHeight` is negative). Default: `false` (classic bottom-up).
    pub top_down: bool,
    /// Write only as many colour-table entries as the supplied palette
    /// actually carries, recording the count in the header's
    /// `biClrUsed` field, instead of zero-padding the table out to the
    /// full `2^bpp` entries with `biClrUsed = 0`.
    ///
    /// Only affects the indexed paths (`Indexed8` / `Indexed4`); the
    /// direct-colour and 16-bit bitfields paths carry no colour table.
    /// A palette with `n` entries shrinks the on-disk table from
    /// `2^bpp × 4` bytes to `n × 4` bytes — meaningful for the common
    /// 2-/4-colour images that would otherwise carry a full 256-entry
    /// (1 KiB) or 16-entry table. The decoder's `biClrUsed`-aware
    /// palette reader consumes the shorter table transparently.
    ///
    /// Default: `false` (full `2^bpp` table, `biClrUsed = 0`) for
    /// byte-for-byte compatibility with prior output.
    pub minimal_palette: bool,
}

/// Opaque token returned by [`encode_bmp`] and [`encode_bmp_plane`]
/// that carries the actual compression used. Inspect with
/// [`EncodedBmpFormat::compression`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncodedBmpFormat {
    /// 32-bit BGRA `BI_RGB`.
    Rgb32,
    /// 24-bit BGR `BI_RGB`.
    Rgb24,
    /// 16-bit `BI_BITFIELDS` RGB 5-6-5.
    Rgb16Bitfields,
    /// 8-bit uncompressed indexed `BI_RGB`.
    Indexed8,
    /// 4-bit uncompressed indexed `BI_RGB`.
    Indexed4,
    /// 8-bit RLE-compressed indexed `BI_RLE8`.
    Rle8,
    /// 4-bit RLE-compressed indexed `BI_RLE4`.
    Rle4,
    /// 1-bit uncompressed indexed `BI_RGB` (monochrome).
    Indexed1,
}

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
        let (bytes, _) = encode_bmp_plane(&plane, bmp_format, None, width, height)?;
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
/// `BITMAPFILEHEADER`). The output format is chosen from
/// [`BmpImage::pixel_format`]:
///
/// * `Rgba` → 32-bit BGRA `BI_RGB` (V3 header)
/// * `Rgb24` → 24-bit BGR `BI_RGB` (V3 header)
/// * `Rgb565` → 16-bit `BI_BITFIELDS` RGB 5-6-5 (V4 header)
/// * `Indexed8` → 8-bit indexed `BI_RGB` or `BI_RLE8` (whichever is
///   smaller); requires `image.palette`.
/// * `Indexed4` → 4-bit indexed `BI_RGB` or `BI_RLE4` (whichever is
///   smaller); requires `image.palette`.
///
/// Rows are written bottom-up per the classic BMP convention.
///
/// Returns the encoded bytes and which format was actually emitted.
pub fn encode_bmp(image: &BmpImage) -> Result<(Vec<u8>, EncodedBmpFormat)> {
    encode_bmp_with_options(image, BmpEncodeOptions::default())
}

/// Same as [`encode_bmp`] but takes a [`BmpEncodeOptions`] so callers
/// can request a top-down DIB layout (negative `biHeight`).
///
/// Top-down output is BMP-spec-compliant for uncompressed `BI_RGB` and
/// `BI_BITFIELDS` only. When `options.top_down == true` and the chosen
/// format would otherwise be `BI_RLE8` / `BI_RLE4`, the encoder falls
/// back to the uncompressed indexed form regardless of which is
/// smaller, since RLE + negative height is illegal per the spec.
pub fn encode_bmp_with_options(
    image: &BmpImage,
    options: BmpEncodeOptions,
) -> Result<(Vec<u8>, EncodedBmpFormat)> {
    if image.planes.is_empty() {
        return Err(Error::invalid("BMP encoder: empty frame plane"));
    }
    encode_bmp_plane_with_options(
        &image.planes[0],
        image.pixel_format,
        image.palette.as_ref(),
        image.width,
        image.height,
        options,
    )
}

/// Encode a single [`BmpPlane`] (width × height pixels in `format`)
/// into a BMP file. Lower-level than [`encode_bmp`] for callers that
/// already have plane bytes laid out without a wrapping [`BmpImage`].
///
/// `palette` is required for [`BmpPixelFormat::Indexed8`] and
/// [`BmpPixelFormat::Indexed4`]; ignored otherwise.
///
/// Returns the encoded bytes and which format was actually emitted.
pub fn encode_bmp_plane(
    plane: &BmpPlane,
    format: BmpPixelFormat,
    palette: Option<&BmpPalette>,
    width: u32,
    height: u32,
) -> Result<(Vec<u8>, EncodedBmpFormat)> {
    encode_bmp_plane_with_options(
        plane,
        format,
        palette,
        width,
        height,
        BmpEncodeOptions::default(),
    )
}

/// Plane-level variant of [`encode_bmp_with_options`].
pub fn encode_bmp_plane_with_options(
    plane: &BmpPlane,
    format: BmpPixelFormat,
    palette: Option<&BmpPalette>,
    width: u32,
    height: u32,
    options: BmpEncodeOptions,
) -> Result<(Vec<u8>, EncodedBmpFormat)> {
    match format {
        BmpPixelFormat::Rgba => {
            let bytes = encode_direct(plane, format, width, height, BI_RGB, None, options)?;
            Ok((bytes, EncodedBmpFormat::Rgb32))
        }
        BmpPixelFormat::Rgb24 => {
            let bytes = encode_direct(plane, format, width, height, BI_RGB, None, options)?;
            Ok((bytes, EncodedBmpFormat::Rgb24))
        }
        BmpPixelFormat::Rgb565 => {
            let bytes = encode_rgb565(plane, width, height, options)?;
            Ok((bytes, EncodedBmpFormat::Rgb16Bitfields))
        }
        BmpPixelFormat::Indexed8 => {
            let pal = palette
                .ok_or_else(|| Error::invalid("BMP encoder: Indexed8 requires a palette"))?;
            encode_indexed8_auto(plane, pal, width, height, options)
        }
        BmpPixelFormat::Indexed4 => {
            let pal = palette
                .ok_or_else(|| Error::invalid("BMP encoder: Indexed4 requires a palette"))?;
            encode_indexed4_auto(plane, pal, width, height, options)
        }
        BmpPixelFormat::Indexed1 => {
            let pal = palette
                .ok_or_else(|| Error::invalid("BMP encoder: Indexed1 requires a palette"))?;
            let (raw_pixels, _) = pack_indexed(plane, 1, width, height, options)?;
            let file = build_indexed_bmp(width, height, 1, BI_RGB, pal, &raw_pixels, options);
            Ok((file, EncodedBmpFormat::Indexed1))
        }
    }
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
///
/// For indexed and 16-bit formats the DIB path always uses the
/// uncompressed form (RLE + ICO mask interaction is undefined).
pub fn encode_dib(image: &BmpImage, double_height_for_ico_mask: bool) -> Result<Vec<u8>> {
    if image.planes.is_empty() {
        return Err(Error::invalid("BMP encoder: empty frame plane"));
    }
    encode_dib_plane(
        &image.planes[0],
        image.pixel_format,
        image.palette.as_ref(),
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
    let (bytes, _) = encode_bmp_plane(&plane, bmp_format, None, width, height)?;
    Ok(bytes)
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
        None,
        width,
        height,
        double_height_for_ico_mask,
    )?)
}

/// Encode a single [`BmpPlane`] into a headerless DIB. Lower-level
/// than [`encode_dib`] for callers that already have plane bytes laid
/// out without a wrapping [`BmpImage`].
///
/// `palette` is required for [`BmpPixelFormat::Indexed8`] and
/// [`BmpPixelFormat::Indexed4`]; ignored otherwise.
///
/// For indexed formats the DIB is always written uncompressed (no RLE)
/// since RLE + ICO AND-mask interaction is undefined.
pub fn encode_dib_plane(
    plane: &BmpPlane,
    format: BmpPixelFormat,
    palette: Option<&BmpPalette>,
    width: u32,
    height: u32,
    double_height_for_ico_mask: bool,
) -> Result<Vec<u8>> {
    // DIB path is always bottom-up (the AND-mask convention assumes
    // bottom-up XOR pixels; nothing in `oxideav-ico` requests top-down).
    let opts = BmpEncodeOptions::default();
    match format {
        BmpPixelFormat::Rgba | BmpPixelFormat::Rgb24 => {
            // Classic 32-bpp BGRA DIB path (used by oxideav-ico).
            let (pixels, _) = pack_rgba(plane, format, width, height, opts)?;
            let w = width;
            let h = height;
            let mut out = Vec::new();
            write_dib_header_v3(
                &mut out,
                w,
                if double_height_for_ico_mask {
                    (h * 2) as i32
                } else {
                    h as i32
                },
                32,
                BI_RGB,
                0,
            );
            out.extend_from_slice(&pixels);
            if double_height_for_ico_mask {
                out.extend_from_slice(&build_and_mask_from_alpha(plane, format, width, height)?);
            }
            Ok(out)
        }
        BmpPixelFormat::Rgb565 => {
            // 16-bit DIB — no ICO mask support for 16-bit (alpha is
            // meaningless in 5-6-5 anyway).
            let (pixels, _) = pack_rgb565(plane, width, height, opts)?;
            let mut out = Vec::new();
            let stored_h = if double_height_for_ico_mask {
                (height * 2) as i32
            } else {
                height as i32
            };
            write_dib_header_v4_bitfields(&mut out, width, stored_h);
            out.extend_from_slice(&pixels);
            Ok(out)
        }
        BmpPixelFormat::Indexed8 => {
            let pal = palette
                .ok_or_else(|| Error::invalid("BMP encoder: Indexed8 requires a palette"))?;
            let (pixels, _) = pack_indexed(plane, 8, width, height, opts)?;
            let stored_h = if double_height_for_ico_mask {
                (height * 2) as i32
            } else {
                height as i32
            };
            let mut out = Vec::new();
            let entries = written_palette_entries(8, pal, opts);
            write_dib_header_v3_indexed(&mut out, width, stored_h, 8, BI_RGB, pal, entries);
            out.extend_from_slice(&pixels);
            Ok(out)
        }
        BmpPixelFormat::Indexed4 => {
            let pal = palette
                .ok_or_else(|| Error::invalid("BMP encoder: Indexed4 requires a palette"))?;
            let (pixels, _) = pack_indexed(plane, 4, width, height, opts)?;
            let stored_h = if double_height_for_ico_mask {
                (height * 2) as i32
            } else {
                height as i32
            };
            let mut out = Vec::new();
            let entries = written_palette_entries(4, pal, opts);
            write_dib_header_v3_indexed(&mut out, width, stored_h, 4, BI_RGB, pal, entries);
            out.extend_from_slice(&pixels);
            Ok(out)
        }
        BmpPixelFormat::Indexed1 => {
            let pal = palette
                .ok_or_else(|| Error::invalid("BMP encoder: Indexed1 requires a palette"))?;
            let (pixels, _) = pack_indexed(plane, 1, width, height, opts)?;
            let stored_h = if double_height_for_ico_mask {
                (height * 2) as i32
            } else {
                height as i32
            };
            let mut out = Vec::new();
            let entries = written_palette_entries(1, pal, opts);
            write_dib_header_v3_indexed(&mut out, width, stored_h, 1, BI_RGB, pal, entries);
            out.extend_from_slice(&pixels);
            Ok(out)
        }
    }
}

// ---------------------------------------------------------------------------
// Per-format encode helpers
// ---------------------------------------------------------------------------

/// Stored signed `biHeight` for a given output height + layout choice.
/// Bottom-up DIBs encode the absolute height; top-down DIBs encode its
/// negation per the BMP spec's signed-height convention.
fn signed_stored_height(h: u32, options: BmpEncodeOptions) -> i32 {
    if options.top_down {
        -(h as i32)
    } else {
        h as i32
    }
}

/// Encode 32-bit BGRA or 24-bit BGR `BI_RGB` BMP (no palette).
fn encode_direct(
    plane: &BmpPlane,
    format: BmpPixelFormat,
    width: u32,
    height: u32,
    compression: u32,
    _palette: Option<&BmpPalette>,
    options: BmpEncodeOptions,
) -> Result<Vec<u8>> {
    let (pixels, bpp) = match format {
        BmpPixelFormat::Rgba => {
            let (p, _) = pack_rgba(plane, format, width, height, options)?;
            (p, 32u16)
        }
        BmpPixelFormat::Rgb24 => {
            let (p, _) = pack_rgb24(plane, width, height, options)?;
            (p, 24u16)
        }
        _ => return Err(Error::invalid("BMP encode_direct: unsupported format")),
    };
    let pixel_bytes = pixels.len() as u32;
    let file_size = BITMAPFILEHEADER_SIZE + BITMAPINFOHEADER_SIZE + pixel_bytes;
    let mut out = Vec::with_capacity(file_size as usize);
    write_file_header(
        &mut out,
        file_size,
        BITMAPFILEHEADER_SIZE + BITMAPINFOHEADER_SIZE,
    );
    write_dib_header_v3(
        &mut out,
        width,
        signed_stored_height(height, options),
        bpp,
        compression,
        0,
    );
    out.extend_from_slice(&pixels);
    Ok(out)
}

/// Encode 16-bit RGB 5-6-5 BI_BITFIELDS (V4 header) BMP.
fn encode_rgb565(
    plane: &BmpPlane,
    width: u32,
    height: u32,
    options: BmpEncodeOptions,
) -> Result<Vec<u8>> {
    let (pixels, _) = pack_rgb565(plane, width, height, options)?;
    let pixel_bytes = pixels.len() as u32;
    // V4 header = 108 bytes; no separate bitfield mask block needed
    // (masks live inside the V4 header at offsets 40-55).
    let file_size = BITMAPFILEHEADER_SIZE + BITMAPV4HEADER_SIZE + pixel_bytes;
    let pixel_offset = BITMAPFILEHEADER_SIZE + BITMAPV4HEADER_SIZE;
    let mut out = Vec::with_capacity(file_size as usize);
    write_file_header(&mut out, file_size, pixel_offset);
    write_dib_header_v4_bitfields(&mut out, width, signed_stored_height(height, options));
    out.extend_from_slice(&pixels);
    Ok(out)
}

/// Encode 8-bit indexed with auto RLE (picks whichever is smaller).
///
/// When `options.top_down == true`, RLE is unconditionally skipped:
/// BMP RLE streams describe a bottom-up scan with `(end-of-line,
/// delta, end-of-bitmap)` escape codes that have no defined meaning
/// under a negative `biHeight`. Fall back to the uncompressed indexed
/// path so the output stays spec-compliant.
fn encode_indexed8_auto(
    plane: &BmpPlane,
    palette: &BmpPalette,
    width: u32,
    height: u32,
    options: BmpEncodeOptions,
) -> Result<(Vec<u8>, EncodedBmpFormat)> {
    let (raw_pixels, _) = pack_indexed(plane, 8, width, height, options)?;

    if !options.top_down {
        // Try RLE8 only when bottom-up.
        let rle_pixels = rle8_encode(&raw_pixels, width, height);
        if rle_pixels.len() < raw_pixels.len() {
            let file = build_indexed_bmp(width, height, 8, BI_RLE8, palette, &rle_pixels, options);
            return Ok((file, EncodedBmpFormat::Rle8));
        }
    }
    let file = build_indexed_bmp(width, height, 8, BI_RGB, palette, &raw_pixels, options);
    Ok((file, EncodedBmpFormat::Indexed8))
}

/// Encode 4-bit indexed with auto RLE (picks whichever is smaller).
///
/// Top-down skips RLE for the same reason as
/// [`encode_indexed8_auto`].
fn encode_indexed4_auto(
    plane: &BmpPlane,
    palette: &BmpPalette,
    width: u32,
    height: u32,
    options: BmpEncodeOptions,
) -> Result<(Vec<u8>, EncodedBmpFormat)> {
    let (raw_pixels, _) = pack_indexed(plane, 4, width, height, options)?;

    if !options.top_down {
        let rle_pixels = rle4_encode(&raw_pixels, width, height);
        if rle_pixels.len() < raw_pixels.len() {
            let file = build_indexed_bmp(width, height, 4, BI_RLE4, palette, &rle_pixels, options);
            return Ok((file, EncodedBmpFormat::Rle4));
        }
    }
    let file = build_indexed_bmp(width, height, 4, BI_RGB, palette, &raw_pixels, options);
    Ok((file, EncodedBmpFormat::Indexed4))
}

/// Number of colour-table entries actually written on disk for the
/// given bit depth, palette, and options.
///
/// With `minimal_palette` set the table is exactly `palette.entries`
/// long (clamped to the `2^bpp` ceiling so a caller can't overflow the
/// index space); otherwise the classic full `2^bpp` table is emitted.
/// A `minimal_palette` table is never shrunk below 1 entry — a
/// zero-entry colour table is meaningless for an indexed bitmap.
fn written_palette_entries(bpp: u16, palette: &BmpPalette, options: BmpEncodeOptions) -> usize {
    let full = palette_entry_count(bpp);
    if options.minimal_palette {
        palette.entries.len().clamp(1, full)
    } else {
        full
    }
}

/// Assemble a complete BMP file for indexed data.
fn build_indexed_bmp(
    width: u32,
    height: u32,
    bpp: u16,
    compression: u32,
    palette: &BmpPalette,
    pixel_data: &[u8],
    options: BmpEncodeOptions,
) -> Vec<u8> {
    let entries = written_palette_entries(bpp, palette, options);
    let palette_bytes = (entries * 4) as u32;
    let pixel_bytes = pixel_data.len() as u32;
    let file_size = BITMAPFILEHEADER_SIZE + BITMAPINFOHEADER_SIZE + palette_bytes + pixel_bytes;
    let pixel_offset = BITMAPFILEHEADER_SIZE + BITMAPINFOHEADER_SIZE + palette_bytes;
    let mut out = Vec::with_capacity(file_size as usize);
    write_file_header(&mut out, file_size, pixel_offset);
    write_dib_header_v3_indexed(
        &mut out,
        width,
        signed_stored_height(height, options),
        bpp,
        compression,
        palette,
        entries,
    );
    out.extend_from_slice(pixel_data);
    out
}

// ---------------------------------------------------------------------------
// Pixel-packing helpers
// ---------------------------------------------------------------------------

/// Map output row index `y` to its source row in the caller's plane.
/// Bottom-up DIBs read source bottom-first so the destination row 0
/// in the BMP file holds the bottom of the picture. Top-down DIBs
/// preserve source ordering — destination row 0 IS source row 0.
#[inline]
fn source_row(y: usize, h: usize, options: BmpEncodeOptions) -> usize {
    if options.top_down {
        y
    } else {
        h - 1 - y
    }
}

/// Pack RGBA or RGB24 input to 32-bit BGRA rows. The output row order
/// matches the layout requested by `options` — bottom-up by default,
/// top-down when `options.top_down` is set.
fn pack_rgba(
    plane: &BmpPlane,
    format: BmpPixelFormat,
    width: u32,
    height: u32,
    options: BmpEncodeOptions,
) -> Result<(Vec<u8>, usize)> {
    let w = width as usize;
    let h = height as usize;
    let in_stride = plane.stride;
    let in_bpp = match format {
        BmpPixelFormat::Rgba => 4,
        BmpPixelFormat::Rgb24 => 3,
        _ => return Err(Error::invalid("pack_rgba: format is not Rgba or Rgb24")),
    };
    if plane.data.len() < in_stride * h {
        return Err(Error::invalid("BMP encoder: frame plane truncated"));
    }
    let out_stride = w * 4;
    let mut out = vec![0u8; out_stride * h];
    for y in 0..h {
        let src_y = source_row(y, h, options);
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

/// Pack RGBA or RGB24 input to 24-bit BGR rows (4-byte row aligned).
/// Row order honours `options.top_down`.
fn pack_rgb24(
    plane: &BmpPlane,
    width: u32,
    height: u32,
    options: BmpEncodeOptions,
) -> Result<(Vec<u8>, usize)> {
    let w = width as usize;
    let h = height as usize;
    let in_stride = plane.stride;
    // Input may be Rgba (4 bpp) or Rgb24 (3 bpp) — detect from stride.
    // We accept any stride >= w*3.
    let in_bpp = if in_stride >= w * 4 { 4 } else { 3 };
    if plane.data.len() < in_stride * h {
        return Err(Error::invalid("BMP encoder: frame plane truncated (rgb24)"));
    }
    let out_stride = row_stride(w, 24);
    let mut out = vec![0u8; out_stride * h];
    for y in 0..h {
        let src_y = source_row(y, h, options);
        let src = &plane.data[src_y * in_stride..src_y * in_stride + w * in_bpp];
        let dst = &mut out[y * out_stride..y * out_stride + w * 3];
        for x in 0..w {
            let (r, g, b) = (src[x * in_bpp], src[x * in_bpp + 1], src[x * in_bpp + 2]);
            dst[x * 3] = b;
            dst[x * 3 + 1] = g;
            dst[x * 3 + 2] = r;
        }
    }
    Ok((out, out_stride))
}

/// Pack 16-bit RGB565 input (2 bytes/px, little-endian) to row-padded
/// output rows. Row order honours `options.top_down`. Passes through
/// the 16-bit pixels verbatim.
fn pack_rgb565(
    plane: &BmpPlane,
    width: u32,
    height: u32,
    options: BmpEncodeOptions,
) -> Result<(Vec<u8>, usize)> {
    let w = width as usize;
    let h = height as usize;
    let in_stride = plane.stride;
    if plane.data.len() < in_stride * h {
        return Err(Error::invalid(
            "BMP encoder: frame plane truncated (rgb565)",
        ));
    }
    let out_stride = row_stride(w, 16);
    let mut out = vec![0u8; out_stride * h];
    for y in 0..h {
        let src_y = source_row(y, h, options);
        let src = &plane.data[src_y * in_stride..src_y * in_stride + w * 2];
        let dst = &mut out[y * out_stride..y * out_stride + w * 2];
        dst.copy_from_slice(src);
    }
    Ok((out, out_stride))
}

/// Pack indexed pixel data with proper row padding. Row order honours
/// `options.top_down`.
///
/// `bpp` must be 1, 4, or 8.
/// For 8-bit: input is 1 byte per pixel (index 0-255).
/// For 4-bit: input is 1 byte per pixel (index 0-15); packing into
///   hi-nibble/lo-nibble is done here.
/// For 1-bit: input is 1 byte per pixel (index 0 or 1, treated as
///   `byte & 1`); packing into MSB-first bytes is done here.
fn pack_indexed(
    plane: &BmpPlane,
    bpp: usize,
    width: u32,
    height: u32,
    options: BmpEncodeOptions,
) -> Result<(Vec<u8>, usize)> {
    let w = width as usize;
    let h = height as usize;
    let in_stride = plane.stride;
    if plane.data.len() < in_stride * h {
        return Err(Error::invalid(
            "BMP encoder: frame plane truncated (indexed)",
        ));
    }
    let out_stride = row_stride(w, bpp);
    let mut out = vec![0u8; out_stride * h];
    for y in 0..h {
        let src_y = source_row(y, h, options);
        let src = &plane.data[src_y * in_stride..src_y * in_stride + w];
        let dst = &mut out[y * out_stride..];
        match bpp {
            8 => {
                dst[..w].copy_from_slice(src);
            }
            4 => {
                for x in 0..w {
                    let idx = src[x] & 0x0F;
                    if x & 1 == 0 {
                        dst[x / 2] = idx << 4;
                    } else {
                        dst[x / 2] |= idx;
                    }
                }
            }
            1 => {
                for x in 0..w {
                    let bit = src[x] & 1;
                    if bit != 0 {
                        dst[x / 8] |= 1 << (7 - (x % 8));
                    }
                }
            }
            _ => return Err(Error::invalid("pack_indexed: bpp must be 1, 4, or 8")),
        }
    }
    Ok((out, out_stride))
}

// ---------------------------------------------------------------------------
// RLE encoders
// ---------------------------------------------------------------------------

/// RLE8 encoder. Input is bottom-up raw indexed rows (4-byte padded).
/// Output is the BI_RLE8 stream (EOL + EOB terminators).
fn rle8_encode(raw: &[u8], width: u32, height: u32) -> Vec<u8> {
    let w = width as usize;
    let h = height as usize;
    let in_stride = row_stride(w, 8);
    let mut out = Vec::new();

    for y in 0..h {
        let row = &raw[y * in_stride..y * in_stride + w];
        encode_rle8_row(&mut out, row);
        if y + 1 < h {
            // End of line
            out.push(0x00);
            out.push(0x00);
        }
    }
    // End of bitmap
    out.push(0x00);
    out.push(0x01);
    out
}

fn encode_rle8_row(out: &mut Vec<u8>, row: &[u8]) {
    let mut i = 0;
    let n = row.len();
    while i < n {
        // Count run of same byte.
        let val = row[i];
        let mut run = 1usize;
        while i + run < n && run < 255 && row[i + run] == val {
            run += 1;
        }
        if run >= 2 {
            // Encoded run: count, value
            out.push(run as u8);
            out.push(val);
            i += run;
        } else {
            // Absolute mode: find how many non-repeating bytes ahead.
            let start = i;
            let mut abs_len = 1usize;
            while i + abs_len < n && abs_len < 255 {
                // Peek ahead: if next 2+ are a run, break.
                let j = i + abs_len;
                if j + 1 < n && row[j] == row[j + 1] {
                    break;
                }
                abs_len += 1;
            }
            // Absolute mode needs >= 3 bytes to be worthwhile (overhead = 2
            // bytes escape + count + pad). For < 3 just emit single encoded
            // runs of 1.
            if abs_len < 3 {
                out.push(1);
                out.push(val);
                i += 1;
            } else {
                out.push(0x00);
                out.push(abs_len as u8);
                out.extend_from_slice(&row[start..start + abs_len]);
                // Absolute mode payload padded to even length.
                if abs_len & 1 != 0 {
                    out.push(0x00);
                }
                i += abs_len;
            }
        }
    }
}

/// RLE4 encoder. Input is bottom-up raw nibble-packed rows (4-byte padded).
/// Output is the BI_RLE4 stream.
fn rle4_encode(raw: &[u8], width: u32, height: u32) -> Vec<u8> {
    let w = width as usize;
    let h = height as usize;
    let in_stride = row_stride(w, 4);
    let mut out = Vec::new();

    for y in 0..h {
        let packed_row = &raw[y * in_stride..y * in_stride + w.div_ceil(2)];
        // Unpack nibbles for easier processing.
        let mut nibbles: Vec<u8> = Vec::with_capacity(w);
        for x in 0..w {
            let byte = packed_row[x / 2];
            let nib = if x & 1 == 0 { byte >> 4 } else { byte & 0x0F };
            nibbles.push(nib);
        }
        encode_rle4_row(&mut out, &nibbles);
        if y + 1 < h {
            // End of line
            out.push(0x00);
            out.push(0x00);
        }
    }
    // End of bitmap
    out.push(0x00);
    out.push(0x01);
    out
}

fn encode_rle4_row(out: &mut Vec<u8>, nibbles: &[u8]) {
    let mut i = 0;
    let n = nibbles.len();
    while i < n {
        let v0 = nibbles[i];
        // Count run of pairs (RLE4 encodes pairs of nibbles).
        // A run of the same nibble value.
        let mut run = 1usize;
        while i + run < n && run < 255 && nibbles[i + run] == v0 {
            run += 1;
        }
        if run >= 2 {
            // Encoded run: count byte, then two nibbles packed (both same).
            out.push(run as u8);
            out.push((v0 << 4) | v0);
            i += run;
        } else {
            // Absolute mode: collect non-repeating nibbles.
            let start = i;
            let mut abs_len = 1usize;
            while i + abs_len < n && abs_len < 255 {
                let j = i + abs_len;
                if j + 1 < n && nibbles[j] == nibbles[j + 1] {
                    break;
                }
                abs_len += 1;
            }
            if abs_len < 3 {
                out.push(1);
                let v1 = if i + 1 < n { nibbles[i + 1] } else { 0 };
                out.push((v0 << 4) | v1);
                i += 1;
            } else {
                out.push(0x00);
                out.push(abs_len as u8);
                // Pack nibbles into bytes.
                for k in (0..abs_len).step_by(2) {
                    let hi = nibbles[start + k];
                    let lo = if k + 1 < abs_len {
                        nibbles[start + k + 1]
                    } else {
                        0
                    };
                    out.push((hi << 4) | lo);
                }
                // Absolute mode payload in RLE4 is padded to even number of
                // bytes (i.e. to a 4-nibble boundary → even byte count).
                let packed_bytes = abs_len.div_ceil(2);
                if packed_bytes & 1 != 0 {
                    out.push(0x00);
                }
                i += abs_len;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Header writers
// ---------------------------------------------------------------------------

fn write_file_header(out: &mut Vec<u8>, file_size: u32, pixel_offset: u32) {
    out.extend_from_slice(&BMP_MAGIC.to_le_bytes());
    out.extend_from_slice(&file_size.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes()); // reserved1
    out.extend_from_slice(&0u16.to_le_bytes()); // reserved2
    out.extend_from_slice(&pixel_offset.to_le_bytes());
}

/// Write a 40-byte BITMAPINFOHEADER (V3) for direct-colour or indexed
/// (non-bitfields) BMPs. Palette entries follow immediately for indexed.
///
/// `stored_height` is the signed BMP `biHeight` — positive for bottom-up
/// layouts, negative for top-down. `image_size` 0 is valid for
/// uncompressed `BI_RGB`; pass the actual compressed byte count for RLE.
fn write_dib_header_v3(
    out: &mut Vec<u8>,
    w: u32,
    stored_height: i32,
    bpp: u16,
    compression: u32,
    image_size: u32,
) {
    out.extend_from_slice(&BITMAPINFOHEADER_SIZE.to_le_bytes());
    out.extend_from_slice(&(w as i32).to_le_bytes());
    out.extend_from_slice(&stored_height.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // planes
    out.extend_from_slice(&bpp.to_le_bytes());
    out.extend_from_slice(&compression.to_le_bytes());
    out.extend_from_slice(&image_size.to_le_bytes());
    out.extend_from_slice(&2835i32.to_le_bytes()); // x_pels/m ≈ 72 DPI
    out.extend_from_slice(&2835i32.to_le_bytes()); // y_pels/m
    out.extend_from_slice(&0u32.to_le_bytes()); // clr_used
    out.extend_from_slice(&0u32.to_le_bytes()); // clr_important
}

/// Write a 40-byte V3 header followed immediately by the colour table.
///
/// `entries` is the number of `RGBQUAD` colour-table entries to write
/// (computed by [`written_palette_entries`]). When it equals the full
/// `2^bpp` count the header leaves `biClrUsed = 0` — the classic
/// "full table" sentinel that prior output used. When it is smaller
/// (minimal-palette mode) the exact count is recorded in `biClrUsed`
/// so the decoder's `palette_entries()` reads back only that many
/// entries. Either way the colour table written matches the count the
/// header advertises, so the pixel offset stays correct.
/// `stored_height` is the signed BMP `biHeight`.
fn write_dib_header_v3_indexed(
    out: &mut Vec<u8>,
    w: u32,
    stored_height: i32,
    bpp: u16,
    compression: u32,
    palette: &BmpPalette,
    entries: usize,
) {
    let image_size: u32 = 0; // valid for BI_RGB; RLE size is embedded in the pixel data
    let full = palette_entry_count(bpp);
    // A full table advertises clr_used = 0 (the spec's "all 2^bpp"
    // sentinel); a shorter table advertises its exact length.
    let clr_used = if entries >= full { 0 } else { entries as u32 };
    out.extend_from_slice(&BITMAPINFOHEADER_SIZE.to_le_bytes());
    out.extend_from_slice(&(w as i32).to_le_bytes());
    out.extend_from_slice(&stored_height.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // planes
    out.extend_from_slice(&bpp.to_le_bytes());
    out.extend_from_slice(&compression.to_le_bytes());
    out.extend_from_slice(&image_size.to_le_bytes());
    out.extend_from_slice(&2835i32.to_le_bytes());
    out.extend_from_slice(&2835i32.to_le_bytes());
    out.extend_from_slice(&clr_used.to_le_bytes()); // 0 → full 2^bpp; else exact count
    out.extend_from_slice(&0u32.to_le_bytes()); // clr_important
                                                // Colour table: on-disk order is B, G, R, 0x00.
    for i in 0..entries {
        if let Some(rgb) = palette.entries.get(i) {
            out.push(rgb[2]); // B
            out.push(rgb[1]); // G
            out.push(rgb[0]); // R
            out.push(0x00);
        } else {
            out.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]);
        }
    }
}

/// Write a 108-byte BITMAPV4HEADER for 16-bit BI_BITFIELDS RGB 5-6-5.
///
/// Canonical masks: R=0xF800, G=0x07E0, B=0x001F, A=0x0000.
/// CS type set to LCS_sRGB (0x73524742) with all endpoints + gamma zero.
/// `stored_height` is signed: negative for a top-down DIB.
fn write_dib_header_v4_bitfields(out: &mut Vec<u8>, w: u32, stored_height: i32) {
    let pixel_bytes = row_stride(w as usize, 16) as u32 * stored_height.unsigned_abs();
    // Header size
    out.extend_from_slice(&BITMAPV4HEADER_SIZE.to_le_bytes());
    out.extend_from_slice(&(w as i32).to_le_bytes());
    out.extend_from_slice(&stored_height.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // planes
    out.extend_from_slice(&16u16.to_le_bytes()); // bpp
    out.extend_from_slice(&BI_BITFIELDS.to_le_bytes()); // compression
    out.extend_from_slice(&pixel_bytes.to_le_bytes()); // image size
    out.extend_from_slice(&2835i32.to_le_bytes()); // x_pels/m
    out.extend_from_slice(&2835i32.to_le_bytes()); // y_pels/m
    out.extend_from_slice(&0u32.to_le_bytes()); // clr_used
    out.extend_from_slice(&0u32.to_le_bytes()); // clr_important
                                                // V4 extension: R/G/B/A masks (offsets 40-55)
    out.extend_from_slice(&0xF800u32.to_le_bytes()); // R mask
    out.extend_from_slice(&0x07E0u32.to_le_bytes()); // G mask
    out.extend_from_slice(&0x001Fu32.to_le_bytes()); // B mask
    out.extend_from_slice(&0x0000u32.to_le_bytes()); // A mask (no alpha)
                                                     // CS type: LCS_sRGB = 0x73524742
    out.extend_from_slice(&0x7352_4742u32.to_le_bytes());
    // CIEXYZTRIPLE (9 × i32 = 36 bytes) — all zero for sRGB
    out.extend_from_slice(&[0u8; 36]);
    // GammaRed, GammaGreen, GammaBlue (3 × u32 = 12 bytes) — zero
    out.extend_from_slice(&[0u8; 12]);
    // Total so far: 40 + 4*4 + 4 + 36 + 12 = 40+16+4+36+12 = 108 ✓
}

// ---------------------------------------------------------------------------
// AND mask (ICO)
// ---------------------------------------------------------------------------

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
        BmpPixelFormat::Rgb24 | BmpPixelFormat::Rgb565 => {
            // No alpha → fully opaque → all-zero AND mask. Short-circuit.
            return Ok(mask);
        }
        BmpPixelFormat::Indexed8 | BmpPixelFormat::Indexed4 | BmpPixelFormat::Indexed1 => {
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

// ---------------------------------------------------------------------------
// Utilities
// ---------------------------------------------------------------------------

/// Number of palette entries written for a given bit depth.
fn palette_entry_count(bpp: u16) -> usize {
    match bpp {
        1 => 2,
        4 => 16,
        8 => 256,
        _ => 0,
    }
}
