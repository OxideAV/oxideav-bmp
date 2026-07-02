//! Deterministic replay of the committed fuzz corpora (Round 383).
//!
//! CI never builds the `cargo-fuzz` harnesses (they need nightly +
//! ASan), so the seed corpora under `fuzz/corpus/` would otherwise be
//! dead weight on every non-fuzz run. This test walks every committed
//! corpus file and pushes the bytes through all six raw decode
//! surfaces, plus the file-header/DIB-header typed parsers — the same
//! panic-free contract the fuzz targets assert, executed on every
//! `cargo test`.
//!
//! Two levels of checking:
//!
//!   * every corpus file, whatever its framing (several targets use a
//!     wire-framed selector prefix rather than raw BMP bytes), must
//!     pass through every decode entry point without panicking;
//!   * files under `corpus/decode/` and `corpus/metadata/` that carry
//!     the `BM` signature are the curated *well-formed* seeds — those
//!     must additionally decode `Ok` end-to-end, so a decoder
//!     regression that breaks a real file (not just hostile bytes)
//!     fails here deterministically.

use oxideav_bmp::{
    decode_bmp, decode_bmp_with_metadata, decode_dib, decode_dib_with_metadata, BitmapFileHeader,
    BitmapInfoHeader, BITMAPFILEHEADER_SIZE,
};
use std::path::PathBuf;

fn corpus_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus")
}

/// Every committed corpus file across every target directory.
fn all_corpus_files() -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    let root = corpus_root();
    let dirs = std::fs::read_dir(&root)
        .unwrap_or_else(|e| panic!("fuzz corpus dir missing at {}: {e}", root.display()));
    for dir in dirs.flatten() {
        if !dir.path().is_dir() {
            continue;
        }
        for file in std::fs::read_dir(dir.path()).unwrap().flatten() {
            if file.path().is_file() {
                let label = format!(
                    "{}/{}",
                    dir.file_name().to_string_lossy(),
                    file.file_name().to_string_lossy()
                );
                out.push((label, std::fs::read(file.path()).unwrap()));
            }
        }
    }
    assert!(
        !out.is_empty(),
        "no corpus files found under {} — seeds should be committed",
        root.display()
    );
    out
}

/// All six decode surfaces + both typed header parsers must return a
/// `Result` for every corpus byte-string, whatever its framing.
#[test]
fn every_corpus_file_is_panic_free_on_all_surfaces() {
    for (label, bytes) in all_corpus_files() {
        let run = || {
            let _ = decode_bmp(&bytes);
            let _ = decode_bmp_with_metadata(&bytes);
            let _ = decode_dib(&bytes, false);
            let _ = decode_dib(&bytes, true);
            let _ = decode_dib_with_metadata(&bytes, false);
            let _ = decode_dib_with_metadata(&bytes, true);
            let _ = BitmapFileHeader::parse(&bytes);
            if bytes.len() > BITMAPFILEHEADER_SIZE as usize {
                let _ = BitmapInfoHeader::parse(&bytes[BITMAPFILEHEADER_SIZE as usize..]);
            }
        };
        // A panic in any surface names the corpus file via unwind.
        std::panic::catch_unwind(run)
            .unwrap_or_else(|_| panic!("corpus file {label} panicked a decode surface"));
    }
}

/// The curated `BM`-signed seeds in the raw-BMP corpora are well-formed
/// by construction and must keep decoding `Ok`.
#[test]
fn wellformed_bmp_seeds_still_decode_ok() {
    let mut seen = 0;
    for (label, bytes) in all_corpus_files() {
        // The curated well-formed seeds are the `.bmp`-named files in
        // the two raw-BMP corpora; the `.bin` stubs there (empty file,
        // bare magic) are deliberately truncated and stay Err.
        let raw_bmp_corpus = label.starts_with("decode/") || label.starts_with("metadata/");
        if !raw_bmp_corpus || !label.ends_with(".bmp") || !bytes.starts_with(b"BM") {
            continue;
        }
        let image = decode_bmp(&bytes)
            .unwrap_or_else(|e| panic!("curated seed {label} stopped decoding: {e:?}"));
        assert!(image.width > 0 && image.height > 0, "{label}: empty image");
        let _ = decode_bmp_with_metadata(&bytes)
            .unwrap_or_else(|e| panic!("curated seed {label} stopped metadata-decoding: {e:?}"));
        seen += 1;
    }
    assert!(
        seen >= 10,
        "expected the curated BM-signed seeds to be present, found {seen}",
    );
}
