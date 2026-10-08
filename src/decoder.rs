//! BMP + DIB decode into the native layout ([`BmpImage`]).
//!
//! Supports (enough to cover every common icon / texture / historical
//! artifact you'd meet in the wild):
//!
//! * 1 / 2 / 4 / 8-bit indexed (`BI_RGB`, `BI_RLE4`, `BI_RLE8`) →
//!   `Pal8` with the colour table attached
//! * 16-bit `BI_RGB` (5-5-5) and `BI_BITFIELDS` 5-5-5 / 5-6-5 → `Rgb555`
//!   / `Rgb565`, the packed words copied verbatim
//! * 24-bit `BI_RGB` → `Bgr24`
//! * 32-bit `BI_RGB` and byte-aligned `BI_BITFIELDS` → `Bgra` (or `Rgba`
//!   for the R,G,B byte order); any other 16 / 32-bit mask set,
//!   including 16-bit alpha masks, is expanded channel by channel to
//!   `Rgba`
//! * every header generation: OS/2 `BITMAPCOREHEADER`, truncated and
//!   full OS/2 2.x headers, V3 `BITMAPINFOHEADER`, Adobe V2 / V3, V4,
//!   V5 (with the colour-space tag and embedded ICC profile surfaced
//!   on the image)
//!
//! Rows are delivered top-down (bottom-up files are flipped while
//! decoding) and tightly packed.
//!
//! Not supported: `BI_JPEG` / `BI_PNG` embedded payloads and the CMYK
//! compressions.
//!
//! The contract entry points live at the crate root ([`crate::decode`],
//! [`crate::decode_with`], [`crate::info`], …); this module keeps the
//! headerless-DIB helpers `oxideav-ico` uses and the deprecated
//! pre-contract names.

use crate::error::{BmpError as Error, Result};
use crate::image::{BmpImage, BmpPixelFormat, ColorInfo, ImageInfo, Metadata, Palette, Plane};
use crate::metadata::{BmpColorSpace, BmpMetadata};
use crate::options::DecodeOptions;
use crate::types::*;

// The framework-side items that used to live in this module keep their
// `decoder::` path for one release.
#[cfg(feature = "registry")]
pub use crate::registry::make_decoder;
#[cfg(feature = "registry")]
#[allow(deprecated)]
pub use crate::registry::{decode_bmp_videoframe, decode_dib_videoframe};

// ---------------------------------------------------------------------------
// Headerless-DIB surface (oxideav-ico) + pre-contract names
// ---------------------------------------------------------------------------

/// Decode a headerless DIB (`BITMAPINFOHEADER` + pixels, no
/// `BITMAPFILEHEADER`) with the default [`DecodeOptions`]. Used by
/// `oxideav-ico`; see [`decode_dib_with`].
pub fn decode_dib(input: &[u8], dib_height_is_doubled_for_mask: bool) -> Result<BmpImage> {
    decode_dib_with(
        input,
        dib_height_is_doubled_for_mask,
        &DecodeOptions::default(),
    )
}

/// Decode a headerless DIB (`BITMAPINFOHEADER` + pixels, no
/// `BITMAPFILEHEADER`) into its native layout, like [`crate::decode_with`]
/// does for a whole file.
///
/// When `dib_height_is_doubled_for_mask` is true, the incoming
/// `biHeight` is 2× the real height (XOR mask + AND mask layout from
/// `.ico` / `.cur`). The returned image dimensions are halved on the
/// height axis, the result is re-laid as `Rgba`, and the 1-bpp AND
/// mask following the XOR pixels is folded into the alpha channel — a
/// 1-bit in the AND mask maps to `alpha = 0` (transparent), a 0-bit
/// keeps whatever the XOR pixels carried.
pub fn decode_dib_with(
    input: &[u8],
    dib_height_is_doubled_for_mask: bool,
    opts: &DecodeOptions,
) -> Result<BmpImage> {
    let (header, _header_bytes) = parse_dib_header(input)?;
    // For a "pure" DIB, pixel data starts right after the header (plus
    // any bit-field masks and colour table) — the canonical layout
    // `canonical_dib_pixel_offset` computes, shared with the file path.
    let pixel_start = canonical_dib_pixel_offset(&header);
    if dib_height_is_doubled_for_mask {
        decode_dib_with_mask(&header, input, pixel_start, opts)
    } else {
        decode_dib_payload(&header, input, 0, pixel_start, opts)
    }
}

/// Decode a complete BMP file into an `Rgba` [`BmpImage`] (the
/// pre-contract shape: palette expanded, channels swizzled).
#[deprecated(
    note = "use oxideav_bmp::decode (IMAGE_CRATE_API); it returns the native layout, \
                     to_rgba8() gives these bytes"
)]
pub fn decode_bmp(input: &[u8]) -> Result<BmpImage> {
    Ok(decode_file(input, &DecodeOptions::default())?.into_rgba())
}

/// Decode a complete BMP file like the deprecated `decode_bmp` and also
/// return the parsed V3 / V4 / V5 header metadata.
#[deprecated(
    note = "use oxideav_bmp::decode (color / metadata are on the image) and \
                     BmpMetadata::from_bmp for the header fields (IMAGE_CRATE_API)"
)]
pub fn decode_bmp_with_metadata(input: &[u8]) -> Result<(BmpImage, BmpMetadata)> {
    let image = decode_file(input, &DecodeOptions::default())?.into_rgba();
    let metadata = BmpMetadata::from_bmp(input)?;
    Ok((image, metadata))
}

/// Decode a headerless DIB like [`decode_dib`] (then re-laid as `Rgba`)
/// and also return the parsed header metadata.
#[deprecated(
    note = "use oxideav_bmp::decode_dib (color / metadata are on the image) and \
                     BmpMetadata::from_dib for the header fields (IMAGE_CRATE_API)"
)]
pub fn decode_dib_with_metadata(
    input: &[u8],
    dib_height_is_doubled_for_mask: bool,
) -> Result<(BmpImage, BmpMetadata)> {
    let image = decode_dib(input, dib_height_is_doubled_for_mask)?.into_rgba();
    let metadata = BmpMetadata::from_dib(input)?;
    Ok((image, metadata))
}

// ---------------------------------------------------------------------------
// Crate-internal entry points behind the root vocabulary
// ---------------------------------------------------------------------------

/// Parse the file header + DIB header of a whole BMP file and resolve
/// the pixel-array offset. Returns `(header, pixel_offset)`.
pub(crate) fn parse_file(input: &[u8], strict: bool) -> Result<(DibHeader, usize)> {
    // Smallest legal DIB is the OS/2 1.x BITMAPCOREHEADER (12 B) on top
    // of the 14-byte BITMAPFILEHEADER. Larger DIB variants are checked
    // again inside `parse_dib_header` after reading the size field.
    if input.len() < (BITMAPFILEHEADER_SIZE + BITMAPCOREHEADER_SIZE) as usize {
        return Err(Error::invalid("BMP: input shorter than header"));
    }
    let file_header = BitmapFileHeader::parse(input)?;
    if strict && !file_header.reserved_is_clean() {
        return Err(Error::invalid(
            "BMP: BITMAPFILEHEADER reserved words must be zero (strict)",
        ));
    }
    let dib = &input[BITMAPFILEHEADER_SIZE as usize..];
    let (header, _) = parse_dib_header(dib)?;
    // Honour `bfOffBits` when it lands at/after the canonical pixel
    // position (so a writer's deliberate gap survives); recover the
    // canonical offset when it is 0 or points implausibly early.
    let stored = file_header.pixel_offset as usize;
    let canonical = canonical_file_pixel_offset(&header);
    if strict && stored < canonical {
        return Err(Error::invalid(format!(
            "BMP: bfOffBits {stored} points inside the header / colour table (canonical {canonical}; strict)"
        )));
    }
    let pixel_offset = resolve_file_pixel_offset(stored, &header);
    Ok((header, pixel_offset))
}

/// [`crate::decode_with`]: whole file → native layout.
pub(crate) fn decode_file(input: &[u8], opts: &DecodeOptions) -> Result<BmpImage> {
    let (header, pixel_offset) = parse_file(input, opts.strict)?;
    decode_dib_payload(
        &header,
        input,
        BITMAPFILEHEADER_SIZE as usize,
        pixel_offset,
        opts,
    )
}

/// [`crate::info`]: header-only description of a whole file.
pub(crate) fn info_file(input: &[u8]) -> Result<ImageInfo> {
    let (header, _) = parse_file(input, false)?;
    validate_header(&header)?;
    let layout = classify(&header);
    let mut info = ImageInfo::new(
        header.absolute_width(),
        header.absolute_height(),
        layout.format,
    );
    info.color = color_from_header(&header);
    info.has_icc = header.cs_type == Some(PROFILE_EMBEDDED) && header.profile_size.unwrap_or(0) > 0;
    info.bits_per_pixel = header.bpp;
    info.compression = header.compression;
    info.header_size = header.header_size;
    info.top_down = header.is_top_down();
    Ok(info)
}

/// Header-only [`BmpMetadata`] for a whole file (used by
/// [`BmpMetadata::from_bmp`]).
pub(crate) fn bmp_metadata_file(input: &[u8]) -> Result<BmpMetadata> {
    let (header, _) = parse_file(input, false)?;
    Ok(bmp_metadata(&header, input, BITMAPFILEHEADER_SIZE as usize))
}

/// Header-only [`BmpMetadata`] for a headerless DIB (used by
/// [`BmpMetadata::from_dib`]).
pub(crate) fn bmp_metadata_dib(input: &[u8]) -> Result<BmpMetadata> {
    let (header, _) = parse_dib_header(input)?;
    Ok(bmp_metadata(&header, input, 0))
}

/// Build the full header metadata, slicing the V5 profile slot out of
/// `whole` (`base` = 14 for a file, 0 for a headerless DIB).
fn bmp_metadata(header: &DibHeader, whole: &[u8], base: usize) -> BmpMetadata {
    let mut metadata = BmpMetadata::from_header(header);
    // Embedded ICC blobs sit after the pixel array. The offset is given
    // relative to the start of the DIB header (i.e. file_offset =
    // base + bV5ProfileData). A linked profile carries a file-path
    // bytestring at the same slot; we surface the offset + size but
    // never load it. Only the `bV5CSType` discriminator distinguishes
    // path-bytes from ICC-bytes on the wire.
    let slot = || {
        read_profile_slot(
            whole,
            base,
            header.profile_data_offset.unwrap_or(0) as usize,
            header.profile_size.unwrap_or(0) as usize,
        )
    };
    if metadata.color_space == Some(BmpColorSpace::ProfileEmbedded) {
        metadata.icc_profile = slot();
    } else if metadata.color_space == Some(BmpColorSpace::ProfileLinked) {
        metadata.linked_profile_path = slot();
    }
    metadata
}

/// The contract [`ColorInfo`] a header signals: `LCS_sRGB` /
/// `LCS_WINDOWS_COLOR_SPACE` → sRGB, everything else → the BMP default.
pub(crate) fn color_from_header(header: &DibHeader) -> ColorInfo {
    match header.cs_type {
        Some(LCS_S_RGB) | Some(LCS_WINDOWS_COLOR_SPACE) => ColorInfo::srgb(),
        _ => ColorInfo::bmp_default(),
    }
}

/// The contract [`Metadata`]: a copy of the embedded ICC profile when
/// the V5 header declares one that fits in the buffer and `copy_icc`
/// ([`DecodeOptions::copy_icc`]) asks for it.
fn metadata_from_header(header: &DibHeader, whole: &[u8], base: usize, copy_icc: bool) -> Metadata {
    let mut m = Metadata::default();
    if copy_icc && header.cs_type == Some(PROFILE_EMBEDDED) {
        m.icc = read_profile_slot(
            whole,
            base,
            header.profile_data_offset.unwrap_or(0) as usize,
            header.profile_size.unwrap_or(0) as usize,
        );
    }
    m
}

/// Slice the V5 trailing-slot blob (embedded ICC bytes or the linked
/// path bytestring) out of the input buffer.
///
/// `base` is the offset of the DIB header start within `input`
/// (14 bytes for a BMP file, 0 for a headerless DIB). `data_offset` is
/// the `bV5ProfileData` field (DIB-relative) and `size` is
/// `bV5ProfileSize`. Returns `None` if the resulting slice would fall
/// past the end of `input` so a malformed V5 header can't poison the
/// metadata path — declared offsets and sizes are still surfaced on
/// the returned [`BmpMetadata`] so callers can investigate.
fn read_profile_slot(
    input: &[u8],
    base: usize,
    data_offset: usize,
    size: usize,
) -> Option<Vec<u8>> {
    if size == 0 {
        return None;
    }
    let start = base.checked_add(data_offset)?;
    let end = start.checked_add(size)?;
    if end > input.len() {
        return None;
    }
    Some(input[start..end].to_vec())
}

// ---------------------------------------------------------------------------
// Native-layout classification
// ---------------------------------------------------------------------------

/// Whether the fourth byte of a 32-bit pixel is alpha or padding.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Alpha {
    /// Keep the stored byte.
    Stored,
    /// The header declares no alpha: force `0xFF`.
    Opaque,
}

/// How the pixel array is turned into the native plane.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kernel {
    /// 1 / 2 / 4 / 8 bpp uncompressed → `Pal8` indices.
    Indexed,
    /// `BI_RLE8` → `Pal8` indices.
    Rle8,
    /// `BI_RLE4` → `Pal8` indices.
    Rle4,
    /// 16 bpp 5-5-5 / 5-6-5: copy the words.
    Packed16,
    /// 24 bpp: copy the B,G,R bytes.
    Bgr24,
    /// 32 bpp with B,G,R,(A) byte order.
    Bgra(Alpha),
    /// 32 bpp `BI_BITFIELDS` with R,G,B,(A) byte order.
    RgbaBytes(Alpha),
    /// Any other 16 / 32 bpp mask set: per-channel expansion to RGBA.
    Expand { r: u32, g: u32, b: u32, a: u32 },
}

#[derive(Clone, Copy, Debug)]
struct Layout {
    format: BmpPixelFormat,
    kernel: Kernel,
}

/// Decide the native layout for a validated header (see
/// [`BmpPixelFormat`] for the table).
fn classify(h: &DibHeader) -> Layout {
    let bitfields = h.compression == BI_BITFIELDS || h.compression == BI_ALPHABITFIELDS;
    match h.bpp {
        1 | 2 | 4 | 8 => {
            let kernel = match h.compression {
                BI_RLE8 => Kernel::Rle8,
                BI_RLE4 => Kernel::Rle4,
                _ => Kernel::Indexed,
            };
            Layout {
                format: BmpPixelFormat::Pal8,
                kernel,
            }
        }
        16 => {
            // Default BI_RGB mapping is 5-5-5 with the high bit
            // reserved. BI_BITFIELDS / BI_ALPHABITFIELDS let the file
            // declare its own layout (e.g. 5-6-5; the alpha-bitfields
            // flavour additionally carries an alpha mask in the V3
            // header tail).
            let (r, g, b, a) = if bitfields {
                (
                    h.mask_r.unwrap_or(0x7C00),
                    h.mask_g.unwrap_or(0x03E0),
                    h.mask_b.unwrap_or(0x001F),
                    h.mask_a.unwrap_or(0),
                )
            } else {
                (0x7C00, 0x03E0, 0x001F, 0)
            };
            match (r, g, b, a) {
                (0x7C00, 0x03E0, 0x001F, 0) => Layout {
                    format: BmpPixelFormat::Rgb555,
                    kernel: Kernel::Packed16,
                },
                (0xF800, 0x07E0, 0x001F, 0) => Layout {
                    format: BmpPixelFormat::Rgb565,
                    kernel: Kernel::Packed16,
                },
                _ => Layout {
                    format: BmpPixelFormat::Rgba,
                    kernel: Kernel::Expand { r, g, b, a },
                },
            }
        }
        24 => Layout {
            format: BmpPixelFormat::Bgr24,
            kernel: Kernel::Bgr24,
        },
        _ => {
            // 32 bpp. Default BI_RGB is BGRA; BI_BITFIELDS /
            // BI_ALPHABITFIELDS may declare otherwise.
            if bitfields && (h.mask_r.is_some() || h.mask_g.is_some() || h.mask_b.is_some()) {
                let r = h.mask_r.unwrap_or(0x00FF_0000);
                let g = h.mask_g.unwrap_or(0x0000_FF00);
                let b = h.mask_b.unwrap_or(0x0000_00FF);
                let a = h.mask_a.unwrap_or(0);
                let alpha = match a {
                    0 => Some(Alpha::Opaque),
                    0xFF00_0000 => Some(Alpha::Stored),
                    _ => None,
                };
                match ((r, g, b), alpha) {
                    ((0x00FF_0000, 0x0000_FF00, 0x0000_00FF), Some(alpha)) => Layout {
                        format: BmpPixelFormat::Bgra,
                        kernel: Kernel::Bgra(alpha),
                    },
                    ((0x0000_00FF, 0x0000_FF00, 0x00FF_0000), Some(alpha)) => Layout {
                        format: BmpPixelFormat::Rgba,
                        kernel: Kernel::RgbaBytes(alpha),
                    },
                    _ => Layout {
                        format: BmpPixelFormat::Rgba,
                        kernel: Kernel::Expand { r, g, b, a },
                    },
                }
            } else if h.compression == BI_RGB && h.mask_a.is_some() {
                // V4 / V5 BI_RGB carries the alpha mask in the header body.
                // R / G / B stay at the default BGRA byte positions (the
                // R/G/B masks are *not* valid under BI_RGB), but a non-zero
                // in-header alpha mask makes the alpha sample valid. A zero
                // alpha mask means "no alpha", so the pixel is opaque —
                // which also fixes the otherwise-transparent decode of a
                // V4 / V5 BI_RGB bitmap whose reserved high bytes are all
                // zero.
                match h.mask_a.unwrap_or(0) {
                    0 => Layout {
                        format: BmpPixelFormat::Bgra,
                        kernel: Kernel::Bgra(Alpha::Opaque),
                    },
                    0xFF00_0000 => Layout {
                        format: BmpPixelFormat::Bgra,
                        kernel: Kernel::Bgra(Alpha::Stored),
                    },
                    a => Layout {
                        format: BmpPixelFormat::Rgba,
                        kernel: Kernel::Expand {
                            r: 0x00FF_0000,
                            g: 0x0000_FF00,
                            b: 0x0000_00FF,
                            a,
                        },
                    },
                }
            } else {
                // Plain V3 BI_RGB: the fourth byte is kept as stored (it
                // is often 0 in older files; callers who want "treat
                // all-zero alpha as opaque" handle that themselves).
                Layout {
                    format: BmpPixelFormat::Bgra,
                    kernel: Kernel::Bgra(Alpha::Stored),
                }
            }
        }
    }
}

/// Reject compressions, depths and geometries the decoder does not
/// handle, before any allocation.
fn validate_header(h: &DibHeader) -> Result<()> {
    match h.compression {
        BI_RGB | BI_BITFIELDS | BI_ALPHABITFIELDS | BI_RLE4 | BI_RLE8 => {}
        BI_JPEG => return Err(Error::invalid("BMP: embedded JPEG not supported")),
        BI_PNG => return Err(Error::invalid("BMP: embedded PNG not supported")),
        // The CMYK family (compression 11 / 12 / 13, "only Windows Metafile
        // CMYK") stores CMYK samples whose channel layout and CMYK→RGB
        // conversion are defined by the WMF spec, not the BMP file-format
        // material available to this crate. Recognise them by name and reject
        // with a distinct message rather than the generic "unknown
        // compression" path, so a CMYK bitmap is reported as a known-but-
        // unsupported format instead of looking like a corrupt header.
        BI_CMYK => return Err(Error::invalid("BMP: CMYK (BI_CMYK) not supported")),
        BI_CMYKRLE8 => {
            return Err(Error::invalid(
                "BMP: CMYK RLE-8 (BI_CMYKRLE8) not supported",
            ))
        }
        BI_CMYKRLE4 => {
            return Err(Error::invalid(
                "BMP: CMYK RLE-4 (BI_CMYKRLE4) not supported",
            ))
        }
        c => return Err(Error::invalid(format!("BMP: unknown compression {c}"))),
    }

    if h.absolute_width() == 0 || h.absolute_height() == 0 {
        return Err(Error::invalid("BMP: zero dimension"));
    }

    // Validate bpp before any per-row allocation. A header with `bpp = 0`
    // (legal only for BI_JPEG / BI_PNG, both rejected above) yields a
    // zero row stride, so the "pixel array truncated" length check would
    // pass for any height and the destination allocation would be
    // attacker-sized. Reject unsupported depths here so a non-zero stride
    // always bounds the height against the available bytes.
    if !matches!(h.bpp, 1 | 2 | 4 | 8 | 16 | 24 | 32) {
        return Err(Error::invalid(format!(
            "BMP: unsupported bit depth {}",
            h.bpp
        )));
    }

    // For compressed formats `biHeight` must be positive "regardless of
    // image orientation" (BITMAPINFOHEADER remarks): an RLE stream
    // describes a bottom-up scan with end-of-line / delta / end-of-bitmap
    // escapes that have no defined meaning under a top-down (negative
    // height) layout. A negative `biHeight` on an RLE bitmap is malformed
    // — reject it rather than silently decoding the |height| rows as if
    // they were bottom-up.
    if (h.compression == BI_RLE8 || h.compression == BI_RLE4) && h.is_top_down() {
        return Err(Error::invalid(
            "BMP: RLE compression requires a positive biHeight (top-down RLE is illegal)",
        ));
    }
    if h.compression == BI_RLE8 && h.bpp != 8 {
        return Err(Error::invalid("BMP: BI_RLE8 requires bpp=8"));
    }
    if h.compression == BI_RLE4 && h.bpp != 4 {
        return Err(Error::invalid("BMP: BI_RLE4 requires bpp=4"));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Payload decode
// ---------------------------------------------------------------------------

/// Decode the pixel array described by `h` into its native layout.
/// `base` is where the DIB header starts inside `whole` (14 for a file,
/// 0 for a headerless DIB); `pixel_offset` is where the pixel array
/// starts.
fn decode_dib_payload(
    h: &DibHeader,
    whole: &[u8],
    base: usize,
    pixel_offset: usize,
    opts: &DecodeOptions,
) -> Result<BmpImage> {
    validate_header(h)?;
    let layout = classify(h);
    let width = h.absolute_width();
    let height = h.absolute_height();
    let bpp_bytes = layout.format.bytes_per_pixel();

    // Limits are checked against the native plane size before the
    // palette is read or any pixel buffer exists.
    opts.check(
        width,
        height,
        u64::from(width) * u64::from(height) * bpp_bytes as u64,
    )?;

    let palette = read_palette(h, whole, pixel_offset)?;
    let w = width as usize;
    let hgt = height as usize;
    let out_stride = w * bpp_bytes;

    let data = match layout.kernel {
        Kernel::Rle8 => {
            let rle_data = rle_input(whole, pixel_offset, width as u64, height as u64)?;
            decode_rle8(rle_data, w, hgt)?
        }
        Kernel::Rle4 => {
            let rle_data = rle_input(whole, pixel_offset, width as u64, height as u64)?;
            decode_rle4(rle_data, w, hgt)?
        }
        kernel => {
            let pixels = pixel_array(h, whole, pixel_offset)?;
            decode_pixels(h, pixels, kernel, out_stride)
        }
    };

    let palette = if layout.format.is_indexed() {
        let mut entries = palette;
        entries.truncate(256);
        Some(Palette::new(entries))
    } else {
        None
    };

    Ok(BmpImage {
        width,
        height,
        format: layout.format,
        planes: vec![Plane::new(out_stride, data)],
        color: color_from_header(h),
        metadata: metadata_from_header(h, whole, base, opts.copy_icc),
        palette,
    })
}

/// Bounds-check and slice the uncompressed pixel array.
fn pixel_array<'a>(h: &DibHeader, whole: &'a [u8], pixel_offset: usize) -> Result<&'a [u8]> {
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
    Ok(&whole[pixel_offset..pixel_end])
}

/// Decode an uncompressed pixel array straight into a single flat
/// top-down plane of the native layout.
///
/// The destination is allocated once and each source scanline is
/// written to its final top-down position: a bottom-up DIB places
/// source row `y` at destination row `height-1-y`, a top-down DIB at
/// row `y`.
fn decode_pixels(h: &DibHeader, pixels: &[u8], kernel: Kernel, out_stride: usize) -> Vec<u8> {
    let width = h.absolute_width() as usize;
    let height = h.absolute_height() as usize;
    let stride = h.row_stride();
    let mut out = vec![0u8; out_stride.saturating_mul(height)];
    let top_down = h.is_top_down();
    let row_dst = |y: usize| -> usize {
        if top_down {
            y
        } else {
            height - 1 - y
        }
    };

    match kernel {
        Kernel::Indexed => {
            // Unpack 1 / 2 / 4-bit indices to one byte per pixel (MSB =
            // leftmost pixel in every sub-byte layout); 8 bpp is a copy.
            let bpp = h.bpp as usize;
            for y in 0..height {
                let row = &pixels[y * stride..y * stride + stride];
                let d = row_dst(y) * out_stride;
                let dst = &mut out[d..d + width];
                match bpp {
                    8 => dst.copy_from_slice(&row[..width]),
                    4 => {
                        for (x, px) in dst.iter_mut().enumerate() {
                            let byte = row[x / 2];
                            *px = if x & 1 == 0 { byte >> 4 } else { byte & 0x0F };
                        }
                    }
                    2 => {
                        for (x, px) in dst.iter_mut().enumerate() {
                            let byte = row[x / 4];
                            let shift = 6 - 2 * (x % 4);
                            *px = (byte >> shift) & 0x03;
                        }
                    }
                    _ => {
                        for (x, px) in dst.iter_mut().enumerate() {
                            let byte = row[x / 8];
                            *px = (byte >> (7 - (x % 8))) & 1;
                        }
                    }
                }
            }
        }
        Kernel::Packed16 | Kernel::Bgr24 | Kernel::Bgra(Alpha::Stored) => {
            // Byte-for-byte copy of the visible row (the DWORD padding is
            // dropped).
            for y in 0..height {
                let row = &pixels[y * stride..y * stride + out_stride];
                let d = row_dst(y) * out_stride;
                out[d..d + out_stride].copy_from_slice(row);
            }
        }
        Kernel::Bgra(Alpha::Opaque) => {
            for y in 0..height {
                let row = &pixels[y * stride..y * stride + out_stride];
                let d = row_dst(y) * out_stride;
                let dst = &mut out[d..d + out_stride];
                dst.copy_from_slice(row);
                for px in dst.chunks_exact_mut(4) {
                    px[3] = 0xFF;
                }
            }
        }
        Kernel::RgbaBytes(alpha) => {
            for y in 0..height {
                let row = &pixels[y * stride..y * stride + out_stride];
                let d = row_dst(y) * out_stride;
                let dst = &mut out[d..d + out_stride];
                dst.copy_from_slice(row);
                if alpha == Alpha::Opaque {
                    for px in dst.chunks_exact_mut(4) {
                        px[3] = 0xFF;
                    }
                }
            }
        }
        Kernel::Expand { r, g, b, a } => expand_masked(
            pixels,
            width,
            height,
            stride,
            h.bpp,
            (r, g, b, a),
            &mut out,
            row_dst,
        ),
        Kernel::Rle8 | Kernel::Rle4 => unreachable!("RLE kernels take the RLE path"),
    }
    out
}

/// Per-channel mask expansion of a 16- or 32-bit pixel array into RGBA.
///
/// Below the size where a 65 536-entry combined value→RGBA table
/// amortises its 256 KiB build, four 256-byte per-channel tables (1 KiB
/// total, L1-resident) decode each pixel with three/four branch-free
/// loads. At or above the threshold (16 bpp only) the combined table's
/// single indexed load per pixel still edges ahead, so it is retained.
/// Both paths emit bit-identical bytes.
#[allow(clippy::too_many_arguments)]
fn expand_masked(
    pixels: &[u8],
    width: usize,
    height: usize,
    stride: usize,
    bpp: u16,
    (mr, mg, mb, ma): (u32, u32, u32, u32),
    out: &mut [u8],
    row_dst: impl Fn(usize) -> usize,
) {
    let out_stride = width * 4;
    if bpp == 16 {
        let total_px = width.saturating_mul(height);
        if total_px >= 1 << 18 {
            let (rs, rn) = shift_len(mr);
            let (gs, gn) = shift_len(mg);
            let (bs, bn) = shift_len(mb);
            let (as_, an) = shift_len(ma);
            let mut lut = vec![0u8; 65_536 * 4];
            for (v, slot) in lut.chunks_exact_mut(4).enumerate() {
                let v = v as u32;
                slot[0] = expand(((v & mr) >> rs) as u8, rn);
                slot[1] = expand(((v & mg) >> gs) as u8, gn);
                slot[2] = expand(((v & mb) >> bs) as u8, bn);
                slot[3] = if an > 0 {
                    expand(((v & ma) >> as_) as u8, an)
                } else {
                    0xFF
                };
            }
            for y in 0..height {
                let row = &pixels[y * stride..y * stride + width * 2];
                let d = row_dst(y) * out_stride;
                let dst = &mut out[d..d + out_stride];
                for (x, px) in dst.chunks_exact_mut(4).enumerate() {
                    let v = u16::from_le_bytes([row[x * 2], row[x * 2 + 1]]) as usize;
                    px.copy_from_slice(&lut[v * 4..v * 4 + 4]);
                }
            }
        } else {
            let rl = ChannelLut::new(mr);
            let gl = ChannelLut::new(mg);
            let bl = ChannelLut::new(mb);
            let al = ChannelLut::new(ma);
            let has_alpha = ma != 0;
            for y in 0..height {
                let row = &pixels[y * stride..y * stride + width * 2];
                let d = row_dst(y) * out_stride;
                let dst = &mut out[d..d + out_stride];
                for (px, src) in dst.chunks_exact_mut(4).zip(row.chunks_exact(2)) {
                    let v = u16::from_le_bytes([src[0], src[1]]) as u32;
                    px[0] = rl.get(v);
                    px[1] = gl.get(v);
                    px[2] = bl.get(v);
                    px[3] = if has_alpha { al.get(v) } else { 0xFF };
                }
            }
        }
    } else {
        let rl = ChannelLut::new(mr);
        let gl = ChannelLut::new(mg);
        let bl = ChannelLut::new(mb);
        let al = ChannelLut::new(ma);
        let has_alpha = ma != 0;
        for y in 0..height {
            let row = &pixels[y * stride..y * stride + width * 4];
            let d = row_dst(y) * out_stride;
            let dst = &mut out[d..d + out_stride];
            for (px, src) in dst.chunks_exact_mut(4).zip(row.chunks_exact(4)) {
                let v = u32::from_le_bytes([src[0], src[1], src[2], src[3]]);
                px[0] = rl.get(v);
                px[1] = gl.get(v);
                px[2] = bl.get(v);
                px[3] = if has_alpha { al.get(v) } else { 0xFF };
            }
        }
    }
}

/// `.ico` / `.cur` sub-image: XOR pixels (native layout) followed by a
/// 1-bpp AND mask. The result is re-laid as `Rgba` so the mask can be
/// folded into alpha.
fn decode_dib_with_mask(
    h: &DibHeader,
    whole: &[u8],
    pixel_offset: usize,
    opts: &DecodeOptions,
) -> Result<BmpImage> {
    // Height in the DIB is doubled to cover the AND mask; actual
    // pixel height is the real image size.
    let mut xor_header = *h;
    xor_header.height = h.height / 2;
    let mut image = decode_dib_payload(&xor_header, whole, 0, pixel_offset, opts)?.into_rgba();

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
    // for ICO is fixed. Apply it row-by-row, remembering that the XOR
    // image is already top-down.
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

// ---------------------------------------------------------------------------
// RLE decoders (→ Pal8 index planes)
// ---------------------------------------------------------------------------

/// Decode a BI_RLE8 stream straight into a single flat top-down index
/// plane (one byte per pixel).
///
/// The stream encodes 8-bit indices bottom-up (stream row 0 = bottom of
/// image); each pixel is written to its already-flipped destination row
/// `(height - 1 - y)`. Pixels the stream never writes — the cells a
/// `delta` jumps over, the tail of a short row after an end-of-line
/// escape, and every cell past an early end-of-bitmap — take **colour
/// index 0**, the first colour-table entry, exactly like any
/// explicitly-coded index-0 pixel. (The BMP *Bitmap Compression*
/// material defines the escape semantics but is silent on the fill
/// colour; Windows fills index 0, the canonical background.)
fn decode_rle8(data: &[u8], width: usize, height: usize) -> Result<Vec<u8>> {
    let mut out = vec![0u8; width.saturating_mul(height)];
    let mut x = 0usize;
    // RLE8 bitmaps are bottom-up: row 0 in the stream is the bottom row.
    let mut y = 0usize;
    let mut i = 0usize;

    macro_rules! put_pixel {
        ($idx:expr) => {
            if x < width && y < height {
                out[(height - 1 - y) * width + x] = $idx;
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
    Ok(out)
}

/// Decode a BI_RLE4 stream straight into a single flat top-down index
/// plane (see [`decode_rle8`] for the bottom-up flip and the index-0
/// background; here the index is a 4-bit nibble).
fn decode_rle4(data: &[u8], width: usize, height: usize) -> Result<Vec<u8>> {
    let mut out = vec![0u8; width.saturating_mul(height)];
    let mut x = 0usize;
    let mut y = 0usize;
    let mut i = 0usize;

    macro_rules! put_pixel {
        ($idx:expr) => {
            if x < width && y < height {
                out[(height - 1 - y) * width + x] = $idx & 0x0F;
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
    Ok(out)
}

// ---------------------------------------------------------------------------
// Header parsers + offset arithmetic (unchanged)
// ---------------------------------------------------------------------------

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

    // OS/2 2.x `OS22XBITMAPHEADER` writers may stop the header anywhere
    // from 16 bytes (size/width/height/planes/bit-count only) up to its
    // full 64-byte form; every field past the truncation point reads as
    // zero. Sizes 16..40 are these truncated forms — they share the
    // 4-byte signed width/height layout and 4-byte `RGBQUAD` palette of
    // `BITMAPINFOHEADER`, so we synthesise a 40-byte-equivalent
    // `DibHeader` from whatever bytes are present. (Full 64-byte
    // headers fall through to the shared `>= 40` INFO path below.)
    if (BITMAPCOREHEADER2_MIN_SIZE..BITMAPINFOHEADER_SIZE).contains(&header_size) {
        return parse_truncated_os22x_header(input, header_size);
    }

    if header_size < BITMAPINFOHEADER_SIZE {
        return Err(Error::invalid(format!(
            "BMP: unsupported header size {header_size}"
        )));
    }
    if input.len() < header_size as usize {
        return Err(Error::invalid("BMP: header size exceeds input"));
    }
    // Every `biSize >= 40` generation (V3 / V2-Adobe / V3-Adobe / V4 /
    // V5) shares the eleven-field BITMAPINFOHEADER layout as a prefix;
    // read it through the typed view so the field offsets live in one
    // place. The length checks above guarantee 40 bytes are present.
    let base = BitmapInfoHeader::from_bytes(input)
        .ok_or_else(|| Error::invalid("BMP: DIB header truncated"))?;
    let width = base.width;
    let height = base.height;
    let planes = base.planes;
    let bpp = base.bit_count;
    let compression = base.compression;
    let image_size = base.image_size;
    let x_pels_per_meter = base.x_pels_per_meter;
    let y_pels_per_meter = base.y_pels_per_meter;
    let clr_used = base.clr_used;
    let clr_important = base.clr_important;

    if width <= 0 {
        return Err(Error::invalid("BMP: non-positive width"));
    }
    if planes != 1 {
        return Err(Error::invalid(format!("BMP: planes={planes} (must be 1)")));
    }

    let (mask_r, mask_g, mask_b, mask_a) =
        if compression == BI_BITFIELDS || compression == BI_ALPHABITFIELDS {
            if header_size >= BITMAPV2INFOHEADER_SIZE {
                // V2 (52 B) / V3 (56 B) Adobe-intermediate headers and
                // every V4+ header carry the R/G/B mask block inside the
                // header body at offsets 40..52. V3 / V4 / V5 extend that
                // by 4 bytes of in-header alpha mask at offset 52. Read
                // alpha when the header is large enough to include it
                // (>= 56 B) regardless of whether `compression` is
                // `BI_BITFIELDS` or `BI_ALPHABITFIELDS` — the latter is
                // documented as the "always-four-masks" cousin and the
                // former is the original three-mask variant that V3+
                // grew an alpha slot for; both reach the same bytes on
                // a V3+ header.
                let ma = if header_size >= BITMAPV3INFOHEADER_SIZE {
                    Some(read_u32_le(input, 52))
                } else {
                    None
                };
                (
                    Some(read_u32_le(input, 40)),
                    Some(read_u32_le(input, 44)),
                    Some(read_u32_le(input, 48)),
                    ma,
                )
            } else {
                // V3 (40-byte) header: masks live in the bytes immediately
                // following the header — 12 bytes (R/G/B) for `BI_BITFIELDS`
                // and 16 bytes (R/G/B/A) for `BI_ALPHABITFIELDS`. Read the
                // alpha mask only for the four-mask variant; for the three-
                // mask variant leave `mask_a = None` so the per-bpp decode
                // arms fall back to "opaque".
                let masks_bytes = if compression == BI_ALPHABITFIELDS {
                    16
                } else {
                    12
                };
                if input.len() < (BITMAPINFOHEADER_SIZE as usize) + masks_bytes {
                    let label = if compression == BI_ALPHABITFIELDS {
                        "BI_ALPHABITFIELDS needs 16 bytes of masks after header"
                    } else {
                        "BI_BITFIELDS needs 12 bytes of masks after header"
                    };
                    return Err(Error::invalid(format!("BMP: {label}")));
                }
                let ma = if compression == BI_ALPHABITFIELDS {
                    Some(read_u32_le(input, 52))
                } else {
                    None
                };
                (
                    Some(read_u32_le(input, 40)),
                    Some(read_u32_le(input, 44)),
                    Some(read_u32_le(input, 48)),
                    ma,
                )
            }
        } else if compression == BI_RGB && header_size >= BITMAPV4HEADER_SIZE {
            // V4 / V5 always reserve the four-mask block at offsets 40..56
            // inside the header body. The BMP spec is explicit that the
            // R / G / B masks are valid *only* under BI_BITFIELDS, but the
            // alpha mask "is valid whenever it is present in the DIB
            // header" (Wikipedia §"the BITFIELDS mechanism", which the
            // BITMAPV4HEADER doc's ARGB32 example corroborates). So under
            // BI_RGB we leave R / G / B at the default BGRA byte order
            // (`None` → the per-bpp arm uses the fixed positions) but read
            // the in-header alpha mask at offset 52: a non-zero value means
            // the writer genuinely populated the high byte with alpha, and
            // a zero value means "no alpha" → the documented opaque
            // fall-back the BI_ALPHABITFIELDS / V3-alpha paths already use.
            (None, None, None, Some(read_u32_le(input, 52)))
        } else {
            (None, None, None, None)
        };

    // ---- V4 / V5 colour-space tail ------------------------------------
    //
    // V4 (header_size >= 108) adds — after the four R/G/B/A masks at
    // offsets 40..56 — a `bV4CSType` u32 at offset 56, a CIEXYZTRIPLE of
    // 9 × i32 endpoints at offsets 60..96, and a 3-u32 gamma triple at
    // 96..108. V5 (header_size >= 124) extends that with `bV5Intent` at
    // 108, `bV5ProfileData` at 112, `bV5ProfileSize` at 116, and a
    // reserved u32 at 120.
    let (cs_type, endpoints, gamma_rgb) = if header_size >= BITMAPV4HEADER_SIZE {
        let cs = read_u32_le(input, 56);
        let mut ep = [0i32; 9];
        for (i, slot) in ep.iter_mut().enumerate() {
            *slot = read_i32_le(input, 60 + i * 4);
        }
        let gr = [
            read_u32_le(input, 96),
            read_u32_le(input, 100),
            read_u32_le(input, 104),
        ];
        (Some(cs), Some(ep), Some(gr))
    } else {
        (None, None, None)
    };
    let (intent, profile_data_offset, profile_size) = if header_size >= BITMAPV5HEADER_SIZE {
        (
            Some(read_u32_le(input, 108)),
            Some(read_u32_le(input, 112)),
            Some(read_u32_le(input, 116)),
        )
    } else {
        (None, None, None)
    };

    // ---- OS/2 2.x full 64-byte OS22XBITMAPHEADER trailing block --------
    //
    // The full IBM header appends 24 bytes after the 40-byte
    // BITMAPINFOHEADER prefix. It is identified solely by `biSize == 64`
    // (the Windows V4 / V5 headers are 108 / 124 and never collide).
    // We read it only at exactly that size: a 64-byte V4-prefix would be
    // ambiguous, but no Windows generation declares biSize 64, so 64 is
    // unambiguously the OS/2 2.x form. The masks / cs_type tail above is
    // gated on >= 108, so a 64-byte header never reaches that path and
    // the colour-space fields stay `None` as expected.
    let os2_header2 = if header_size == OS22XBITMAPHEADER_SIZE {
        Some(Os2Header2Raw {
            units: read_u16_le(input, 40),
            // offset 42 is documented padding (ignored).
            recording: read_u16_le(input, 44),
            rendering: read_u16_le(input, 46),
            size1: read_u32_le(input, 48),
            size2: read_u32_le(input, 52),
            color_encoding: read_u32_le(input, 56),
            identifier: read_u32_le(input, 60),
        })
    } else {
        None
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
            cs_type,
            endpoints,
            gamma_rgb,
            intent,
            profile_data_offset,
            profile_size,
            os2_header2,
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
            cs_type: None,
            endpoints: None,
            gamma_rgb: None,
            intent: None,
            profile_data_offset: None,
            profile_size: None,
            os2_header2: None,
        },
        BITMAPCOREHEADER_SIZE as usize,
    ))
}

/// Parse a *truncated* OS/2 2.x `OS22XBITMAPHEADER` (`biSize` in
/// `16..40`) into a [`DibHeader`].
///
/// The OS/2 2.x header shares the 40-byte `BITMAPINFOHEADER` field
/// layout — 4-byte signed width/height, 2-byte planes, 2-byte
/// bit-count, then `biCompression` / `biSizeImage` / resolution /
/// palette counts — but a writer may stop the header early and have the
/// remaining fields read as zero. The 16-byte form (size/width/height/
/// planes/bit-count only) is the canonical case; the BMP Suite's
/// `pal8os2v2-16.bmp` is encoded this way. We read each field only when
/// `header_size` is long enough to include it and default the rest to
/// zero (the same "trailing fields are zero" rule the spec describes).
///
/// Layout (offsets shared with `BITMAPINFOHEADER`):
/// ```text
///   off  0  u32  biSize        (16..40 here)
///   off  4  i32  biWidth
///   off  8  i32  biHeight      (signed — top-down when negative)
///   off 12  u16  biPlanes      (must be 1)
///   off 14  u16  biBitCount
///   off 16  u32  biCompression (present only when header_size >= 20)
///   off 20  u32  biSizeImage   (present only when header_size >= 24)
///   off 24  i32  biXPelsPerM   (present only when header_size >= 28)
///   off 28  i32  biYPelsPerM   (present only when header_size >= 32)
///   off 32  u32  biClrUsed     (present only when header_size >= 36)
///   off 36  u32  biClrImportant(present only when header_size >= 40)
/// ```
fn parse_truncated_os22x_header(input: &[u8], header_size: u32) -> Result<(DibHeader, usize)> {
    let hs = header_size as usize;
    if input.len() < hs {
        return Err(Error::invalid("BMP: OS22XBITMAPHEADER truncated"));
    }
    // Read a field that lives at `[off, off + 4)` only when the declared
    // header is long enough to fully contain it; otherwise the field is
    // absent and reads as zero per the truncated-header convention.
    let opt_u32 = |off: usize| -> u32 {
        if hs >= off + 4 {
            read_u32_le(input, off)
        } else {
            0
        }
    };
    let opt_i32 = |off: usize| -> i32 {
        if hs >= off + 4 {
            read_i32_le(input, off)
        } else {
            0
        }
    };

    let width = read_i32_le(input, 4);
    let height = read_i32_le(input, 8);
    let planes = read_u16_le(input, 12);
    let bpp = read_u16_le(input, 14);
    let compression = opt_u32(16);
    let image_size = opt_u32(20);
    let x_pels_per_meter = opt_i32(24);
    let y_pels_per_meter = opt_i32(28);
    let clr_used = opt_u32(32);
    let clr_important = opt_u32(36);

    if width <= 0 {
        return Err(Error::invalid("BMP: non-positive width"));
    }
    if planes != 1 {
        return Err(Error::invalid(format!("BMP: planes={planes} (must be 1)")));
    }
    // A truncated OS/2 2.x header has no room for the appended R/G/B(/A)
    // bitfield mask block (that block sits between the header and the
    // pixel array on a 40-byte `BITMAPINFOHEADER`, but here the header
    // never reaches 40 bytes). `Huffman 1D` (the OS/2 alias for
    // compression value 3) and `RLE-24` (value 4) are likewise
    // undecodable here. Reject anything other than the plain `BI_RGB` /
    // `BI_RLE8` / `BI_RLE4` stream a truncated header can legally carry.
    match compression {
        BI_RGB | BI_RLE8 | BI_RLE4 => {}
        c => {
            return Err(Error::invalid(format!(
                "BMP: truncated OS22XBITMAPHEADER cannot carry compression {c}"
            )));
        }
    }

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
            mask_r: None,
            mask_g: None,
            mask_b: None,
            mask_a: None,
            cs_type: None,
            endpoints: None,
            gamma_rgb: None,
            intent: None,
            profile_data_offset: None,
            profile_size: None,
            os2_header2: None,
        },
        hs,
    ))
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

/// Canonical DIB-relative offset of the pixel array: the DIB header,
/// followed by the appended bit-field mask block (V3 `BI_BITFIELDS` /
/// `BI_ALPHABITFIELDS` only — V4/V5 carry the masks inside the header
/// body), followed by the colour table. This is the byte position the
/// pixel bits occupy in the standard, gap-free BMP layout the spec's
/// "Bitmap Storage" section describes (header → masks → colour table →
/// pixels). It is measured from the start of the DIB header, so a
/// headerless DIB uses it directly and a BMP file adds the 14-byte
/// `BITMAPFILEHEADER` (see [`canonical_file_pixel_offset`]).
///
/// All arithmetic saturates because the colour-table size is bounded
/// only by the attacker-controlled `biClrUsed` (up to `u32::MAX`); the
/// downstream pixel/palette bounds checks reject an implausibly large
/// result.
fn canonical_dib_pixel_offset(h: &DibHeader) -> usize {
    let entry_size = palette_entry_bytes(h);
    let color_table_bytes = h.palette_entries().saturating_mul(entry_size);
    // The appended mask block exists only on the 40-byte V3 header; the
    // intermediate Adobe V2 (52 B) / V3 (56 B) and the V4 / V5 headers
    // all carry their masks inside the header body.
    let masks_bytes = if h.header_size == BITMAPINFOHEADER_SIZE {
        match h.compression {
            BI_BITFIELDS => 12usize,
            BI_ALPHABITFIELDS => 16usize,
            _ => 0usize,
        }
    } else {
        0usize
    };
    (h.header_size as usize)
        .saturating_add(masks_bytes)
        .saturating_add(color_table_bytes)
}

/// Canonical file-relative pixel-array offset: the 14-byte
/// `BITMAPFILEHEADER` plus [`canonical_dib_pixel_offset`].
fn canonical_file_pixel_offset(h: &DibHeader) -> usize {
    (BITMAPFILEHEADER_SIZE as usize).saturating_add(canonical_dib_pixel_offset(h))
}

/// Resolve the pixel-array offset for a BMP *file* given the stored
/// `bfOffBits` and the parsed DIB header.
///
/// `bfOffBits` is the spec-blessed source of truth (it lets a writer
/// leave a gap between the colour table and the pixels), so a value that
/// lands at or past the canonical position is honoured verbatim. But a
/// non-trivial population of in-the-wild minimal encoders write
/// `bfOffBits = 0` (the field is simply left unset), and a corrupt or
/// lazy writer can leave a value that points *inside* the header /
/// colour table. Either way the stored offset cannot be where the pixels
/// actually start, so we fall back to the canonical layout the
/// "Bitmap Storage" section defines (file header → DIB header → masks →
/// colour table → pixels). This only ever moves the read forward to the
/// earliest byte the pixels could legally occupy; a larger, valid
/// `bfOffBits` is left untouched so legitimate gaps still work.
fn resolve_file_pixel_offset(stored: usize, h: &DibHeader) -> usize {
    let canonical = canonical_file_pixel_offset(h);
    if stored >= canonical {
        stored
    } else {
        canonical
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

/// A 256-entry per-channel expansion table for the `BI_BITFIELDS` /
/// `BI_RGB` mask paths.
///
/// `table[sub]` holds the 8-bit expansion of the `n`-bit sub-value
/// `sub`, where `sub = ((pixel & mask) >> shift) as u8` is the raw
/// channel sample already extracted from the packed pixel. Building a
/// full 256-byte table for each channel turns the per-pixel `expand()`
/// `match` (four branches per pixel) into a single L1-resident indexed
/// load. All four channel tables together are 1 KiB — cache-friendly at
/// any image size, unlike a combined 256 KiB value→RGBA table whose
/// random-access decode loop thrashes L2 on scattered pixel values.
///
/// The bytes are bit-identical to the old per-pixel path: `table[i]` is
/// exactly `expand(i as u8, n)`, and the `as u8` truncation on lookup
/// matches the `(… ) as u8` the direct path applied, so a mask wider
/// than 8 bits (`n >= 8`) resolves through the same low-8 truncation.
struct ChannelLut {
    mask: u32,
    shift: u32,
    table: [u8; 256],
}

impl ChannelLut {
    fn new(mask: u32) -> Self {
        let (shift, n) = shift_len(mask);
        let mut table = [0u8; 256];
        for (i, t) in table.iter_mut().enumerate() {
            *t = expand(i as u8, n);
        }
        Self { mask, shift, table }
    }

    #[inline(always)]
    fn get(&self, v: u32) -> u8 {
        // `as u8` truncates the extracted sub-value to the low 8 bits
        // (matching the historical direct path) so the index is always
        // in `0..256` and the bounds check is elided.
        self.table[((v & self.mask) >> self.shift) as u8 as usize]
    }
}
