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
12. `ReadOptions::max_elements` (`scene.rs`, default `None`) and
    `geom::parse_component_within` (`geom.rs`): `read_model` counts every
    vertex, edge, face, loop, edge use and instance over the root and all
    definitions, and fails with `Malformed("too many elements")` past the
    cap. A record of millions of empty faces is six bytes each and deflates
    about a thousand to one, so a small file could otherwise make the per-face
    tables take gigabytes. `parse_component` keeps its behaviour (no cap).
13. Definitions count against `max_elements` too (`scene.rs`
    `read_definitions`): each costs `DEFINITION_WEIGHT` (50) before it is
    made, and each parsed geometry 1 (`parse_component_within`). An empty
    `ComponentDefinition` record is six bytes but a few hundred bytes once
    read, so without this a stream of empty definitions grew without bound.
14. The claimed stream size is enforced: `InflateRead::pump` (`stream.rs`)
    fails with `Malformed("deflate stream is longer than its entry claims")`
    once it has inflated more than the entry's `uncompressed_size`, and
    `read_stream` clamps the model record's end to that size. Upstream
    inflated until the deflate stream ended, so a caller bounding its work by
    the claimed size could be given far more.
15. Legacy topology (`legacy.rs`): `legacy::read_model`, `read_index` and
    `read_model_and_index` give a pre-2021 file in the 2021+ path's shape, a
    `scene::Model` with one `geom::DefGeometry` per definition (`pid` is the
    definition's archive slot, which instances name in `def_pid`) and one for
    the root, plus a `SceneIndex` (textures as `TextureLocation::Inline`) and
    `ReadStats`. Built from the arenas before a definition releases them
    (`emit_topology`), so no placement is walked: no bounds, reachability or
    instance tables, and `finish` (whose placement grouping is quadratic) is
    not called. Hidden faces are kept with bit 0 of `face_flags` set (the
    batches still skip them); the face's back material, plane constant and
    front texture flags, which were read and dropped, are kept; instances
    carry group, image and hidden. `ReadOptions::max_elements` applies, with
    definitions weighing 50 as in item 13. `read`'s `finish` now builds its
    index with the same code (`index_and_stats`), which changes nothing in
    what `read` returns.
16. Guides: `geom::RawGuide` and `DefGeometry::guides` (the 2021+ path:
    item 24). Legacy `CConstructionLine` (point, direction; the start
    and end parameters are dropped, the line is infinite) and
    `CConstructionPoint` records, which were read and discarded, are kept
    unless hidden; hidden ones are counted in `ReadStats::hidden_guides`.
    The field layout (point, direction, start and end parameters) was
    cross-checked against openskp's reader (see item 18 for the source).
17. A definition's JPEG thumbnail (`CDib` subtype 1) as SketchUp 2024 writes
    it into a 2017-2020 file is followed by a `u32` its length does not
    cover, the quality (70 on every such file). Upstream took it for a
    back-reference and a null, so a definition list counted them as
    definitions and read the root count four bytes early: the truck and
    bikes lost root entities and barstools failed with "implausible root
    entity count". `skip_thumbnail_quality` skips it only after a thumbnail,
    only when it is at most 100, and only when what follows reads with the
    word skipped and not without it (a definition; a root count and a first
    root entity; an instance's transform and name). A JPEG without the word
    reads as before. Texture images keep `texture_block`'s own check.
18. SketchUp 2020 container: the version map's `CLayerManager` schema is
    read (`layer_manager_schema`, 4 up to 2019, 7 in 2020). At 7 or more the
    walk skips the 16 bytes `00 00 00 ff fe ff 00` and nine zeros SketchUp
    2020 writes before the definition count and before each definition's
    entity count (the latter only when the undecoded `u32` after it is 0, so
    that a shorter filler running into that word does not pass for it).
    Where they are not exactly there it falls back to
    openskp's `findCountAfterV20Filler` (https://github.com/iamahsanmehmood/openskp,
    commit `13ae161fa0108647a04ad5730776120bb6fe8c83`, MIT, copyright (c)
    2024-2026 Ahsan Mehmood; ported, not copied), for the definition count,
    each definition's entity count (searched before reading, as openskp
    does for version 20), the relationship count and the root count. Changed
    from openskp: the definition count's candidate must be followed by a
    definition rather than any plausible tag, so that a file with no
    definitions is refused rather than read from a later word.
19. Material names `*N` read as `MaterialN` (`materials::display_name`, as
    on the 2021+ path).
20. A textured material's colour is opaque: the fourth byte of the texture
    block's average colour is a colorize flag (0 or 0xff), not an alpha.
21. Hardening: `read` and `read_model` walk from at most 4 candidate slot
    bases (`MAX_BASES`; the probe offers one per model layer);
    `resync_definition` scans at most 64 MiB per file over all walks
    (`RESYNC_BUDGET`; it was 64 MiB per call); the burn-band arithmetic
    (`translate_ref`, `register_burn`, the edge-use parent check) is checked
    and fails with `Malformed`; loop ranges saturate.
22. A definition that fails and after which no next definition is found is
    now an error (`Malformed`), for `read` too. Upstream returned the model
    read so far, with the rest of the definitions and the whole root list
    unread, which `read_model` would have given as an empty root.
23. `SceneIndex::counts.instances` leaves out placements inside a definition
    that failed. `image_defs` is a set.
24. Guides from 2021+ files: `parse_component_within` reads
    `ComponentConstructionLines > ConstructionLine` (point and direction from
    the first six of `ConstructionLineGeomLine`'s eight doubles; the start and
    end parameters are dropped, the line is infinite) and
    `ComponentConstructionPoints > ConstructionPoint`
    (`ConstructionPointPosition`) into `DefGeometry::guides`. Layout from
    openskp's VFF reader (`_extract_geometry_from_nodes`, tags `9113`/`6942`/
    `6A42` and `9213`/`6C42`/`6D42` in its spelling;
    https://github.com/iamahsanmehmood/openskp, commit
    `13ae161fa0108647a04ad5730776120bb6fe8c83`, MIT, copyright (c) 2024-2026
    Ahsan Mehmood; ported, not copied), checked against the point records of
    SketchUp 2024 files. Changed from openskp: the hidden bit of the
    `ConstructionGeometry` wrapper's `DrawingElementFlags` is read, and hidden
    guides are skipped and counted in `DefGeometry::hidden_guides`; a record
    in either list that is not that list's guide, or lacks its body (a point
    shorter than 3 doubles, a line shorter than 8), is skipped and counted in
    `DefGeometry::unread_guides`. `scene::Model::hidden_guides` and
    `unread_guides` sum both over the model (a legacy read counts hidden
    guides in `ReadStats::hidden_guides` instead). Each kept guide counts
    against the element budget.
25. `skip_thumbnail_quality`: removed a branch that could not be taken (a
    `u32` of at most 100 cannot start a definition) and reworded its comment.
26. Guide layers: `DefGeometry::guide_layer`, parallel to `guides`, holds each kept guide's
    layer as `edge_layer` and `face_layer` do: on the 2021+ path the layer pid of the
    `ConstructionGeometry` wrapper's `DrawingElement` (or `NO_LAYER`), on the legacy path
    the layer index of the guide's drawbase layer slot (which was read and dropped).
27. Layer references past 32 bits (2021+ path): `geom::layer_ref` gives `NO_LAYER` for a
    face's, edge's or guide's layer pid that does not fit a `u32` (upstream cut it to its low
    32 bits, which could name another layer). Instances keep the full pid in
    `RawInstance::layer`; the importer drops one past 32 bits the same way. A layer's own id
    saturates at `u32::MAX` (`NO_LAYER`), so nothing can refer to such a layer.
28. Legacy annotations counted: `ReadStats::annotations` counts the `CDimensionLinear` and
    `CText` records `new_obj` reads (they were read only to stay in step and dropped), so the
    importer can say how many it left out. `CDimensionRadial` is still not a known class (see
    the module notes). A unit test in `legacy.rs` reads a crafted `CText` record.
29. Hidden guides kept: `DefGeometry::hidden_guide_items` and `hidden_guide_layer` (parallel,
    layer as `guide_layer`) hold the hidden guides that `guides` leaves out, on both the
    2021+ and the legacy path (a legacy mark rolls them back with the other guide arenas).
    `hidden_guides` and `ReadStats::hidden_guides` still count them. Each kept one counts
    against the element budget.
