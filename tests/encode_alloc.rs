//! Allocation budget of `encode`, measured with the counting global
//! allocator in `counting_alloc`.
//!
//! The encoder writes each source row straight into its final place in
//! the output, so encoding a plane allocates the file once plus a small
//! constant, never a second plane-sized buffer. A 1024 x 1024 RGBA plane
//! is 4 MiB and `SMALL` is 8 KiB, a 512th of that, so any extra copy of
//! the pixels fails these tests.

mod counting_alloc;

use counting_alloc::*;
use oxideav_bmp::{encode, EncodeOptions};

#[test]
fn encode_allocates_the_file_once() {
    let image = rgba_image();
    for top_down in [false, true] {
        let opts = EncodeOptions::default().with_top_down(top_down);
        let (result, bytes) = allocated(|| encode(&image, &opts));
        let file = result.unwrap();
        assert_eq!(file.len(), RGBA_FILE);
        check(&format!("encode top_down={top_down}"), bytes, file.len());
    }
}

/// The indexed layouts take the same path. The buffer is reserved once
/// for the raw file plus the RLE probe's one-row slack (`2 × width + 2`
/// bytes past the raw array, how far the probe can run before it gives
/// up); the probe writes its stream into that buffer, so neither an RLE
/// win nor a fallback to the raw array costs a second plane. When the
/// stream wins, the buffer is shrunk to the file, which the counter sees
/// as one more allocation of the (compressed) file's size. The `BI_RLE4`
/// probe reads nibbles in place, with no scratch row.
#[test]
fn indexed_encode_allocates_the_file_once() {
    let slack = 2 * W as usize + 2;
    for (what, image, raw_file, rle_wins) in rle_candidates() {
        let (result, bytes) = allocated(|| encode(&image, &EncodeOptions::default()));
        let file = result.unwrap();
        assert_eq!(file.len() < raw_file, rle_wins, "{what}: RLE choice");
        let shrink = if rle_wins { file.len() } else { 0 };
        check(&what, bytes, raw_file + slack + shrink);
    }
}
