# M13c Dimensions and Text Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** SketchUp-style annotations: linear dimensions between two points or along an edge and radial (radius/diameter) dimensions on arcs and circles that follow the geometry they are attached to; leader text pointing at a point and screen text fixed to the view; 3D text as geometry; a model display-units setting used by dimension and leader labels; all in `.skr`, commands and queries, and in the window through a Dimension tool, a Text tool and a 3D Text dialog.

**Architecture:** Two new per-mesh entity kinds in the kernel `Model`, following the guide template (ids from the mesh counter, ops `Add*`/`Remove*`, tags, transform, copy, erase, clip, group/explode/duplicate): `Dimension` (`EntityId::Dimension`, written `r<n>`) and `Note` (text; `EntityId::Note`, written `n<n>`). Attachment is by `Anchor::Vertex(VertexId)` (follows moves for free) or `Anchor::Point(DVec3)`; a radial dimension anchors to a `CurveId`. A kernel upkeep pass (like `Model::upkeep` for curves) freezes an anchor to its last position (`Vertex` -> `Point`, curve -> stored circle) when its vertex or curve goes away, in the same change set so undo restores the attachment. Dimension and leader geometry (extension lines, dimension line, arrows, leader line) is drawn by the render crate as lines (so it appears in headless renders); label text is drawn by the window's egui overlay (absent from headless renders; agents read label text through queries). A document-level `Units { length: LengthUnit, precision: u8 }` (saved in `.skr`) formats labels; the rest of the UI keeps meters until M14f. 3D text turns glyph outlines (skrifa, Inter Regular) into faces with holes, optionally extruded, inside a new group.

**Tech Stack:** Rust 2024, egui 0.36.2, wgpu 30. `skrifa =0.44.0` (already in `Cargo.lock` through epaint) may become a direct dependency of `session`; `Cargo.lock` must gain no new package.

**Spec:** `docs/superpowers/roadmap.md` row M13c.

## Global Constraints

- Only `skrifa =0.44.0` may be added as a direct dependency; `Cargo.lock` gains no new package.
- Existing behaviour unchanged; golden images unchanged for models without annotations; files without annotations or units changes save at their current schema; schema 12 is written only when the model has a dimension, a note or non-default units.
- New command variants get arms in `Document::execute`/`edit` and `session::batch::id_slots` (no `_` arm); new `EntityId` variants are handled in every exhaustive match (no `_` arms added to hide them).
- Text limits: note and 3D text strings 1..=1024 characters (trimmed non-empty), at most 64 lines; 3D text height > 0 and within the coordinate limit.
- Gate per task: fmt, clippy `-D warnings`, `cargo test --workspace`, and `cargo clippy --workspace --all-targets --target x86_64-pc-windows-msvc -- -D warnings`; CI on push. `export PATH="$HOME/.cargo/bin:$PATH"`.
- Docs (`docs/agent-api.md`) updated in the task that changes behaviour. No emoji. Short commit messages, no attribution lines. Never `git add -A`.

## Spec notes (rulings)

1. **Dimension model:** `Dimension::Linear { a: Anchor, b: Anchor, offset: DVec3, tag }` (the dimension line is the segment a-b moved by `offset`, which is perpendicular to a-b; label = distance |a-b|); `Dimension::Radial { curve: RadialAnchor, diameter: bool, leader: DVec3, tag }` where `RadialAnchor::Curve(CurveId) | Fixed { center, normal, radius }` (label "R 1.5 m" or "Ø 3 m"; `leader` is the label position offset from the center). A linear dimension between two points that coincide is refused.
2. **Note model:** `Note::Leader { anchor: Anchor, position: DVec3, text: Option<String>, tag }` (`text: None` = automatic text: the anchored point's coordinates formatted in the display units) and `Note::Screen { at: [f64; 2] /* 0..1 fractions of the view, top-left origin */, text: String, tag }` (root mesh only).
3. **Following and freezing:** vertex anchors follow every edit that keeps the vertex. When an anchor vertex is removed (erase, weld, heal, redraw) the anchor becomes `Point` at the vertex's last position; a radial dimension whose curve is removed or no longer a circle/arc becomes `Fixed` with its last circle. Both happen in the same change set as the edit (undo restores the attachment). Move/Rotate/Scale of selected annotations transforms their `Point` anchors, offsets and positions; vertex anchors move with their vertices.
4. **Copy, group, clip:** annotations are selectable and copied, erased, moved, tagged, grouped, exploded and clipped like guides; a copied dimension whose vertices are copied with it anchors to the copies, otherwise its anchors become `Point`s.
5. **Units:** `Units { length: Meters | Centimeters | Millimeters | Inches | FeetInches, precision: 0..=6 decimals (for FeetInches: denominator 1,2,4,8,16,32,64 as fractional inches) }`, default Meters with precision 3 (matches today's `format_length`). Command `set_units`, query `units`. One formatter `format_length_in(meters, &Units) -> String` in session, used by labels; the Measurements box and Entity Info stay as they are (M14f).
6. **Commands:** `add_dimension { a: AnchorInput, b: AnchorInput, offset: [f64;3] }` where `AnchorInput` is a vertex id string or a point; `add_edge_dimension { edge, offset }` (anchors the edge's two vertices); `add_radial_dimension { edge (any edge of the arc/circle), diameter?: bool, leader: [f64;3] }` (refused unless the edge's curve is a circle or arc); `add_note { anchor: AnchorInput, position: [f64;3], text?: string }`; `add_screen_note { at: [f64;2], text }`; `change_note { note, text? (null = automatic for leaders) }`; `add_3d_text { text, position: [f64;3], height, depth?: f64 (0 = flat), filled?: bool (default true) }` (new group at `position` in the XY plane of the open context, text reading along +X, baseline at `position`); `set_units`. Each one undo step. Queries: `entity` info for `d`/`n` ids (kind, anchors resolved to points, label text), `annotations` (all in the open context with label text). Exports: OBJ/STL/glTF and `.skp` report "N dimensions/notes were not written"; `.skp` import reports them as not imported (2021+ tag constants exist but are not parsed).
7. **Window:** Dimension tool (key: none; Tools menu and rail): click a point, click a second point, move to set the offset, click; or click an edge then click to place; or click an arc/circle edge then click to place a radial dimension. Text tool: click a point (leader) or empty space (screen note), then place and type in a small sheet; double-click a note with Select to edit its text in the same sheet. 3D Text: Tools > 3D Text... opens a sheet (text, height, depth, filled), then the result is placed with the cursor like Paste (reuse the Paste tool's placement flow with the new group's outline). Labels: egui overlay text at the projected label position, theme colours, hidden when behind the camera; picking: annotations are picked by their drawn lines and by their label rectangle. Units: a "Model" tab in Settings with Units and Precision (the setting is saved in the model, not prefs; the tab says so). A Model Info window is out of scope.
8. **3D text geometry:** glyph outlines from skrifa (quadratic/cubic curves flattened to at most 12 segments per curve, tolerance 1% of the height), contours nested by containment into faces with holes (even-odd), letters advance by the font's advance widths, line breaks move down by 1.2 x height; extruded by `depth` with push/pull semantics when > 0. Unsupported characters (no glyph) are skipped and reported in the result. Font: Inter Regular, moved to `crates/session/assets/fonts/` (the app includes it from there), licence files moved with it.

## Review Focus

1. Erase one end vertex of a dimensioned edge: the dimension stays at the same place (frozen), undo re-attaches it; move the remaining vertex: only the attached end follows.
2. Copy a dimensioned rectangle with its dimension: the copy's dimension follows the copied vertices, not the originals.
3. Annotations inside a group: the dimension anchors to the group's vertices (same mesh) and draws/labels correctly when the group is moved, rotated or scaled, and when edited open.
4. Save/reopen keeps annotations, units, frozen anchors and screen notes; a schema 11 file opens with none; a file with an annotation whose vertex id is missing is refused with a clear error.
5. 3D text "Hello o8%" produces valid closed faces with holes for o, 8 and %, extrudes cleanly, and a character with no glyph is skipped and reported.

---

### Task 1: Annotations in the kernel

**Files:** `crates/kernel/src/{ids.rs, annotation.rs (new), model.rs, data.rs, curves.rs (upkeep hook), guides.rs patterns, group.rs, clip.rs, tag.rs, scene.rs, transform.rs, lib.rs}`; tests `crates/kernel/tests/integration/annotations.rs`.

- [ ] **Step 1: Write the failing tests** for rulings 1-4 at kernel level (ops and inverses, upkeep freezing in the same change set, transform, copy remap, group/explode/duplicate, clip extract/insert, tags, data round trip and validation) and Review Focus 1-3 (kernel parts).
- [ ] **Step 2: Run** — expected: failures.
- [ ] **Step 3: Implement** following the guide template.
- [ ] **Step 4: Run** the gate.
- [ ] **Step 5: Commit** `git commit -m "Dimensions and notes in the kernel"`.

### Task 2: Commands, queries, units and files

**Files:** `crates/session/src/{command.rs, document.rs (+ document/annotations.rs new), query.rs, batch.rs, file.rs (schema 12), units.rs (new), export.rs, skp_export.rs, skp.rs (not-imported counts)}`, `crates/api/tests/integration/dispatch.rs`, `docs/agent-api.md`; tests `crates/session/tests/integration/annotations.rs`.

- [ ] **Step 1: Write the failing tests** for rulings 5-6 and Review Focus 4, plus Move/Rotate/Scale/Erase/Copy/Select of annotation ids, export warnings and the units formatter table (every unit and precision, negative and zero, feet-inches carry e.g. 11 63/64" rounding up to 1').
- [ ] **Step 2-5:** run, implement, gate, commit `git commit -m "Dimension and note commands, units and files"`.

### Task 3: Drawing, labels and selection

**Files:** `crates/render/src/{annotations.rs (new), scene.rs, renderer.rs}`, `crates/tools/src/{scenery.rs, inference.rs (picking), select.rs, move_tool.rs, probe.rs}`, `crates/app/src/{feedback.rs (Hud labels), ui/viewport.rs, editor.rs, info.rs}`; tests in each crate (render line generation, picking, kittest labels, Entity Info lines).

- [ ] **Step 1: Write the failing tests:** line generation for linear/radial/leader shapes (extension lines, arrows sized in pixels), selection highlight and hover, picking by line and by label rectangle, box select, label text and placement (behind-camera hidden), screen notes at fractions of the view, Entity Info for a dimension and a note, golden images unchanged without annotations.
- [ ] **Step 2-5:** run, implement, gate, commit `git commit -m "Draw, label and select dimensions and notes"`.

### Task 4: Dimension and Text tools, note editing, units in Settings

**Files:** `crates/tools/src/{dimension.rs (new), text.rs (new), tool.rs, lib.rs}`, `crates/api/src/editor.rs` (`ToolName`), `crates/app/src/{shortcuts.rs, menus.rs, ui/icons.rs, ui/rail.rs, ui/viewport.rs, ui/sheets.rs, ui/settings.rs, editor.rs, app.rs}`, `docs/agent-api.md`; tests in `crates/tools/tests/scenery.rs`, app UI tests and tool_input tests.

- [ ] **Step 1: Write the failing tests** for ruling 7 (both tools' click flows through tool_input, Esc, typed offset distance for the Dimension tool, the text sheet, double-click edit, the Settings Model tab).
- [ ] **Step 2-5:** run, implement, gate, commit `git commit -m "Dimension and Text tools"`.

### Task 5: 3D text

**Files:** `crates/session/{Cargo.toml, assets/fonts/ (moved), src/text3d.rs (new), src/command.rs, src/document.rs}`, `crates/app/src/ui/fonts.rs` (include path), `crates/app/src/{ui/sheets.rs, menus.rs, editor.rs}`, `THIRD_PARTY_NOTICES.md` paths, `docs/agent-api.md`; tests `crates/session/tests/integration/text3d.rs` and an app UI test.

- [ ] **Step 1: Write the failing tests** for ruling 8 and Review Focus 5 (face count and holes for "o8%", extrusion validity, multi-line, skipped characters reported, limits refused).
- [ ] **Step 2-5:** run, implement, gate, commit `git commit -m "3D text"`.

### After the tasks (controller)

- Push `dev`; CI green. Roadmap row M13c done; spec bullet; memory; merge and push; next milestone (M13d).
