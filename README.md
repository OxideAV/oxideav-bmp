# oxideav-bmp

[![CI](https://github.com/OxideAV/oxideav-bmp/actions/workflows/ci.yml/badge.svg)](https://github.com/OxideAV/oxideav-bmp/actions/workflows/ci.yml) [![crates.io](https://img.shields.io/crates/v/oxideav-bmp.svg)](https://crates.io/crates/oxideav-bmp) [![docs.rs](https://docs.rs/oxideav-bmp/badge.svg)](https://docs.rs/oxideav-bmp) [![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

Pure-Rust BMP (Windows bitmap) decoder, encoder and container for the
[`oxideav`](https://github.com/OxideAV/oxideav) framework, following
the OxideAV **image-crate API contract** (`IMAGE_CRATE_API.md` in the
workspace). Also exposes the headerless **DIB** helpers used by `.ico`
/ `.cur` sub-images.

## Standalone use

```toml
[dependencies]
oxideav-bmp = { version = "0.1", default-features = false }
```

```rust
let bytes = std::fs::read("in.bmp")?;
if oxideav_bmp::probe(&bytes) {
    let info = oxideav_bmp::info(&bytes)?;         // header only: width, height, format, colour
    let img  = oxideav_bmp::decode(&bytes)?;       // BmpImage, native layout (Pal8 / Rgb555 / Bgr24 / Bgra …)
    let rgba: Vec<u8> = img.to_rgba8();            // tightly packed RGBA, 4 × width bytes per row
    let (w, h) = (img.width(), img.height());

    let opts = oxideav_bmp::EncodeOptions::default().with_top_down(true);
    let out: Vec<u8> = oxideav_bmp::encode_rgba8(w, h, &rgba, &opts)?;   // 32-bit BGRA BI_RGB
    std::fs::write("out.bmp", out)?;
    let _ = info;
}
# Ok::<(), Box<dyn std::error::Error>>(())
```

Root vocabulary (identical across every `oxideav-*` image crate):
`probe`, `info -> ImageInfo`, `decode -> BmpImage`,
`decode_with(&DecodeOptions)`, `decode_rgb8 -> RgbImage`,
`decode_rgba8 -> RgbaImage`, `decode_from<R: Read>`,
`encode(&BmpImage, &EncodeOptions) -> Vec<u8>`, `encode_rgb8`,
`encode_rgba8`, `encode_to<W: Write>`; types `BmpImage { width, height,
format: PixelFormat, planes: Vec<Plane>, color: ColorInfo, metadata:
Metadata, palette: Option<Palette> }`, `RgbImage` / `RgbaImage { width,
height, data }`, `Plane { stride, data }`, `ColorInfo { range,
primaries, transfer, matrix }`, `Metadata { icc, exif, xmp, gamma }`,
`Palette { entries: Vec<[u8; 4]> }`, `ImageInfo`, `DecodeOptions`,
`EncodeOptions`, `PixelFormat` (= `BmpPixelFormat`), `Error`
(= `BmpError`: `InvalidData`, `Unsupported`, `LimitExceeded`, `Io`).
BMP has one image per file, so there is no `decode_all`.

BMP-specific depth on top of the contract: `encode_with_report` (also
tells you which on-disk variant was written — RLE or raw, bitfields,
…), `decode_dib` / `decode_dib_with` / `encode_dib` (headerless DIBs for
`.ico` / `.cur`), `BmpMetadata::from_bmp` / `from_dib` (every V3 / V4 /
V5 header field, header-only), `BmpBitfields` (mask presets), the typed
`BitmapFileHeader` / `BitmapInfoHeader` / `DibHeader` views and the
`BI_*` / `LCS_*` constants.

### Migrating from 0.1.x

The pre-contract names remain for one release as `#[deprecated]`
wrappers: `decode_bmp` (now `decode(..)` + `to_rgba8()` — the old
function always returned `Rgba`, `decode` returns the native layout),
`decode_bmp_with_metadata` / `decode_dib_with_metadata` (now `decode` /
`decode_dib` plus `BmpMetadata::from_bmp` / `from_dib`), `encode_bmp` /
`encode_bmp_with_options` / `encode_bmp_plane*` (now `encode` /
`encode_with_report`), `encode_bmp_bitfields` / `encode_bmp_with_icc_profile`
/ `encode_bmp_with_linked_icc_profile` / `encode_bmp_with_calibrated_rgb`
(now `EncodeOptions` fields: `bitfields`, `embed_icc` + `metadata.icc`,
`linked_icc`, `calibrated_rgb`), `BmpEncodeOptions` (now `EncodeOptions`,
`#[non_exhaustive]`, built with `with_*`), `BmpPlane` (now `Plane`),
`BmpPalette` (`[u8; 3]` entries; now `Palette` with `[u8; 4]`),
`BmpPixelFormat::Indexed8` (now `Pal8`), and the `*_videoframe`
wrappers. `BmpImage` lost its `pts` field (it belongs to the framework
frame) and `pixel_format` is now `format`.

## Framework use

The default `registry` feature pulls in `oxideav-core`:

```rust
let mut ctx = oxideav_core::RuntimeContext::new();
oxideav_bmp::register(&mut ctx);          // codec "bmp" + container "bmp" (.bmp / .dib)
```

`register_codecs(&mut CodecRegistry)` / `register_containers(&mut
ContainerRegistry)` install the two halves separately; `make_decoder` /
`make_encoder` are the factories. The framework decoder hands the
pipeline the **native layout** (`registry_pixel_format`): `Pal8` with
the palette side-channel for every indexed depth (the 4- / 2- / 1-bit
layouts are already one index per byte), `Bgr24` / `Bgra` / `Rgb24` /
`Rgba` as stored, and `Rgb24` for the 16-bit `Rgb555` / `Rgb565`
layouts (core has no 16-bit RGB name; 5 / 6-bit samples bit-replicated);
the colour-signal side-channel rides along when the file carries a
colour space. The demuxer declares the same layout on its stream.
Nothing is pre-converted to `Rgba` — use `oxideav-pixfmt`, or
`decode_rgba8` standalone. The framework encoder
accepts `Rgba`, `Rgb24`, `Bgra`, `Bgr24` and `Pal8` (palette from the
frame's palette side-channel) and reads `EncodeOptions` from
`CodecParameters::options` (`top_down`, `minimal_palette`, `rle`,
`embed_icc`). `From<BmpImage> for VideoFrame`,
`BmpImage::from_video_frame(&VideoFrame, &CodecParameters)` and
`TryFrom<(&VideoFrame, &CodecParameters)>` bridge the two worlds
(`Pal8` carries its palette as the frame's palette side-channel, a
signalled colour space as the colour-signal side-channel).

## Supported layouts

### Decode (`decode` returns the native layout)

| Bit depth | Compression | Native `PixelFormat` | `to_rgba8` |
| --- | --- | --- | --- |
| 1 / 2 / 4 | `BI_RGB` | `Pal8` — indices unpacked to one byte per pixel, colour table in `palette` | palette lookup, α = 255 |
| 4 | `BI_RLE4` | `Pal8` (delta + absolute mode; skipped cells → index 0) | palette lookup |
| 8 | `BI_RGB`, `BI_RLE8` | `Pal8` | palette lookup |
| 16 | `BI_RGB`, or `BI_BITFIELDS` with the 5-5-5 masks | `Rgb555` (packed `u16` words, verbatim) | 5-bit → 8-bit by bit replication |
| 16 | `BI_BITFIELDS` with the 5-6-5 masks | `Rgb565` (verbatim) | 5/6-bit → 8-bit by bit replication |
| 16 | any other mask set, `BI_ALPHABITFIELDS` | `Rgba` (every channel expanded) | copy |
| 24 | `BI_RGB` | `Bgr24` | swizzle, α = 255 |
| 32 | `BI_RGB` (V3) | `Bgra` — fourth byte kept as stored | swizzle |
| 32 | `BI_RGB` (V4 / V5) | `Bgra` — alpha from the in-header alpha mask, `0xFF` if the mask is zero | swizzle |
| 32 | `BI_BITFIELDS` / `BI_ALPHABITFIELDS`, byte-aligned B,G,R(,A) | `Bgra` | swizzle |
| 32 | `BI_BITFIELDS` / `BI_ALPHABITFIELDS`, byte-aligned R,G,B(,A) | `Rgba` | copy |
| 32 | any other mask set | `Rgba` (every channel expanded through the masks) | copy |

Rows are always delivered top-down and tightly packed (no DWORD
padding); bottom-up files are flipped while decoding. Every header
generation is accepted: OS/2 1.x `BITMAPCOREHEADER` (12 B, 3-byte
`RGBTRIPLE` palette), truncated and full OS/2 2.x `OS22XBITMAPHEADER`
(16…64 B), `BITMAPINFOHEADER` (V3, 40 B), Adobe `BITMAPV2INFOHEADER` /
`BITMAPV3INFOHEADER` (52 / 56 B), `BITMAPV4HEADER` (108 B),
`BITMAPV5HEADER` (124 B). `BI_JPEG`, `BI_PNG` and the CMYK compressions
are rejected with `Error::InvalidData`; the OS/2 `BA` / `CI` / `CP` /
`IC` / `PT` container wrappers with `Error::Unsupported`.

### Encode (`encode` writes the image's layout)

| Input `PixelFormat` | On disk | Compression | Header |
| --- | --- | --- | --- |
| `Rgba`, `Bgra` (4 B/px) | 32-bit BGRA, alpha in the fourth byte | `BI_RGB` | V3 |
| `Rgb24`, `Bgr24` (3 B/px) | 24-bit BGR | `BI_RGB` | V3 |
| `Rgb555` (2 B/px) | 16-bit 5-5-5 | `BI_RGB` | V3 |
| `Rgb565` (2 B/px) | 16-bit 5-6-5, masks in the header | `BI_BITFIELDS` | V4 (`LCS_sRGB`) |
| `Pal8` (1 B/px) | 8-bit indexed | `BI_RGB` or `BI_RLE8` (whichever is smaller) | V3 |
| `Indexed4` (1 B/px, values 0..=15) | 4-bit indexed | `BI_RGB` or `BI_RLE4` | V3 |
| `Indexed2` (1 B/px, values 0..=3) | 2-bit indexed (Windows CE) | `BI_RGB` | V3 |
| `Indexed1` (1 B/px, values 0 / 1) | 1-bit indexed | `BI_RGB` | V3 |

Every layout `BmpImage` can express is writable, so `encode` never
converts silently and `Error::Unsupported` is reserved for mutually
exclusive header options (see below) and for `bitfields` on an indexed
image. `encode_rgb8` writes 24-bit `BI_RGB`, `encode_rgba8` 32-bit
`BI_RGB` with the alpha byte stored in the fourth byte (a V3 header has
no way to declare it; `decode` hands it back verbatim, readers that
ignore the byte see the colours). The indexed layouts need
`BmpImage::palette`. **Lossless round trip:** `decode(encode(img))`
reproduces the planes, palette and metadata of every native layout
(`Bgra`, `Bgr24`, `Rgb555`, `Rgb565`, `Pal8`); `Rgba` / `Rgb24` come
back as `Bgra` / `Bgr24` and `Indexed4` / `Indexed2` / `Indexed1` as
`Pal8`, pixel values intact — pinned by `tests/contract.rs` and the
`encode_roundtrip` fuzz target.

## Options

`DecodeOptions` (`#[non_exhaustive]`, `Default`, `with_*`):
`max_width` / `max_height: Option<u32>`, `max_pixels` /
`max_bytes: Option<u64>` (`None` = unlimited; default: no dimension cap,
1 GiB of decoded plane), `strict: bool` (default `false`; `true` rejects
a `bfOffBits` that points inside the header / colour table instead of
recovering the canonical offset, and non-zero `BITMAPFILEHEADER`
reserved words).

`EncodeOptions` (`#[non_exhaustive]`, `Default`, `with_*`):

| Field | Default | Effect |
| --- | --- | --- |
| `top_down: bool` | `false` | rows stored top-down, `biHeight` negative; disables RLE (illegal for top-down) |
| `minimal_palette: bool` | `false` | write only the palette's entries, recording the count in `biClrUsed`, instead of a full `2^bpp` table |
| `rle: bool` | `true` | allow `BI_RLE8` / `BI_RLE4` for `Pal8` / `Indexed4` when smaller than raw |
| `bitfields: Option<BmpBitfields>` | `None` | explicit-mask `BI_BITFIELDS` / `BI_ALPHABITFIELDS` (V3 + 12 / 16-byte mask tail) for `Rgba` / `Rgb24` / `Bgra` / `Bgr24` input; presets `RGB565`, `RGB555`, `ARGB1555`, `BGRA8888`, `BGRX8888` |
| `embed_icc: bool` | `true` | write a V5 `PROFILE_EMBEDDED` header carrying `metadata.icc` when the image has one |
| `linked_icc: Option<Vec<u8>>` | `None` | V5 `PROFILE_LINKED` header with this path bytestring in the profile slot |
| `rendering_intent: u32` | `LCS_GM_IMAGES` | `bV5Intent` for the two V5 modes |
| `calibrated_rgb: Option<CalibratedRgb>` | `None` | V4 `LCS_CALIBRATED_RGB` header with the given CIE endpoints and Q16.16 gamma triple |

A file has one DIB header, so `bitfields`, `calibrated_rgb` and the V5
profile modes are mutually exclusive — asking for two is
`Error::Unsupported`. `encode_with_report` returns the
`EncodedBmpFormat` actually written.

## Metadata and colour

`BmpImage::color` is filled from the header: V4 / V5 `LCS_sRGB` and
`LCS_WINDOWS_COLOR_SPACE` ("the system default color space, sRGB")
decode to `ColorInfo::srgb()` (full range, H.273 primaries 1, transfer
13, matrix 0); V3 / OS/2 headers, `LCS_CALIBRATED_RGB` and the two ICC
modes decode to `ColorInfo::bmp_default()` — full-range device RGB
with unspecified primaries and transfer. `BmpImage::metadata.icc` holds
the embedded ICC profile of a V5 `PROFILE_EMBEDDED` file (when the
declared slot fits in the buffer); `exif` and `xmp` are always `None`
(BMP has no such slots) and `gamma` is `None` — the V4 per-channel
Q16.16 "tone response curve" triple has no documented relation to the
single file-gamma convention the contract field uses, so it stays in
`BmpMetadata::gamma_rgb`. `ImageInfo::has_icc` reports a declared
embedded profile of non-zero size. The full header record (colour-space
tag, endpoints, gamma triple, rendering intent, profile offset / size,
linked path bytes, pixels-per-metre, colour counts, OS/2 2.x fields) is
`BmpMetadata::from_bmp(&bytes)` / `BmpMetadata::from_dib(&dib)`, both
header-only.

## Limits

`probe` is allocation-free and total. `info` and `BmpMetadata::from_*`
read headers only. `decode_with` checks `DecodeOptions` against the
header before the colour table is read or any pixel buffer exists; a
hostile header fails with `Error::LimitExceeded`. Independently of the
options, the uncompressed path never allocates more than the input can
back (`stride × height` is bounds-checked against the buffer first) and
the RLE path caps the output grid at `255 × stream length` pixels, so a
40-byte header claiming 2³¹ × 2³¹ is rejected, not allocated. Eight
cargo-fuzz targets (`decode`, `rle_stream`, `header_forge`, `metadata`,
`encode_roundtrip`, `icc_roundtrip`, `dib_roundtrip`,
`bitfields_roundtrip`) cover `probe` / `info` / `decode` / `decode_with`
/ `decode_rgb8` / `decode_rgba8` / `decode_dib` and every encode option;
`tests/contract.rs` pins the contract behaviour and `tests/*.rs` replay
the curated corpora and adversarial suites on every `cargo test`.

---

# Format specifics

The sections below are the format-level detail behind the tables above.

## Decode details

### RLE skipped-pixel + orientation semantics

An RLE bitmap is an *indexed* image, so every cell the `BI_RLE8` /
`BI_RLE4` stream never writes — the ones a `delta` jumps over, the tail
of a short row past an end-of-line escape, and everything past an early
end-of-bitmap — resolves to **colour index 0**, the first colour-table
entry (the canonical Windows background), not transparent black. A
`magick`-CLI black-box test corroborates the fill against an external
decoder. RLE is also strictly bottom-up: the *Bitmap Header Types*
remarks require a positive `biHeight` for any compressed format, so a
top-down (negative `biHeight`) RLE bitmap is rejected at the decode
boundary rather than silently decoded mirrored.

### CMYK family (`BI_CMYK` / `BI_CMYKRLE8` / `BI_CMYKRLE4`)

Compression values `11` / `12` / `13` are the "only Windows Metafile
CMYK" family: an uncompressed CMYK pixel array (`BI_CMYK`, 11) and the
same CMYK samples carried in the `BI_RLE8` (`BI_CMYKRLE8`, 12) and
`BI_RLE4` (`BI_CMYKRLE4`, 13) run-length framings. The CMYK channel
layout and the CMYK→RGB conversion are defined by the Windows Metafile
(WMF) specification, not the BMP file-format material this crate works
from, so these three are **recognised by name and rejected at the decode
boundary** with a distinct error (`BMP: CMYK (BI_CMYK) not supported`,
etc.) rather than the generic `unknown compression {n}` path — a CMYK
bitmap is reported as a known-but-unsupported format instead of looking
like a corrupt header. The `BI_CMYK` / `BI_CMYKRLE8` / `BI_CMYKRLE4`
constants are public so callers can pre-screen. Full CMYK decode is
blocked on a WMF-CMYK channel-order + conversion trace in
`docs/image/bmp/`.

### Truncated OS/2 2.x `OS22XBITMAPHEADER` (16…39 B)

The OS/2 2.x header (`BITMAPINFOHEADER2` in IBM's documentation) shares
the 40-byte `BITMAPINFOHEADER` field layout — 4-byte signed
width/height, then compression / image-size / resolution / palette
counts — and grows it by 24 trailing bytes (units / fill-direction /
halftoning / colour-encoding / app-id) for a full 64-byte form. A
writer may legally stop the header early and have every field past the
truncation point read as zero; the 16-byte form (`biSize` / width /
height / planes / bit-count only) is the canonical case, exercised by
the BMP Suite's `pal8os2v2-16.bmp`. The decoder reads each field only
when the declared `biSize` is long enough to contain it and defaults
the rest to zero. Unlike the 12-byte OS/2 1.x `BITMAPCOREHEADER`, the
truncated OS/2 2.x header uses the 4-byte signed width/height (so a
negative `biHeight` selects top-down rows) and 4-byte `RGBQUAD`
palette. A truncated header has no room for the appended bitfield-mask
block, so `BI_BITFIELDS` (the OS/2 `Huffman 1D` alias) and `BI_JPEG`
(the OS/2 `RLE-24` alias) are rejected on these sizes — only plain
`BI_RGB` / `BI_RLE8` / `BI_RLE4` streams decode.

### Full 64-byte OS/2 2.x `OS22XBITMAPHEADER` trailing fields

When the DIB header is the *full* 64-byte form, the 24 bytes that sit
past the 40-byte `BITMAPINFOHEADER` prefix carry IBM's extra
print-oriented descriptors: a resolution-units `WORD` (offset 40, only
defined value `0` = pixels per metre), a recording / fill-direction
`WORD` (offset 44, only defined value `0` = lower-left origin), a
halftoning-algorithm `WORD` (offset 46) with two `DWORD` parameters
(offsets 48 / 52), a colour-table-encoding `DWORD` (offset 56, only
defined value `0` = RGB), and an application-defined identifier `DWORD`
(offset 60). The `WORD` at offset 42 is documented padding. These are
surfaced through `BmpMetadata::os2_header2: Option<BmpOs2Header2>`,
populated only for an exactly-64-byte header (every Windows generation
and the truncated OS/2 2.x forms report `None`):

```rust
# let bytes: &[u8] = &std::fs::read("in.bmp")?;
use oxideav_bmp::{BmpMetadata, BmpOs2Halftone};
let md = BmpMetadata::from_bmp(bytes)?;
if let Some(h2) = md.os2_header2 {
    h2.units_is_pels_per_meter();   // units == 0
    h2.is_bottom_up();              // recording == 0 (lower-left origin)
    h2.color_encoding_is_rgb();     // color_encoding == 0
    match h2.halftone {
        BmpOs2Halftone::None            => {}            // 0
        BmpOs2Halftone::ErrorDiffusion  => {}            // 1: size1 = % damping
        BmpOs2Halftone::Panda           => {}            // 2: size1/size2 = pattern X/Y
        BmpOs2Halftone::SuperCircle     => {}            // 3: size1/size2 = pattern X/Y
        BmpOs2Halftone::Unknown(v)      => { let _ = v; } // verbatim passthrough
    }
    let _ = (h2.halftone_size1, h2.halftone_size2, h2.identifier);
}
# Ok::<(), Box<dyn std::error::Error>>(())
```

Every raw field is passed through verbatim so a non-standard write is
distinguishable from the documented default; the colour-space tail
stays `None` because a 64-byte header is below the 108-byte V4
threshold. Pixel decode is unchanged — the trailing block is metadata
only.

### Typed `BitmapFileHeader` view

The 14-byte `BITMAPFILEHEADER` prefix is also surfaced as a typed
struct for callers that want to inspect the file header without
running the full decode (probe / dispatcher / fuzz consumers):

```rust
# let bytes: &[u8] = &std::fs::read("in.bmp")?;
use oxideav_bmp::BitmapFileHeader;

// `parse` validates buffer length + the `0x4D42` `bfType` signature.
let h = BitmapFileHeader::parse(bytes)?;
assert!(h.has_canonical_magic());     // distinguishes "BM" from OS/2
                                      // `BA`/`CI`/`CP`/`IC`/`PT` variants
let _ = h.magic_kind();               // BmpFileMagic — the classified signature
let _ = h.file_size;                  // bfSize (may be 0 — informational)
let _ = h.pixel_offset;               // bfOffBits — start of pixel array
let _ = h.reserved_is_clean();        // bfReserved1/2 zero per the spec

// `from_bytes` is the unchecked variant (returns `None` on a short
// buffer; the magic check is skipped). Encoder consumers go the
// other way via `to_bytes()` for a deterministic 14-byte layout.
# Ok::<(), Box<dyn std::error::Error>>(())
```

`decode`, `info` and `BmpMetadata::from_bmp` all funnel the file
header parse through this struct, so the "shorter than header" and
"missing 'BM' signature" error messages come from a single source.

### OS/2 file-magic recognition

The two-byte `bfType` signature is classified by `BmpFileMagic`. Beyond
the canonical `BM`, the staged file-format reference lists five OS/2-era
container signatures: `BA` (bitmap array), `CI` (colour icon), `CP`
(colour pointer), `IC` (icon), and `PT` (pointer). None carry a bare DIB
this crate can decode — the `BA` array is a linked list of sub-bitmaps
(archive-walking is docs-blocked) and the icon / pointer wrappers carry
hotspot metadata — so each is **recognised and rejected** with a precise
`Unsupported` error naming the two-char signature (e.g. `'BA' OS/2
bitmap-array container is not supported`) rather than the generic
`InvalidData("missing 'BM'")`. A wholly unrecognised word keeps the
historical `missing 'BM'` message. The `OS2_MAGIC_BA` … `OS2_MAGIC_PT`
constants and the `BmpFileMagic` classifier are part of the public API.

### `bfOffBits` recovery (zero / implausibly-early offset)

`bfOffBits` is the spec's source of truth for where the pixel array
starts — a writer may leave a gap between the colour table and the
pixels, and the decoder honours any value that lands at or past the
canonical position. But minimal encoders frequently leave the field
unset (`bfOffBits = 0`), and a corrupt or lazy writer can point it
*inside* the DIB header or colour table; in either case the stored
offset cannot be where the pixels actually begin. The decoder now
recovers the canonical pixel-array offset — file header → DIB header →
bit-field mask block (V3 `BI_BITFIELDS` / `BI_ALPHABITFIELDS` only) →
colour table → pixels, the layout the *Bitmap Storage* material
describes — instead of reading header / palette bytes as pixels or
tripping the "pixel array truncated" check. The recovery only ever
moves the read *forward* to the earliest byte the pixels could legally
occupy, so a larger, valid `bfOffBits` (a deliberate gap) is left
untouched. `decode`, `info` and `BmpMetadata::from_bmp` share the
resolution (`DecodeOptions::strict` turns the recovery off); the
headerless `decode_dib` path always uses the canonical layout.

### Typed `BitmapInfoHeader` view + `DibHeaderKind`

The 40-byte `BITMAPINFOHEADER` that opens every V3-and-later DIB is
likewise surfaced as a typed struct — the eleven documented fields
(`header_size` / `width` / `height` / `planes` / `bit_count` /
`compression` / `image_size` / `x_pels_per_meter` / `y_pels_per_meter`
/ `clr_used` / `clr_important`) at their on-disk offsets:

```rust
# let bmp = std::fs::read("in.bmp")?;
use oxideav_bmp::{BitmapFileHeader, BitmapInfoHeader, DibHeaderKind};

// `parse` validates buffer length + the biSize discrimination:
// 12 (CORE — different WORD-based layout) and other sub-40 sizes are
// rejected; >= 40 is accepted since V2/V3/V4/V5 (and odd in-the-wild
// sizes like the OS/2 2.x 64-byte variant) all share the 40-byte
// INFO prefix.
let h = BitmapInfoHeader::parse(&bmp[BitmapFileHeader::SIZE..])?;
h.kind();              // Option<DibHeaderKind> — Info/V2Info/V3Info/V4/V5
h.is_top_down();       // negative biHeight
h.row_stride();        // documented ((w*bpp + 31) & !31) >> 3 formula
h.palette_entries();   // biClrUsed with the 0 = 2^bpp sentinel applied
h.planes_is_valid();   // biPlanes == 1 ("must be set to 1")

// `from_bytes` is the unchecked variant (None on a short buffer);
// `to_bytes()` renders the deterministic 40-byte layout back.
// `DibHeaderKind::from_size(biSize)` maps 12/40/52/56/108/124 to the
// six known header generations.
# Ok::<(), Box<dyn std::error::Error>>(())
```

`parse_dib_header` inside the decoder now reads the eleven base fields
through this struct (the extended mask / colour-space tails stay on
the wide `DibHeader`), so the field offsets live in a single place.

### `BITMAPV2INFOHEADER` (52 B) + `BITMAPV3INFOHEADER` (56 B)

V2 (52 B) and V3 (56 B) are the Adobe-published intermediate header
generations that sit between `BITMAPINFOHEADER` (40 B) and
`BITMAPV4HEADER` (108 B). V2 extends V3-INFO by 12 bytes of in-header
R/G/B bit masks (offsets 40 / 44 / 48), so a `BI_BITFIELDS` 52-byte
header carries its masks **inside** the header body — no separate
12-byte mask tail sits between the header and the pixel array. V3
(56 B) extends V2 by a 4-byte alpha mask at offset 52, matching the
slot V4 / V5 use; `BI_BITFIELDS` on a 56-byte header therefore
behaves as the four-mask R/G/B/A path that V3 `BI_ALPHABITFIELDS`
provides on the 40-byte header. The full colour-space tail (V4 adds
`bV4CSType` / endpoints / gamma at offset 56+; V5 piles
`bV5Intent` / `bV5ProfileData` / `bV5ProfileSize` / reserved on top)
is absent on both intermediate headers, so the metadata path returns
`color_space = None` / `endpoints = None` / `gamma_rgb = None` /
`rendering_intent = None` for these files while the V3-tail fields
(`pixels_per_meter_x` / `pixels_per_meter_y` / `colors_used` /
`colors_important`) stay readable since V2 inherits every byte
24..40 from `BITMAPINFOHEADER`. A zero alpha mask on V3 collapses to
opaque output (the same convention V3 `BI_ALPHABITFIELDS` and V4 / V5
use).

### V3+ device-resolution + palette-count metadata

`BmpMetadata` (`BmpMetadata::from_bmp` / `BmpMetadata::from_dib`)
also surfaces the four V3+ metadata fields
that pre-date colour management: `biXPelsPerMeter`, `biYPelsPerMeter`,
`biClrUsed`, and `biClrImportant`. The named accessors:

```rust
# let bytes: &[u8] = &std::fs::read("in.bmp")?;
let md = oxideav_bmp::BmpMetadata::from_bmp(bytes)?;
let _ = md.pixels_per_meter_x;  // Option<i32>  — None on OS/2 V1
let _ = md.pixels_per_meter_y;  // Option<i32>
let _ = md.colors_used;         // Option<u32>  — `0` = "all 2^bpp"
let _ = md.colors_important;    // Option<u32>  — `0` = "all important"
let _ = md.dpi_x();             // Option<u32>  — derived, rounded to nearest
let _ = md.dpi_y();             // Option<u32>
# Ok::<(), Box<dyn std::error::Error>>(())
```

V3 (`BITMAPINFOHEADER`, 40 B) is the first BMP header generation to
carry these fields; V4 and V5 inherit them at the same byte offsets.
The OS/2 12-byte `BITMAPCOREHEADER` pre-dates them entirely and the
accessors return `None`. For V3+ headers the raw pels-per-metre value
is passed through verbatim (so the `0` "resolution unknown" sentinel
is distinguishable from "header doesn't carry the field"); the
`dpi_x()` / `dpi_y()` helpers convert to dots-per-inch using exactly
one inch = 0.0254 m and round to the nearest integer. The helpers
return `None` for the `0` sentinel and for semantically-invalid
negative values so a misencoded file doesn't generate a nonsensical
"0 DPI" or negative DPI downstream.

### V4 / V5 colour-space metadata + embedded ICC profile

`decode` puts the contract view on the image (`BmpImage::color`,
`BmpImage::metadata.icc`); `BmpMetadata::from_bmp` /
`BmpMetadata::from_dib` parse the full header record so callers that
need the V4/V5 colour-management tail can inspect `bV4CSType`, the `CIEXYZTRIPLE`
endpoints, the `R/G/B` gamma triple, the V5 rendering intent, and the
on-disk `bV5ProfileData` / `bV5ProfileSize` fields. A V5 header that
declares `PROFILE_EMBEDDED` additionally surfaces the embedded ICC blob
as `BmpMetadata::icc_profile: Option<Vec<u8>>`; `PROFILE_LINKED`
surfaces the offset + size so callers can resolve the path themselves.

```rust
# let bytes: &[u8] = &std::fs::read("in.bmp")?;
let md = oxideav_bmp::BmpMetadata::from_bmp(bytes)?;
match md.color_space {
    Some(oxideav_bmp::BmpColorSpace::SRgb) => /* sRGB */ {}
    Some(oxideav_bmp::BmpColorSpace::ProfileEmbedded) => {
        let icc = md.icc_profile.as_deref().unwrap_or(&[]);
        // hand off `icc` to your colour-management pipeline
    }
    _ => {}
}
# Ok::<(), Box<dyn std::error::Error>>(())
```

The typed accessor `BmpMetadata::icc_profile_ref()` collapses the
PROFILE_EMBEDDED / PROFILE_LINKED / no-ICC discrimination into a
single `BmpIccProfileRef<'_>` enum so callers don't have to match on
`color_space` and then read `icc_profile` / `linked_profile_path` /
`profile_data_offset` / `profile_size` by hand:

```rust
# let bytes: &[u8] = &std::fs::read("in.bmp")?;
# let md = oxideav_bmp::BmpMetadata::from_bmp(bytes)?;
use oxideav_bmp::BmpIccProfileRef;
match md.icc_profile_ref() {
    BmpIccProfileRef::Embedded(icc)    => { /* embedded ICC bytes */ }
    BmpIccProfileRef::Linked(path)     => { /* path bytestring */ }
    BmpIccProfileRef::Declared { .. }  => { /* V5 declared a PROFILE_* but the bytes were unreachable */ }
    BmpIccProfileRef::None             => { /* V3 / V4 / V5 LCS_* — no ICC reference */ }
}
# Ok::<(), Box<dyn std::error::Error>>(())
```

`PROFILE_LINKED` bitmaps now also surface the path bytestring through
the dedicated `BmpMetadata::linked_profile_path: Option<Vec<u8>>`
field (parallel to `icc_profile` for the embedded variant). The
decoder still never opens the file the path points at — the path is
returned verbatim and its encoding (typically null-terminated ANSI on
Windows) is the caller's responsibility.


V3 / OS/2 headers report every metadata field as `None` (they pre-date
colour management). V4 fills `color_space` / `endpoints` / `gamma_rgb`;
V5 additionally fills `rendering_intent`. The pixel decode does not
depend on the declared colour space (`decode` returns the native
layout, `BmpImage::color` carries the sRGB / default signalling). A V5
header that lies about
its ICC offset / size (slice falls past EOF) leaves
`icc_profile = None` with the declared fields still populated so the
metadata path can never make decode fail on its own.

`encode` with `BmpImage::metadata.icc = Some(..)` (and the default
`EncodeOptions::embed_icc = true`) is the matching encode side: for an
`Rgba`, `Bgra`, `Rgb24`, `Bgr24`, `Rgb555`, `Rgb565`, `Pal8`,
`Indexed4`, `Indexed2` or `Indexed1` image, with
`EncodeOptions::rendering_intent` (default `LCS_GM_IMAGES`; 0 for
unspecified, or one of `LCS_GM_BUSINESS` / `LCS_GM_GRAPHICS` /
`LCS_GM_IMAGES` / `LCS_GM_ABS_COLORIMETRIC`), the encoder emits a
124-byte `BITMAPV5HEADER` with `bV5CSType = PROFILE_EMBEDDED` followed
by the colour table (for indexed input) + pixel array + ICC blob.
`top_down` is honoured on every arm; `minimal_palette` trims the
on-disk colour table on the indexed paths. The `Rgb565` arm sets
`biCompression = BI_BITFIELDS` and writes the canonical 5-6-5 masks
into the V5 four-mask region; no separate 12-byte mask tail sits
between the header and the pixel array. The indexed paths set
`biCompression = BI_RGB` (RLE is never chosen on V5 paths since the
spec doesn't define how an RLE pixel stream and a trailing
colour-management blob co-exist on disk).

`EncodeOptions::linked_icc = Some(path)` writes the same 124-byte
`BITMAPV5HEADER` shape but with `bV5CSType = PROFILE_LINKED` and a
caller-supplied **path-string blob** in the trailing slot rather than
the ICC bytes themselves. The path encoding is system-dependent per
spec (typically null-terminated ANSI on Windows); the encoder surfaces
the buffer verbatim so callers that need UTF-16 / URL transport can
pass whatever blob they choose. Decoder side: `BmpMetadata::from_bmp`
sets `BmpColorSpace::ProfileLinked` and exposes `profile_data_offset` /
`profile_size` so callers can resolve the path themselves — the
decoder never auto-loads the linked file. Supported pixel formats
(every `PixelFormat`), `top_down`, and `minimal_palette` handling
match the embedded path.

`EncodeOptions::calibrated_rgb = Some(CalibratedRgb { endpoints, gamma })`
is the V4 colour-space counterpart to those V5 + ICC paths: instead of
pointing at an embedded or linked ICC profile it emits a 108-byte
`BITMAPV4HEADER` with `bV4CSType = LCS_CALIBRATED_RGB` and bakes the
caller-supplied CIE endpoints (`[i32; 9]` `CIEXYZTRIPLE`, packed
R.x R.y R.z G.x G.y G.z B.x B.y B.z) and per-channel gamma triple
(`[u32; 3]`, unsigned 16.16 fixed point) directly into the header's
endpoint / gamma fields. The decoder round-trips it:
`BmpMetadata::from_bmp` reports `BmpColorSpace::Calibrated` and
returns the same `endpoints` + `gamma_rgb` the encoder was given (V4
carries no rendering intent, so `rendering_intent` stays `None`).
Supported pixel formats and option handling match the ICC paths:
`Rgba` / `Bgra` (32-bit BGRA `BI_RGB`), `Rgb24` / `Bgr24` (24-bit BGR
`BI_RGB`), `Rgb555` (16-bit `BI_RGB` 5-5-5, high bit reserved, no mask
block), `Rgb565` (16-bit `BI_BITFIELDS` 5-6-5 with the canonical masks
in the V4 four-mask region), and the indexed `Pal8` / `Indexed4` /
`Indexed2` / `Indexed1` (uncompressed `BI_RGB`, colour table between the header and
the pixel array). RLE is never chosen so the header shape is
deterministic; `top_down` and `minimal_palette` are honoured on every
arm. A caller that only wants to *tag* a bitmap as calibrated without
asserting specific primaries may pass all-zero endpoints + gamma.

`Rgb565` input on either V5 + ICC path emits a 124-byte V5 header
with `biCompression = BI_BITFIELDS`; the canonical R=0xF800 /
G=0x07E0 / B=0x001F masks ride in the header's four-mask region at
offsets 40..56 (the V4 / V5 mask slot) so no separate 12-byte mask
tail is written before the pixel array. The ICC blob
(`PROFILE_EMBEDDED`) or path-string blob (`PROFILE_LINKED`) sits in
the trailing slot exactly as for the `Rgba` / `Rgb24` arms.

`Rgb555` input on either V5 + ICC path (and on the V4-calibrated path)
emits the 16-bit pixels as plain `BI_RGB` 5-5-5 — the high bit is
reserved and R/G/B occupy bits 14..10 / 9..5 / 4..0, so **no** mask
block is written and the header's four-mask region stays zero (the
encode counterpart of the decoder's 16-bit `BI_RGB` 5-5-5 path). The
trailing ICC / path / endpoint-gamma colour-management payload is
unaffected.

`Pal8` / `Indexed4` / `Indexed2` / `Indexed1` input is also accepted on
both V5 + ICC paths: the encoder emits a 124-byte V5 header
with `biCompression = BI_RGB`, writes the colour table between the
header and the pixel array (so `bfOffBits = 14 + 124 + entries × 4`),
sets `biClrUsed` from the supplied palette (honouring
`minimal_palette` to trim the on-disk table to exactly the entries
the caller provided), and parks the ICC or path blob at
`bV5ProfileData` immediately after the pixel array. RLE is never
chosen on the V5 paths since the BMP spec doesn't define how an RLE
pixel stream and a trailing colour-management blob co-exist on disk;
`top_down` is honoured. The decoder side resolves indices against the
palette the same way it does for V3 indexed BMPs and surfaces the
ICC blob (`PROFILE_EMBEDDED`) or the path-string blob
(`PROFILE_LINKED`) through the existing `BmpMetadata` shape with no
caller changes.

`BI_ALPHABITFIELDS` (compression value 6) is the four-mask variant of
`BI_BITFIELDS` documented for Windows CE 5.0+ and accepted by recent
Windows builds: on a V3 (40-byte) `BITMAPINFOHEADER` it appends 16
bytes of R/G/B/A masks instead of `BI_BITFIELDS`' 12 bytes (R/G/B).
On V4/V5 headers the masks already live in the header body, so
`BI_ALPHABITFIELDS` and `BI_BITFIELDS` decode identically. Truncated
mask tails are rejected at the parser boundary; an explicit
`alpha mask = 0` falls back to opaque output to match the
`BI_BITFIELDS` convention.

### 32-bit `BI_RGB` alpha on V4 / V5 headers

The BMP spec is precise about which channels are valid for an
uncompressed 32-bit bitmap: under `BI_RGB` the R / G / B samples occupy
the default BGRA byte order and the per-channel R / G / B masks are
*not* read (they are valid only under `BI_BITFIELDS`), but **the alpha
mask is valid whenever it is present in the DIB header**. V4 / V5
headers always reserve the four-mask block at offsets 40..56 inside the
header body, so a V4 / V5 `BI_RGB` 32-bit bitmap can legitimately carry
an alpha mask there. The decoder honours it: a non-zero in-header alpha
mask makes the alpha sample valid and is extracted *through the mask*
(so an alpha mask parked anywhere — not just the canonical high-byte
`0xFF000000` ARGB layout — decodes correctly), while a zero alpha mask
yields opaque output (the same zero-mask → opaque convention the
`BI_ALPHABITFIELDS` and V3 alpha paths use). This fixes the
otherwise-fully-transparent decode of a V4 / V5 `BI_RGB` bitmap whose
reserved high bytes happen to be zero.

The plain 40-byte `BITMAPINFOHEADER` (V3) `BI_RGB` path is deliberately
*unchanged*: it has no in-header alpha-mask slot, so it keeps reading
the reserved high byte directly as alpha — the behaviour this crate's
own 32-bit BGRA encoder (`encode` → V3 `BI_RGB`) relies on for a
lossless alpha round-trip. The colour-managed V4 / V5 encode paths
(`embed_icc` / `linked_icc` / `calibrated_rgb`) write the canonical
`0xFF000000` alpha mask for 32-bit `Rgba` / `Bgra` input so the file
they emit is a spec-correct
alpha-carrying bitmap rather than one that hides opacity in the
reserved byte.

## Encode details

### Minimal colour table (`biClrUsed`)

```rust
# use oxideav_bmp::{encode, EncodeOptions};
# let image = oxideav_bmp::BmpImage::from_rgba8(1, 1, vec![0, 0, 0, 255]).unwrap();
let bytes = encode(&image, &EncodeOptions::default().with_minimal_palette(true))?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

By default the indexed paths write a full `2^bpp` colour table and
leave `biClrUsed = 0` (the "all colours used" sentinel). Setting
`minimal_palette: true` instead writes exactly as many `RGBQUAD`
entries as the image's `Palette` carries and records that count
in `biClrUsed` — a 2-colour 8-bit image sheds 254 unused entries
(1016 bytes); a 1-entry `Indexed1` table sheds 4 bytes. The count is
clamped to `[1, 2^bpp]`; a palette that already fills the space keeps
the `biClrUsed = 0` sentinel. Composable with `top_down`. The
decoder's `biClrUsed`-aware palette reader consumes the trimmed table
transparently.

### Top-down DIB output

`encode(&image, &EncodeOptions::default().with_top_down(true))`
emits a top-down DIB — rows stored top-to-bottom, `biHeight` written
as a negative integer per the BMP signed-height convention.
Compatible with every `PixelFormat`; the 8/4-bit indexed paths force
the uncompressed fall-back
when `top_down` is set since RLE escape codes have no defined meaning
under a negative `biHeight`. `Indexed1` is always uncompressed and so
unaffected.

### Explicit-mask `BI_BITFIELDS` / `BI_ALPHABITFIELDS` (V3 + mask tail)

`EncodeOptions::bitfields = Some(masks)` emits a bit-field BMP using the classic Windows
in-file mask layout: a 40-byte `BITMAPINFOHEADER` (V3) followed by a
12-byte (R/G/B) or 16-byte (R/G/B/A) DWORD mask tail immediately after
the header, then the pixel array. This is distinct from the in-header
mask block a V4 / V5 header carries — the masks sit **between** the
40-byte header and the pixels, which is exactly the V3 trailing-mask
layout the decoder already reads.

```rust
# let image = oxideav_bmp::BmpImage::from_rgba8(1, 1, vec![0, 0, 0, 255]).unwrap();
use oxideav_bmp::{encode, BmpBitfields, EncodeOptions};

let bytes = encode(&image, &EncodeOptions::default().with_bitfields(BmpBitfields::BGRA8888))?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

`BmpBitfields` carries the four channel masks plus the on-disk bit depth
(16 or 32) and ships five presets:

| Preset      | bpp | R / G / B / A masks                              | Tail | Compression |
| ----------- | --- | ------------------------------------------------ | ---- | ----------- |
| `RGB565`    | 16  | `F800` / `07E0` / `001F` / —                     | 12 B | `BI_BITFIELDS` |
| `RGB555`    | 16  | `7C00` / `03E0` / `001F` / —                     | 12 B | `BI_BITFIELDS` |
| `ARGB1555`  | 16  | `7C00` / `03E0` / `001F` / `8000`                | 16 B | `BI_ALPHABITFIELDS` |
| `BGRA8888`  | 32  | `00FF0000` / `0000FF00` / `000000FF` / `FF000000`| 16 B | `BI_ALPHABITFIELDS` |
| `BGRX8888`  | 32  | `00FF0000` / `0000FF00` / `000000FF` / —         | 12 B | `BI_BITFIELDS` |

The source plane is `Rgba` / `Rgb24` / `Bgra` / `Bgr24`; each 8-bit channel is requantised
to its mask width (the inverse of the decoder's shift-and-scale
`expand`). A non-zero alpha mask selects the four-mask
`BI_ALPHABITFIELDS` tail so alpha survives the round-trip; a zero alpha
mask selects the three-mask `BI_BITFIELDS` tail and decoders treat every
pixel as opaque. For the **byte-aligned 32-bpp presets** every channel
keeps its full 8 bits, so `BGRA8888` round-trips bit-exact (alpha
included) and `BGRX8888` round-trips colour bit-exact (alpha decodes
`0xFF`). `Rgb24` sources synthesise a full-scale alpha run.
`top_down` is honoured. Custom mask sets are accepted as long as
`BmpBitfields::validate` passes — each non-zero mask must be a single
contiguous bit run, masks must not overlap, and every bit must fit
inside `bpp` bits.

## DIB helpers for `.ico`

```rust
# let image = oxideav_bmp::BmpImage::from_rgba8(1, 1, vec![0, 0, 0, 255]).unwrap();
// Headerless DIB (BITMAPINFOHEADER + pixels). No BITMAPFILEHEADER.
let dib = oxideav_bmp::encode_dib(&image, /* doubled */ false)?;
let image = oxideav_bmp::decode_dib(&dib, /* doubled */ false)?;   // native layout

// ICO sub-image variant — height field is 2×, a 1-bpp AND mask is
// appended after the XOR pixels, alpha-channel of the source drives
// the mask (alpha==0 ⇒ mask bit set ⇒ transparent).
let ico_sub = oxideav_bmp::encode_dib(&image, /* doubled */ true)?;
// Decoding a doubled-height DIB folds the AND mask into alpha, so the
// result is always `Rgba`.
let rgba = oxideav_bmp::decode_dib(&ico_sub, /* doubled */ true)?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

## Robustness — property tests + fuzzing

Four deterministic adversarial suites run on every `cargo test`:

* `tests/malformed_inputs.rs` (31 tests) mutates encoder output
  structurally: every-byte truncation sweep, single-bit-flip across
  each header byte, header-size lies (V4/V5 claim on a V3 body),
  negative / zero / `i32::MIN` dimensions, `bfOffBits` past EOF,
  `biClrUsed` over-claim up to `u32::MAX`, illegal bit depths / plane
  counts / compression IDs, RLE-stream truncation, BI_BITFIELDS mask
  truncation, ICO doubled-height edge cases, OS/2 `BITMAPCOREHEADER`
  truncations, plus a deterministic random-mutation burst (1280
  corruptions across 5 base fixtures).
* `tests/hostile_metadata.rs` (9 tests, round 383) points the same
  style at the V4/V5 metadata surfaces: truncation sweeps of four
  colour-management fixtures through both framings, an exhaustive
  single-bit-flip sweep over every fixture byte, hostile
  `bV5ProfileData` / `bV5ProfileSize` pairs (u32 saturation, exact
  wrap, EOF straddle — embedded and linked), a 256-value `bV5CSType`
  probe, and extreme-magnitude round-trips (i32::MIN/MAX endpoints,
  u32::MAX gamma, undefined intents, 0/4096-byte blobs).
* `tests/hostile_masks_overflow.rs` (13 tests, round 383): all 48
  single-bit R/G/B mask positions at 16/32 bpp must decode;
  overlapping / non-contiguous / all-ones / above-bpp mask sets,
  `BI_ALPHABITFIELDS` alpha words and V4 in-header mask patches are
  panic-checked; dimension pairs that wrap i32/u32 area + stride
  maths; `biClrUsed` / `bfOffBits` / `biSizeImage` saturation; RLE4
  delta / absolute-mode / encoded-run overruns and a truncation ×
  byte-value grid over an all-opcode RLE4 stream.
* `tests/fuzz_corpus_replay.rs` (2 tests, round 383) replays every
  committed fuzz corpus file through all six decode surfaces plus the
  typed header parsers, and requires the curated `.bmp` seeds to keep
  decoding `Ok` — the corpora stay exercised even where CI can't
  build the ASan harnesses.

The shared contract: every malformed input must return `Err` (or, for
the ICO doubled-height path's documented missing-AND-mask tolerance,
return safely with the XOR alpha preserved) — never panic, index out
of bounds, or OOM-abort.

Eight `cargo-fuzz` targets live in `fuzz/`:

* `decode` — feeds arbitrary bytes to the contract surface (`probe`,
  `info`, `decode`, `decode_with` under tight strict limits,
  `decode_rgb8`, `decode_rgba8`) and to `decode_dib` (both the plain
  and the doubled-height XOR+AND-mask modes), running `to_rgba8` /
  `to_rgb8` on everything that decodes. The
  seed corpus carries one valid BMP per header / depth / compression
  variant (32/24/16/8/4/1-bpp, RLE4/RLE8, top-down, minimal-palette,
  V4 bitfields header) plus a couple of degenerate framings.
* `rle_stream` — narrows the input so libfuzzer spends its
  iteration budget on the BI_RLE8 / BI_RLE4 state machines instead of
  re-discovering valid 14-byte BITMAPFILEHEADERs. The first three
  fuzz bytes pick the RLE flavour (8 vs 4-bpp), width (1..=255) and
  height (1..=255); the harness wraps the remainder as the pixel
  payload of a synthetic BMP carrying a maximal colour table. Seed
  corpus is two real RLE pixel streams lifted from the `decode` seeds.
* `encode_roundtrip` — closes the symmetry by exercising
  the **encoder** with fuzzer-controlled pixels / palette / encode
  options, then decoding the output back. The first four input bytes
  pick the pixel format (all ten `PixelFormat`s, via `byte % 10`), the
  `top_down` / `minimal_palette` / `rle` option
  flags, and the geometry (clamped to 1..=64 px per axis to keep each
  iteration under ~16 KiB of plane data). The remainder fills the
  pixel plane and, for indexed formats, the palette tail (three bytes
  per `[R, G, B]` entry, padded with zeros so every index resolves).
  The harness asserts the contract's lossless promise on every format:
  `to_rgba8()` of the decoded image equals the source's, and the native
  layouts (`Bgra` / `Bgr24` / `Rgb555` / `Rgb565` / `Pal8`) come back
  with the same `format`, plane bytes and palette. Six seed inputs live
  in `fuzz/corpus/encode_roundtrip/`.
* `metadata` — fuzzes `BmpMetadata::from_bmp` / `from_dib` and the
  `color` / `metadata.icc` the decoder stamps on the image, which are
  independent surfaces with their own attacker-controlled offset / slicing maths
  that the pixel-only `decode` target never reaches: the V4 colour-space
  tail (`bV4CSType`, the nine-`i32` `CIEXYZTRIPLE` endpoints, the
  three-`u32` gamma triple), the V5 colour-management tail
  (`bV5Intent` / `bV5ProfileData` / `bV5ProfileSize`), and the trailing
  ICC / linked-path blob slice `input[base + bV5ProfileData ..][.. size]`
  where both offset and size are fuzzer-controlled `u32` fields. Both
  DIB framings (plain + doubled-height XOR+AND) are fuzzed so the slice
  base (14 for a BMP file, 0 for a header-less DIB) varies. Five seed
  inputs (plain V3, V4 calibrated-RGB, V5 embedded ICC on direct-colour
  and indexed images, V5 linked ICC) live in `fuzz/corpus/metadata/`.
* `bitfields_roundtrip` — drives the explicit-mask
  `EncodeOptions::bitfields` encoder with fuzzer-controlled pixels, a mask
  preset (RGB565 / RGB555 / ARGB1555 / BGRA8888 / BGRX8888) or an
  arbitrary mask set that exercises `BmpBitfields::validate`'s reject
  path, plus the `top_down` option, then decodes the output. The
  byte-aligned 32-bpp presets additionally assert the documented exact
  round-trip (`BGRA8888` lossless including alpha; `BGRX8888`
  colour-exact with alpha decoding opaque) and the native `Bgra`
  layout; the 16-bpp presets are shape-checked.
* `header_forge` (round 383) — the fuzzer's bytes become raw DIB
  header *fields* (Core / Info / V2 / V3 / V4 / V5 / OS2-64 plus
  arbitrary `biSize`) wrapped in always-well-formed BMP + DIB framing
  (magic selector, wrapping `bfOffBits` delta, verbatim body), so the
  iteration budget lands inside the header-validation matrix instead
  of rediscovering signatures and offsets. Every forged file runs
  through `info` / `decode` / `BmpMetadata::from_bmp` and the three
  DIB parse surfaces. 15 seeds derived from the
  `decode` + `metadata` corpora.
* `icc_roundtrip` (round 383) — drives the three colour-management
  encode modes (`metadata.icc` + `embed_icc`, `linked_icc`,
  `calibrated_rgb`) across 9 pixel formats × options × blob sizes, then
  asserts via `decode` + `BmpMetadata::from_bmp` that the colour-space
  tag, blob / path bytes (header record and `metadata.icc`), endpoints +
  gamma, and every pixel return verbatim. Encoder `Err` is accepted; undecodable encoder output is
  a crash.
* `dib_roundtrip` (round 383) — drives `encode_dib` (the `.ico` /
  `.cur` shared surface) across all 10 formats and both layouts (plain
  + doubled-height XOR/AND). Matching-flag decode must succeed with
  exact geometry, the plain layout must be pixel-exact, and the same bytes
  are decoded under the opposite mask flag as a panic-check — the
  classic hostile-`.ico` confusion.

All eight targets share the same panic-free contract — every input
returns a `Result` rather than panicking, indexing out of bounds, or
OOM-aborting — and build against the framework-free standalone path
(`default-features = false`).

```sh
cargo +nightly fuzz run decode          # or any of the other seven:
cargo +nightly fuzz run header_forge    # rle_stream, metadata,
cargo +nightly fuzz run icc_roundtrip   # encode_roundtrip,
cargo +nightly fuzz run dib_roundtrip   # bitfields_roundtrip
```

The `decode` harness shook out and fixed several header-driven
denial-of-service paths (RLE / `bpp = 0` / `biClrUsed` over-allocation)
in earlier rounds; see `CHANGELOG.md`. The round-383 hardening
campaign ran ≈ 19 M executions across the eight targets (2 M+ each on
the four decode-side targets at ~25–160 k execs/sec, 2 M each on the
four encoder round-trip targets including a `-use_value_profile=1`
soak) with **zero decoder / encoder defects** — the only stops were
two libFuzzer rss-watermark trips under a deliberately tightened
1 GiB ceiling that replayed clean in isolation and completed at the
default limit (allocator quarantine accumulation, not a decoder
allocation). A daily `.github/workflows/fuzz.yml` job runs all eight
targets on a shared 30-minute budget via the org reusable workflow's
`[[bin]]` auto-discovery.

## Benchmarks

Criterion benches at `benches/` cover the decoder, encoder, and full
roundtrip across every bit depth + compression combination. They build
fresh fixtures via the public encoder API so nothing is committed
to disk.

```sh
cargo bench -p oxideav-bmp --bench decode
cargo bench -p oxideav-bmp --bench encode
cargo bench -p oxideav-bmp --bench roundtrip
```

Indicative throughput (Apple M-series, `--quick`):

| Bench                                         | Throughput     |
| --------------------------------------------- | -------------- |
| `decode_rgba_320x240`                         | ~5.0 GiB/s     |
| `decode_rgb24_640x480`                        | ~3.4 GiB/s     |
| `decode_indexed8_320x240`                     | ~1.2 GiB/s     |
| `decode_rle8_320x240` (row-constant fixture)  | ~1.2 GiB/s     |
| `encode_rgba_320x240`                         | ~10 GiB/s      |
| `encode_indexed8_random_320x240` (RLE try+fb) | ~1.27 GiB/s    |
| `encode_indexed8_rle_friendly_320x240`        | ~2.0 GiB/s     |
| `roundtrip_rgba_320x240`                      | ~3.95 GiB/s    |
| `roundtrip_dib_ico_rgba_64x64`                | ~1.7 GiB/s     |

Per-operation before/after timings for the hot-path optimisation work
live in [`BENCHMARKS.md`](BENCHMARKS.md).

Every decode path fills one flat top-down RGBA plane in a single pass (a
`chunks_exact_mut(4)` cursor, no per-scanline allocation) — the
uncompressed depths, and now the `BI_RLE8` / `BI_RLE4` decoders too,
which write each pixel straight to its flipped destination row instead
of building per-row vectors and concatenating them. Indexed depths index
a fixed-size padded palette array so the per-pixel bounds check drops
out. For `BI_BITFIELDS` / mask-carrying `BI_RGB`, four 256-byte
per-channel expansion tables (L1-resident) replace the per-pixel mask
expansions below the size where a 65 536-entry combined value→RGBA table
amortises its build; the combined table is kept above that threshold.
The encoder's BGR(A) packers walk `chunks_exact` for a bounds-check-free
shuffle, and the indexed RLE size probe aborts as soon as the compressed
stream exceeds the raw array on incompressible input. `encode` and
`encode_with_report` write each pixel row straight into its final place
in the output, with no intermediate plane (the RLE probe writes into the
same buffer), so an encode allocates the file and little else.
