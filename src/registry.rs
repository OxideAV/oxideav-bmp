//! `oxideav-core` integration layer for `oxideav-bmp`.
//!
//! Gated behind the default-on `registry` feature so image-library
//! consumers can depend on `oxideav-bmp` with `default-features = false`
//! and skip the `oxideav-core` dependency entirely.
//!
//! The module exposes:
//! * [`register`] / [`register_codecs`] / [`register_containers`] — the
//!   `RuntimeContext` / `CodecRegistry` / `ContainerRegistry` entry
//!   points the umbrella `oxideav` crate calls during framework
//!   initialisation.
//! * [`make_decoder`] / [`make_encoder`] — the trait-side factories; the
//!   `Decoder` / `Encoder` impls are thin adapters over
//!   [`crate::decode_with`] / [`crate::encode`].
//! * `From<BmpImage> for VideoFrame` and [`BmpImage::from_video_frame`]
//!   — the plane plus the palette / colour-signal side-channels.
//! * The `From<BmpError> for oxideav_core::Error` conversion + the
//!   `CodecOptionsStruct` impl for [`EncodeOptions`].
//! * The deprecated `*_videoframe` wrappers `oxideav-ico` still calls.

use oxideav_core::Decoder;
use oxideav_core::Encoder;
use oxideav_core::RuntimeContext;
use oxideav_core::{
    parse_options, CodecCapabilities, CodecId, CodecInfo, CodecOptionsStruct, CodecParameters,
    CodecRegistry, ColorPrimaries, ColorSignal, ContainerRegistry, Frame, MatrixCoefficients,
    OptionField, OptionKind, OptionValue, Packet, PixelFormat, TimeBase, TransferCharacteristics,
    VideoFrame, VideoPlane,
};

use crate::container;
use crate::error::BmpError;
use crate::image::{BmpImage, BmpPixelFormat, ColorInfo, ColorRange, Palette, Plane};
use crate::options::{DecodeOptions, EncodeOptions};

/// Convert a [`BmpError`] into the framework-shared `oxideav_core::Error`
/// so trait impls in this crate can use `?` on errors returned by the
/// framework-free decode/encode functions.
impl From<BmpError> for oxideav_core::Error {
    fn from(e: BmpError) -> Self {
        match e {
            BmpError::InvalidData(s) => oxideav_core::Error::InvalidData(s),
            BmpError::Unsupported(s) => oxideav_core::Error::Unsupported(s),
            BmpError::LimitExceeded(s) => oxideav_core::Error::ResourceExhausted(s),
            BmpError::Io(e) => oxideav_core::Error::Io(e),
        }
    }
}

// ---- Pixel-format + colour-signal mapping ----

/// The framework pixel format a [`BmpPixelFormat`] maps to by name;
/// `None` for the BMP-specific layouts the core enum does not have
/// (`Rgb555`, `Rgb565`, `Indexed4` / `Indexed2` / `Indexed1`).
pub fn to_core_pixel_format(pf: BmpPixelFormat) -> Option<PixelFormat> {
    Some(match pf {
        BmpPixelFormat::Rgba => PixelFormat::Rgba,
        BmpPixelFormat::Rgb24 => PixelFormat::Rgb24,
        BmpPixelFormat::Bgra => PixelFormat::Bgra,
        BmpPixelFormat::Bgr24 => PixelFormat::Bgr24,
        BmpPixelFormat::Pal8 => PixelFormat::Pal8,
        _ => return None,
    })
}

/// Map a framework pixel format to [`BmpPixelFormat`]. Returns `Err` for
/// pixel formats the BMP codec can't carry.
pub fn from_core_pixel_format(pf: PixelFormat) -> oxideav_core::Result<BmpPixelFormat> {
    Ok(match pf {
        PixelFormat::Rgba => BmpPixelFormat::Rgba,
        PixelFormat::Rgb24 => BmpPixelFormat::Rgb24,
        PixelFormat::Bgra => BmpPixelFormat::Bgra,
        PixelFormat::Bgr24 => BmpPixelFormat::Bgr24,
        PixelFormat::Pal8 => BmpPixelFormat::Pal8,
        other => {
            return Err(oxideav_core::Error::unsupported(format!(
                "BMP: pixel format {other:?} not supported"
            )))
        }
    })
}

/// The framework layout the registry decoder emits for a decoded
/// image: the native layout wherever the core enum has its name
/// (`Rgba` / `Rgb24` / `Bgra` / `Bgr24` / `Pal8`); the sub-byte indexed
/// layouts (`Indexed4` / `Indexed2` / `Indexed1`, already one index per
/// byte) ride `Pal8` with the same plane and palette; `Rgb555` /
/// `Rgb565`, which core has no 16-bit RGB name for, are widened to
/// `Rgb24` (bit-replicated 5 / 6-bit samples). The BMP demuxer
/// declares the same layout on its stream.
pub fn registry_pixel_format(pf: BmpPixelFormat) -> PixelFormat {
    match to_core_pixel_format(pf) {
        Some(core) => core,
        None if pf.is_indexed() => PixelFormat::Pal8,
        None => PixelFormat::Rgb24,
    }
}

/// Re-label / widen a decoded image to the layout
/// [`registry_pixel_format`] names (a no-op for layouts core has).
fn into_registry_layout(image: BmpImage) -> BmpImage {
    match image.format {
        BmpPixelFormat::Indexed4 | BmpPixelFormat::Indexed2 | BmpPixelFormat::Indexed1 => {
            // One index per byte already; only the label changes.
            BmpImage {
                format: BmpPixelFormat::Pal8,
                ..image
            }
        }
        BmpPixelFormat::Rgb555 | BmpPixelFormat::Rgb565 => {
            let data = image.to_rgb8();
            BmpImage {
                width: image.width,
                height: image.height,
                format: BmpPixelFormat::Rgb24,
                planes: vec![Plane::new(image.width as usize * 3, data)],
                color: image.color,
                metadata: image.metadata,
                palette: None,
            }
        }
        _ => image,
    }
}

impl TryFrom<PixelFormat> for BmpPixelFormat {
    type Error = oxideav_core::Error;
    fn try_from(pf: PixelFormat) -> oxideav_core::Result<Self> {
        from_core_pixel_format(pf)
    }
}

/// [`ColorInfo`] as the framework's [`ColorSignal`] (code points map
/// 1:1; `Unspecified` range stays unspecified).
pub fn to_color_signal(c: &ColorInfo) -> ColorSignal {
    let range = match c.range {
        ColorRange::Unspecified => oxideav_core::ColorRange::Unspecified,
        ColorRange::Limited => oxideav_core::ColorRange::Limited,
        ColorRange::Full => oxideav_core::ColorRange::Full,
    };
    ColorSignal::new(
        range,
        ColorPrimaries(c.primaries),
        TransferCharacteristics(c.transfer),
        MatrixCoefficients(c.matrix),
    )
}

/// The inverse of [`to_color_signal`].
pub fn from_color_signal(s: &ColorSignal) -> ColorInfo {
    let range = match s.range {
        oxideav_core::ColorRange::Limited => ColorRange::Limited,
        oxideav_core::ColorRange::Full => ColorRange::Full,
        _ => ColorRange::Unspecified,
    };
    ColorInfo::new(range, s.primaries.0, s.transfer.0, s.matrix.0)
}

// ---- Frame bridge ----

fn image_into_video_frame(mut image: BmpImage, pts: Option<i64>) -> VideoFrame {
    let stride = image.stride();
    let data = if image.planes.is_empty() {
        Vec::new()
    } else {
        std::mem::take(&mut image.planes[0].data)
    };
    let mut frame = VideoFrame {
        pts,
        planes: vec![VideoPlane { stride, data }],
    };
    if let (true, Some(p)) = (image.format.is_indexed(), &image.palette) {
        frame.set_palette(p.to_rgb());
    }
    let c = image.color;
    if c.primaries != ColorInfo::UNSPECIFIED
        || c.transfer != ColorInfo::UNSPECIFIED
        || c.range == ColorRange::Limited
    {
        frame.set_color_signal(to_color_signal(&c));
    }
    frame
}

impl From<BmpImage> for VideoFrame {
    /// The pixel plane (`pts` `None`), plus the palette side-channel
    /// for the indexed layouts and the colour-signal side-channel when
    /// the image signals a colour space (sRGB from a V4 / V5 header).
    fn from(image: BmpImage) -> Self {
        image_into_video_frame(image, None)
    }
}

impl From<&BmpImage> for VideoFrame {
    fn from(image: &BmpImage) -> Self {
        image_into_video_frame(image.clone(), None)
    }
}

impl BmpImage {
    /// Rebuild an image from a framework frame and the stream
    /// parameters that describe it (`width`, `height` and
    /// `pixel_format` are required; a `Pal8` frame needs the palette
    /// side-channel). The frame's colour-signal side-channel, when
    /// attached, becomes `color`.
    pub fn from_video_frame(
        frame: &VideoFrame,
        params: &CodecParameters,
    ) -> oxideav_core::Result<Self> {
        let width = params
            .width
            .ok_or_else(|| oxideav_core::Error::invalid("BMP: missing width"))?;
        let height = params
            .height
            .ok_or_else(|| oxideav_core::Error::invalid("BMP: missing height"))?;
        let pix = from_core_pixel_format(
            params
                .pixel_format
                .ok_or_else(|| oxideav_core::Error::invalid("BMP: missing pixel_format"))?,
        )?;
        let plane = frame
            .image_planes()
            .first()
            .ok_or_else(|| oxideav_core::Error::invalid("BMP: frame has no planes"))?;
        let mut img = BmpImage::new(
            width,
            height,
            pix,
            vec![Plane::new(plane.stride, plane.data.clone())],
        )?;
        if pix.is_indexed() {
            let rgb = frame.palette().ok_or_else(|| {
                oxideav_core::Error::invalid("BMP: Pal8 frame without a palette side-channel")
            })?;
            let entries: Vec<[u8; 3]> = rgb.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect();
            img.palette = Some(Palette::from_rgb(&entries));
        }
        if let Some(sig) = frame.color_signal() {
            img.color = from_color_signal(&sig);
        }
        Ok(img)
    }
}

impl TryFrom<(&VideoFrame, &CodecParameters)> for BmpImage {
    type Error = oxideav_core::Error;
    fn try_from((frame, params): (&VideoFrame, &CodecParameters)) -> oxideav_core::Result<Self> {
        BmpImage::from_video_frame(frame, params)
    }
}

// ---- CodecOptionsStruct (registry-only schema for EncodeOptions) ----

impl CodecOptionsStruct for EncodeOptions {
    const SCHEMA: &'static [OptionField] = &[
        OptionField {
            name: "top_down",
            kind: OptionKind::Bool,
            default: OptionValue::Bool(false),
            help: "Store rows top-down (negative biHeight). Disables RLE.",
        },
        OptionField {
            name: "minimal_palette",
            kind: OptionKind::Bool,
            default: OptionValue::Bool(false),
            help: "Write only the palette entries the image carries (biClrUsed) \
                   instead of a full 2^bpp colour table.",
        },
        OptionField {
            name: "rle",
            kind: OptionKind::Bool,
            default: OptionValue::Bool(true),
            help: "Allow BI_RLE8 / BI_RLE4 for indexed input when smaller than raw.",
        },
        OptionField {
            name: "embed_icc",
            kind: OptionKind::Bool,
            default: OptionValue::Bool(true),
            help: "Write a V5 header embedding the image's ICC profile when it has one.",
        },
    ];
    fn apply(&mut self, key: &str, v: &OptionValue) -> oxideav_core::Result<()> {
        match key {
            "top_down" => self.top_down = v.as_bool()?,
            "minimal_palette" => self.minimal_palette = v.as_bool()?,
            "rle" => self.rle = v.as_bool()?,
            "embed_icc" => self.embed_icc = v.as_bool()?,
            _ => unreachable!("guarded by SCHEMA"),
        }
        Ok(())
    }
}

// ---- Decoder trait impl + factory ----

/// Factory registered with the codec registry. Consumes one packet per
/// whole BMP file and produces one frame in the image's native layout
/// per [`registry_pixel_format`] — `Pal8` with the palette side-channel
/// for every indexed depth, `Bgr24` / `Bgra` / `Rgb24` / `Rgba` as
/// stored, `Rgb24` for the 16-bit layouts — plus the colour-signal
/// side-channel when the file carries a colour space (V4 / V5 sRGB).
/// Nothing is pre-converted to `Rgba`; use `oxideav-pixfmt` (or
/// [`crate::decode_rgba8`]) for that. BMP is a single-image format, so
/// `flush()` just drains the one pending frame.
pub fn make_decoder(_params: &CodecParameters) -> oxideav_core::Result<Box<dyn Decoder>> {
    Ok(Box::new(BmpDecoder {
        codec_id: CodecId::new(crate::CODEC_ID_STR),
        pending: None,
        eof: false,
    }))
}

/// BMP `Decoder` trait impl (see [`make_decoder`]).
pub struct BmpDecoder {
    codec_id: CodecId,
    pending: Option<VideoFrame>,
    eof: bool,
}

impl Decoder for BmpDecoder {
    fn codec_id(&self) -> &CodecId {
        &self.codec_id
    }
    fn send_packet(&mut self, packet: &Packet) -> oxideav_core::Result<()> {
        let image = crate::decoder::decode_file(&packet.data, &DecodeOptions::default())?;
        self.pending = Some(image_into_video_frame(
            into_registry_layout(image),
            packet.pts,
        ));
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

// ---- Encoder trait impl + factory ----

/// Factory registered with the codec registry: one `VideoFrame`
/// (`Rgba` / `Rgb24` / `Bgra` / `Bgr24` / `Pal8` per
/// `CodecParameters::pixel_format`) becomes one BMP file packet.
/// `CodecParameters::options` is parsed as [`EncodeOptions`]
/// (`top_down`, `minimal_palette`, `rle`, `embed_icc`).
pub fn make_encoder(params: &CodecParameters) -> oxideav_core::Result<Box<dyn Encoder>> {
    let opts = parse_options::<EncodeOptions>(&params.options)?;
    let mut out_params = CodecParameters::video(CodecId::new(crate::CODEC_ID_STR));
    out_params.width = params.width;
    out_params.height = params.height;
    out_params.pixel_format = params.pixel_format;
    Ok(Box::new(BmpEncoder {
        codec_id: CodecId::new(crate::CODEC_ID_STR),
        out_params,
        opts,
        pending: None,
        eof: false,
    }))
}

/// BMP `Encoder` trait impl (see [`make_encoder`]).
pub struct BmpEncoder {
    codec_id: CodecId,
    out_params: CodecParameters,
    opts: EncodeOptions,
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
    fn send_frame(&mut self, frame: &Frame) -> oxideav_core::Result<()> {
        let vf = match frame {
            Frame::Video(v) => v,
            _ => {
                return Err(oxideav_core::Error::invalid(
                    "BMP encoder: expected video frame",
                ))
            }
        };
        let image = BmpImage::from_video_frame(vf, &self.out_params)?;
        let bytes = crate::encoder::encode_image(&image, &self.opts)?.0;
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

// ---- Deprecated VideoFrame-flavoured wrappers (oxideav-ico) ----

/// Decode a complete BMP file into an `Rgba` `VideoFrame`.
#[deprecated(
    note = "use oxideav_bmp::decode and VideoFrame::from(image) (IMAGE_CRATE_API); this wrapper widens to Rgba"
)]
pub fn decode_bmp_videoframe(input: &[u8]) -> oxideav_core::Result<VideoFrame> {
    let image = crate::decoder::decode_file(input, &DecodeOptions::default())?;
    Ok(image_into_video_frame(image.into_rgba(), None))
}

/// Decode a headerless DIB into an `Rgba` `VideoFrame`.
#[deprecated(
    note = "use oxideav_bmp::decode_dib and VideoFrame::from(image) (IMAGE_CRATE_API); this wrapper widens to Rgba"
)]
pub fn decode_dib_videoframe(
    input: &[u8],
    dib_height_is_doubled_for_mask: bool,
) -> oxideav_core::Result<VideoFrame> {
    let image = crate::decoder::decode_dib(input, dib_height_is_doubled_for_mask)?;
    Ok(image_into_video_frame(image.into_rgba(), None))
}

fn frame_image(
    frame: &VideoFrame,
    format: PixelFormat,
    width: u32,
    height: u32,
) -> oxideav_core::Result<BmpImage> {
    let mut params = CodecParameters::video(CodecId::new(crate::CODEC_ID_STR));
    params.width = Some(width);
    params.height = Some(height);
    params.pixel_format = Some(format);
    BmpImage::from_video_frame(frame, &params)
}

/// Encode a `VideoFrame` into a complete BMP file.
#[deprecated(note = "use BmpImage::from_video_frame and oxideav_bmp::encode (IMAGE_CRATE_API)")]
pub fn encode_bmp_videoframe(
    frame: &VideoFrame,
    format: PixelFormat,
    width: u32,
    height: u32,
) -> oxideav_core::Result<Vec<u8>> {
    let image = frame_image(frame, format, width, height)?;
    Ok(crate::encoder::encode_image(&image, &EncodeOptions::default())?.0)
}

/// Encode a `VideoFrame` into a headerless DIB.
#[deprecated(note = "use BmpImage::from_video_frame and oxideav_bmp::encode_dib (IMAGE_CRATE_API)")]
pub fn encode_dib_videoframe(
    frame: &VideoFrame,
    format: PixelFormat,
    width: u32,
    height: u32,
    double_height_for_ico_mask: bool,
) -> oxideav_core::Result<Vec<u8>> {
    let image = frame_image(frame, format, width, height)?;
    Ok(crate::encoder::encode_dib(
        &image,
        double_height_for_ico_mask,
    )?)
}

// ---- Container + registration ----

/// Register the BMP codec into the supplied [`CodecRegistry`].
pub fn register_codecs(reg: &mut CodecRegistry) {
    let caps = CodecCapabilities::video("bmp_sw")
        .with_intra_only(true)
        .with_lossless(true)
        .with_max_size(65535, 65535)
        .with_pixel_formats(vec![
            PixelFormat::Rgba,
            PixelFormat::Rgb24,
            PixelFormat::Bgra,
            PixelFormat::Bgr24,
            PixelFormat::Pal8,
        ]);
    reg.register(
        CodecInfo::new(CodecId::new(crate::CODEC_ID_STR))
            .capabilities(caps)
            .decoder(make_decoder)
            .encoder(make_encoder)
            .encoder_options::<EncodeOptions>(),
    );
}

/// Register the BMP container demuxer + muxer + extension + probe
/// into the supplied [`ContainerRegistry`].
pub fn register_containers(reg: &mut ContainerRegistry) {
    container::register(reg);
}

/// Unified registration entry point — installs the BMP codec into the
/// codec sub-registry and the BMP container into the container
/// sub-registry of the supplied [`RuntimeContext`].
pub fn register(ctx: &mut RuntimeContext) {
    register_codecs(&mut ctx.codecs);
    register_containers(&mut ctx.containers);
}

oxideav_core::register!("bmp", register);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_installs_codec_and_container() {
        let mut ctx = RuntimeContext::new();
        register(&mut ctx);
        let id = CodecId::new(crate::CODEC_ID_STR);
        assert!(
            ctx.codecs.has_decoder(&id),
            "BMP decoder factory not installed via RuntimeContext"
        );
        assert!(
            ctx.codecs.has_encoder(&id),
            "BMP encoder factory not installed via RuntimeContext"
        );
        assert_eq!(
            ctx.containers.container_for_extension("bmp"),
            Some("bmp"),
            "BMP container extension not installed via RuntimeContext"
        );
    }

    #[test]
    fn frame_bridge_round_trips_pal8_and_color() {
        let img = BmpImage::packed(2, 1, BmpPixelFormat::Pal8, 2, vec![0, 1])
            .unwrap()
            .with_palette(Palette::from_rgb(&[[1, 2, 3], [4, 5, 6]]))
            .with_color(ColorInfo::srgb());
        let frame = VideoFrame::from(&img);
        assert_eq!(frame.palette(), Some(&[1u8, 2, 3, 4, 5, 6][..]));
        let mut params = CodecParameters::video(CodecId::new(crate::CODEC_ID_STR));
        params.width = Some(2);
        params.height = Some(1);
        params.pixel_format = Some(PixelFormat::Pal8);
        let back = BmpImage::try_from((&frame, &params)).unwrap();
        assert_eq!(back, img);
    }

    #[test]
    fn pixel_format_names_mirror_core() {
        for (b, c) in [
            (BmpPixelFormat::Rgba, PixelFormat::Rgba),
            (BmpPixelFormat::Rgb24, PixelFormat::Rgb24),
            (BmpPixelFormat::Bgra, PixelFormat::Bgra),
            (BmpPixelFormat::Bgr24, PixelFormat::Bgr24),
            (BmpPixelFormat::Pal8, PixelFormat::Pal8),
        ] {
            assert_eq!(format!("{b:?}"), format!("{c:?}"));
            assert_eq!(to_core_pixel_format(b), Some(c));
            assert_eq!(from_core_pixel_format(c).unwrap(), b);
        }
        assert_eq!(to_core_pixel_format(BmpPixelFormat::Rgb565), None);
        assert_eq!(
            registry_pixel_format(BmpPixelFormat::Rgb565),
            PixelFormat::Rgb24
        );
        assert_eq!(
            registry_pixel_format(BmpPixelFormat::Indexed1),
            PixelFormat::Pal8
        );
    }

    /// One packet through `make_decoder`, as the framework sees it.
    fn decode_via_registry(bytes: Vec<u8>) -> VideoFrame {
        let params = CodecParameters::video(CodecId::new(crate::CODEC_ID_STR));
        let mut dec = make_decoder(&params).unwrap();
        dec.send_packet(&Packet::new(0, TimeBase::new(1, 1), bytes))
            .unwrap();
        match dec.receive_frame().unwrap() {
            Frame::Video(v) => v,
            _ => panic!("non-video frame"),
        }
    }

    #[test]
    fn decoder_emits_native_layouts_not_rgba() {
        use crate::{encode, EncodeOptions};
        // Pal8: index plane + palette side-channel.
        let pal = BmpImage::packed(3, 2, BmpPixelFormat::Pal8, 3, vec![0, 1, 2, 2, 1, 0])
            .unwrap()
            .with_palette(Palette::from_rgb(&[
                [10, 20, 30],
                [40, 50, 60],
                [70, 80, 90],
            ]));
        let v = decode_via_registry(encode(&pal, &EncodeOptions::default()).unwrap());
        assert_eq!(v.image_plane_count(), 1);
        assert_eq!(v.planes[0].stride, 3, "Pal8 is one byte per pixel");
        assert_eq!(v.planes[0].data, [0, 1, 2, 2, 1, 0]);
        // The default encode writes the full 256-entry table; the
        // image's entries lead it.
        let pal_rgb = v.palette().expect("Pal8 frame carries its palette");
        assert_eq!(pal_rgb.len(), 256 * 3);
        assert_eq!(&pal_rgb[..9], &[10u8, 20, 30, 40, 50, 60, 70, 80, 90]);
        assert!(
            v.color_signal().is_none(),
            "V3 file carries no colour space"
        );

        // Indexed1 → Pal8 label, same one-index-per-byte plane.
        let bw = BmpImage::packed(4, 1, BmpPixelFormat::Indexed1, 4, vec![1, 0, 0, 1])
            .unwrap()
            .with_palette(Palette::from_rgb(&[[0, 0, 0], [255, 255, 255]]));
        let v = decode_via_registry(encode(&bw, &EncodeOptions::default()).unwrap());
        assert_eq!(v.planes[0].stride, 4);
        assert_eq!(v.planes[0].data, [1, 0, 0, 1]);
        assert_eq!(
            v.palette().map(|p| &p[..6]),
            Some(&[0u8, 0, 0, 255, 255, 255][..])
        );

        // Bgr24 stays Bgr24 (BMP's own 24 bpp order, no swizzle).
        let bgr = BmpImage::packed(1, 1, BmpPixelFormat::Bgr24, 3, vec![1, 2, 3]).unwrap();
        let v = decode_via_registry(encode(&bgr, &EncodeOptions::default()).unwrap());
        assert_eq!(v.planes[0].stride, 3);
        assert_eq!(v.planes[0].data, [1, 2, 3]);
        assert!(v.palette().is_none());

        // Bgra stays Bgra (4 bytes, file order).
        let bgra = BmpImage::packed(1, 1, BmpPixelFormat::Bgra, 4, vec![1, 2, 3, 200]).unwrap();
        let v = decode_via_registry(encode(&bgra, &EncodeOptions::default()).unwrap());
        assert_eq!(v.planes[0].stride, 4);
        assert_eq!(v.planes[0].data, [1, 2, 3, 200]);

        // Rgb565 (no core name) widens to Rgb24 by bit replication.
        let px = 0xF800u16; // R = 31 (bits 15..11), G = 0, B = 0
        let r565 =
            BmpImage::packed(1, 1, BmpPixelFormat::Rgb565, 2, px.to_le_bytes().to_vec()).unwrap();
        let v = decode_via_registry(encode(&r565, &EncodeOptions::default()).unwrap());
        assert_eq!(v.planes[0].stride, 3);
        assert_eq!(v.planes[0].data, [255, 0, 0]);
    }

    #[test]
    fn demuxer_declares_the_native_layout() {
        use crate::{encode, EncodeOptions};
        let pal = BmpImage::packed(2, 1, BmpPixelFormat::Pal8, 2, vec![0, 1])
            .unwrap()
            .with_palette(Palette::from_rgb(&[[1, 2, 3], [4, 5, 6]]));
        let bytes = encode(&pal, &EncodeOptions::default()).unwrap();
        let ctx = RuntimeContext::new();
        let dmx =
            crate::container::open_demuxer(Box::new(std::io::Cursor::new(bytes)), &ctx.codecs)
                .unwrap();
        assert_eq!(
            dmx.streams()[0].params.pixel_format,
            Some(PixelFormat::Pal8)
        );
        let bgr = BmpImage::packed(1, 1, BmpPixelFormat::Bgr24, 3, vec![1, 2, 3]).unwrap();
        let bytes = encode(&bgr, &EncodeOptions::default()).unwrap();
        let dmx =
            crate::container::open_demuxer(Box::new(std::io::Cursor::new(bytes)), &ctx.codecs)
                .unwrap();
        assert_eq!(
            dmx.streams()[0].params.pixel_format,
            Some(PixelFormat::Bgr24)
        );
    }
}
