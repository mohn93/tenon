# M11a Design Foundation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The window gets a design system under an unchanged layout: `ui.rs` becomes a `ui/` module tree, the UI tests find widgets by label through AccessKit (`egui_kittest`), a theme module installs dark and light token sets with Inter, JetBrains Mono and Phosphor built in, a widget kit and one shortcut table drive the menus, tooltips and key handling, the window's viewport follows the theme (day canvas gradient, accent selection with 2 px edges and an 18% face tint) while `render_view` and headless renders stay as they are, the right panel sizes its sections from measured text, Undo and Redo are disabled when empty and name their step, the window has a 960 x 600 pt minimum, and View > Theme (System, Dark, Light) is saved in `~/.sketchup-alt/prefs.json`.

**Architecture:**
- `app/src/ui/` replaces `ui.rs`: `mod.rs` (types, `draw`), `toolbar.rs`, `status.rs`, `right.rs` (Entity Info and the panel's measured layout), `materials.rs`, `outliner.rs`, `dialogs.rs`, plus the new `theme.rs` (tokens, metrics, egui style mapping, the viewport's `ViewStyle`), `fonts.rs` (embedded fonts), `icons.rs` (Phosphor glyphs) and `widgets.rs` (the kit). Tests live in `ui/tests/` and drive `draw` through `egui_kittest`.
- `app/src/shortcuts.rs` (new, top level because `editor.rs` uses it): the one table of key bindings; menus and tooltips take their text from it, `app.rs` looks Cmd chords up in it and `editor.rs` looks plain keys up in it.
- `app/src/prefs.rs` (new): window preferences read at start and written on change.
- `render`: a `ViewStyle` (new `style.rs`) set per renderer: `ViewStyle::CLASSIC` (the default, today's look) for `render_view`, headless and the goldens; the window sets one from the theme. The lighting uniforms carry the selection colors, the face tint, the line fades and the day canvas; a canvas pass draws the day gradient and a second edge pass widens selected edges, both only when the style asks.
- `session`: `Document::undo_label` and `redo_label` (the next step's label) so the toolbar does not clone the history every frame.

**Tech Stack:** Rust 2024; egui, egui-winit, egui-wgpu `=0.36.2`; wgpu `=30.0.1`; new: `egui-phosphor =0.14.0` (MIT OR Apache-2.0, depends on `egui ^0.36`), `egui_kittest =0.36.2` (dev only); Inter 4.1 and JetBrains Mono 2.304 TTFs (OFL 1.1) in `crates/app/assets/fonts/`; `serde`/`serde_json` (already workspace dependencies) move into the app's normal dependencies for the prefs file.

**Spec:** `docs/superpowers/specs/2026-10-10-sketchup-alt-design.md`, "Interface milestones", M11a. Design audit reports (token tables, refactor order, risks): `visual.md`, `engineering.md`, `ia.md`, `interaction.md` in `/private/tmp/claude-501/-Users-mohn93-Desktop-side-projects-sketchup/7b020704-4344-4767-b2d2-50b34442d2ad/scratchpad/design/`. The executor reads the spec bullet and this plan; the reports only explain where values came from.

## Global Constraints

- No emoji anywhere (code, comments, docs, commit messages). Comments succinct, in the style of the surrounding code.
- Before any cargo command: `export PATH="$HOME/.cargo/bin:$PATH"`. After every task: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace` pass. The suite has 1159 tests at the start; no task leaves fewer (a test replaced by a twin keeps the count; count with `cargo test --workspace 2>&1 | grep "test result" | awk '{s+=$4} END {print s}'`).
- Pinned versions: egui, egui-winit, egui-wgpu and egui_kittest `=0.36.2`; wgpu `=30.0.1`; egui-phosphor `=0.14.0`. Every new dependency is added to `[workspace.dependencies]` with `=` and used with `.workspace = true`.
- From the spec: the layout is unchanged (toolbar on top, status bar at the bottom, right panel on the right, same order of controls). Accent `#4C8DFF` dark, `#2463EB` light. 4 pt spacing grid, 28 pt controls, corner radius 6 for controls, 8 for windows, 4 for swatches, 2 pt accent focus ring. Inter 13 pt body, 11 pt uppercase section titles, JetBrains Mono for values. Theme System, Dark, Light, default System, from View > Theme, saved in `~/.sketchup-alt/prefs.json` (window chrome). `render_view` and headless renders keep their current backgrounds, so golden images and agent renders do not change. Selected edges 2 px in the accent, face tint 18%. Undo and Redo disabled when there is nothing to undo or redo, tooltips naming the step. Minimum window 960 x 600 pt.
- Idle stays at 0% CPU: no unconditional `request_repaint`; a measured layout may call `request_discard` only when a measurement changed.
- Git: work on `dev`; other agents may share the checkout: run `git status` and `git diff` first and stage only the files you changed (never `git add -A` or `git add .`); short plain commit messages, no attribution lines. Never push or merge.
- Do not launch the app window (no `cargo run` of the GUI). Tests never write under `~/.sketchup-alt`: prefs tests use `std::env::temp_dir()` paths with the process id in the name.
- Downloaded archives go into a fresh empty folder under the scratchpad, never into the repo; only the checked TTF and license files are copied into `crates/app/assets/fonts/`.

## Review Focus

- A prefs file that is missing, empty, not JSON, names an unknown theme, cannot be read, or sits in a `~/.sketchup-alt` that is a symlink, or HOME unset: the window starts with defaults and never panics; a theme change still applies for the session; a broken file is replaced at the next change (Task 9 tests: `load` on each bad input returns defaults; `save` into a symlinked folder is an error and leaves the link's target untouched).
- Keys that worked before the shortcut table must still work the same way: Shift+L picks Line, Shift+F enters fly mode, Shift+G makes a component, Shift+Z zooms to extents, Cmd+Z repeats when held but Cmd+S does not, Cmd+I with Shift does nothing, and Cmd chords on non-Latin layouts still reach their letters (Task 4 tests: `plain` and `command` cases, and a test that every Cmd letter in the table has a physical-key fallback in `shortcut_letter`).
- Idle repaint: the measured right panel, the themed widgets and tooltips must let the window go idle (Task 3 test `the_panels_settle_in_one_frame`: after the harness settles, one more `try_run` takes a single step).
- A model with 40 materials and 60-character names at the 960 x 600 minimum: no list row cut in half at the bottom of the Materials list, the panel keeps its width, the Outliner keeps its rows, a too-short window scrolls the panel (Task 3 tests `no_material_row_is_cut_by_the_list_bottom` at 800, 600 and 520 pt and `long_names_keep_the_panel_width`; Task 8 `the_panels_fit_the_smallest_window`).
- Switching theme or the OS appearance with a selection showing: the viewport restyles on the next frame without a model rebuild, selected guides included (Task 6 test `restyling_recolors_a_selected_guide`), and the window's style never reaches `render_view` (the goldens are unchanged, and `Snapshotter` keeps its own renderer at `ViewStyle::CLASSIC`).

## Spec notes (rulings made in this plan)

- **Module layout.** Flat `ui/` (no `panels/` subfolder): `mod.rs`, `toolbar.rs`, `status.rs`, `right.rs`, `materials.rs`, `outliner.rs`, `dialogs.rs`, `theme.rs`, `fonts.rs`, `icons.rs`, `widgets.rs`, tests in `ui/tests/{mod,menus,entity,layout,materials,outliner}.rs`. The shortcut table is `crate::shortcuts`, not `ui::shortcuts`, because `editor.rs` dispatches through it; prefs are `crate::prefs`.
- **Tests.** Every UI test moves to `egui_kittest` and finds widgets by label; the coordinate sweeps are deleted once their twins pass (Task 2), so the count is unchanged. The harness draws the panels into a root `Ui` over the whole window, as `Context::run_ui` gives the app (the harness's own `Ui` has an 8 pt margin). The Outliner keeps its `#[cfg(test)]` row record (path, rect, selected, open), because AccessKit cannot say a row is bold (open); its tests click the recorded rects. No snapshot images (`egui_kittest`'s `snapshot`/`wgpu` features stay off).
- **Accessible names added for the tests** (Task 2), each a behaviour-free change: menu items are named without their shortcut text ("New", not "New Cmd+N"); sliders and value fields are labelled by their row's label ("Angle", "Opacity", "Glow", "Offset", "Rotation", "Scale", "Size (m)"; a two-field row gives both fields the label); palette swatches are named `Color #RRGGBB`; the material name field is named "Material name".
- **Icons.** `egui-phosphor 0.14.0` depends on `egui ^0.36` (checked against crates.io), so it is used as a crate, regular weight only; no vendoring.
- **Fonts.** Inter 4.1 static TTFs Regular and SemiBold (egui does not read variable-font axes) and JetBrains Mono 2.304 Regular, from their GitHub releases, checked by SHA-256, with their OFL texts beside them in `crates/app/assets/fonts/`. Families: Proportional = Inter, Phosphor, then egui's defaults (emoji fallback); Monospace = JetBrains Mono, then egui's defaults; `Name("inter-semibold")` = Inter SemiBold, Inter. Text styles: Body and Button 13 pt Inter; Small 11 pt Inter; Heading (window titles) 13 pt Inter SemiBold; Monospace 13 pt JetBrains Mono; section titles 11 pt Inter SemiBold, uppercase, secondary color. Values (numeric fields, the measurements box, the activity log, recovery paths) use Monospace.
- **Tokens** are visual.md's table: dark bg.app `#141518`, bg.panel `#1C1D21`, bg.raised `#25272C`, bg.hover `#2E3036`, bg.pressed `#373A41`, border `#33363C`, text.primary `#E8E9EC`, text.secondary `#A3A7B0`, text.disabled `#6B6F78`, accent `#4C8DFF`, accent fill accent at 22%, warn `#F5B544`, danger `#F2555A`, success `#3DBE74`; light `#ECEEF1`, `#FFFFFF`, `#F6F7F9`, `#E9ECF0`, `#DFE3E8`, `#D8DCE2`, `#17191E`, `#5B616C`, `#9EA4AD`, accent `#2463EB`, accent at 14%, `#B7791F`, `#D93036`, `#1E9E57`. Text on an accent fill is white in both.
- **Metrics.** `item_spacing` 8 x 4, `button_padding` 8 x 4, `interact_size` 40 x 28 (28 pt controls), window margin 12, menu margin 8, indent 16; radii 6 (controls), 8 (windows and menus), 4 (swatches and key chips); swatches 16 pt; icons 16 pt. The 2 pt focus ring is `widgets.active.bg_stroke` (egui styles a focused widget as active) and `selection.stroke` (a focused text field's frame); so selected list rows have accent text on the accent fill. Disabled widgets keep egui's disabled fade; `text.disabled` is used where the kit paints text itself. The toolbar keeps local spacing (6 pt between items, 4 pt button padding) so it stays one row at 1,280 pt as M8b requires, until M11b replaces it.
- **Theme choice.** `ThemeChoice` System (default), Dark, Light, mapped to egui's `ThemePreference`; both styles are installed with `set_style_of`, so System follows the OS through egui-winit (initial `window.theme()` and `ThemeChanged`). Headless never builds a theme.
- **Viewport.** Only the window's renderer gets a theme `ViewStyle`; `Snapshotter` (render_view, headless) and the goldens use `ViewStyle::CLASSIC`, today's values bit for bit (`#F5F5F5` clear, old blue `[0.10, 0.35, 0.95]` edges, 50% tint toward `[0.25, 0.45, 0.95]`, 1 px). Window by day: a vertical gradient, light `#F4F6F8` to `#E3E7EC`, dark `#4A4D55` to `#383B42` ("mid-dark": the darker `#2A2D33` of visual.md left black edges unreadable against the canvas); the grid and axes fade 60% toward the canvas middle in the dark theme (0 in light), edges never fade by day. By night both themes keep `NIGHT_BACKGROUND` and the 0.8 line fade. Selection in the window: edges and selected guides in the accent, edges 2 px (the selected edges are drawn twice more, offset one pixel across and one down, only when some edge is selected), faces mixed 18% toward the accent; the see-through opacity boost is unchanged.
- **Right panel.** Keeps its place and stays fixed-width, 200 pt (180 before: 13 pt Inter and 28 pt controls do not fit two value fields in 180). Its sections size from what the last frame measured (Entity Info, the headers, the controls under the Materials list) and from a row pitch (control height plus item spacing): the Materials list shows whole rows only, at most 8 and at least 2; the Outliner keeps at least 5 rows; a panel shorter than the sum scrolls. A changed measurement stores itself and asks for one discarded pass (`request_discard`), never a repaint.
- **Undo and Redo** become icon buttons (Phosphor `ARROW_U_UP_LEFT`, `ARROW_U_UP_RIGHT`) in the same toolbar place, named "Undo" and "Redo" for AccessKit, disabled when there is nothing to undo or redo; the tooltip reads `Undo <Step>` with the key chip, the step being the history label with underscores as spaces and the first letter capital (`push_pull` reads "Push pull", `batch` reads "Batch"). The Cmd+Z path is unchanged (with nothing to undo it still reports why in the status bar).
- **Shortcut table.** One entry per binding: the 11 tools (Space and the tool letters), New, Open, Save, Save As, Import, Export, Quit, Undo, Redo (the only two that act again while held), Erase Selection (Delete), Make Group (G), Make Component (Shift+G), Close Group (Escape), Zoom Extents (Shift+Z), Fly mode (F). Cmd chords match their modifiers exactly (Shift+Cmd+N, Shift+Cmd+O and Shift+Cmd+Q no longer act; they did by accident). Plain keys: an uppercase letter with no binding of its own acts as its lowercase one, so Shift+L is Line and Shift+F flies as before. Delete, Backspace and Escape keep the editor's own multi-step handling (typed text first); their table entries label the menus and tooltips, and tests check the editor does what they say. Fly mode's own keys and the arrow locks are not table shortcuts. Key text: on macOS `⇧⌘S`, `⌫`, `Esc`, `Space`, `L`; elsewhere `Ctrl+Shift+S`, `Delete`, `Esc`, `Space`, `L` (Inter has the macOS key glyphs).
- **Prefs.** `~/.sketchup-alt/prefs.json` (`HOME` unset: in memory only, one warning at start), JSON `{"theme": "system" | "dark" | "light"}`. Missing file: defaults, silently. Unreadable or invalid file (including an unknown theme): defaults and one stderr warning; it is overwritten at the next change. Unknown keys are ignored and not kept. Written on each change through a temporary file renamed into place, mode 0600, in `~/.sketchup-alt` made or checked with `cli::ensure_private_dir` (0700, no symlink); a failed write is a stderr warning and the choice still holds for the session. Neither the API nor headless reads or writes it.
- **Docs.** agent-api.md lists View > Theme (and the prefs file) and the minimum window size as window chrome; the other preferences the spec names (UI scale, panel sizes, recent files) are added by the milestones that make them.
- **Not in M11a** (engineering.md proposals for later milestones): the AccessKit adapter, native menus, UI scale, the frame-time overlay, the icon rail and the resizable dock.

## File Structure

| File | Responsibility | Task |
|---|---|---|
| `crates/app/src/ui/mod.rs` (was `ui.rs`) | `UiAction`, `View` and row types, `draw` | 1, 3, 7, 9 |
| `crates/app/src/ui/toolbar.rs` | the top bar: menus, tools, Undo/Redo, group buttons | 1, 2, 3, 4, 7, 9 |
| `crates/app/src/ui/status.rs` | the status bar and measurements box | 1 |
| `crates/app/src/ui/right.rs` | the right panel: Entity Info, texture position, measured layout | 1, 3, 5 |
| `crates/app/src/ui/materials.rs` | the Materials list and its controls | 1, 2, 3, 5 |
| `crates/app/src/ui/outliner.rs` | the Outliner | 1, 2, 3, 5 |
| `crates/app/src/ui/dialogs.rs` | Agent Activity and Soften Edges windows | 1, 2 |
| `crates/app/src/ui/fields.rs` (temporary) | value rows, until they join the kit | 1, 2, 5 |
| `crates/app/src/ui/theme.rs` (new) | tokens, metrics, `install`, `ThemeChoice`, `view_style` | 3, 4, 5, 6, 7, 9 |
| `crates/app/src/ui/fonts.rs` (new) | embedded fonts and families | 3 |
| `crates/app/src/ui/icons.rs` (new) | Phosphor glyphs the panels use | 7 |
| `crates/app/src/ui/widgets.rs` (new) | the kit: tool button, menu item, key hint (4); section header, numeric row, swatch (5); icon button (7) | 4, 5, 7 |
| `crates/app/src/ui/tests/*.rs` (new) | kittest harness and the panels' tests | 2-9 |
| `crates/app/src/shortcuts.rs` (new) | the shortcut table | 4 |
| `crates/app/src/prefs.rs` (new) | prefs file | 9 |
| `crates/app/src/gui.rs` | installs fonts and theme, passes the window's theme, reports and sets the theme | 3, 6, 9 |
| `crates/app/src/app.rs` | key dispatch through the table, viewport style, undo labels, minimum size, prefs | 4, 6, 7, 8, 9 |
| `crates/app/src/editor.rs` | plain keys through the table | 4 |
| `crates/app/src/main.rs` | module declarations, prefs path | 4, 9 |
| `crates/app/assets/fonts/*` (new) | TTFs, OFL texts, sources | 3 |
| `crates/render/src/style.rs` (new), `renderer.rs`, `scene.rs`, `lib.rs`, `shaders/scene.wgsl` | `ViewStyle`, uniforms, canvas and wide-edge passes | 6 |
| `crates/render/tests/headless.rs` | style tests | 6 |
| `crates/session/src/document.rs`, `crates/session/tests/commands.rs` | `undo_label`, `redo_label` | 7 |
| `Cargo.toml`, `crates/app/Cargo.toml` | new dependencies | 2, 3, 9 |
| `docs/agent-api.md` | window chrome list | 10 |
| `docs/superpowers/roadmap.md` | M11a row | 11 |

---|---|---|
| `crates/app/src/ui/mod.rs` (was `ui.rs`) | `UiAction`, `View` and row types, `draw` | 1, 3, 6, 8, 10 |
| `crates/app/src/ui/toolbar.rs` | the top bar: menus, tools, Undo/Redo, group buttons | 1, 2, 5, 8, 10 |
| `crates/app/src/ui/status.rs` | the status bar and measurements box | 1 |
| `crates/app/src/ui/right.rs` | the right panel: Entity Info, texture position, measured layout | 1, 2, 6 |
| `crates/app/src/ui/materials.rs` | the Materials list and its controls | 1, 2, 6 |
| `crates/app/src/ui/outliner.rs` | the Outliner | 1, 2, 6 |
| `crates/app/src/ui/dialogs.rs` | Agent Activity and Soften Edges windows | 1, 2, 5 |
| `crates/app/src/ui/theme.rs` (new) | tokens, metrics, `install`, `ThemeChoice`, `view_style` | 3, 7 |
| `crates/app/src/ui/fonts.rs` (new) | embedded fonts and families | 3 |
| `crates/app/src/ui/icons.rs` (new) | Phosphor glyphs the panels use | 4 |
| `crates/app/src/ui/widgets.rs` (new) | tool button, icon button, section header, numeric row, swatch, menu item, key hint | 4 |
| `crates/app/src/ui/tests/*.rs` (new) | kittest harness and the panels' tests | 2-10 |
| `crates/app/src/shortcuts.rs` (new) | the shortcut table | 4 |
| `crates/app/src/prefs.rs` (new) | prefs file | 10 |
| `crates/app/src/gui.rs` | installs fonts and theme, passes the window's theme, reports the resolved theme | 3, 7, 10 |
| `crates/app/src/app.rs` | key dispatch through the table, viewport style, undo labels, minimum size, prefs | 4, 7, 8, 9, 10 |
| `crates/app/src/editor.rs` | plain keys through the table | 4 |
| `crates/app/src/main.rs` | module declarations, prefs path | 1, 4, 10 |
| `crates/app/assets/fonts/*` (new) | TTFs, OFL texts, sources | 3 |
| `crates/render/src/style.rs` (new), `renderer.rs`, `scene.rs`, `lib.rs`, `shaders/scene.wgsl` | `ViewStyle`, uniforms, canvas and wide-edge passes | 7 |
| `crates/render/tests/headless.rs` | style tests | 7 |
| `crates/session/src/document.rs`, `crates/session/tests/commands.rs` | `undo_label`, `redo_label` | 8 |
| `Cargo.toml`, `crates/app/Cargo.toml` | new dependencies | 2, 3, 10 |
| `docs/agent-api.md` | window chrome list | 11 |
| `docs/superpowers/roadmap.md` | M11a row | 12 |

---

### Task 1: Split `ui.rs` into a `ui/` module tree

A pure move: no line of behaviour changes, every test keeps passing unchanged.

**Files:**
- Move: `crates/app/src/ui.rs` to `crates/app/src/ui/mod.rs`
- Create: `crates/app/src/ui/toolbar.rs`, `status.rs`, `right.rs`, `materials.rs`, `outliner.rs`, `dialogs.rs`, `fields.rs`, `sweep_tests.rs`

**Interfaces:**
- Consumes: nothing new.
- Produces: `crate::ui::{UiAction, MaterialRow, TextureRow, Agents, View, TexturePosition, FLY_HELP, draw}` unchanged (paths and signatures). Inside `ui`: `toolbar::toolbar(ui: &mut Ui, view: &View, actions: &mut Vec<UiAction>)`, `status::status_bar(ui, view, actions)`, `right::right_panel(ui, view, actions)`, `materials::materials(ui, view, actions)`, `outliner::outliner(ui, view, actions)`, `dialogs::{agent_activity, soften_window}`, `fields::{DragRow, drag_row, take_row}`, all `pub(super)`.

- [ ] **Step 1: Record the baseline**

```bash
cd ~/Desktop/side_projects/sketchup && export PATH="$HOME/.cargo/bin:$PATH"
git status && cargo test --workspace 2>&1 | grep "test result" | awk '{s+=$4} END {print s}'
```
Expected: a clean tree on `dev` and `1159`.

- [ ] **Step 2: Move the file**

```bash
mkdir -p crates/app/src/ui && git mv crates/app/src/ui.rs crates/app/src/ui/mod.rs
```
`main.rs` keeps `mod ui;` (a folder with `mod.rs` resolves the same).

- [ ] **Step 3: Split the code by function**

Cut each item from `ui/mod.rs` into its file, unchanged except `fn` becoming `pub(super) fn` (and `const`/`struct` becoming `pub(super)`) where another file uses it. Each new file starts with a one-line `//!` comment and the `use` lines it needs.

| File | Items (as named in today's `ui.rs`) |
|---|---|
| `mod.rs` (stays) | module doc, `UiAction`, `MaterialRow`, `TextureRow`, `RIGHT_PANEL_WIDTH`, `Agents`, `View`, `TexturePosition`, `FLY_HELP`, `PANEL_MIN_HEIGHT`, `draw` |
| `toolbar.rs` | the body of `Panel::top("toolbar").show(...)` becomes `pub(super) fn toolbar(ui: &mut Ui, view: &View, actions: &mut Vec<UiAction>)`; `draw` calls `Panel::top("toolbar").show(ui, \|ui\| toolbar::toolbar(ui, view, &mut actions));` |
| `status.rs` | the body of `Panel::bottom("status").show(...)` becomes `pub(super) fn status_bar(ui, view, actions)` |
| `right.rs` | `right_panel`, `end_texture_position_edit`, `FACES_KEY`, `OFFSET_KEY`, `ROTATION_KEY`, `SCALE_KEY`, `millimeters`, `degree_tenths`, `thousandths`, `texture_position`, `Field` |
| `fields.rs` | `DragRow`, `drag_row`, `take_row` |
| `materials.rs` | `DEFAULT_SWATCH`, `MATERIALS_MAX_HEIGHT`, `BELOW_MATERIALS_LIST`, `swatch`, `image_swatch`, `rgb`, `materials`, `glow_slider`, `tenths`, `texture_size_key`, `end_texture_size_edit`, `texture_controls`, `percent` |
| `outliner.rs` | `OUTLINER_MIN_HEIGHT`, `outliner`, `outliner_id`, `expand_open_path`, `outliner_node`, `outliner_row`, `row_button`, `outliner_label` |
| `dialogs.rs` | `agent_activity`, `soften_window` |
| `sweep_tests.rs` | the whole `mod tests { ... }` body (without the `mod tests {` line and its closing brace), declared in `mod.rs` as `#[cfg(test)] mod sweep_tests;` |

`BELOW_MATERIALS_LIST` uses `OUTLINER_MIN_HEIGHT`: import it in `materials.rs` with `use super::outliner::OUTLINER_MIN_HEIGHT;`.

In `outliner.rs`, the three test hooks change their path from `tests::` to `super::sweep_tests::`:

```rust
    #[cfg(test)]
    ui.data_mut(|d| d.remove::<Vec<super::sweep_tests::Recorded>>(super::sweep_tests::recorded_id()));
```
and every `tests::record(` becomes `super::sweep_tests::record(`. In `sweep_tests.rs`, `Recorded`, `recorded_id` and `record` stay `pub(super)` (visible in `ui`, so in `ui::outliner`).

`sweep_tests.rs` starts with:

```rust
//! The panels' tests as coordinate sweeps; replaced by `ui/tests/` in the next task.

use super::*;
use super::outliner::{OUTLINER_MIN_HEIGHT, outliner_label};
use egui::{Pos2, Rect, Vec2};
```
plus whatever else the compiler names (for example `use crate::editor::MATERIAL_PALETTE;`); add imports only.

- [ ] **Step 4: Build, test, lint**

```bash
cargo build -p app && cargo test -p app && cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --all --check
cargo test --workspace 2>&1 | grep "test result" | awk '{s+=$4} END {print s}'
```
Expected: all pass, `1159`. Fix only imports and visibility until it does; `git diff -M --stat` should show `ui.rs -> ui/mod.rs` as a rename plus the new files.

- [ ] **Step 5: Commit**

```bash
git status && git diff --stat
git add crates/app/src/ui/
git commit -m "Split ui.rs into a ui module tree"
```

---

### Task 2: Label-based UI tests with egui_kittest

Every one of the 28 UI tests gets a twin that finds widgets by label through the AccessKit tree; once all twins pass, the sweeps are deleted.

**Files:**
- Modify: `Cargo.toml` (workspace dependency), `crates/app/Cargo.toml` (dev-dependency)
- Modify: `crates/app/src/ui/toolbar.rs`, `dialogs.rs`, `fields.rs`, `materials.rs` (accessible names)
- Create: `crates/app/src/ui/tests/mod.rs`, `menus.rs`, `entity.rs`, `layout.rs`, `materials.rs`, `outliner.rs`
- Modify: `crates/app/src/ui/mod.rs` (`#[cfg(test)] mod tests;`), `outliner.rs` (hooks move to `tests`)
- Delete: `crates/app/src/ui/sweep_tests.rs`

**Interfaces:**
- Consumes: Task 1's module tree.
- Produces (test-only, used by every later task's UI tests): in `ui::tests`, `pub(super) struct Fixture` (owns everything `View` borrows, plus `actions: Vec<UiAction>` and `central: egui::Rect`), `Fixture::view(&self) -> View<'_>`, `Fixture::take(&mut self) -> Vec<UiAction>`, `type H = egui_kittest::Harness<'static, Fixture>`, `SCREEN: Vec2` (1280 x 800), `harness(f: Fixture) -> H`, `harness_sized(f: Fixture, size: Vec2) -> H`, `click(h: &mut H, label: &str) -> Vec<UiAction>`, `click_at(h: &mut H, pos: Pos2, modifiers: egui::Modifiers) -> Vec<UiAction>`, `type_into(h: &mut H, label: &str, k: usize, text: &str, end: egui::Key) -> Vec<UiAction>`, `rect_of(h: &H, label: &str) -> egui::Rect`, `drawn_texts(h: &H) -> Vec<String>`; the Outliner hooks `Recorded`, `recorded_id()`, `record(...)` (same as today) and `rows(h: &H) -> Vec<Recorded>`. In `ui::materials`, `pub(super) fn palette_label(c: [u8; 3]) -> String`.

- [ ] **Step 1: Add the dev-dependency**

`Cargo.toml`, `[workspace.dependencies]`, after `egui-winit`:

```toml
egui_kittest = "=0.36.2"
```
`crates/app/Cargo.toml`, `[dev-dependencies]`:

```toml
egui_kittest.workspace = true
```
Run `cargo tree -p app -e normal,dev -i egui --depth 1` and check every line shows `egui v0.36.2` (one egui in the graph). If cargo reports a second egui or a resolution failure, stop and report it.

- [ ] **Step 2: Give the controls the tests need accessible names**

Behaviour-free; each name is what a screen reader will also read.

In `toolbar.rs`, a helper used by every File, Edit and Camera menu item that has a shortcut text, so the item's name is its label alone:

```rust
/// A menu item showing `key` on its right, named `label` alone for AccessKit.
fn item(ui: &mut Ui, label: &str, key: &str, enabled: bool) -> egui::Response {
    let response = ui.add_enabled(enabled, egui::Button::new(label).shortcut_text(key));
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, response.enabled(), label)
    });
    response
}
```
Replace `ui.add(egui::Button::new(label).shortcut_text(key))` (File items, Zoom Extents) and `ui.add_enabled(view.has_selection, erase)` (Erase Selection) with `item(...)`.

In `fields.rs` (`drag_row`), keep the label's response and label every field by it:

```rust
    let label_id = if N > 1 { Some(ui.label(label).id) } else { None };
    // ...inside the horizontal closure:
            let label_id = label_id.unwrap_or_else(|| ui.label(label).id);
            for value in &mut values {
                let field = ui
                    .add(
                        egui::DragValue::new(value)
                            .range(range.clone())
                            .speed(speed)
                            .max_decimals(3)
                            .update_while_editing(false),
                    )
                    .labelled_by(label_id);
```
In `dialogs.rs` (Soften Edges) and `materials.rs` (Opacity, Glow), keep the label's id and add `.labelled_by(id)` to the `Slider` response, for example:

```rust
                let label = ui.label("Angle");
                if ui
                    .add(egui::Slider::new(&mut value, 0..=180).suffix("°"))
                    .labelled_by(label.id)
                    .changed()
```
In `materials.rs`, the palette swatches and the name field:

```rust
/// How a palette swatch is named for AccessKit: `Color #C45440`.
pub(super) fn palette_label(c: [u8; 3]) -> String {
    format!("Color #{:02X}{:02X}{:02X}", c[0], c[1], c[2])
}
```
```rust
        for color in MATERIAL_PALETTE {
            let response = swatch(ui, rgb(color));
            response.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Button, true, palette_label(color))
            });
```
```rust
    let edit = ui.add(egui::TextEdit::singleline(&mut text).desired_width(f32::INFINITY));
    edit.widget_info(|| {
        let mut info = egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "Material name");
        info.current_text_value = Some(text.clone());
        info
    });
```
Run `cargo test -p app`: the sweep tests still pass (nothing moved).

- [ ] **Step 3: Write the harness**

`crates/app/src/ui/tests/mod.rs`:

```rust
//! The panels' tests, driven through egui_kittest: widgets are found by their names in the
//! AccessKit tree, so the tests do not depend on where the layout puts them.

mod entity;
mod layout;
mod materials;
mod menus;
mod outliner;

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use egui::{Pos2, Rect, Ui, Vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use kernel::{EntityId, TexturePlacement};
use session::{Lighting, MaterialId, OutlinerTable, Projection};
use tools::ToolKind;

use super::{Agents, MaterialRow, TexturePosition, TextureRow, UiAction, View, draw};
use crate::editor::{ImportUnits, Status};

/// Everything a test's `View` borrows, owned, and what the panels reported.
pub(super) struct Fixture {
    pub tool: ToolKind,
    pub status: Status,
    pub info: Vec<String>,
    pub has_selection: bool,
    pub has_instance_selected: bool,
    pub inside_instance: bool,
    pub has_guides: bool,
    pub projection: Projection,
    pub lighting: Lighting,
    pub flying: bool,
    pub import_units: ImportUnits,
    pub materials: Vec<MaterialRow>,
    pub material: Option<MaterialId>,
    pub outliner: OutlinerTable,
    pub context: Vec<EntityId>,
    pub selection: BTreeSet<EntityId>,
    pub connected: usize,
    pub log: Option<Vec<String>>,
    pub recovered: Vec<PathBuf>,
    pub soften: Option<u8>,
    pub edges_soft: Option<bool>,
    pub texture_position: Option<TexturePosition>,
    /// Every action the panels reported since the last `take`.
    pub actions: Vec<UiAction>,
    /// The 3D view's rectangle in the last frame.
    pub central: Rect,
}

impl Default for Fixture {
    fn default() -> Self {
        Self {
            tool: ToolKind::Line,
            status: Status {
                tool: "Line",
                prompt: "Click to start a line".into(),
                label: "Length",
                value: Some("2.5_".into()),
                message: Some("Not a length: x".into()),
                lock: None,
            },
            info: vec!["Nothing selected".into()],
            has_selection: false,
            has_instance_selected: false,
            inside_instance: false,
            has_guides: false,
            projection: Projection::Perspective,
            lighting: Lighting::Day,
            flying: false,
            import_units: ImportUnits::Meters,
            materials: Vec::new(),
            material: None,
            outliner: OutlinerTable {
                roots: Vec::new(),
                by_definition: BTreeMap::new(),
            },
            context: Vec::new(),
            selection: BTreeSet::new(),
            connected: 0,
            log: None,
            recovered: Vec::new(),
            soften: None,
            edges_soft: None,
            texture_position: None,
            actions: Vec::new(),
            central: Rect::NOTHING,
        }
    }
}

impl Fixture {
    pub fn view(&self) -> View<'_> {
        View {
            agents: Agents {
                connected: self.connected,
                log: self.log.as_deref(),
                recovered: &self.recovered,
            },
            tool: self.tool,
            status: &self.status,
            info: &self.info,
            has_selection: self.has_selection,
            has_instance_selected: self.has_instance_selected,
            inside_instance: self.inside_instance,
            has_guides: self.has_guides,
            projection: self.projection,
            lighting: self.lighting,
            flying: self.flying,
            import_units: self.import_units,
            materials: &self.materials,
            material: self.material,
            outliner: &self.outliner,
            context: &self.context,
            selection: &self.selection,
            soften: self.soften,
            edges_soft: self.edges_soft,
            texture_position: self.texture_position.clone(),
        }
    }

    /// The actions reported since the last call.
    pub fn take(&mut self) -> Vec<UiAction> {
        std::mem::take(&mut self.actions)
    }
}

pub(super) type H = Harness<'static, Fixture>;

/// The window's default size (see `app.rs`).
pub(super) const SCREEN: Vec2 = Vec2::new(1280.0, 800.0);

pub(super) fn harness(f: Fixture) -> H {
    harness_sized(f, SCREEN)
}

/// A harness drawing the panels for `f` in a window of `size` points, settled, with nothing
/// reported yet. Frames are 50 ms apart, so two clicks in a row are a double-click.
pub(super) fn harness_sized(f: Fixture, size: Vec2) -> H {
    let mut h = Harness::builder()
        .with_size(size)
        .with_step_dt(0.05)
        .build_ui_state(
            |ui: &mut Ui, f: &mut Fixture| {
                // A root over the whole window, as `Context::run_ui` gives the app: the
                // harness's own Ui has a margin.
                let ctx = ui.ctx().clone();
                let mut root = Ui::new(
                    ctx.clone(),
                    egui::Id::new("panels"),
                    egui::UiBuilder::new()
                        .layer_id(egui::LayerId::background())
                        .max_rect(ctx.viewport_rect()),
                );
                let (actions, central) = draw(&mut root, &f.view());
                f.actions.extend(actions);
                f.central = central;
            },
            f,
        );
    h.run_ok();
    h.state_mut().actions.clear();
    h
}

/// Clicks the widget named `label` and runs the frames that follow. Returns the actions.
pub(super) fn click(h: &mut H, label: &str) -> Vec<UiAction> {
    h.get_by_label(label).click();
    h.run_ok();
    h.state_mut().take()
}

/// Presses and releases the primary button at `pos` with `modifiers` held, then runs.
pub(super) fn click_at(h: &mut H, pos: Pos2, modifiers: egui::Modifiers) -> Vec<UiAction> {
    h.event(egui::Event::PointerMoved(pos));
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers,
        });
    }
    h.run_ok();
    h.state_mut().take()
}

/// Clicks the `k`th field named `label`, types `text`, presses `end` and runs. Returns the
/// actions.
pub(super) fn type_into(h: &mut H, label: &str, k: usize, text: &str, end: egui::Key) -> Vec<UiAction> {
    h.get_all_by_label(label)
        .nth(k)
        .unwrap_or_else(|| panic!("field {k} named {label}"))
        .click();
    h.run_ok();
    h.event(egui::Event::Text(text.into()));
    h.run_ok();
    h.key_press(end);
    h.run_ok();
    h.state_mut().take()
}

/// Where the widget named `label` is, in points.
pub(super) fn rect_of(h: &H, label: &str) -> Rect {
    h.get_by_label(label).rect()
}

/// Every text the last frame painted (tooltips and shortcut texts are not in the AccessKit
/// tree under their own names).
pub(super) fn drawn_texts(h: &H) -> Vec<String> {
    fn collect(shape: &egui::Shape, out: &mut Vec<String>) {
        match shape {
            egui::Shape::Text(t) => out.push(t.galley.text().to_string()),
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| collect(s, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    for clipped in &h.output().shapes {
        collect(&clipped.shape, &mut out);
    }
    out
}

/// A row the Outliner laid out in the last frame: a node's name (or the Model row, with an
/// empty path) or a node's expand button, with the marks it was drawn with.
#[derive(Clone, Debug)]
pub(super) struct Recorded {
    pub path: Vec<EntityId>,
    pub rect: Rect,
    pub toggle: bool,
    pub selected: bool,
    pub open: bool,
}
```
Then move `recorded_id()` and `record(...)` from `sweep_tests.rs` here unchanged (keep `sweep_tests.rs`'s copies until Step 6; point `outliner.rs`'s hooks at `super::tests::` now, and make `sweep_tests.rs` use `super::tests::{Recorded, recorded_id}`), and add:

```rust
/// The Outliner's rows in the last frame.
pub(super) fn rows(h: &H) -> Vec<Recorded> {
    h.ctx
        .data(|d| d.get_temp::<Vec<Recorded>>(recorded_id()))
        .unwrap_or_default()
}

/// Faces 3, 4, ... as entity ids.
pub(super) fn faces(ids: &[u64]) -> Vec<EntityId> {
    ids.iter().map(|&k| EntityId::Face(kernel::FaceId(k))).collect()
}

/// The Brick material, id 31, untextured.
pub(super) fn brick() -> MaterialRow {
    MaterialRow {
        id: MaterialId(31),
        name: "Brick".into(),
        color: [180, 60, 40],
        opacity: 1.0,
        emission: 0.0,
        texture: None,
    }
}

/// A texture shown with a material: 1 x 0.5 m, 2 x 1 px.
pub(super) fn small_texture() -> TextureRow {
    TextureRow {
        swatch: egui::TextureId::default(),
        width: 1.0,
        height: 0.5,
        pixels: [2, 1],
    }
}

/// Faces selected with their texture placement shown in Entity Info.
pub(super) fn with_texture_position(faces: Vec<EntityId>, placement: TexturePlacement) -> Fixture {
    Fixture {
        info: vec![format!("{} faces", faces.len())],
        texture_position: Some(TexturePosition { placement, faces }),
        ..Default::default()
    }
}
```
In `ui/mod.rs`: `#[cfg(test)] mod tests;` next to `mod sweep_tests;`.

Write one smoke test first in `tests/menus.rs` to check the API names and the root-Ui setup:

```rust
use super::*;

#[test]
fn the_file_menu_reports_its_items() {
    for (item, action) in [
        ("New", UiAction::NewModel),
        ("Open...", UiAction::OpenFile),
        ("Save", UiAction::SaveFile),
        ("Save As...", UiAction::SaveFileAs),
        ("Import...", UiAction::ImportFile),
        ("Export...", UiAction::ExportFile),
    ] {
        let mut h = harness(Fixture::default());
        click(&mut h, "File");
        assert_eq!(click(&mut h, item), [action], "{item}");
    }
}
```
Run `cargo test -p app ui::tests::menus`. Expected: PASS. If `NodeT`, `get_all_by_label` or `rect()` are named differently in kittest 0.4.0, fix the imports from `cargo doc -p egui_kittest --no-deps` before writing more.

- [ ] **Step 4: Write the twins**

Each twin keeps its sweep's name and asserts the same behaviour. `tests/menus.rs` (continued):

```rust
#[test]
fn the_view_menu_and_the_indicator_open_agent_activity() {
    let mut h = harness(Fixture::default());
    assert!(rect_of(&h, "View").min.x > rect_of(&h, "Camera").min.x, "View right of Camera");
    click(&mut h, "View");
    assert_eq!(click(&mut h, "Agent Activity"), [UiAction::ShowAgentActivity(true)]);

    // Status bar: nothing at 0 agents, "Agents: 2" at 2, and a click on it opens the log.
    let h = harness(Fixture::default());
    assert!(h.query_by_label_contains("Agents:").is_none());
    let mut h = harness(Fixture { connected: 2, ..Default::default() });
    let at = rect_of(&h, "Agents: 2");
    assert!(at.min.y >= h.state().central.max.y, "in the status bar: {at:?}");
    assert_eq!(click(&mut h, "Agents: 2"), [UiAction::ShowAgentActivity(true)]);

    // The window lists the log and the recovery files, and closes.
    let mut h = harness(Fixture {
        connected: 2,
        log: Some(vec!["5 s ago  #1  execute save /a.skr  ok".into()]),
        recovered: vec![PathBuf::from("/r/untitled-1.skr")],
        ..Default::default()
    });
    for want in ["2 agents connected", "5 s ago  #1  execute save /a.skr  ok", "/r/untitled-1.skr"] {
        h.get_by_label(want);
    }
    assert_eq!(click(&mut h, "Close window"), [UiAction::ShowAgentActivity(false)]);
}

/// Opens the Camera menu over a fixture from `f` and clicks `item`.
fn camera_menu(f: impl Fn() -> Fixture, item: &str) -> Vec<UiAction> {
    let mut h = harness(f());
    click(&mut h, "Camera");
    click(&mut h, item)
}

#[test]
fn the_camera_menu_has_views_zoom_extents_and_projections() {
    use session::StandardView;
    let plain = Fixture::default;
    assert_eq!(camera_menu(plain, "Zoom Extents"), [UiAction::ZoomExtents]);
    for v in StandardView::ALL {
        assert_eq!(camera_menu(plain, v.label()), [UiAction::StandardView(v)], "{v:?}");
    }
    assert_eq!(
        camera_menu(plain, "Parallel Projection"),
        [UiAction::SetProjection(Projection::Parallel)]
    );
    assert_eq!(camera_menu(plain, "Fly mode (F)"), [UiAction::ToggleFly]);
    assert_eq!(camera_menu(plain, "Night lighting"), [UiAction::SetLighting(Lighting::Night)]);
    let parallel_night = || Fixture {
        projection: Projection::Parallel,
        lighting: Lighting::Night,
        ..Default::default()
    };
    assert_eq!(
        camera_menu(parallel_night, "Perspective"),
        [UiAction::SetProjection(Projection::Perspective)]
    );
    // Unchecking Night lighting goes back to day.
    assert_eq!(camera_menu(parallel_night, "Night lighting"), [UiAction::SetLighting(Lighting::Day)]);
}

/// A harness with the Edit menu open.
fn edit_menu(has_selection: bool, has_guides: bool) -> H {
    let mut h = harness(Fixture { has_selection, has_guides, ..Default::default() });
    click(&mut h, "Edit");
    h
}

#[test]
fn the_edit_menu_erases_the_selection_and_deletes_guides() {
    for (item, action) in [
        ("Erase Selection", UiAction::EraseSelection),
        ("Delete Guides", UiAction::DeleteGuides),
        ("Soften Edges...", UiAction::ShowSoften(true)),
    ] {
        let mut h = edit_menu(true, true);
        assert_eq!(click(&mut h, item), [action], "{item}");
    }
}

#[test]
fn the_edit_menu_items_need_a_selection_and_guides() {
    for item in ["Erase Selection", "Delete Guides"] {
        let mut h = edit_menu(false, false);
        assert!(h.get_by_label(item).accesskit_node().is_disabled(), "{item}");
        assert_eq!(click(&mut h, item), [], "{item}");
    }
    // Soften Edges... works on the open context.
    let mut h = edit_menu(false, false);
    assert_eq!(click(&mut h, "Soften Edges..."), [UiAction::ShowSoften(true)]);
}

#[test]
fn the_soften_window_sets_its_angle_softens_hardens_and_closes() {
    let open = || Fixture { soften: Some(20), ..Default::default() };
    for (label, action) in [
        ("Soften", UiAction::Soften),
        ("Harden", UiAction::Harden),
        ("Close window", UiAction::ShowSoften(false)),
    ] {
        let mut h = harness(open());
        assert_eq!(click(&mut h, label), [action], "{label}");
    }
    // A click near the slider's left end sets a small angle, three quarters along a large one.
    let mut angles = Vec::new();
    for along in [0.04, 0.75] {
        let mut h = harness(open());
        let slider = rect_of(&h, "Angle");
        let width = h.ctx.style().spacing.slider_width;
        let at = Pos2::new(slider.min.x + along * width, slider.center().y);
        for a in click_at(&mut h, at, egui::Modifiers::NONE) {
            if let UiAction::SetSoftenAngle(angle) = a {
                angles.push(angle);
            }
        }
    }
    assert!(angles.iter().any(|a| *a < 20), "{angles:?}");
    assert!(angles.iter().any(|a| *a > 90), "{angles:?}");
    assert!(angles.iter().all(|a| *a <= 180), "{angles:?}");
    // Closed: no window to click.
    let h = harness(Fixture::default());
    for label in ["Soften", "Harden", "Angle"] {
        assert!(h.query_by_label(label).is_none(), "{label}");
    }
}
```
`tests/entity.rs`:

```rust
use super::*;

#[test]
fn entity_info_shows_a_soft_checkbox_for_selected_edges() {
    for (soft, wanted) in [(false, UiAction::SetSoft(true)), (true, UiAction::SetSoft(false))] {
        let mut h = harness(Fixture {
            info: vec!["1 edge".into()],
            edges_soft: Some(soft),
            ..Default::default()
        });
        assert_eq!(click(&mut h, "Soft"), [wanted]);
    }
    assert!(harness(Fixture::default()).query_by_label("Soft").is_none());
}

/// The texture position fields, each with the action typing 2 into it for `faces` makes.
fn texture_fields(faces: &[EntityId]) -> [(&'static str, usize, UiAction); 5] {
    let f = faces.to_vec();
    [
        ("Offset", 0, UiAction::SetTextureOffset(f.clone(), [2000, 0])),
        ("Offset", 1, UiAction::SetTextureOffset(f.clone(), [0, 2000])),
        ("Rotation", 0, UiAction::SetTextureRotation(f.clone(), 20)),
        ("Scale", 0, UiAction::SetTextureScale(f.clone(), [2000, 1000])),
        ("Scale", 1, UiAction::SetTextureScale(f, [1000, 2000])),
    ]
}

#[test]
fn entity_info_shows_texture_position_fields_for_selected_faces() {
    let selected = faces(&[3, 4]);
    let fixture = || with_texture_position(selected.clone(), TexturePlacement::default());
    for (label, action) in [
        ("Rotate 90", UiAction::RotateTexture90),
        ("Reset", UiAction::ResetTexturePosition),
    ] {
        let mut h = harness(fixture());
        assert_eq!(click(&mut h, label), [action], "{label}");
    }
    // Typing 2 into each field sets that field alone, in its own units.
    for (label, k, action) in texture_fields(&selected) {
        let mut h = harness(fixture());
        assert_eq!(type_into(&mut h, label, k, "2", egui::Key::Enter), [action], "{label} {k}");
    }
    // Without faces selected there are no texture fields.
    let h = harness(Fixture::default());
    for label in ["Offset", "Rotation", "Scale", "Rotate 90", "Reset"] {
        assert!(h.query_by_label(label).is_none(), "{label}");
    }
}

#[test]
fn a_texture_field_applies_to_the_faces_it_was_edited_for() {
    // Typing into a field with face 3 selected, then entering it once face 4 (placed
    // otherwise) is selected, sets face 3 from face 3's values.
    let (three, four) = (faces(&[3]), faces(&[4]));
    let other = TexturePlacement {
        offset: [0.5, 0.75],
        rotation_deg: 30.0,
        scale: [3.0, 3.0],
    };
    for (label, k, action) in texture_fields(&three) {
        let mut h = harness(with_texture_position(three.clone(), TexturePlacement::default()));
        h.get_all_by_label(label).nth(k).unwrap().click();
        h.run_ok();
        h.event(egui::Event::Text("2".into()));
        h.run_ok();
        h.state_mut().texture_position = Some(TexturePosition {
            placement: other,
            faces: four.clone(),
        });
        h.key_press(egui::Key::Enter);
        h.run_ok();
        assert_eq!(h.state_mut().take(), [action], "{label} {k}");
    }
}

#[test]
fn a_stepped_texture_value_applies_when_its_fields_go_away() {
    // Stepping a field with the arrow keys on face 3, then selecting something else (the
    // fields go away), applies the stepped value to face 3.
    let three = faces(&[3]);
    let texture = |a: &UiAction| {
        matches!(
            a,
            UiAction::SetTextureOffset(..) | UiAction::SetTextureRotation(..) | UiAction::SetTextureScale(..)
        )
    };
    let mut acted = Vec::new();
    for (label, k) in [("Offset", 0), ("Offset", 1), ("Rotation", 0), ("Scale", 0), ("Scale", 1)] {
        let mut h = harness(with_texture_position(three.clone(), TexturePlacement::default()));
        h.get_all_by_label(label).nth(k).unwrap().click();
        h.run_ok();
        for _ in 0..2 {
            h.key_press(egui::Key::ArrowUp);
            h.run_ok();
        }
        h.state_mut().take();
        h.state_mut().texture_position = None;
        h.run_ok();
        acted.extend(h.state_mut().take().into_iter().filter(texture));
        // Faces selected again later: nothing left over applies then.
        h.state_mut().texture_position = Some(TexturePosition {
            placement: TexturePlacement::default(),
            faces: three.clone(),
        });
        h.run_ok();
        let later: Vec<_> = h.state_mut().take().into_iter().filter(texture).collect();
        assert!(later.is_empty(), "{label} {k}: {later:?}");
    }
    assert!(acted.contains(&UiAction::SetTextureRotation(three.clone(), 5)), "{acted:?}");
    assert!(
        acted.iter().all(|a| matches!(
            a,
            UiAction::SetTextureOffset(f, _) | UiAction::SetTextureRotation(f, _) | UiAction::SetTextureScale(f, _)
                if *f == three
        )),
        "{acted:?}"
    );
}
```
`tests/layout.rs`:

```rust
use super::*;
use crate::ui::outliner::OUTLINER_MIN_HEIGHT;
use kernel::DefinitionKind;
use session::OutlinerRow;

#[test]
fn panels_leave_a_central_view_inside_the_window() {
    let h = harness(Fixture::default());
    let rect = h.state().central;
    assert!(rect.min.x == 0.0 && rect.min.y > 0.0, "toolbar above: {rect:?}");
    assert!(rect.max.x < 1280.0, "info panel on the right: {rect:?}");
    assert!(rect.max.y < 800.0, "status bar below: {rect:?}");
    assert!(rect.width() > 600.0 && rect.height() > 500.0, "{rect:?}");
}

/// A fixture with every toolbar button enabled.
fn toolbar_fixture() -> Fixture {
    Fixture {
        has_selection: true,
        has_instance_selected: true,
        inside_instance: true,
        has_guides: true,
        ..Default::default()
    }
}

/// The toolbar's buttons other than the tools, by name.
fn toolbar_buttons() -> Vec<(&'static str, UiAction)> {
    vec![
        ("Undo", UiAction::Undo),
        ("Redo", UiAction::Redo),
        ("Make group", UiAction::MakeGroup),
        ("Make component", UiAction::MakeComponent),
        ("Explode", UiAction::Explode),
        ("Close group", UiAction::CloseInstance),
    ]
}

/// Every toolbar button reports itself in a window of `size`.
fn every_toolbar_button_works(size: Vec2) {
    for kind in ToolKind::ALL {
        let mut h = harness_sized(toolbar_fixture(), size);
        assert_eq!(click(&mut h, kind.label()), [UiAction::Tool(kind)], "{kind:?} at {size:?}");
    }
    for (label, action) in toolbar_buttons() {
        let mut h = harness_sized(toolbar_fixture(), size);
        assert_eq!(click(&mut h, label), [action], "{label} at {size:?}");
    }
}

/// The 3D view's top (the toolbar's height) in a window `width` points wide.
fn view_top(width: f32) -> f32 {
    harness_sized(toolbar_fixture(), Vec2::new(width, 800.0)).state().central.min.y
}

#[test]
fn clicking_a_tool_button_reports_it() {
    every_toolbar_button_works(SCREEN);
    // The default window keeps the toolbar on one row, as a very wide one does.
    assert_eq!(view_top(1280.0), view_top(2400.0), "one toolbar row at 1280 points");
}

#[test]
fn a_narrow_window_wraps_the_toolbar_and_keeps_every_button() {
    let one_row = view_top(2400.0);
    for width in [900.0, 640.0] {
        every_toolbar_button_works(Vec2::new(width, 800.0));
        let top = view_top(width);
        assert!(top > one_row + 10.0, "the toolbar wraps at {width} points: {top} vs {one_row}");
        assert!(top < 200.0, "{top} points of toolbar at {width}");
        let central = harness_sized(toolbar_fixture(), Vec2::new(width, 800.0)).state().central;
        assert!(central.width() > 0.0 && central.height() > 0.0, "a 3D view at {width}: {central:?}");
    }
}

#[test]
fn the_right_panel_keeps_its_width_with_a_texture_shown() {
    let mut h = harness(Fixture {
        materials: vec![brick()],
        material: Some(MaterialId(31)),
        ..Default::default()
    });
    let width = |h: &mut H| {
        for _ in 0..3 {
            h.run_ok();
        }
        h.state().central.width()
    };
    let before = width(&mut h);
    h.state_mut().materials[0].texture = Some(TextureRow {
        swatch: egui::TextureId::default(),
        width: 123.456,
        height: 789.012,
        pixels: [4096, 4096],
    });
    assert_eq!(width(&mut h), before, "a texture shown");
    let shown = h.state_mut().materials[0].texture.take();
    assert_eq!(width(&mut h), before, "after the texture");
    h.state_mut().materials[0].texture = shown;
    h.state_mut().texture_position = Some(TexturePosition {
        placement: TexturePlacement {
            offset: [-123456.789, -987654.321],
            rotation_deg: 359.9,
            scale: [999.999, 999.999],
        },
        faces: faces(&[3]),
    });
    assert_eq!(width(&mut h), before, "texture position");
}

/// `n` materials named "Material 0", "Material 1", ...
pub(super) fn many_materials(n: u64) -> Vec<MaterialRow> {
    (0..n)
        .map(|k| MaterialRow {
            id: MaterialId(100 + k),
            name: format!("Material {k}"),
            color: [10, 20, 30],
            opacity: 1.0,
            emission: 0.0,
            texture: None,
        })
        .collect()
}

/// `n` unnamed groups at the root.
pub(super) fn many_groups(n: u64) -> OutlinerTable {
    OutlinerTable {
        roots: (0..n)
            .map(|k| OutlinerRow {
                id: format!("i{}", 200 + k).parse().unwrap(),
                definition: "d1".parse().unwrap(),
                kind: DefinitionKind::Group,
                name: String::new(),
                material: None,
            })
            .collect(),
        by_definition: BTreeMap::new(),
    }
}

/// A fixture with 40 materials (the first chosen) and `groups` groups.
pub(super) fn crowded(groups: u64) -> Fixture {
    Fixture {
        info: vec!["1 group".into()],
        materials: many_materials(40),
        material: Some(MaterialId(100)),
        outliner: many_groups(groups),
        ..Default::default()
    }
}

#[test]
fn a_short_window_leaves_the_outliner_its_minimum_height() {
    let height = 520.0;
    let h = harness_sized(crowded(40), Vec2::new(1280.0, height));
    let model = rect_of(&h, "Model");
    assert!(
        model.min.y <= height - OUTLINER_MIN_HEIGHT,
        "the Outliner starts at {} in a {height} point window",
        model.min.y
    );
}

#[test]
fn a_very_short_window_scrolls_the_right_panel() {
    let height = 300.0;
    let mut h = harness_sized(crowded(1), Vec2::new(1280.0, height));
    let before = rect_of(&h, "Model").min.y;
    assert!(before > height, "the Outliner is below the window at {before}");
    // Scroll with the pointer over Entity Info.
    h.event(egui::Event::PointerMoved(Pos2::new(1200.0, 60.0)));
    h.event(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: Vec2::new(0.0, -400.0),
        phase: egui::TouchPhase::Move,
        modifiers: egui::Modifiers::NONE,
    });
    h.run_ok();
    let after = rect_of(&h, "Model").min.y;
    assert!(after < before - 100.0, "the panel scrolled from {before} to {after}");
}
```
`tests/materials.rs`:

```rust
use super::*;
use crate::editor::MATERIAL_PALETTE;
use crate::ui::materials::palette_label;

/// Brick (chosen) and Glass.
fn two_materials() -> Fixture {
    let mut glass = brick();
    glass.id = MaterialId(32);
    glass.name = "Glass".into();
    glass.color = [90, 170, 170];
    glass.opacity = 0.4;
    Fixture {
        materials: vec![brick(), glass],
        material: Some(MaterialId(31)),
        ..Default::default()
    }
}

/// Brick, chosen, with `texture`.
fn brick_with(texture: Option<TextureRow>) -> Fixture {
    let mut row = brick();
    row.texture = texture;
    Fixture {
        materials: vec![row],
        material: Some(MaterialId(31)),
        ..Default::default()
    }
}

/// Clicks `along` (0 to 1) of the way across the slider named `label`.
fn click_slider(h: &mut H, label: &str, along: f32) -> Vec<UiAction> {
    let slider = rect_of(h, label);
    let width = h.ctx.style().spacing.slider_width.min(slider.width());
    let at = Pos2::new(slider.min.x + along * width, slider.center().y);
    click_at(h, at, egui::Modifiers::NONE)
}

#[test]
fn the_materials_list_chooses_adds_renames_recolors_and_deletes() {
    let first = palette_label(MATERIAL_PALETTE[0]);
    for (label, action) in [
        ("Default", UiAction::ChooseMaterial(None)),
        ("Brick", UiAction::ChooseMaterial(Some(MaterialId(31)))),
        ("Glass", UiAction::ChooseMaterial(Some(MaterialId(32)))),
        ("Add", UiAction::AddMaterial),
        ("Delete", UiAction::DeleteMaterial(MaterialId(31))),
        (first.as_str(), UiAction::RecolorMaterial(MaterialId(31), MATERIAL_PALETTE[0])),
    ] {
        let mut h = harness(two_materials());
        assert_eq!(click(&mut h, label), [action], "{label}");
    }
    let mut h = harness(two_materials());
    let renamed = type_into(&mut h, "Material name", 0, "Old", egui::Key::Enter);
    assert!(
        matches!(renamed.as_slice(), [UiAction::RenameMaterial(MaterialId(31), name)] if name.contains("Old")),
        "{renamed:?}"
    );
    let mut h = harness(two_materials());
    let opacity = click_slider(&mut h, "Opacity", 0.04);
    assert!(
        matches!(opacity.as_slice(), [UiAction::SetMaterialOpacity(MaterialId(31), p)] if *p < 100),
        "{opacity:?}"
    );
    let mut h = harness(two_materials());
    let glow = click_slider(&mut h, "Glow", 0.75);
    assert!(
        matches!(glow.as_slice(), [UiAction::SetMaterialEmission(MaterialId(31), t)] if *t > 0),
        "{glow:?}"
    );
    // The default material has no name, colors or sliders, and cannot be deleted.
    let mut h = harness(Fixture { material: None, ..two_materials() });
    for label in ["Material name", first.as_str(), "Opacity", "Glow"] {
        assert!(h.query_by_label(label).is_none(), "{label}");
    }
    assert_eq!(click(&mut h, "Delete"), []);
}

#[test]
fn the_materials_list_loads_removes_and_sizes_textures() {
    let mut h = harness(brick_with(None));
    assert_eq!(click(&mut h, "Texture..."), [UiAction::LoadTexture(MaterialId(31))]);
    for label in ["Remove", "Size (m)"] {
        assert!(h.query_by_label(label).is_none(), "{label}");
    }
    for (label, action) in [
        ("Brick", UiAction::ChooseMaterial(Some(MaterialId(31)))),
        ("Texture...", UiAction::LoadTexture(MaterialId(31))),
        ("Remove", UiAction::RemoveTexture(MaterialId(31))),
    ] {
        let mut h = harness(brick_with(Some(small_texture())));
        assert_eq!(click(&mut h, label), [action], "{label}");
    }
    // Typing 2 into the width field, then the height field, sets the size in millimeters.
    for (k, size) in [(0, [2000, 500]), (1, [1000, 2000])] {
        let mut h = harness(brick_with(Some(small_texture())));
        assert_eq!(
            type_into(&mut h, "Size (m)", k, "2", egui::Key::Enter),
            [UiAction::SetTextureSize(MaterialId(31), size)]
        );
    }
}

#[test]
fn a_typed_texture_size_is_one_change_when_entered() {
    for (k, size) in [(0, [250, 500]), (1, [1000, 250])] {
        let mut h = harness(brick_with(Some(small_texture())));
        h.get_all_by_label("Size (m)").nth(k).unwrap().click();
        h.run_ok();
        for key in ["0", ".", "2", "5"] {
            h.event(egui::Event::Text(key.into()));
            h.run_ok();
        }
        assert_eq!(h.state_mut().take(), [], "nothing while typing");
        h.key_press(egui::Key::Enter);
        h.run_ok();
        assert_eq!(h.state_mut().take(), [UiAction::SetTextureSize(MaterialId(31), size)]);
    }
}

#[test]
fn a_dragged_texture_size_applies_to_its_material_when_another_is_chosen() {
    for k in 0..2 {
        let mut h = harness(brick_with(Some(small_texture())));
        let at = h.get_all_by_label("Size (m)").nth(k).unwrap().rect().center();
        h.event(egui::Event::PointerMoved(at));
        h.event(egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        });
        h.run_ok();
        for dx in [10.0, 20.0, 40.0] {
            h.event(egui::Event::PointerMoved(at + Vec2::new(dx, 0.0)));
            h.run_ok();
        }
        h.state_mut().take();
        // Still holding the button, another material is chosen (as the API or a shortcut might).
        h.state_mut().material = None;
        h.run_ok();
        let switched = h.state_mut().take();
        assert!(
            matches!(switched.as_slice(), [UiAction::SetTextureSize(MaterialId(31), _)]),
            "field {k}: {switched:?}"
        );
        h.event(egui::Event::PointerButton {
            pos: at + Vec2::new(40.0, 0.0),
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        });
        h.run_ok();
        h.state_mut().material = Some(MaterialId(31));
        h.run_ok();
        assert_eq!(h.state_mut().take(), [], "field {k}: nothing applies later");
    }
}

#[test]
fn dragging_the_opacity_slider_sets_the_opacity_once_when_it_ends() {
    let mut h = harness(brick_with(None));
    let slider = rect_of(&h, "Opacity");
    let width = h.ctx.style().spacing.slider_width.min(slider.width());
    let start = Pos2::new(slider.min.x + 0.9 * width, slider.center().y);
    let button = |pos, pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    h.event(egui::Event::PointerMoved(start));
    h.event(button(start, true));
    h.run_ok();
    for k in 1..=10 {
        h.event(egui::Event::PointerMoved(start - Vec2::new(3.0 * k as f32, 0.0)));
        h.run_ok();
    }
    assert_eq!(h.state_mut().take(), [], "nothing while dragging");
    h.event(button(start - Vec2::new(30.0, 0.0), false));
    h.run_ok();
    let actions = h.state_mut().take();
    assert!(
        matches!(actions.as_slice(), [UiAction::SetMaterialOpacity(MaterialId(31), p)] if *p < 100),
        "{actions:?}"
    );
}

#[test]
fn escape_in_the_material_name_field_discards_the_edit() {
    let mut h = harness(brick_with(None));
    let entered = type_into(&mut h, "Material name", 0, "Old", egui::Key::Enter);
    assert!(entered.iter().any(|a| matches!(a, UiAction::RenameMaterial(..))), "Enter renames: {entered:?}");
    let mut h = harness(brick_with(None));
    let escaped = type_into(&mut h, "Material name", 0, "Old", egui::Key::Escape);
    assert!(!escaped.iter().any(|a| matches!(a, UiAction::RenameMaterial(..))), "{escaped:?}");
}

#[test]
fn deleting_a_material_does_not_also_rename_it() {
    // Type into the name field, then press and release on Delete in one frame.
    let mut h = harness(brick_with(None));
    h.get_by_label("Material name").click();
    h.run_ok();
    h.event(egui::Event::Text("Old".into()));
    h.run_ok();
    let actions = click(&mut h, "Delete");
    assert!(actions.contains(&UiAction::DeleteMaterial(MaterialId(31))), "{actions:?}");
    assert!(!actions.iter().any(|a| matches!(a, UiAction::RenameMaterial(..))), "{actions:?}");
}
```
`tests/outliner.rs` (the row record stays the way to see open and selected marks):

```rust
use super::*;
use crate::ui::outliner::outliner_label;
use kernel::DefinitionKind;
use session::OutlinerRow;

fn table_row(id: &str, definition: &str, kind: DefinitionKind, name: &str) -> OutlinerRow {
    OutlinerRow {
        id: id.parse().unwrap(),
        definition: definition.parse().unwrap(),
        kind,
        name: name.into(),
        material: None,
    }
}

/// Two "Box" components (definition d1) each placing the group i27 (definition d2).
fn boxes() -> OutlinerTable {
    let component = |id| table_row(id, "d1", DefinitionKind::Component, "Box");
    let group = table_row("i27", "d2", DefinitionKind::Group, "");
    OutlinerTable {
        roots: vec![component("i55"), component("i56")],
        by_definition: [
            ("d1".parse().unwrap(), vec![group]),
            ("d2".parse().unwrap(), Vec::new()),
        ]
        .into(),
    }
}

fn ids(ids: &[&str]) -> Vec<EntityId> {
    ids.iter().map(|id| id.parse().unwrap()).collect()
}

/// The boxes with `context` open and `selection` selected.
fn boxes_fixture(context: &[&str], selection: &[&str]) -> Fixture {
    let selection: BTreeSet<EntityId> = ids(selection).into_iter().collect();
    Fixture {
        info: vec!["1 group".into()],
        has_selection: !selection.is_empty(),
        has_instance_selected: !selection.is_empty(),
        inside_instance: !context.is_empty(),
        outliner: boxes(),
        context: ids(context),
        selection,
        ..Default::default()
    }
}

/// The name row (not the expand button) at `path`.
fn row<'a>(rows: &'a [Recorded], path: &[EntityId]) -> Option<&'a Recorded> {
    rows.iter().find(|r| !r.toggle && r.path == path)
}

fn row_at(rows: &[Recorded], path: &[EntityId]) -> Pos2 {
    row(rows, path)
        .unwrap_or_else(|| panic!("a row for {path:?} in {rows:?}"))
        .rect
        .center()
}

fn toggle_at(rows: &[Recorded], path: &[EntityId]) -> Pos2 {
    rows.iter()
        .find(|r| r.toggle && r.path == path)
        .unwrap_or_else(|| panic!("an expand button for {path:?}"))
        .rect
        .center()
}

#[test]
fn outliner_labels_name_groups_and_components() {
    let table = boxes();
    assert_eq!(outliner_label(&table.roots[0]), "<Box>");
    assert_eq!(outliner_label(&table.children(table.roots[0].definition)[0]), "Group");
    let named = table_row("i3", "d4", DefinitionKind::Group, "Wall");
    assert_eq!(outliner_label(&named), "Wall");
}

#[test]
fn outliner_rows_show_the_open_path_and_the_selection_and_select_their_paths() {
    let fixture = || boxes_fixture(&["i55"], &["i27"]);
    let laid = rows(&harness(fixture()));
    // The first box is open, so expanded; the second starts collapsed.
    let names: Vec<&[EntityId]> = laid.iter().filter(|r| !r.toggle).map(|r| r.path.as_slice()).collect();
    assert_eq!(names, [ids(&[]), ids(&["i55"]), ids(&["i55", "i27"]), ids(&["i56"])]);
    let marks = |path: &[&str]| {
        let r = row(&laid, &ids(path)).unwrap();
        (r.open, r.selected)
    };
    assert_eq!(marks(&["i55"]), (true, false));
    assert_eq!(marks(&["i55", "i27"]), (false, true));
    assert_eq!(marks(&["i56"]), (false, false));
    for (path, modifiers, toggle) in [
        (&["i55"][..], egui::Modifiers::NONE, false),
        (&["i55", "i27"], egui::Modifiers::NONE, false),
        (&["i56"], egui::Modifiers::NONE, false),
        (&["i56"], egui::Modifiers::SHIFT, true),
        (&["i56"], egui::Modifiers::COMMAND, true),
        (&["i56"], egui::Modifiers::CTRL, true),
    ] {
        // A fresh harness for each, so no two clicks make a double-click.
        let mut h = harness(fixture());
        let clicked = click_at(&mut h, row_at(&laid, &ids(path)), modifiers);
        let wanted = UiAction::OutlinerSelect { path: ids(path), toggle };
        assert_eq!(clicked, [wanted], "{path:?} with {modifiers:?}");
    }
}

#[test]
fn the_model_row_goes_back_to_the_root_only_from_inside() {
    for (context, wanted) in [(&["i55"][..], vec![UiAction::OpenPath(Vec::new())]), (&[][..], Vec::new())] {
        let mut h = harness(boxes_fixture(context, &[]));
        let laid = rows(&h);
        let model = row(&laid, &[]).unwrap();
        assert_eq!(model.open, context.is_empty(), "bold at the root");
        assert_eq!(click_at(&mut h, model.rect.center(), egui::Modifiers::NONE), wanted);
    }
}

#[test]
fn double_clicking_an_outliner_row_opens_it_and_expanding_shows_children() {
    let mut h = harness(boxes_fixture(&["i55"], &[]));
    let inner = row_at(&rows(&h), &ids(&["i55", "i27"]));
    let mut actions = click_at(&mut h, inner, egui::Modifiers::NONE);
    actions.extend(click_at(&mut h, inner, egui::Modifiers::NONE));
    assert!(actions.contains(&UiAction::OpenPath(ids(&["i55", "i27"]))), "{actions:?}");
    // The expand button left of the second box shows its group.
    let mut h = harness(boxes_fixture(&["i55"], &[]));
    let button = toggle_at(&rows(&h), &ids(&["i56"]));
    assert_eq!(click_at(&mut h, button, egui::Modifiers::NONE), []);
    let wanted = UiAction::OutlinerSelect { path: ids(&["i56", "i27"]), toggle: false };
    let at = row_at(&rows(&h), &ids(&["i56", "i27"]));
    // Far enough from the last click in time not to make a double-click.
    h.run_steps(10);
    assert_eq!(click_at(&mut h, at, egui::Modifiers::NONE), [wanted]);
}

#[test]
fn a_newly_opened_path_expands_its_ancestors() {
    let mut h = harness(boxes_fixture(&[], &[]));
    assert!(row(&rows(&h), &ids(&["i56", "i27"])).is_none(), "collapsed");
    let context = ids(&["i56", "i27"]);
    h.state_mut().context = context.clone();
    h.state_mut().inside_instance = true;
    h.run_ok();
    let laid = rows(&h);
    assert!(row(&laid, &context).expect("the open group shows").open);
    // Collapsing it by hand sticks while the context stays the same.
    click_at(&mut h, toggle_at(&laid, &ids(&["i56"])), egui::Modifiers::NONE);
    h.run_ok();
    assert!(row(&rows(&h), &context).is_none(), "{:?}", rows(&h));
}
```
- [ ] **Step 5: Run both suites**

```bash
cargo test -p app ui::
```
Expected: the 28 sweeps and the 28 twins pass (56 UI tests). A twin that fails while its sweep passes is a twin bug or a missing accessible name; fix the twin (or add the name as in Step 2), never the behaviour. Then `cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --all`.

- [ ] **Step 6: Commit with both suites, then delete the sweeps**

```bash
git status && git diff --stat
git add Cargo.toml Cargo.lock crates/app/Cargo.toml crates/app/src/ui/
git commit -m "UI tests by label through egui_kittest"
git rm crates/app/src/ui/sweep_tests.rs
```
Remove `#[cfg(test)] mod sweep_tests;` from `ui/mod.rs`. Run the full gate; the count is back to `1159`:

```bash
cargo test --workspace 2>&1 | grep "test result" | awk '{s+=$4} END {print s}'
cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --all --check
git add crates/app/src/ui/mod.rs
git commit -m "Remove the coordinate-sweep UI tests"
```

---

### Task 3: Theme, fonts, and a right panel sized by measurement

The tokens and fonts land, and the right panel stops relying on heights tuned to the old font so the taller 28 pt controls never clip a row.

**Files:**
- Modify: `Cargo.toml` (`egui-phosphor`), `crates/app/Cargo.toml` (`egui-phosphor`, `serde`)
- Create: `crates/app/assets/fonts/Inter-Regular.ttf`, `Inter-SemiBold.ttf`, `JetBrainsMono-Regular.ttf`, `OFL-Inter.txt`, `OFL-JetBrainsMono.txt`, `README.md`
- Create: `crates/app/src/ui/theme.rs`, `crates/app/src/ui/fonts.rs`
- Modify: `crates/app/src/ui/mod.rs` (modules, `RIGHT_PANEL_WIDTH` 200, the right panel's least height), `right.rs` (measured layout), `materials.rs` (list in whole rows), `outliner.rs` (least rows), `toolbar.rs` (local spacing)
- Modify: `crates/app/src/gui.rs` (`Gui::new` installs the theme; passes `window.theme()`), `crates/app/src/app.rs` (both `Gui::new` calls)
- Modify: `crates/app/src/ui/tests/mod.rs` (harness installs the theme), `tests/layout.rs`

**Interfaces:**
- Consumes: Task 2's harness.
- Produces: `ui::theme::{ThemeChoice (System | Dark | Light, Default System, serde lowercase), Tokens, DARK, LIGHT, style(t: &Tokens, dark: bool) -> egui::Style, install(ctx: &egui::Context, choice: ThemeChoice), set_choice(ctx: &egui::Context, choice: ThemeChoice)}` and the metrics `GRID`, `CONTROL_HEIGHT`, `CONTROL_RADIUS`, `WINDOW_RADIUS`, `FOCUS_RING`, `BODY_SIZE`, `SMALL_SIZE`. Later tasks add `tokens` and `SWATCH_RADIUS` (4), `SECTION_SIZE`, `SECTION_GAP` and `SWATCH_SIZE` (5), `view_style` (6), `ICON_SIZE` (7), `ThemeChoice::ALL` and `label` (9), each with its first user: an unused item fails clippy in a binary crate. `ui::fonts::{definitions() -> egui::FontDefinitions, semibold() -> egui::FontFamily, INTER, INTER_SEMIBOLD, MONO}`. `ui::right::{MATERIAL_ROWS_MAX, MATERIAL_ROWS_MIN, OUTLINER_ROWS_MIN, row_pitch(style: &egui::Style) -> f32, least_height(ui: &Ui) -> f32}`. `Gui::new(window, device, format, theme: ThemeChoice)`.

- [ ] **Step 1: Fetch and check the fonts**

```bash
D=$(mktemp -d) && cd "$D"
curl -fsSL -o inter.zip https://github.com/rsms/inter/releases/download/v4.1/Inter-4.1.zip
curl -fsSL -o jbm.zip https://github.com/JetBrains/JetBrainsMono/releases/download/v2.304/JetBrainsMono-2.304.zip
unzip -q inter.zip extras/ttf/Inter-Regular.ttf extras/ttf/Inter-SemiBold.ttf LICENSE.txt -d inter
unzip -q jbm.zip fonts/ttf/JetBrainsMono-Regular.ttf OFL.txt -d jbm
shasum -a 256 inter/extras/ttf/Inter-Regular.ttf inter/extras/ttf/Inter-SemiBold.ttf \
  jbm/fonts/ttf/JetBrainsMono-Regular.ttf inter/LICENSE.txt jbm/OFL.txt
```
Expected, in order:

```
40d692fce188e4471e2b3cba937be967878f631ad3ebbbdcd587687c7ebe0c82
78a843fade9d4612a5567302fb595b56976eb5fcebf4fea5a5912d638bafcde3
a0bf60ef0f83c5ed4d7a75d45838548b1f6873372dfac88f71804491898d138f
262481e844521b326f5ecd053e59b98c8b2da78c8ee1bdbb6e8174305e54935a
30f0c136e3c88e422d0791acd97238870f9054a9729bc34cf2ff0d4ed8cac4ad
```
Any other hash: stop and report. Then copy into the repo:

```bash
F=~/Desktop/side_projects/sketchup/crates/app/assets/fonts && mkdir -p "$F"
cp inter/extras/ttf/Inter-Regular.ttf inter/extras/ttf/Inter-SemiBold.ttf jbm/fonts/ttf/JetBrainsMono-Regular.ttf "$F"/
cp inter/LICENSE.txt "$F"/OFL-Inter.txt && cp jbm/OFL.txt "$F"/OFL-JetBrainsMono.txt
cd ~/Desktop/side_projects/sketchup && rm -rf "$D"
```
`crates/app/assets/fonts/README.md`:

```markdown
# Fonts built into the app

Licensed under the SIL Open Font License 1.1 (texts beside them).

| File | Source | SHA-256 |
|---|---|---|
| Inter-Regular.ttf, Inter-SemiBold.ttf | Inter 4.1, https://github.com/rsms/inter/releases/download/v4.1/Inter-4.1.zip (`extras/ttf/`) | 40d692fc...e0c82, 78a843fa...afcde3 |
| JetBrainsMono-Regular.ttf | JetBrains Mono 2.304, https://github.com/JetBrains/JetBrainsMono/releases/download/v2.304/JetBrainsMono-2.304.zip (`fonts/ttf/`) | a0bf60ef...d138f |

The Phosphor icon font comes with the `egui-phosphor` crate (MIT).
```
- [ ] **Step 2: Add the dependencies**

`Cargo.toml` `[workspace.dependencies]`: `egui-phosphor = "=0.14.0"`. `crates/app/Cargo.toml` `[dependencies]`: `egui-phosphor.workspace = true` and `serde.workspace = true`. Check `cargo tree -p app -i egui-phosphor` shows `egui-phosphor v0.14.0` depending on `egui v0.36.2`.

- [ ] **Step 3: Write the failing theme and font tests**

At the end of the new `crates/app/src/ui/theme.rs` (create the file with only this test module and `use` lines first, so it fails to compile for want of the items):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// WCAG contrast ratio of two opaque colors.
    fn contrast(a: Color32, b: Color32) -> f32 {
        let channel = |u: u8| {
            let s = f32::from(u) / 255.0;
            if s <= 0.04045 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) }
        };
        let lum = |c: Color32| 0.2126 * channel(c.r()) + 0.7152 * channel(c.g()) + 0.0722 * channel(c.b());
        let (x, y) = (lum(a), lum(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }

    #[test]
    fn the_accents_are_the_spec_values() {
        assert_eq!(DARK.accent, Color32::from_rgb(0x4C, 0x8D, 0xFF));
        assert_eq!(LIGHT.accent, Color32::from_rgb(0x24, 0x63, 0xEB));
    }

    #[test]
    fn text_and_accent_read_against_the_panels() {
        for t in [&DARK, &LIGHT] {
            assert!(contrast(t.text_primary, t.bg_panel) >= 7.0);
            assert!(contrast(t.text_secondary, t.bg_panel) >= 4.5);
            assert!(contrast(t.accent, t.bg_panel) >= 3.0);
            assert!(contrast(t.on_accent, t.accent) >= 3.0);
        }
    }

    #[test]
    fn the_style_follows_the_grid_radii_and_type_scale() {
        for (t, dark) in [(&DARK, true), (&LIGHT, false)] {
            let s = style(t, dark);
            assert_eq!(s.visuals.dark_mode, dark);
            assert_eq!(s.spacing.interact_size.y, CONTROL_HEIGHT);
            for v in [s.spacing.item_spacing, s.spacing.button_padding] {
                assert_eq!((v.x % GRID, v.y % GRID), (0.0, 0.0), "{v:?} on the 4 pt grid");
            }
            assert_eq!(s.visuals.widgets.inactive.corner_radius, CornerRadius::same(6));
            assert_eq!(s.visuals.window_corner_radius, CornerRadius::same(8));
            assert_eq!(s.visuals.widgets.active.bg_stroke, Stroke::new(2.0, t.accent));
            assert_eq!(s.visuals.selection.stroke, Stroke::new(2.0, t.accent));
            assert_eq!(s.visuals.selection.bg_fill, t.accent_fill);
            assert_eq!(s.visuals.panel_fill, t.bg_panel);
            let size = |ts: TextStyle| s.text_styles[&ts].size;
            assert_eq!((size(TextStyle::Body), size(TextStyle::Small)), (13.0, 11.0));
            assert_eq!(s.text_styles[&TextStyle::Monospace].family, FontFamily::Monospace);
        }
    }

    #[test]
    fn install_sets_both_styles_and_the_choice() {
        let ctx = egui::Context::default();
        let frame = |system: Option<egui::Theme>| egui::RawInput {
            system_theme: system,
            ..Default::default()
        };
        install(&ctx, ThemeChoice::Dark);
        ctx.run_ui(frame(None), |_| {}).drop_without_applying_deltas();
        assert_eq!(ctx.theme(), egui::Theme::Dark);
        assert_eq!(ctx.style().visuals.panel_fill, DARK.bg_panel);
        set_choice(&ctx, ThemeChoice::Light);
        ctx.run_ui(frame(None), |_| {}).drop_without_applying_deltas();
        assert_eq!(ctx.style().visuals.panel_fill, LIGHT.bg_panel);
        // System follows what the window reports.
        set_choice(&ctx, ThemeChoice::System);
        ctx.run_ui(frame(Some(egui::Theme::Dark)), |_| {}).drop_without_applying_deltas();
        assert_eq!(ctx.theme(), egui::Theme::Dark);
        ctx.run_ui(frame(Some(egui::Theme::Light)), |_| {}).drop_without_applying_deltas();
        assert_eq!(ctx.theme(), egui::Theme::Light);
    }

    #[test]
    fn text_icons_and_values_have_their_fonts() {
        let fonts = super::super::fonts::definitions();
        assert_eq!(fonts.families[&FontFamily::Proportional][..2], ["inter", "phosphor"]);
        assert_eq!(fonts.families[&FontFamily::Monospace][0], "jetbrains-mono");
        let ctx = egui::Context::default();
        install(&ctx, ThemeChoice::Dark);
        ctx.run_ui(egui::RawInput::default(), |_| {}).drop_without_applying_deltas();
        let body = FontId::proportional(13.0);
        ctx.fonts_mut(|f| {
            assert!(f.has_glyph(&body, egui_phosphor::regular::CURSOR.chars().next().unwrap()));
            assert!(f.has_glyph(&body, '⌘'), "the macOS key glyphs come with Inter");
        });
    }
}
```
Run: `cargo test -p app ui::theme`. Expected: FAIL to compile (`DARK`, `style`, `install` missing).

- [ ] **Step 4: Write `fonts.rs`**

```rust
//! The fonts built into the binary: Inter for text, JetBrains Mono for values and Phosphor for
//! icons (see `assets/fonts/README.md`).

use std::sync::Arc;

use egui::{FontData, FontDefinitions, FontFamily};

pub const INTER: &str = "inter";
pub const INTER_SEMIBOLD: &str = "inter-semibold";
pub const MONO: &str = "jetbrains-mono";

static INTER_REGULAR_TTF: &[u8] = include_bytes!("../../assets/fonts/Inter-Regular.ttf");
static INTER_SEMIBOLD_TTF: &[u8] = include_bytes!("../../assets/fonts/Inter-SemiBold.ttf");
static MONO_TTF: &[u8] = include_bytes!("../../assets/fonts/JetBrainsMono-Regular.ttf");

/// egui's fonts with Inter first for text (then the Phosphor icons, then egui's own fonts as
/// fallbacks), JetBrains Mono first for monospace, and an Inter SemiBold family for titles.
pub fn definitions() -> FontDefinitions {
    let mut fonts = FontDefinitions::default();
    for (name, bytes) in [
        (INTER, INTER_REGULAR_TTF),
        (INTER_SEMIBOLD, INTER_SEMIBOLD_TTF),
        (MONO, MONO_TTF),
    ] {
        fonts
            .font_data
            .insert(name.into(), Arc::new(FontData::from_static(bytes)));
    }
    fonts
        .families
        .entry(FontFamily::Proportional)
        .or_default()
        .insert(0, INTER.into());
    fonts
        .families
        .entry(FontFamily::Monospace)
        .or_default()
        .insert(0, MONO.into());
    // Second in the proportional list, after Inter.
    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
    let mut titles = vec![INTER_SEMIBOLD.to_string()];
    titles.extend(fonts.families[&FontFamily::Proportional].iter().cloned());
    fonts.families.insert(semibold(), titles);
    fonts
}

/// Inter SemiBold, for section and window titles.
pub fn semibold() -> FontFamily {
    FontFamily::Name(INTER_SEMIBOLD.into())
}
```
- [ ] **Step 5: Write `theme.rs`**

Above the tests:

```rust
//! Design tokens: the dark and light color sets, the spacing grid, corner radii and type sizes,
//! and how they map onto egui's style.

use egui::{Color32, CornerRadius, FontFamily, FontId, Margin, Stroke, TextStyle, Vec2};
use serde::{Deserialize, Serialize};

use super::fonts;

/// Which theme the window uses; System follows the operating system's appearance.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeChoice {
    #[default]
    System,
    Dark,
    Light,
}

/// One theme's colors. The whole set the spec names is defined here; a few tokens (`success`)
/// get their first user in a later milestone.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tokens {
    /// Behind the panels.
    pub bg_app: Color32,
    pub bg_panel: Color32,
    /// Inputs, menus and windows.
    pub bg_raised: Color32,
    pub bg_hover: Color32,
    pub bg_pressed: Color32,
    pub border: Color32,
    pub text_primary: Color32,
    pub text_secondary: Color32,
    pub text_disabled: Color32,
    pub accent: Color32,
    /// The accent under selected rows.
    pub accent_fill: Color32,
    /// Text and icons on an accent fill.
    pub on_accent: Color32,
    pub warn: Color32,
    pub danger: Color32,
    pub success: Color32,
    /// The 3D view's day canvas, top and bottom.
    pub canvas_top: Color32,
    pub canvas_bottom: Color32,
}

const fn hex(rgb: u32) -> Color32 {
    Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

/// `rgb` at `alpha` out of 255.
const fn hex_alpha(rgb: u32, alpha: u8) -> Color32 {
    Color32::from_rgba_unmultiplied_const((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8, alpha)
}

pub const DARK: Tokens = Tokens {
    bg_app: hex(0x141518),
    bg_panel: hex(0x1C1D21),
    bg_raised: hex(0x25272C),
    bg_hover: hex(0x2E3036),
    bg_pressed: hex(0x373A41),
    border: hex(0x33363C),
    text_primary: hex(0xE8E9EC),
    text_secondary: hex(0xA3A7B0),
    text_disabled: hex(0x6B6F78),
    accent: hex(0x4C8DFF),
    // 22%
    accent_fill: hex_alpha(0x4C8DFF, 56),
    on_accent: Color32::WHITE,
    warn: hex(0xF5B544),
    danger: hex(0xF2555A),
    success: hex(0x3DBE74),
    canvas_top: hex(0x4A4D55),
    canvas_bottom: hex(0x383B42),
};

pub const LIGHT: Tokens = Tokens {
    bg_app: hex(0xECEEF1),
    bg_panel: hex(0xFFFFFF),
    bg_raised: hex(0xF6F7F9),
    bg_hover: hex(0xE9ECF0),
    bg_pressed: hex(0xDFE3E8),
    border: hex(0xD8DCE2),
    text_primary: hex(0x17191E),
    text_secondary: hex(0x5B616C),
    text_disabled: hex(0x9EA4AD),
    accent: hex(0x2463EB),
    // 14%
    accent_fill: hex_alpha(0x2463EB, 36),
    on_accent: Color32::WHITE,
    warn: hex(0xB7791F),
    danger: hex(0xD93036),
    success: hex(0x1E9E57),
    canvas_top: hex(0xF4F6F8),
    canvas_bottom: hex(0xE3E7EC),
};

/// The spacing grid every gap and padding is a multiple of.
pub const GRID: f32 = 4.0;
/// A control's height.
pub const CONTROL_HEIGHT: f32 = 28.0;
pub const CONTROL_RADIUS: u8 = 6;
/// Windows and menus.
pub const WINDOW_RADIUS: u8 = 8;
/// The width of the accent ring around a focused or chosen control.
pub const FOCUS_RING: f32 = 2.0;
pub const BODY_SIZE: f32 = 13.0;
pub const SMALL_SIZE: f32 = 11.0;

/// egui's style for one theme's tokens.
pub fn style(t: &Tokens, dark: bool) -> egui::Style {
    let mut style = egui::Style {
        visuals: if dark { egui::Visuals::dark() } else { egui::Visuals::light() },
        ..Default::default()
    };
    let v = &mut style.visuals;
    v.panel_fill = t.bg_panel;
    v.window_fill = t.bg_raised;
    v.extreme_bg_color = t.bg_app;
    v.faint_bg_color = t.bg_raised;
    v.code_bg_color = t.bg_raised;
    v.text_edit_bg_color = Some(t.bg_raised);
    v.window_stroke = Stroke::new(1.0, t.border);
    v.window_corner_radius = CornerRadius::same(WINDOW_RADIUS);
    v.menu_corner_radius = CornerRadius::same(WINDOW_RADIUS);
    v.hyperlink_color = t.accent;
    v.warn_fg_color = t.warn;
    v.error_fg_color = t.danger;
    v.weak_text_color = Some(t.text_secondary);
    v.selection.bg_fill = t.accent_fill;
    // Also a focused text field's frame.
    v.selection.stroke = Stroke::new(FOCUS_RING, t.accent);
    let w = &mut v.widgets;
    for (state, fill) in [
        (&mut w.noninteractive, t.bg_panel),
        (&mut w.inactive, t.bg_raised),
        (&mut w.hovered, t.bg_hover),
        (&mut w.active, t.bg_pressed),
        (&mut w.open, t.bg_hover),
    ] {
        state.bg_fill = fill;
        state.weak_bg_fill = fill;
        state.bg_stroke = Stroke::new(1.0, t.border);
        state.fg_stroke = Stroke::new(1.0, t.text_primary);
        state.corner_radius = CornerRadius::same(CONTROL_RADIUS);
        state.expansion = 0.0;
    }
    // egui draws a focused widget as active, so this is the focus ring.
    w.active.bg_stroke = Stroke::new(FOCUS_RING, t.accent);

    let s = &mut style.spacing;
    s.item_spacing = Vec2::new(2.0 * GRID, GRID);
    s.button_padding = Vec2::new(2.0 * GRID, GRID);
    s.interact_size = Vec2::new(10.0 * GRID, CONTROL_HEIGHT);
    s.window_margin = Margin::same(12);
    s.menu_margin = Margin::same(8);
    s.indent = 4.0 * GRID;

    style.text_styles = [
        (TextStyle::Small, FontId::new(SMALL_SIZE, FontFamily::Proportional)),
        (TextStyle::Body, FontId::new(BODY_SIZE, FontFamily::Proportional)),
        (TextStyle::Button, FontId::new(BODY_SIZE, FontFamily::Proportional)),
        (TextStyle::Heading, FontId::new(BODY_SIZE, fonts::semibold())),
        (TextStyle::Monospace, FontId::new(BODY_SIZE, FontFamily::Monospace)),
    ]
    .into();
    style
}

/// Installs the fonts and both themes' styles on `ctx`, and picks `choice`.
pub fn install(ctx: &egui::Context, choice: ThemeChoice) {
    ctx.set_fonts(fonts::definitions());
    ctx.set_style_of(egui::Theme::Dark, style(&DARK, true));
    ctx.set_style_of(egui::Theme::Light, style(&LIGHT, false));
    set_choice(ctx, choice);
}

/// Switches to `choice`; the styles stay installed.
pub fn set_choice(ctx: &egui::Context, choice: ThemeChoice) {
    ctx.set_theme(match choice {
        ThemeChoice::System => egui::ThemePreference::System,
        ThemeChoice::Dark => egui::ThemePreference::Dark,
        ThemeChoice::Light => egui::ThemePreference::Light,
    });
}
```
In `ui/mod.rs`: `pub mod theme;` and `mod fonts;` (`pub(crate) mod fonts;` if the compiler asks). Run `cargo test -p app ui::theme`. Expected: PASS. `ThemeChoice::Dark` and `Light` are constructed by the serde derive, so clippy does not call them unused before Task 9 uses them; if it does, add `ThemeChoice::ALL` and `label()` from Task 9 Step 3 here instead. If `text_and_accent_read_against_the_panels` fails for a token, report the ratio rather than change a spec value.

- [ ] **Step 6: Install the theme in the window and in the harness**

`gui.rs`:

```rust
use crate::ui::theme::{self, ThemeChoice};
...
    pub fn new(
        window: &Window,
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        theme: ThemeChoice,
    ) -> Self {
        let ctx = egui::Context::default();
        theme::install(&ctx, theme);
        let state = egui_winit::State::new(
            ctx.clone(),
            ViewportId::ROOT,
            window,
            Some(window.scale_factor() as f32),
            // So System starts with the window's appearance; egui-winit follows `ThemeChanged`.
            window.theme(),
            Some(device.limits().max_texture_dimension_2d as usize),
        );
```
`app.rs`: both `Gui::new(&window, &device, config.format)` and `Gui::new(&self.window, &device, config.format)` get a fourth argument `ThemeChoice::System` (Task 9 passes the saved choice).

`ui/tests/mod.rs`, at the end of `harness_sized` before `h.run_ok()`:

```rust
    theme::install(&h.ctx, ThemeChoice::Dark);
    // As the harness set it up: no animations, so one run settles everything.
    h.ctx.all_styles_mut(|s| {
        s.animation_time = 0.0;
        s.scroll_animation = egui::style::ScrollAnimation::none();
    });
```
with `use super::theme::{self, ThemeChoice};`.

- [ ] **Step 7: Keep the toolbar to one row**

In `toolbar.rs`, replace the `item_spacing.x = 7.0` line and its comment with:

```rust
    // Tighter than the theme so the default window's toolbar stays on one row (M8b), until
    // M11b moves the tools to a rail.
    ui.spacing_mut().item_spacing.x = 6.0;
    ui.spacing_mut().button_padding.x = 4.0;
```
If `clicking_a_tool_button_reports_it` still sees two rows at 1,280 pt after Step 9, lower these two to 4.0 and 2.0; never change the theme's metrics for it.

- [ ] **Step 8: Size the right panel from measurement**

Replace the magic heights. In `ui/mod.rs`: `RIGHT_PANEL_WIDTH` becomes `200.0` (doc: "Room for two value fields side by side at 13 pt.") and `PANEL_MIN_HEIGHT` goes; the panel body becomes:

```rust
        .show(ui, |ui| {
            let least = right::least_height(ui);
            if ui.available_height() >= least {
                right::right_panel(ui, view, &mut actions);
            } else {
                // A short window: lay the panel out at its least height and let it scroll.
                egui::ScrollArea::vertical()
                    .id_salt("right-panel")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let size = Vec2::new(ui.available_width(), least);
                        ui.allocate_ui(size, |ui| right::right_panel(ui, view, &mut actions));
                    });
            }
        });
```
In `right.rs`:

```rust
/// The Materials list shows whole rows, at most this many and at least `MATERIAL_ROWS_MIN`.
pub(super) const MATERIAL_ROWS_MAX: usize = 8;
pub(super) const MATERIAL_ROWS_MIN: usize = 2;
/// The Outliner keeps room for at least this many rows.
pub(super) const OUTLINER_ROWS_MIN: usize = 5;

/// A list row's height and the gap under it: the rows are controls.
pub(super) fn row_pitch(style: &egui::Style) -> f32 {
    style.spacing.interact_size.y + style.spacing.item_spacing.y
}

/// What the last frame measured of the right panel's fixed parts, in points.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct Measured {
    /// From the panel's top to the Materials list: Entity Info and the Materials title.
    pub above_list: f32,
    /// From under the Materials list to the Outliner's list: the material's controls and the
    /// Outliner's title.
    pub below_list: f32,
}

fn measured_id() -> egui::Id {
    egui::Id::new("right-panel-measured")
}

pub(super) fn measured(ctx: &egui::Context) -> Measured {
    ctx.data(|d| d.get_temp(measured_id())).unwrap_or_default()
}

/// The least height the panel lays out in: its fixed parts and both lists' least rows.
pub(super) fn least_height(ui: &Ui) -> f32 {
    let m = measured(ui.ctx());
    let rows = (MATERIAL_ROWS_MIN + OUTLINER_ROWS_MIN) as f32;
    m.above_list + m.below_list + rows * row_pitch(ui.style())
}

/// Keeps this frame's measurement. When it moved, egui lays the frame out again at once with
/// it (a discarded pass, not a repaint), so nothing is drawn from a stale measure.
fn remember(ui: &Ui, now: Measured) {
    let last = measured(ui.ctx());
    let moved = (now.above_list - last.above_list).abs() > 0.5
        || (now.below_list - last.below_list).abs() > 0.5;
    if moved {
        ui.data_mut(|d| d.insert_temp(measured_id(), now));
        ui.ctx().request_discard("the right panel measured its sections");
    }
}
```
`right_panel` records the panel's top, and takes the list edges from `materials` and the Outliner list's top from `outliner`:

```rust
pub(super) fn right_panel(ui: &mut Ui, view: &View, actions: &mut Vec<UiAction>) {
    let top = ui.cursor().top();
    // ... Entity Info, unchanged ...
    ui.separator();
    let (list_top, list_bottom) = materials(ui, view, actions);
    ui.separator();
    let outliner_list = outliner(ui, view, actions);
    remember(
        ui,
        Measured {
            above_list: list_top - top,
            below_list: outliner_list - list_bottom,
        },
    );
}
```
`materials.rs`: `materials` returns `(f32, f32)`, the list's top and the cursor's top after it; `MATERIALS_MAX_HEIGHT` and `BELOW_MATERIALS_LIST` go. The list:

```rust
    // Whole rows only: as many as leave the controls below and the Outliner's least rows room.
    let pitch = row_pitch(ui.style());
    let below = measured(ui.ctx()).below_list + OUTLINER_ROWS_MIN as f32 * pitch;
    let fit = ((ui.available_height() - below) / pitch).floor().max(0.0) as usize;
    let rows = fit.clamp(MATERIAL_ROWS_MIN, MATERIAL_ROWS_MAX);
    let list_top = ui.cursor().top();
    egui::ScrollArea::vertical()
        .id_salt("materials")
        .max_height(rows as f32 * pitch - ui.spacing().item_spacing.y)
        .show(ui, |ui| { /* the rows, unchanged */ });
    let list_bottom = ui.cursor().top();
```
The rest of `materials` (buttons, name, colors, sliders, texture) moves into `fn material_controls(ui: &mut Ui, view: &View, actions: &mut Vec<UiAction>)` so its early `return` (no material chosen) does not skip the return value; `materials` ends with `material_controls(ui, view, actions); (list_top, list_bottom)`.

`outliner.rs`: `outliner` returns `f32`, the cursor's top just before its `ScrollArea`; `OUTLINER_MIN_HEIGHT` goes and the area uses `.min_scrolled_height(OUTLINER_ROWS_MIN as f32 * row_pitch(ui.style()))`.

- [ ] **Step 9: Update the layout twin and add the panel tests**

In `tests/layout.rs`, replace the `OUTLINER_MIN_HEIGHT` import with `use crate::ui::right::{OUTLINER_ROWS_MIN, row_pitch};` and the assertion in `a_short_window_leaves_the_outliner_its_minimum_height` with:

```rust
    let least = OUTLINER_ROWS_MIN as f32 * row_pitch(&h.ctx.style());
    assert!(
        model.min.y <= height - least,
        "the Outliner starts at {} in a {height} point window",
        model.min.y
    );
```
Add:

```rust
#[test]
fn no_material_row_is_cut_by_the_list_bottom() {
    for height in [800.0, 600.0, 520.0] {
        let h = harness_sized(crowded(40), Vec2::new(1280.0, height));
        let bottom = rect_of(&h, "Add").min.y - h.ctx.style().spacing.item_spacing.y;
        for k in 0..40 {
            let row = rect_of(&h, &format!("Material {k}"));
            assert!(
                !(row.min.y < bottom - 0.5 && row.max.y > bottom + 0.5),
                "Material {k} straddles the list's bottom {bottom} at {height} points: {row:?}"
            );
        }
    }
}

#[test]
fn long_names_keep_the_panel_width() {
    let long = "A material whose name is far longer than the panel is wide";
    let mut f = crowded(3);
    f.materials[0].name = long.into();
    f.outliner.roots[0].name = long.into();
    let width = harness(f).state().central.width();
    assert_eq!(width, harness(crowded(3)).state().central.width());
}

#[test]
fn the_panels_settle_in_one_frame() {
    // Nothing changed: one frame and no request for another (the window can go idle).
    for f in [Fixture::default(), crowded(40)] {
        let mut h = harness(f);
        assert_eq!(h.try_run().unwrap(), 1);
    }
}
```
- [ ] **Step 10: Run everything**

```bash
cargo test -p app && cargo test --workspace 2>&1 | grep "test result" | awk '{s+=$4} END {print s}'
cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --all --check
```
Expected: all pass; 1159 plus the 5 theme tests and 3 layout tests. A failing twin means a layout regression (a clipped row, a wrapped toolbar); fix the layout, not the twin's intent.

- [ ] **Step 11: Commit**

```bash
git status && git diff --stat
git add Cargo.toml Cargo.lock crates/app/Cargo.toml crates/app/assets/fonts crates/app/src/ui crates/app/src/gui.rs crates/app/src/app.rs
git commit -m "Theme tokens, Inter, JetBrains Mono and Phosphor; right panel sized by measurement"
```

---

### Task 4: The shortcut table, and the toolbar and menus on the first widgets

One table of key bindings that the window's key handling uses, and the first widgets of the kit (tool button, menu item, key hint), with the toolbar and menus moved onto them. Each widget lands with its first user (an unused item fails clippy in a binary crate); the rest of the kit comes with the right panel (Task 5) and Undo/Redo (Task 7). Nothing moves on screen.

**Files:**
- Create: `crates/app/src/shortcuts.rs`, `crates/app/src/ui/widgets.rs`, `crates/app/src/ui/tests/widgets.rs`
- Modify: `crates/app/src/main.rs` (`mod shortcuts;`), `crates/app/src/ui/mod.rs` (`pub(crate) mod widgets;`), `crates/app/src/ui/theme.rs` (`tokens`, `SWATCH_RADIUS`), `crates/app/src/ui/tests/mod.rs` (`mod widgets;`, `drawn_texts_of`)
- Modify: `crates/app/src/editor.rs` (plain keys through the table), `crates/app/src/app.rs` (Cmd chords through the table)
- Modify: `crates/app/src/ui/toolbar.rs` (the kit), `crates/app/src/ui/tests/menus.rs`

**Interfaces:**
- Consumes: `ui::theme` (Task 3), `UiAction`, Task 2's harness.
- Produces:
  - `crate::shortcuts::{Bound, Key, Chord, Shortcut, TABLE, chord(bound: Bound) -> Option<Chord>, lookup(chord: Chord) -> Option<&'static Shortcut>, command(letter: char, shift: bool, repeat: bool) -> Option<Bound>, plain(c: char) -> Option<Bound>}`, `Chord::text(self) -> String`, `Chord::text_for(self, mac: bool) -> String`, `Bound::action(self) -> Option<UiAction>`.
  - `ui::theme::{tokens(theme: egui::Theme) -> &'static Tokens, SWATCH_RADIUS}`.
  - `ui::widgets::{tool_button(ui: &mut Ui, kind: ToolKind, selected: bool) -> Response, menu_item(ui: &mut Ui, label: &str, bound: Option<Bound>, enabled: bool) -> Response, hint(ui: &mut Ui, label: &str, bound: Option<Bound>), chip(ui: &mut Ui, text: &str) -> Response}`.
  - Test-only: `ui::tests::drawn_texts_of<S>(h: &Harness<'_, S>) -> Vec<String>`.

- [ ] **Step 1: Write the failing shortcut tests**

`crates/app/src/shortcuts.rs` starts with its test module (and `use` lines):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_binding_and_every_chord_is_in_the_table_once() {
        for (i, a) in TABLE.iter().enumerate() {
            for b in &TABLE[i + 1..] {
                assert_ne!(a.bound, b.bound, "{:?} twice", a.bound);
                assert_ne!(a.chord, b.chord, "{:?} and {:?} share a key", a.bound, b.bound);
            }
        }
    }

    #[test]
    fn tool_keys_are_the_tools_own() {
        for kind in ToolKind::ALL {
            let key = match kind.shortcut() {
                ' ' => Key::Space,
                c => Key::Letter(c),
            };
            assert_eq!(
                chord(Bound::Tool(kind)),
                Some(Chord { command: false, shift: false, key }),
                "{kind:?}"
            );
        }
    }

    #[test]
    fn plain_keys_find_their_bindings() {
        assert_eq!(plain(' '), Some(Bound::Tool(ToolKind::Select)));
        assert_eq!(plain('l'), Some(Bound::Tool(ToolKind::Line)));
        // An uppercase letter with no binding of its own acts as its lowercase one.
        assert_eq!(plain('L'), Some(Bound::Tool(ToolKind::Line)));
        assert_eq!(plain('F'), Some(Bound::ToggleFly));
        assert_eq!(plain('g'), Some(Bound::MakeGroup));
        assert_eq!(plain('G'), Some(Bound::MakeComponent));
        assert_eq!(plain('Z'), Some(Bound::ZoomExtents));
        assert_eq!(plain('z'), None);
        assert_eq!(plain('5'), None);
        // Cmd chords are not plain keys.
        assert_eq!(plain('s'), None);
    }

    #[test]
    fn command_chords_match_their_modifiers_and_repeat_only_undo_and_redo() {
        assert_eq!(command('z', false, false), Some(Bound::Undo));
        assert_eq!(command('z', false, true), Some(Bound::Undo), "held");
        assert_eq!(command('z', true, true), Some(Bound::Redo), "held");
        assert_eq!(command('s', false, false), Some(Bound::SaveFile));
        assert_eq!(command('s', false, true), None, "held Cmd+S saves once");
        assert_eq!(command('s', true, false), Some(Bound::SaveFileAs));
        assert_eq!(command('i', false, false), Some(Bound::ImportFile));
        assert_eq!(command('i', true, false), None);
        assert_eq!(command('e', true, false), Some(Bound::ExportFile));
        assert_eq!(command('e', false, false), None);
        assert_eq!(command('q', false, false), Some(Bound::Quit));
        assert_eq!(command('l', false, false), None);
    }

    #[test]
    fn chords_read_as_mac_glyphs_or_words() {
        let text = |b, mac| chord(b).unwrap().text_for(mac);
        assert_eq!(text(Bound::SaveFileAs, true), "⇧⌘S");
        assert_eq!(text(Bound::SaveFileAs, false), "Ctrl+Shift+S");
        assert_eq!(text(Bound::Undo, true), "⌘Z");
        assert_eq!(text(Bound::MakeComponent, true), "⇧G");
        assert_eq!(text(Bound::MakeComponent, false), "Shift+G");
        assert_eq!(text(Bound::Tool(ToolKind::Rectangle), false), "R");
        assert_eq!(text(Bound::Tool(ToolKind::Select), true), "Space");
        assert_eq!(text(Bound::EraseSelection, true), "⌫");
        assert_eq!(text(Bound::EraseSelection, false), "Delete");
        assert_eq!(text(Bound::CloseInstance, true), "Esc");
    }

    #[test]
    fn bindings_ask_for_the_panels_actions() {
        assert_eq!(Bound::Tool(ToolKind::Move).action(), Some(UiAction::Tool(ToolKind::Move)));
        assert_eq!(Bound::SaveFileAs.action(), Some(UiAction::SaveFileAs));
        assert_eq!(Bound::Quit.action(), None);
    }
}
```
Add `mod shortcuts;` to `main.rs`. Run `cargo test -p app shortcuts`. Expected: FAIL to compile.

- [ ] **Step 2: Write the table**

Above the tests:

```rust
//! Every keyboard shortcut in one table. Menus and tooltips take their key text from it and
//! the window's key handling looks keys up in it, so a label cannot name a key that does
//! something else.

use tools::ToolKind;

use crate::ui::UiAction;

/// What a shortcut does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bound {
    Tool(ToolKind),
    NewModel,
    OpenFile,
    SaveFile,
    SaveFileAs,
    ImportFile,
    ExportFile,
    Quit,
    Undo,
    Redo,
    EraseSelection,
    MakeGroup,
    MakeComponent,
    CloseInstance,
    ZoomExtents,
    ToggleFly,
}

/// The key of a shortcut.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    /// A lowercase ASCII letter.
    Letter(char),
    Space,
    Delete,
    Escape,
}

/// A key and the modifiers held with it. `command` is Cmd on macOS and Ctrl elsewhere (the
/// window takes either everywhere).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Chord {
    pub command: bool,
    pub shift: bool,
    pub key: Key,
}

#[derive(Clone, Copy, Debug)]
pub struct Shortcut {
    pub bound: Bound,
    pub chord: Chord,
    /// Held down, the key acts again.
    pub repeats: bool,
}

const fn on(bound: Bound, command: bool, shift: bool, key: Key) -> Shortcut {
    Shortcut {
        bound,
        chord: Chord { command, shift, key },
        repeats: false,
    }
}

const fn tool(kind: ToolKind, c: char) -> Shortcut {
    on(Bound::Tool(kind), false, false, Key::Letter(c))
}

const fn repeating(s: Shortcut) -> Shortcut {
    Shortcut { repeats: true, ..s }
}

pub const TABLE: [Shortcut; 26] = [
    on(Bound::Tool(ToolKind::Select), false, false, Key::Space),
    tool(ToolKind::Line, 'l'),
    tool(ToolKind::Rectangle, 'r'),
    tool(ToolKind::Circle, 'c'),
    tool(ToolKind::Arc, 'a'),
    tool(ToolKind::PushPull, 'p'),
    tool(ToolKind::Move, 'm'),
    tool(ToolKind::Rotate, 'q'),
    tool(ToolKind::TapeMeasure, 't'),
    tool(ToolKind::Erase, 'e'),
    tool(ToolKind::Paint, 'b'),
    on(Bound::NewModel, true, false, Key::Letter('n')),
    on(Bound::OpenFile, true, false, Key::Letter('o')),
    on(Bound::SaveFile, true, false, Key::Letter('s')),
    on(Bound::SaveFileAs, true, true, Key::Letter('s')),
    on(Bound::ImportFile, true, false, Key::Letter('i')),
    on(Bound::ExportFile, true, true, Key::Letter('e')),
    on(Bound::Quit, true, false, Key::Letter('q')),
    repeating(on(Bound::Undo, true, false, Key::Letter('z'))),
    repeating(on(Bound::Redo, true, true, Key::Letter('z'))),
    on(Bound::EraseSelection, false, false, Key::Delete),
    on(Bound::MakeGroup, false, false, Key::Letter('g')),
    on(Bound::MakeComponent, false, true, Key::Letter('g')),
    on(Bound::CloseInstance, false, false, Key::Escape),
    on(Bound::ZoomExtents, false, true, Key::Letter('z')),
    on(Bound::ToggleFly, false, false, Key::Letter('f')),
];

/// The chord bound to `bound`, if any.
pub fn chord(bound: Bound) -> Option<Chord> {
    TABLE.iter().find(|s| s.bound == bound).map(|s| s.chord)
}

pub fn lookup(chord: Chord) -> Option<&'static Shortcut> {
    TABLE.iter().find(|s| s.chord == chord)
}

/// What Cmd (or Ctrl) with `letter` (lowercase) does; `repeat` when the key is held down.
pub fn command(letter: char, shift: bool, repeat: bool) -> Option<Bound> {
    let chord = Chord {
        command: true,
        shift,
        key: Key::Letter(letter),
    };
    lookup(chord)
        .filter(|s| s.repeats || !repeat)
        .map(|s| s.bound)
}

/// What a key typed without Cmd does, as the editor gets it: `' '` for Space, a letter
/// uppercase exactly when Shift is held. An uppercase letter with no binding of its own acts
/// as its lowercase one, so Shift+L picks Line while Shift+G makes a component.
pub fn plain(c: char) -> Option<Bound> {
    let find = |shift, key| {
        lookup(Chord {
            command: false,
            shift,
            key,
        })
        .map(|s| s.bound)
    };
    if c == ' ' {
        return find(false, Key::Space);
    }
    if !c.is_ascii_alphabetic() {
        return None;
    }
    let key = Key::Letter(c.to_ascii_lowercase());
    let shift = c.is_ascii_uppercase();
    find(shift, key).or_else(|| if shift { find(false, key) } else { None })
}

impl Chord {
    /// How menus and tooltips show the chord on this system.
    pub fn text(self) -> String {
        self.text_for(cfg!(target_os = "macos"))
    }

    /// `⇧⌘S` on macOS, `Ctrl+Shift+S` elsewhere.
    pub fn text_for(self, mac: bool) -> String {
        let key = match self.key {
            Key::Letter(c) => c.to_ascii_uppercase().to_string(),
            Key::Space => "Space".into(),
            Key::Delete if mac => "⌫".into(),
            Key::Delete => "Delete".into(),
            Key::Escape => "Esc".into(),
        };
        let (shift, command) = if mac { ("⇧", "⌘") } else { ("Shift+", "Ctrl+") };
        let shift = if self.shift { shift } else { "" };
        let command = if self.command { command } else { "" };
        if mac {
            format!("{shift}{command}{key}")
        } else {
            format!("{command}{shift}{key}")
        }
    }
}

impl Bound {
    /// The panels' action for the binding; None for Quit, which the window handles itself.
    pub fn action(self) -> Option<UiAction> {
        Some(match self {
            Bound::Tool(kind) => UiAction::Tool(kind),
            Bound::NewModel => UiAction::NewModel,
            Bound::OpenFile => UiAction::OpenFile,
            Bound::SaveFile => UiAction::SaveFile,
            Bound::SaveFileAs => UiAction::SaveFileAs,
            Bound::ImportFile => UiAction::ImportFile,
            Bound::ExportFile => UiAction::ExportFile,
            Bound::Quit => return None,
            Bound::Undo => UiAction::Undo,
            Bound::Redo => UiAction::Redo,
            Bound::EraseSelection => UiAction::EraseSelection,
            Bound::MakeGroup => UiAction::MakeGroup,
            Bound::MakeComponent => UiAction::MakeComponent,
            Bound::CloseInstance => UiAction::CloseInstance,
            Bound::ZoomExtents => UiAction::ZoomExtents,
            Bound::ToggleFly => UiAction::ToggleFly,
        })
    }
}
```
Run `cargo test -p app shortcuts`. Expected: PASS.

- [ ] **Step 3: Dispatch keys through the table (failing tests first)**

In `editor.rs`'s tests:

```rust
    #[test]
    fn shifted_letters_keep_their_meanings() {
        let mut e = editor();
        assert!(e.key(EditorKey::Char('L')));
        assert_eq!(e.tool_kind(), ToolKind::Line);
        assert!(e.key(EditorKey::Char('E')));
        assert_eq!(e.tool_kind(), ToolKind::Erase);
        assert!(e.key(EditorKey::Char('F')));
        assert!(e.flying(), "Shift+F flies as f does");
    }

    #[test]
    fn the_editor_does_what_the_table_says_of_delete_and_escape() {
        use crate::shortcuts::{Bound, Chord, Key, chord};
        let plain = |key| Some(Chord { command: false, shift: false, key });
        assert_eq!(chord(Bound::EraseSelection), plain(Key::Delete));
        assert_eq!(chord(Bound::CloseInstance), plain(Key::Escape));
        // Delete erases the selection: a selected edge goes.
        let mut e = editor();
        e.execute(Command::AddEdge {
            start: [0.0, 0.0, 0.0],
            end: [1.0, 0.0, 0.0],
        })
        .unwrap();
        let ids: Vec<EntityId> = e.doc().model().edges().map(|(id, _)| EntityId::Edge(id)).collect();
        e.execute(Command::Select {
            ids,
            mode: Default::default(),
        })
        .unwrap();
        assert!(e.key(EditorKey::Delete));
        assert_eq!(e.doc().model().edges().count(), 0);
    }
```
The Escape half (closing an open group) is already covered by the existing Escape tests; this test pins the table to the key.

In `app.rs`'s tests, a test that every Cmd letter in the table survives non-Latin layouts:

```rust
    #[test]
    fn every_command_letter_has_a_physical_key_fallback() {
        use crate::shortcuts::{Key as ShortcutKey, TABLE};
        let code = |c: char| match c {
            'n' => KeyCode::KeyN,
            'o' => KeyCode::KeyO,
            's' => KeyCode::KeyS,
            'i' => KeyCode::KeyI,
            'e' => KeyCode::KeyE,
            'q' => KeyCode::KeyQ,
            'z' => KeyCode::KeyZ,
            other => panic!("add {other} to shortcut_letter's fallbacks and here"),
        };
        let arabic = Key::Character(SmolStr::new("ب"));
        for s in TABLE.iter().filter(|s| s.chord.command) {
            let ShortcutKey::Letter(c) = s.chord.key else { continue };
            assert_eq!(shortcut_letter(&arabic, PhysicalKey::Code(code(c))), Some(c), "{:?}", s.bound);
        }
    }
```
Run `cargo test -p app shifted_letters every_command_letter the_editor_does_what`. Expected: `shifted_letters_keep_their_meanings` and the fallback test PASS already (behaviour kept so far), the Delete test passes too; they guard the change below.

`editor.rs`, in `key`, replace the chain from `} else if c == 'g' {` through the `ToolKind::from_shortcut` branch with:

```rust
                } else if let Some(bound) = shortcuts::plain(c) {
                    self.run_shortcut(bound)
                } else {
                    false
                }
```
and add:

```rust
    /// Does what a key typed without Cmd is bound to. True if anything changed.
    fn run_shortcut(&mut self, bound: Bound) -> bool {
        match bound {
            Bound::Tool(kind) => {
                self.select_tool(kind);
                true
            }
            Bound::MakeGroup => self.make_group(),
            Bound::MakeComponent => self.make_component(),
            Bound::ZoomExtents => self.zoom_extents(),
            Bound::ToggleFly => self.set_flying(true),
            // Cmd chords, and Delete and Escape, which `key` handles before letters.
            _ => false,
        }
    }
```
with `use crate::shortcuts::{self, Bound};`. Update `key`'s doc comment to say the letters come from `shortcuts::TABLE`.

`app.rs`, in the `KeyboardInput` pressed branch, replace the chain from `if command_key && letter == Some('z') {` down to (and including) the `else if command_key || s.modifiers.alt_key() {` arm with:

```rust
                if command_key {
                    let shift = s.modifiers.shift_key();
                    match letter.and_then(|l| shortcuts::command(l, shift, event.repeat)) {
                        Some(Bound::Quit) => {
                            if s.settle_changes() {
                                event_loop.exit();
                            }
                            false
                        }
                        Some(bound) => {
                            if let Some(action) = bound.action() {
                                s.apply(action);
                            }
                            true
                        }
                        None => false,
                    }
                } else if s.modifiers.alt_key() {
                    // Alt/Option is the orbit modifier; Option+letter must not type or switch
                    // tools through its physical-key fallback.
                    false
                } else {
```
(the final `editor_key(...)` arm stays). `use crate::shortcuts::{self, Bound};`. `apply` already runs `new_model`, `open`, `save(false)`, `save(true)`, `import`, `export`, `undo` and `redo` exactly as the old arms did.

Run `cargo test -p app`. Expected: PASS, including every existing editor key test.


- [ ] **Step 4: Write the failing widget and toolbar tests**

`crates/app/src/ui/tests/widgets.rs` (add `mod widgets;` to `tests/mod.rs`; move the body of `drawn_texts` there into `pub(super) fn drawn_texts_of<S>(h: &Harness<'_, S>) -> Vec<String>` and make `drawn_texts(h: &H)` call it):

```rust
use super::*;
use crate::shortcuts::{Bound, chord};
use crate::ui::widgets;

/// A themed harness around `f`, settled; frames 50 ms apart.
pub(super) fn kit<'a>(f: impl FnMut(&mut Ui) + 'a) -> Harness<'a> {
    let mut h = Harness::builder().with_step_dt(0.05).build_ui(f);
    theme::install(&h.ctx, ThemeChoice::Dark);
    h.run_ok();
    h
}

#[test]
fn a_tool_button_is_named_for_its_tool_and_shows_its_key() {
    let mut clicked = false;
    let mut h = kit(|ui| {
        clicked |= widgets::tool_button(ui, ToolKind::Rectangle, true).clicked();
    });
    assert_eq!(h.get_by_label("Rectangle").accesskit_node().is_selected(), Some(true));
    h.get_by_label("Rectangle").hover();
    h.run_steps(20);
    let texts = drawn_texts_of(&h);
    let key = chord(Bound::Tool(ToolKind::Rectangle)).unwrap().text();
    assert!(texts.contains(&key), "{key} in {texts:?}");
    h.get_by_label("Rectangle").click();
    h.run_ok();
    drop(h);
    assert!(clicked);
}

#[test]
fn a_menu_item_is_named_without_its_key_and_shows_it() {
    let h = kit(|ui| {
        widgets::menu_item(ui, "Save As...", Some(Bound::SaveFileAs), true);
        widgets::menu_item(ui, "Delete Guides", None, false);
    });
    h.get_by_label("Save As...");
    assert!(h.get_by_label("Delete Guides").accesskit_node().is_disabled());
    let key = chord(Bound::SaveFileAs).unwrap().text();
    assert!(drawn_texts_of(&h).contains(&key));
}
```
Append to `tests/menus.rs` (with `use crate::shortcuts::{Bound, chord};`):

```rust
#[test]
fn menu_items_show_the_tables_keys() {
    let mut h = harness(Fixture::default());
    click(&mut h, "File");
    let texts = drawn_texts(&h);
    for bound in [
        Bound::NewModel,
        Bound::OpenFile,
        Bound::SaveFile,
        Bound::SaveFileAs,
        Bound::ImportFile,
        Bound::ExportFile,
    ] {
        let key = chord(bound).unwrap().text();
        assert!(texts.contains(&key), "{key} in {texts:?}");
    }
    let mut h = harness(Fixture::default());
    click(&mut h, "Camera");
    let fly = format!("Fly mode ({})", chord(Bound::ToggleFly).unwrap().text());
    h.get_by_label(&fly);
}

#[test]
fn tool_buttons_mark_the_active_tool_and_name_their_keys() {
    let mut h = harness(Fixture::default());
    assert_eq!(h.get_by_label("Line").accesskit_node().is_selected(), Some(true));
    assert_eq!(h.get_by_label("Move").accesskit_node().is_selected(), Some(false));
    h.get_by_label("Rectangle").hover();
    h.run_steps(20);
    let key = chord(Bound::Tool(ToolKind::Rectangle)).unwrap().text();
    assert!(drawn_texts(&h).contains(&key), "the tooltip's key chip");
}

#[test]
fn group_buttons_name_their_keys() {
    let mut h = harness(Fixture { has_selection: true, ..Default::default() });
    h.get_by_label("Make component").hover();
    h.run_steps(20);
    let key = chord(Bound::MakeComponent).unwrap().text();
    assert!(drawn_texts(&h).contains(&key), "{key}");
}
```
Run `cargo test -p app ui::tests`. Expected: FAIL to compile (`widgets` missing).

- [ ] **Step 5: Write the first widgets**

`theme.rs` gains (above `GRID`):

```rust
/// The tokens of the theme egui resolved (for System, the one the window reported).
pub fn tokens(theme: egui::Theme) -> &'static Tokens {
    match theme {
        egui::Theme::Dark => &DARK,
        egui::Theme::Light => &LIGHT,
    }
}
```
and `/// Swatches and key chips.` `pub const SWATCH_RADIUS: u8 = 4;` after `WINDOW_RADIUS`.

`crates/app/src/ui/widgets.rs`:

```rust
//! The widget kit: the controls the panels are built from, drawn from the theme's tokens and
//! named for AccessKit.

use egui::{Color32, CornerRadius, Margin, Response, RichText, Ui, Vec2, WidgetInfo, WidgetType};
use tools::ToolKind;

use super::theme;
use crate::shortcuts::{self, Bound};

/// A tool's button: its name, on an accent fill while it is the active tool. The tooltip names
/// the tool and its key.
pub fn tool_button(ui: &mut Ui, kind: ToolKind, selected: bool) -> Response {
    let t = theme::tokens(ui.ctx().theme());
    let mut text = RichText::new(kind.label());
    if selected {
        text = text.color(t.on_accent);
    }
    let response = ui.add(
        egui::Button::new(text)
            .fill(if selected { t.accent } else { Color32::TRANSPARENT })
            .frame_when_inactive(selected)
            .min_size(Vec2::new(0.0, theme::CONTROL_HEIGHT)),
    );
    response.widget_info(|| WidgetInfo::selected(WidgetType::Button, true, selected, kind.label()));
    response.on_hover_ui(|ui| hint(ui, kind.label(), Some(Bound::Tool(kind))))
}

/// A menu item showing the key bound to `bound` on its right; named `label` alone.
pub fn menu_item(ui: &mut Ui, label: &str, bound: Option<Bound>, enabled: bool) -> Response {
    let mut button = egui::Button::new(label);
    if let Some(chord) = bound.and_then(shortcuts::chord) {
        button = button.shortcut_text(chord.text());
    }
    let response = ui.add_enabled(enabled, button);
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, enabled, label));
    response
}

/// A tooltip's body: `label`, then the key bound to `bound` in a chip.
pub fn hint(ui: &mut Ui, label: &str, bound: Option<Bound>) {
    ui.horizontal(|ui| {
        ui.label(label);
        if let Some(chord) = bound.and_then(shortcuts::chord) {
            chip(ui, &chord.text());
        }
    });
}

/// Small muted text on a raised, rounded background: a key in a tooltip.
pub fn chip(ui: &mut Ui, text: &str) -> Response {
    let t = theme::tokens(ui.ctx().theme());
    egui::Frame::new()
        .fill(t.bg_hover)
        .corner_radius(CornerRadius::same(theme::SWATCH_RADIUS))
        .inner_margin(Margin::symmetric(4, 0))
        .show(ui, |ui| {
            ui.label(
                RichText::new(text)
                    .size(theme::SMALL_SIZE)
                    .color(t.text_secondary),
            )
        })
        .response
}
```
In `ui/mod.rs`: `pub(crate) mod widgets;`. If a builder method is named differently in egui 0.36.2 (`frame_when_inactive`, `Frame::new`, `Margin::symmetric` taking `i8`), use the 0.36.2 equivalent from `cargo doc -p egui --no-deps` and keep the behaviour. Tooltips are drawn only while hovered, after egui's own delay, so nothing repaints at idle.

- [ ] **Step 6: Move the toolbar and menus onto the kit**

In `toolbar.rs`, remove Task 2's `item` helper and use the kit (layout and order unchanged):

```rust
        ui.menu_button("File", |ui| {
            for (label, bound, action) in [
                ("New", Bound::NewModel, UiAction::NewModel),
                ("Open...", Bound::OpenFile, UiAction::OpenFile),
                ("Save", Bound::SaveFile, UiAction::SaveFile),
                ("Save As...", Bound::SaveFileAs, UiAction::SaveFileAs),
                ("Import...", Bound::ImportFile, UiAction::ImportFile),
                ("Export...", Bound::ExportFile, UiAction::ExportFile),
            ] {
                if widgets::menu_item(ui, label, Some(bound), true).clicked() {
                    actions.push(action);
                }
            }
            // Import Units: unchanged.
        });
        ui.menu_button("Edit", |ui| {
            if widgets::menu_item(ui, "Erase Selection", Some(Bound::EraseSelection), view.has_selection)
                .clicked()
            {
                actions.push(UiAction::EraseSelection);
            }
            if widgets::menu_item(ui, "Delete Guides", None, view.has_guides).clicked() {
                actions.push(UiAction::DeleteGuides);
            }
            ui.separator();
            if widgets::menu_item(ui, "Soften Edges...", None, true).clicked() {
                actions.push(UiAction::ShowSoften(true));
            }
        });
```
Camera: "Zoom Extents" through `widgets::menu_item(ui, "Zoom Extents", Some(Bound::ZoomExtents), true)`, each standard view through `widgets::menu_item(ui, v.label(), None, true)`, the projections and Night lighting as they are, and the fly checkbox's text from the table:

```rust
                let fly = format!("Fly mode ({})", key_text(Bound::ToggleFly));
                if ui.checkbox(&mut flying, fly).on_hover_text(FLY_HELP).clicked() {
```
View: `widgets::menu_item(ui, "Agent Activity", None, true)`. Tools:

```rust
        for kind in ToolKind::ALL {
            if widgets::tool_button(ui, kind, view.tool == kind).clicked() {
                actions.push(UiAction::Tool(kind));
            }
        }
```
Undo and Redo keep their buttons with `.on_hover_ui(|ui| widgets::hint(ui, "Undo", Some(Bound::Undo)))` (and Redo); Task 7 replaces them. The group buttons:

```rust
        let buttons = [
            (view.has_selection, "Make group", Some(Bound::MakeGroup), UiAction::MakeGroup),
            (view.has_selection, "Make component", Some(Bound::MakeComponent), UiAction::MakeComponent),
            (view.has_instance_selected, "Explode", None, UiAction::Explode),
            (view.inside_instance, "Close group", Some(Bound::CloseInstance), UiAction::CloseInstance),
        ];
        for (enabled, label, bound, action) in buttons {
            let button = ui
                .add_enabled(enabled, egui::Button::new(label))
                .on_hover_ui(|ui| widgets::hint(ui, label, bound))
                .on_disabled_hover_ui(|ui| widgets::hint(ui, label, bound));
            if button.clicked() {
                actions.push(action);
            }
        }
```
with

```rust
/// The key text of `bound` from the shortcut table.
fn key_text(bound: Bound) -> String {
    shortcuts::chord(bound).map(|c| c.text()).unwrap_or_default()
}
```
and `use crate::shortcuts::{self, Bound}; use super::widgets;`.

- [ ] **Step 7: Gate and commit**

```bash
cargo test -p app && cargo test --workspace 2>&1 | grep "test result" | awk '{s+=$4} END {print s}'
cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --all --check
git status && git diff --stat
git add crates/app/src/shortcuts.rs crates/app/src/main.rs crates/app/src/editor.rs crates/app/src/app.rs crates/app/src/ui
git commit -m "Shortcut table for keys, menus and tooltips; toolbar on the widget kit"
```
Expected: all pass, including `clicking_a_tool_button_reports_it` (one row at 1,280 pt).

---

### Task 5: The right panel on the kit

Section titles, value fields and swatches join the kit and the right panel moves onto them; the panel stays where it is.

**Files:**
- Modify: `crates/app/src/ui/widgets.rs`, `crates/app/src/ui/theme.rs` (three metrics)
- Modify: `crates/app/src/ui/right.rs`, `materials.rs`, `outliner.rs`
- Delete: `crates/app/src/ui/fields.rs`
- Test: `crates/app/src/ui/tests/widgets.rs`, `crates/app/src/ui/tests/materials.rs`

**Interfaces:**
- Consumes: `widgets` and `theme::tokens` (Task 4), `materials::palette_label` (Task 2), `tests::widgets::kit` (Task 4).
- Produces: `ui::widgets::{section_header(ui: &mut Ui, title: &str) -> Response, NumericRow<const N: usize> { done: Option<([f64; N], [f64; N])>, editing: bool }, numeric_row<const N: usize>(ui: &mut Ui, key: egui::Id, label: &str, shown: [f64; N], range: RangeInclusive<f64>, speed: f64) -> NumericRow<N>, take_row<const N: usize>(ui: &mut Ui, key: egui::Id) -> Option<([f64; N], [f64; N])>, Fill (Color(Color32) | Image(egui::TextureId)), swatch(ui: &mut Ui, fill: Fill, label: &str, selected: bool) -> Response}`; `theme::{SECTION_SIZE, SECTION_GAP, SWATCH_SIZE}`. Material rows' swatches are named `"{name} color"` (`"Default color"` for the default material).

- [ ] **Step 1: Write the failing tests**

Append to `tests/widgets.rs` (add `use crate::ui::widgets::Fill;`):

```rust
#[test]
fn a_section_header_is_named_as_written_and_drawn_uppercase() {
    let h = kit(|ui| {
        widgets::section_header(ui, "Entity Info");
    });
    h.get_by_label("Entity Info");
    assert!(drawn_texts_of(&h).iter().any(|t| t == "ENTITY INFO"));
}

#[test]
fn a_swatch_is_named_and_marked_when_chosen() {
    let h = kit(|ui| {
        widgets::swatch(ui, Fill::Color(egui::Color32::RED), "Brick color", true);
        widgets::swatch(ui, Fill::Color(egui::Color32::BLUE), "Glass color", false);
    });
    assert_eq!(h.get_by_label("Brick color").accesskit_node().is_selected(), Some(true));
    assert_eq!(h.get_by_label("Glass color").accesskit_node().is_selected(), Some(false));
}

#[test]
fn a_numeric_row_reports_a_typed_value_once_entered() {
    let key = egui::Id::new("test-row");
    let mut done = Vec::new();
    let mut h = kit(|ui| {
        if let Some(d) = widgets::numeric_row(ui, key, "Size (m)", [1.0, 0.5], 0.0..=10.0, 0.01).done {
            done.push(d);
        }
    });
    h.get_all_by_label("Size (m)").nth(1).unwrap().click();
    h.run_ok();
    h.event(egui::Event::Text("2".into()));
    h.run_ok();
    h.key_press(egui::Key::Enter);
    h.run_ok();
    drop(h);
    assert_eq!(done, [([1.0, 0.5], [1.0, 2.0])]);
}
```
Append to `tests/materials.rs`:

```rust
#[test]
fn sections_are_titled_and_the_chosen_material_and_color_are_marked() {
    let mut f = brick_with(None);
    f.materials[0].color = MATERIAL_PALETTE[2];
    let h = harness(f);
    for title in ["Entity Info", "Materials", "Outliner"] {
        h.get_by_label(title);
    }
    let texts = drawn_texts(&h);
    for title in ["ENTITY INFO", "MATERIALS", "OUTLINER"] {
        assert!(texts.iter().any(|t| t == title), "{title}");
    }
    let marked = |label: &str| h.get_by_label(label).accesskit_node().is_selected();
    assert_eq!(marked(&palette_label(MATERIAL_PALETTE[2])), Some(true), "the material's color");
    assert_eq!(marked(&palette_label(MATERIAL_PALETTE[0])), Some(false));
    assert_eq!(marked("Brick color"), Some(true), "the chosen material");
    assert_eq!(marked("Default color"), Some(false));
}

#[test]
fn a_material_swatch_chooses_it() {
    let mut h = harness(two_materials());
    assert_eq!(click(&mut h, "Glass color"), [UiAction::ChooseMaterial(Some(MaterialId(32)))]);
}
```
Run `cargo test -p app ui::tests`. Expected: FAIL to compile (`section_header`, `swatch`, `numeric_row` missing).

- [ ] **Step 2: Add the widgets**

`theme.rs`, after `SMALL_SIZE`:

```rust
/// Section titles, shown uppercase.
pub const SECTION_SIZE: f32 = 11.0;
/// The gap above a section title.
pub const SECTION_GAP: f32 = 12.0;
pub const SWATCH_SIZE: f32 = 16.0;
```
`widgets.rs` (extend the `egui` import with `Sense, Stroke, StrokeKind` and add `use std::ops::RangeInclusive;` and `use super::fonts;`):

```rust
/// A section's title: small, uppercase, muted, with a line under it. Named as written.
pub fn section_header(ui: &mut Ui, title: &str) -> Response {
    let t = theme::tokens(ui.ctx().theme());
    ui.add_space(theme::SECTION_GAP);
    let font = egui::FontId::new(theme::SECTION_SIZE, fonts::semibold());
    let galley = ui
        .painter()
        .layout_no_wrap(title.to_uppercase(), font, t.text_secondary);
    let height = galley.size().y + theme::GRID;
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::hover());
    ui.painter().galley(rect.left_top(), galley, t.text_secondary);
    ui.painter()
        .hline(rect.x_range(), rect.bottom(), Stroke::new(1.0, t.border));
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, title));
    response
}

/// What a `numeric_row` frame gave.
pub struct NumericRow<const N: usize> {
    /// When a drag ended or a typed value was entered: the values shown when the edit began,
    /// and the values now.
    pub done: Option<([f64; N], [f64; N])>,
    /// Whether a field is being dragged or typed in.
    pub editing: bool,
}
```
Then move `drag_row` and `take_row` from `fields.rs` here: `drag_row` becomes `pub fn numeric_row` (its doc comment and body verbatim, including the `.labelled_by(label_id)` of Task 2, returning `NumericRow`), with one line added inside the `horizontal` closure before the field loop:

```rust
            // Values in the value font.
            ui.style_mut().override_text_style = Some(egui::TextStyle::Monospace);
```
`take_row` becomes `pub fn take_row` verbatim. `git rm crates/app/src/ui/fields.rs` and drop `mod fields;`. Then:

```rust
/// What a swatch shows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Fill {
    Color(Color32),
    Image(egui::TextureId),
}

/// A small square of a color or an image that can be clicked, ringed in the accent when
/// `selected`. Named `label`.
pub fn swatch(ui: &mut Ui, fill: Fill, label: &str, selected: bool) -> Response {
    let t = theme::tokens(ui.ctx().theme());
    let radius = CornerRadius::same(theme::SWATCH_RADIUS);
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(theme::SWATCH_SIZE), Sense::click());
    match fill {
        Fill::Color(color) => ui.painter().rect_filled(rect, radius, color),
        Fill::Image(image) => egui::Image::new((image, rect.size()))
            .corner_radius(radius)
            .paint_at(ui, rect),
    }
    ui.painter()
        .rect_stroke(rect, radius, Stroke::new(1.0, t.border), StrokeKind::Inside);
    if selected {
        ui.painter().rect_stroke(
            rect,
            radius,
            Stroke::new(theme::FOCUS_RING, t.accent),
            StrokeKind::Outside,
        );
    }
    response.widget_info(|| WidgetInfo::selected(WidgetType::Button, true, selected, label));
    response
}
```
- [ ] **Step 3: Move the panel onto the kit**

- `right.rs`: `ui.heading("Entity Info")` becomes `widgets::section_header(ui, "Entity Info");`; `texture_position` and `end_texture_position_edit` call `widgets::numeric_row` and `widgets::take_row` (same arguments; `DragRow` is now `NumericRow`).
- `materials.rs`: `ui.heading("Materials")` becomes `widgets::section_header(ui, "Materials");`; `swatch` and `image_swatch` go; the list's rows yield `(id, name, fill)` with `Fill::Color(DEFAULT_SWATCH)` for Default, `Fill::Image(t.swatch)` for a textured material and `Fill::Color(rgb(m.color))` otherwise, and each row is

```rust
            for (id, name, fill) in rows {
                ui.horizontal(|ui| {
                    let chosen = view.material == id;
                    let picked = widgets::swatch(ui, fill, &format!("{name} color"), chosen).clicked();
                    let label = ui.selectable_label(chosen, name);
                    if picked || label.clicked() {
                        actions.push(UiAction::ChooseMaterial(id));
                    }
                });
            }
```
The palette (Task 2's `widget_info` call there goes; the kit names the swatch):

```rust
        for color in MATERIAL_PALETTE {
            let response =
                widgets::swatch(ui, Fill::Color(rgb(color)), &palette_label(color), color == m.color);
            if response.clicked() && color != m.color {
                actions.push(UiAction::RecolorMaterial(m.id, color));
            }
        }
```
`texture_controls` and `end_texture_size_edit` use `widgets::numeric_row` and `widgets::take_row`.
- `outliner.rs`: `ui.heading("Outliner")` becomes `widgets::section_header(ui, "Outliner");`.

- [ ] **Step 4: Run, gate, commit**

```bash
cargo test -p app && cargo test --workspace 2>&1 | grep "test result" | awk '{s+=$4} END {print s}'
cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --all --check
git status && git diff --stat
git add crates/app/src/ui
git commit -m "Right panel on the widget kit"
```
Expected: all pass, including `no_material_row_is_cut_by_the_list_bottom` and `the_panels_settle_in_one_frame` (the section titles change the measured heights; the measurement absorbs it).

---

### Task 6: The window's viewport follows the theme

A `ViewStyle` per renderer: `CLASSIC` (today's look, bit for bit) stays the default, so `render_view`, headless renders and the goldens do not change; the window's renderer gets one from the theme every frame.

**Files:**
- Create: `crates/render/src/style.rs`
- Modify: `crates/render/src/lib.rs`, `renderer.rs`, `scene.rs` (`shown_guides`), `shaders/scene.wgsl`
- Modify: `crates/app/src/ui/theme.rs` (`view_style`), `crates/app/src/gui.rs` (`Gui::theme`), `crates/app/src/app.rs` (set the style each frame)
- Test: `crates/render/tests/headless.rs`, `crates/app/src/ui/theme.rs`

**Interfaces:**
- Consumes: `theme::tokens` (Task 4).
- Produces: `render::ViewStyle { day_canvas: [[f32; 4]; 2], day_ground_fade: f32, selected_edge: [f32; 4], selected_face: [f32; 3], face_tint: f32, wide_selected_edges: bool }`, `ViewStyle::CLASSIC`, `ViewStyle::day_background(&self) -> [f32; 4]`, `ViewStyle::has_gradient(&self) -> bool`, `Renderer::set_style(&mut self, style: ViewStyle)`; `scene::shown_guides(guides: &[GuideShape], lit: &[bool], color: [f32; 4]) -> Vec<GuideShape>`; `ui::theme::view_style(theme: egui::Theme) -> render::ViewStyle`; `Gui::theme(&self) -> egui::Theme`.

- [ ] **Step 1: Write the failing render tests**

In `crates/render/tests/headless.rs` (add `use render::ViewStyle;`):

```rust
/// Renders `model` with `selection`, lit by `lighting` and drawn in `style`, from `camera`.
fn render_styled(
    camera: &Camera,
    model: &Model,
    selection: &BTreeSet<EntityId>,
    style: ViewStyle,
    lighting: Lighting,
) -> Option<Vec<u8>> {
    let Some(screen) = Offscreen::new() else {
        eprintln!("no GPU adapter, skipping");
        return None;
    };
    let mut renderer = screen.renderer();
    renderer.set_scene(&Scene::from_root(model.clone()), &[], selection);
    renderer.set_style(style);
    renderer.set_lighting(lighting);
    Some(screen.draw(&mut renderer, camera, Viewport::full((W, H)), &[]))
}

/// A style selecting in magenta, a color nothing else in these scenes has.
const MAGENTA: ViewStyle = ViewStyle {
    selected_edge: [1.0, 0.0, 1.0, 1.0],
    selected_face: [1.0, 0.0, 1.0],
    face_tint: 0.18,
    wide_selected_edges: true,
    ..ViewStyle::CLASSIC
};

fn magenta(p: [u8; 4]) -> bool {
    p[0] > 150 && p[1] < 110 && p[2] > 150
}

fn count(px: &[u8], f: impl Fn([u8; 4]) -> bool) -> usize {
    px.chunks(4).filter(|p| f([p[0], p[1], p[2], p[3]])).count()
}

#[test]
fn a_style_tints_selected_faces_lightly_in_its_color() {
    let cam = camera();
    let model = ground_square();
    let face = model.faces().next().unwrap().0;
    let selected = BTreeSet::from([EntityId::Face(face)]);
    let (Some(plain), Some(tinted)) = (
        render_styled(&cam, &model, &BTreeSet::new(), MAGENTA, Lighting::Day),
        render_styled(&cam, &model, &selected, MAGENTA, Lighting::Day),
    ) else {
        return;
    };
    let at = DVec3::new(0.5, 0.5, 0.0);
    let (p, t) = (pixel_at(&plain, &cam, at), pixel_at(&tinted, &cam, at));
    assert!(t[0] > p[0] && t[2] > p[2], "toward magenta: {p:?} -> {t:?}");
    assert!(
        (i32::from(p[1]) - i32::from(t[1])).abs() < 60,
        "18% leaves the face showing: {p:?} -> {t:?}"
    );
}

#[test]
fn wide_selected_edges_cover_more_pixels() {
    let cam = camera();
    let model = ground_square();
    let edges: BTreeSet<EntityId> = model.edges().map(|(id, _)| EntityId::Edge(id)).collect();
    let thin = ViewStyle { wide_selected_edges: false, ..MAGENTA };
    let (Some(a), Some(b)) = (
        render_styled(&cam, &model, &edges, thin, Lighting::Day),
        render_styled(&cam, &model, &edges, MAGENTA, Lighting::Day),
    ) else {
        return;
    };
    let (thin_px, wide_px) = (count(&a, magenta), count(&b, magenta));
    assert!(thin_px > 0, "the selected edges show");
    assert!(wide_px as f64 > 1.6 * thin_px as f64, "{thin_px} -> {wide_px} pixels");
}

#[test]
fn the_day_canvas_is_a_gradient_and_night_keeps_its_background() {
    // Looking horizontally: the upper half sees only sky.
    let mut cam = camera();
    cam.eye = DVec3::new(0.0, -10.0, 1.0);
    cam.target = DVec3::new(0.0, 0.0, 1.0);
    let style = ViewStyle {
        day_canvas: [[0.1, 0.1, 0.1, 1.0], [0.9, 0.9, 0.9, 1.0]],
        ..ViewStyle::CLASSIC
    };
    let empty = Model::new();
    let none = BTreeSet::new();
    let (Some(day), Some(night)) = (
        render_styled(&cam, &empty, &none, style, Lighting::Day),
        render_styled(&cam, &empty, &none, style, Lighting::Night),
    ) else {
        return;
    };
    let (top, quarter) = (pixel(&day, 0, 0), pixel(&day, 0, H / 4));
    assert!(i32::from(quarter[0]) - i32::from(top[0]) > 30, "lighter downward: {top:?} {quarter:?}");
    assert_eq!(pixel(&night, 0, 0), pixel(&night, 0, H / 4), "night is one color");
}

#[test]
fn restyling_recolors_a_selected_guide() {
    let c = camera();
    let mut m = Model::new();
    m.add_guide(kernel::Guide::Line {
        point: c.target,
        direction: c.right(),
    })
    .unwrap();
    let guide = m.guides().next().unwrap().0;
    let Some(screen) = Offscreen::new() else { return };
    let mut renderer = screen.renderer();
    renderer.set_scene(&Scene::from_root(m), &[], &BTreeSet::from([EntityId::Guide(guide)]));
    renderer.set_style(MAGENTA);
    let px = screen.draw(&mut renderer, &c, Viewport::full((W, H)), &[]);
    assert!(count(&px, magenta) > 0, "the guide in the style's color");
    let green = ViewStyle { selected_edge: [0.0, 1.0, 0.0, 1.0], ..MAGENTA };
    renderer.set_style(green);
    let px = screen.draw(&mut renderer, &c, Viewport::full((W, H)), &[]);
    assert_eq!(count(&px, magenta), 0, "no rebuild needed");
    assert!(count(&px, |p| p[1] > 150 && p[0] < 110 && p[2] < 110) > 0);
}
```
Run `cargo test -p render --test headless`. Expected: FAIL to compile (`ViewStyle`, `set_style` missing).

- [ ] **Step 2: Write `style.rs`**

```rust
//! How a renderer draws what the window's theme decides. `ViewStyle::CLASSIC` is the default
//! and what `render_view`, headless renders and the golden images use.

use crate::grid::BACKGROUND;
use crate::scene::SELECTED_EDGE_COLOR;

/// The canvas, the line fading and the selection's look. Colors are sRGB-authored like
/// `grid::BACKGROUND`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewStyle {
    /// The day background: the view's top and bottom, a vertical gradient between them.
    pub day_canvas: [[f32; 4]; 2],
    /// How far the grid and axes fade toward the canvas by day (0 to 1). Night has its own.
    pub day_ground_fade: f32,
    /// Selected edges and guides.
    pub selected_edge: [f32; 4],
    /// The color selected faces are tinted toward, and how far (0 to 1).
    pub selected_face: [f32; 3],
    pub face_tint: f32,
    /// Selected edges drawn about 2 px wide rather than 1.
    pub wide_selected_edges: bool,
}

impl ViewStyle {
    /// The look before themes: a flat light background, a blue selection at half strength.
    pub const CLASSIC: Self = Self {
        day_canvas: [BACKGROUND, BACKGROUND],
        day_ground_fade: 0.0,
        selected_edge: SELECTED_EDGE_COLOR,
        selected_face: [0.25, 0.45, 0.95],
        face_tint: 0.5,
        wide_selected_edges: false,
    };

    /// The day background's middle: the clear color, and what lines fade toward by day.
    pub fn day_background(&self) -> [f32; 4] {
        let [a, b] = self.day_canvas;
        [0, 1, 2, 3].map(|k| (a[k] + b[k]) / 2.0)
    }

    /// Whether the day canvas is drawn as a gradient rather than cleared to one color.
    pub fn has_gradient(&self) -> bool {
        self.day_canvas[0] != self.day_canvas[1]
    }
}

impl Default for ViewStyle {
    fn default() -> Self {
        Self::CLASSIC
    }
}
```
`lib.rs`: `pub mod style;` and `pub use style::ViewStyle;`. (`(a + a) / 2.0 == a` exactly in floating point, so CLASSIC's background is `BACKGROUND` bit for bit.)

- [ ] **Step 3: Carry the style in the lighting uniforms (shader)**

`shaders/scene.wgsl`: the `Lighting` struct becomes

```wgsl
struct Lighting {
    // x: how many lights; y: the ambient light; z: the headlight; w: 1 at night, else 0.
    mode: vec4<f32>,
    // The background, which lines fade toward.
    background: vec4<f32>,
    // Selected edges and guides.
    selected_edge: vec4<f32>,
    // rgb: the color selected faces are tinted toward; w: how far.
    selected_face: vec4<f32>,
    // x: how far unselected edges fade toward the background; y: how far the grid and axes do;
    // zw: the view's size in pixels.
    lines: vec4<f32>,
    // The day canvas's top and bottom.
    canvas_top: vec4<f32>,
    canvas_bottom: vec4<f32>,
    lights: array<Light, MAX_LIGHTS>,
};
```
Remove `NIGHT_LINE_FADE`, `night_line`, `SELECTED_EDGE` and `SELECTED`; add

```wgsl
fn faded_line(color: vec4<f32>, fade: f32) -> vec4<f32> {
    return vec4<f32>(mix(color.rgb, lighting.background.rgb, fade), color.a);
}
```
`vs_edge` returns `line_vertex(pos, select(faded_line(color, lighting.lines.x), lighting.selected_edge, selected > 0.5), 1.0005)`; `vs_ground` and `vs_flat_ground` use `faded_line(color, lighting.lines.y)`; `face_color`'s tint line becomes `let base = mix(color, lighting.selected_face.rgb, lighting.selected_face.w * in.selected);`. Update the comments that named `SELECTED_EDGE` or "fade at night" to say the style sets them. Add:

```wgsl
// Selected edges drawn again one pixel across (instance 0) and one pixel down (instance 1), so
// they read about 2 px wide; the other edges are moved outside the view and clipped away.
@vertex
fn vs_edge_wide(
    @location(0) pos: vec3<f32>,
    @location(1) color: vec4<f32>,
    @location(2) selected: f32,
    @builtin(instance_index) k: u32,
) -> LineOut {
    var out = line_vertex(pos, lighting.selected_edge, 1.0005);
    if selected < 0.5 {
        out.pos = vec4<f32>(2.0, 2.0, 0.0, 1.0);
        return out;
    }
    let pixel = select(vec2<f32>(0.0, 2.0), vec2<f32>(2.0, 0.0), k == 0u) / lighting.lines.zw;
    out.pos = vec4<f32>(out.pos.xy + pixel * out.pos.w, out.pos.zw);
    return out;
}

struct CanvasOut {
    @builtin(position) pos: vec4<f32>,
    // 0 at the view's top, 1 at its bottom.
    @location(0) down: f32,
};

// One triangle covering the view, at the far plane.
@vertex
fn vs_canvas(@builtin(vertex_index) i: u32) -> CanvasOut {
    let xy = vec2<f32>(f32(i & 1u) * 4.0 - 1.0, f32(i >> 1u) * 4.0 - 1.0);
    var out: CanvasOut;
    out.pos = vec4<f32>(xy, 0.0, 1.0);
    out.down = (1.0 - xy.y) * 0.5;
    return out;
}

@fragment
fn fs_canvas(in: CanvasOut) -> @location(0) vec4<f32> {
    return mix(lighting.canvas_top, lighting.canvas_bottom, in.down);
}
```
- [ ] **Step 4: The renderer side**

`renderer.rs`:
- `LightingUniforms` gains, after `background` and in the shader's order: `selected_edge: [f32; 4]`, `selected_face: [f32; 4]`, `lines: [f32; 4]`, `canvas_top: [f32; 4]`, `canvas_bottom: [f32; 4]`, each with a doc comment matching the shader's. `LightingUniforms::new(used, shadow_slots, mode, style: &ViewStyle, view: (u32, u32))` starts them at `[0.0; 4]` in its struct literal (with `background: background(mode, style)`) and fills them:

```rust
        let night = mode == Lighting::Night;
        let [r, g, b] = style.selected_face;
        uniforms.selected_edge = style.selected_edge;
        uniforms.selected_face = [r, g, b, style.face_tint];
        uniforms.lines = [
            if night { NIGHT_LINE_FADE } else { 0.0 },
            if night { NIGHT_LINE_FADE } else { style.day_ground_fade },
            view.0.max(1) as f32,
            view.1.max(1) as f32,
        ];
        uniforms.canvas_top = style.day_canvas[0];
        uniforms.canvas_bottom = style.day_canvas[1];
```
with `/// How far unselected edges, the grid and the axes fade toward the background at night, so they stay quiet outlines rather than bright lines on the dark.` `const NIGHT_LINE_FADE: f32 = 0.8;` (moved from the shader).
- `background(mode)` becomes `background(mode: Lighting, style: &ViewStyle) -> [f32; 4]`: Night `NIGHT_BACKGROUND`, Day `style.day_background()`; both callers pass `&self.style`.
- `Renderer` gains fields `style: ViewStyle` (`ViewStyle::CLASSIC` in `new`), `guide_flags: Vec<bool>`, `edges_selected: bool`, `canvas_pipeline: wgpu::RenderPipeline`, `wide_edge_pipeline: wgpu::RenderPipeline`. The pipelines, next to `edge_pipeline`:

```rust
        // Selected edges a second and third time, offset a pixel (see `vs_edge_wide`).
        let wide_edge_pipeline = pipeline(
            "wide edges",
            "vs_edge_wide",
            "fs_line",
            &flagged_lines,
            lines,
            DepthUse {
                write: true,
                compare: wgpu::CompareFunction::GreaterEqual,
            },
            None,
        );
        // The day canvas's gradient, under everything.
        let canvas_pipeline = pipeline(
            "canvas",
            "vs_canvas",
            "fs_canvas",
            &[],
            wgpu::PrimitiveTopology::TriangleList,
            DepthUse {
                write: false,
                compare: wgpu::CompareFunction::Always,
            },
            None,
        );
```
- `set_model` also sets `self.guide_flags = vec![false; self.base_guides.len()]` and `self.edges_selected = false`. `set_selection` keeps the flags and colors guides with the style:

```rust
        self.edges_selected = flags.edges.iter().any(|&f| f > 0.5);
        self.guides = shown_guides(&self.base_guides, &flags.guides, self.style.selected_edge);
        self.guide_flags = flags.guides;
```
(take `flags.guides` after its last borrow; clone if the borrow checker asks).
- New:

```rust
    /// Draws the frames from now on in `style`: the window sets its theme's; render_view and
    /// headless keep `ViewStyle::CLASSIC`. Recolors selected guides without a rebuild.
    pub fn set_style(&mut self, style: ViewStyle) {
        if style == self.style {
            return;
        }
        self.style = style;
        self.guides = shown_guides(&self.base_guides, &self.guide_flags, style.selected_edge);
    }
```
- `render_in`: build the lighting uniforms with `&self.style` and the viewport's size (`(viewport.width, viewport.height)`); inside the pass, first thing after `set_bind_group`:

```rust
                if self.lighting == Lighting::Day && self.style.has_gradient() {
                    pass.set_pipeline(&self.canvas_pipeline);
                    pass.draw(0..3, 0..1);
                }
```
and right after the edges are drawn:

```rust
                if self.style.wide_selected_edges
                    && self.edges_selected
                    && let Some(edges) = &self.edges
                {
                    bind(&mut pass, &self.wide_edge_pipeline, edges);
                    pass.draw(0..edges.count, 0..2);
                }
```
`scene.rs`: `shown_guides` takes `color: [f32; 4]` and uses it for lit guides (doc: "...in `color`"); its callers in `scene.rs` tests pass `SELECTED_EDGE_COLOR`. `scene_geometry` (CPU path) keeps `SELECTED_EDGE_COLOR`.

Run:

```bash
cargo test -p render
```
Expected: PASS, the four new tests included, and every golden test unchanged (no `UPDATE_GOLDEN`; a golden mismatch means CLASSIC is not bit for bit, fix that). Without a GPU adapter the GPU tests skip; say so in the report if that happens.

- [ ] **Step 5: The window sets its theme's style (failing test first)**

In `theme.rs` tests:

```rust
    #[test]
    fn the_view_follows_the_theme() {
        let unit = |c: Color32| [c.r(), c.g(), c.b()].map(|u| f32::from(u) / 255.0);
        for (theme, t) in [(egui::Theme::Dark, &DARK), (egui::Theme::Light, &LIGHT)] {
            let s = view_style(theme);
            let [r, g, b] = unit(t.accent);
            assert_eq!(s.selected_edge, [r, g, b, 1.0]);
            assert_eq!(s.selected_face, [r, g, b]);
            assert_eq!(s.face_tint, 0.18);
            assert!(s.wide_selected_edges);
            assert!(s.has_gradient());
            let [r, g, b] = unit(t.canvas_top);
            assert_eq!(s.day_canvas[0], [r, g, b, 1.0]);
        }
        assert_eq!(view_style(egui::Theme::Light).day_ground_fade, 0.0);
        assert!(view_style(egui::Theme::Dark).day_ground_fade > 0.0, "a dark canvas quiets the grid");
    }
```
Run `cargo test -p app the_view_follows_the_theme`: FAIL to compile. Then in `theme.rs`:

```rust
/// How far faces are tinted toward the accent when selected.
pub const SELECTION_TINT: f32 = 0.18;
/// How far the grid and axes fade toward the dark theme's day canvas (black edges keep theirs).
pub const DARK_GROUND_FADE: f32 = 0.6;

/// How the window's 3D view looks in `theme`: its day canvas and the selection in the accent.
pub fn view_style(theme: egui::Theme) -> render::ViewStyle {
    let t = tokens(theme);
    let rgba = |c: Color32| {
        let [r, g, b] = [c.r(), c.g(), c.b()].map(|u| f32::from(u) / 255.0);
        [r, g, b, 1.0]
    };
    let accent = rgba(t.accent);
    render::ViewStyle {
        day_canvas: [rgba(t.canvas_top), rgba(t.canvas_bottom)],
        day_ground_fade: match theme {
            egui::Theme::Dark => DARK_GROUND_FADE,
            egui::Theme::Light => 0.0,
        },
        selected_edge: accent,
        selected_face: [accent[0], accent[1], accent[2]],
        face_tint: SELECTION_TINT,
        wide_selected_edges: true,
    }
}
```
`gui.rs`:

```rust
    /// The theme the panels are drawn in now (System resolved to what the window reports).
    pub fn theme(&self) -> egui::Theme {
        self.ctx.theme()
    }
```
`app.rs`, in the frame, just before `self.renderer.set_lighting(doc.lighting);`:

```rust
        // The view follows the panels' theme; render_view keeps its own renderer's classic look.
        self.renderer
            .set_style(theme::view_style(self.gui.theme()));
```
(`use crate::ui::theme;`). `set_style` returns at once when nothing changed, so this costs nothing per frame. The offscreen `Snapshotter` is never given a style.

- [ ] **Step 6: Gate and commit**

```bash
cargo test --workspace 2>&1 | grep "test result" | awk '{s+=$4} END {print s}'
cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --all --check
git status && git diff --stat
git add crates/render crates/app/src/ui/theme.rs crates/app/src/gui.rs crates/app/src/app.rs
git commit -m "Viewport follows the theme: day canvas gradient, accent selection, 2 px selected edges"
```
`git status` must show no change under `crates/render/tests/golden/`.

---

### Task 7: Undo and Redo disabled when empty, naming their step

**Files:**
- Modify: `crates/session/src/document.rs` (`undo_label`, `redo_label`)
- Test: `crates/session/tests/commands.rs`
- Create: `crates/app/src/ui/icons.rs`
- Modify: `crates/app/src/ui/widgets.rs` (`icon_button`), `crates/app/src/ui/theme.rs` (`ICON_SIZE`), `crates/app/src/ui/mod.rs` (`View::undo`, `View::redo`, `mod icons;`), `crates/app/src/ui/toolbar.rs`, `crates/app/src/app.rs` (fills the labels)
- Test: `crates/app/src/ui/tests/mod.rs` (`Fixture::undo`, `Fixture::redo`), `tests/layout.rs`, `tests/menus.rs`, `tests/widgets.rs`

**Interfaces:**
- Consumes: `widgets::hint` (Task 4), `shortcuts::Bound`.
- Produces: `Document::undo_label(&self) -> Option<&str>`, `Document::redo_label(&self) -> Option<&str>`; `View::undo: Option<&'a str>`, `View::redo: Option<&'a str>` (raw history labels); `ui::icons::{UNDO, REDO}`; `widgets::icon_button(ui: &mut Ui, icon: &str, label: &str, tooltip: &str, bound: Option<Bound>, enabled: bool) -> Response`; `toolbar::step_name(label: &str) -> String`; `theme::ICON_SIZE`.

- [ ] **Step 1: Write the failing session test**

In `crates/session/tests/commands.rs`:

```rust
#[test]
fn the_next_undo_and_redo_steps_are_named() {
    let mut d = doc();
    assert_eq!((d.undo_label(), d.redo_label()), (None, None));
    edge(&mut d, [0.0, 0.0, 0.0], [1.0, 0.0, 0.0]);
    assert_eq!((d.undo_label(), d.redo_label()), (Some("add_edge"), None));
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert_eq!((d.undo_label(), d.redo_label()), (None, Some("add_edge")));
    // The same names the history query lists first.
    assert_eq!(ask(&d, json!({"type": "history"}))["redo"][0], "add_edge");
}
```
Run `cargo test -p session --test commands the_next_undo`. Expected: FAIL to compile.

- [ ] **Step 2: Implement**

`document.rs`, next to the `History` query's code:

```rust
    /// The label of the step `undo` would undo next (the `history` query's first `undo`).
    pub fn undo_label(&self) -> Option<&str> {
        self.undo.last().map(|s| s.label.as_str())
    }

    /// The label of the step `redo` would redo next.
    pub fn redo_label(&self) -> Option<&str> {
        self.redo.last().map(|s| s.label.as_str())
    }
```
Run the test. Expected: PASS.

- [ ] **Step 3: Write the failing UI tests**

`Fixture` gains `pub undo: Option<String>` and `pub redo: Option<String>` (default `None`), passed as `undo: self.undo.as_deref(), redo: self.redo.as_deref()` in `view()`. In `tests/layout.rs`, `toolbar_fixture()` sets `undo: Some("push_pull".into()), redo: Some("paint".into())` so the twins still click both.

In `tests/widgets.rs`:

```rust
#[test]
fn an_icon_button_is_named_and_can_be_disabled() {
    let mut h = kit(|ui| {
        widgets::icon_button(ui, crate::ui::icons::UNDO, "Undo", "Nothing to undo", Some(Bound::Undo), false);
    });
    assert!(h.get_by_label("Undo").accesskit_node().is_disabled());
    h.get_by_label("Undo").hover();
    h.run_steps(20);
    assert!(drawn_texts_of(&h).iter().any(|t| t == "Nothing to undo"));
}
```
In `tests/menus.rs`:

```rust
#[test]
fn undo_and_redo_are_disabled_when_empty_and_name_their_step() {
    let mut h = harness(Fixture::default());
    for label in ["Undo", "Redo"] {
        assert!(h.get_by_label(label).accesskit_node().is_disabled(), "{label}");
        assert_eq!(click(&mut h, label), [], "{label}");
    }
    let mut h = harness(Fixture { undo: Some("push_pull".into()), ..Default::default() });
    assert!(!h.get_by_label("Undo").accesskit_node().is_disabled());
    assert!(h.get_by_label("Redo").accesskit_node().is_disabled());
    h.get_by_label("Undo").hover();
    h.run_steps(20);
    let texts = drawn_texts(&h);
    assert!(texts.iter().any(|t| t == "Undo Push pull"), "{texts:?}");
    assert!(texts.contains(&chord(Bound::Undo).unwrap().text()), "{texts:?}");
    assert_eq!(click(&mut h, "Undo"), [UiAction::Undo]);
}

#[test]
fn history_labels_read_as_words() {
    use crate::ui::toolbar::step_name;
    assert_eq!(step_name("push_pull"), "Push pull");
    assert_eq!(step_name("batch"), "Batch");
    assert_eq!(step_name(""), "");
}
```
Run `cargo test -p app ui::tests`. Expected: FAIL to compile.

- [ ] **Step 4: Implement**

`theme.rs`, after `SWATCH_SIZE`: `pub const ICON_SIZE: f32 = 16.0;`. `ui/icons.rs`:

```rust
//! The Phosphor glyphs the panels use.

use egui_phosphor::regular as ph;

pub const UNDO: &str = ph::ARROW_U_UP_LEFT;
pub const REDO: &str = ph::ARROW_U_UP_RIGHT;
```
`widgets.rs`:

```rust
/// A square button showing `icon`, named `label`; its tooltip is `tooltip` and the key bound
/// to `bound`, also while disabled (then the icon takes the disabled text color).
pub fn icon_button(
    ui: &mut Ui,
    icon: &str,
    label: &str,
    tooltip: &str,
    bound: Option<Bound>,
    enabled: bool,
) -> Response {
    let t = theme::tokens(ui.ctx().theme());
    let mut glyph = RichText::new(icon).size(theme::ICON_SIZE);
    if !enabled {
        glyph = glyph.color(t.text_disabled);
    }
    let response = ui.add_enabled(
        enabled,
        egui::Button::new(glyph).min_size(Vec2::splat(theme::CONTROL_HEIGHT)),
    );
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, enabled, label));
    response
        .on_hover_ui(|ui| hint(ui, tooltip, bound))
        .on_disabled_hover_ui(|ui| hint(ui, tooltip, bound))
}
```
`ui/mod.rs`: `mod icons;`; `View` gains

```rust
    /// The label of the step Undo would undo (`Document::undo_label`); None: nothing to undo.
    pub undo: Option<&'a str>,
    /// The label of the step Redo would redo.
    pub redo: Option<&'a str>,
```
`toolbar.rs`, replacing the Undo and Redo buttons in place:

```rust
        let undo = view.undo.map_or("Nothing to undo".into(), |l| format!("Undo {}", step_name(l)));
        if widgets::icon_button(ui, icons::UNDO, "Undo", &undo, Some(Bound::Undo), view.undo.is_some())
            .clicked()
        {
            actions.push(UiAction::Undo);
        }
        let redo = view.redo.map_or("Nothing to redo".into(), |l| format!("Redo {}", step_name(l)));
        if widgets::icon_button(ui, icons::REDO, "Redo", &redo, Some(Bound::Redo), view.redo.is_some())
            .clicked()
        {
            actions.push(UiAction::Redo);
        }
```
and

```rust
/// A history label as words: `push_pull` reads "Push pull".
pub(super) fn step_name(label: &str) -> String {
    let words = label.replace('_', " ");
    let mut chars = words.chars();
    chars
        .next()
        .map(|c| c.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}
```
`app.rs`, in the `View` it builds: `undo: self.editor.doc().undo_label(), redo: self.editor.doc().redo_label(),`. Keyboard Cmd+Z is unchanged (with nothing to undo it still says so in the status bar).

- [ ] **Step 5: Gate and commit**

```bash
cargo test --workspace 2>&1 | grep "test result" | awk '{s+=$4} END {print s}'
cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --all --check
git status && git diff --stat
git add crates/session/src/document.rs crates/session/tests/commands.rs crates/app/src/ui crates/app/src/app.rs
git commit -m "Undo and Redo disabled when empty, tooltips naming the step"
```

---

### Task 8: Minimum window size

**Files:**
- Modify: `crates/app/src/app.rs` (`MIN_WINDOW`, `window_attributes`)
- Test: `crates/app/src/app.rs` tests, `crates/app/src/ui/tests/layout.rs`

**Interfaces:**
- Consumes: Task 2's `every_toolbar_button_works`, `crowded`.
- Produces: `app::MIN_WINDOW: LogicalSize<f64>` (960 x 600), `fn window_attributes() -> WindowAttributes`.

- [ ] **Step 1: Write the failing tests**

`app.rs` tests:

```rust
    #[test]
    fn the_window_cannot_shrink_below_960_by_600() {
        use winit::dpi::Size;
        let attrs = window_attributes();
        assert_eq!(attrs.min_inner_size, Some(Size::Logical(LogicalSize::new(960.0, 600.0))));
        assert_eq!(attrs.inner_size, Some(Size::Logical(LogicalSize::new(1280.0, 800.0))));
    }
```
`tests/layout.rs`:

```rust
#[test]
fn the_panels_fit_the_smallest_window() {
    let size = Vec2::new(960.0, 600.0);
    every_toolbar_button_works(size);
    let h = harness_sized(crowded(40), size);
    let view = h.state().central;
    assert!(view.width() >= 600.0 && view.height() >= 350.0, "a usable 3D view: {view:?}");
    h.get_by_label("Model");
}
```
Run `cargo test -p app the_window_cannot the_panels_fit`. Expected: the first FAILS to compile (`window_attributes` missing); the second may already pass (it pins the minimum's layout).

- [ ] **Step 2: Implement**

```rust
/// The window's least size, in points: the panels and a usable 3D view.
const MIN_WINDOW: LogicalSize<f64> = LogicalSize::new(960.0, 600.0);

/// The window as it opens: title, default size and least size.
fn window_attributes() -> WindowAttributes {
    Window::default_attributes()
        .with_title("sketchup-alt")
        .with_inner_size(LogicalSize::new(1280.0, 800.0))
        .with_min_inner_size(MIN_WINDOW)
}
```
and in `resumed`, `let attrs = window_attributes();` replaces the inline builder (`use winit::window::WindowAttributes;`).

- [ ] **Step 3: Gate and commit**

```bash
cargo test --workspace 2>&1 | grep "test result" | awk '{s+=$4} END {print s}'
cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --all --check
git status && git diff --stat
git add crates/app/src/app.rs crates/app/src/ui/tests/layout.rs
git commit -m "Minimum window size 960 x 600"
```

---

### Task 9: Preferences file and View > Theme

**Files:**
- Create: `crates/app/src/prefs.rs`
- Modify: `crates/app/Cargo.toml` (`serde_json` from dev-dependencies to dependencies), `crates/app/src/main.rs` (`mod prefs;`, the prefs path), `crates/app/src/app.rs` (load, apply, save), `crates/app/src/gui.rs` (`set_theme`), `crates/app/src/ui/theme.rs` (`ThemeChoice::ALL`, `label`), `crates/app/src/ui/mod.rs` (`UiAction::SetTheme`, `View::theme`), `crates/app/src/ui/toolbar.rs` (View > Theme)
- Test: `crates/app/src/prefs.rs`, `crates/app/src/ui/tests/menus.rs`, `tests/mod.rs` (`Fixture::theme`)

**Interfaces:**
- Consumes: `theme::{ThemeChoice, set_choice}` (Task 3), `cli::{FALLBACK_DIR, ensure_private_dir}`.
- Produces: `prefs::{Prefs { theme: ThemeChoice }, path_in(home: &Path) -> PathBuf, load(path: &Path) -> Prefs, save(path: &Path, prefs: &Prefs) -> io::Result<()>}`; `UiAction::SetTheme(ThemeChoice)`; `View::theme: ThemeChoice`; `Gui::set_theme(&self, choice: ThemeChoice)`; `App::new(..., recovery, prefs: Option<PathBuf>)`.

- [ ] **Step 1: Write the failing prefs tests**

`crates/app/src/prefs.rs` starts with:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    /// A fresh folder standing in for HOME.
    fn home(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("sketchup-alt-prefs-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn prefs_round_trip_in_a_private_folder() {
        let home = home("round-trip");
        let path = path_in(&home);
        assert_eq!(load(&path), Prefs::default(), "no file yet");
        save(&path, &Prefs { theme: ThemeChoice::Light }).unwrap();
        assert_eq!(load(&path).theme, ThemeChoice::Light);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{\n  \"theme\": \"light\"\n}");
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&path), 0o600);
        assert_eq!(mode(path.parent().unwrap()), 0o700);
        save(&path, &Prefs { theme: ThemeChoice::Dark }).unwrap();
        assert_eq!(load(&path).theme, ThemeChoice::Dark, "replaced");
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn bad_prefs_files_give_the_defaults() {
        let home = home("bad");
        let path = path_in(&home);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        for bytes in [&b""[..], b"not json", b"{\"theme\": \"purple\"}", b"[1, 2]", b"{\"theme\": 3}"] {
            std::fs::write(&path, bytes).unwrap();
            assert_eq!(load(&path), Prefs::default(), "{}", String::from_utf8_lossy(bytes));
        }
        // Keys it does not know are ignored.
        std::fs::write(&path, b"{\"theme\": \"dark\", \"later\": 3}").unwrap();
        assert_eq!(load(&path).theme, ThemeChoice::Dark);
        // Something that is not a file.
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert_eq!(load(&path), Prefs::default());
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn prefs_are_not_written_through_a_linked_folder() {
        let home = home("linked");
        let elsewhere = home.join("elsewhere");
        std::fs::create_dir(&elsewhere).unwrap();
        std::os::unix::fs::symlink(&elsewhere, home.join(crate::cli::FALLBACK_DIR)).unwrap();
        let path = path_in(&home);
        assert!(save(&path, &Prefs { theme: ThemeChoice::Dark }).is_err());
        assert!(!elsewhere.join("prefs.json").exists());
        std::fs::remove_dir_all(home).unwrap();
    }
}
```
Add `mod prefs;` to `main.rs`. Run `cargo test -p app prefs`. Expected: FAIL to compile.

- [ ] **Step 2: Implement the prefs file**

`crates/app/Cargo.toml`: move `serde_json.workspace = true` from `[dev-dependencies]` to `[dependencies]`. Above the tests:

```rust
//! Window preferences kept between sessions in `~/.sketchup-alt/prefs.json`. They are window
//! chrome: never part of a model, never read or written by the API or in headless mode.

use std::io::{self, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::ui::theme::ThemeChoice;

/// What the prefs file keeps. Keys it does not know are ignored, and dropped at the next save.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    pub theme: ThemeChoice,
}

/// The prefs file for the home folder `home`.
pub fn path_in(home: &Path) -> PathBuf {
    home.join(crate::cli::FALLBACK_DIR).join("prefs.json")
}

/// The prefs at `path`: the defaults when there is no file, and the defaults with a warning
/// on stderr when it cannot be read or is not valid (it is replaced at the next change).
pub fn load(path: &Path) -> Prefs {
    match std::fs::read(path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Prefs::default(),
        Err(e) => {
            eprintln!("warning: cannot read {}: {e}; using the default preferences", path.display());
            Prefs::default()
        }
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|e| {
            eprintln!(
                "warning: {} is not valid preferences (line {}, column {}); using the defaults",
                path.display(),
                e.line(),
                e.column()
            );
            Prefs::default()
        }),
    }
}

/// Writes `prefs` to `path` (mode 0600) through a temporary file renamed into place, in a
/// folder made or checked private like the rest of `~/.sketchup-alt` (0700, not a link).
pub fn save(path: &Path, prefs: &Prefs) -> io::Result<()> {
    let dir = path
        .parent()
        .ok_or_else(|| io::Error::other("the preferences path has no folder"))?;
    crate::cli::ensure_private_dir(dir)?;
    let json = serde_json::to_vec_pretty(prefs).map_err(io::Error::other)?;
    let temp = dir.join(format!(".prefs.json.{}.tmp", std::process::id()));
    // Left by a run that stopped halfway, if any.
    let _ = std::fs::remove_file(&temp);
    let written = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temp)
        .and_then(|mut file| {
            file.write_all(&json)?;
            file.sync_all()
        })
        .and_then(|()| std::fs::rename(&temp, path));
    if written.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    written
}
```
Run `cargo test -p app prefs`. Expected: PASS.

- [ ] **Step 3: View > Theme (failing test first)**

`Fixture` gains `pub theme: ThemeChoice` (default `ThemeChoice::System`), passed as `theme: self.theme`. In `tests/menus.rs`:

```rust
#[test]
fn the_view_menu_sets_the_theme() {
    for (item, choice) in [("System", ThemeChoice::System), ("Dark", ThemeChoice::Dark), ("Light", ThemeChoice::Light)] {
        let mut h = harness(Fixture::default());
        click(&mut h, "View");
        click(&mut h, "Theme");
        assert_eq!(click(&mut h, item), [UiAction::SetTheme(choice)], "{item}");
    }
    let mut h = harness(Fixture { theme: ThemeChoice::Dark, ..Default::default() });
    click(&mut h, "View");
    click(&mut h, "Theme");
    assert_eq!(h.get_by_label("Dark").accesskit_node().is_selected(), Some(true));
    assert_eq!(h.get_by_label("Light").accesskit_node().is_selected(), Some(false));
}
```
Run: FAIL to compile. Then:
- `theme.rs`:

```rust
impl ThemeChoice {
    pub const ALL: [ThemeChoice; 3] = [ThemeChoice::System, ThemeChoice::Dark, ThemeChoice::Light];

    pub fn label(self) -> &'static str {
        match self {
            ThemeChoice::System => "System",
            ThemeChoice::Dark => "Dark",
            ThemeChoice::Light => "Light",
        }
    }
}
```
- `ui/mod.rs`: `UiAction::SetTheme(ThemeChoice)` (doc: "Draw the window in this theme, and keep it in the preferences."), `View::theme: ThemeChoice` (doc: "The theme chosen in View > Theme.").
- `toolbar.rs`, in the View menu after Agent Activity:

```rust
            ui.menu_button("Theme", |ui| {
                for choice in ThemeChoice::ALL {
                    if ui.selectable_label(view.theme == choice, choice.label()).clicked() {
                        actions.push(UiAction::SetTheme(choice));
                    }
                }
            });
```
- `gui.rs`:

```rust
    /// Draws the panels in `choice` from the next frame on.
    pub fn set_theme(&self, choice: ThemeChoice) {
        theme::set_choice(&self.ctx, choice);
    }
```
- `main.rs`, next to `recovery`:

```rust
            let prefs = std::env::var_os("HOME").map(|home| prefs::path_in(Path::new(&home)));
```
and the HOME warning becomes `"warning: HOME is not set: work an agent discards in the window is not kept, and preferences last only for the session"`; pass `prefs` to `App::new` as a new last argument, which keeps it in `App` and hands it to `State::new`.
- `app.rs` `State` gains `prefs: Prefs` and `prefs_path: Option<PathBuf>`. `State::new` loads first and builds the panels in the saved theme:

```rust
        let prefs = prefs_path.as_deref().map(prefs::load).unwrap_or_default();
        // ...
        let gui = Gui::new(&window, &device, config.format, prefs.theme);
```
`recreate_gpu` passes `self.prefs.theme`. The `View` gets `theme: self.prefs.theme`. `apply`:

```rust
            UiAction::SetTheme(choice) => {
                self.prefs.theme = choice;
                self.gui.set_theme(choice);
                if let Some(path) = &self.prefs_path
                    && let Err(e) = prefs::save(path, &self.prefs)
                {
                    eprintln!("warning: cannot save the preferences to {}: {e}", path.display());
                }
            }
```
(the frame after an action is requested already; the viewport restyles in it through Task 6). Headless never builds a `State`, so it never touches the file.

- [ ] **Step 4: Gate and commit**

```bash
cargo test --workspace 2>&1 | grep "test result" | awk '{s+=$4} END {print s}'
cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --all --check
git status && git diff --stat
git add crates/app/Cargo.toml Cargo.lock crates/app/src/prefs.rs crates/app/src/main.rs crates/app/src/app.rs crates/app/src/gui.rs crates/app/src/ui
git commit -m "Preferences file and View > Theme"
```

---

### Task 10: Docs: the window-only chrome list

**Files:**
- Modify: `docs/agent-api.md` ("Window actions and the API")

- [ ] **Step 1: Edit the table and the chrome paragraph**

In the table, the Undo row becomes:

```
| Undo, Redo (Cmd+Z, Shift+Cmd+Z; disabled when there is nothing to undo or redo, the tooltip naming the step) | `undo`, `redo`; `history` lists the steps |
```
and after the "View > Agent Activity" row add:

```
| View > Theme (System, Dark, Light) | none: window chrome, kept in `~/.sketchup-alt/prefs.json` |
```
The chrome paragraph becomes:

```
Window chrome, which changes nothing in the model, its view or the editor, has no command: the
window's size and position (at least 960 x 600 points), the panels' sizes and scroll positions,
which menus are open, tooltips, which Outliner rows are expanded, the export dialog's suggested
format, keyboard focus, and the window's preferences: the theme (View > Theme), kept in
`~/.sketchup-alt/prefs.json`, which the API never reads or writes. The window's 3D view follows
the theme (its day background and the selection's color and width); `render_view` keeps one
fixed look, so its images do not depend on the window. The file dialogs are chrome too (commands
take paths); while one, or the unsaved-changes prompt, is open, the window answers no requests.
```
- [ ] **Step 2: Check and commit**

`grep -n "prefs.json\|960 x 600" docs/agent-api.md` shows both. Then:

```bash
git status && git diff --stat
git add docs/agent-api.md
git commit -m "agent-api: theme and minimum window size are window chrome"
```

---

### Task 11: Roadmap and whole-branch review

- [ ] **Step 1: Full gate**

```bash
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace 2>&1 | grep "test result" | awk '{s+=$4} END {print s}'
cargo build --release -p app
git status --short crates/render/tests/golden/
```
Expected: clean, at least 1159 plus this milestone's tests, a release build, and no golden image changed. Do not launch the window.

- [ ] **Step 2: Roadmap row**

In `docs/superpowers/roadmap.md`, the M11a row's last cell `planned` becomes `done <first>..<last>` with the short hashes of Task 1's commit and Task 10's commit (`git log --oneline` after `3dc03fe`).

```bash
git status && git diff --stat
git add docs/superpowers/roadmap.md
git commit -m "Roadmap: M11a done"
```
- [ ] **Step 3: Whole-branch review**

Use superpowers:requesting-code-review over `3dc03fe..HEAD` against the spec's M11a bullet and this plan's Spec notes and Review Focus. Ask the reviewer to check in particular: CLASSIC renders bit for bit (goldens untouched); no unconditional repaint (`the_panels_settle_in_one_frame`, `request_discard` only on a changed measure); the key paths keep their old behaviour (Shift letters, held Cmd+S, non-Latin layouts); prefs never written by headless or the API; no test lost (count). Fix findings in follow-up commits, rerun Step 1, and report the rulings in "Spec notes" with the milestone's summary.
