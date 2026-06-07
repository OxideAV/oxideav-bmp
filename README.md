# oxideav-bmp

Pure-Rust BMP (Windows bitmap) codec and container for the
[`oxideav`](https://github.com/OxideAV/oxideav) framework. Also
exposes the headerless **DIB** helpers used by `.ico` / `.cur`
sub-images.

## Decode

| Bit depth | Compression    | Output     |
| --------- | -------------- | ---------- |
| 1         | `BI_RGB`       | `Rgba`     |
| 4         | `BI_RGB`       | `Rgba`     |
| 4         | `BI_RLE4`      | `Rgba` (delta + absolute mode) |
| 8         | `BI_RGB`       | `Rgba`     |
| 8         | `BI_RLE8`      | `Rgba` (delta + absolute mode) |
| 16        | `BI_RGB`       | `Rgba` (5-5-5) |
| 16        | `BI_BITFIELDS` | `Rgba` (mask-derived) |
| 16        | `BI_ALPHABITFIELDS` | `Rgba` (mask-derived, R/G/B/A) |
| 24        | `BI_RGB`       | `Rgba` (BGR→RGB, α=0xFF) |
| 32        | `BI_RGB`       | `Rgba` (BGRA→RGBA) |
| 32        | `BI_BITFIELDS` | `Rgba` (mask-derived) |
| 32        | `BI_ALPHABITFIELDS` | `Rgba` (mask-derived, R/G/B/A) |

`BITMAPCOREHEADER` (OS/2 1.x, 12 B), `BITMAPINFOHEADER` (v3, 40 B),
`BITMAPV4HEADER`, and `BITMAPV5HEADER` are all accepted. The OS/2 path
honours the 3-byte `RGBTRIPLE` colour-table layout (V3+ uses 4-byte
`RGBQUAD`). Bottom-up and top-down row orders are auto-detected from
the sign of `biHeight`; output is always top-down `Rgba`. `BI_JPEG`
and `BI_PNG` are rejected at the boundary.

### V3+ device-resolution + palette-count metadata

`BmpMetadata` (returned by `decode_bmp_with_metadata` /
`decode_dib_with_metadata`) also surfaces the four V3+ metadata fields
that pre-date colour management: `biXPelsPerMeter`, `biYPelsPerMeter`,
`biClrUsed`, and `biClrImportant`. The named accessors:

```rust
let (_image, md) = oxideav_bmp::decode_bmp_with_metadata(bytes)?;
md.pixels_per_meter_x      // Option<i32>  — None on OS/2 V1
md.pixels_per_meter_y      // Option<i32>
md.colors_used             // Option<u32>  — `0` = "all 2^bpp"
md.colors_important        // Option<u32>  — `0` = "all important"
md.dpi_x();                // Option<u32>  — derived, rounded to nearest
md.dpi_y();                // Option<u32>
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

`decode_bmp_with_metadata` / `decode_dib_with_metadata` return a
`(BmpImage, BmpMetadata)` pair so callers that need the V4/V5
colour-management tail can inspect `bV4CSType`, the `CIEXYZTRIPLE`
endpoints, the `R/G/B` gamma triple, the V5 rendering intent, and the
on-disk `bV5ProfileData` / `bV5ProfileSize` fields. A V5 header that
declares `PROFILE_EMBEDDED` additionally surfaces the embedded ICC blob
as `BmpMetadata::icc_profile: Option<Vec<u8>>`; `PROFILE_LINKED`
surfaces the offset + size so callers can resolve the path themselves.

```rust
let (image, md) = oxideav_bmp::decode_bmp_with_metadata(bytes)?;
match md.color_space {
    Some(oxideav_bmp::BmpColorSpace::SRgb) => /* sRGB */ {}
    Some(oxideav_bmp::BmpColorSpace::ProfileEmbedded) => {
        let icc = md.icc_profile.as_deref().unwrap_or(&[]);
        // hand off `icc` to your colour-management pipeline
    }
    _ => {}
}
```

The typed accessor `BmpMetadata::icc_profile_ref()` collapses the
PROFILE_EMBEDDED / PROFILE_LINKED / no-ICC discrimination into a
single `BmpIccProfileRef<'_>` enum so callers don't have to match on
`color_space` and then read `icc_profile` / `linked_profile_path` /
`profile_data_offset` / `profile_size` by hand:

```rust
use oxideav_bmp::BmpIccProfileRef;
match md.icc_profile_ref() {
    BmpIccProfileRef::Embedded(icc)    => { /* embedded ICC bytes */ }
    BmpIccProfileRef::Linked(path)     => { /* path bytestring */ }
    BmpIccProfileRef::Declared { .. }  => { /* V5 declared a PROFILE_* but the bytes were unreachable */ }
    BmpIccProfileRef::None             => { /* V3 / V4 / V5 LCS_* — no ICC reference */ }
}
```

`PROFILE_LINKED` bitmaps now also surface the path bytestring through
the dedicated `BmpMetadata::linked_profile_path: Option<Vec<u8>>`
field (parallel to `icc_profile` for the embedded variant). The
decoder still never opens the file the path points at — the path is
returned verbatim and its encoding (typically null-terminated ANSI on
Windows) is the caller's responsibility.


V3 / OS/2 headers report every metadata field as `None` (they pre-date
colour management). V4 fills `color_space` / `endpoints` / `gamma_rgb`;
V5 additionally fills `rendering_intent`. The decode-path itself is
unchanged — pixels still come out as top-down `Rgba` regardless of the
declared colour space — and the original `decode_bmp` / `decode_dib`
entry points stay byte-for-byte compatible. A V5 header that lies about
its ICC offset / size (slice falls past EOF) leaves
`icc_profile = None` with the declared fields still populated so the
metadata path can never make decode fail on its own.

`encode_bmp_with_icc_profile` is the matching encode side: pass an
`Rgba`, `Rgb24`, `Rgb565`, `Indexed8`, `Indexed4`, or `Indexed1`
`BmpImage` plus an ICC blob plus an intent constant (0 for
unspecified, or one of `LCS_GM_BUSINESS` / `LCS_GM_GRAPHICS` /
`LCS_GM_IMAGES` / `LCS_GM_ABS_COLORIMETRIC`) and the encoder emits a
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

`encode_bmp_with_linked_icc_profile` writes the same 124-byte
`BITMAPV5HEADER` shape but with `bV5CSType = PROFILE_LINKED` and a
caller-supplied **path-string blob** in the trailing slot rather than
the ICC bytes themselves. The path encoding is system-dependent per
spec (typically null-terminated ANSI on Windows); the encoder surfaces
the buffer verbatim so callers that need UTF-16 / URL transport can
pass whatever blob they choose. Decoder side: `decode_bmp_with_metadata`
sets `BmpColorSpace::ProfileLinked` and exposes `profile_data_offset` /
`profile_size` so callers can resolve the path themselves — the
decoder never auto-loads the linked file. Supported pixel formats
(`Rgba` / `Rgb24` / `Rgb565` / `Indexed8` / `Indexed4` / `Indexed1`),
`top_down`, and `minimal_palette` handling match the embedded path.

`Rgb565` input on either V5 + ICC path emits a 124-byte V5 header
with `biCompression = BI_BITFIELDS`; the canonical R=0xF800 /
G=0x07E0 / B=0x001F masks ride in the header's four-mask region at
offsets 40..56 (the V4 / V5 mask slot) so no separate 12-byte mask
tail is written before the pixel array. The ICC blob
(`PROFILE_EMBEDDED`) or path-string blob (`PROFILE_LINKED`) sits in
the trailing slot exactly as for the `Rgba` / `Rgb24` arms.

`Indexed8` / `Indexed4` / `Indexed1` input is also accepted on both
V5 + ICC paths (round 231): the encoder emits a 124-byte V5 header
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

## Encode

| Input format        | BMP output                    | Header |
| ------------------- | ----------------------------- | ------ |
| `Rgba` (4 B/px)     | 32-bit BGRA `BI_RGB`          | V3     |
| `Rgb24` (3 B/px)    | 24-bit BGR `BI_RGB`           | V3     |
| `Rgb565` (2 B/px)   | 16-bit `BI_BITFIELDS` 5-6-5   | V4     |
| `Indexed8` (1 B/px) | 8-bit indexed `BI_RGB` or `BI_RLE8` (auto) | V3 |
| `Indexed4` (1 B/px) | 4-bit indexed `BI_RGB` or `BI_RLE4` (auto) | V3 |
| `Indexed1` (1 B/px) | 1-bit indexed `BI_RGB` (monochrome) | V3 |

For `Rgb565` the V4 header carries canonical masks R=0xF800, G=0x07E0,
B=0x001F. For 8/4-bit indexed formats the encoder tries RLE compression
first and falls back to uncompressed when RLE does not shrink the
output. BMP has no RLE flavour at 1 bpp so `Indexed1` is always
emitted as uncompressed `BI_RGB`.

`Indexed8`, `Indexed4`, and `Indexed1` all require a `BmpPalette`
alongside the image: up to 256 (8-bit), 16 (4-bit), or 2 (1-bit)
entries. Pixel-byte inputs carry `idx & 0xFF` / `idx & 0x0F` /
`idx & 1` respectively; the encoder packs them MSB-first per the BMP
spec. Unused entries are zero-padded in the on-disk colour table; set
`minimal_palette = true` to record only the entries actually supplied.

### Minimal colour table (`biClrUsed`)

```rust
encode_bmp_with_options(&image, BmpEncodeOptions {
    minimal_palette: true,
    ..Default::default()
})
```

By default the indexed paths write a full `2^bpp` colour table and
leave `biClrUsed = 0` (the "all colours used" sentinel). Setting
`minimal_palette: true` instead writes exactly as many `RGBQUAD`
entries as the supplied `BmpPalette` carries and records that count
in `biClrUsed` — a 2-colour 8-bit image sheds 254 unused entries
(1016 bytes); a 1-entry `Indexed1` table sheds 4 bytes. The count is
clamped to `[1, 2^bpp]`; a palette that already fills the space keeps
the `biClrUsed = 0` sentinel. Composable with `top_down`. The
decoder's `biClrUsed`-aware palette reader (and the `magick` black-box
validator) consume the trimmed table transparently.

### Top-down DIB output

`encode_bmp_with_options(&image, BmpEncodeOptions { top_down: true })`
emits a top-down DIB — rows stored top-to-bottom, `biHeight` written
as a negative integer per the BMP signed-height convention.
Compatible with `Rgba` / `Rgb24` / `Rgb565` / `Indexed8` / `Indexed4` /
`Indexed1`; the 8/4-bit indexed paths force the uncompressed fall-back
when `top_down` is set since RLE escape codes have no defined meaning
under a negative `biHeight`. `Indexed1` is always uncompressed and so
unaffected.

## DIB helpers for `.ico`

```rust
// Headerless DIB (BITMAPINFOHEADER + pixels). No BITMAPFILEHEADER.
let dib = oxideav_bmp::encode_dib(&frame, /* doubled */ false)?;
let frame = oxideav_bmp::decode_dib(&dib, /* doubled */ false)?;

// ICO sub-image variant — height field is 2×, a 1-bpp AND mask is
// appended after the XOR pixels, alpha-channel of the source drives
// the mask (alpha==0 ⇒ mask bit set ⇒ transparent).
let ico_sub = oxideav_bmp::encode_dib(&frame, /* doubled */ true)?;
```

## Robustness — property tests + fuzzing

`tests/malformed_inputs.rs` runs 31 deterministic structural-mutation
tests on top of the public encoder API: every-byte truncation sweep,
single-bit-flip across each header byte, header-size lies (V4/V5 claim
on a V3 body), negative / zero / `i32::MIN` dimensions, `bfOffBits`
past EOF, `biClrUsed` over-claim up to `u32::MAX`, illegal bit depths /
plane counts / compression IDs, RLE-stream truncation, BI_BITFIELDS
mask truncation, ICO doubled-height edge cases, OS/2
`BITMAPCOREHEADER` truncations, plus a deterministic random-mutation
burst (1280 corruptions across 5 base fixtures). The contract is the
same as the fuzz harness: every malformed input must return `Err`
(or, for the ICO doubled-height path's documented missing-AND-mask
tolerance, return safely with the XOR alpha preserved) — never panic,
index out of bounds, or OOM-abort.

Three `cargo-fuzz` targets live in `fuzz/`:

* `decode` — feeds arbitrary bytes to `decode_bmp` and to `decode_dib`
  (both the plain and the doubled-height XOR+AND-mask modes). The
  seed corpus carries one valid BMP per header / depth / compression
  variant (32/24/16/8/4/1-bpp, RLE4/RLE8, top-down, minimal-palette,
  V4 bitfields header) plus a couple of degenerate framings.
* `rle_stream` (round 162) — narrows the input so libfuzzer spends its
  iteration budget on the BI_RLE8 / BI_RLE4 state machines instead of
  re-discovering valid 14-byte BITMAPFILEHEADERs. The first three
  fuzz bytes pick the RLE flavour (8 vs 4-bpp), width (1..=255) and
  height (1..=255); the harness wraps the remainder as the pixel
  payload of a synthetic BMP carrying a maximal colour table. Seed
  corpus is two real RLE pixel streams lifted from the `decode` seeds.
* `encode_roundtrip` (round 198) — closes the symmetry by exercising
  the **encoder** with fuzzer-controlled pixels / palette / encode
  options, then decoding the output back. The first four input bytes
  pick the pixel format (`Rgba` / `Rgb24` / `Rgb565` / `Indexed8` /
  `Indexed4` / `Indexed1`), the `top_down` + `minimal_palette` option
  flags, and the geometry (clamped to 1..=64 px per axis to keep each
  iteration under ~16 KiB of plane data). The remainder fills the
  pixel plane and, for indexed formats, the palette tail (three bytes
  per `[R, G, B]` entry, padded with zeros so every index resolves).
  For the two direct-colour formats the harness additionally asserts
  that every decoded pixel byte matches what the encoder was given
  (R / G / B / alpha); indexed and `Rgb565` paths are panic-checked
  only since the decoder materialises `Rgba` and a 1 B/px → 4 B/px
  comparison would be apples-to-oranges. Six seed inputs (one per
  format) live in `fuzz/corpus/encode_roundtrip/`.

All three targets share the same panic-free contract — every input
returns a `Result` rather than panicking, indexing out of bounds, or
OOM-aborting — and build against the framework-free standalone path
(`default-features = false`).

```sh
cargo +nightly fuzz run decode
cargo +nightly fuzz run rle_stream
cargo +nightly fuzz run encode_roundtrip
```

The `decode` harness shook out and fixed several header-driven
denial-of-service paths (RLE / `bpp = 0` / `biClrUsed` over-allocation);
see `CHANGELOG.md`. A local 20-second `rle_stream` run lands ~1.5 M
inputs (~72 k execs/sec) with zero crashes. A 60-second
`encode_roundtrip` run lands ~1.33 M inputs (~21.8 k execs/sec, peak
RSS ~480 MB) with zero crashes — every direct-colour input survived
the encode→decode pair byte-for-byte. A daily
`.github/workflows/fuzz.yml` job runs all three targets on a shared
30-minute budget via the org reusable workflow's `[[bin]]`
auto-discovery.

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

Round 129 headline numbers (Apple M-series, `--quick`):

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

## Registration

```rust
let mut codecs = oxideav_codec::CodecRegistry::new();
let mut containers = oxideav_container::ContainerRegistry::new();
oxideav_bmp::register(&mut codecs, &mut containers);
```
