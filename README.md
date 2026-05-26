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
| 24        | `BI_RGB`       | `Rgba` (BGR→RGB, α=0xFF) |
| 32        | `BI_RGB`       | `Rgba` (BGRA→RGBA) |
| 32        | `BI_BITFIELDS` | `Rgba` (mask-derived) |

`BITMAPCOREHEADER` (OS/2 1.x, 12 B), `BITMAPINFOHEADER` (v3, 40 B),
`BITMAPV4HEADER`, and `BITMAPV5HEADER` are all accepted. The OS/2 path
honours the 3-byte `RGBTRIPLE` colour-table layout (V3+ uses 4-byte
`RGBQUAD`). Bottom-up and top-down row orders are auto-detected from
the sign of `biHeight`; output is always top-down `Rgba`. `BI_JPEG`
and `BI_PNG` are rejected at the boundary.

## Encode

| Input format        | BMP output                    | Header |
| ------------------- | ----------------------------- | ------ |
| `Rgba` (4 B/px)     | 32-bit BGRA `BI_RGB`          | V3     |
| `Rgb24` (3 B/px)    | 24-bit BGR `BI_RGB`           | V3     |
| `Rgb565` (2 B/px)   | 16-bit `BI_BITFIELDS` 5-6-5   | V4     |
| `Indexed8` (1 B/px) | 8-bit indexed `BI_RGB` or `BI_RLE8` (auto) | V3 |
| `Indexed4` (1 B/px) | 4-bit indexed `BI_RGB` or `BI_RLE4` (auto) | V3 |

For `Rgb565` the V4 header carries canonical masks R=0xF800, G=0x07E0,
B=0x001F. For indexed formats the encoder tries RLE compression first
and falls back to uncompressed when RLE does not shrink the output.

`Indexed8` and `Indexed4` require a `BmpPalette` alongside the image.
Up to 256 (8-bit) or 16 (4-bit) entries; unused entries are
zero-padded in the on-disk colour table.

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
(1016 bytes). The count is clamped to `[1, 2^bpp]`; a palette that
already fills the space keeps the `biClrUsed = 0` sentinel. Composable
with `top_down`. The decoder's `biClrUsed`-aware palette reader (and
ImageMagick) consume the trimmed table transparently.

### Top-down DIB output

`encode_bmp_with_options(&image, BmpEncodeOptions { top_down: true })`
emits a top-down DIB — rows stored top-to-bottom, `biHeight` written
as a negative integer per the BMP signed-height convention.
Compatible with `Rgba` / `Rgb24` / `Rgb565` / `Indexed8` / `Indexed4`;
the indexed paths force the uncompressed fall-back when `top_down`
is set since RLE escape codes have no defined meaning under a
negative `biHeight`.

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

A `cargo-fuzz` harness lives in `fuzz/`. The `decode` target feeds
arbitrary bytes to `decode_bmp` and to `decode_dib` (both the plain and
the doubled-height XOR+AND-mask modes) and asserts the decoder always
returns a `Result` rather than panicking, indexing out of bounds, or
OOM-aborting. It builds against the framework-free standalone path
(`default-features = false`).

```sh
cargo +nightly fuzz run decode
```

The seed corpus carries one valid BMP per header / depth / compression
variant (32/24/16/8/4/1-bpp, RLE4/RLE8, top-down, minimal-palette, V4
bitfields header) plus a couple of degenerate framings. The harness
shook out and fixed several header-driven denial-of-service paths (RLE /
`bpp = 0` / `biClrUsed` over-allocation); see `CHANGELOG.md`. A daily
`.github/workflows/fuzz.yml` job runs the target on a 30-minute budget.

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
