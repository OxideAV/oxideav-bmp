//! `encode_into` into a buffer with `encoded_size_bound` bytes of spare
//! capacity allocates nothing, measured with the counting global
//! allocator in `counting_alloc`.

mod counting_alloc;

use counting_alloc::*;
use oxideav_bmp::{encode, encode_into, encoded_size_bound, EncodeOptions};

/// A caller that reserves `encoded_size_bound` bytes of spare capacity
/// encodes with no allocation at all, whatever the layout and whether
/// the RLE stream wins; the room left after the file is the caller's.
#[test]
fn encode_into_a_reserved_buffer_does_not_allocate() {
    let opts = EncodeOptions::default();
    let mut images = vec![("Rgba".to_string(), rgba_image())];
    images.extend(
        rle_candidates()
            .into_iter()
            .map(|(what, image, ..)| (what, image)),
    );
    for (what, image) in images {
        let bound = encoded_size_bound(&image, &opts).unwrap();
        let mut out = Vec::with_capacity(4 + bound);
        out.extend_from_slice(b"head");
        let capacity = out.capacity();
        let (result, bytes) = allocated(|| encode_into(&image, &opts, &mut out));
        result.unwrap();
        println!(
            "{what}: encode_into into {bound} reserved bytes wrote {} and allocated {bytes}",
            out.len() - 4
        );
        assert_eq!(
            bytes, 0,
            "{what}: encode_into allocated into a reserved buffer"
        );
        assert_eq!(
            out.capacity(),
            capacity,
            "{what}: the buffer was not replaced"
        );
        assert_eq!(&out[4..], &encode(&image, &opts).unwrap()[..], "{what}");
    }
    let rgba = rgba_image();
    assert_eq!(
        encoded_size_bound(&rgba, &opts).unwrap(),
        RGBA_FILE,
        "the exact size when no RLE probe runs"
    );
}
