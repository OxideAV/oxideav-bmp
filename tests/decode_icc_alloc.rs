//! A decode under `DecodeOptions::with_copy_icc(false)` leaves an
//! embedded ICC profile in the file, measured with the counting global
//! allocator in `counting_alloc`: decoding a 1024 x 1024 RGBA file with
//! a 1 MiB profile allocates the pixel plane, not the plane plus a copy
//! of the profile.

mod counting_alloc;

use counting_alloc::*;
use oxideav_bmp::{decode_dib_with, decode_with, encode, DecodeOptions, EncodeOptions, Metadata};

/// The embedded profile: 1 MiB, a quarter of the pixel plane.
const PROFILE: usize = 1 << 20;

/// The native `Rgba` plane a decode of the file returns.
const PLANE: usize = (W * H * 4) as usize;

/// A V5 file embedding a [`PROFILE`]-byte profile after the pixels.
fn file_with_profile() -> Vec<u8> {
    let image = rgba_image().with_metadata(Metadata::new().with_icc(vec![0x5a; PROFILE]));
    let file = encode(&image, &EncodeOptions::default()).unwrap();
    // 14-byte file header, 124-byte V5 header, pixels, profile.
    assert_eq!(file.len(), 14 + 124 + PLANE + PROFILE);
    file
}

fn without_profile() -> DecodeOptions {
    DecodeOptions::default().with_copy_icc(false)
}

#[test]
fn copying_the_profile_is_the_default() {
    assert!(DecodeOptions::default().copy_icc);
    let file = file_with_profile();
    let (image, bytes) = allocated(|| decode_with(&file, &DecodeOptions::default()));
    let image = image.unwrap();
    assert_eq!(image.metadata.icc, Some(vec![0x5a; PROFILE]));
    assert!(bytes >= PLANE + PROFILE, "{bytes} bytes allocated");
}

#[test]
fn a_file_decoded_without_its_profile_allocates_the_plane_alone() {
    let file = file_with_profile();
    let (image, bytes) = allocated(|| decode_with(&file, &without_profile()));
    let image = image.unwrap();
    check("decode_with without the profile", bytes, PLANE);
    assert_eq!(image.metadata.icc, None);
    // The pixels and the colour are the default decode's.
    let copied = decode_with(&file, &DecodeOptions::default()).unwrap();
    assert_eq!(image.planes, copied.planes);
    assert_eq!(image.color, copied.color);
}

#[test]
fn a_dib_decoded_without_its_profile_allocates_the_plane_alone() {
    // The DIB is the file after its 14-byte file header: the profile
    // offset is counted from the DIB header, and the pixels follow the
    // header at the canonical position.
    let file = file_with_profile();
    let dib = &file[14..];
    let copied = decode_dib_with(dib, false, &DecodeOptions::default()).unwrap();
    assert_eq!(copied.metadata.icc.as_ref().map(Vec::len), Some(PROFILE));
    let (image, bytes) = allocated(|| decode_dib_with(dib, false, &without_profile()));
    let image = image.unwrap();
    check("decode_dib_with without the profile", bytes, PLANE);
    assert_eq!(image.metadata.icc, None);
    assert_eq!(image.planes, copied.planes);
}
