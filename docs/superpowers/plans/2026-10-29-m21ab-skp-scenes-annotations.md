# M21a-M21b: SketchUp scenes both ways, SketchUp annotations

## M21a tasks

1. **Import, 2017-2020.** `session/src/skp.rs` fed `add_pages` only for VFF files. The legacy
   reader already returns `SceneIndex::scenes`; feed them the same way. Legacy layer ids in
   `SceneDef::hidden_layer_ids` and in `LayerDef::id` are both positions in the reader's layer
   list (`layer_index`), so `add_tags`' id map applies unchanged (checked against the
   tutorial's 2024 twin: same hidden tag names in all twelve scenes). The "N scenes were not
   imported" warning and its counter go.
2. **Export.** `skpwrite` gains `Doc::pages` (name, optional view, hidden tag indices). The
   layout is read off SketchUp's own 2017 file with scenes (corpus `tutorial01_v2017.skp`,
   12 pages) and the 2020 gondola; see PORTED.md item 16. Pages go after the page manager's
   count in the scaffold tail; the manager's current-scene pointer, the tail's `CSkpStyle`
   declaration (now a class reference) and the tail's later slots are patched.
3. **Session.** `skp_export` builds pages from `Scene::pages`, reports `scenes` written, and
   warns for what a page cannot hold.

## Rulings

- Capture flags are `0x1ff` as SketchUp writes, less the camera bit for a scene with no view
  (the camera object is always present, a dummy one). Every other block is always written, so
  the record never depends on which bits are set.
- A scene with `tags: None` is written showing every tag (flag kept, empty list) and warned;
  clearing the layer bit while keeping the block risks a mis-parse in SketchUp.
- Each page repeats the scaffold's own style, geographic, axes and options blocks: switching
  scenes in SketchUp restores what the exported model already has.
- Thumbnail: a flat grey 256x125 PNG (deflated with miniz_oxide, already a workspace
  dependency); SketchUp redraws it on scene update.
- Parallel views write the fov in degrees as Tenon holds it plus the height at the target
  (2 d tan(fov/2)), so the importer's reconstruction returns the same field of view.
- Positions, joints, lighting modes, shadows, styles, tag-colouring, x-ray and hidden-geometry
  display cannot be written: counted per scene in the export warnings.
- Not verified in SketchUp itself (no SketchUp here); verified byte-structure against a
  SketchUp-written page and by round trip through our reader.

## Tests

`skpwrite` `pages.rs` (round trip, escape slots, determinism, bad input);
`session` `skp_export.rs` (export -> legacy reader -> import, warnings), corpus tests for the
tutorial (`corpus_tutorial_scenes`, `corpus_tutorial_scenes_round_trip`).

## M21b tasks

1. **Find the layouts.** Corpus search (TLV walk of every file's `model.dat`): only
   `openskp_fixtures/Untitled.skp` has linear dimensions (13) and only the tutorial has text
   (3 labels, all pushpin leaders); no file has a radial dimension or screen text. Legacy
   `CDimensionLinear`/`CText` are skipped by the legacy reader and say nothing about field
   meanings.
2. **Reader** (`skp` crate, `annotations.rs`): `RawAnnotation::{Linear, Leader}` per
   `DefGeometry`, `AnnotationSkips` counters (radial, screen text, hidden, unread), element
   budget spent per record.
3. **Kernel:** `ImportGeometry::{dimensions, notes}` built in `add_shaped` after the guides;
   an end within `LENGTH_TOL` of a vertex anchors to it, else fixed; refused ones counted
   (`TreeReport::skipped_annotations`); cap `MAX_IMPORT_ANNOTATIONS` (10,000).
4. **Session:** conversion to meters with the finite and coordinate-limit checks, report
   fields `dimensions` and `notes`, warnings per skipped kind.

## M21b rulings

- Offset of a linear dimension = `DimensionYPos * (normal x x_axis)`. The sign is inferred:
  in `Untitled.skp` the dimensions come out above top edges and below floor edges (outside
  the body); the other sign puts them inside the wall. Not confirmed in SketchUp.
- `DimensionXPos` (text along the line) and the text position mode are dropped (Tenon has
  neither); a dimension's own text is not kept (counted).
- Text maps only for leader type 2 (pushpin); the point ref's position is used for type 1
  (free) and an entity pid (type 3) is resolved for a vertex or guide point of the same
  definition; anything else is unread. Leader vector is the text's offset from the point.
- Skipped and counted, layouts unknown: radial dimensions (the arc, leader parameter and
  ratio fields) and screen / view-based text (position fields unverified). Hidden ones too.
- 2017-2020 files keep the "N dimensions and text labels were not imported" warning.
