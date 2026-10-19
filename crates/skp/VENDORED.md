# Vendored: openskp-rs `skpcore`

This crate is the `core/skpcore` reader from openskp-rs, copied into this
repository and changed as listed below.

- **Upstream:** https://github.com/hakkokimkr/openskp-rs
- **Commit:** `dee12a0b43a90ad67e6407e4f9a73634e581677e`
- **Path:** `core/skpcore/src` (all modules, including `legacy.rs`)
- **Licence:** MIT, copyright (c) 2026 Hakko Kim; see `LICENSE` (upstream's, unchanged)

Upstream's tests are not copied: they read a private corpus.

## Changes

1. Package renamed `skp`, `publish = false`, `edition = "2021"` (upstream's),
   `miniz_oxide` from the workspace (`=0.8.9`).
2. `FileSource` and the `memmap2` dependency removed: they held the crate's only
   `unsafe`. `MemorySource` is kept. The crate root now has
   `#![forbid(unsafe_code)]` (upstream: `#![forbid(unsafe_op_in_unsafe_fn)]`).
3. `#![allow(clippy::all)]` at the crate root: the code is upstream's and keeps
   its style. Rustc warnings are fixed instead (one unneeded `mut` in
   `edges.rs`).
4. Formatted with this workspace's `cargo fmt`.
5. `InflateRead::pump` (`stream.rs`): once the compressed input is used up, it
   keeps calling `inflate` with empty input and `MZFlush::Finish` until
   `StreamEnd` or no progress. Upstream stopped as soon as the input was used
   up, losing output the inflater still held when its stream spanned several
   input chunks (a model.dat re-deflated by zlib at level 6, or the desk
   fixture's model.dat repeated four times at miniz level 1).
6. Lengths read from the file are checked before anything is allocated:
   `ByteSource::read_vec` checks the range against the source first (this
   covers the zip central directory read in `ZipDirectory::find` and the
   compressed bytes `read_entry` reads); a size that does not fit in `usize`
   is `OutOfRange`. What `read_entry` inflates is capped separately (item 10).
   `InflateRead::new` now returns `Result` and refuses a compressed range that
   runs past the source (`OutOfRange`).
7. `Vec::with_capacity` of the zip entry count is capped by how many records
   the directory has room for.
8. `RecordStream::payload` grows its buffer as bytes arrive (at most doubling,
   from 64 KiB) instead of allocating the record's claimed size up front; the
   768 MiB cap stays.
9. Offset arithmetic on values read from the file uses `checked_add`
   (`Error::Malformed` on overflow): the central directory offset, zip entry
   header and data offsets, `InflateRead`'s input end, `RecordStream::enter`'s
   record end and `RangeRead`'s reads and skips. `Le::take` checks
   `at + len`; `RangeRead::total_hint` saturates.
10. `read_entry` refuses an entry whose claimed inflated size (or, stored, its
    size) is over `zip::MAX_ENTRY_BYTES` (128 MiB) with `Error::Malformed`
    before reading it, and inflates at most `min(claimed + 64, cap)` bytes.
    Upstream inflated up to whatever size the entry claimed, so a small
    deflated entry could expand to gigabytes. `read_entry_within` takes a
    tighter cap; `Model::index` reads each `materials/*/material.xml` with a
    1 MiB cap and skips a larger one, as it already skipped an unreadable one.
11. `Model::index_without_placements` (`scene.rs`): `index` without the
    two placement walks, `bounds` and `instance_tables` (`bbox` is `None`,
    `counts.instances` 0). Those walks grow with the number of resolved
    placements, not the file's size: a small file whose definitions place
    each other many times, many levels deep, makes them take minutes and
    hundreds of megabytes. `index` now calls it and adds the two walks, so
    its result is unchanged. The importer calls only this one.
