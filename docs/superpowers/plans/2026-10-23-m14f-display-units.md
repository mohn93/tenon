# M14f Display Units Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The model's display units (already saved since M13c for dimension and note labels) become complete and are used everywhere: length formats (decimal m, cm, mm, inches; architectural feet and inches with fractions; engineering decimal feet), length precision, length snapping, angle precision and angle snapping; every displayed length and angle (Measurements box, status messages, Entity Info, sheets, dimensions, notes, print, phone viewer) follows them, and typed values accept the model unit for bare numbers plus `3'6"`, `1/2"`, `3'6 1/2"`, `20cm`. The API keeps meters and degrees.

**Architecture:** `session::units` owns the `Units` settings, formatting (`format_length`, `format_angle`) and parsing (`parse_length`, `parse_angle`) with the model's units; the tools crate's `vcb` helpers become thin wrappers that take `&Units` from a new `ToolContext.units`. Length snapping rounds cursor-driven lengths to the snap step in the tools; angle snapping replaces the fixed 15 degree step. `.skr` gains the new fields (schema 17 only when one is not default).

**Tech Stack:** Rust 2024, egui 0.36.2. No new dependencies.

**Spec:** `docs/superpowers/roadmap.md` row M14f.

## Global Constraints

- No new dependencies. With default units (meters, precision 3, no length snapping, angle precision 1, angle snapping 15 degrees) the window shows what it shows today except where today's text is inconsistent (document every visible change); golden images unchanged; schema 16 and older output byte-identical with default units.
- The API (commands, queries, events, `tool_input` coordinates) keeps meters and degrees; only display strings and typed `tool_input` values follow the units.
- New or changed commands keep `id_slots`, api dispatch and `docs/agent-api.md` in step.
- Gate per task (project rules in the SDD rules files): native gate + `cargo check --target wasm32-unknown-unknown -p kernel -p session -p tools -p render -p api -p app --lib -p web`; `scripts/build-web.sh` in Task 3.
- No emoji. Short commit messages. Never `git add -A`.

## Spec notes (rulings)

1. **Units:** `length`: `meters | centimeters | millimeters | inches | feet | feet_inches` (`feet` is engineering decimal feet, new; `feet_inches` is architectural); `precision` as today (decimals 0-6, or for `feet_inches` the finest fraction 1/2^p inch, 0-6); new `length_snap` (meters, 0 = off, default 0, up to 1000 m), `angle_precision` (decimals 0-3, default 1), `angle_snap` (degrees, 0 = off, default 15, at most 90). `set_units` takes each as optional; one undo step as today; `units` query reports all fields.
2. **Formatting:** `format_length(meters, &Units)`: decimal units with the unit symbol (`1.25 m`, `125 cm`, `1250 mm`, `49.21"`, `4.1'`), trailing zeros stripped as today, no `-0`; `feet_inches` as today (`3' 6 1/2"`). `format_angle(degrees, &Units)` with `angle_precision` decimals and a degree sign. Areas are out of scope.
3. **Parsing:** `parse_length(text, &Units) -> meters`: a bare number is in the model's unit (inches for `feet_inches`, feet for `feet`); explicit suffixes always win (`m`, `cm`, `mm`, `in`, `"`, `ft`, `'`); feet-inches forms `3'6"`, `3' 6"`, `3'6`, `3'-6"`, fractions `1/2"`, `6 1/2"`, `3'6 1/2"`, `3' 6-1/2"`; a leading minus; spaces trimmed; anything else is an error naming the text. `parse_angle` unchanged (degrees, optional sign and `°`/`deg`); slopes unchanged. Typed values in every tool use these.
4. **Snapping:** with `length_snap` set, lengths taken from the cursor (Line, Rectangle and Rotated Rectangle sides, Circle/Polygon radius, Arc, Move and Copy distance along an inferred direction, Push/Pull distance, Offset distance, Tape Measure distance, Dimension offset excluded) round to the nearest multiple; typed values are never rounded; snapping to inferred points wins over length snapping (a point snap keeps its exact position). `angle_snap` replaces the 15 degree step in Rotate, Protractor and Rotated Rectangle angles (0 = free angles).
5. **Where displayed:** every tool's measurement and status messages; Entity Info lengths, coordinates and angles; the 3D Text sheet's height and depth fields (shown and typed in the model unit); dimension and note labels (as today); print scale bar (as today); phone viewer part sizes and measure (as today); Soften angle and field of view stay in degrees with their current precision (not model lengths).
6. **Files:** new fields `#[serde(default)]` and skipped when default; schema 17 written only when one of them is not default (older builds refuse unknown fields); `.skp` import maps SketchUp's unit (legacy files expose it) to `length` (inches -> `feet_inches` when SketchUp's format is architectural, else `inches`; feet -> `feet`; mm, cm, m) when the reader gives it; documented.
7. **Window:** Settings > Model tab: Format (Decimal, Architectural, Engineering), Unit (m, cm, mm, inches; for Decimal), Precision (per format), Length snapping (checkbox and value, typed in the model unit), Angle precision, Angle snapping (checkbox and value). Each change is one `set_units` (one undo step).

## Review Focus

1. Formatting and parsing round trips for every format and precision (including fractions reducing, carrying into feet, negatives, tiny and huge values), and errors for bad text.
2. No tool or panel still prints raw meters or fixed-decimal degrees (grep for `format_length(`, `format_angle(`, `" m"`, `°`).
3. Length snapping never fights point inference; typed values exact; angle snap 0 frees angles.
4. `.skr` schema 17 only when needed, older output byte-identical; `.skp` unit mapping.

---

### Task 1: Units settings, formatting and parsing

**Files:** `crates/session/src/{units.rs, command.rs, document/annotations.rs, file.rs, skp.rs, query.rs}`, `crates/skp` only if the 2021+ reader needs to expose the unit (report if so), api dispatch, `docs/agent-api.md`; tests in session (`units` unit tests and integration).

- [ ] **Step 1: Write the failing tests** for rulings 1-3 and 6 (Review Focus 1, 4).
- [ ] **Step 2-5:** run, implement, gate, commit `git commit -m "Units: formats, snapping settings and parsing"`.

### Task 2: Tools follow the units

**Files:** `crates/tools/src/{tool.rs (ToolContext.units), vcb.rs (wrappers), every tool that formats or parses}`, `crates/app/src/editor.rs` (fill ToolContext.units), docs; tests in the tools crate and app tool_input tests.

- [ ] **Step 1: Write the failing tests** for rulings 3-5 for tools (Review Focus 2-3).
- [ ] **Step 2-5:** run, implement, gate, commit `git commit -m "Tools follow the display units"`.

### Task 3: Entity Info, sheets and the Model settings

**Files:** `crates/app/src/{info.rs, ui/sheets.rs, ui/settings.rs, ui/mod.rs, app.rs}`, `docs/agent-api.md` window rows; UI tests.

- [ ] **Step 1: Write the failing tests** for rulings 5 and 7.
- [ ] **Step 2-5:** run, implement, gate (+ build-web), commit `git commit -m "Units in Entity Info and Settings"`.

### After the tasks (controller)

- Push `dev`; CI green; roadmap row M14f done; spec bullet; memory; merge and push; next milestone (M14g).
