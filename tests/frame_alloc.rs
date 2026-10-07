//! `VideoFrame::from(&BmpImage)` copies the pixel plane and nothing
//! else, measured with the counting global allocator in
//! `counting_alloc`.

mod counting_alloc;

#[cfg(feature = "registry")]
mod registry {
    use super::counting_alloc::*;
    use oxideav_core::VideoFrame;

    /// A frame owns its plane, so converting a borrowed image copies the
    /// pixels once; the ICC profile, which a frame cannot carry, is not
    /// copied along with them.
    #[test]
    fn frame_from_borrowed_image_copies_only_the_plane() {
        let image =
            rgba_image().with_metadata(oxideav_bmp::Metadata::new().with_icc(vec![0x5a; 1 << 20]));
        let plane = image.planes[0].data.len();
        let (frame, bytes) = allocated(|| VideoFrame::from(&image));
        assert_eq!(frame.planes[0].data, image.planes[0].data);
        check("VideoFrame::from(&BmpImage)", bytes, plane);
    }
}
