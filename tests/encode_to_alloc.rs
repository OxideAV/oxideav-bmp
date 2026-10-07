//! `encode_to` writes as it encodes, measured with the counting global
//! allocator in `counting_alloc`: one write chunk plus a row, never the
//! file; and the writes it makes, recorded.

mod counting_alloc;

use counting_alloc::*;
use oxideav_bmp::{encode, encode_to, BmpImage, EncodeOptions, Metadata};

/// One 64 KiB write chunk plus a row, for raw rows and for the RLE
/// stream alike (the RLE stream is measured first, a row at a time).
#[test]
fn encode_to_streams_through_a_bounded_chunk() {
    let mut images = vec![("Rgba".to_string(), rgba_image())];
    images.extend(
        rle_candidates()
            .into_iter()
            .map(|(what, image, ..)| (what, image)),
    );
    for (what, image) in images {
        let (result, bytes) =
            allocated(|| encode_to(&image, &EncodeOptions::default(), std::io::sink()));
        result.unwrap();
        println!("{what}: encode_to allocated {bytes} bytes");
        assert!(
            bytes <= 64 * 1024 + 2 * W as usize * 4,
            "{what}: encode_to allocated {bytes} bytes"
        );
    }
}

/// Records the length of every `write` call.
struct Writes(Vec<usize>, Vec<u8>);

impl std::io::Write for Writes {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.push(buf.len());
        self.1.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Whole rows collect until 64 KiB is reached, so every write but the
/// last is at least 64 KiB; a V5 profile goes out in a write of its own.
#[test]
fn encode_to_writes_at_least_64_kib_of_whole_rows_at_a_time() {
    let image = rgba_image();
    let mut w = Writes(Vec::new(), Vec::new());
    encode_to(&image, &EncodeOptions::default(), &mut w).unwrap();
    assert_eq!(w.1, encode(&image, &EncodeOptions::default()).unwrap());
    // The 54-byte header and 16 rows of 4096 bytes.
    assert_eq!(w.0[0], 54 + 16 * 4096);
    let (last, rest) = w.0.split_last().unwrap();
    assert!(rest.iter().all(|&n| n >= 64 * 1024), "{:?}", w.0);
    assert!(*last > 0);

    let icc = vec![0x5a; 1000];
    let with_icc: BmpImage = rgba_image().with_metadata(Metadata::new().with_icc(icc.clone()));
    let mut w = Writes(Vec::new(), Vec::new());
    encode_to(&with_icc, &EncodeOptions::default(), &mut w).unwrap();
    assert_eq!(
        *w.0.last().unwrap(),
        icc.len(),
        "the profile in its own write"
    );
    assert_eq!(w.1, encode(&with_icc, &EncodeOptions::default()).unwrap());
}
