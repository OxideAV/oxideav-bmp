# oxideav-bmp benchmarks

Criterion micro-benchmarks for the BMP decode and encode hot paths.
Three self-contained harnesses build their fixtures on the fly with the
public encoder API (no committed image files):

```sh
cargo bench -p oxideav-bmp --bench decode
cargo bench -p oxideav-bmp --bench encode
cargo bench -p oxideav-bmp --bench roundtrip
```

Absolute timings are machine-specific — the numbers below were taken on
an Apple-silicon laptop (aarch64, `--warm-up-time 1 --measurement-time
2`, criterion 0.5) and are useful only as **relative** before/after
deltas for the round-404 optimisation pass. Re-run locally to A/B a
change; the deltas, not the microsecond figures, are what carry over.

## Decode

| Benchmark              | pixels     | before  | after   | Δ     |
| ---------------------- | ---------- | ------- | ------- | ----- |
| rgba 320×240 (32bpp)   | 76 800     | 8.5 µs  | 9.0 µs  | —¹    |
| rgb24 640×480          | 307 200    | 29.7 µs | 30.2 µs | —     |
| **rgb565 320×240**     | 76 800     | 113 µs  | 49 µs   | **−57 %** |
| rgb565 640×480         | 307 200    | 197 µs² | 192 µs  | —²    |
| **indexed8 320×240**   | 76 800     | 27 µs   | 19.8 µs | **−27 %** |
| indexed4 320×240       | 76 800     | 33.5 µs | 32.1 µs | −4 %  |
| **rle8 320×240**       | 76 800     | 58.6 µs | 37.5 µs | **−36 %** |
| **rle4 320×240**       | 76 800     | 70.8 µs | 44.9 µs | **−37 %** |
| dib rgba 320×240       | 76 800     | 9.3 µs  | 9.1 µs  | —     |
| dib ico rgba 64×64     | 4 096      | 2.04 µs | 1.92 µs | −6 %  |

¹ 32bpp `BI_RGB` BGRA→RGBA path is untouched; the ±5 % is run-to-run
  variance between separately linked bench binaries.
² The 640×480 `rgb565` bench was added this round to guard the
  large-image side. `before` is the pre-round code's combined-LUT path
  (measured by restoring `HEAD`); the per-channel path is gated to stay
  below it, so the large case is preserved rather than regressed.

## Encode

| Benchmark                 | before   | after    | Δ      |
| ------------------------- | -------- | -------- | ------ |
| **rgba 320×240**          | 22.6 µs  | 19.6 µs  | **−13 %** |
| **rgb24 640×480**         | 174 µs   | 158 µs   | **−9 %**  |
| rgb565 320×240            | 14.1 µs  | 13.7 µs  | −3 %   |
| **indexed8 random 320×240** | 56.6 µs | 52.4 µs | **−7 %**  |
| indexed8 rle 320×240      | ~34 µs   | ~34 µs   | flat³  |
| **indexed4 rle 320×240**  | 109 µs   | 104 µs   | **−5 %**  |
| indexed8 min-pal 320×240  | 34.7 µs  | 34.8 µs  | flat   |
| **rgba top-down 320×240** | 22.2 µs  | 19.3 µs  | **−13 %** |
| **dib rgba 320×240**      | 23.2 µs  | 19.8 µs  | **−15 %** |
| **dib ico rgba 64×64**    | 3.42 µs  | 2.67 µs  | **−22 %** |

³ The compressible-RLE path is unchanged; its criterion CI is wide
  (~10 %), so a single-run delta on it is noise.

## What changed (round 404)

1. **Per-channel expansion LUTs for `BI_BITFIELDS` / `BI_RGB` masks
   (decode 16/32bpp).** The per-pixel `expand()` `match` (four branches
   per pixel) is replaced by four 256-byte per-channel tables (1 KiB
   total, L1-resident) — three/four branch-free indexed loads per pixel.
   For 16bpp this is used below the pixel count where the 65 536-entry
   combined value→RGBA table amortises its 256 KiB build; the combined
   table is retained above the threshold, where its single-load loop
   still edges ahead on low-cardinality content. 32bpp bitfields (which
   never had a combined table) uses the per-channel tables everywhere.
   Bytes are bit-identical.

2. **Padded-palette indexing (decode 1/2/4/8bpp).** A fixed-size RGBA
   palette array (2/4/16/256 entries) lets the hot loop index with a
   masked value whose range the compiler already knows, dropping the
   per-pixel `.get().unwrap_or()` bounds check. Missing palette entries
   keep the canonical `[0,0,0,0xFF]` fallback.

3. **`chunks_exact` BGR(A) packers (encode).** `pack_rgba` / `pack_rgb24`
   walked source/dest byte-by-byte inside the per-pixel loop (a bounds
   check per access) and re-matched the input format per pixel. The
   format branch is hoisted out and both sides iterate `chunks_exact`, so
   the fixed 3/4-byte shuffle is bounds-check-free and vectorisable.

4. **RLE encoder work reduction (encode).** `rle4_encode` reuses one
   nibble scratch buffer instead of allocating per row; both RLE encoders
   reserve their output capacity; and the size probe aborts the moment
   the emitted stream reaches the raw-array budget, so an incompressible
   image no longer scans the whole plane to produce a result that is then
   discarded. The chosen format and output bytes are unchanged.

5. **Flat-plane RLE decode (decode).** `decode_rle8` / `decode_rle4`
   built a `Vec<Vec<u8>>` (one allocation per scanline) that the caller
   then reversed and concatenated. They now write a single top-down plane
   directly, each pixel to its already-flipped row, with palette lookups
   through the padded table — dropping `height + 1` allocations, a full
   copy pass, and the per-pixel bounds check.

## In-place row writer (Unreleased)

The encoder writes each source row straight into its final place in the
output buffer instead of packing the rows into a plane-sized buffer of
their own and copying that into the file. The `BI_RLE8` / `BI_RLE4`
probe writes its stream into the same buffer, and the `BI_RLE4` probe
reads the nibbles from the index bytes in place. Output bytes are
unchanged (`tests/encode_identity.rs`).

Bytes allocated by one `encode` of a 1024×1024 image, counted by the
global allocator in `tests/encode_alloc.rs` (v0.1.8 is "before"):

| Image                      | file      | before    | after     |
| -------------------------- | --------- | --------- | --------- |
| `Rgba`                     | 4 194 358 | 8 388 662 | 4 194 358 |
| `Pal8` noise (raw array)   | 1 049 654 | 3 148 856 | 1 051 704 |
| `Pal8` runs (`BI_RLE8`)    | 35 894    | 2 135 096 | 1 087 598 |
| `Indexed4` noise (raw)     | 524 406   | 1 576 056 | 526 456   |
| `Indexed4` runs (`BI_RLE4`) | 34 934   | 1 086 584 | 561 390   |

For `Pal8` / `Indexed4` the buffer is reserved once for the raw file
plus 2 × width + 2 bytes, the most the RLE probe writes past the raw
array before it gives up; when the RLE stream wins, the buffer is
shrunk to the file, which the counter records as one more allocation
of the file's size.
