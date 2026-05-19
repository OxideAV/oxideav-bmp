# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
