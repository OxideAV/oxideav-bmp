//! The image-crate contract (`IMAGE_CRATE_API.md`) exercised through the
//! public root vocabulary only — no deprecated names, no `oxideav-core`,
//! so the file also runs under `--no-default-features`.

use std::io::Cursor;

use oxideav_bmp::{
    decode, decode_from, decode_rgb8, decode_rgba8, decode_with, encode, encode_into, encode_rgb8,
    encode_rgba8, encode_to, encode_with_report, encoded_size_bound, info, probe, BmpBitfields,
    BmpImage, BmpMetadata, CalibratedRgb, ColorInfo, ColorRange, DecodeOptions, EncodeOptions,
    EncodedBmpFormat, Error, Palette, PixelFormat, Plane, BI_ALPHABITFIELDS, BI_BITFIELDS, BI_RGB,
    BI_RLE8,
};

const W: u32 = 5; // odd width: every row needs DWORD padding on disk
const H: u32 = 3;

fn gradient(bpp: usize, seed: u8) -> Vec<u8> {
    (0..W as usize * H as usize * bpp)
        .map(|i| (i as u8).wrapping_mul(37).wrapping_add(seed))
        .collect()
}

fn image(format: PixelFormat) -> BmpImage {
    let bpp = format.bytes_per_pixel();
    let mut data = gradient(bpp, 11);
    match format {
        PixelFormat::Indexed4 => data.iter_mut().for_each(|b| *b &= 0x0F),
        PixelFormat::Indexed2 => data.iter_mut().for_each(|b| *b &= 0x03),
        PixelFormat::Indexed1 => data.iter_mut().for_each(|b| *b &= 0x01),
        _ => {}
    }
    let img = BmpImage::packed(W, H, format, W as usize * bpp, data).unwrap();
    if format.is_indexed() {
        let n = 1usize << format.bits_per_pixel();
        let entries: Vec<[u8; 3]> = (0..n)
            .map(|i| [i as u8, (i * 7) as u8, (255 - i) as u8])
            .collect();
        img.with_palette(Palette::from_rgb(&entries))
    } else {
        img
    }
}

fn all_formats() -> [PixelFormat; 10] {
    [
        PixelFormat::Rgba,
        PixelFormat::Rgb24,
        PixelFormat::Bgra,
        PixelFormat::Bgr24,
        PixelFormat::Rgb555,
        PixelFormat::Rgb565,
        PixelFormat::Pal8,
        PixelFormat::Indexed4,
        PixelFormat::Indexed2,
        PixelFormat::Indexed1,
    ]
}

/// The layout `decode` hands back for an image encoded from `format`.
fn native_of(format: PixelFormat) -> PixelFormat {
    match format {
        PixelFormat::Rgba => PixelFormat::Bgra,
        PixelFormat::Rgb24 => PixelFormat::Bgr24,
        PixelFormat::Indexed4 | PixelFormat::Indexed2 | PixelFormat::Indexed1 => PixelFormat::Pal8,
        other => other,
    }
}

#[test]
fn probe_is_total() {
    assert!(!probe(&[]));
    assert!(!probe(b"B"));
    assert!(probe(b"BM"));
    assert!(!probe(b"\x89PNG"));
    let bytes = encode(&image(PixelFormat::Rgb24), &EncodeOptions::default()).unwrap();
    assert!(probe(&bytes));
}

#[test]
fn lossless_round_trip_every_layout() {
    for format in all_formats() {
        let img = image(format);
        let (bytes, token) = encode_with_report(&img, &EncodeOptions::default()).unwrap();
        let back = decode(&bytes).unwrap_or_else(|e| panic!("{format:?}: {e}"));
        assert_eq!(back.format, native_of(format), "{format:?}");
        assert_eq!((back.width, back.height), (W, H));
        assert_eq!(back.to_rgba8(), img.to_rgba8(), "{format:?} pixels");
        assert_eq!(back.to_rgb8(), img.to_rgb8(), "{format:?} rgb");
        // Native layouts come back byte for byte, palette included.
        if native_of(format) == format {
            assert_eq!(back.planes, img.planes, "{format:?} planes");
            assert_eq!(back.palette, img.palette, "{format:?} palette");
            assert_eq!(back.metadata, img.metadata, "{format:?} metadata");
        }
        // The report names the variant.
        let expect = match format {
            PixelFormat::Rgba | PixelFormat::Bgra => EncodedBmpFormat::Rgb32,
            PixelFormat::Rgb24 | PixelFormat::Bgr24 => EncodedBmpFormat::Rgb24,
            PixelFormat::Rgb555 => EncodedBmpFormat::Rgb16Rgb,
            PixelFormat::Rgb565 => EncodedBmpFormat::Rgb16Bitfields,
            PixelFormat::Pal8 => EncodedBmpFormat::Indexed8,
            PixelFormat::Indexed4 => EncodedBmpFormat::Indexed4,
            PixelFormat::Indexed2 => EncodedBmpFormat::Indexed2,
            PixelFormat::Indexed1 => EncodedBmpFormat::Indexed1,
            _ => unreachable!(),
        };
        assert_eq!(token, expect, "{format:?} token");
        // `info` agrees with `decode` without touching pixels.
        let i = info(&bytes).unwrap();
        assert_eq!((i.width, i.height, i.frames), (W, H, 1));
        assert_eq!(i.format, back.format, "{format:?} info.format");
        assert_eq!(i.has_alpha, back.has_alpha());
        assert_eq!(i.color, back.color);
        assert_eq!(i.bits_per_pixel, format.bits_per_pixel());
        assert!(!i.has_icc && !i.has_exif && !i.has_xmp);
        assert_eq!(i.compression, token.compression());
    }
}

#[test]
fn sub_byte_indices_unpack_to_pal8() {
    let img = image(PixelFormat::Indexed4);
    let bytes = encode(&img, &EncodeOptions::default()).unwrap();
    let back = decode(&bytes).unwrap();
    assert_eq!(back.format, PixelFormat::Pal8);
    assert_eq!(back.planes[0].stride, W as usize);
    assert_eq!(back.planes[0].data, img.planes[0].data);
    assert_eq!(back.palette.as_ref().unwrap().len(), 16);
    let i = info(&bytes).unwrap();
    assert_eq!(i.bits_per_pixel, 4);
    assert_eq!(i.format, PixelFormat::Pal8);
}

#[test]
fn rle8_decodes_to_pal8_indices() {
    // A flat image compresses, so the encoder picks RLE8.
    let data = vec![3u8; 64 * 8];
    let img = BmpImage::packed(64, 8, PixelFormat::Pal8, 64, data.clone())
        .unwrap()
        .with_palette(Palette::from_rgb(&[
            [0, 0, 0],
            [1, 1, 1],
            [2, 2, 2],
            [9, 8, 7],
        ]));
    let (bytes, token) = encode_with_report(&img, &EncodeOptions::default()).unwrap();
    assert_eq!(token, EncodedBmpFormat::Rle8);
    assert_eq!(info(&bytes).unwrap().compression, BI_RLE8);
    let back = decode(&bytes).unwrap();
    assert_eq!(back.format, PixelFormat::Pal8);
    assert_eq!(back.planes[0].data, data);
    assert_eq!(&back.to_rgb8()[..3], &[9, 8, 7]);
    // `rle: false` forces the raw layout; the pixels are the same.
    let (raw, token) = encode_with_report(&img, &EncodeOptions::default().with_rle(false)).unwrap();
    assert_eq!(token, EncodedBmpFormat::Indexed8);
    assert_eq!(info(&raw).unwrap().compression, BI_RGB);
    assert_eq!(decode(&raw).unwrap().planes, back.planes);
}

#[test]
fn minimal_palette_round_trips_short_tables() {
    let img = BmpImage::packed(W, H, PixelFormat::Pal8, W as usize, vec![1u8; 15])
        .unwrap()
        .with_palette(Palette::from_rgb(&[[10, 20, 30], [40, 50, 60]]));
    let bytes = encode(&img, &EncodeOptions::default().with_minimal_palette(true)).unwrap();
    let back = decode(&bytes).unwrap();
    assert_eq!(back, img);
}

#[test]
fn sixteen_bit_expansion_is_bit_replication() {
    // 0x7FFF: R=G=B=0b11111 → 255; 0x4210: R=0b10000 → 0x84, G=0b10000 → 0x84, B=0b10000 → 0x84.
    let words: Vec<u16> = vec![0x7FFF, 0x4210, 0x0000, 0x8000];
    let data: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
    let img = BmpImage::packed(4, 1, PixelFormat::Rgb555, 8, data).unwrap();
    assert_eq!(
        img.to_rgba8(),
        [255, 255, 255, 255, 0x84, 0x84, 0x84, 255, 0, 0, 0, 255, 0, 0, 0, 255]
    );
    // 5-6-5: 0xFFFF → white; 0x0400 → G = 0b100000 → 0x82.
    let words: Vec<u16> = vec![0xFFFF, 0x0400];
    let data: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
    let img = BmpImage::packed(2, 1, PixelFormat::Rgb565, 4, data).unwrap();
    assert_eq!(img.to_rgb8(), [255, 255, 255, 0, 0x82, 0]);
    // And the decoder agrees with the in-memory kernel.
    let bytes = encode(&img, &EncodeOptions::default()).unwrap();
    let back = decode(&bytes).unwrap();
    assert_eq!(back.format, PixelFormat::Rgb565);
    assert_eq!(back.to_rgb8(), img.to_rgb8());
}

#[test]
fn bgr_layouts_swizzle_and_keep_alpha() {
    let img = BmpImage::packed(1, 1, PixelFormat::Bgra, 4, vec![1, 2, 3, 4]).unwrap();
    assert_eq!(img.to_rgba8(), [3, 2, 1, 4]);
    assert_eq!(img.to_rgb8(), [3, 2, 1]);
    let img = BmpImage::packed(1, 1, PixelFormat::Bgr24, 3, vec![1, 2, 3]).unwrap();
    assert_eq!(img.to_rgba8(), [3, 2, 1, 255]);
    let img = BmpImage::from_rgb8(1, 1, vec![7, 8, 9]).unwrap();
    assert_eq!(img.format(), PixelFormat::Rgb24);
    assert_eq!(img.as_bytes(), Some(&[7u8, 8, 9][..]));
    assert_eq!(img.to_rgba8(), [7, 8, 9, 255]);
    assert_eq!(img.into_raw(), [7, 8, 9]);
}

#[test]
fn raw_constructors_are_fallible() {
    // Short buffer / zero dimension → InvalidData; no panic, no alias.
    assert!(matches!(
        BmpImage::from_rgb8(2, 1, vec![1, 2, 3]),
        Err(Error::InvalidData(_))
    ));
    assert!(matches!(
        BmpImage::from_rgba8(1, 1, vec![1, 2, 3]),
        Err(Error::InvalidData(_))
    ));
    assert!(matches!(
        BmpImage::from_rgba8(0, 1, vec![]),
        Err(Error::InvalidData(_))
    ));
    let img = BmpImage::from_rgba8(1, 1, vec![1, 2, 3, 4]).unwrap();
    assert_eq!(img.format(), PixelFormat::Rgba);
    assert_eq!(img.planes[0].stride, 4);
}

#[test]
fn palette_lookup_pads_missing_entries_opaque_black() {
    let img = BmpImage::packed(3, 1, PixelFormat::Pal8, 3, vec![0, 1, 200])
        .unwrap()
        .with_palette(Palette::from_rgb(&[[1, 2, 3], [4, 5, 6]]));
    assert_eq!(img.to_rgba8(), [1, 2, 3, 255, 4, 5, 6, 255, 0, 0, 0, 255]);
    assert!(img.try_to_rgba8().is_ok());
    let no_pal = BmpImage::packed(1, 1, PixelFormat::Pal8, 1, vec![0]).unwrap();
    assert!(matches!(no_pal.try_to_rgb8(), Err(Error::InvalidData(_))));
    assert_eq!(no_pal.to_rgb8(), [0, 0, 0]);
}

#[test]
fn raw_paths_and_streams() {
    let rgba: Vec<u8> = gradient(4, 3);
    let bytes = encode_rgba8(W, H, &rgba, &EncodeOptions::default()).unwrap();
    let i = info(&bytes).unwrap();
    assert_eq!(
        (i.bits_per_pixel, i.format, i.has_alpha),
        (32, PixelFormat::Bgra, true)
    );
    let back = decode_rgba8(&bytes).unwrap();
    assert_eq!((back.width, back.height), (W, H));
    assert_eq!(back.data, rgba, "alpha survives the 32-bit V3 fourth byte");
    let rgb = decode_rgb8(&bytes).unwrap();
    let expect: Vec<u8> = rgba.chunks(4).flat_map(|p| p[..3].to_vec()).collect();
    assert_eq!(rgb.data, expect);
    assert_eq!(rgb.stride(), W as usize * 3);

    let rgb_src: Vec<u8> = gradient(3, 5);
    let bytes = encode_rgb8(W, H, &rgb_src, &EncodeOptions::default()).unwrap();
    assert_eq!(info(&bytes).unwrap().bits_per_pixel, 24);
    assert_eq!(decode_rgb8(&bytes).unwrap().data, rgb_src);
    let via_stream = decode_from(Cursor::new(&bytes)).unwrap();
    assert_eq!(via_stream, decode(&bytes).unwrap());
    let mut out = Vec::new();
    encode_to(&via_stream, &EncodeOptions::default(), &mut out).unwrap();
    assert_eq!(out, bytes);

    // Wrong buffer length is a geometry error, never a panic.
    assert!(matches!(
        encode_rgb8(W, H, &rgb_src[..10], &EncodeOptions::default()),
        Err(Error::InvalidData(_))
    ));
}

#[test]
fn encode_into_appends_and_fails_without_touching_the_buffer() {
    for format in all_formats() {
        let img = image(format);
        let opts = EncodeOptions::default();
        let bytes = encode(&img, &opts).unwrap();
        let mut out = b"head".to_vec();
        encode_into(&img, &opts, &mut out).unwrap();
        assert_eq!(&out[..4], b"head", "{format:?}");
        assert_eq!(&out[4..], &bytes[..], "{format:?}: same bytes as encode");
        // A second file appends after the first.
        encode_into(&img, &opts, &mut out).unwrap();
        assert_eq!(&out[4 + bytes.len()..], &bytes[..], "{format:?}");
    }
    // Mutually exclusive header options: an error, and the buffer is
    // left exactly as it was (length and capacity).
    let opts = EncodeOptions::default()
        .with_bitfields(BmpBitfields::RGB565)
        .with_calibrated_rgb(CalibratedRgb::new([0; 9], [0; 3]));
    let mut out = b"head".to_vec();
    let capacity = out.capacity();
    let err = encode_into(&image(PixelFormat::Rgba), &opts, &mut out).unwrap_err();
    assert!(matches!(err, Error::Unsupported(_)));
    assert_eq!((out.as_slice(), out.capacity()), (&b"head"[..], capacity));
    // A short plane (the fields are public) fails the same way.
    let mut short = image(PixelFormat::Bgr24);
    short.planes[0].data.truncate(4);
    assert!(matches!(
        encode_into(&short, &EncodeOptions::default(), &mut out),
        Err(Error::InvalidData(_))
    ));
    assert_eq!((out.as_slice(), out.capacity()), (&b"head"[..], capacity));
}

#[test]
fn encoded_size_bound_covers_the_file() {
    for format in all_formats() {
        let img = image(format);
        for opts in [
            EncodeOptions::default(),
            EncodeOptions::default().with_top_down(true),
            EncodeOptions::default().with_rle(false),
        ] {
            let bound = encoded_size_bound(&img, &opts).unwrap();
            let (bytes, written) = encode_with_report(&img, &opts).unwrap();
            let rle_probe = matches!(format, PixelFormat::Pal8 | PixelFormat::Indexed4)
                && opts.rle
                && !opts.top_down;
            if rle_probe {
                // The uncompressed file plus 2 × width + 2.
                let raw = encode(&img, &opts.clone().with_rle(false)).unwrap();
                assert_eq!(bound, raw.len() + 2 * W as usize + 2, "{format:?}");
                assert!(bytes.len() <= raw.len(), "{format:?} {written:?}");
            } else {
                assert_eq!(bound, bytes.len(), "{format:?} {opts:?}: the exact size");
            }
        }
    }
    // The same errors as `encode`.
    let mut short = image(PixelFormat::Rgba);
    short.planes[0].data.truncate(4);
    let opts = EncodeOptions::default();
    assert_eq!(
        encoded_size_bound(&short, &opts).unwrap_err().to_string(),
        encode(&short, &opts).unwrap_err().to_string()
    );
}

#[test]
fn limits_fire_before_decoding() {
    let bytes = encode(&image(PixelFormat::Bgra), &EncodeOptions::default()).unwrap();
    let cases = [
        DecodeOptions::default().with_max_width(W - 1),
        DecodeOptions::default().with_max_height(H - 1),
        DecodeOptions::default().with_max_pixels(u64::from(W * H) - 1),
        DecodeOptions::default().with_max_bytes(u64::from(W * H * 4) - 1),
    ];
    for opts in cases {
        assert!(
            matches!(decode_with(&bytes, &opts), Err(Error::LimitExceeded(_))),
            "{opts:?}"
        );
    }
    let exact = DecodeOptions::default()
        .with_max_width(W)
        .with_max_height(H)
        .with_max_pixels(u64::from(W * H))
        .with_max_bytes(u64::from(W * H * 4));
    assert!(decode_with(&bytes, &exact).is_ok());
    assert!(decode_with(&bytes, &DecodeOptions::default().unlimited()).is_ok());
    assert_eq!(
        DecodeOptions::default().max_bytes,
        Some(DecodeOptions::DEFAULT_MAX_BYTES)
    );
}

#[test]
fn strict_mode_rejects_tolerated_headers() {
    let mut bytes = encode(&image(PixelFormat::Bgr24), &EncodeOptions::default()).unwrap();
    let strict = DecodeOptions::default().with_strict(true);
    assert!(decode_with(&bytes, &strict).is_ok());
    // Non-zero reserved word.
    bytes[6] = 1;
    assert!(decode_with(&bytes, &DecodeOptions::default()).is_ok());
    assert!(matches!(
        decode_with(&bytes, &strict),
        Err(Error::InvalidData(_))
    ));
    bytes[6] = 0;
    // bfOffBits = 0 (the "unset" writers) is recovered leniently only.
    bytes[10..14].copy_from_slice(&0u32.to_le_bytes());
    assert!(decode(&bytes).is_ok());
    assert!(matches!(
        decode_with(&bytes, &strict),
        Err(Error::InvalidData(_))
    ));
}

#[test]
fn colour_and_metadata_follow_the_header() {
    // V3: the BMP default (full-range device RGB).
    let v3 = encode(&image(PixelFormat::Bgr24), &EncodeOptions::default()).unwrap();
    let img = decode(&v3).unwrap();
    assert_eq!(img.color, ColorInfo::bmp_default());
    assert_eq!(img.color.range, ColorRange::Full);
    assert!(!img.color.is_specified());
    assert!(img.metadata.is_empty());

    // V4 LCS_sRGB (the 5-6-5 path) → sRGB code points.
    let v4 = encode(&image(PixelFormat::Rgb565), &EncodeOptions::default()).unwrap();
    assert_eq!(decode(&v4).unwrap().color, ColorInfo::srgb());
    assert_eq!(info(&v4).unwrap().color, ColorInfo::srgb());

    // V5 PROFILE_EMBEDDED: the image's ICC profile goes out and comes back.
    let icc = vec![0xACu8; 37];
    let mut src = image(PixelFormat::Bgra);
    src.metadata.icc = Some(icc.clone());
    let v5 = encode(&src, &EncodeOptions::default()).unwrap();
    let back = decode(&v5).unwrap();
    assert_eq!(back.metadata.icc.as_deref(), Some(&icc[..]));
    assert_eq!(back, src, "ICC images round-trip exactly");
    let i = info(&v5).unwrap();
    assert!(i.has_icc);
    assert_eq!(i.header_size, 124);
    let meta = BmpMetadata::from_bmp(&v5).unwrap();
    assert_eq!(meta.icc_profile.as_deref(), Some(&icc[..]));
    assert!(meta.rendering_intent.is_some());
    // `embed_icc = false` drops it.
    let plain = encode(&src, &EncodeOptions::default().with_embed_icc(false)).unwrap();
    assert!(!info(&plain).unwrap().has_icc);
    assert!(decode(&plain).unwrap().metadata.icc.is_none());
}

#[test]
fn header_families_are_exclusive() {
    let mut src = image(PixelFormat::Bgra);
    src.metadata.icc = Some(vec![1, 2, 3]);
    let cal = CalibratedRgb::new([0; 9], [0x0002_3333; 3]);
    let both = EncodeOptions::default().with_calibrated_rgb(cal);
    assert!(matches!(encode(&src, &both), Err(Error::Unsupported(_))));
    let both = EncodeOptions::default()
        .with_bitfields(BmpBitfields::BGRA8888)
        .with_linked_icc(b"c:\\p.icc\0".to_vec());
    assert!(matches!(encode(&src, &both), Err(Error::Unsupported(_))));
    // Each alone works and decodes.
    src.metadata.icc = None;
    let cal_bytes = encode(&src, &EncodeOptions::default().with_calibrated_rgb(cal)).unwrap();
    assert_eq!(
        BmpMetadata::from_bmp(&cal_bytes).unwrap().gamma_rgb,
        Some([0x0002_3333; 3])
    );
    assert_eq!(decode(&cal_bytes).unwrap().to_rgba8(), src.to_rgba8());
    let (bf, token) = encode_with_report(
        &src,
        &EncodeOptions::default().with_bitfields(BmpBitfields::BGRA8888),
    )
    .unwrap();
    assert_eq!(token, EncodedBmpFormat::AlphaBitfields);
    assert_eq!(info(&bf).unwrap().compression, BI_ALPHABITFIELDS);
    let (bf_opaque, token) = encode_with_report(
        &src,
        &EncodeOptions::default().with_bitfields(BmpBitfields::BGRX8888),
    )
    .unwrap();
    assert_eq!(token, EncodedBmpFormat::Bitfields);
    assert_eq!(info(&bf_opaque).unwrap().compression, BI_BITFIELDS);
    assert!(decode(&bf_opaque)
        .unwrap()
        .to_rgba8()
        .chunks(4)
        .all(|p| p[3] == 0xFF));
    let back = decode(&bf).unwrap();
    assert_eq!(back.format, PixelFormat::Bgra);
    assert_eq!(back.planes, src.planes);
    // Bitfields on an indexed image: not representable.
    assert!(matches!(
        encode(
            &image(PixelFormat::Pal8),
            &EncodeOptions::default().with_bitfields(BmpBitfields::RGB565)
        ),
        Err(Error::Unsupported(_))
    ));
}

#[test]
fn top_down_and_bottom_up_decode_identically() {
    let img = image(PixelFormat::Bgr24);
    let up = encode(&img, &EncodeOptions::default()).unwrap();
    let down = encode(&img, &EncodeOptions::default().with_top_down(true)).unwrap();
    assert!(!info(&up).unwrap().top_down);
    assert!(info(&down).unwrap().top_down);
    assert_eq!(decode(&up).unwrap().planes, decode(&down).unwrap().planes);
}

#[test]
fn constructors_validate_geometry() {
    assert!(matches!(
        BmpImage::new(0, 1, PixelFormat::Rgba, vec![Plane::new(4, vec![0; 4])]),
        Err(Error::InvalidData(_))
    ));
    assert!(matches!(
        BmpImage::new(2, 1, PixelFormat::Rgba, vec![Plane::new(4, vec![0; 8])]),
        Err(Error::InvalidData(_))
    ));
    assert!(matches!(
        BmpImage::new(1, 2, PixelFormat::Rgba, vec![Plane::new(4, vec![0; 4])]),
        Err(Error::InvalidData(_))
    ));
    assert!(matches!(
        BmpImage::new(1, 1, PixelFormat::Rgba, vec![]),
        Err(Error::InvalidData(_))
    ));
    let ok = BmpImage::new(1, 1, PixelFormat::Rgba, vec![Plane::new(8, vec![0; 8])]).unwrap();
    assert_eq!(ok.stride(), 8);
    // An indexed image without a palette cannot be encoded.
    let pal8 = BmpImage::packed(1, 1, PixelFormat::Pal8, 1, vec![0]).unwrap();
    assert!(matches!(
        encode(&pal8, &EncodeOptions::default()),
        Err(Error::InvalidData(_))
    ));
}

#[test]
fn errors_carry_io_and_display() {
    struct Failing;
    impl std::io::Read for Failing {
        fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("boom"))
        }
    }
    let err = decode_from(Failing).unwrap_err();
    assert!(matches!(err, Error::Io(_)));
    assert!(err.to_string().contains("boom"));
    assert!(std::error::Error::source(&err).is_some());
    let err = decode(b"BMxx").unwrap_err();
    assert!(matches!(err, Error::InvalidData(_)));
    assert!(err.to_string().starts_with("invalid data"));
}

#[test]
fn hostile_inputs_never_panic() {
    let good = encode(&image(PixelFormat::Pal8), &EncodeOptions::default()).unwrap();
    for len in 0..good.len() {
        let _ = probe(&good[..len]);
        let _ = info(&good[..len]);
        let _ = decode(&good[..len]);
    }
    for i in 0..good.len() {
        let mut m = good.clone();
        m[i] ^= 0xFF;
        let _ = info(&m);
        if let Ok(img) = decode(&m) {
            let _ = img.to_rgba8();
        }
    }
}
