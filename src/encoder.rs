//! BMP + DIB encode.
//!
//! Supported output variants:
//!
//! | Format                | Compression  | Header |
//! | --------------------- | ------------ | ------ |
//! | 32-bit BGRA           | `BI_RGB`     | V3     |
//! | 24-bit BGR            | `BI_RGB`     | V3     |
//! | 16-bit RGB 5-5-5      | `BI_RGB`     | V3     |
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
//! [`BmpPixelFormat::Rgb555`] is written as 16-bit BI_RGB 5-5-5 (V3 header).
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
    /// 16-bit `BI_RGB` RGB 5-5-5.
    Rgb16Rgb,
    /// 16-bit `BI_BITFIELDS` RGB 5-6-5.
    Rgb16Bitfields,
    /// 8-bit uncompressed indexed `BI_RGB`.
    Indexed8,
    /// 4-bit uncompressed indexed `BI_RGB`.
    Indexed4,
    /// 2-bit uncompressed indexed `BI_RGB` (Windows CE 4-colour).
    Indexed2,
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
        BmpPixelFormat::Rgb555 => {
            let bytes = encode_rgb555(plane, width, height, options)?;
            Ok((bytes, EncodedBmpFormat::Rgb16Rgb))
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
        BmpPixelFormat::Indexed2 => {
            let pal = palette
                .ok_or_else(|| Error::invalid("BMP encoder: Indexed2 requires a palette"))?;
            let (raw_pixels, _) = pack_indexed(plane, 2, width, height, options)?;
            let file = build_indexed_bmp(width, height, 2, BI_RGB, pal, &raw_pixels, options);
            Ok((file, EncodedBmpFormat::Indexed2))
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

// ---------------------------------------------------------------------------
// Explicit-mask BI_BITFIELDS / BI_ALPHABITFIELDS encoder (V3 header)
// ---------------------------------------------------------------------------

/// Per-channel bit masks for an explicit-mask `BI_BITFIELDS` /
/// `BI_ALPHABITFIELDS` BMP.
///
/// `r` / `g` / `b` / `a` are the 32-bit DWORD masks selecting each
/// channel's bits inside the packed pixel word. `bpp` is the on-disk
/// bit depth (16 or 32). When `a` is zero the encoder writes a 12-byte
/// (three-mask) `BI_BITFIELDS` tail and decoders treat every pixel as
/// opaque; a non-zero `a` writes a 16-byte (four-mask)
/// `BI_ALPHABITFIELDS` tail so the alpha channel survives the round-trip.
///
/// The masks for each channel must be a single contiguous run of set
/// bits (the BMP `BI_BITFIELDS` mechanism is shift-and-scale, not an
/// arbitrary bit permutation); the masks must not overlap and must fit
/// inside `bpp` bits. [`BmpBitfields::validate`] enforces this.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BmpBitfields {
    /// On-disk bit depth: 16 or 32.
    pub bpp: u16,
    /// Red-channel DWORD mask.
    pub r: u32,
    /// Green-channel DWORD mask.
    pub g: u32,
    /// Blue-channel DWORD mask.
    pub b: u32,
    /// Alpha-channel DWORD mask. Zero ⇒ no alpha (three-mask
    /// `BI_BITFIELDS`); non-zero ⇒ four-mask `BI_ALPHABITFIELDS`.
    pub a: u32,
}

impl BmpBitfields {
    /// 16-bit RGB 5-6-5: R bits 15..11, G bits 10..5, B bits 4..0, no
    /// alpha. The canonical Windows/`BI_BITFIELDS` 16-bpp layout.
    pub const RGB565: Self = Self {
        bpp: 16,
        r: 0xF800,
        g: 0x07E0,
        b: 0x001F,
        a: 0x0000,
    };
    /// 16-bit RGB 5-5-5: R bits 14..10, G bits 9..5, B bits 4..0, bit 15
    /// unused, no alpha.
    pub const RGB555: Self = Self {
        bpp: 16,
        r: 0x7C00,
        g: 0x03E0,
        b: 0x001F,
        a: 0x0000,
    };
    /// 16-bit ARGB 1-5-5-5: A bit 15, R bits 14..10, G bits 9..5, B bits
    /// 4..0. The 16-bpp four-mask `BI_ALPHABITFIELDS` layout.
    pub const ARGB1555: Self = Self {
        bpp: 16,
        r: 0x7C00,
        g: 0x03E0,
        b: 0x001F,
        a: 0x8000,
    };
    /// 32-bit BGRA 8-8-8-8: B bits 7..0, G bits 15..8, R bits 23..16,
    /// A bits 31..24 — the byte-aligned alpha-carrying layout. Every
    /// channel is a full 8-bit run, so the round-trip is bit-exact.
    pub const BGRA8888: Self = Self {
        bpp: 32,
        r: 0x00FF_0000,
        g: 0x0000_FF00,
        b: 0x0000_00FF,
        a: 0xFF00_0000,
    };
    /// 32-bit BGRX 8-8-8-8: same channel layout as [`Self::BGRA8888`]
    /// but the top byte is unused (no alpha mask) — a three-mask
    /// `BI_BITFIELDS` 32-bpp bitmap. Colour is bit-exact; alpha is
    /// dropped (decoders read every pixel as opaque).
    pub const BGRX8888: Self = Self {
        bpp: 32,
        r: 0x00FF_0000,
        g: 0x0000_FF00,
        b: 0x0000_00FF,
        a: 0x0000_0000,
    };

    /// `true` when an alpha mask is present (a four-mask
    /// `BI_ALPHABITFIELDS` tail is required).
    pub fn has_alpha(&self) -> bool {
        self.a != 0
    }

    /// The `biCompression` value this mask set declares: `BI_ALPHABITFIELDS`
    /// when an alpha mask is present, else `BI_BITFIELDS`.
    pub fn compression(&self) -> u32 {
        if self.has_alpha() {
            BI_ALPHABITFIELDS
        } else {
            BI_BITFIELDS
        }
    }

    /// Validate the mask set: `bpp` must be 16 or 32, each non-zero mask
    /// must be a single contiguous bit run, masks must not overlap, and
    /// every mask bit must fit inside `bpp` bits.
    pub fn validate(&self) -> Result<()> {
        if self.bpp != 16 && self.bpp != 32 {
            return Err(Error::invalid(format!(
                "BMP bitfields: bpp must be 16 or 32, got {}",
                self.bpp
            )));
        }
        let limit: u32 = if self.bpp == 32 {
            0xFFFF_FFFF
        } else {
            (1u32 << self.bpp) - 1
        };
        let channels = [("R", self.r), ("G", self.g), ("B", self.b), ("A", self.a)];
        let mut union: u32 = 0;
        for (name, mask) in channels {
            if mask == 0 {
                continue; // a zero mask means "channel absent"
            }
            if mask & !limit != 0 {
                return Err(Error::invalid(format!(
                    "BMP bitfields: {name} mask {mask:#010x} exceeds {} bits",
                    self.bpp
                )));
            }
            // Contiguous run check: a value whose set bits are contiguous
            // satisfies `m & (m + lsb) == 0` where lsb isolates the low set
            // bit — equivalently `(m >> tz)` is one less than a power of two.
            let normalised = mask >> mask.trailing_zeros();
            if normalised & (normalised + 1) != 0 {
                return Err(Error::invalid(format!(
                    "BMP bitfields: {name} mask {mask:#010x} is not a contiguous bit run"
                )));
            }
            if union & mask != 0 {
                return Err(Error::invalid(format!(
                    "BMP bitfields: {name} mask {mask:#010x} overlaps another channel"
                )));
            }
            union |= mask;
        }
        if self.r == 0 && self.g == 0 && self.b == 0 {
            return Err(Error::invalid(
                "BMP bitfields: at least one of R/G/B must be non-zero",
            ));
        }
        Ok(())
    }
}

/// Encode a [`BmpImage`] as an explicit-mask `BI_BITFIELDS` /
/// `BI_ALPHABITFIELDS` BMP using a 40-byte `BITMAPINFOHEADER` (V3) with
/// the per-channel masks written as a 12-byte (RGB) or 16-byte (RGBA)
/// tail immediately after the header — the classic Windows in-file mask
/// layout, distinct from the V4/V5 in-header mask block the
/// [`encode_bmp`] `Rgb565` path emits.
///
/// The source plane must be [`BmpPixelFormat::Rgba`] or
/// [`BmpPixelFormat::Rgb24`]; each 8-bit channel is requantised down to
/// the width of its mask and shifted into place. For a byte-aligned
/// 32-bpp mask set ([`BmpBitfields::BGRA8888`] /
/// [`BmpBitfields::BGRX8888`]) every channel keeps its full 8 bits so the
/// round-trip through [`decode_bmp`] is bit-exact.
///
/// [`BmpEncodeOptions::top_down`] is honoured (negative `biHeight`);
/// `minimal_palette` is irrelevant to direct-colour bitfields.
pub fn encode_bmp_bitfields(
    image: &BmpImage,
    masks: BmpBitfields,
    options: BmpEncodeOptions,
) -> Result<Vec<u8>> {
    if image.planes.is_empty() {
        return Err(Error::invalid("BMP encoder: empty frame plane"));
    }
    encode_bmp_plane_bitfields(
        &image.planes[0],
        image.pixel_format,
        masks,
        image.width,
        image.height,
        options,
    )
}

/// Plane-level variant of [`encode_bmp_bitfields`].
pub fn encode_bmp_plane_bitfields(
    plane: &BmpPlane,
    format: BmpPixelFormat,
    masks: BmpBitfields,
    width: u32,
    height: u32,
    options: BmpEncodeOptions,
) -> Result<Vec<u8>> {
    masks.validate()?;
    let pixels = pack_bitfields(plane, format, masks, width, height, options)?;
    let mask_tail = if masks.has_alpha() { 16u32 } else { 12u32 };
    let pixel_bytes = pixels.len() as u32;
    let header_and_tail = BITMAPINFOHEADER_SIZE + mask_tail;
    let pixel_offset = BITMAPFILEHEADER_SIZE + header_and_tail;
    let file_size = pixel_offset + pixel_bytes;
    let mut out = Vec::with_capacity(file_size as usize);
    write_file_header(&mut out, file_size, pixel_offset);
    write_dib_header_v3(
        &mut out,
        width,
        signed_stored_height(height, options),
        masks.bpp,
        masks.compression(),
        0,
    );
    // V3 mask tail: 3 DWORDs (R/G/B) for BI_BITFIELDS, 4 (R/G/B/A) for
    // BI_ALPHABITFIELDS, immediately after the 40-byte header.
    out.extend_from_slice(&masks.r.to_le_bytes());
    out.extend_from_slice(&masks.g.to_le_bytes());
    out.extend_from_slice(&masks.b.to_le_bytes());
    if masks.has_alpha() {
        out.extend_from_slice(&masks.a.to_le_bytes());
    }
    out.extend_from_slice(&pixels);
    Ok(out)
}

/// Pack `Rgba` / `Rgb24` source pixels into the on-disk word layout
/// declared by `masks`, with 4-byte-aligned rows and the row order
/// `options.top_down` requests.
///
/// Each source channel is an 8-bit sample. It is requantised to the bit
/// width of its mask (`v >> (8 - n)`, the inverse of the decoder's
/// `expand` shift-and-scale) and shifted up by the mask's trailing-zero
/// count. A zero mask drops the channel. `Rgb24` input has no alpha, so
/// an alpha mask receives the fully-opaque value `(1 << n) - 1`.
fn pack_bitfields(
    plane: &BmpPlane,
    format: BmpPixelFormat,
    masks: BmpBitfields,
    width: u32,
    height: u32,
    options: BmpEncodeOptions,
) -> Result<Vec<u8>> {
    let w = width as usize;
    let h = height as usize;
    let in_stride = plane.stride;
    let in_bpp = match format {
        BmpPixelFormat::Rgba => 4,
        BmpPixelFormat::Rgb24 => 3,
        other => {
            return Err(Error::invalid(format!(
                "BMP bitfields: source must be Rgba or Rgb24, got {other:?}"
            )))
        }
    };
    if plane.data.len() < in_stride * h {
        return Err(Error::invalid(
            "BMP encoder: frame plane truncated (bitfields)",
        ));
    }
    let out_bpp_bytes = (masks.bpp / 8) as usize;
    let out_stride = row_stride(w, masks.bpp as usize);
    let (rs, rn) = mask_shift_len(masks.r);
    let (gs, gn) = mask_shift_len(masks.g);
    let (bs, bn) = mask_shift_len(masks.b);
    let (as_, an) = mask_shift_len(masks.a);
    let mut out = vec![0u8; out_stride * h];
    for y in 0..h {
        let src_y = source_row(y, h, options);
        let src = &plane.data[src_y * in_stride..src_y * in_stride + w * in_bpp];
        let dst = &mut out[y * out_stride..y * out_stride + w * out_bpp_bytes];
        for x in 0..w {
            let r = src[x * in_bpp];
            let g = src[x * in_bpp + 1];
            let b = src[x * in_bpp + 2];
            let a = if in_bpp == 4 {
                src[x * in_bpp + 3]
            } else {
                0xFF
            };
            let mut word: u32 = 0;
            if rn > 0 {
                word |= (quantise(r, rn) as u32) << rs;
            }
            if gn > 0 {
                word |= (quantise(g, gn) as u32) << gs;
            }
            if bn > 0 {
                word |= (quantise(b, bn) as u32) << bs;
            }
            if an > 0 {
                word |= (quantise(a, an) as u32) << as_;
            }
            let bytes = word.to_le_bytes();
            dst[x * out_bpp_bytes..x * out_bpp_bytes + out_bpp_bytes]
                .copy_from_slice(&bytes[..out_bpp_bytes]);
        }
    }
    Ok(out)
}

/// Trailing-zero shift and run-length of a single-run channel mask.
/// `(0, 0)` for a zero mask (absent channel).
fn mask_shift_len(mask: u32) -> (u32, u32) {
    if mask == 0 {
        return (0, 0);
    }
    let shift = mask.trailing_zeros();
    let len = 32 - mask.leading_zeros() - shift;
    (shift, len)
}

/// Requantise an 8-bit sample to an `n`-bit channel run.
///
/// * `n == 8` — identity, so byte-aligned masks are bit-exact.
/// * `n < 8`  — drop the low `8 - n` bits (the inverse of the decoder's
///   `expand`, which scales an `n`-bit sample back up to 8).
/// * `n > 8`  — left-justify the 8 bits into the `n`-bit run and
///   replicate the high bits into the freed low bits, so a full-scale
///   input (`0xFF`) maps to a full-scale `n`-bit value rather than
///   leaving the low bits dark. Only reachable for a 32-bpp mask wider
///   than a byte, which `validate` permits.
fn quantise(sample: u8, n: u32) -> u32 {
    let s = sample as u32;
    match n.cmp(&8) {
        core::cmp::Ordering::Equal => s,
        core::cmp::Ordering::Less => s >> (8 - n),
        core::cmp::Ordering::Greater => {
            let extra = n - 8;
            ((s << extra) | (s >> (8 - extra).min(8))) & ((1u32 << n) - 1)
        }
    }
}

/// Encode a `BmpImage` into a complete BMP file with a V5 header that
/// declares an embedded ICC profile (`PROFILE_EMBEDDED`).
///
/// The V5 header (124 bytes) replaces the V3/V4 header the standard
/// [`encode_bmp`] path would emit; `bV5CSType` is set to
/// [`PROFILE_EMBEDDED`], `bV5ProfileData` points at the byte offset of
/// the ICC blob from the start of the DIB (i.e. immediately after the
/// pixel array), and `bV5ProfileSize` records the blob's length. The
/// ICC bytes are written verbatim — no parsing or validation happens on
/// the encoder side; the caller is responsible for supplying a real ICC
/// 1.x / 2.x / 4.x profile.
///
/// Supported pixel formats:
///
/// * Direct-colour: [`BmpPixelFormat::Rgba`] (32-bit BGRA),
///   [`BmpPixelFormat::Rgb24`] (24-bit BGR), [`BmpPixelFormat::Rgb555`]
///   (16-bit `BI_RGB` 5-5-5 — high bit reserved, no mask block; the V5
///   header's four-mask region stays zero), and [`BmpPixelFormat::Rgb565`]
///   (16-bit `BI_BITFIELDS` 5-6-5 — the V5 header's four-mask region at
///   offsets 40..56 carries the canonical R=0xF800 / G=0x07E0 / B=0x001F
///   quadruple so no separate 12-byte mask tail is written before the
///   pixel array).
/// * Indexed: [`BmpPixelFormat::Indexed8`], [`BmpPixelFormat::Indexed4`],
///   [`BmpPixelFormat::Indexed1`] — emitted always as uncompressed
///   `BI_RGB` (V5 + RLE is not a documented pairing), with the colour
///   table written between the 124-byte V5 header and the pixel array
///   and `biClrUsed` set per [`BmpEncodeOptions::minimal_palette`]. The
///   ICC blob still rides at `bV5ProfileData` immediately after the
///   pixel array; `bfOffBits` skips the V5 header + palette to point at
///   the pixels exactly the same way a V3 indexed BMP advertises its
///   pixel-array offset.
///
/// `rendering_intent` is the V5 `bV5Intent` field. Use 0 for
/// "unspecified" or one of the [`LCS_GM_*`](crate::LCS_GM_BUSINESS)
/// constants. The encoder honours [`BmpEncodeOptions::top_down`]
/// (negative `biHeight`) on every path and honours
/// [`BmpEncodeOptions::minimal_palette`] on the indexed paths.
///
/// Roundtrips: [`decode_bmp_with_metadata`](crate::decode_bmp_with_metadata)
/// reads the ICC blob back into [`BmpMetadata::icc_profile`](crate::BmpMetadata::icc_profile).
pub fn encode_bmp_with_icc_profile(
    image: &BmpImage,
    icc_profile: &[u8],
    rendering_intent: u32,
    options: BmpEncodeOptions,
) -> Result<Vec<u8>> {
    if image.planes.is_empty() {
        return Err(Error::invalid("BMP encoder: empty frame plane"));
    }
    if image.pixel_format.is_indexed() {
        return encode_bmp_v5_indexed_with_profile_blob(
            image,
            icc_profile,
            PROFILE_EMBEDDED,
            rendering_intent,
            options,
        );
    }
    let plane = &image.planes[0];
    let width = image.width;
    let height = image.height;
    let (pixels, bpp, compression, masks) = match image.pixel_format {
        BmpPixelFormat::Rgba => {
            let (p, _) = pack_rgba(plane, image.pixel_format, width, height, options)?;
            (p, 32u16, BI_RGB, RGBA32_ALPHA_MASK_V4_V5)
        }
        BmpPixelFormat::Rgb24 => {
            let (p, _) = pack_rgb24(plane, width, height, options)?;
            (p, 24u16, BI_RGB, [0u32; 4])
        }
        BmpPixelFormat::Rgb555 => {
            let (p, _) = pack_rgb555(plane, width, height, options)?;
            (p, 16u16, BI_RGB, [0u32; 4])
        }
        BmpPixelFormat::Rgb565 => {
            let (p, _) = pack_rgb565(plane, width, height, options)?;
            (p, 16u16, BI_BITFIELDS, RGB565_MASKS_V5)
        }
        other => {
            return Err(Error::unsupported(format!(
                "BMP encoder: V5 + ICC profile not yet supported for {other:?}",
            )))
        }
    };
    // Layout:
    //   [BITMAPFILEHEADER 14 B]
    //   [BITMAPV5HEADER 124 B]   ← `cs_type = PROFILE_EMBEDDED`
    //   [pixel array]
    //   [ICC blob, `icc_profile.len()` bytes]
    //
    // `bV5ProfileData` is DIB-relative (i.e. 124 + pixel_bytes), and
    // `bV5ProfileSize` is the ICC blob length. The on-disk pixel
    // offset that `BITMAPFILEHEADER::bfOffBits` carries skips the V5
    // header but never the profile blob, since the blob lives after
    // the pixel array. The 16-bpp BI_BITFIELDS arm keeps the same
    // shape: the four-mask region at offsets 40..56 of the V5 header
    // already carries R / G / B / A so the decoder reads the masks
    // out of the header body and never needs a 12-byte tail before
    // the pixel array.
    let pixel_bytes = pixels.len() as u32;
    let icc_bytes = icc_profile.len() as u32;
    let dib_size = BITMAPV5HEADER_SIZE + pixel_bytes + icc_bytes;
    let file_size = BITMAPFILEHEADER_SIZE + dib_size;
    let pixel_offset = BITMAPFILEHEADER_SIZE + BITMAPV5HEADER_SIZE;
    let profile_data_offset = BITMAPV5HEADER_SIZE + pixel_bytes;

    let mut out = Vec::with_capacity(file_size as usize);
    write_file_header(&mut out, file_size, pixel_offset);
    write_dib_header_v5_with_profile(
        &mut out,
        width,
        signed_stored_height(height, options),
        bpp,
        pixel_bytes,
        compression,
        masks,
        PROFILE_EMBEDDED,
        rendering_intent,
        profile_data_offset,
        icc_bytes,
    );
    out.extend_from_slice(&pixels);
    out.extend_from_slice(icc_profile);
    Ok(out)
}

/// Encode a `BmpImage` into a complete BMP file with a V5 header that
/// declares a *linked* ICC profile (`PROFILE_LINKED`) — the V5 header
/// records the offset + length of a caller-supplied path-string blob
/// rather than embedding the ICC bytes themselves.
///
/// The V5 layout matches [`encode_bmp_with_icc_profile`] byte-for-byte
/// except that `bV5CSType` is set to [`PROFILE_LINKED`] (the four-byte
/// `'LINK'` tag) and the blob that lives at
/// `bV5ProfileData / bV5ProfileSize` carries the path of an external
/// ICC profile file rather than the profile bytes themselves. The path
/// encoding is system-dependent per the BMP spec — typically null-
/// terminated ANSI on Windows; we surface the buffer the caller supplies
/// verbatim so callers that need a different transport (UTF-16, URL)
/// can pass whatever blob they choose. The decoder side already
/// distinguishes `PROFILE_LINKED` from `PROFILE_EMBEDDED` in
/// [`BmpMetadata::color_space`](crate::BmpMetadata::color_space) and
/// surfaces the `profile_data_offset` / `profile_size` fields so the
/// caller can resolve the path itself; the decoder never auto-loads
/// the linked file.
///
/// Supported pixel formats: every format accepted by
/// [`encode_bmp_with_icc_profile`] — [`BmpPixelFormat::Rgba`],
/// [`BmpPixelFormat::Rgb24`], [`BmpPixelFormat::Rgb555`] (16-bit
/// `BI_RGB` 5-5-5), [`BmpPixelFormat::Rgb565`], plus
/// [`BmpPixelFormat::Indexed8`] / [`BmpPixelFormat::Indexed4`] /
/// [`BmpPixelFormat::Indexed1`] (always written as uncompressed
/// `BI_RGB`, with the colour table sitting between the V5 header and
/// the pixel array). `rendering_intent`,
/// [`BmpEncodeOptions::top_down`], and
/// [`BmpEncodeOptions::minimal_palette`] all have the same meaning as on
/// the embedded path. The linked-path blob is written verbatim; the
/// caller is responsible for the path-string encoding and any null
/// terminator the consumer expects.
pub fn encode_bmp_with_linked_icc_profile(
    image: &BmpImage,
    linked_path: &[u8],
    rendering_intent: u32,
    options: BmpEncodeOptions,
) -> Result<Vec<u8>> {
    if image.planes.is_empty() {
        return Err(Error::invalid("BMP encoder: empty frame plane"));
    }
    if image.pixel_format.is_indexed() {
        return encode_bmp_v5_indexed_with_profile_blob(
            image,
            linked_path,
            PROFILE_LINKED,
            rendering_intent,
            options,
        );
    }
    let plane = &image.planes[0];
    let width = image.width;
    let height = image.height;
    let (pixels, bpp, compression, masks) = match image.pixel_format {
        BmpPixelFormat::Rgba => {
            let (p, _) = pack_rgba(plane, image.pixel_format, width, height, options)?;
            (p, 32u16, BI_RGB, RGBA32_ALPHA_MASK_V4_V5)
        }
        BmpPixelFormat::Rgb24 => {
            let (p, _) = pack_rgb24(plane, width, height, options)?;
            (p, 24u16, BI_RGB, [0u32; 4])
        }
        BmpPixelFormat::Rgb555 => {
            let (p, _) = pack_rgb555(plane, width, height, options)?;
            (p, 16u16, BI_RGB, [0u32; 4])
        }
        BmpPixelFormat::Rgb565 => {
            let (p, _) = pack_rgb565(plane, width, height, options)?;
            (p, 16u16, BI_BITFIELDS, RGB565_MASKS_V5)
        }
        other => {
            return Err(Error::unsupported(format!(
                "BMP encoder: V5 + linked ICC profile not yet supported for {other:?}",
            )))
        }
    };
    // Same layout shape as the PROFILE_EMBEDDED path: the path blob
    // sits where the ICC bytes would. `bV5CSType` distinguishes the
    // two on the wire. The 16-bpp BI_BITFIELDS arm reuses the V5
    // header's four-mask region (offsets 40..56) for the canonical
    // R / G / B / A 5-6-5 layout — no separate 12-byte mask tail.
    let pixel_bytes = pixels.len() as u32;
    let path_bytes = linked_path.len() as u32;
    let dib_size = BITMAPV5HEADER_SIZE + pixel_bytes + path_bytes;
    let file_size = BITMAPFILEHEADER_SIZE + dib_size;
    let pixel_offset = BITMAPFILEHEADER_SIZE + BITMAPV5HEADER_SIZE;
    let profile_data_offset = BITMAPV5HEADER_SIZE + pixel_bytes;

    let mut out = Vec::with_capacity(file_size as usize);
    write_file_header(&mut out, file_size, pixel_offset);
    write_dib_header_v5_with_profile(
        &mut out,
        width,
        signed_stored_height(height, options),
        bpp,
        pixel_bytes,
        compression,
        masks,
        PROFILE_LINKED,
        rendering_intent,
        profile_data_offset,
        path_bytes,
    );
    out.extend_from_slice(&pixels);
    out.extend_from_slice(linked_path);
    Ok(out)
}

/// Encode a [`BmpImage`] into a complete BMP file carrying a 108-byte
/// `BITMAPV4HEADER` with `bV4CSType = LCS_CALIBRATED_RGB` and the
/// caller-supplied CIE endpoints + per-channel gamma.
///
/// `LCS_CALIBRATED_RGB` (value `0`) is the V4 colour-space mode in
/// which the endpoint + gamma fields — rather than a named colour
/// space (`LCS_sRGB`, …) or an embedded/linked ICC profile — define
/// the bitmap's colour. Per the `BITMAPV4HEADER` documentation the
/// `bV4Endpoints` `CIEXYZTRIPLE` carries the x / y / z coordinates of
/// the red, green, and blue endpoints (nine `FXPT2DOT30` `LONG`s,
/// packed R.x R.y R.z G.x G.y G.z B.x B.y B.z) and each of
/// `bV4GammaRed` / `bV4GammaGreen` / `bV4GammaBlue` is the tone-
/// response curve in unsigned 16.16 fixed point (upper 16 bits
/// integer, lower 16 bits fraction). Both are ignored by a reader
/// unless `bV4CSType == LCS_CALIBRATED_RGB`; this entry point always
/// writes that tag.
///
/// `endpoints` is the nine-`i32` `CIEXYZTRIPLE` in the same packing
/// the decoder surfaces as [`BmpMetadata::endpoints`](crate::BmpMetadata::endpoints);
/// `gamma_rgb` is the `[GammaRed, GammaGreen, GammaBlue]` triple the
/// decoder surfaces as [`BmpMetadata::gamma_rgb`](crate::BmpMetadata::gamma_rgb).
/// A caller that only wants to *tag* the bitmap as calibrated without
/// asserting specific primaries may pass all-zero endpoints + gamma.
///
/// Supported pixel formats: [`BmpPixelFormat::Rgba`] (32-bit BGRA
/// `BI_RGB`), [`BmpPixelFormat::Rgb24`] (24-bit BGR `BI_RGB`),
/// [`BmpPixelFormat::Rgb555`] (16-bit `BI_RGB` 5-5-5, high bit reserved,
/// no mask block), [`BmpPixelFormat::Rgb565`] (16-bit `BI_BITFIELDS`
/// 5-6-5, masks in the V4 four-mask region), and the indexed
/// [`BmpPixelFormat::Indexed8`] / `Indexed4` / `Indexed1` (uncompressed
/// `BI_RGB` with the colour table sitting between the V4 header and the
/// pixel array). RLE is never chosen — like the V5 + ICC paths, the
/// V4-calibrated path keeps the pixel array uncompressed so the header
/// shape is deterministic. [`BmpEncodeOptions::top_down`] and
/// [`BmpEncodeOptions::minimal_palette`] have the same meaning as on
/// every other encode path.
///
/// The decoder round-trips this header: `decode_bmp_with_metadata`
/// reports [`BmpColorSpace::Calibrated`](crate::BmpColorSpace::Calibrated)
/// and returns the same endpoints + gamma the encoder was given.
pub fn encode_bmp_with_calibrated_rgb(
    image: &BmpImage,
    endpoints: [i32; 9],
    gamma_rgb: [u32; 3],
    options: BmpEncodeOptions,
) -> Result<Vec<u8>> {
    if image.planes.is_empty() {
        return Err(Error::invalid("BMP encoder: empty frame plane"));
    }
    if image.pixel_format.is_indexed() {
        return encode_bmp_v4_indexed_calibrated(image, endpoints, gamma_rgb, options);
    }
    let plane = &image.planes[0];
    let width = image.width;
    let height = image.height;
    let (pixels, bpp, compression, masks) = match image.pixel_format {
        BmpPixelFormat::Rgba => {
            let (p, _) = pack_rgba(plane, image.pixel_format, width, height, options)?;
            (p, 32u16, BI_RGB, RGBA32_ALPHA_MASK_V4_V5)
        }
        BmpPixelFormat::Rgb24 => {
            let (p, _) = pack_rgb24(plane, width, height, options)?;
            (p, 24u16, BI_RGB, [0u32; 4])
        }
        BmpPixelFormat::Rgb555 => {
            let (p, _) = pack_rgb555(plane, width, height, options)?;
            (p, 16u16, BI_RGB, [0u32; 4])
        }
        BmpPixelFormat::Rgb565 => {
            let (p, _) = pack_rgb565(plane, width, height, options)?;
            (p, 16u16, BI_BITFIELDS, RGB565_MASKS_V5)
        }
        other => {
            return Err(Error::unsupported(format!(
                "BMP encoder: V4 calibrated-RGB not supported for {other:?}",
            )))
        }
    };
    // Layout: [BITMAPFILEHEADER 14 B][BITMAPV4HEADER 108 B][pixel array].
    // The 16-bpp BI_BITFIELDS arm carries the canonical 5-6-5 masks in
    // the V4 four-mask region (offsets 40..56), so no separate 12-byte
    // mask tail sits before the pixel array.
    let pixel_bytes = pixels.len() as u32;
    let file_size = BITMAPFILEHEADER_SIZE + BITMAPV4HEADER_SIZE + pixel_bytes;
    let pixel_offset = BITMAPFILEHEADER_SIZE + BITMAPV4HEADER_SIZE;

    let mut out = Vec::with_capacity(file_size as usize);
    write_file_header(&mut out, file_size, pixel_offset);
    write_dib_header_v4_calibrated(
        &mut out,
        width,
        signed_stored_height(height, options),
        bpp,
        pixel_bytes,
        compression,
        masks,
        0, // clr_used — direct-colour has no colour table
        endpoints,
        gamma_rgb,
    );
    out.extend_from_slice(&pixels);
    Ok(out)
}

/// Indexed arm of [`encode_bmp_with_calibrated_rgb`]
/// ([`BmpPixelFormat::Indexed8`] / `Indexed4` / `Indexed1`).
///
/// Layout (file-byte offsets):
/// * 0..14 `BITMAPFILEHEADER`
/// * 14..122 `BITMAPV4HEADER` (108 B; `bV4CSType = LCS_CALIBRATED_RGB`,
///   four-mask region zeroed, `biClrUsed` reflects the on-disk table)
/// * 122..122+pal `RGBQUAD` colour table (`pal = entries × 4`)
/// * 122+pal..end Packed indexed pixel bytes (uncompressed `BI_RGB`)
///
/// `bfOffBits` is `14 + 108 + pal`. RLE is never used (same rationale
/// as the V5 + ICC indexed path: a deterministic header shape).
fn encode_bmp_v4_indexed_calibrated(
    image: &BmpImage,
    endpoints: [i32; 9],
    gamma_rgb: [u32; 3],
    options: BmpEncodeOptions,
) -> Result<Vec<u8>> {
    let plane = &image.planes[0];
    let width = image.width;
    let height = image.height;
    let bpp: u16 = match image.pixel_format {
        BmpPixelFormat::Indexed8 => 8,
        BmpPixelFormat::Indexed4 => 4,
        BmpPixelFormat::Indexed2 => 2,
        BmpPixelFormat::Indexed1 => 1,
        _ => {
            return Err(Error::invalid(
                "encode_bmp_v4_indexed_calibrated: not an indexed format",
            ));
        }
    };
    let palette = image.palette.as_ref().ok_or_else(|| {
        Error::invalid("BMP encoder: V4 calibrated-RGB indexed input requires a palette")
    })?;
    let entries = written_palette_entries(bpp, palette, options);
    let full = palette_entry_count(bpp);
    let clr_used = if entries >= full { 0 } else { entries as u32 };

    let (pixels, _) = pack_indexed(plane, bpp as usize, width, height, options)?;

    let palette_bytes = (entries * 4) as u32;
    let pixel_bytes = pixels.len() as u32;
    let dib_size = BITMAPV4HEADER_SIZE + palette_bytes + pixel_bytes;
    let file_size = BITMAPFILEHEADER_SIZE + dib_size;
    let pixel_offset = BITMAPFILEHEADER_SIZE + BITMAPV4HEADER_SIZE + palette_bytes;

    let mut out = Vec::with_capacity(file_size as usize);
    write_file_header(&mut out, file_size, pixel_offset);
    write_dib_header_v4_calibrated(
        &mut out,
        width,
        signed_stored_height(height, options),
        bpp,
        pixel_bytes,
        BI_RGB,
        [0u32; 4],
        clr_used,
        endpoints,
        gamma_rgb,
    );
    // Colour table (B, G, R, 0x00) right after the V4 header.
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
    out.extend_from_slice(&pixels);
    Ok(out)
}

/// Shared V5 + profile-blob assembler for the indexed encode paths
/// ([`BmpPixelFormat::Indexed8`] / `Indexed4` / `Indexed1`). Both the
/// embedded (`PROFILE_EMBEDDED`) and linked (`PROFILE_LINKED`) public
/// entry points dispatch here when the input carries a palette; the
/// only thing that changes between the two is the `cs_type` tag and
/// the meaning of the trailing blob (ICC bytes vs path bytes — the
/// encoder treats both as opaque payload).
///
/// Layout (file-byte offsets given relative to start of file):
/// * 0..14 `BITMAPFILEHEADER`
/// * 14..138 `BITMAPV5HEADER` (124 B; `biClrUsed` reflects the on-disk
///   colour-table length)
/// * 138..138+pal `RGBQUAD` colour table (`pal = entries × 4`)
/// * 138+pal..138+pal+pix Packed indexed pixel bytes
/// * 138+pal+pix..end Profile blob (ICC bytes or path bytes)
///
/// `bfOffBits` is `14 + 124 + pal` so consumers reading the V5 BMP
/// land on the pixel array exactly as with a V3 indexed BMP.
/// `bV5ProfileData` is DIB-relative (so `124 + pal + pix`), matching
/// the direct-colour V5 paths. RLE is never used on the V5 paths
/// because the BMP spec doesn't define how an RLE pixel stream and a
/// trailing colour-management blob co-exist on disk; the indexed
/// `top_down = true` direct-colour fall-back already routes through
/// uncompressed `BI_RGB` for the same reason.
fn encode_bmp_v5_indexed_with_profile_blob(
    image: &BmpImage,
    blob: &[u8],
    cs_type: u32,
    rendering_intent: u32,
    options: BmpEncodeOptions,
) -> Result<Vec<u8>> {
    let plane = &image.planes[0];
    let width = image.width;
    let height = image.height;
    let bpp: u16 = match image.pixel_format {
        BmpPixelFormat::Indexed8 => 8,
        BmpPixelFormat::Indexed4 => 4,
        BmpPixelFormat::Indexed2 => 2,
        BmpPixelFormat::Indexed1 => 1,
        _ => {
            return Err(Error::invalid(
                "encode_bmp_v5_indexed_with_profile_blob: not an indexed format",
            ));
        }
    };
    let palette = image
        .palette
        .as_ref()
        .ok_or_else(|| Error::invalid("BMP encoder: V5 + ICC indexed input requires a palette"))?;
    let entries = written_palette_entries(bpp, palette, options);
    let full = palette_entry_count(bpp);
    let clr_used = if entries >= full { 0 } else { entries as u32 };

    // Pack pixel bytes (honours `top_down` via `source_row`).
    let (pixels, _) = pack_indexed(plane, bpp as usize, width, height, options)?;

    let palette_bytes = (entries * 4) as u32;
    let pixel_bytes = pixels.len() as u32;
    let blob_bytes = blob.len() as u32;
    let dib_size = BITMAPV5HEADER_SIZE + palette_bytes + pixel_bytes + blob_bytes;
    let file_size = BITMAPFILEHEADER_SIZE + dib_size;
    let pixel_offset = BITMAPFILEHEADER_SIZE + BITMAPV5HEADER_SIZE + palette_bytes;
    // DIB-relative; matches what the direct-colour V5 paths advertise.
    let profile_data_offset = BITMAPV5HEADER_SIZE + palette_bytes + pixel_bytes;

    let mut out = Vec::with_capacity(file_size as usize);
    write_file_header(&mut out, file_size, pixel_offset);
    write_dib_header_v5_indexed_with_profile(
        &mut out,
        width,
        signed_stored_height(height, options),
        bpp,
        pixel_bytes,
        clr_used,
        cs_type,
        rendering_intent,
        profile_data_offset,
        blob_bytes,
    );
    // Colour table (B, G, R, 0x00) right after the V5 header.
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
    out.extend_from_slice(&pixels);
    out.extend_from_slice(blob);
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
        BmpPixelFormat::Rgb555 => {
            // 16-bit DIB — no ICO mask support for 16-bit (alpha is
            // meaningless in 5-5-5 anyway). A 16-bpp `BI_RGB` DIB is
            // unambiguously RGB 555 so a plain 40-byte header suffices.
            let (pixels, _) = pack_rgb555(plane, width, height, opts)?;
            let mut out = Vec::new();
            let stored_h = if double_height_for_ico_mask {
                (height * 2) as i32
            } else {
                height as i32
            };
            write_dib_header_v3(&mut out, width, stored_h, 16, BI_RGB, 0);
            out.extend_from_slice(&pixels);
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
        BmpPixelFormat::Indexed2 => {
            let pal = palette
                .ok_or_else(|| Error::invalid("BMP encoder: Indexed2 requires a palette"))?;
            let (pixels, _) = pack_indexed(plane, 2, width, height, opts)?;
            let stored_h = if double_height_for_ico_mask {
                (height * 2) as i32
            } else {
                height as i32
            };
            let mut out = Vec::new();
            let entries = written_palette_entries(2, pal, opts);
            write_dib_header_v3_indexed(&mut out, width, stored_h, 2, BI_RGB, pal, entries);
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

/// Encode 16-bit RGB 5-5-5 `BI_RGB` (V3 header) BMP.
///
/// For a 16-bpp bitmap whose `biCompression` is `BI_RGB` the documented
/// pixel layout is always RGB 555 — the high bit reserved, then R in
/// bits 14..10, G in bits 9..5, B in bits 4..0. No `BI_BITFIELDS` mask
/// block is required (those only appear when the file declares its own
/// non-default masks), so a plain 40-byte `BITMAPINFOHEADER` suffices:
/// header → pixel array, with the pixel offset at `14 + 40`. The input
/// plane carries the 555 words already packed little-endian, two bytes
/// per pixel, so the packer only re-strides them to the 4-byte-aligned
/// on-disk row pitch.
fn encode_rgb555(
    plane: &BmpPlane,
    width: u32,
    height: u32,
    options: BmpEncodeOptions,
) -> Result<Vec<u8>> {
    let (pixels, _) = pack_rgb555(plane, width, height, options)?;
    let pixel_bytes = pixels.len() as u32;
    let file_size = BITMAPFILEHEADER_SIZE + BITMAPINFOHEADER_SIZE + pixel_bytes;
    let pixel_offset = BITMAPFILEHEADER_SIZE + BITMAPINFOHEADER_SIZE;
    let mut out = Vec::with_capacity(file_size as usize);
    write_file_header(&mut out, file_size, pixel_offset);
    write_dib_header_v3(
        &mut out,
        width,
        signed_stored_height(height, options),
        16,
        BI_RGB,
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
fn pack_rgb555(
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
            "BMP encoder: frame plane truncated (rgb555)",
        ));
    }
    // The input already carries packed little-endian 5-5-5 words (the
    // high bit reserved), two bytes per pixel — identical wire shape to
    // the 5-6-5 packer. We only re-stride to the 4-byte-aligned on-disk
    // row pitch and apply the bottom-up / top-down row order.
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
/// `bpp` must be 1, 2, 4, or 8.
/// For 8-bit: input is 1 byte per pixel (index 0-255).
/// For 4-bit: input is 1 byte per pixel (index 0-15); packing into
///   hi-nibble/lo-nibble is done here.
/// For 2-bit: input is 1 byte per pixel (index 0-3, treated as
///   `byte & 3`); four pixels pack per byte, the left-most pixel in the
///   two most-significant bits.
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
            2 => {
                // Four 2-bit indices per byte; the left-most pixel sits in
                // the two most-significant bits (Windows CE indexed layout,
                // the encode counterpart of the decoder's 2-bpp unpack).
                for x in 0..w {
                    let idx = src[x] & 0x03;
                    let shift = 6 - 2 * (x % 4);
                    dst[x / 4] |= idx << shift;
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
            _ => return Err(Error::invalid("pack_indexed: bpp must be 1, 2, 4, or 8")),
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

/// Write a 108-byte `BITMAPV4HEADER` with
/// `bV4CSType = LCS_CALIBRATED_RGB` and caller-supplied CIE endpoints +
/// per-channel gamma. Used by [`encode_bmp_with_calibrated_rgb`] and its
/// indexed helper.
///
/// Layout (offsets relative to the start of the DIB header):
/// * 0..40   classic `BITMAPINFOHEADER` fields (`biClrUsed` = `clr_used`)
/// * 40..56  R / G / B / A masks (`masks`; zeroed for `BI_RGB`,
///   canonical 5-6-5 for the 16-bpp `BI_BITFIELDS` arm)
/// * 56..60  `bV4CSType` = [`LCS_CALIBRATED_RGB`]
/// * 60..96  `bV4Endpoints` CIEXYZTRIPLE (9 × `i32`, packed
///   R.x R.y R.z G.x G.y G.z B.x B.y B.z)
/// * 96..108 `bV4GammaRed` / `bV4GammaGreen` / `bV4GammaBlue`
///   (3 × `u32`, unsigned 16.16 fixed point)
///
/// `image_size` is the byte length of the pixel array (written to the
/// `biSizeImage` slot at offset 20); `compression` and `masks`
/// parameterise the V3-prefix `biCompression` field and the four-mask
/// region the same way [`write_dib_header_v5_with_profile`] does.
/// `stored_height` is signed: negative for a top-down DIB.
#[allow(clippy::too_many_arguments)]
fn write_dib_header_v4_calibrated(
    out: &mut Vec<u8>,
    w: u32,
    stored_height: i32,
    bpp: u16,
    image_size: u32,
    compression: u32,
    masks: [u32; 4],
    clr_used: u32,
    endpoints: [i32; 9],
    gamma_rgb: [u32; 3],
) {
    // Classic V3 prefix (40 B).
    out.extend_from_slice(&BITMAPV4HEADER_SIZE.to_le_bytes());
    out.extend_from_slice(&(w as i32).to_le_bytes());
    out.extend_from_slice(&stored_height.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // planes
    out.extend_from_slice(&bpp.to_le_bytes());
    out.extend_from_slice(&compression.to_le_bytes());
    out.extend_from_slice(&image_size.to_le_bytes());
    out.extend_from_slice(&2835i32.to_le_bytes()); // x_pels/m ≈ 72 DPI
    out.extend_from_slice(&2835i32.to_le_bytes()); // y_pels/m
    out.extend_from_slice(&clr_used.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // clr_important
                                                // V4 tail: R/G/B/A masks (offsets 40..56).
    out.extend_from_slice(&masks[0].to_le_bytes()); // R
    out.extend_from_slice(&masks[1].to_le_bytes()); // G
    out.extend_from_slice(&masks[2].to_le_bytes()); // B
    out.extend_from_slice(&masks[3].to_le_bytes()); // A
                                                    // bV4CSType — calibrated RGB: endpoints + gamma are authoritative.
    out.extend_from_slice(&LCS_CALIBRATED_RGB.to_le_bytes());
    // CIEXYZTRIPLE endpoints (9 × i32 = 36 bytes).
    for v in endpoints {
        out.extend_from_slice(&v.to_le_bytes());
    }
    // Gamma R / G / B (3 × u32 = 12 bytes), unsigned 16.16 fixed point.
    for g in gamma_rgb {
        out.extend_from_slice(&g.to_le_bytes());
    }
    // Total: 40 + 16 + 4 + 36 + 12 = 108 ✓
}

/// Write a 124-byte `BITMAPV5HEADER` with `bV5CSType = PROFILE_EMBEDDED`.
///
/// Layout (offsets given relative to the start of the DIB header):
/// * 0..40   classic `BITMAPINFOHEADER` fields
/// * 40..56  R / G / B / A masks (zeroed — direct-colour `BI_RGB`)
/// * 56..60  `bV5CSType` = [`PROFILE_EMBEDDED`]
/// * 60..96  CIEXYZTRIPLE endpoints (zeroed)
/// * 96..108 R / G / B gamma (zeroed)
/// * 108..112 `bV5Intent`
/// * 112..116 `bV5ProfileData` (DIB-relative byte offset to ICC blob)
/// * 116..120 `bV5ProfileSize`
/// * 120..124 reserved (zero)
///
/// `image_size` is the byte length of the pixel array — written to
/// the classic `biSizeImage` slot at offset 20.
///
/// `compression` and `masks` parameterise the V3-prefix `biCompression`
/// field and the four-mask region at offsets 40..56. Direct-colour
/// `BI_RGB` paths pass `(BI_RGB, [0,0,0,0])`; the 16-bit V5 path passes
/// `(BI_BITFIELDS, [0xF800, 0x07E0, 0x001F, 0])` so the decoder picks
/// up the canonical RGB 5-6-5 mask layout from the header body the same
/// way it would on a V4 header.
#[allow(clippy::too_many_arguments)]
fn write_dib_header_v5_with_profile(
    out: &mut Vec<u8>,
    w: u32,
    stored_height: i32,
    bpp: u16,
    image_size: u32,
    compression: u32,
    masks: [u32; 4],
    cs_type: u32,
    rendering_intent: u32,
    profile_data_offset: u32,
    profile_size: u32,
) {
    // Classic V3 prefix (40 B).
    out.extend_from_slice(&BITMAPV5HEADER_SIZE.to_le_bytes());
    out.extend_from_slice(&(w as i32).to_le_bytes());
    out.extend_from_slice(&stored_height.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // planes
    out.extend_from_slice(&bpp.to_le_bytes());
    out.extend_from_slice(&compression.to_le_bytes()); // direct-colour or BI_BITFIELDS
    out.extend_from_slice(&image_size.to_le_bytes());
    out.extend_from_slice(&2835i32.to_le_bytes()); // x_pels/m ≈ 72 DPI
    out.extend_from_slice(&2835i32.to_le_bytes()); // y_pels/m
    out.extend_from_slice(&0u32.to_le_bytes()); // clr_used
    out.extend_from_slice(&0u32.to_le_bytes()); // clr_important
                                                // V4 tail: R/G/B/A masks. Zeroed for BI_RGB direct-colour;
                                                // canonical 5-6-5 layout for 16-bpp BI_BITFIELDS.
    out.extend_from_slice(&masks[0].to_le_bytes()); // R
    out.extend_from_slice(&masks[1].to_le_bytes()); // G
    out.extend_from_slice(&masks[2].to_le_bytes()); // B
    out.extend_from_slice(&masks[3].to_le_bytes()); // A
                                                    // bV5CSType — caller-selected: PROFILE_EMBEDDED routes the
                                                    // trailing blob through `BmpMetadata::icc_profile`;
                                                    // PROFILE_LINKED keeps the trailing blob as an opaque
                                                    // external-path bytestring that the decoder surfaces via
                                                    // `profile_data_offset` / `profile_size` without
                                                    // auto-loading.
    out.extend_from_slice(&cs_type.to_le_bytes());
    // CIEXYZTRIPLE endpoints (9 × i32 = 36 bytes) — zero; the profile
    // (embedded or linked) is the authoritative description.
    out.extend_from_slice(&[0u8; 36]);
    // Gamma R / G / B (3 × u32 = 12 bytes) — zero.
    out.extend_from_slice(&[0u8; 12]);
    // V5 tail.
    out.extend_from_slice(&rendering_intent.to_le_bytes());
    out.extend_from_slice(&profile_data_offset.to_le_bytes());
    out.extend_from_slice(&profile_size.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // bV5Reserved
                                                // Total: 40 + 16 + 4 + 36 + 12 + 4 + 4 + 4 + 4 = 124 ✓
}

/// Canonical RGB 5-6-5 mask quadruple (R, G, B, A=0) — used by the V5
/// 16-bpp BI_BITFIELDS encode path so its mask region matches the V4
/// 16-bpp encoder and the decoder's `BI_BITFIELDS` reader.
const RGB565_MASKS_V5: [u32; 4] = [0xF800, 0x07E0, 0x001F, 0x0000];

/// Canonical 32-bit alpha mask quadruple `(R=0, G=0, B=0, A=0xFF000000)`
/// for the V4 / V5 `BmpPixelFormat::Rgba` encode paths. The format stays
/// `BI_RGB` (so R / G / B keep the default BGRA byte order and their masks
/// are left zero, since the R / G / B masks are only valid under
/// `BI_BITFIELDS`), but the in-header alpha-mask slot at offset 52 is set
/// to the canonical high-byte mask. The BMP spec treats the alpha sample
/// as valid "whenever the alpha mask is present in the DIB header", so this
/// makes the emitted V4 / V5 32-bit file a spec-correct alpha-carrying
/// bitmap whose alpha the decoder recovers through the mask — rather than a
/// `BI_RGB` file that hides opacity in the reserved DWORD byte where a
/// strict reader would discard it.
const RGBA32_ALPHA_MASK_V4_V5: [u32; 4] = [0, 0, 0, 0xFF00_0000];

/// V5 header writer for the indexed encode paths. Layout is identical
/// to [`write_dib_header_v5_with_profile`] except that `biCompression`
/// is fixed at `BI_RGB`, the four-mask region at offsets 40..56 is
/// zeroed (the colour table that follows the V5 header carries the
/// colour assignment), and `biClrUsed` is written explicitly so a
/// minimal-palette table can be advertised to the decoder.
///
/// The header still finishes with the V5 tail
/// (`bV5Intent` / `bV5ProfileData` / `bV5ProfileSize` / `bV5Reserved`),
/// so the trailing ICC or path blob lives at `bV5ProfileData` after
/// the pixel array exactly like the direct-colour V5 paths.
#[allow(clippy::too_many_arguments)]
fn write_dib_header_v5_indexed_with_profile(
    out: &mut Vec<u8>,
    w: u32,
    stored_height: i32,
    bpp: u16,
    image_size: u32,
    clr_used: u32,
    cs_type: u32,
    rendering_intent: u32,
    profile_data_offset: u32,
    profile_size: u32,
) {
    out.extend_from_slice(&BITMAPV5HEADER_SIZE.to_le_bytes());
    out.extend_from_slice(&(w as i32).to_le_bytes());
    out.extend_from_slice(&stored_height.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // planes
    out.extend_from_slice(&bpp.to_le_bytes());
    out.extend_from_slice(&BI_RGB.to_le_bytes()); // indexed V5 → always uncompressed
    out.extend_from_slice(&image_size.to_le_bytes());
    out.extend_from_slice(&2835i32.to_le_bytes());
    out.extend_from_slice(&2835i32.to_le_bytes());
    out.extend_from_slice(&clr_used.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // clr_important
                                                // V4 mask region zeroed for BI_RGB indexed.
    out.extend_from_slice(&[0u8; 16]);
    out.extend_from_slice(&cs_type.to_le_bytes());
    out.extend_from_slice(&[0u8; 36]); // CIEXYZTRIPLE — zero, profile is authoritative
    out.extend_from_slice(&[0u8; 12]); // Gamma R/G/B — zero
    out.extend_from_slice(&rendering_intent.to_le_bytes());
    out.extend_from_slice(&profile_data_offset.to_le_bytes());
    out.extend_from_slice(&profile_size.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // bV5Reserved
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
        BmpPixelFormat::Rgb24 | BmpPixelFormat::Rgb555 | BmpPixelFormat::Rgb565 => {
            // No alpha → fully opaque → all-zero AND mask. Short-circuit.
            return Ok(mask);
        }
        BmpPixelFormat::Indexed8
        | BmpPixelFormat::Indexed4
        | BmpPixelFormat::Indexed2
        | BmpPixelFormat::Indexed1 => {
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
        2 => 4,
        4 => 16,
        8 => 256,
        _ => 0,
    }
}
