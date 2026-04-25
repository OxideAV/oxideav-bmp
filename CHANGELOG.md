# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
