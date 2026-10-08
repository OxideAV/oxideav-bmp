//! Byte-identity pin for the encoder.
//!
//! The encoder output is fixed: a change to how the encoder buffers or
//! orders its work must not change one output byte. This file pins the
//! output of every encode entry point against digests recorded from the
//! v0.1.8 encoder (commit `103d759`):
//!
//! * synthetic images in every [`PixelFormat`], at odd widths (so every
//!   row carries DWORD padding on disk), with and without stride padding
//!   in the source plane, with noisy, run-heavy, mixed and constant
//!   content (runs of 8 pixels, and of 256 and 600 pixels, past the
//!   255-pixel RLE packet), through every [`EncodeOptions`] family
//!   (default, no RLE, minimal palette, the five bitfield presets, V4
//!   calibrated RGB, V5 embedded / linked / both profiles, embedding
//!   switched off, mutually exclusive options), each in both row orders;
//! * indexed images with an empty and an oversized palette;
//! * images changed through the public fields after validation: a
//!   truncated plane, no plane at all, a stride below the row width, a
//!   zero width or height, an indexed image without a palette;
//! * the well-formed seed images under `fuzz/corpus/decode/`, decoded
//!   and re-encoded through the same families;
//! * the headerless DIB ([`encode_dib`] and the deprecated
//!   [`encode_dib_plane`]), both mask layouts, for every image above;
//! * images over 64 KiB, also written through [`encode_to`] into a
//!   writer that takes at most 1000 bytes per call.
//!
//! Every entry point's result goes into the digest: [`encode_with_report`]
//! (bytes and the reported [`EncodedBmpFormat`], so the RLE-or-raw choice
//! is pinned), the plane-level, bitfields, calibrated-RGB, ICC and linked
//! deprecated names, [`encode_rgb8`] / [`encode_rgba8`], each as bytes or
//! as the error message. [`encode`], [`encode_to`], [`encode_into`],
//! [`encode_bmp`] and [`encode_bmp_with_options`] must return exactly
//! what [`encode_with_report`] returns, error messages included,
//! [`encoded_size_bound`] must fail as it does or cover the file, and
//! any entry point that succeeds must write the bytes `encode` writes. With
//! the `registry` feature the registry encoder and the deprecated
//! `VideoFrame` names are pinned the same way, for the synthetic and the
//! seed images, in their own table.
//!
//! Each digest is FNV-1a 64. To re-record after an intentional output
//! change, run
//! `OXIDEAV_BMP_PRINT_DIGESTS=1 cargo test --test encode_identity -- --nocapture`
//! and paste the printed tables over `GOLDEN`, `GOLDEN_LARGE` and
//! `GOLDEN_REGISTRY`.

// The pre-contract entry points are part of what this file pins.
#![allow(deprecated)]

use oxideav_bmp::{
    decode, encode, encode_bmp, encode_bmp_bitfields, encode_bmp_plane, encode_bmp_plane_bitfields,
    encode_bmp_plane_with_options, encode_bmp_with_calibrated_rgb, encode_bmp_with_icc_profile,
    encode_bmp_with_linked_icc_profile, encode_bmp_with_options, encode_dib, encode_dib_plane,
    encode_into, encode_rgb8, encode_rgba8, encode_to, encode_with_report, encoded_size_bound,
    BmpBitfields, BmpImage, BmpPalette, CalibratedRgb, EncodeOptions, EncodedBmpFormat, Error,
    Palette, PixelFormat, LCS_GM_BUSINESS,
};

const FORMATS: [PixelFormat; 10] = [
    PixelFormat::Rgba,
    PixelFormat::Bgra,
    PixelFormat::Rgb24,
    PixelFormat::Bgr24,
    PixelFormat::Rgb555,
    PixelFormat::Rgb565,
    PixelFormat::Pal8,
    PixelFormat::Indexed4,
    PixelFormat::Indexed2,
    PixelFormat::Indexed1,
];

const ICC: &[u8] = b"not a real ICC profile, odd length: 37";
const LINKED: &[u8] = b"C:\\Windows\\System32\\spool\\drivers\\color\\sRGB.icm\0";
const ENDPOINTS: [i32; 9] = [
    0x0000_6666,
    0x0000_3333,
    0x0000_0303,
    0x0000_4ccc,
    0x0000_9999,
    0x0000_1919,
    0x0000_2666,
    0x0000_0f0f,
    0x0000_cccc,
];
const GAMMA: [u32; 3] = [0x0002_3333, 0x0002_3333, 0x0002_3333];

/// FNV-1a 64: a dependency-free, order-sensitive digest.
struct Fnv(u64);

impl Fnv {
    fn new() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }

    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 ^= u64::from(b);
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
}

fn xorshift(state: &mut u32) -> u8 {
    *state ^= *state << 13;
    *state ^= *state >> 17;
    *state ^= *state << 5;
    (*state >> 8) as u8
}

#[derive(Clone, Copy)]
enum Content {
    /// Every byte random: RLE loses.
    Noise,
    /// Runs of eight equal pixels: RLE wins.
    Runs,
    /// A run-heavy left half and a random right half: RLE wins with
    /// absolute-mode stretches in the stream.
    Mixed,
    /// Every pixel of a row equal: one run the width of the row, split
    /// into 255-pixel RLE packets.
    Constant,
}

/// One synthetic source image. `pad` extra bytes per source row (filled
/// with noise the encoder must ignore); `short_palette` gives indexed
/// layouts a 3-entry table instead of a full `2^bpp` one, so
/// `minimal_palette` changes the output.
fn synthetic(
    format: PixelFormat,
    width: u32,
    height: u32,
    pad: usize,
    content: Content,
    short_palette: bool,
    seed: u32,
) -> BmpImage {
    let bpp = format.bytes_per_pixel();
    let stride = width as usize * bpp + pad;
    let mut state = seed | 1;
    let mut data = vec![0u8; stride * height as usize];
    for y in 0..height as usize {
        let row = &mut data[y * stride..(y + 1) * stride];
        for (i, b) in row.iter_mut().enumerate() {
            let x = i / bpp;
            let noise = xorshift(&mut state);
            // Runs of eight equal pixels whose channels differ, so a
            // channel swizzle shows in the output.
            let run = ((x / 8) as u8)
                .wrapping_mul(17)
                .wrapping_add(y as u8)
                .wrapping_add((i % bpp) as u8 * 50);
            *b = match content {
                Content::Constant => (y as u8)
                    .wrapping_mul(29)
                    .wrapping_add((i % bpp) as u8 * 50),
                Content::Noise => noise,
                Content::Runs if i < width as usize * bpp => run,
                Content::Mixed if x < width as usize / 2 => run,
                _ => noise,
            };
        }
    }
    let img = BmpImage::packed(width, height, format, stride, data).unwrap();
    if !format.is_indexed() {
        return img;
    }
    let full = 1usize << format.bits_per_pixel();
    let n = if short_palette { full.min(3) } else { full };
    // The alpha byte varies: the encoder must write 0 into the reserved
    // RGBQUAD byte regardless.
    let entries: Vec<[u8; 4]> = (0..n)
        .map(|i| [i as u8, (i * 7) as u8, (255 - i) as u8, (i * 3) as u8])
        .collect();
    img.with_palette(Palette::new(entries))
}

/// The labelled source images for one layout, including the two error
/// cases.
fn images(format: PixelFormat) -> Vec<(&'static str, BmpImage)> {
    let mut out = vec![
        ("1x1", synthetic(format, 1, 1, 0, Content::Noise, false, 1)),
        ("5x3", synthetic(format, 5, 3, 0, Content::Noise, true, 2)),
        (
            "13x4+pad3",
            synthetic(format, 13, 4, 3, Content::Noise, false, 3),
        ),
        (
            "32x8 runs",
            synthetic(format, 32, 8, 0, Content::Runs, false, 4),
        ),
        (
            "31x6+pad2 runs",
            synthetic(format, 31, 6, 2, Content::Runs, true, 5),
        ),
        (
            "40x5 mixed",
            synthetic(format, 40, 5, 0, Content::Mixed, false, 6),
        ),
        (
            "256x2 constant",
            synthetic(format, 256, 2, 0, Content::Constant, false, 11),
        ),
        (
            "600x3+pad1 constant",
            synthetic(format, 600, 3, 1, Content::Constant, true, 12),
        ),
    ];
    if format.is_indexed() {
        let empty = synthetic(format, 5, 3, 0, Content::Noise, false, 13);
        out.push((
            "empty palette",
            empty.with_palette(Palette::new(Vec::new())),
        ));
        let oversized = synthetic(format, 5, 3, 0, Content::Noise, false, 14);
        let entries = (0..300u32)
            .map(|i| [i as u8, (i / 2) as u8, 7, 0])
            .collect();
        out.push((
            "300-entry palette",
            oversized.with_palette(Palette::new(entries)),
        ));
    }
    // A plane shortened after validation (the fields are public).
    let mut truncated = synthetic(format, 5, 3, 0, Content::Noise, false, 7);
    truncated.planes[0].data.pop();
    out.push(("truncated", truncated));
    // No plane at all.
    let mut no_planes = synthetic(format, 5, 3, 0, Content::Noise, false, 15);
    no_planes.planes.clear();
    out.push(("no planes", no_planes));
    // A stride one byte below the row width, with enough data for the
    // last row: rows overlap in the source.
    let mut short_stride = synthetic(format, 5, 3, 0, Content::Noise, false, 16);
    short_stride.planes[0].stride -= 1;
    out.push(("short stride", short_stride));
    // Dimensions zeroed after validation: every file encoder rejects
    // them, the headerless DIB writes its header (and colour table)
    // alone.
    let mut zero_width = synthetic(format, 5, 3, 0, Content::Noise, false, 9);
    zero_width.width = 0;
    out.push(("zero width", zero_width));
    let mut zero_height = synthetic(format, 5, 3, 0, Content::Noise, false, 10);
    zero_height.height = 0;
    out.push(("zero height", zero_height));
    if format.is_indexed() {
        let no_palette = synthetic(format, 5, 3, 0, Content::Noise, false, 8).with_palette(None);
        out.push(("no palette", no_palette));
    }
    out
}

/// One encode request: the options, plus whether the image carries the
/// ICC profile in its metadata.
struct Case {
    label: String,
    options: EncodeOptions,
    with_icc: bool,
}

/// Every option family, each in both row orders.
fn cases() -> Vec<Case> {
    let presets = [
        ("RGB565", BmpBitfields::RGB565),
        ("RGB555", BmpBitfields::RGB555),
        ("ARGB1555", BmpBitfields::ARGB1555),
        ("BGRA8888", BmpBitfields::BGRA8888),
        ("BGRX8888", BmpBitfields::BGRX8888),
    ];
    let calibrated = CalibratedRgb::new(ENDPOINTS, GAMMA);
    let mut families: Vec<(String, EncodeOptions, bool)> = vec![
        ("default".into(), EncodeOptions::default(), false),
        (
            "no_rle".into(),
            EncodeOptions::default().with_rle(false),
            false,
        ),
        (
            "minimal_palette".into(),
            EncodeOptions::default().with_minimal_palette(true),
            false,
        ),
        (
            "calibrated".into(),
            EncodeOptions::default()
                .with_calibrated_rgb(calibrated)
                .with_minimal_palette(true),
            false,
        ),
        ("embed_icc".into(), EncodeOptions::default(), true),
        (
            "icc_not_embedded".into(),
            EncodeOptions::default().with_embed_icc(false),
            true,
        ),
        (
            "linked_icc".into(),
            EncodeOptions::default()
                .with_linked_icc(LINKED.to_vec())
                .with_rendering_intent(LCS_GM_BUSINESS)
                .with_minimal_palette(true),
            false,
        ),
        (
            "linked_and_embedded".into(),
            EncodeOptions::default()
                .with_linked_icc(LINKED.to_vec())
                .with_rendering_intent(LCS_GM_BUSINESS),
            true,
        ),
        (
            "exclusive".into(),
            EncodeOptions::default()
                .with_bitfields(BmpBitfields::RGB565)
                .with_calibrated_rgb(calibrated),
            false,
        ),
    ];
    for (name, masks) in presets {
        families.push((
            format!("bitfields {name}"),
            EncodeOptions::default().with_bitfields(masks),
            false,
        ));
    }
    let mut out = Vec::new();
    for (name, options, with_icc) in families {
        for top_down in [false, true] {
            out.push(Case {
                label: format!("{name}{}", if top_down { " top_down" } else { "" }),
                options: options.clone().with_top_down(top_down),
                with_icc,
            });
        }
    }
    out
}

fn with_icc(image: &BmpImage, icc: bool) -> BmpImage {
    let mut image = image.clone();
    if icc {
        image.metadata.icc = Some(ICC.to_vec());
    }
    image
}

/// Fold one encode result into `digest`.
fn absorb(digest: &mut Fnv, label: &str, result: &Result<(Vec<u8>, EncodedBmpFormat), Error>) {
    digest.write(label.as_bytes());
    match result {
        Ok((bytes, format)) => {
            digest.write(format!("{format:?}").as_bytes());
            digest.write(&(bytes.len() as u64).to_le_bytes());
            digest.write(bytes);
        }
        Err(e) => digest.write(format!("error: {e}").as_bytes()),
    }
}

/// Fold the result of an entry point that returns bytes only.
fn absorb_bytes(digest: &mut Fnv, label: &str, result: &Result<Vec<u8>, Error>) {
    digest.write(label.as_bytes());
    match result {
        Ok(bytes) => {
            digest.write(&(bytes.len() as u64).to_le_bytes());
            digest.write(bytes);
        }
        Err(e) => digest.write(format!("error: {e}").as_bytes()),
    }
}

/// A result with its error replaced by the error's message.
fn msg<T: Clone>(r: &Result<T, Error>) -> Result<T, String> {
    match r {
        Ok(v) => Ok(v.clone()),
        Err(e) => Err(e.to_string()),
    }
}

/// `a` and `b` are the same result: equal values, or errors with the
/// same message.
fn exact<T: PartialEq>(what: &str, a: Result<T, String>, b: Result<T, String>) {
    match (&a, &b) {
        (Err(x), Err(y)) => assert_eq!(x, y, "{what}"),
        (Ok(_), Ok(_)) => assert!(a == b, "{what}: values differ"),
        (Err(x), _) | (_, Err(x)) => panic!("{what}: only one side failed: {x}"),
    }
}

/// An entry point that succeeds writes what [`encode_with_report`]
/// writes. (Its errors are pinned through the digest: the plane-level
/// names validate the geometry first and word their errors differently.)
fn agrees(
    what: &str,
    via: &Result<Vec<u8>, Error>,
    report: &Result<(Vec<u8>, EncodedBmpFormat), Error>,
) {
    if let (Ok(a), Ok((b, _))) = (via, report) {
        assert!(a == b, "{what}: bytes differ from encode_with_report");
    }
}

/// The plane rows `encode_rgb8` / `encode_rgba8` take: each row's pixel
/// bytes, back to back, clamped to what the plane holds.
fn tight(image: &BmpImage) -> Vec<u8> {
    let plane = &image.planes[0];
    let row = image.width as usize * image.format.bytes_per_pixel();
    let mut out = Vec::new();
    for y in 0..image.height as usize {
        let start = (y * plane.stride).min(plane.data.len());
        let end = (start + row).min(plane.data.len());
        out.extend_from_slice(&plane.data[start..end]);
    }
    out
}

/// Run every entry point on `image` + `case`: fold each result into
/// `digest`, and check the ones that must match [`encode_with_report`].
fn entry_points(
    what: &str,
    image: &BmpImage,
    case: &Case,
    report: &Result<(Vec<u8>, EncodedBmpFormat), Error>,
    digest: &mut Fnv,
) {
    let opts = &case.options;
    let report_bytes = report
        .as_ref()
        .map(|(b, _)| b.clone())
        .map_err(|e| e.to_string());

    exact(
        &format!("{what}: encode"),
        msg(&encode(image, opts)),
        report_bytes.clone(),
    );
    let mut streamed = Vec::new();
    let via_stream = encode_to(image, opts, &mut streamed).map(|()| streamed.clone());
    exact(
        &format!("{what}: encode_to"),
        msg(&via_stream),
        report_bytes.clone(),
    );
    if via_stream.is_err() {
        assert!(
            streamed.is_empty(),
            "{what}: encode_to wrote before failing"
        );
    }
    let mut appended = b"prefix".to_vec();
    let via_into = encode_into(image, opts, &mut appended).map(|()| appended[6..].to_vec());
    exact(
        &format!("{what}: encode_into"),
        msg(&via_into),
        report_bytes.clone(),
    );
    assert_eq!(&appended[..6], b"prefix", "{what}: encode_into");
    if via_into.is_err() {
        assert_eq!(
            appended.len(),
            6,
            "{what}: encode_into kept bytes after failing"
        );
    }
    match (encoded_size_bound(image, opts), &report_bytes) {
        (Ok(bound), Ok(bytes)) => assert!(bytes.len() <= bound, "{what}: encoded_size_bound"),
        (Err(a), Err(b)) => assert_eq!(&a.to_string(), b, "{what}: encoded_size_bound"),
        // The one error the bound cannot see: decided by the RLE probe
        // during the encode (an over-limit uncompressed file).
        (Ok(_), Err(b)) => assert!(
            b.contains("RLE stream does not bring it under"),
            "{what}: encoded_size_bound succeeded where encode failed: {b}"
        ),
        (Err(a), Ok(_)) => panic!("{what}: encoded_size_bound failed where encode did not: {a}"),
    }
    exact(
        &format!("{what}: encode_bmp_with_options"),
        msg(&encode_bmp_with_options(image, opts.clone())),
        msg(report),
    );
    if *opts == EncodeOptions::default() {
        exact(
            &format!("{what}: encode_bmp"),
            msg(&encode_bmp(image)),
            msg(report),
        );
    }

    // Plane-level names: no metadata, the palette as RGB triplets.
    let palette = image.palette.as_ref().map(|p| BmpPalette::from(p.clone()));
    let (w, h, format) = (image.width, image.height, image.format);
    if let (Some(plane), false) = (image.planes.first(), case.with_icc) {
        let via =
            encode_bmp_plane_with_options(plane, format, palette.as_ref(), w, h, opts.clone());
        if *opts == EncodeOptions::default() {
            exact(
                &format!("{what}: encode_bmp_plane"),
                msg(&encode_bmp_plane(plane, format, palette.as_ref(), w, h)),
                msg(&via),
            );
        }
        let via = via.map(|(b, _)| b);
        agrees(
            &format!("{what}: encode_bmp_plane_with_options"),
            &via,
            report,
        );
        absorb_bytes(digest, "plane", &via);
        if let (Some(masks), None) = (opts.bitfields, opts.calibrated_rgb) {
            let plain = opts.clone().with_bitfields(None);
            let via = encode_bmp_bitfields(image, masks, plain.clone());
            agrees(&format!("{what}: encode_bmp_bitfields"), &via, report);
            absorb_bytes(digest, "bitfields", &via);
            let via = encode_bmp_plane_bitfields(plane, format, masks, w, h, plain);
            if image.palette.is_none() {
                agrees(&format!("{what}: encode_bmp_plane_bitfields"), &via, report);
            }
            absorb_bytes(digest, "plane bitfields", &via);
        }
        if let (Some(cal), None) = (opts.calibrated_rgb, opts.bitfields) {
            let plain = opts.clone().with_calibrated_rgb(None);
            let via = encode_bmp_with_calibrated_rgb(image, cal.endpoints, cal.gamma, plain);
            agrees(
                &format!("{what}: encode_bmp_with_calibrated_rgb"),
                &via,
                report,
            );
            absorb_bytes(digest, "calibrated", &via);
        }
        if let Some(path) = opts.linked_icc.as_deref() {
            let plain = opts.clone().with_linked_icc(None);
            let via = encode_bmp_with_linked_icc_profile(image, path, opts.rendering_intent, plain);
            agrees(
                &format!("{what}: encode_bmp_with_linked_icc_profile"),
                &via,
                report,
            );
            absorb_bytes(digest, "linked", &via);
        }
        // The raw-buffer convenience names, for the layouts they take.
        let rgb = match format {
            PixelFormat::Rgb24 => Some(encode_rgb8(w, h, &tight(image), opts)),
            PixelFormat::Rgba => Some(encode_rgba8(w, h, &tight(image), opts)),
            _ => None,
        };
        if let Some(via) = rgb {
            agrees(&format!("{what}: encode_rgb8 / encode_rgba8"), &via, report);
            absorb_bytes(digest, "raw buffer", &via);
        }
    } else if case.with_icc && opts.embed_icc && opts.linked_icc.is_none() {
        let mut bare = image.clone();
        bare.metadata.icc = None;
        let via = encode_bmp_with_icc_profile(&bare, ICC, opts.rendering_intent, opts.clone());
        agrees(
            &format!("{what}: encode_bmp_with_icc_profile"),
            &via,
            report,
        );
        absorb_bytes(digest, "icc", &via);
    }
}

/// Encode `image` through every case, check every entry point, and fold
/// the results into one digest. Every on-disk variant written is
/// recorded in `seen`.
fn digest_all(name: &str, image: &BmpImage, seen: &mut Vec<EncodedBmpFormat>) -> Fnv {
    let mut digest = Fnv::new();
    for case in cases() {
        let image = with_icc(image, case.with_icc);
        let result = encode_with_report(&image, &case.options);
        absorb(&mut digest, &case.label, &result);
        entry_points(
            &format!("{name} / {}", case.label),
            &image,
            &case,
            &result,
            &mut digest,
        );
        if let Ok((_, format)) = &result {
            if !seen.contains(format) {
                seen.push(*format);
            }
        }
    }
    digest
}

fn digest_dib(image: &BmpImage) -> Fnv {
    let mut digest = Fnv::new();
    let palette = image.palette.as_ref().map(|p| BmpPalette::from(p.clone()));
    for doubled in [false, true] {
        let result = encode_dib(image, doubled);
        absorb_bytes(&mut digest, &format!("dib doubled={doubled}"), &result);
        // The plane-level name validates the geometry first, so it
        // rejects what `encode_dib` writes as a bare header.
        if let Some(plane) = image.planes.first() {
            let plane = encode_dib_plane(
                plane,
                image.format,
                palette.as_ref(),
                image.width,
                image.height,
                doubled,
            );
            absorb_bytes(&mut digest, &format!("dib plane doubled={doubled}"), &plane);
        }
    }
    digest
}

/// The well-formed seed images under `fuzz/corpus/decode/`, decoded to
/// their native layout, in file-name order.
fn corpus() -> Vec<(String, BmpImage)> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/decode");
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("corpus dir missing at {}: {e}", dir.display()))
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "bmp"))
        .collect();
    files.sort();
    assert!(!files.is_empty(), "no seed images under {}", dir.display());
    files
        .into_iter()
        .map(|path| {
            let name = format!("corpus {}", path.file_name().unwrap().to_string_lossy());
            (name, decode(&std::fs::read(&path).unwrap()).unwrap())
        })
        .collect()
}

fn actual_digests() -> Vec<(String, u64)> {
    let mut out = Vec::new();
    let mut seen = Vec::new();
    for format in FORMATS {
        let images = images(format);
        let mut dib = Fnv::new();
        for (label, image) in &images {
            let name = format!("{format:?} {label}");
            out.push((name.clone(), digest_all(&name, image, &mut seen).0));
            dib.write(&digest_dib(image).0.to_le_bytes());
        }
        out.push((format!("{format:?} dib"), dib.0));
    }
    for (name, image) in corpus() {
        let mut digest = digest_all(&name, &image, &mut seen);
        digest.write(&digest_dib(&image).0.to_le_bytes());
        out.push((name, digest.0));
    }
    // The cases reach every on-disk variant, RLE wins and losses
    // included.
    for format in [
        EncodedBmpFormat::Rgb32,
        EncodedBmpFormat::Rgb24,
        EncodedBmpFormat::Rgb16Rgb,
        EncodedBmpFormat::Rgb16Bitfields,
        EncodedBmpFormat::Indexed8,
        EncodedBmpFormat::Indexed4,
        EncodedBmpFormat::Indexed2,
        EncodedBmpFormat::Indexed1,
        EncodedBmpFormat::Rle8,
        EncodedBmpFormat::Rle4,
        EncodedBmpFormat::Bitfields,
        EncodedBmpFormat::AlphaBitfields,
    ] {
        assert!(seen.contains(&format), "no case writes {format:?}");
    }
    out
}

/// Compare `actual` with a recorded table, or print it in the table's
/// syntax when `OXIDEAV_BMP_PRINT_DIGESTS` is set.
fn compare(table: &str, actual: &[(String, u64)], golden: &[(&str, u64)]) {
    if std::env::var_os("OXIDEAV_BMP_PRINT_DIGESTS").is_some() {
        println!("const {table}: &[(&str, u64)] = &[");
        for (name, digest) in actual {
            println!("    ({name:?}, {digest:#018x}),");
        }
        println!("];");
        return;
    }
    let names: Vec<&str> = actual.iter().map(|(n, _)| n.as_str()).collect();
    let golden_names: Vec<&str> = golden.iter().map(|(n, _)| *n).collect();
    assert_eq!(names, golden_names, "{table}: the case list changed");
    let changed: Vec<&str> = actual
        .iter()
        .zip(golden)
        .filter(|(a, g)| a.1 != g.1)
        .map(|(a, _)| a.0.as_str())
        .collect();
    assert!(
        changed.is_empty(),
        "{table}: encoder output changed for: {changed:?}"
    );
}

#[test]
fn every_encode_path_is_byte_identical_to_v0_1_8() {
    compare("GOLDEN", &actual_digests(), GOLDEN);
}

/// A writer that takes at most `limit` bytes per call, so `write_all`
/// has to loop.
struct Trickle {
    out: Vec<u8>,
    limit: usize,
}

impl std::io::Write for Trickle {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let n = buf.len().min(self.limit);
        self.out.extend_from_slice(&buf[..n]);
        Ok(n)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Images larger than the 64 KiB chunk `encode_to` writes in, so the
/// streamed raw rows, the streamed RLE stream and the V5 profile blob
/// all cross several chunk boundaries.
fn large_cases() -> Vec<(&'static str, BmpImage, EncodeOptions)> {
    let icc = |image: BmpImage| with_icc(&image, true);
    vec![
        (
            "Rgba 300x300 noise",
            synthetic(PixelFormat::Rgba, 300, 300, 0, Content::Noise, false, 11),
            EncodeOptions::default(),
        ),
        (
            "Bgra 300x300+pad1 noise top_down",
            synthetic(PixelFormat::Bgra, 300, 300, 1, Content::Noise, false, 12),
            EncodeOptions::default().with_top_down(true),
        ),
        (
            "Rgb24 301x200 mixed embed_icc",
            icc(synthetic(
                PixelFormat::Rgb24,
                301,
                200,
                0,
                Content::Mixed,
                false,
                13,
            )),
            EncodeOptions::default(),
        ),
        (
            "Rgb565 333x200 noise calibrated",
            synthetic(PixelFormat::Rgb565, 333, 200, 0, Content::Noise, false, 14),
            EncodeOptions::default().with_calibrated_rgb(CalibratedRgb::new(ENDPOINTS, GAMMA)),
        ),
        (
            "Bgr24 257x300 noise bitfields BGRA8888",
            synthetic(PixelFormat::Bgr24, 257, 300, 0, Content::Noise, false, 15),
            EncodeOptions::default().with_bitfields(BmpBitfields::BGRA8888),
        ),
        (
            "Pal8 1024x512 mixed (RLE8 wins)",
            synthetic(PixelFormat::Pal8, 1024, 512, 0, Content::Mixed, false, 16),
            EncodeOptions::default(),
        ),
        (
            "Pal8 1024x300 noise (RLE8 loses)",
            synthetic(PixelFormat::Pal8, 1024, 300, 0, Content::Noise, false, 17),
            EncodeOptions::default(),
        ),
        (
            "Indexed4 1023x512+pad3 mixed (RLE4 wins)",
            synthetic(
                PixelFormat::Indexed4,
                1023,
                512,
                3,
                Content::Mixed,
                true,
                18,
            ),
            EncodeOptions::default().with_minimal_palette(true),
        ),
        (
            "Indexed1 2000x300 mixed linked_icc",
            synthetic(
                PixelFormat::Indexed1,
                2000,
                300,
                0,
                Content::Mixed,
                false,
                19,
            ),
            EncodeOptions::default().with_linked_icc(LINKED.to_vec()),
        ),
    ]
}

#[test]
fn large_images_are_byte_identical_to_v0_1_8_through_every_sink() {
    // `encode_to` hands `Trickle` whole chunks; `write_all` loops.
    let mut actual = Vec::new();
    for (name, image, opts) in large_cases() {
        let result = encode_with_report(&image, &opts);
        let (bytes, format) = result.as_ref().unwrap_or_else(|e| panic!("{name}: {e}"));
        let mut digest = Fnv::new();
        absorb(&mut digest, name, &result);
        actual.push((name.to_string(), digest.0));
        if name.contains("wins") {
            assert!(
                matches!(format, EncodedBmpFormat::Rle8 | EncodedBmpFormat::Rle4),
                "{name}: {format:?}"
            );
        }
        let mut trickle = Trickle {
            out: Vec::new(),
            limit: 1000,
        };
        encode_to(&image, &opts, &mut trickle).unwrap();
        assert!(trickle.out == *bytes, "{name}: encode_to");
        let mut appended = b"prefix".to_vec();
        encode_into(&image, &opts, &mut appended).unwrap();
        assert!(appended[6..] == bytes[..], "{name}: encode_into");
    }
    compare("GOLDEN_LARGE", &actual, GOLDEN_LARGE);
}

/// The registry encoder and the deprecated `VideoFrame` names, pinned
/// in their own table because they exist only with the `registry`
/// feature.
#[cfg(feature = "registry")]
mod registry {
    use super::*;
    use oxideav_bmp::{
        encode_bmp_videoframe, encode_dib_videoframe, make_encoder, to_core_pixel_format,
    };
    use oxideav_core::{CodecId, CodecOptions, CodecParameters, Frame, VideoFrame};

    /// The message a registry error carries: the `BmpError` message the
    /// adapter maps into the framework error, where there is one.
    fn message(e: &oxideav_core::Error) -> String {
        match e {
            oxideav_core::Error::InvalidData(s)
            | oxideav_core::Error::Unsupported(s)
            | oxideav_core::Error::ResourceExhausted(s) => s.clone(),
            other => format!("{other:?}"),
        }
    }

    fn absorb_core(digest: &mut Fnv, label: &str, result: &Result<Vec<u8>, oxideav_core::Error>) {
        let result = match result {
            Ok(bytes) => Ok(bytes.clone()),
            Err(e) => Err(Error::invalid(message(e))),
        };
        absorb_bytes(digest, label, &result);
    }

    /// Every registry path for `image`, folded into one digest: the
    /// registry encoder for every case the codec options can express
    /// (which must write what `encode_with_report` writes when it
    /// succeeds), `encode_bmp_videoframe`, and `encode_dib_videoframe`
    /// in both mask layouts.
    fn digest(name: &str, image: &BmpImage) -> Option<u64> {
        let pixel_format = to_core_pixel_format(image.format)?;
        let mut digest = Fnv::new();
        let frame = VideoFrame::from(image);
        for case in cases() {
            let opts = &case.options;
            if case.with_icc
                || opts.bitfields.is_some()
                || opts.calibrated_rgb.is_some()
                || opts.linked_icc.is_some()
            {
                continue;
            }
            let mut params = CodecParameters::video(CodecId::new("bmp"));
            params.width = Some(image.width);
            params.height = Some(image.height);
            params.pixel_format = Some(pixel_format);
            params.options = CodecOptions::new()
                .set("top_down", opts.top_down.to_string())
                .set("minimal_palette", opts.minimal_palette.to_string())
                .set("rle", opts.rle.to_string());
            let mut enc = make_encoder(&params).unwrap();
            let got = enc
                .send_frame(&Frame::Video(frame.clone()))
                .and_then(|()| enc.receive_packet())
                .map(|p| p.data);
            let report = encode_with_report(image, opts);
            if let (Ok(a), Ok((b, _))) = (&got, &report) {
                assert!(a == b, "{name} / {}: registry encoder", case.label);
            }
            absorb_core(&mut digest, &case.label, &got);
        }
        let via = encode_bmp_videoframe(&frame, pixel_format, image.width, image.height);
        absorb_core(&mut digest, "encode_bmp_videoframe", &via);
        for doubled in [false, true] {
            let via =
                encode_dib_videoframe(&frame, pixel_format, image.width, image.height, doubled);
            absorb_core(
                &mut digest,
                &format!("encode_dib_videoframe {doubled}"),
                &via,
            );
        }
        Some(digest.0)
    }

    #[test]
    fn registry_paths_are_byte_identical_to_v0_1_8() {
        let mut actual = Vec::new();
        let synthetic = FORMATS.into_iter().flat_map(|format| {
            images(format)
                .into_iter()
                .map(move |(label, image)| (format!("{format:?} {label}"), image))
        });
        for (name, image) in synthetic.chain(corpus()) {
            if let Some(d) = digest(&name, &image) {
                actual.push((name, d));
            }
        }
        compare("GOLDEN_REGISTRY", &actual, GOLDEN_REGISTRY);
    }

    /// Registry digests recorded from the v0.1.8 encoder (commit
    /// `103d759`).
    const GOLDEN_REGISTRY: &[(&str, u64)] = &[
        ("Rgba 1x1", 0xd200aba4fe91a50c),
        ("Rgba 5x3", 0x18ab43bc4b40c37a),
        ("Rgba 13x4+pad3", 0xb01bed5a5a295556),
        ("Rgba 32x8 runs", 0x84d589ad203a9300),
        ("Rgba 31x6+pad2 runs", 0xd599d15c3e2e56bb),
        ("Rgba 40x5 mixed", 0x15d24cff1eee0111),
        ("Rgba 256x2 constant", 0x9920d6d36f0e96c7),
        ("Rgba 600x3+pad1 constant", 0x443e729e87130b05),
        ("Rgba truncated", 0x89dca07dd944e8c7),
        ("Rgba no planes", 0x429947f84390cf31),
        ("Rgba short stride", 0x5db10cd4d9c0b2ff),
        ("Rgba zero width", 0x945ac977e8bc5cd8),
        ("Rgba zero height", 0x945ac977e8bc5cd8),
        ("Bgra 1x1", 0x0ad27da10e53873c),
        ("Bgra 5x3", 0xbffe6049fb5a170a),
        ("Bgra 13x4+pad3", 0x4303dbaa66c3a662),
        ("Bgra 32x8 runs", 0xa7d918d126318d80),
        ("Bgra 31x6+pad2 runs", 0x44baca8cf851c9b3),
        ("Bgra 40x5 mixed", 0x31b5016ac107f5ed),
        ("Bgra 256x2 constant", 0x69a9979d39d826c7),
        ("Bgra 600x3+pad1 constant", 0x69cde3dd9cfc7505),
        ("Bgra truncated", 0x89dca07dd944e8c7),
        ("Bgra no planes", 0x9d4381d169023911),
        ("Bgra short stride", 0x6fdc7af1fd1604bf),
        ("Bgra zero width", 0x945ac977e8bc5cd8),
        ("Bgra zero height", 0x945ac977e8bc5cd8),
        ("Rgb24 1x1", 0x8c0ffc135b91e17b),
        ("Rgb24 5x3", 0x9a96ed01cb794c70),
        ("Rgb24 13x4+pad3", 0xfa359ab0f035bf18),
        ("Rgb24 32x8 runs", 0x9bf090d5d3ee3080),
        ("Rgb24 31x6+pad2 runs", 0x02f616f4e05675dc),
        ("Rgb24 40x5 mixed", 0x43a1da7a0dc26b2a),
        ("Rgb24 256x2 constant", 0xb15c6b1329d06377),
        ("Rgb24 600x3+pad1 constant", 0xc6916a974e8b7785),
        ("Rgb24 truncated", 0xb848f20a829a2438),
        ("Rgb24 no planes", 0x7d68c846fda24f40),
        ("Rgb24 short stride", 0x4fb279888839ded7),
        ("Rgb24 zero width", 0x945ac977e8bc5cd8),
        ("Rgb24 zero height", 0x945ac977e8bc5cd8),
        ("Bgr24 1x1", 0x8283c53565bf12ab),
        ("Bgr24 5x3", 0x170aea3452c645f8),
        ("Bgr24 13x4+pad3", 0x9e941f105341f544),
        ("Bgr24 32x8 runs", 0xdc654b50a8c8cc00),
        ("Bgr24 31x6+pad2 runs", 0x8acd16153652dfec),
        ("Bgr24 40x5 mixed", 0x0c2d94dbffc6395a),
        ("Bgr24 256x2 constant", 0x0f8843caaa88eb77),
        ("Bgr24 600x3+pad1 constant", 0x9cab671ea4f18745),
        ("Bgr24 truncated", 0xb848f20a829a2438),
        ("Bgr24 no planes", 0x7d7345a2e0fcb000),
        ("Bgr24 short stride", 0xc2f54624c8cf07b7),
        ("Bgr24 zero width", 0x945ac977e8bc5cd8),
        ("Bgr24 zero height", 0x945ac977e8bc5cd8),
        ("Pal8 1x1", 0x6cbc7ce1ab33f791),
        ("Pal8 5x3", 0x13d9717c5bc59d78),
        ("Pal8 13x4+pad3", 0x55b55da3d5f1e1b9),
        ("Pal8 32x8 runs", 0x44897b308a8ac7ba),
        ("Pal8 31x6+pad2 runs", 0xe9f0d845985fc2a3),
        ("Pal8 40x5 mixed", 0xc83a4d64bfdf4d2a),
        ("Pal8 256x2 constant", 0xfa39b36bf54610fd),
        ("Pal8 600x3+pad1 constant", 0xbf1b38458f833b51),
        ("Pal8 empty palette", 0x48605f580b6abec4),
        ("Pal8 300-entry palette", 0x49ef58879a7f7011),
        ("Pal8 truncated", 0xacd077617f37ec63),
        ("Pal8 no planes", 0x7fa937c612ed1265),
        ("Pal8 short stride", 0x1fbfbca21ca40bf9),
        ("Pal8 zero width", 0x945ac977e8bc5cd8),
        ("Pal8 zero height", 0x945ac977e8bc5cd8),
        ("Pal8 no palette", 0x48605f580b6abec4),
        ("corpus indexed1_8x2.bmp", 0x15edecb9ee9e5cf1),
        ("corpus indexed4_8x6.bmp", 0x940e5d22e7068a20),
        ("corpus indexed8_8x6.bmp", 0x6b13c05ed1708e78),
        ("corpus minpal8_4x4.bmp", 0x3d2a7c51863bb6ab),
        ("corpus rgb24_7x5.bmp", 0x53c1a7fcb571d4b0),
        ("corpus rgba32_8x6.bmp", 0x95fbff108e798bd3),
        ("corpus rle4_32x8.bmp", 0x02d915ec34b88a22),
        ("corpus rle8_32x8.bmp", 0x6d3a55d7a39cac9a),
        ("corpus topdown_rgba_4x4.bmp", 0x0a5ea1600db7d024),
    ];
}

/// Digests recorded from the v0.1.8 encoder (commit `103d759`).
const GOLDEN: &[(&str, u64)] = &[
    ("Rgba 1x1", 0xd50c846f75bd3d3b),
    ("Rgba 5x3", 0xd8d70fe9315e389d),
    ("Rgba 13x4+pad3", 0xb7e97161c965a92f),
    ("Rgba 32x8 runs", 0x86728420df535e25),
    ("Rgba 31x6+pad2 runs", 0xe609c781a74e6cd7),
    ("Rgba 40x5 mixed", 0xf49e17ccdd7a8ee7),
    ("Rgba 256x2 constant", 0x22940f840de2fa99),
    ("Rgba 600x3+pad1 constant", 0x6219e5be56670d6d),
    ("Rgba truncated", 0xdb70c938a3c5a37f),
    ("Rgba no planes", 0xb7f348af8d2ccbe9),
    ("Rgba short stride", 0x79ce4c9e4e789f12),
    ("Rgba zero width", 0xde274108f894edb1),
    ("Rgba zero height", 0xde274108f894edb1),
    ("Rgba dib", 0x41fbf09870052acf),
    ("Bgra 1x1", 0x19a606d1f2a6f1bf),
    ("Bgra 5x3", 0x9380756d5d061c73),
    ("Bgra 13x4+pad3", 0x60e61de0f1b2ba27),
    ("Bgra 32x8 runs", 0xecd068b760ac0c05),
    ("Bgra 31x6+pad2 runs", 0x249369f8e1547def),
    ("Bgra 40x5 mixed", 0x8fb75833b105edd3),
    ("Bgra 256x2 constant", 0xa72b0ee8b17f9ecb),
    ("Bgra 600x3+pad1 constant", 0xa0b8e290856ba463),
    ("Bgra truncated", 0x113a2f4b71311bf7),
    ("Bgra no planes", 0xb7f348af8d2ccbe9),
    ("Bgra short stride", 0x60211e683e004b1c),
    ("Bgra zero width", 0xf3c60c4873a281e3),
    ("Bgra zero height", 0xf3c60c4873a281e3),
    ("Bgra dib", 0xafb2f1c52fae2491),
    ("Rgb24 1x1", 0x98d3a1031bc1ee57),
    ("Rgb24 5x3", 0x483e6ad819b20575),
    ("Rgb24 13x4+pad3", 0x0399135843410817),
    ("Rgb24 32x8 runs", 0x7a2c625f8e59ce93),
    ("Rgb24 31x6+pad2 runs", 0xa52e4e2afd5761eb),
    ("Rgb24 40x5 mixed", 0x36d3b0b9672b9d5d),
    ("Rgb24 256x2 constant", 0x7bccba83345dc1d3),
    ("Rgb24 600x3+pad1 constant", 0x66eb1de5e5bc4a77),
    ("Rgb24 truncated", 0x0fdc90c56debebd3),
    ("Rgb24 no planes", 0xb7f348af8d2ccbe9),
    ("Rgb24 short stride", 0x0aa3ad93978b0288),
    ("Rgb24 zero width", 0xde274108f894edb1),
    ("Rgb24 zero height", 0xde274108f894edb1),
    ("Rgb24 dib", 0x97560a1e04d9a816),
    ("Bgr24 1x1", 0x889556e0a45b880b),
    ("Bgr24 5x3", 0x7709fe8399e5d5cd),
    ("Bgr24 13x4+pad3", 0xe3095ede9ac0c771),
    ("Bgr24 32x8 runs", 0x1e7c127f1ee921d5),
    ("Bgr24 31x6+pad2 runs", 0x43f1764fe10799b3),
    ("Bgr24 40x5 mixed", 0xa2230b4bf31e072d),
    ("Bgr24 256x2 constant", 0x2ab804527a5ea625),
    ("Bgr24 600x3+pad1 constant", 0x24b303d6d425981f),
    ("Bgr24 truncated", 0x7cd5cd71ff7082cf),
    ("Bgr24 no planes", 0xb7f348af8d2ccbe9),
    ("Bgr24 short stride", 0xfd6cde19c4918b86),
    ("Bgr24 zero width", 0xf3c60c4873a281e3),
    ("Bgr24 zero height", 0xf3c60c4873a281e3),
    ("Bgr24 dib", 0x109abf63c7f5d81a),
    ("Rgb555 1x1", 0x24966ecf8a8bd795),
    ("Rgb555 5x3", 0x13dcebff88aa77ef),
    ("Rgb555 13x4+pad3", 0x6ffd32f643ccdecd),
    ("Rgb555 32x8 runs", 0x3ca757612477e751),
    ("Rgb555 31x6+pad2 runs", 0x95e4bc9fdf3a5c9f),
    ("Rgb555 40x5 mixed", 0x4d2deb8a95291ce7),
    ("Rgb555 256x2 constant", 0x7fdcdbae744d6b73),
    ("Rgb555 600x3+pad1 constant", 0xb36733f9856c9b5f),
    ("Rgb555 truncated", 0x61f203daf8351937),
    ("Rgb555 no planes", 0xb7f348af8d2ccbe9),
    ("Rgb555 short stride", 0xb8e35e2917316862),
    ("Rgb555 zero width", 0xf3c60c4873a281e3),
    ("Rgb555 zero height", 0xf3c60c4873a281e3),
    ("Rgb555 dib", 0xe23769d03853b8d0),
    ("Rgb565 1x1", 0x5e4cdda2a850310b),
    ("Rgb565 5x3", 0x533f60dcac81d641),
    ("Rgb565 13x4+pad3", 0x07900c486e283647),
    ("Rgb565 32x8 runs", 0x8ce321d07f3d63e3),
    ("Rgb565 31x6+pad2 runs", 0x42222d7e36d6d413),
    ("Rgb565 40x5 mixed", 0x5e3a6f1aed613c4f),
    ("Rgb565 256x2 constant", 0x9d7db28db149af95),
    ("Rgb565 600x3+pad1 constant", 0x3b6f47c9632f1d75),
    ("Rgb565 truncated", 0xb22534726f7317fd),
    ("Rgb565 no planes", 0xb7f348af8d2ccbe9),
    ("Rgb565 short stride", 0xeae12e0d6d5a385a),
    ("Rgb565 zero width", 0xf3c60c4873a281e3),
    ("Rgb565 zero height", 0xf3c60c4873a281e3),
    ("Rgb565 dib", 0x905b407ae8682adb),
    ("Pal8 1x1", 0xdfff4ac396d0b075),
    ("Pal8 5x3", 0x6908f1a167fd6a81),
    ("Pal8 13x4+pad3", 0x1421c14768ba2cb3),
    ("Pal8 32x8 runs", 0xfc2546987179b8db),
    ("Pal8 31x6+pad2 runs", 0x9b99c4559a13dd4e),
    ("Pal8 40x5 mixed", 0xa415740fe96b989c),
    ("Pal8 256x2 constant", 0xa178b3822c92f245),
    ("Pal8 600x3+pad1 constant", 0x5b8d30fc8d91d56e),
    ("Pal8 empty palette", 0x76f888a433844267),
    ("Pal8 300-entry palette", 0x668b840310030ee5),
    ("Pal8 truncated", 0x6b4e9fabe2c8b1d7),
    ("Pal8 no planes", 0xb7f348af8d2ccbe9),
    ("Pal8 short stride", 0x66b7319e68bf33f6),
    ("Pal8 zero width", 0xf3c60c4873a281e3),
    ("Pal8 zero height", 0xf3c60c4873a281e3),
    ("Pal8 no palette", 0x731db5455916be2f),
    ("Pal8 dib", 0x0b5215e206f635f5),
    ("Indexed4 1x1", 0x392f672a286f9f9d),
    ("Indexed4 5x3", 0x115ac4afc5cb71f9),
    ("Indexed4 13x4+pad3", 0x79e2be705a3ae60d),
    ("Indexed4 32x8 runs", 0x1bb99a58cd6f0afe),
    ("Indexed4 31x6+pad2 runs", 0x5c67b7afb85aa9a5),
    ("Indexed4 40x5 mixed", 0x360cfaf773b7726f),
    ("Indexed4 256x2 constant", 0x9e0961d7c60e1ad9),
    ("Indexed4 600x3+pad1 constant", 0x8cfa06b289734a03),
    ("Indexed4 empty palette", 0xbcb349618d5e17d3),
    ("Indexed4 300-entry palette", 0x57d3649c18e6fabf),
    ("Indexed4 truncated", 0x3345fcbe01eaa957),
    ("Indexed4 no planes", 0xb7f348af8d2ccbe9),
    ("Indexed4 short stride", 0x4e3b8b1bf4643f14),
    ("Indexed4 zero width", 0xf3c60c4873a281e3),
    ("Indexed4 zero height", 0xf3c60c4873a281e3),
    ("Indexed4 no palette", 0xfac46aeb420cb23b),
    ("Indexed4 dib", 0x46f3246d1acd6d2d),
    ("Indexed2 1x1", 0xe827f0d1681b6b3d),
    ("Indexed2 5x3", 0x216b119e9af0d56b),
    ("Indexed2 13x4+pad3", 0xca84e6b977434945),
    ("Indexed2 32x8 runs", 0x9304fa1ac0f99695),
    ("Indexed2 31x6+pad2 runs", 0x460d515182bf3f7b),
    ("Indexed2 40x5 mixed", 0x46c6b6eeca782ac5),
    ("Indexed2 256x2 constant", 0xf4325a8681eeff7f),
    ("Indexed2 600x3+pad1 constant", 0x91f1a0c9d044db71),
    ("Indexed2 empty palette", 0x8b2007fcd98eb669),
    ("Indexed2 300-entry palette", 0x339d2496c4198bb7),
    ("Indexed2 truncated", 0xf34a5f8dfb6a4187),
    ("Indexed2 no planes", 0xb7f348af8d2ccbe9),
    ("Indexed2 short stride", 0x0e3063b1957fbd7a),
    ("Indexed2 zero width", 0xf3c60c4873a281e3),
    ("Indexed2 zero height", 0xf3c60c4873a281e3),
    ("Indexed2 no palette", 0x0a526e5e58b49423),
    ("Indexed2 dib", 0xda4aa5233a563681),
    ("Indexed1 1x1", 0xb2c31b5293bf1089),
    ("Indexed1 5x3", 0x1e7cfabee141bd81),
    ("Indexed1 13x4+pad3", 0x95e50df27d7db7b5),
    ("Indexed1 32x8 runs", 0xac4255704b910195),
    ("Indexed1 31x6+pad2 runs", 0x1fcae56a047ebcb5),
    ("Indexed1 40x5 mixed", 0xa6e0567d940d1439),
    ("Indexed1 256x2 constant", 0xe075094be7fa7ba7),
    ("Indexed1 600x3+pad1 constant", 0x8e04878e7aec2e39),
    ("Indexed1 empty palette", 0xdd90cfc1208e7d15),
    ("Indexed1 300-entry palette", 0xf95e6373e82564f1),
    ("Indexed1 truncated", 0x02093c50807f60e7),
    ("Indexed1 no planes", 0xb7f348af8d2ccbe9),
    ("Indexed1 short stride", 0x5c864a179142c742),
    ("Indexed1 zero width", 0xf3c60c4873a281e3),
    ("Indexed1 zero height", 0xf3c60c4873a281e3),
    ("Indexed1 no palette", 0xaccee5c7903bf565),
    ("Indexed1 dib", 0x3280bca0f74b0cb2),
    ("corpus indexed1_8x2.bmp", 0x32c49e25033272ca),
    ("corpus indexed4_8x6.bmp", 0x5362a19095c18614),
    ("corpus indexed8_8x6.bmp", 0x0ff6f15d1ce739cf),
    ("corpus minpal8_4x4.bmp", 0x1c6570012092b569),
    ("corpus rgb24_7x5.bmp", 0x534013710f6fe154),
    ("corpus rgb565_7x5.bmp", 0xfaf50ca285b001f9),
    ("corpus rgba32_8x6.bmp", 0x85f9bfcae158581f),
    ("corpus rle4_32x8.bmp", 0x163e3e844a73dc97),
    ("corpus rle8_32x8.bmp", 0x3584b351ea423454),
    ("corpus topdown_rgba_4x4.bmp", 0x63f32937157ffddd),
];

/// Digests of the large cases, recorded from the v0.1.8 encoder (commit
/// `103d759`).
const GOLDEN_LARGE: &[(&str, u64)] = &[
    ("Rgba 300x300 noise", 0xe3a08bb5657c7583),
    ("Bgra 300x300+pad1 noise top_down", 0xe903689659bedcfe),
    ("Rgb24 301x200 mixed embed_icc", 0x1392620d4687cc03),
    ("Rgb565 333x200 noise calibrated", 0x6b3c794e057c6ee3),
    ("Bgr24 257x300 noise bitfields BGRA8888", 0x840377f68026461d),
    ("Pal8 1024x512 mixed (RLE8 wins)", 0xff7e18f66a34a6f0),
    ("Pal8 1024x300 noise (RLE8 loses)", 0x0acea0c38066927e),
    (
        "Indexed4 1023x512+pad3 mixed (RLE4 wins)",
        0x853e2a87c1c2c31a,
    ),
    ("Indexed1 2000x300 mixed linked_icc", 0x092c2a98157afd62),
];
