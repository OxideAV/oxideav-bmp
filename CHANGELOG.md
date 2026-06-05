# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

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
