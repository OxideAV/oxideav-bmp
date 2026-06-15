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

`BITMAPCOREHEADER` (OS/2 1.x, 12 B), the truncated OS/2 2.x
`OS22XBITMAPHEADER` (16…39 B), `BITMAPINFOHEADER` (v3, 40 B; the full
64-byte `OS22XBITMAPHEADER` decodes through this path on its 40-byte
INFO prefix), `BITMAPV2INFOHEADER` (52 B, Adobe-intermediate
RGB-masks-in-header), `BITMAPV3INFOHEADER` (56 B, adds the in-header
alpha mask slot), `BITMAPV4HEADER`, and `BITMAPV5HEADER` are all
accepted. The OS/2 1.x path honours the 3-byte `RGBTRIPLE` colour-table
layout (the OS/2 2.x and every V3+ header use 4-byte `RGBQUAD`).
Bottom-up and top-down row orders are auto-detected from the sign of
`biHeight`; output is always top-down `Rgba`. `BI_JPEG` and `BI_PNG`
are rejected at the boundary.

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
use oxideav_bmp::{BmpOs2Halftone, decode_bmp_with_metadata};
let (_image, md) = decode_bmp_with_metadata(bytes)?;
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
use oxideav_bmp::BitmapFileHeader;

// `parse` validates buffer length + the `0x4D42` `bfType` signature.
let h = BitmapFileHeader::parse(bytes)?;
assert!(h.has_canonical_magic());     // distinguishes "BM" from OS/2
                                      // `BA`/`CI`/`CP` archive variants
let _ = h.file_size;                  // bfSize (may be 0 — informational)
let _ = h.pixel_offset;               // bfOffBits — start of pixel array
let _ = h.reserved_is_clean();        // bfReserved1/2 zero per the spec

// `from_bytes` is the unchecked variant (returns `None` on a short
// buffer; the magic check is skipped). Encoder consumers go the
// other way via `to_bytes()` for a deterministic 14-byte layout.
```

`decode_bmp` and `decode_bmp_with_metadata` now both funnel the file
header parse through this struct, so the "shorter than header" and
"missing 'BM' signature" error messages come from a single source.

### Typed `BitmapInfoHeader` view + `DibHeaderKind`

The 40-byte `BITMAPINFOHEADER` that opens every V3-and-later DIB is
likewise surfaced as a typed struct — the eleven documented fields
(`header_size` / `width` / `height` / `planes` / `bit_count` /
`compression` / `image_size` / `x_pels_per_meter` / `y_pels_per_meter`
/ `clr_used` / `clr_important`) at their on-disk offsets:

```rust
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

`encode_bmp_with_calibrated_rgb` is the V4 colour-space counterpart to
those V5 + ICC paths: instead of pointing at an embedded or linked ICC
profile it emits a 108-byte `BITMAPV4HEADER` with
`bV4CSType = LCS_CALIBRATED_RGB` and bakes the caller-supplied CIE
endpoints (`[i32; 9]` `CIEXYZTRIPLE`, packed
R.x R.y R.z G.x G.y G.z B.x B.y B.z) and per-channel gamma triple
(`[u32; 3]`, unsigned 16.16 fixed point) directly into the header's
endpoint / gamma fields. The decoder round-trips it:
`decode_bmp_with_metadata` reports `BmpColorSpace::Calibrated` and
returns the same `endpoints` + `gamma_rgb` the encoder was given (V4
carries no rendering intent, so `rendering_intent` stays `None`).
Supported pixel formats and option handling match the ICC paths:
`Rgba` (32-bit BGRA `BI_RGB`), `Rgb24` (24-bit BGR `BI_RGB`),
`Rgb565` (16-bit `BI_BITFIELDS` 5-6-5 with the canonical masks in the
V4 four-mask region), and the indexed `Indexed8` / `Indexed4` /
`Indexed1` (uncompressed `BI_RGB`, colour table between the header and
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

`Indexed8` / `Indexed4` / `Indexed1` input is also accepted on both
V5 + ICC paths: the encoder emits a 124-byte V5 header
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
| `Rgb555` (2 B/px)   | 16-bit `BI_RGB` 5-5-5         | V3     |
| `Rgb565` (2 B/px)   | 16-bit `BI_BITFIELDS` 5-6-5   | V4     |
| `Indexed8` (1 B/px) | 8-bit indexed `BI_RGB` or `BI_RLE8` (auto) | V3 |
| `Indexed4` (1 B/px) | 4-bit indexed `BI_RGB` or `BI_RLE4` (auto) | V3 |
| `Indexed1` (1 B/px) | 1-bit indexed `BI_RGB` (monochrome) | V3 |

For a 16-bpp `BI_RGB` bitmap the on-disk layout is always RGB 5-5-5
(high bit reserved, then R in bits 14..10, G in bits 9..5, B in bits
4..0), so `Rgb555` input is emitted with a plain 40-byte
`BITMAPINFOHEADER` and **no** `BI_BITFIELDS` mask block — the encode
counterpart of the decoder's 16-bit `BI_RGB` 5-5-5 path. Input is one
little-endian 5-5-5 `u16` per pixel (the same packed wire shape
`Rgb565` accepts). `top_down` is honoured (negative `biHeight`); the
headerless DIB helper (`encode_dib` / `encode_dib_plane`) also accepts
`Rgb555`. For `Rgb565` the V4 header carries canonical masks R=0xF800,
G=0x07E0, B=0x001F. For 8/4-bit indexed formats the encoder tries RLE
compression
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
decoder's `biClrUsed`-aware palette reader consumes the trimmed table
transparently.

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

Four `cargo-fuzz` targets live in `fuzz/`:

* `decode` — feeds arbitrary bytes to `decode_bmp` and to `decode_dib`
  (both the plain and the doubled-height XOR+AND-mask modes). The
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
* `metadata` — fuzzes the `decode_bmp_with_metadata` /
  `decode_dib_with_metadata` entry points, which are independent public
  surfaces with their own attacker-controlled offset / slicing maths
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

All four targets share the same panic-free contract — every input
returns a `Result` rather than panicking, indexing out of bounds, or
OOM-aborting — and build against the framework-free standalone path
(`default-features = false`).

```sh
cargo +nightly fuzz run decode
cargo +nightly fuzz run rle_stream
cargo +nightly fuzz run encode_roundtrip
cargo +nightly fuzz run metadata
```

The `decode` harness shook out and fixed several header-driven
denial-of-service paths (RLE / `bpp = 0` / `biClrUsed` over-allocation);
see `CHANGELOG.md`. A local 20-second `rle_stream` run lands ~1.5 M
inputs (~72 k execs/sec) with zero crashes. A 60-second
`encode_roundtrip` run lands ~1.33 M inputs (~21.8 k execs/sec, peak
RSS ~480 MB) with zero crashes — every direct-colour input survived
the encode→decode pair byte-for-byte. A 60-second `metadata` run lands
~1.08 M inputs with zero crashes. A daily
`.github/workflows/fuzz.yml` job runs all four targets on a shared
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

The uncompressed pixel-unpack path fills one flat top-down RGBA plane
in a single pass (a `chunks_exact_mut(4)` cursor, no per-scanline
allocation); for 16 bpp `BI_BITFIELDS` at large sizes a 65 536-entry
value→RGBA lookup table replaces the four per-pixel mask expansions.

## Registration

```rust
let mut codecs = oxideav_codec::CodecRegistry::new();
let mut containers = oxideav_container::ContainerRegistry::new();
oxideav_bmp::register(&mut codecs, &mut containers);
```
