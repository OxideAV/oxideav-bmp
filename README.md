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
| 8         | `BI_RGB`       | `Rgba`     |
| 16        | `BI_RGB`       | `Rgba` (5-5-5) |
| 16        | `BI_BITFIELDS` | `Rgba` (mask-derived) |
| 24        | `BI_RGB`       | `Rgba` (BGR→RGB, α=0xFF) |
| 32        | `BI_RGB`       | `Rgba` (BGRA→RGBA) |
| 32        | `BI_BITFIELDS` | `Rgba` (mask-derived) |

`BITMAPINFOHEADER` (v3, 40 B), `BITMAPV4HEADER`, and `BITMAPV5HEADER`
are all accepted. Bottom-up and top-down row orders are auto-detected
from the sign of `biHeight`; output is always top-down. `BI_RLE4`,
`BI_RLE8`, `BI_JPEG`, and `BI_PNG` are rejected at the boundary.

## Encode

Always writes **32-bit BGRA `BI_RGB`** — the simplest layout that
preserves alpha without a `BI_BITFIELDS` negotiation. Input may be
`Rgba` or `Rgb24` (the latter is padded with `α=0xFF` at encode time).

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

## Registration

```rust
let mut codecs = oxideav_codec::CodecRegistry::new();
let mut containers = oxideav_container::ContainerRegistry::new();
oxideav_bmp::register(&mut codecs, &mut containers);
```
