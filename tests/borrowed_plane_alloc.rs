//! The deprecated plane-level and ICC encoders, the registry encoder
//! and `encode_into_with_icc_profile` read what the caller lends in
//! place, measured with the counting global allocator in
//! `counting_alloc`: a 1024 x 1024 RGBA encode allocates the file, not
//! the file plus a copy of the plane or the profile.

// The deprecated plane-level and ICC names are what this file pins.
#![allow(deprecated)]

mod counting_alloc;

use counting_alloc::*;
use oxideav_bmp::{
    encode, encode_bmp_plane, encode_bmp_with_icc_profile, encode_into_with_icc_profile,
    encoded_size_bound_with_icc_profile, BmpImage, BmpPixelFormat, EncodeOptions, Metadata,
    Palette, Plane, LCS_GM_IMAGES,
};

/// The deprecated plane-level names read the caller's plane in place
/// instead of wrapping a copy of it in a `BmpImage`.
#[test]
fn encode_bmp_plane_allocates_the_file_once() {
    let plane = rgba_plane();
    let (result, bytes) = allocated(|| encode_bmp_plane(&plane, BmpPixelFormat::Rgba, None, W, H));
    let (file, _) = result.unwrap();
    assert_eq!(file.len(), RGBA_FILE);
    check("encode_bmp_plane", bytes, file.len());
}

/// The deprecated ICC name embeds the given profile without cloning
/// the image (plane and all) to attach it.
#[test]
fn encode_bmp_with_icc_profile_allocates_the_file_once() {
    let image = rgba_image();
    let icc = vec![0x5a; 4096];
    let (result, bytes) = allocated(|| {
        encode_bmp_with_icc_profile(&image, &icc, LCS_GM_IMAGES, EncodeOptions::default())
    });
    let file = result.unwrap();
    // 14-byte file header, 124-byte V5 header, pixels, profile.
    assert_eq!(file.len(), 14 + 124 + (W * H * 4) as usize + icc.len());
    check("encode_bmp_with_icc_profile", bytes, file.len());
}

/// A profile no byte of which repeats in the pixels or the headers.
fn profile(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i * 13 + 7) as u8).collect()
}

/// The images the profile encode is pinned on: 32-bit, 24-bit and
/// indexed, each 64 x 32.
fn small_images() -> Vec<BmpImage> {
    let (w, h) = (64usize, 32usize);
    let bytes = |bpp: usize| (0..w * h * bpp).map(|i| (i * 7 % 256) as u8).collect();
    vec![
        BmpImage::new(
            64,
            32,
            BmpPixelFormat::Rgba,
            vec![Plane::new(w * 4, bytes(4))],
        )
        .unwrap(),
        BmpImage::new(
            64,
            32,
            BmpPixelFormat::Rgb24,
            vec![Plane::new(w * 3, bytes(3))],
        )
        .unwrap(),
        BmpImage::new(64, 32, BmpPixelFormat::Pal8, vec![Plane::new(w, bytes(1))])
            .unwrap()
            .with_palette(Palette::from_rgb(
                &(0..=255u8).map(|v| [v, 255 - v, v / 2]).collect::<Vec<_>>(),
            )),
    ]
}

/// `encode_into_with_icc_profile` writes the bytes the deprecated
/// `encode_bmp_with_icc_profile` writes at the default rendering
/// intent, and the bytes `encode` writes for the image carrying the
/// profile, appended after what the buffer holds.
#[test]
fn encode_into_with_icc_profile_writes_what_the_deprecated_entry_writes() {
    let icc = profile(3000);
    for image in small_images() {
        for opts in [
            EncodeOptions::default(),
            EncodeOptions::default().with_top_down(true),
        ] {
            let what = format!("{:?}, top_down {}", image.format, opts.top_down);
            let deprecated =
                encode_bmp_with_icc_profile(&image, &icc, LCS_GM_IMAGES, opts.clone()).unwrap();
            let mut out = b"held".to_vec();
            encode_into_with_icc_profile(&image, &opts, &icc, &mut out).unwrap();
            assert_eq!(&out[..4], b"held", "{what}");
            assert_eq!(out[4..], deprecated[..], "{what}");
            let carried = image
                .clone()
                .with_metadata(Metadata::new().with_icc(icc.clone()));
            assert_eq!(out[4..], encode(&carried, &opts).unwrap()[..], "{what}");
            let bound = encoded_size_bound_with_icc_profile(&image, &opts, &icc).unwrap();
            assert_eq!(bound, deprecated.len(), "{what}");
        }
    }
}

/// The lent profile takes the place of one the image carries; the
/// options still decide the header, so `embed_icc = false` writes none.
#[test]
fn the_lent_profile_replaces_the_images_and_follows_the_options() {
    let image = small_images().remove(0);
    let icc = profile(3000);
    let carrying_another = image
        .clone()
        .with_metadata(Metadata::new().with_icc(vec![1u8; 100]));
    let mut out = Vec::new();
    encode_into_with_icc_profile(&carrying_another, &EncodeOptions::default(), &icc, &mut out)
        .unwrap();
    let expected =
        encode_bmp_with_icc_profile(&image, &icc, LCS_GM_IMAGES, EncodeOptions::default()).unwrap();
    assert_eq!(out, expected);

    let plain = EncodeOptions::default().with_embed_icc(false);
    let mut out = Vec::new();
    encode_into_with_icc_profile(&image, &plain, &icc, &mut out).unwrap();
    assert_eq!(out, encode(&image, &plain).unwrap());
}

/// Into a buffer reserved at `encoded_size_bound_with_icc_profile`,
/// the profile encode allocates nothing; into an empty one, the file
/// once. The 1 MiB profile is never copied.
#[test]
fn encode_into_with_icc_profile_allocates_the_file_once() {
    let image = rgba_image();
    let icc = profile(1 << 20);
    let opts = EncodeOptions::default();
    let file = 14 + 124 + (W * H * 4) as usize + icc.len();
    let bound = encoded_size_bound_with_icc_profile(&image, &opts, &icc).unwrap();
    assert_eq!(bound, file);
    let mut reserved = Vec::with_capacity(bound);
    let (result, bytes) =
        allocated(|| encode_into_with_icc_profile(&image, &opts, &icc, &mut reserved));
    result.unwrap();
    assert_eq!(reserved.len(), file);
    check("encode_into_with_icc_profile, reserved", bytes, 0);
    let mut empty = Vec::new();
    let (result, bytes) =
        allocated(|| encode_into_with_icc_profile(&image, &opts, &icc, &mut empty));
    result.unwrap();
    assert_eq!(empty, reserved);
    check("encode_into_with_icc_profile, empty", bytes, file);
}

#[cfg(feature = "registry")]
mod registry {
    use super::counting_alloc::*;
    use oxideav_bmp::make_encoder;
    use oxideav_core::{CodecId, CodecParameters, Frame, PixelFormat, VideoFrame, VideoPlane};

    /// The registry encoder reads the frame's plane in place instead of
    /// copying it into a `BmpImage` first.
    #[test]
    fn registry_encoder_allocates_the_file_once() {
        let plane = rgba_plane();
        let frame = Frame::Video(VideoFrame {
            pts: None,
            planes: vec![VideoPlane {
                stride: plane.stride,
                data: plane.data,
            }],
        });
        let mut params = CodecParameters::video(CodecId::new("bmp"));
        params.width = Some(W);
        params.height = Some(H);
        params.pixel_format = Some(PixelFormat::Rgba);
        let (packet, bytes) = allocated(|| {
            let mut enc = make_encoder(&params).unwrap();
            enc.send_frame(&frame).unwrap();
            enc.receive_packet().unwrap()
        });
        assert_eq!(packet.data.len(), RGBA_FILE);
        check("registry encoder", bytes, packet.data.len());
    }
}
