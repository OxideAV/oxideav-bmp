//! A counting global allocator and the fixtures the allocation tests
//! share. Each `*_alloc.rs` test file includes this module, so each
//! test binary installs its own counter.
//!
//! The counter is per thread (the test harness allocates on its own
//! threads) and counts the bytes asked for, not the bytes still live:
//! `alloc`, `alloc_zeroed`, and the new size of every `realloc`.

// Every test file uses a different subset of the fixtures.
#![allow(dead_code)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use oxideav_bmp::{BmpImage, BmpPixelFormat, Palette, Plane};

struct Counting;

thread_local! {
    static COUNTING: Cell<bool> = const { Cell::new(false) };
    static BYTES: Cell<usize> = const { Cell::new(0) };
}

fn note(size: usize) {
    // `try_with`: the thread-locals are gone while a thread exits.
    let _ = COUNTING.try_with(|on| {
        if on.get() {
            let _ = BYTES.try_with(|b| b.set(b.get() + size));
        }
    });
}

// SAFETY: every method forwards its arguments to `System` unchanged, so
// `System`'s guarantees hold; the counting itself only touches
// const-initialised thread-locals without destructors, which never
// allocate.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        note(layout.size());
        System.alloc(layout)
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        note(layout.size());
        System.alloc_zeroed(layout)
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout)
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        note(new_size);
        System.realloc(ptr, layout, new_size)
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// Bytes allocated on this thread while `f` runs.
pub fn allocated<R>(f: impl FnOnce() -> R) -> (R, usize) {
    BYTES.with(|b| b.set(0));
    COUNTING.with(|c| c.set(true));
    let r = f();
    COUNTING.with(|c| c.set(false));
    (r, BYTES.with(|b| b.get()))
}

pub const W: u32 = 1024;
pub const H: u32 = 1024;
/// 14-byte file header + 40-byte V3 header + the 32-bit pixel array.
pub const RGBA_FILE: usize = 14 + 40 + (W * H * 4) as usize;
/// What an encode may allocate beyond the buffers it needs. Nothing
/// plane-sized fits.
pub const SMALL: usize = 8 * 1024;

pub fn rgba_plane() -> Plane {
    let data = (0..W as usize * H as usize * 4)
        .map(|i| (i.wrapping_mul(31) ^ (i >> 7)) as u8)
        .collect();
    Plane::new(W as usize * 4, data)
}

pub fn rgba_image() -> BmpImage {
    BmpImage::new(W, H, BmpPixelFormat::Rgba, vec![rgba_plane()]).unwrap()
}

/// `needed`: the file, or the buffers the encode cannot do without.
pub fn check(what: &str, allocated: usize, needed: usize) {
    println!("{what}: {allocated} bytes allocated, {needed} needed");
    assert!(
        allocated <= needed + SMALL,
        "{what} allocated {allocated} bytes where {needed} are needed: {} more",
        allocated - needed
    );
}

/// The indexed images that may be RLE-compressed: `Pal8` and `Indexed4`,
/// with noise (the stream loses to the raw array) and with runs (it
/// wins). Each is 1024 x 1024 with a two-entry palette.
pub fn rle_candidates() -> Vec<(String, BmpImage, usize, bool)> {
    let w = W as usize;
    let h = H as usize;
    let noise: Vec<u8> = (0..w * h)
        .map(|i| (i.wrapping_mul(37) >> 3) as u8)
        .collect();
    let runs: Vec<u8> = (0..w * h).map(|i| ((i % w) / 64) as u8).collect();
    let palette = Palette::from_rgb(&[[0, 0, 0], [255, 255, 255]]);
    let mut out = Vec::new();
    for (format, table, row) in [
        (BmpPixelFormat::Pal8, 256 * 4, w),
        (BmpPixelFormat::Indexed4, 16 * 4, w / 2),
    ] {
        // Raw `BI_RGB` file: both headers, full table, the packed rows.
        let raw_file = 14 + 40 + table + row * h;
        for (what, data, rle_wins) in [("noise", &noise, false), ("runs", &runs, true)] {
            let image = BmpImage::new(W, H, format, vec![Plane::new(w, data.clone())])
                .unwrap()
                .with_palette(palette.clone());
            out.push((format!("{format:?} {what}"), image, raw_file, rle_wins));
        }
    }
    out
}
