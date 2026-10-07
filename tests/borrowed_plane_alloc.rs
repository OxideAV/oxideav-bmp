//! The deprecated plane-level and ICC encoders and the registry encoder
//! read the caller's plane in place, measured with the counting global
//! allocator in `counting_alloc`: a 1024 x 1024 RGBA encode allocates
//! the file, not the file plus a copy of the plane.

// The deprecated plane-level and ICC names are what this file pins.
#![allow(deprecated)]

mod counting_alloc;

use counting_alloc::*;
use oxideav_bmp::{
    encode_bmp_plane, encode_bmp_with_icc_profile, BmpPixelFormat, EncodeOptions, LCS_GM_IMAGES,
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
