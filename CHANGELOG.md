# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed

- *(encode)* **Full-width 32-bit bitfields mask no longer overflow-panics**
  (round 366, found by the `bitfields_roundtrip` fuzz target): a
  `BmpBitfields` channel mask of `0xFFFF_FFFF` is a contiguous bit run
  and must validate, but `BmpBitfields::validate`'s contiguity check
  computed `(mask >> tz) + 1`, which overflowed `u32` (panic "attempt to
  add with overflow") when the normalised mask was `u32::MAX`. The check
  now uses `wrapping_add(1)` so a full-width mask wraps to `0` and is
  correctly accepted. This restores the daily `bitfields_roundtrip` fuzz
  job to green; new lib regression test pins the validate + encode +
  decode path.
- *(decode)* **Top-down (negative `biHeight`) RLE bitmaps are rejected**
  (round 366): the `BITMAPINFOHEADER` remarks require that "for
  compressed formats, `biHeight` must be positive, regardless of image
  orientation" — an RLE stream describes a bottom-up scan whose
  end-of-line / delta / end-of-bitmap escapes have no defined meaning
  under a top-down layout. A `BI_RLE8` / `BI_RLE4` bitmap with a negative
  `biHeight` is malformed; the decoder now returns a precise error
  instead of silently decoding the `|height|` rows as if they were
  bottom-up (which produced a vertically-mirrored, garbage image). New
  lib tests cover the RLE8 and RLE4 cases (lib +2 = 182).
- *(decode)* **RLE skipped pixels resolve to colour index 0** (round 366):
  an RLE bitmap is an *indexed* image, so the cells the stream never
  writes — the ones a `delta` jumps over, the tail of a short row past an
  end-of-line escape, and everything past an early end-of-bitmap — take
  **colour index 0** (the first colour-table entry), exactly like an
  explicitly-coded index-0 pixel. The `BI_RLE8` / `BI_RLE4` decoders
  previously left every untouched cell as transparent black
  (`[0,0,0,0]`), so a bitmap whose index 0 is a non-`(0,0,0)` colour — or
  any palette where index 0 should decode opaque — came out wrong on
  every skipped cell (wrong colour *and* `alpha = 0` instead of `0xFF`).
  Both RLE rows are now pre-filled with the RGBA of palette index 0 (the
  *Bitmap Compression* material defines the escape semantics but is
  silent on the fill colour; Windows fills index 0, the canonical
  background). New lib tests cover the immediate end-of-bitmap fill, a
  `delta`-jump skip, a short-row-after-end-of-line tail, and the RLE4
  analogue (lib +4 = 180).

### Added

- *(encode)* **Explicit-mask `BI_BITFIELDS` / `BI_ALPHABITFIELDS`
  encoder** (round 342): `encode_bmp_bitfields` /
  `encode_bmp_plane_bitfields` emit a bit-field BMP using the classic
  Windows in-file mask layout — a 40-byte `BITMAPINFOHEADER` (V3)
  followed by a 12-byte (R/G/B) or 16-byte (R/G/B/A) DWORD mask tail
  immediately after the header — distinct from the V4/V5 in-header mask
  block the `Rgb565` path emits. The new `BmpBitfields` struct carries
  the four channel masks plus bit depth and ships five presets: `RGB565`,
  `RGB555`, `ARGB1555` (16-bpp), `BGRA8888`, `BGRX8888` (32-bpp).
  `validate()` enforces single-run, non-overlapping, in-range masks.
  Byte-aligned 32-bpp masks round-trip bit-exact through `decode_bmp`
  (alpha for `BGRA8888`, opaque for `BGRX8888`); `Rgb24` sources
  synthesise a full-scale alpha run. `top_down` is honoured.
- *(decode)* **OS/2 file-magic recognition** (round 342): the five
  OS/2-era container signatures (`BA` bitmap array, `CI` colour icon,
  `CP` colour pointer, `IC` icon, `PT` pointer) are now classified by the
  new public `BmpFileMagic` enum (plus `OS2_MAGIC_BA` … `OS2_MAGIC_PT`
  constants) and rejected with a precise `Unsupported` error naming the
  two-char signature instead of the generic `InvalidData("missing
  'BM'")`. Unrecognised words keep the historical message. Archive
  walking / hotspot parsing stay docs-blocked — recognise-and-reject
  only.
- *(fuzz)* **`bitfields_roundtrip` target** (round 342): exercises the
  explicit-mask encoder with fuzzer-controlled masks + pixels, asserting
  the no-panic contract plus the exact round-trip for the byte-aligned
  32-bpp presets. Also covered `Indexed2` in the `encode_roundtrip`
  harness's match arms + selector, restoring the scheduled-Fuzz build.
- *(encode)* **Windows CE 2-bit/pixel indexed output** (round 334): the
  new `BmpPixelFormat::Indexed2` encode format is the symmetric
  counterpart of the round-330 2-bpp decoder. Input is one byte per
  pixel carrying a 0..=3 index (only the low two bits are read); the
  encoder packs four pixels per byte with the left-most pixel in the
  two most-significant bits and emits a plain 40-byte V3 `BI_RGB`
  header with a 4-entry colour table (BMP defines no RLE flavour at
  2 bpp, so the output is always uncompressed). `top_down` (negative
  `biHeight`) and `minimal_palette` (trim the on-disk table, clamped to
  `[1, 4]`, with the exact `biClrUsed`) are honoured, matching the
  `Indexed1` / `Indexed4` / `Indexed8` arms; the headerless `encode_dib`
  path and the V4-calibrated / V5 + ICC colour-managed indexed encoders
  accept it too. A new `EncodedBmpFormat::Indexed2` variant reports the
  chosen on-disk shape. The 2-bpp pixels round-trip bit-exact through
  `decode_bmp`.
- *(decode)* **Windows CE 2-bit/pixel indexed depth** (round 330): the
  `2` bpp pixel format documented for Windows CE is now decoded — four
  pixels pack per byte with the left-most pixel in the two
  most-significant bits, each a 2-bit index into a 4-entry colour table
  (`biClrUsed = 0` resolves to the full `2^2 = 4` entries). Bottom-up
  and top-down (negative `biHeight`) row orders both decode to top-down
  `Rgba`. The depth is now accepted alongside `1 / 4 / 8 / 16 / 24 / 32`
  by the decoder's bit-depth guards and the typed `BitmapInfoHeader` /
  `DibHeader` `palette_entries()` accessors.
- *(decode)* **`bfOffBits` recovery for minimal / corrupt writers**
  (round 327): when the `BITMAPFILEHEADER` `bfOffBits` field is `0`
  (left unset by minimal encoders) or points implausibly early — inside
  the DIB header or colour table — the decoder now recovers the
  canonical pixel-array offset (file header → DIB header → bit-field mask
  block → colour table → pixels, per the spec's *Bitmap Storage* layout)
  instead of reading header / palette bytes as pixels or failing the
  truncation check. A `bfOffBits` at or past the canonical position is
  still honoured verbatim, so a writer's deliberate gap between the
  colour table and the pixels survives. Both `decode_bmp` and
  `decode_bmp_with_metadata` share the resolution; the
  header→masks→colour-table offset maths is now a single
  `canonical_dib_pixel_offset` helper (previously duplicated across the
  three `decode_dib*` entry points).
- *(decode)* **CMYK compression family recognised by name** (round 322):
  the `BI_CMYK` (11), `BI_CMYKRLE8` (12), and `BI_CMYKRLE4` (13)
  "Windows Metafile CMYK" compression values now have public constants
  and are rejected at the decode boundary with a distinct, named error
  (`BMP: CMYK (BI_CMYK) not supported`, etc.) instead of the generic
  `unknown compression {n}` path. The WMF-defined CMYK channel layout
  and CMYK→RGB conversion are outside this crate's BMP docs, so full
  decode stays blocked; a CMYK bitmap is now reported as a
  known-but-unsupported format rather than looking like a corrupt header.
- *(encode)* colour-managed encode paths now accept `Rgb555` (16-bit
  `BI_RGB` 5-5-5): `encode_bmp_with_icc_profile` (V5 PROFILE_EMBEDDED),
  `encode_bmp_with_linked_icc_profile` (V5 PROFILE_LINKED), and
  `encode_bmp_with_calibrated_rgb` (V4 LCS_CALIBRATED_RGB) — emitted as
  plain `BI_RGB` with no bitfields mask block (high bit reserved)

### Fixed

- *(decode)* **32-bit `BI_RGB` alpha on V4 / V5 headers** (round 318):
  the BMP spec treats the 32-bit alpha sample as valid "whenever the
  alpha mask is present in the DIB header" — the R / G / B masks are
  valid only under `BI_BITFIELDS`, but the alpha mask in the V4 / V5
  in-header four-mask block applies even under `BI_RGB`. The decoder now
  honours a V4 / V5 in-header alpha mask on a 32-bit `BI_RGB` bitmap:
  a non-zero mask extracts the alpha sample through the mask (so a
  non-canonical alpha-mask position decodes correctly, not just the
  high-byte ARGB layout), while a zero alpha mask yields opaque output
  (the same zero-mask → opaque convention `BI_ALPHABITFIELDS` and the
  V3 alpha path already use). This fixes the otherwise-transparent
  decode of a V4 / V5 `BI_RGB` bitmap whose reserved high bytes are
  zero. The plain 40-byte `BITMAPINFOHEADER` (V3) `BI_RGB` path is
  deliberately unchanged — it has no in-header alpha-mask slot, so it
  keeps reading the reserved high byte as alpha (the behaviour the
  crate's own 32-bit BGRA encoder relies on for a lossless RGBA
  round-trip).

### Changed

- *(encode)* the V4 / V5 colour-managed encode paths
  (`encode_bmp_with_icc_profile`, `encode_bmp_with_linked_icc_profile`,
  `encode_bmp_with_calibrated_rgb`) now write the canonical
  `0xFF000000` alpha mask into the header's four-mask region for
  32-bit `Rgba` input (the format stays `BI_RGB`; R / G / B keep the
  default BGRA byte order with their masks zero). Previously the alpha
  was written into the reserved high byte with a zero alpha mask, which
  a strict reader would discard; the emitted file is now a
  spec-correct alpha-carrying V4 / V5 bitmap whose alpha the decoder
  (this crate's and others') recovers through the mask. The on-disk
  R / G / B bytes are unchanged.

## [0.1.6](https://github.com/OxideAV/oxideav-bmp/compare/v0.1.5...v0.1.6) - 2026-06-15

### Added

- *(encode)* 16-bit BI_RGB 5-5-5 encode (BmpPixelFormat::Rgb555)
- surface full 64-byte OS/2 2.x OS22XBITMAPHEADER trailing fields

### Other

- add `metadata` target over decode_*_with_metadata (round 300)
- V4 calibrated-RGB path (encode_bmp_with_calibrated_rgb)
- single-allocation flat-buffer uncompressed decode (r286 profile-opt)
- truncated OS/2 2.x OS22XBITMAPHEADER decode (biSize 16..40) (r275)
- typed BitmapInfoHeader parser + DibHeaderKind biSize discrimination (r268)
- typed BitmapFileHeader parser + accessors + roundtrip (r265)
- BITMAPV2INFOHEADER (52 B) + BITMAPV3INFOHEADER (56 B) (r261)
- surface V3+ device-resolution + palette-count fields
- drop release-plz.toml — use release-plz defaults across the workspace
- typed BmpIccProfileRef accessor on V5 + linked-path field (r237)
- V5 + ICC profile encode now accepts indexed input (r231)
- V5 + ICC profile encode now accepts Rgb565 (BI_BITFIELDS) (r225)
- V5 PROFILE_LINKED encode side (encode_bmp_with_linked_icc_profile) (r210)
- V4/V5 colour-space + embedded ICC profile decode/encode (r205)

### Added

- **16-bit `BI_RGB` 5-5-5 encode (`BmpPixelFormat::Rgb555`)** (round 310):
  a new encode-only pixel format that closes the encode/decode symmetry
  for the canonical 16-bpp BMP form. The decoder has always read 16-bit
  `BI_RGB` as RGB 5-5-5 (high bit reserved, R in bits 14..10, G in bits
  9..5, B in bits 4..0); per the documented rule "for 16-bpp bitmaps, if
  `biCompression` equals `BI_RGB`, the format is always RGB 555," that
  layout is unambiguous and needs no `BI_BITFIELDS` mask block. The
  encoder now emits it with a plain 40-byte `BITMAPINFOHEADER`
  (`biBitCount = 16`, `biCompression = BI_RGB`, `bfOffBits = 14 + 40` —
  no colour table, no mask tail). Input is one little-endian 5-5-5 `u16`
  per pixel (the same packed wire shape `Rgb565` already accepts); the
  packer only re-strides to the 4-byte-aligned on-disk row pitch.
  `BmpEncodeOptions::top_down` (negative `biHeight`) is honoured, and the
  headerless DIB helpers (`encode_dib` / `encode_dib_plane`) accept the
  new format too (no ICO AND-mask, since a 5-5-5 word carries no alpha).
  A new `EncodedBmpFormat::Rgb16Rgb` token is returned. The V4/V5 + ICC /
  calibrated-RGB encode paths continue to route 16-bit colour through the
  `Rgb565` `BI_BITFIELDS` arm (those headers carry their masks in the
  body), so `Rgb555` there returns `Unsupported` as before for any other
  format. Four new lib tests cover the V3 header shape + opaque-red
  roundtrip, the top-down vertical-gradient row order, the
  truncated-plane error, and the headerless DIB roundtrip (lib total 137).

- **Full 64-byte OS/2 2.x `OS22XBITMAPHEADER` trailing-field metadata**
  (round 306): the 24 bytes that the full IBM header appends after the
  40-byte `BITMAPINFOHEADER` prefix — `usUnits` (offset 40),
  `usRecording` / fill-direction (offset 44), `usRendering` / halftoning
  algorithm (offset 46), the two halftoning parameters `cSize1` /
  `cSize2` (offsets 48 / 52), `ulColorEncoding` (offset 56) and the
  application `ulIdentifier` (offset 60) — are now surfaced through a new
  `BmpMetadata::os2_header2: Option<BmpOs2Header2>` field. It is `Some`
  only when the decoded DIB header is exactly 64 bytes (every Windows
  generation, the 12-byte OS/2 1.x `BITMAPCOREHEADER`, and the truncated
  OS/2 2.x forms report `None`). The halftoning algorithm is mapped to a
  new `BmpOs2Halftone` enum (`None` / `ErrorDiffusion` / `Panda` /
  `SuperCircle` / `Unknown(u16)` passthrough); convenience predicates
  `units_is_pels_per_meter()` / `is_bottom_up()` /
  `color_encoding_is_rgb()` test each `0`-valued documented default while
  the raw values stay readable so a non-standard write is distinguishable
  from the default. Pixel decode is unchanged — the trailing block is
  metadata only and a 64-byte header stays below the 108-byte V4
  colour-space threshold (so `color_space` / `rendering_intent` remain
  `None`). New named constants `OS2_UNITS_PELS_PER_METER` /
  `OS2_RECORDING_BOTTOM_UP` / `OS2_COLOR_ENCODING_RGB` /
  `OS2_HALFTONE_{NONE,ERROR_DIFFUSION,PANDA,SUPER_CIRCLE}` and the
  `OS22XBITMAPHEADER_SIZE` size constant are now re-exported. Six new
  tests cover pixel decode, the all-zero defaults, each halftoning
  algorithm, non-standard passthrough values, and the `None` result for
  CORE / truncated headers.

- **`metadata` fuzz target** (round 300, depth round): a fourth
  `cargo-fuzz` harness in `fuzz/` that feeds arbitrary bytes to the
  `decode_bmp_with_metadata` / `decode_dib_with_metadata` entry points.
  These are independent public surfaces with their own
  attacker-controlled offset / slicing maths that the pixel-only
  `decode` target never reaches: the V4 colour-space tail (`bV4CSType`,
  the nine-`i32` `CIEXYZTRIPLE` endpoints, the three-`u32` gamma
  triple), the V5 colour-management tail (`bV5Intent` /
  `bV5ProfileData` / `bV5ProfileSize`), and the trailing ICC /
  linked-path blob slice `input[base + bV5ProfileData ..][.. size]`
  where both the offset and the size are fuzzer-controlled `u32`
  fields. Both DIB framings (plain + doubled-height XOR+AND) are fuzzed
  so the slice base (14 for a BMP file, 0 for a header-less DIB)
  varies. Five seed inputs (plain V3, V4 calibrated-RGB, V5 embedded
  ICC on direct-colour and indexed images, V5 linked ICC) live in
  `fuzz/corpus/metadata/`. A 60-second local run lands ~1.08 M inputs
  with zero crashes; the harness builds against the framework-free
  standalone path (`default-features = false`) and shares the same
  panic-free contract as the other three targets.

- **`encode_bmp_with_calibrated_rgb` — V4 calibrated-RGB encode**
  (round 294): a new public entry point that emits a 108-byte
  `BITMAPV4HEADER` with `bV4CSType = LCS_CALIBRATED_RGB` plus
  caller-supplied CIE endpoints (`[i32; 9]` `CIEXYZTRIPLE`, packed
  R.x R.y R.z G.x G.y G.z B.x B.y B.z) and a per-channel gamma triple
  (`[u32; 3]`, unsigned 16.16 fixed point). This is the colour-space
  counterpart to the existing `encode_bmp_with_icc_profile` /
  `encode_bmp_with_linked_icc_profile` V5 paths: where those declare
  an embedded or linked ICC profile, the calibrated path bakes the
  endpoint + gamma fields directly into the V4 header. Supported pixel
  formats: `Rgba` (32-bit BGRA `BI_RGB`), `Rgb24` (24-bit BGR
  `BI_RGB`), `Rgb565` (16-bit `BI_BITFIELDS` 5-6-5, masks in the V4
  four-mask region), and the indexed `Indexed8` / `Indexed4` /
  `Indexed1` (uncompressed `BI_RGB`, colour table between the header
  and the pixel array). RLE is never chosen so the header shape stays
  deterministic; `BmpEncodeOptions::top_down` (negative `biHeight`)
  and `minimal_palette` are honoured on every arm. The decoder
  round-trips the header: `decode_bmp_with_metadata` reports
  `BmpColorSpace::Calibrated` and returns the same `endpoints` +
  `gamma_rgb` the encoder was given. Eight new roundtrip tests cover
  the direct-colour, 5-6-5 masks-in-header, top-down, indexed (full +
  minimal palette), missing-palette error, and zero-endpoint
  tag-only cases.

### Performance

- **Single-allocation flat-buffer uncompressed decode** (round 286,
  profile-opt depth round): the uncompressed pixel-unpack path
  (`decode_pixels`, all of 1 / 4 / 8 / 16 / 24 / 32 bpp) previously
  built a `Vec<Vec<u8>>` — one heap allocation per scanline — pushed
  each pixel with `extend_from_slice(&[r, g, b, a])`, and then the
  caller reversed the row vector and concatenated it into one flat
  plane: three passes over the pixels plus `height + 1` separate
  allocations. The decoder now allocates the destination RGBA plane
  once and writes each source scanline straight to its final top-down
  position (the bottom-up flip is resolved by an index map instead of
  a later `rev()` + concat), writing pixels through a
  `chunks_exact_mut(4)` cursor so there is no per-pixel capacity check
  and no second copy. For 16 bpp `BI_BITFIELDS`, images at or above
  2^18 pixels precompute the full 65 536-entry value→RGBA table once
  and replace the four per-pixel mask `expand()` calls with a single
  indexed load; smaller images keep the direct per-pixel path so an
  icon never pays the table-build cost. Output bytes are bit-identical
  (FNV-1a-verified across the 32-bit BGRA, 24-bit BGR, 16-bit 5-6-5
  bitfields, 8-bit and 4-bit indexed fixtures, before vs. after). On a
  best-of-5 wall-clock harness the per-format decode dropped: 32 bpp
  320×240 56→9.6 µs (≈5.9×), 24 bpp 640×480 253→32 µs (≈8×), 8-bit
  indexed 320×240 58→28 µs (≈2.1×), 4-bit 68→48 µs (≈1.4×), 16 bpp
  5-6-5 640×480 ≈193 µs via the LUT; the RLE paths are unchanged. The
  full uncompressed corpus sum fell ≈710→≈380 µs (≈1.86×). All 159
  existing tests continue to pass with no decoded-byte changes.

### Added

- **Truncated OS/2 2.x `OS22XBITMAPHEADER` decode (`biSize` 16…39 B)**
  (round 275): the OS/2 2.x header (`BITMAPINFOHEADER2` in IBM's
  documentation) shares the 40-byte `BITMAPINFOHEADER` field layout
  and may legally be truncated anywhere from 16 bytes upward with the
  remaining fields read as zero — the canonical 16-byte form
  (`biSize` / width / height / planes / bit-count only) is exercised
  by the BMP Suite's `pal8os2v2-16.bmp`. The decoder now accepts these
  sizes: it reads each field only when the declared `biSize` is long
  enough to contain it and defaults the rest to zero. Unlike the
  12-byte OS/2 1.x `BITMAPCOREHEADER`, the truncated OS/2 2.x header
  uses 4-byte signed width/height (so a negative `biHeight` selects
  top-down rows) and 4-byte `RGBQUAD` palette entries. A truncated
  header has no room for the appended bitfield-mask block, so
  `BI_BITFIELDS` (the OS/2 `Huffman 1D` alias) and `BI_JPEG` (the OS/2
  `RLE-24` alias) are rejected on these sizes; only plain `BI_RGB` /
  `BI_RLE8` / `BI_RLE4` streams decode. The full 64-byte
  `OS22XBITMAPHEADER` continues to decode through the shared
  `biSize >= 40` INFO path on its 40-byte prefix. Four new lib tests
  cover the 16-byte form, every intermediate truncation point
  (20/24/28/32/36 B), the signed-height top-down case, and the
  `BI_BITFIELDS`-rejection. The `malformed_inputs` size-rejection
  sweep was narrowed to the genuinely-illegal sizes (below 12 and the
  13…15 gap) now that 16…39 are legal.

- **Typed `BitmapInfoHeader` parser + `DibHeaderKind` discriminator**
  (round 268): the 40-byte `BITMAPINFOHEADER` that opens every
  V3-and-later DIB now has a dedicated public struct mirroring the
  eleven documented fields at their on-disk offsets (`header_size` /
  `width` / `height` / `planes` / `bit_count` / `compression` /
  `image_size` / `x_pels_per_meter` / `y_pels_per_meter` / `clr_used`
  / `clr_important`), completing the typed-header pair started by
  r265's `BitmapFileHeader`. Entry points mirror the file-header
  shape: `from_bytes` (unchecked, `None` on a sub-40-byte buffer),
  `parse` (validates buffer length + the `biSize` discrimination the
  header-types doc prescribes — `biSize = 12` is rejected with a
  dedicated message since the `WORD`-based `BITMAPCOREHEADER` layout
  would read back as garbage through INFO offsets; other sub-40 sizes
  are rejected as unsupported; `>= 40` is accepted since the V2 / V3
  / V4 / V5 generations and odd in-the-wild sizes such as the OS/2
  2.x 64-byte variant all carry the 40-byte INFO prefix), and
  `to_bytes` (deterministic 40-byte render). Accessors:
  `kind() -> Option<DibHeaderKind>`, `is_top_down()` (negative
  `biHeight`), `absolute_width()` / `absolute_height()`,
  `row_stride()` (the documented `((w*bpp + 31) & !31) >> 3`
  formula), `palette_entries()` (`biClrUsed` with the `0 = 2^bpp`
  sentinel applied on indexed depths), and the informational
  `planes_is_valid()` (`biPlanes == 1`, "must be set to 1"). The new
  `DibHeaderKind` enum maps the six known `biSize` generations
  (12 / 40 / 52 / 56 / 108 / 124 → `Core` / `Info` / `V2Info` /
  `V3Info` / `V4` / `V5`) with `from_size` / `size` / the
  `has_info_prefix()` predicate (everything but `Core` shares the
  INFO field layout as a prefix). The decoder's `parse_dib_header`
  now reads the eleven base fields through the typed struct so the
  offsets live in one place; the extended mask / colour-space tails
  stay on the wide `DibHeader` and every error message is unchanged.
  Lib tests (+21 = 115; crate total 155): layout/offset render,
  byte roundtrips (including dirty values), short-buffer `None` /
  `Err`, CORE-size + sub-40 rejection, extended + unknown `>= 40`
  acceptance, planes / top-down / stride / palette accessor
  semantics, the Bitmap Storage doc's Redbrick.bmp worked example
  (32×32 4-bpp header bytes parse to the documented values, 512-byte
  index array reconstructed from `row_stride()`), `DibHeaderKind`
  mapping exhaustiveness + roundtrip, and an encoder/decoder
  agreement check on a real encoded BMP. Re-exported from the crate
  root alongside `BitmapFileHeader` / `DibHeader`.

- **Typed `BitmapFileHeader` parser** (round 265): the 14-byte
  `BITMAPFILEHEADER` that prefixes every real `.bmp` file now has a
  dedicated public struct with named accessors for each on-disk
  field (`file_type` / `file_size` / `reserved1` / `reserved2` /
  `pixel_offset`) instead of the inline `read_u16_le` / `read_u32_le`
  pokes the decoder open-coded historically. Three entry points are
  exposed: `BitmapFileHeader::from_bytes` (unchecked field read,
  returns `None` on a short buffer; suitable for fuzz / probe
  consumers), `BitmapFileHeader::parse` (validates buffer length +
  the `0x4D42` `bfType` signature), and `BitmapFileHeader::to_bytes`
  (renders the deterministic 14-byte layout for encoder consumers).
  Two informational predicates round out the surface:
  `has_canonical_magic()` distinguishes `"BM"` from the OS/2-era
  alternates (`BA` array, `CI` colour icon, `CP` colour pointer)
  the decoder doesn't accept, and `reserved_is_clean()` reports
  whether `bfReserved1` / `bfReserved2` are both zero (the spec
  says they "must be zero" but real-world writers leak garbage
  there). The decoder's `decode_bmp` and `decode_bmp_with_metadata`
  entry points now funnel through `BitmapFileHeader::parse`, so the
  validated `bfType` check and the "shorter than header" error
  string used historically are now reachable from a single source.
  Re-exported from the crate root alongside the existing typed
  `DibHeader`.

- **`BITMAPV2INFOHEADER` (52 B) + `BITMAPV3INFOHEADER` (56 B) decode**
  (round 261): the two Adobe-published intermediate DIB header
  generations that sit between `BITMAPINFOHEADER` (40 B) and
  `BITMAPV4HEADER` (108 B) now decode through the same mask-driven
  16 / 32-bpp path the V4 / V5 case uses. V2 carries the R / G / B
  bit masks **inside** the header body at offsets 40 / 44 / 48 (no
  trailing 12-byte mask tail between header and pixel array); V3
  adds the 4-byte alpha mask at offset 52 (matching the slot V4 / V5
  use). The full V4 colour-space tail — `bV4CSType`, the
  `CIEXYZTRIPLE` endpoints, the R / G / B gamma triple — is absent
  on both intermediate headers, so the metadata path returns
  `color_space = None` / `endpoints = None` / `gamma_rgb = None` /
  `rendering_intent = None` while the V3-tail fields the 40-byte
  `BITMAPINFOHEADER` already carries
  (`pixels_per_meter_x` / `pixels_per_meter_y` / `colors_used` /
  `colors_important`) stay readable since V2 inherits every byte
  24..40 from the base header. New public size constants
  `BITMAPV2INFOHEADER_SIZE = 52` and `BITMAPV3INFOHEADER_SIZE = 56`
  re-exported from the crate root alongside the existing
  `BITMAPV4HEADER_SIZE` / `BITMAPV5HEADER_SIZE`. Lib tests (+4 = 71):
  `v2_info_header_52b_bitfields_32bpp_decodes` exercises the V2
  three-mask path against canonical BGRA-style masks and a
  0xAABBCCDD payload;
  `v3_info_header_56b_bitfields_32bpp_decodes_with_alpha` covers the
  V3 four-mask path with a populated alpha mask;
  `v3_info_header_56b_zero_alpha_mask_yields_opaque` covers the
  documented zero-alpha-mask → opaque convention (same as V3
  `BI_ALPHABITFIELDS` and V4 / V5);
  `v2_info_header_52b_metadata_reports_header_size` asserts the
  metadata builder reports `header_size = 52`, leaves every V4 / V5
  colour-management field at `None`, and surfaces the V3 DPI +
  palette-count fields as inherited from `BITMAPINFOHEADER`.
- **V3+ device-resolution + palette-count metadata surfaced on
  `BmpMetadata`** (round 255): the four V3-and-later
  `BITMAPINFOHEADER` fields that pre-date the V4/V5 colour-management
  tail — `biXPelsPerMeter`, `biYPelsPerMeter`, `biClrUsed`,
  `biClrImportant` — are now exposed via
  `BmpMetadata::pixels_per_meter_x` / `pixels_per_meter_y` /
  `colors_used` / `colors_important`, each as `Option<i32>` /
  `Option<u32>`. `None` distinguishes the OS/2 12-byte
  `BITMAPCOREHEADER` case (the fields don't exist there) from V3+
  headers that set the field to the documented `0` sentinel
  ("resolution unknown" / "all colours important"). Two named
  helpers `BmpMetadata::dpi_x()` / `dpi_y()` convert the
  pels-per-metre values to dots-per-inch using one inch = 0.0254 m
  exactly, rounded to the nearest integer; both return `None` for
  the `0` sentinel and for semantically-invalid negative inputs so
  a misencoded file can't yield a nonsensical "0 DPI" or negative
  DPI downstream. Surfaced through both `decode_bmp_with_metadata`
  and `decode_dib_with_metadata` (the metadata builder reads from
  the already-parsed `DibHeader` so no pixel-pipeline changes were
  needed). Lib tests (+5 = 67):
  `v3_metadata_surfaces_resolution_and_palette_fields` exercises a
  hand-built 72-DPI / 144-DPI V3 fixture against the dot-per-inch
  conversion + the four raw fields;
  `v3_metadata_zero_resolution_returns_none_dpi` covers the
  documented `0` sentinel (raw fields stay `Some(0)`, `dpi_*`
  returns `None`);
  `v3_metadata_rejects_negative_pels_per_meter` covers misencoded
  negative inputs;
  `os2_bitmapcoreheader_metadata_has_no_resolution_fields` covers
  the OS/2 V1 fall-back to `None`; and
  `v5_metadata_inherits_resolution_fields` confirms V5 headers
  carry the same fields at the same DIB-relative offsets as V3.
  All metadata changes are purely additive — `decode_bmp` /
  `decode_dib` / the `Decoder` trait-impl path are untouched.

- **Typed ICC profile accessor on V5 metadata** (round 237):
  `BmpMetadata::icc_profile_ref()` returns a new `BmpIccProfileRef<'_>`
  enum that collapses the `PROFILE_EMBEDDED` / `PROFILE_LINKED` /
  no-ICC discrimination into a single discriminated view, so callers
  no longer have to match on `color_space` and then read
  `icc_profile` / `linked_profile_path` / `profile_data_offset` /
  `profile_size` by hand. Variants: `Embedded(&[u8])`,
  `Linked(&[u8])`, `Declared { cs_type, profile_data_offset,
  profile_size }` (V5 declared a `PROFILE_*` but the trailing-slot
  bytes were unreachable), and `None` (no V5 ICC reference). The
  `PROFILE_LINKED` path bytestring is now also surfaced through a new
  named field `BmpMetadata::linked_profile_path: Option<Vec<u8>>` in
  parallel to `icc_profile` for the embedded variant; the decoder
  still never opens the file the path points at. Lib test:
  `v5_icc_profile_ref_discriminates_embedded_linked_and_absent`
  (+1 lib = 62) covers Embedded, Linked, None (V3), and lying-offset
  Declared{...} outcomes; both `decode_bmp_with_metadata` and
  `decode_dib_with_metadata` populate the new linked-path field. The
  private V5-trailing-slot extractor was renamed from
  `read_embedded_icc` to `read_profile_slot` to reflect that the same
  byte-extraction shape serves both the embedded and linked variants
  (the `bV5CSType` discriminator decides which named field receives
  the bytes).

- **V5 + ICC profile encode side now accepts indexed input** (round 231):
  `encode_bmp_with_icc_profile` and `encode_bmp_with_linked_icc_profile`
  grow three new arms — `Indexed8`, `Indexed4`, `Indexed1` — closing the
  last "Unsupported" gap on the V5 + ICC paths. The encoder emits a
  124-byte `BITMAPV5HEADER` with `biCompression = BI_RGB`, writes the
  caller-supplied `BmpPalette` between the V5 header and the pixel array
  (so `bfOffBits = 14 + 124 + entries × 4`), sets `biClrUsed` from the
  palette (honouring `BmpEncodeOptions::minimal_palette` to trim the
  on-disk colour table to exactly the entries the caller supplied), and
  parks the ICC blob (`PROFILE_EMBEDDED`) or path-string blob
  (`PROFILE_LINKED`) at `bV5ProfileData` immediately after the pixel
  array — same DIB-relative offset shape as the direct-colour V5 paths.
  RLE is never chosen on the V5 paths since the BMP spec doesn't define
  how an RLE pixel stream and a trailing colour-management blob co-exist
  on disk. `top_down` (negative `biHeight`) is honoured on every indexed
  arm. The decoder side needs no change: it already resolves indexed V5
  BMPs through the existing palette + pixel-array path and surfaces the
  ICC / linked-path blob through the existing `BmpMetadata` shape.
  Internally a new `BmpPixelFormat::is_indexed()` helper plus a private
  `encode_bmp_v5_indexed_with_profile_blob` + a paired
  `write_dib_header_v5_indexed_with_profile` writer carry the new layout.
  Lib tests: `v5_embedded_icc_indexed8_roundtrips`,
  `v5_embedded_icc_indexed8_minimal_palette`,
  `v5_embedded_icc_indexed8_top_down_roundtrips`,
  `v5_embedded_icc_indexed4_path`, `v5_embedded_icc_indexed1_path`,
  `v5_linked_icc_indexed8_path`,
  `v5_linked_icc_indexed4_minimal_palette_path` (+7 lib = 61). The two
  pre-existing `v5_*_rejects_unsupported_format` tests were rewritten in
  the same commit to assert the still-valid contract (indexed input
  without a palette returns `InvalidData`), since "indexed = rejected"
  is no longer the right premise. No new dependencies; works in both
  `registry` and standalone (`default-features = false`) builds.

- **V5 + ICC profile encode side now accepts `Rgb565`** (round 225):
  both `encode_bmp_with_icc_profile` and
  `encode_bmp_with_linked_icc_profile` grow a third direct-colour arm
  alongside the existing `Rgba` (32-bit BGRA) and `Rgb24` (24-bit BGR)
  paths. The new arm emits a 124-byte `BITMAPV5HEADER` with
  `biCompression = BI_BITFIELDS` and writes the canonical R=0xF800 /
  G=0x07E0 / B=0x001F masks into the V5 four-mask region at offsets
  40..56 — the same slot V4 / V5 headers always use for masks, so no
  separate 12-byte mask tail is written between the header and the
  pixel array. The trailing slot still carries the ICC blob
  (`PROFILE_EMBEDDED`) or the path-string blob (`PROFILE_LINKED`)
  byte-for-byte identical to the `Rgba` / `Rgb24` arms; `top_down`
  (negative `biHeight`) is honoured. Internally
  `write_dib_header_v5_with_profile` was generalised to take
  `(compression, masks)` parameters instead of hard-coding
  `(BI_RGB, [0; 4])`, with a new `RGB565_MASKS_V5` constant carrying
  the canonical 5-6-5 quadruple. Indexed input still returns
  `BmpError::Unsupported` on both V5 paths since threading a colour
  table through a V5 layout would need a wider rewrite. Lib tests:
  `v5_embedded_icc_rgb565_path`,
  `v5_embedded_icc_rgb565_top_down_roundtrips`,
  `v5_linked_icc_rgb565_path` (+3 lib = 54). No new dependencies;
  works in both `registry` and standalone
  (`default-features = false`) builds.

- **V5 `PROFILE_LINKED` encode side** (round 210): new
  `encode_bmp_with_linked_icc_profile(image, linked_path, intent, options)`
  closes the encode/decode symmetry for the `LCS_PROFILE_LINKED`
  colour-space tag introduced on the decode side in r205. The encoder
  writes the same 124-byte `BITMAPV5HEADER` shape as
  `encode_bmp_with_icc_profile` but with `bV5CSType = PROFILE_LINKED`
  and a caller-supplied path-string blob in the
  `bV5ProfileData` / `bV5ProfileSize` slot instead of the ICC bytes
  themselves. The path encoding is system-dependent per the BMP spec
  (typically null-terminated ANSI on Windows); the encoder surfaces
  the buffer verbatim so callers that need UTF-16 / URL transport can
  pass whatever blob they choose. Supported pixel formats
  (`Rgba` / `Rgb24`) and `top_down` handling match the embedded path;
  indexed / 16-bit input still routes through `Unsupported` for the V5
  paths until the V3 / V4 layouts grow a V5 colour-space tail.
  Internally the V5-header writer
  (`write_dib_header_v5_embedded_profile` →
  `write_dib_header_v5_with_profile`) was generalised to accept the
  `cs_type` constant rather than hard-coding `PROFILE_EMBEDDED`, so
  both encode paths share the same on-disk byte layout below the
  `cs_type` field. Lib tests:
  `v5_with_linked_icc_profile_path_surfaces`,
  `v5_linked_icc_top_down_roundtrips`, `v5_linked_icc_rgb24_path`,
  `v5_linked_icc_rejects_unsupported_format`,
  `v5_linked_icc_empty_path_still_valid` (+5 lib = 51). No new
  dependencies; works in both `registry` and standalone
  (`default-features = false`) builds.

- **V4 / V5 colour-space metadata + embedded ICC profile** (round 205):
  new `decode_bmp_with_metadata` / `decode_dib_with_metadata` entry
  points return a `(BmpImage, BmpMetadata)` pair so callers can read
  back the `bV4CSType` / `bV5CSType` colour-space tag, the
  `CIEXYZTRIPLE` endpoints, the `R/G/B` gamma triple, the V5
  rendering intent, and (for `PROFILE_EMBEDDED`) the embedded ICC
  profile blob itself. V3 (40-byte) and OS/2 12-byte headers report
  every metadata field as `None` — those header variants pre-date
  colour management. V4 (108-byte) fills `color_space` /
  `endpoints` / `gamma_rgb`; V5 (124-byte) additionally fills
  `rendering_intent`. A V5 header that declares `PROFILE_EMBEDDED`
  decodes the ICC bytes at
  `whole[BITMAPFILEHEADER_SIZE + bV5ProfileData..][..bV5ProfileSize]`
  into `BmpMetadata::icc_profile`. A V5 header that lies about its
  offset / size (slice falls past EOF) leaves `icc_profile = None`
  with the declared `profile_data_offset` / `profile_size` still
  surfaced so metadata never makes decode fail on its own. The
  existing `decode_bmp` / `decode_dib` entry points remain
  byte-for-byte compatible — the metadata path is purely additive.
  New encode side: `encode_bmp_with_icc_profile(image, icc_bytes,
  intent, options)` writes a 124-byte `BITMAPV5HEADER` with
  `bV5CSType = PROFILE_EMBEDDED` for `Rgba` / `Rgb24` input plus
  honoured `top_down`; indexed / 16-bit input is rejected with
  `BmpError::Unsupported` for now (those use V3 / V4 headers whose
  layout would need a wider rewrite to make room for a V5 tail).
  New public surface: `BmpColorSpace` (`Calibrated` / `SRgb` /
  `Windows` / `ProfileLinked` / `ProfileEmbedded` / `Unknown(u32)`),
  `BmpRenderingIntent` (`Unspecified` / `Saturation` /
  `RelativeColorimetric` / `Perceptual` / `AbsoluteColorimetric` /
  `Unknown(u32)`), `BmpMetadata`, plus constants `LCS_CALIBRATED_RGB`,
  `LCS_S_RGB`, `LCS_WINDOWS_COLOR_SPACE`, `PROFILE_LINKED`,
  `PROFILE_EMBEDDED`, `LCS_GM_BUSINESS` / `LCS_GM_GRAPHICS` /
  `LCS_GM_IMAGES` / `LCS_GM_ABS_COLORIMETRIC`. Lib tests:
  `v5_srgb_header_parses_and_decodes`,
  `v5_intent_perceptual_round_trips`,
  `v3_decode_with_metadata_has_no_v4_v5_fields`,
  `v4_rgb565_metadata_surfaces_srgb`,
  `v5_with_embedded_icc_profile_roundtrips`,
  `v5_embedded_icc_top_down_preserves_profile`,
  `v5_embedded_icc_rgb24_path`,
  `v5_embedded_icc_rejects_unsupported_format`,
  `v5_truncated_icc_does_not_panic`. No new dependencies; works in
  both `registry` and standalone (`default-features = false`)
  builds.

- **`encode_roundtrip` cargo-fuzz target** (round 198): closes the
  symmetry of the two existing decoder-side harnesses by exercising
  the encoder with fuzzer-controlled pixels / palette / encode
  options, then decoding the encoder's output back. Format selector
  + `top_down` / `minimal_palette` flags + geometry (clamped to
  1..=64 px per axis) come from the first four input bytes; the
  remainder fills the pixel plane and, for indexed formats, the
  palette tail (three bytes per `[R, G, B]` entry, padded with zeros).
  For `Rgba` and `Rgb24` the harness asserts byte-for-byte roundtrip
  equality across the encode→decode pair; `Rgb565` / `Indexed8` /
  `Indexed4` / `Indexed1` are panic-checked only (the decoder expands
  everything to 4 B/px `Rgba` so a 1 B/px index ↔ 4 B/px expanded
  comparison would be apples-to-oranges). A 60 s local run landed
  ~1.33 M iterations (~21.8 k execs/sec, peak RSS ~480 MB) with
  zero panics, OOMs, or roundtrip mismatches. Six seed inputs (one
  per pixel format) live in `fuzz/corpus/encode_roundtrip/`; the
  daily `.github/workflows/fuzz.yml` job picks the new bin up
  automatically via the org reusable workflow's `[[bin]]`
  auto-discovery.

## [0.1.5](https://github.com/OxideAV/oxideav-bmp/compare/v0.1.4...v0.1.5) - 2026-05-29

### Other

- add BI_ALPHABITFIELDS (compression value 6) support (r182)
- add 1-bit indexed (monochrome) BMP write path (r176)
- add rle_stream target for BI_RLE8 / BI_RLE4 state machines (r162)
- property tests for malformed inputs (r155 depth-mode)
- add daily fuzz.yml CI + 1-bpp corpus seed
- add criterion decode/encode/roundtrip harnesses (r129)
- add cargo-fuzz decode target + fix header-driven DoS paths
- minimal biClrUsed colour table for indexed BMP output
- OS/2 BITMAPCOREHEADER read + top-down DIB write

### Added

- **`BI_ALPHABITFIELDS` (compression value 6) decode** (round 182): the
  four-mask variant of `BI_BITFIELDS` documented for Windows CE 5.0+
  and accepted by recent Windows builds. On a V3 (40-byte)
  `BITMAPINFOHEADER` the variant appends 16 bytes of R/G/B/A masks
  immediately after the header (vs `BI_BITFIELDS`' 12 bytes of R/G/B);
  the parser reads the alpha mask too and feeds it into the existing
  16-bpp and 32-bpp mask-driven decode paths. V4/V5 headers already
  carry all four masks in the header body, so `BI_ALPHABITFIELDS` on
  those header sizes is treated identically to `BI_BITFIELDS`.
  Truncated mask tails on V3 are rejected with a dedicated error
  message; an explicit zero alpha mask falls back to opaque output to
  match the `BI_BITFIELDS` convention. The `pixel_start` computation
  in `decode_dib` was updated to count the four-mask tail toward the
  palette / pixel-array offset. Lib tests:
  `alpha_bitfields_v3_32bpp_decodes`,
  `alpha_bitfields_v3_32bpp_alpha_zero_means_transparent`,
  `alpha_bitfields_v3_truncated_masks_rejected`,
  `alpha_bitfields_v3_zero_alpha_mask_yields_opaque`,
  `alpha_bitfields_v3_16bpp_5551`. The
  `unknown_compression_is_rejected` property test was updated to drop
  compression value 6 from the rejected-values list (the value is now
  legitimately recognised). New constant: `BI_ALPHABITFIELDS = 6`
  re-exported from the crate root. No new dependencies; works in both
  `registry` and standalone builds.

- **1-bit indexed (monochrome) encode** (round 176): new
  `BmpPixelFormat::Indexed1` variant + the `EncodedBmpFormat::Indexed1`
  return token, closing the asymmetry between decode (which has always
  supported 1-bpp bitmaps via the palette path) and encode (which
  previously only emitted 4 / 8 / 16 / 24 / 32-bpp). Input is one
  byte per pixel carrying `idx & 1` (0 → black, 1 → white in the
  classic palette); the encoder MSB-first-packs the byte stream
  into the on-disk 1-bpp layout with the standard 4-byte row padding.
  Caller supplies a `BmpPalette` of up to 2 entries; the
  `minimal_palette` flag clamps written entries to `[1, 2]` and the
  one-entry case is exercised by the test suite. BMP has no RLE
  flavour at 1 bpp so this path is always emitted uncompressed
  (`BI_RGB`). The DIB helper (`encode_dib`) accepts `Indexed1` for
  callers wanting a headerless monochrome DIB; `top_down` is
  honoured (BI_RGB + signed `biHeight` is spec-legal at 1 bpp).
  Lib tests: `roundtrip_1bit_indexed`, `indexed1_packs_msb_first`,
  `indexed1_top_down_roundtrips`, `indexed1_without_palette_errors`,
  `indexed1_minimal_palette_one_entry_clamped`. No new dependencies
  and the standalone (`default-features = false`) build picks up the
  new variant automatically.

- **RLE-focused fuzz target** (round 162, `fuzz/fuzz_targets/rle_stream.rs`):
  a second `cargo-fuzz` target that narrows the input so libfuzzer's
  iteration budget lands on the `BI_RLE8` / `BI_RLE4` state machines
  themselves rather than re-discovering valid 14-byte
  BITMAPFILEHEADERs. The first three fuzz bytes pick the RLE flavour
  (8 vs 4-bpp), width (1..=255) and height (1..=255); the remaining
  bytes become the on-disk RLE pixel payload of a synthetic BMP with
  a maximal colour table (256 × `RGBQUAD` for RLE8, 16 for RLE4) so a
  palette lookup never short-circuits the walk. Seed corpus is two
  pixel streams extracted from the existing `decode` seeds. Same
  panic-free contract; local 20-second smoke run lands ~1.5 M inputs
  at ~72 k execs/sec with zero crashes. CI picks up the new target
  automatically via the org reusable workflow's `[[bin]]` discovery.

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
  `minimal_palette_top_down_roundtrips`, plus `magick`-CLI black-box
  cross-validation `magick_minimal_palette_8bpp` confirming the
  external validator resolves pixels against the trimmed `biClrUsed=2`
  table.

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
  `encode_top_down_indexed8_skips_rle`. `magick`-CLI black-box
  cross-validation test `magick_top_down_rgba` verifies the external
  validator honours the negative `biHeight` on our output.
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
- `tests/magick_validate.rs` integration tests: 7 `magick`-CLI
  black-box cross-validation cases (32-bit, 24-bit, 16-bit, 8-bit
  indexed, 4-bit indexed, RLE8, RLE4).

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
