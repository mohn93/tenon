# M21c: Section planes in scenes and dense cuts

## Part 1: scenes store section planes

- Kernel `Page.sections: Option<PageSections>`: `active: BTreeMap<MeshId, SectionId>` (the plane
  active in each context that had one) plus the display (`planes`, `cuts`, `fill`,
  `material_fill`). `None` = not stored, as `positions`/`joints`.
- Going to a scene makes exactly the stored planes active: a context not listed has none active
  (SketchUp's Active Section Planes). The activations are `Scene::set_active_section` changes
  joined into the go-to step ("Go to scene <name>"), so they undo like activating by hand. The
  display is a view setting (not undone), like the lighting.
- A plane erased (or moved into a group, or its definition pruned) is dropped from every scene:
  `Scene::edit` calls `drop_stale_sections` after any edit while some page stores sections, as a
  `ChangePage` step in the same change, so undo restores it.
- Commands: `sections: Option<bool>` on add_scene (absent/false: not stored), update_scene
  (absent: stored again if stored), change_scene (false drops, true stores if absent, absent
  keeps; counts as a change argument). Queries: `scenes[].sections`, `scene.sections`
  (`{active:[{context, plane}], planes, cuts, fill, fill_color}`).
- `.tnn` schema 30 only when some scene stores sections; older files unchanged. PageData gets
  `sections` (flags written only when off).
- Scenes panel: "Section planes" checkbox. `render_view` with `scene` (t = 1) renders a frame
  with the planes active and the scene's display (call's `section` override still wins). Print
  takes no scene, so nothing to do there.
- Ruling: a plane moved into a group by Group is dropped from the scenes (the stored context no
  longer holds it).

## Part 2: section fill in dense room grids

Parity of ray crossings fails in a room of a grid whose walls are shared single faces: every ray
meets an even number of walls. Rule (in `Arrangement::solid_faces`): a region the parity vote
rejects becomes solid when (a) it is enclosed in 3D (all six rays meet a surface, so an open
courtyard stays empty) and (b) it shares an outline edge with a solid region (the neighbour's
cycle across the edge is a region's outer cycle, not a hole cycle). Repeated to a fixed point, so
the middle of a 5x5x5 grid fills from the rooms around it. A cavity inside a solid is a separate
connected piece, hence a hole of the solid around it and never a neighbour beside it, so it stays
a hole. Render, picking and print all go through `section_loops` (session `section_clip.rs`), so
all are fixed.

## Tasks
1. Failing test `rooms_sharing_single_faces_are_all_filled` (3x3x3 and 5x5x5), cavity guard test.
2. Rule above.
3. Kernel page sections, data, stale drop; tests.
4. Session commands, queries, go-to, schema 30; tests.
5. App checkbox, render_view frame, docs/agent-api.md, roadmap.
