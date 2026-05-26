# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **Property tests for malformed inputs** (round 155, `tests/malformed_inputs.rs`):
  31 deterministic structural-mutation tests that complement the
  `cargo-fuzz` `decode` target. Each test starts from a valid
  encoder output (RGBA / RGB24 / RGB565 / Indexed8 / Indexed4 / DIB /
  ICO-DIB) and applies one targeted transform — every-byte
  truncation sweep, single-bit-flip across each header byte,
  header-size lie (V4/V5 claim with V3 body, sub-12-byte sizes,
  4-GiB sizes), negative / zero / `i32::MIN` width-height,
  `bfOffBits` past EOF or inside the file header, `biClrUsed`
  over-claim up to `u32::MAX`, illegal bit-depths (0/2/3/5/6/7/9/…
  /0xFFFF), illegal planes (anything ≠ 1), unknown compression
  IDs, BI_RLE8 / BI_RLE4 with the wrong bpp, RLE-stream truncation
  inside the pixel region, palette truncation, BI_BITFIELDS mask
  truncation, all-zero BI_BITFIELDS masks, ICO doubled-height with
  odd / `i32::MAX` height, OS/2 `BITMAPCOREHEADER` zero-width /
  unsupported-bpp / truncated-palette — plus a deterministic
  random-mutation burst (5 base fixtures × 256 rounds × 1-4 byte
  flips). 67 tests total in the crate (27 lib + 9 magick + 31 new).
  No new dependencies; pure std + the existing public API. The
  ICO doubled-height variant intentionally tolerates a missing AND
  mask (some real-world icons lie about the trailing 1-bpp size)
  so the truncation sweep relaxes to "no panic, Ok-or-Err allowed"
  past the XOR-pixel region for that path — confirmed against the
  documented behaviour in `decode_dib_with_mask`.

- **Fuzz CI**: daily `.github/workflows/fuzz.yml` runs the `decode`
  cargo-fuzz target on the org reusable fuzz workflow (30-minute budget,
  decode-only panic-free harness). Added a 1-bpp indexed seed
  (`indexed1_8x2.bmp`) to the corpus so the 1-bit palette-lookup path is
  seeded directly. Local 60s baseline: 6.5M executions, zero crashes.

- **Benchmarks**: Criterion harnesses at `benches/{decode,encode,roundtrip}.rs`
  mirroring the cinepak / tta / flac shape. Decode bench covers RGBA /
  RGB24 / RGB565 / Indexed8 / Indexed4 / RLE8 / RLE4 / DIB / ICO-DIB
  variants (9 scenarios). Encode bench covers the same set plus the
  `minimal_palette` and `top_down` option paths and the
  "random data → RLE auto-picker falls back to BI_RGB" vs "row-constant
  data → RLE chosen" picker cases (10 scenarios). Roundtrip bench
  pairs each bit depth's encoder and decoder so a regression that
  silently mis-encodes panics on the decode side (6 scenarios). All
  fixtures synthesised on the fly from `xorshift32` / gradient
  generators — no committed binary fixtures. Run with
  `cargo bench -p oxideav-bmp --bench <decode|encode|roundtrip>`.
  Baseline measurements (Apple M-series, `--quick`): decode ≈
  1.2..5.0 GiB/s depending on path, encode ≈ 1.27..12 GiB/s, full
  roundtrip ≈ 0.4..4.0 GiB/s.

- **Fuzzing**: `cargo-fuzz` harness at `fuzz/fuzz_targets/decode.rs`
  driving the standalone decode entry points (`decode_bmp`, plus
  `decode_dib` in both the plain and doubled-height XOR+AND-mask modes)
  over arbitrary bytes — the contract is that every malformed input
  returns `Err` rather than panicking, indexing out of bounds, or
  OOM-aborting. Built `default-features = false` (framework-free path,
  no `oxideav-core`). Seed corpus of 11 valid BMPs (32/24/16/8/4-bpp,
  RLE4/RLE8, top-down, minimal-palette) plus degenerate edge inputs.
  7.6M executions / 0 crashes after the fixes below.

### Fixed

- **Decoder DoS / panic hardening** (found by the new fuzz harness):
  - `decode_dib`: computing the colour-table size as
    `palette_entries() as u32 * entry_size` overflowed `u32` for a
    huge `biClrUsed` and aborted; the offset maths now runs in `usize`
    with saturating arithmetic.
  - RLE4 / RLE8: the decoders pre-allocated the full `width × height`
    grid from the header alone, so a `0x7FFF_FFFF × 0x7FFF_FFFF` claim
    asked the allocator for exabytes (OOM-abort). A byte-ceiling guard
    (`width × height ≤ rle_bytes × 255`, the maximum a stream of that
    length can decode to) now rejects inconsistent dimensions, and an
    out-of-range `bfOffBits` is rejected before the RLE slice instead
    of panicking.
  - `bpp = 0` (only legal for the rejected BI_JPEG / BI_PNG) yielded a
    zero row stride, so the "pixel array truncated" check passed for
    any height and `decode_pixels` reserved a 134-million-row vector
    (OOM). Bit depth is now validated up front; a non-zero stride keeps
    the height bounded by the available bytes.
  - Pixel-array and ICO AND-mask offset maths switched to saturating
    arithmetic so attacker-supplied dimensions can't wrap a length
    check into an in-bounds slice.
  - Regression tests: `decode_dib_huge_clr_used_does_not_overflow`,
    `rle8_giant_dimensions_rejected_not_oom`,
    `rle4_giant_dimensions_rejected_not_oom`,
    `rle8_pixel_offset_past_eof_rejected`,
    `zero_bpp_huge_height_rejected_not_oom`,
    `rle8_small_consistent_dims_still_decode`.

- **Encoder**: minimal colour-table (`biClrUsed`-limited palette) write
  path via the new `BmpEncodeOptions::minimal_palette` flag. When set,
  the indexed paths (`Indexed8` / `Indexed4`) write only as many
  `RGBQUAD` colour-table entries as the supplied `BmpPalette` actually
  carries and record that count in the header's `biClrUsed` field,
  instead of zero-padding the table out to the full `2^bpp` entries
  with `biClrUsed = 0`. A 2-colour 8-bit image drops 254 unused
  entries (1016 bytes); a 4-colour 4-bit image drops 12 (48 bytes).
  A palette that already fills the `2^bpp` space keeps the classic
  `biClrUsed = 0` "full table" sentinel, and the entry count is
  clamped to `[1, 2^bpp]` so it can never overflow the index space.
  The decoder's existing `biClrUsed`-aware palette reader consumes the
  shorter table transparently; default output (`minimal_palette =
  false`) is byte-for-byte unchanged. Composes with `top_down`.
- Tests: `minimal_palette_8bit_shrinks_table_and_roundtrips`,
  `minimal_palette_4bit_shrinks_table_and_roundtrips`,
  `minimal_palette_full_table_keeps_clr_used_zero`,
  `minimal_palette_top_down_roundtrips`, plus ImageMagick
  cross-validation `magick_minimal_palette_8bpp` confirming `magick`
  resolves pixels against the trimmed `biClrUsed=2` table.

## [0.1.4](https://github.com/OxideAV/oxideav-bmp/compare/v0.1.3...v0.1.4) - 2026-05-05

### Other

- fix clippy::ptr_arg in magick_validate (use &Path not &PathBuf)
- add 24-bit / 16-bit RGB565 / indexed-8 / indexed-4 / RLE8 / RLE4 write paths

### Added

- **Decoder**: OS/2 1.x `BITMAPCOREHEADER` (12-byte) read support. The
  alternate 12-byte header layout is recognised alongside the 40 / 108 /
  124-byte variants; `u16` width / height fields and 3-byte `RGBTRIPLE`
  palette entries are decoded into the same `BmpImage` shape as every
  other path. Negative heights are not legal in this header so the
  decoder always treats OS/2 input as bottom-up.
- **Encoder**: Top-down DIB output via `encode_bmp_with_options` /
  `encode_bmp_plane_with_options` and the new `BmpEncodeOptions`
  struct. Setting `top_down: true` writes rows top-to-bottom with a
  negative `biHeight` per the BMP spec. Indexed formats fall back to
  uncompressed `BI_RGB` when `top_down` is requested since RLE
  streams with a negative height are spec-illegal.
- `BmpEncodeOptions` (currently exposes `top_down: bool`) and the
  matching `encode_bmp_with_options` / `encode_bmp_plane_with_options`
  entry points. The existing `encode_bmp` / `encode_bmp_plane`
  remain unchanged — they call the new entry points with
  `BmpEncodeOptions::default()`.
- `BITMAPCOREHEADER_SIZE` public constant exposed from the crate
  root.
- Tests: `decode_os2_bitmapcoreheader_24bpp`,
  `decode_os2_bitmapcoreheader_4bpp_indexed`,
  `encode_top_down_rgba_negative_height_and_roundtrip`,
  `encode_top_down_rgb24_roundtrip`,
  `encode_top_down_indexed8_skips_rle`. ImageMagick cross-validation
  test `magick_top_down_rgba` verifies magick honours the negative
  `biHeight` on our output.
- **Decoder**: `BI_RLE8` and `BI_RLE4` decode support. Both encoded-run
  and absolute-mode packets are handled; delta (`0x02`) escape codes are
  also accepted. Output is always top-down `Rgba`, same as every other
  decode path.
- **Encoder**: 24-bit BGR `BI_RGB` write path (`BmpPixelFormat::Rgb24`).
  Input strides of 3 or 4 bytes/pixel are both accepted; output rows are
  4-byte padded per spec.
- **Encoder**: 16-bit `BI_BITFIELDS` RGB 5-6-5 write path
  (`BmpPixelFormat::Rgb565`). Emits a BITMAPV4HEADER (108 B) with
  canonical masks R=0xF800, G=0x07E0, B=0x001F, A=0x0000 and CS type
  set to LCS_sRGB.
- **Encoder**: 8-bit uncompressed indexed `BI_RGB` write path
  (`BmpPixelFormat::Indexed8`). Requires a `BmpPalette`; unused colour
  table slots are zero-padded to fill all 256 entries.
- **Encoder**: 4-bit uncompressed indexed `BI_RGB` write path
  (`BmpPixelFormat::Indexed4`). Same palette contract; full 16-entry
  table written.
- **Encoder**: `BI_RLE8` write path with automatic fallback. Encodes a
  run-length compressed payload and chooses it only when it is strictly
  smaller than the raw indexed bytes; otherwise falls back to
  uncompressed `BI_RGB`.
- **Encoder**: `BI_RLE4` write path with automatic fallback (same
  heuristic as RLE8).
- `BmpPixelFormat::Rgb565`, `BmpPixelFormat::Indexed8`,
  `BmpPixelFormat::Indexed4` variants added to the pixel-format enum.
- `BmpPalette` struct added for passing colour tables to indexed encode
  paths.
- `BmpImage::palette: Option<BmpPalette>` field added (always `None` on
  the decode path, which always produces `Rgba`).
- `EncodedBmpFormat` enum returned by `encode_bmp` / `encode_bmp_plane`
  so callers can tell which compression was actually used.
- `tests/magick_validate.rs` integration tests: 7 ImageMagick
  cross-validation cases (32-bit, 24-bit, 16-bit, 8-bit indexed, 4-bit
  indexed, RLE8, RLE4).

### Changed

- `encode_bmp` and `encode_bmp_plane` now return `Result<(Vec<u8>,
  EncodedBmpFormat)>` instead of `Result<Vec<u8>>`. Callers that only
  care about the bytes can destructure with `let (bytes, _) = …`.
- `encode_bmp_plane` gains a `palette: Option<&BmpPalette>` parameter
  (required for indexed formats, ignored otherwise).

## [0.1.3](https://github.com/OxideAV/oxideav-bmp/compare/v0.1.2...v0.1.3) - 2026-05-04

### Other

- cargo fmt: reorder use statements in src/lib.rs
- Standalone-friendly retrofit: gate oxideav-core behind `registry`

### Changed

- Standalone-friendly retrofit (#360): `oxideav-core` is now an
  optional dep behind a default-on `registry` cargo feature.
  Image-library consumers can depend on `oxideav-bmp` with
  `default-features = false` to get a framework-free build that
  exposes the standalone `decode_bmp` / `encode_bmp` / `decode_dib` /
  `encode_dib` API plus crate-local `BmpImage` / `BmpPixelFormat` /
  `BmpError` types. The `Decoder` / `Encoder` trait surface and the
  container registration stay behind the `registry` feature.
- `encode_bmp` / `encode_dib` signatures now take a `&BmpImage`
  (carrying width, height, format inline). New `encode_bmp_plane` /
  `encode_dib_plane` helpers expose the underlying plane-based API.
  `decode_bmp` / `decode_dib` now return `BmpImage` instead of
  `oxideav_core::VideoFrame`. Compatibility wrappers
  `decode_bmp_videoframe` / `decode_dib_videoframe` /
  `encode_bmp_videoframe` / `encode_dib_videoframe` (registry-gated)
  preserve the previous `oxideav_core::VideoFrame`-shaped API for
  consumers like `oxideav-ico` mid-migration.
- Dropped the unused `oxideav-pixfmt` dependency.

## [0.1.2](https://github.com/OxideAV/oxideav-bmp/compare/v0.1.1...v0.1.2) - 2026-05-03

### Other

- cargo fmt: pending rustfmt cleanup
- replace never-match regex with semver_check = false
- migrate to centralized OxideAV/.github reusable workflows
- adopt slim VideoFrame/AudioFrame shape
- pin release-plz to patch-only bumps

## [0.1.1](https://github.com/OxideAV/oxideav-bmp/compare/v0.1.0...v0.1.1) - 2026-04-25

### Other

- drop oxideav-codec/oxideav-container shims, import from oxideav-core
- release v0.0.1

## [0.1.0](https://github.com/OxideAV/oxideav-bmp/compare/v0.0.1...v0.1.0) - 2026-04-19

### Other

- promote to 0.1.0

### Added

- Initial release: pure-Rust BMP (Windows bitmap) decoder + encoder +
  container.
- Decode: 1 / 4 / 8 / 16 / 24 / 32-bit `BI_RGB`, 16 / 32-bit
  `BI_BITFIELDS`. Palette expansion on indexed depths. Bottom-up and
  top-down row orders. `BITMAPINFOHEADER` v3 / v4 / v5.
- Encode: always 32-bit BGRA `BI_RGB` (preserves alpha, no
  `BI_BITFIELDS` negotiation).
- Headerless **DIB** helpers (`decode_dib` / `encode_dib`) used by
  `.ico` / `.cur` sub-images: optional `double_height + 1bpp AND
  mask` layout driven by the RGBA alpha channel.
- Container + codec registration matching every other image sibling.
