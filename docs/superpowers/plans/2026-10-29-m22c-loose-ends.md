# M22c: T-junction cracks between LOD chunks, Make Unique and scene section planes

## 1. T-junctions (crates/render/src/lod.rs)

M21e pins vertices whose exact positions lie in 2+ chunks. A vertex of chunk A in the middle of
an edge of chunk B is not at a shared position, so clustering could move it off B's edge.

- `open_edges`: per chunk, the edges (by exact position) used by one triangle only. A T-junction
  always involves such edges on both sides, so nothing else is looked at.
- `t_junctions`: a spatial hash of all chunks' open edges (grids of 4x growing cells, an edge in
  the finest grid whose cell is at least its length, in every cell its widened box touches); each
  open-edge end is looked up in its cell; one within 0.5 mm of an open edge of another chunk is
  pinned, with that edge's ends. `Borders::pin_t_junctions` adds them to the pinned places.
- `Borders::find` collects each chunk's open edges with its places on the same threads, then runs
  the pass once.
- Test: a left sheet with a vertex every 0.5 m beside a right sheet with one every 1 m along x = 20,
  clustered at several level pairs; every border piece of both survives. Without the pass the
  left border breaks (asserted too).

## 2. Make Unique and scene planes (crates/kernel/src/group.rs)

`duplicate_mapped` records, for each section plane it copies (also in groups inside the copy),
old (mesh, plane) -> new (mesh, plane). `make_unique` then changes every page whose
`PageSections.active` holds an old pair to also hold the new one, as `Step::ChangePage` steps in the
same `SceneChange` (one undo step; undo restores). Test in
crates/session/tests/integration/scene_sections.rs: go to the scene, save and open, undo, redo.
