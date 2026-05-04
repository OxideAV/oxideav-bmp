# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
