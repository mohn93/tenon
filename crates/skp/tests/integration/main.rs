//! The crate's integration tests, in one binary so a full run builds and starts one test
//! program per crate instead of one per file.

mod fixtures;
mod robustness;

use skp::container::{Container, Generation};
use skp::scene::{read_model, Model, ReadOptions};
use skp::source::MemorySource;
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::path::Path;

/// Records the largest single allocation each thread makes, so a test can
/// check that a damaged length was refused before it was allocated.
struct Tracking;

thread_local! {
    static LARGEST: Cell<usize> = const { Cell::new(0) };
}

fn note(size: usize) {
    let _ = LARGEST.try_with(|l| l.set(l.get().max(size)));
}

// SAFETY: every call forwards to `System` unchanged; the only addition is a
// thread-local counter that never allocates.
unsafe impl GlobalAlloc for Tracking {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        note(layout.size());
        // SAFETY: the caller's contract for `alloc` is passed through.
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        note(layout.size());
        // SAFETY: as for `alloc`.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: `ptr` came from `System` with this layout.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        note(new_size);
        // SAFETY: as for `dealloc`, and `new_size` is the caller's.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOCATOR: Tracking = Tracking;

/// Runs `f` and returns the largest single allocation it made on this thread.
pub fn largest_allocation(f: impl FnOnce()) -> usize {
    LARGEST.with(|l| l.set(0));
    f();
    LARGEST.with(Cell::get)
}

/// What a damaged fixture may allocate at once: far above anything the
/// fixtures need, far below what a damaged length field can claim.
pub const ALLOCATION_LIMIT: usize = 16 * 1024 * 1024;

/// The two CC BY 4.0 fixtures (see `tests/fixtures/ATTRIBUTION.md`).
pub const FIXTURES: [&str; 2] = ["cupboard.skp", "desk.skp"];

pub fn fixture(name: &str) -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()))
}

/// Opens and reads a whole file the way an importer would.
pub fn read(bytes: Vec<u8>) -> skp::Result<(Container, Model, MemorySource)> {
    let src = MemorySource::new(bytes);
    let c = Container::open(&src)?;
    if c.generation != Generation::Vff {
        return Err(skp::Error::Malformed("not a VFF file"));
    }
    let m = read_model(&src, &c, &ReadOptions::default())?;
    Ok((c, m, src))
}

/// Faces in the root and in every definition.
pub fn face_count(m: &Model) -> usize {
    m.root.faces() + m.definitions.iter().map(|d| d.geom.faces()).sum::<usize>()
}
