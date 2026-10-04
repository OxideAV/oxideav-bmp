//! The image-crate contract records (`IMAGE_CRATE_API.md`): the
//! native-layout [`BmpImage`], its pixel-format tag, the shared
//! [`Plane`] / [`Palette`] / [`ColorInfo`] / [`Metadata`] records, the
//! raw [`RgbImage`] / [`RgbaImage`] results and the header-only
//! [`ImageInfo`].
//!
//! Everything here builds without `oxideav-core`; the `registry`
//! feature adds the `VideoFrame` bridge in [`crate::registry`].

/// Pixel layout of a [`BmpImage`].
///
/// Variant names mirror `oxideav_core::PixelFormat` where the core enum
/// has the layout (`Pal8`, `Bgr24`, `Bgra`, `Rgb24`, `Rgba`); the 16-bit
/// and sub-byte indexed layouts are BMP-specific and keep their own
/// names.
///
/// **Decode** ([`crate::decode`]) returns the file's native layout:
///
/// | On disk | Native layout |
/// |---|---|
/// | 1 / 2 / 4 / 8 bpp `BI_RGB`, `BI_RLE4`, `BI_RLE8` | [`Pal8`](Self::Pal8) — one index byte per pixel (sub-byte depths are unpacked to 8-bit indices), colour table in [`BmpImage::palette`] |
/// | 16 bpp `BI_RGB`, or `BI_BITFIELDS` with the 5-5-5 masks | [`Rgb555`](Self::Rgb555) |
/// | 16 bpp `BI_BITFIELDS` with the 5-6-5 masks | [`Rgb565`](Self::Rgb565) |
/// | 24 bpp `BI_RGB` | [`Bgr24`](Self::Bgr24) |
/// | 32 bpp `BI_RGB`, or `BI_BITFIELDS` with the byte-aligned B,G,R(,A) masks | [`Bgra`](Self::Bgra) |
/// | 32 bpp `BI_BITFIELDS` with the byte-aligned R,G,B(,A) masks | [`Rgba`](Self::Rgba) |
/// | any other 16 / 32 bpp mask set (including 16 bpp alpha masks) | [`Rgba`](Self::Rgba), every channel expanded to 8 bits by bit replication |
///
/// Rows are always delivered top-down and tightly packed (no DWORD
/// padding); bottom-up files are flipped while decoding.
///
/// **Encode** ([`crate::encode`]) accepts every variant: `Rgba` /
/// `Bgra` → 32 bpp `BI_RGB`; `Rgb24` / `Bgr24` → 24 bpp `BI_RGB`;
/// `Rgb555` → 16 bpp `BI_RGB`; `Rgb565` → 16 bpp `BI_BITFIELDS` (V4
/// header); `Pal8` / `Indexed4` / `Indexed2` / `Indexed1` → the indexed
/// bitmap of that depth (RLE8 / RLE4 when smaller, see
/// [`crate::EncodeOptions::rle`]). The three sub-byte indexed variants
/// are encode-side depth selectors: the plane still carries one index
/// byte per pixel, the encoder packs it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum BmpPixelFormat {
    /// Packed 8-bit RGBA, 4 bytes per pixel.
    Rgba,
    /// Packed 8-bit RGB, 3 bytes per pixel.
    Rgb24,
    /// Packed 8-bit BGRA, 4 bytes per pixel — BMP's native 32 bpp
    /// order. The fourth byte is the file's fourth byte: alpha when
    /// the header declares an alpha mask, `0xFF` when a V4 / V5 header
    /// declares none, and the stored (possibly zero) byte for a plain
    /// V3 `BI_RGB` bitmap, which has no way to say.
    Bgra,
    /// Packed 8-bit BGR, 3 bytes per pixel — BMP's native 24 bpp order.
    Bgr24,
    /// 16-bit RGB 5-5-5, little-endian `u16` per pixel: bit 15
    /// reserved, R in bits 14..10, G in 9..5, B in 4..0.
    Rgb555,
    /// 16-bit RGB 5-6-5, little-endian `u16` per pixel: R in bits
    /// 15..11, G in 10..5, B in 4..0.
    Rgb565,
    /// 8-bit palette index, 1 byte per pixel; the colour table is
    /// [`BmpImage::palette`].
    Pal8,
    /// 4-bit palette index as 1 byte per pixel (values `0..=15`); the
    /// encoder packs two pixels per byte, high nibble first.
    Indexed4,
    /// 2-bit palette index as 1 byte per pixel (values `0..=3`); the
    /// encoder packs four pixels per byte, leftmost in the two most
    /// significant bits (the Windows CE 4-colour layout).
    Indexed2,
    /// 1-bit palette index as 1 byte per pixel (values `0` / `1`); the
    /// encoder packs eight pixels per byte, MSB first.
    Indexed1,
}

/// Contract alias: `oxideav_bmp::PixelFormat` is [`BmpPixelFormat`].
pub type PixelFormat = BmpPixelFormat;

impl BmpPixelFormat {
    /// The pre-contract name of [`BmpPixelFormat::Pal8`].
    #[deprecated(note = "use BmpPixelFormat::Pal8 (IMAGE_CRATE_API naming)")]
    #[allow(non_upper_case_globals)]
    pub const Indexed8: Self = Self::Pal8;

    /// Bytes per pixel of the in-memory plane (`1` for every indexed
    /// variant, which are one index byte per pixel).
    pub const fn bytes_per_pixel(self) -> usize {
        match self {
            Self::Rgba | Self::Bgra => 4,
            Self::Rgb24 | Self::Bgr24 => 3,
            Self::Rgb555 | Self::Rgb565 => 2,
            Self::Pal8 | Self::Indexed4 | Self::Indexed2 | Self::Indexed1 => 1,
        }
    }

    /// Bits per pixel the layout occupies on disk.
    pub const fn bits_per_pixel(self) -> u16 {
        match self {
            Self::Rgba | Self::Bgra => 32,
            Self::Rgb24 | Self::Bgr24 => 24,
            Self::Rgb555 | Self::Rgb565 => 16,
            Self::Pal8 => 8,
            Self::Indexed4 => 4,
            Self::Indexed2 => 2,
            Self::Indexed1 => 1,
        }
    }

    /// `true` for the layouts that carry an alpha byte (`Rgba`, `Bgra`).
    pub const fn has_alpha(self) -> bool {
        matches!(self, Self::Rgba | Self::Bgra)
    }

    /// `true` if the format indexes a colour table (`Pal8`, `Indexed4`,
    /// `Indexed2`, `Indexed1`).
    pub const fn is_indexed(self) -> bool {
        matches!(
            self,
            Self::Pal8 | Self::Indexed4 | Self::Indexed2 | Self::Indexed1
        )
    }

    /// Index mask applied to an index byte of this layout (`0xFF` for
    /// `Pal8`, `0x0F` / `0x03` / `0x01` for the sub-byte depths, `0` for
    /// direct colour).
    pub(crate) const fn index_mask(self) -> u8 {
        match self {
            Self::Pal8 => 0xFF,
            Self::Indexed4 => 0x0F,
            Self::Indexed2 => 0x03,
            Self::Indexed1 => 0x01,
            _ => 0,
        }
    }
}

/// One pixel plane: `stride` bytes per row, `data` holding at least
/// `stride × height` bytes. Every BMP layout is packed, so a
/// [`BmpImage`] has exactly one.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Plane {
    /// Bytes per row.
    pub stride: usize,
    /// Row-major bytes, `stride × height` long.
    pub data: Vec<u8>,
}

impl Plane {
    /// Wrap a plane buffer with its row stride.
    pub fn new(stride: usize, data: Vec<u8>) -> Self {
        Self { stride, data }
    }
}

/// The pre-contract name of [`Plane`] (same fields).
#[deprecated(note = "use oxideav_bmp::Plane (IMAGE_CRATE_API)")]
pub type BmpPlane = Plane;

/// Colour table of an indexed image: RGBA entries, index `i` at
/// `entries[i]`. BMP colour tables have no alpha (`RGBQUAD.rgbReserved`
/// is reserved), so decoded entries carry `a = 255`; the encoder writes
/// `0` into the reserved byte regardless of the entry's alpha.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct Palette {
    /// `[r, g, b, a]` per entry, at most 256 entries.
    pub entries: Vec<[u8; 4]>,
}

impl Palette {
    /// Wrap a list of RGBA entries.
    pub fn new(entries: Vec<[u8; 4]>) -> Self {
        Self { entries }
    }

    /// Build an opaque palette from `[r, g, b]` triplets.
    pub fn from_rgb(rgb: &[[u8; 3]]) -> Self {
        Self {
            entries: rgb.iter().map(|c| [c[0], c[1], c[2], 0xFF]).collect(),
        }
    }

    /// Number of entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// `true` when the table has no entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Entry `index`, if present.
    pub fn get(&self, index: u8) -> Option<[u8; 4]> {
        self.entries.get(index as usize).copied()
    }

    /// The entries as packed `R, G, B` triplets (the `VideoFrame`
    /// palette side-channel layout).
    pub fn to_rgb(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.entries.len() * 3);
        for e in &self.entries {
            out.extend_from_slice(&e[..3]);
        }
        out
    }
}

/// The pre-contract palette: `[r, g, b]` entries (no alpha). Converts
/// into [`Palette`] with `a = 255`.
#[deprecated(note = "use oxideav_bmp::Palette (IMAGE_CRATE_API)")]
#[derive(Debug, Clone, Default)]
pub struct BmpPalette {
    /// Colour entries in `[R, G, B]` order.
    pub entries: Vec<[u8; 3]>,
}

#[allow(deprecated)]
impl From<BmpPalette> for Palette {
    fn from(p: BmpPalette) -> Self {
        Palette::from_rgb(&p.entries)
    }
}

#[allow(deprecated)]
impl From<&BmpPalette> for Palette {
    fn from(p: &BmpPalette) -> Self {
        Palette::from_rgb(&p.entries)
    }
}

#[allow(deprecated)]
impl From<Palette> for BmpPalette {
    fn from(p: Palette) -> Self {
        BmpPalette {
            entries: p.entries.iter().map(|e| [e[0], e[1], e[2]]).collect(),
        }
    }
}

/// Nominal sample range (H.273 `VideoFullRangeFlag`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum ColorRange {
    /// No range was signalled.
    #[default]
    Unspecified,
    /// Limited (video / studio) range.
    Limited,
    /// Full (PC) range.
    Full,
}

/// Colour signalling of an image: the sample range plus the H.273
/// `ColourPrimaries` / `TransferCharacteristics` / `MatrixCoefficients`
/// code points (`2` = unspecified).
///
/// BMP carries colour signalling only in V4 / V5 headers (`bV4CSType` /
/// `bV5CSType`): `LCS_sRGB` and `LCS_WINDOWS_COLOR_SPACE` ("the system
/// default color space, sRGB") decode to [`ColorInfo::srgb`]; every
/// other case — V3 / OS/2 headers, `LCS_CALIBRATED_RGB` (whose CIE
/// endpoints and gamma triple stay in [`crate::BmpMetadata`]), and
/// the two ICC profile modes — decodes to [`ColorInfo::bmp_default`]:
/// full-range RGB with unspecified primaries and transfer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct ColorInfo {
    /// Sample range.
    pub range: ColorRange,
    /// H.273 `ColourPrimaries` code point (`1` = BT.709 / sRGB, `2` =
    /// unspecified).
    pub primaries: u8,
    /// H.273 `TransferCharacteristics` code point (`13` = sRGB, `2` =
    /// unspecified).
    pub transfer: u8,
    /// H.273 `MatrixCoefficients` code point (`0` = identity / RGB).
    pub matrix: u8,
}

impl ColorInfo {
    /// H.273 "unspecified" code point.
    pub const UNSPECIFIED: u8 = 2;
    /// H.273 `MatrixCoefficients` identity (RGB) code point.
    pub const MATRIX_IDENTITY: u8 = 0;
    /// H.273 `ColourPrimaries` BT.709 / sRGB code point.
    pub const PRIMARIES_BT709: u8 = 1;
    /// H.273 `TransferCharacteristics` IEC 61966-2-1 sRGB code point.
    pub const TRANSFER_SRGB: u8 = 13;

    /// Build a description from its four parts.
    pub const fn new(range: ColorRange, primaries: u8, transfer: u8, matrix: u8) -> Self {
        Self {
            range,
            primaries,
            transfer,
            matrix,
        }
    }

    /// Every field unspecified.
    pub const fn unspecified() -> Self {
        Self::new(
            ColorRange::Unspecified,
            Self::UNSPECIFIED,
            Self::UNSPECIFIED,
            Self::UNSPECIFIED,
        )
    }

    /// BMP's default when the header carries no usable colour-space
    /// tag: full-range device RGB (`matrix` 0), primaries and transfer
    /// unspecified.
    pub const fn bmp_default() -> Self {
        Self::new(
            ColorRange::Full,
            Self::UNSPECIFIED,
            Self::UNSPECIFIED,
            Self::MATRIX_IDENTITY,
        )
    }

    /// sRGB (IEC 61966-2-1): BT.709 primaries, sRGB transfer, identity
    /// matrix, full range — what `LCS_sRGB` / `LCS_WINDOWS_COLOR_SPACE`
    /// signal.
    pub const fn srgb() -> Self {
        Self::new(
            ColorRange::Full,
            Self::PRIMARIES_BT709,
            Self::TRANSFER_SRGB,
            Self::MATRIX_IDENTITY,
        )
    }

    /// Set the range.
    pub fn with_range(mut self, range: ColorRange) -> Self {
        self.range = range;
        self
    }

    /// Set the primaries code point.
    pub fn with_primaries(mut self, primaries: u8) -> Self {
        self.primaries = primaries;
        self
    }

    /// Set the transfer code point.
    pub fn with_transfer(mut self, transfer: u8) -> Self {
        self.transfer = transfer;
        self
    }

    /// Set the matrix code point.
    pub fn with_matrix(mut self, matrix: u8) -> Self {
        self.matrix = matrix;
        self
    }

    /// `true` when both primaries and transfer are specified (`!= 2`).
    pub fn is_specified(&self) -> bool {
        self.primaries != Self::UNSPECIFIED && self.transfer != Self::UNSPECIFIED
    }
}

impl Default for ColorInfo {
    /// [`ColorInfo::bmp_default`].
    fn default() -> Self {
        Self::bmp_default()
    }
}

/// The metadata blobs every image crate surfaces. BMP can carry only
/// the ICC profile (V5 `PROFILE_EMBEDDED`); `exif` and `xmp` are always
/// `None`, and `gamma` is `None` too — the V4 per-channel Q16.16
/// "tone response curve" triple has no documented relation to the
/// single file-gamma convention this field uses, so it stays in
/// [`crate::BmpMetadata::gamma_rgb`].
#[derive(Debug, Clone, PartialEq, Default)]
#[non_exhaustive]
pub struct Metadata {
    /// Embedded ICC profile bytes (V5 `PROFILE_EMBEDDED`).
    pub icc: Option<Vec<u8>>,
    /// Exif payload — BMP has none; always `None`.
    pub exif: Option<Vec<u8>>,
    /// XMP packet — BMP has none; always `None`.
    pub xmp: Option<Vec<u8>>,
    /// File gamma — not populated for BMP (see the type docs).
    pub gamma: Option<f32>,
}

impl Metadata {
    /// Empty metadata.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set (or clear) the ICC profile.
    pub fn with_icc(mut self, icc: impl Into<Option<Vec<u8>>>) -> Self {
        self.icc = icc.into();
        self
    }

    /// Set (or clear) the Exif payload.
    pub fn with_exif(mut self, exif: impl Into<Option<Vec<u8>>>) -> Self {
        self.exif = exif.into();
        self
    }

    /// Set (or clear) the XMP packet.
    pub fn with_xmp(mut self, xmp: impl Into<Option<Vec<u8>>>) -> Self {
        self.xmp = xmp.into();
        self
    }

    /// Set (or clear) the file gamma.
    pub fn with_gamma(mut self, gamma: impl Into<Option<f32>>) -> Self {
        self.gamma = gamma.into();
        self
    }

    /// `true` when no field is set.
    pub fn is_empty(&self) -> bool {
        self.icc.is_none() && self.exif.is_none() && self.xmp.is_none() && self.gamma.is_none()
    }
}

/// A decoded (or to-be-encoded) BMP picture in its native layout.
///
/// `planes` holds exactly one packed plane; `color` and `metadata` are
/// filled from the header; `palette` is `Some` for the indexed
/// layouts. Rows run top-down regardless of the file's `biHeight`
/// sign.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct BmpImage {
    /// Picture width in pixels.
    pub width: u32,
    /// Picture height in pixels.
    pub height: u32,
    /// Native pixel layout.
    pub format: PixelFormat,
    /// Pixel planes — exactly one for BMP.
    pub planes: Vec<Plane>,
    /// Colour signalling (range + H.273 code points).
    pub color: ColorInfo,
    /// ICC profile (the only metadata blob BMP carries).
    pub metadata: Metadata,
    /// Colour table for the indexed layouts; `None` for direct colour.
    pub palette: Option<Palette>,
}

impl BmpImage {
    /// Assemble an image from its geometry, layout and planes (exactly
    /// one for BMP), validating the geometry: non-zero dimensions, one
    /// plane whose `stride` covers `width × bytes_per_pixel` and whose
    /// `data` covers `stride × height`. Colour is
    /// [`ColorInfo::bmp_default`], metadata empty, no palette; the
    /// `with_*` builders fill those in. An indexed image additionally
    /// needs [`Self::with_palette`] before it can be encoded.
    pub fn new(
        width: u32,
        height: u32,
        format: PixelFormat,
        planes: Vec<Plane>,
    ) -> crate::error::Result<Self> {
        use crate::error::BmpError as Error;
        if width == 0 || height == 0 {
            return Err(Error::invalid("BMP image: zero dimension"));
        }
        if planes.len() != 1 {
            return Err(Error::invalid(format!(
                "BMP image: expected exactly one plane, got {}",
                planes.len()
            )));
        }
        let plane = &planes[0];
        let min_stride = (width as usize).saturating_mul(format.bytes_per_pixel());
        if plane.stride < min_stride {
            return Err(Error::invalid(format!(
                "BMP image: stride {} is below the {} bytes a {}-pixel row of {:?} needs",
                plane.stride, min_stride, width, format
            )));
        }
        let needed = plane.stride.saturating_mul(height as usize);
        if plane.data.len() < needed {
            return Err(Error::invalid(format!(
                "BMP image: plane holds {} bytes, {} × {} rows need {}",
                plane.data.len(),
                plane.stride,
                height,
                needed
            )));
        }
        Ok(Self {
            width,
            height,
            format,
            planes,
            color: ColorInfo::bmp_default(),
            metadata: Metadata::default(),
            palette: None,
        })
    }

    /// One packed plane with an explicit row stride (validated like
    /// [`Self::new`]).
    pub fn packed(
        width: u32,
        height: u32,
        format: PixelFormat,
        stride: usize,
        data: Vec<u8>,
    ) -> crate::error::Result<Self> {
        Self::new(width, height, format, vec![Plane::new(stride, data)])
    }

    /// Tightly packed `Rgb24` from `3 × width × height` bytes (stride
    /// `3 × width`), validated like [`Self::new`]: a zero dimension or a
    /// short buffer is `Error::InvalidData`.
    pub fn from_rgb8(width: u32, height: u32, data: Vec<u8>) -> crate::error::Result<Self> {
        Self::packed(
            width,
            height,
            PixelFormat::Rgb24,
            (width as usize).saturating_mul(3),
            data,
        )
    }

    /// Tightly packed `Rgba` from `4 × width × height` bytes (stride
    /// `4 × width`), validated like [`Self::new`].
    pub fn from_rgba8(width: u32, height: u32, data: Vec<u8>) -> crate::error::Result<Self> {
        Self::packed(
            width,
            height,
            PixelFormat::Rgba,
            (width as usize).saturating_mul(4),
            data,
        )
    }

    /// Set the colour signalling.
    pub fn with_color(mut self, color: ColorInfo) -> Self {
        self.color = color;
        self
    }

    /// Set the metadata.
    pub fn with_metadata(mut self, metadata: Metadata) -> Self {
        self.metadata = metadata;
        self
    }

    /// Set (or clear) the palette.
    pub fn with_palette(mut self, palette: impl Into<Option<Palette>>) -> Self {
        self.palette = palette.into();
        self
    }

    /// Picture width in pixels.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Picture height in pixels.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Native pixel layout.
    pub fn format(&self) -> PixelFormat {
        self.format
    }

    /// Bytes per pixel of [`Self::format`].
    pub fn bytes_per_pixel(&self) -> usize {
        self.format.bytes_per_pixel()
    }

    /// Row stride in bytes of the pixel plane (`0` if the image has no
    /// plane).
    pub fn stride(&self) -> usize {
        self.planes.first().map(|p| p.stride).unwrap_or(0)
    }

    /// The pixel bytes — `Some` for every BMP image that has its plane
    /// (all BMP layouts are packed), `None` only for an image built
    /// without planes.
    pub fn as_bytes(&self) -> Option<&[u8]> {
        self.planes.first().map(|p| p.data.as_slice())
    }

    /// Consume the image and return its plane bytes (planes
    /// concatenated in order, strides as reported).
    pub fn into_raw(self) -> Vec<u8> {
        let mut planes = self.planes.into_iter();
        let mut out = planes.next().map(|p| p.data).unwrap_or_default();
        for p in planes {
            out.extend_from_slice(&p.data);
        }
        out
    }

    /// `true` when the layout carries an alpha byte (`Rgba` / `Bgra`);
    /// BMP palettes are opaque, so indexed images never do.
    pub fn has_alpha(&self) -> bool {
        self.format.has_alpha()
    }

    /// Tightly packed 8-bit RGB (`3 × width × height` bytes), exact
    /// for every native layout: palette entries looked up (missing
    /// entries read as opaque black), 5- and 6-bit channels widened by
    /// bit replication (`0b11111` → `0xFF`), BGR orders swizzled. Rows
    /// beyond the plane read as zero, so the call never panics.
    pub fn to_rgb8(&self) -> Vec<u8> {
        self.convert(3)
    }

    /// Tightly packed 8-bit RGBA (`4 × width × height` bytes): as
    /// [`Self::to_rgb8`] with the alpha byte of `Rgba` / `Bgra` kept
    /// as stored, `0xFF` for every other layout.
    pub fn to_rgba8(&self) -> Vec<u8> {
        self.convert(4)
    }

    /// [`Self::to_rgb8`] that reports a geometry mismatch instead of
    /// zero-filling (for images not produced by this crate's decoder).
    pub fn try_to_rgb8(&self) -> crate::error::Result<Vec<u8>> {
        self.check_geometry()?;
        Ok(self.to_rgb8())
    }

    /// [`Self::to_rgba8`] that reports a geometry mismatch instead of
    /// zero-filling.
    pub fn try_to_rgba8(&self) -> crate::error::Result<Vec<u8>> {
        self.check_geometry()?;
        Ok(self.to_rgba8())
    }

    /// The same picture re-laid as `Rgba` (palette dropped, colour and
    /// metadata kept) — what the pre-contract decode entry points and
    /// the framework decoder hand out.
    pub(crate) fn into_rgba(self) -> Self {
        let data = self.to_rgba8();
        Self {
            width: self.width,
            height: self.height,
            format: PixelFormat::Rgba,
            planes: vec![Plane::new(self.width as usize * 4, data)],
            color: self.color,
            metadata: self.metadata,
            palette: None,
        }
    }

    fn check_geometry(&self) -> crate::error::Result<()> {
        use crate::error::BmpError as Error;
        let plane = self
            .planes
            .first()
            .ok_or_else(|| Error::invalid("BMP image: no plane"))?;
        let min_stride = (self.width as usize).saturating_mul(self.format.bytes_per_pixel());
        if plane.stride < min_stride
            || plane.data.len() < plane.stride.saturating_mul(self.height as usize)
        {
            return Err(Error::invalid("BMP image: plane geometry mismatch"));
        }
        if self.format.is_indexed() && self.palette.is_none() {
            return Err(Error::invalid("BMP image: indexed layout without palette"));
        }
        Ok(())
    }

    /// Shared RGB / RGBA conversion kernel. `out_bpp` is 3 or 4.
    fn convert(&self, out_bpp: usize) -> Vec<u8> {
        let w = self.width as usize;
        let h = self.height as usize;
        let mut out = vec![0u8; w.saturating_mul(h).saturating_mul(out_bpp)];
        let Some(plane) = self.planes.first() else {
            return out;
        };
        let in_bpp = self.format.bytes_per_pixel();
        let row_bytes = w.saturating_mul(in_bpp);
        // Opaque fill for the layouts without alpha.
        if out_bpp == 4 && !self.format.has_alpha() {
            for px in out.chunks_exact_mut(4) {
                px[3] = 0xFF;
            }
        }
        let pal: [[u8; 4]; 256] = if self.format.is_indexed() {
            padded_palette(self.palette.as_ref().map(|p| p.entries.as_slice()))
        } else {
            [[0, 0, 0, 0xFF]; 256]
        };
        let index_mask = self.format.index_mask();
        for y in 0..h {
            let start = y.saturating_mul(plane.stride);
            let Some(end) = start.checked_add(row_bytes) else {
                break;
            };
            if end > plane.data.len() {
                break;
            }
            let src = &plane.data[start..end];
            let dst = &mut out[y * w * out_bpp..(y + 1) * w * out_bpp];
            match self.format {
                PixelFormat::Rgba => {
                    for (d, s) in dst.chunks_exact_mut(out_bpp).zip(src.chunks_exact(4)) {
                        d[..out_bpp].copy_from_slice(&s[..out_bpp]);
                    }
                }
                PixelFormat::Rgb24 => {
                    for (d, s) in dst.chunks_exact_mut(out_bpp).zip(src.chunks_exact(3)) {
                        d[..3].copy_from_slice(s);
                    }
                }
                PixelFormat::Bgra => {
                    for (d, s) in dst.chunks_exact_mut(out_bpp).zip(src.chunks_exact(4)) {
                        d[0] = s[2];
                        d[1] = s[1];
                        d[2] = s[0];
                        if out_bpp == 4 {
                            d[3] = s[3];
                        }
                    }
                }
                PixelFormat::Bgr24 => {
                    for (d, s) in dst.chunks_exact_mut(out_bpp).zip(src.chunks_exact(3)) {
                        d[0] = s[2];
                        d[1] = s[1];
                        d[2] = s[0];
                    }
                }
                PixelFormat::Rgb555 => {
                    for (d, s) in dst.chunks_exact_mut(out_bpp).zip(src.chunks_exact(2)) {
                        let v = u16::from_le_bytes([s[0], s[1]]);
                        d[0] = expand5(((v >> 10) & 0x1F) as u8);
                        d[1] = expand5(((v >> 5) & 0x1F) as u8);
                        d[2] = expand5((v & 0x1F) as u8);
                    }
                }
                PixelFormat::Rgb565 => {
                    for (d, s) in dst.chunks_exact_mut(out_bpp).zip(src.chunks_exact(2)) {
                        let v = u16::from_le_bytes([s[0], s[1]]);
                        d[0] = expand5(((v >> 11) & 0x1F) as u8);
                        d[1] = expand6(((v >> 5) & 0x3F) as u8);
                        d[2] = expand5((v & 0x1F) as u8);
                    }
                }
                PixelFormat::Pal8
                | PixelFormat::Indexed4
                | PixelFormat::Indexed2
                | PixelFormat::Indexed1 => {
                    for (d, &idx) in dst.chunks_exact_mut(out_bpp).zip(src.iter()) {
                        let e = &pal[(idx & index_mask) as usize];
                        d[..out_bpp].copy_from_slice(&e[..out_bpp]);
                    }
                }
            }
        }
        out
    }
}

/// Widen a 5-bit sample to 8 bits by bit replication (`v << 3 | v >> 2`).
#[inline]
pub(crate) const fn expand5(v: u8) -> u8 {
    (v << 3) | (v >> 2)
}

/// Widen a 6-bit sample to 8 bits by bit replication (`v << 2 | v >> 4`).
#[inline]
pub(crate) const fn expand6(v: u8) -> u8 {
    (v << 2) | (v >> 4)
}

/// A 256-entry lookup table: the palette's entries, then opaque black
/// for every index the table does not cover.
pub(crate) fn padded_palette(palette: Option<&[[u8; 4]]>) -> [[u8; 4]; 256] {
    let mut out = [[0u8, 0, 0, 0xFF]; 256];
    if let Some(p) = palette {
        for (o, e) in out.iter_mut().zip(p.iter()) {
            *o = *e;
        }
    }
    out
}

/// Tightly packed 8-bit RGB image: `width × height × 3` bytes,
/// row-major, channel order `R, G, B`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct RgbImage {
    /// Image width in pixels.
    pub width: u32,
    /// Image height in pixels.
    pub height: u32,
    /// `width × height × 3` bytes.
    pub data: Vec<u8>,
}

impl RgbImage {
    /// Wrap a tightly packed `width × height × 3` RGB buffer.
    pub fn new(width: u32, height: u32, data: Vec<u8>) -> Self {
        Self {
            width,
            height,
            data,
        }
    }

    /// The pixel bytes.
    pub fn as_bytes(&self) -> &[u8] {
        &self.data
    }

    /// Consume the image and return the pixel bytes.
    pub fn into_raw(self) -> Vec<u8> {
        self.data
    }

    /// Stride (bytes per row) — always `width × 3`.
    pub fn stride(&self) -> usize {
        self.width as usize * 3
    }
}

/// Tightly packed 8-bit RGBA image: `width × height × 4` bytes,
/// row-major, channel order `R, G, B, A`. Opaque source layouts are
/// promoted with `α = 255`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct RgbaImage {
    /// Image width in pixels.
    pub width: u32,
    /// Image height in pixels.
    pub height: u32,
    /// `width × height × 4` bytes.
    pub data: Vec<u8>,
}

impl RgbaImage {
    /// Wrap a tightly packed `width × height × 4` RGBA buffer.
    pub fn new(width: u32, height: u32, data: Vec<u8>) -> Self {
        Self {
            width,
            height,
            data,
        }
    }

    /// The pixel bytes.
    pub fn as_bytes(&self) -> &[u8] {
        &self.data
    }

    /// Consume the image and return the pixel bytes.
    pub fn into_raw(self) -> Vec<u8> {
        self.data
    }

    /// Stride (bytes per row) — always `width × 4`.
    pub fn stride(&self) -> usize {
        self.width as usize * 4
    }
}

/// What [`crate::info`] learns from the file and DIB headers without
/// touching the pixel array.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct ImageInfo {
    /// Picture width in pixels.
    pub width: u32,
    /// Picture height in pixels.
    pub height: u32,
    /// The layout [`crate::decode`] would return.
    pub format: PixelFormat,
    /// Number of images — always `1` for BMP.
    pub frames: u32,
    /// `true` when the decoded layout carries an alpha byte.
    pub has_alpha: bool,
    /// Colour signalling, resolved as [`crate::decode`] would.
    pub color: ColorInfo,
    /// A V5 header declares an embedded ICC profile of non-zero size.
    pub has_icc: bool,
    /// BMP has no Exif; always `false`.
    pub has_exif: bool,
    /// BMP has no XMP; always `false`.
    pub has_xmp: bool,
    /// `biBitCount` — bits per pixel on disk (1 / 2 / 4 / 8 / 16 / 24 / 32).
    pub bits_per_pixel: u16,
    /// `biCompression` (`BI_RGB`, `BI_RLE8`, `BI_RLE4`, `BI_BITFIELDS`,
    /// `BI_ALPHABITFIELDS`).
    pub compression: u32,
    /// DIB header size in bytes (12 / 16…64 / 40 / 52 / 56 / 108 / 124).
    pub header_size: u32,
    /// The file stores its rows top-down (`biHeight < 0`).
    pub top_down: bool,
}

impl ImageInfo {
    /// Build a header description; `frames` 1, no metadata flags,
    /// default colour — fill the rest with field assignment.
    pub fn new(width: u32, height: u32, format: PixelFormat) -> Self {
        Self {
            width,
            height,
            format,
            frames: 1,
            has_alpha: format.has_alpha(),
            color: ColorInfo::bmp_default(),
            has_icc: false,
            has_exif: false,
            has_xmp: false,
            bits_per_pixel: format.bits_per_pixel(),
            compression: 0,
            header_size: 0,
            top_down: false,
        }
    }
}
