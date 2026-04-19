# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
