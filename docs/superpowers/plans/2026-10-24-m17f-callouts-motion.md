# M17f Callouts and Motion Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let an agent label parts and show motion: one-command callouts that point at a part, notes and dimensions in `render_view` images, scenes that store where parts are, animated scene transitions, frames of a transition rendered on request, and an exploded-view command.

**Architecture:** `add_label` computes an anchor point and a note position and adds an ordinary leader note. Headless and `render_view` images draw annotations with the same egui text painting as the window, through an offscreen egui pass composited by egui-wgpu onto the rendered image. `Page` gains `positions: Option<BTreeMap<InstanceId, DAffine3>>` for root instances; going to a scene applies them; scene transitions interpolate camera and positions over a duration in the window and in `render_view { scene, t }`.

**Tech Stack:** Rust 2024, egui 0.36.2, egui-wgpu (already used by app), wgpu 30. No new dependencies.

**Spec:** `docs/superpowers/roadmap.md` row M17f.

## Global Constraints

- No new dependencies. Existing behaviour unchanged except where ruled; golden images unchanged (they have no annotations, or are re-checked to be identical).
- New commands and fields: every registration site, `.skr` save/load (old files load), `.skp` scene import/export unaffected, `docs/agent-api.md`, in the task that adds them.
- Gate per task (project rules): native gate + the wasm32 `cargo check` line; `scripts/build-web.sh` when the window changes.
- No emoji. Short commit messages. Never `git add -A`.

## Spec notes (rulings)

1. **`add_label { target, text, position? }`**: `target` is a face, edge, vertex or instance of the open context. The anchor: a vertex's position; an edge's midpoint; a face's interior point nearest its centroid (inside the face, not in a hole); an instance's bounding box center projected onto its nearest face along the line toward the camera eye if one is hit, else the box center. Without `position`, the note goes outward from the open context's bounding box center through the anchor, at a distance of 15% of that box's diagonal, raised by 5% of it along +z; with `position`, there. Adds a leader note anchored to the point (a vertex target anchors to the vertex, so it follows it). One undo step; outcome `created` names the note. Text rules as `add_note`.
2. **Annotations in images.** `render_view`, the MCP render tool, the headless `render` and print/PDF output draw leader notes, screen notes and dimensions as the window draws them (text, leader lines, arrows, the same font sizes in points scaled to the image's pixel density: 1 point = 1 pixel at the default; images wider than 2000 px scale text by width / 1000 capped at 4). Done with an offscreen egui pass painted onto the rendered texture with egui-wgpu. Hidden annotations, those on hidden tags, and those behind active section cuts follow the window's rules. Print/PDF is in scope only if it renders through the same path; otherwise record it as a follow-up.
3. **Scene positions.** `add_scene`, `update_scene` and `change_scene` take `positions` : an array of instance ids of the root (store those instances' current transforms), `true` (update/change: store again the same set from current transforms), or `false` (stop storing). An id not a root instance is `invalid_input`; an instance erased later is dropped from scenes that stored it (in the same undo step as the erase). `scenes` query lists each scene's stored instance ids. `.skr` saves them.
4. **Going to a scene** that stores positions sets those instances' transforms in one undo step labelled "Go to scene <name>" (the camera part stays a view change as today). No positions stored: unchanged behaviour (no undo step).
5. **Transitions.** Model setting `scene_transition { on (default true), seconds (0 to 10, default 1) }` saved in `.skr`, in Model Info, and a command `set_scene_transition`. In the window, going to a scene animates the camera (eye, target, up interpolated; fov too; projection switches at the end) and stored positions over the duration with smoothstep easing; positions interpolate per instance as translation lerp, rotation slerp (of the polar-decomposed rotation) and scale lerp; the model change is applied at the start as the undo step, and the animation only affects drawing. Input during the animation finishes it at once. Agents (`go_to_scene` over the API) are not animated.
6. **Frames.** `render_view` takes `scene` (a PageId) and `t` (0 to 1, default 1) and optional `from` (a PageId; default the current view and positions): it renders the interpolated camera and positions at `t` without changing the document. With `scene` the `camera` param is not allowed.
7. **`explode_view { ids, factor, center? }`**: moves each listed instance of the open context by `(c_i - center) * factor`, where c_i is its bounding box center and `center` defaults to the center of all their boxes; `factor` finite, -0.99 to 100; one undo step; outcome `modified`. A non-instance id is `invalid_input`.

## Review Focus

1. Label anchors are on the target (inside faces, not in holes) and positions sensible; notes in images match the window's placement (pixel checks of a note's text region being non-background).
2. Offscreen egui pass: no GPU validation errors, works headless, works in the WebGL2 build or is switched off there with a reason.
3. Scene positions: undo of go-to restores; erase drops ids in the same step; `.skr` round trip; old files load.
4. Interpolation correctness (slerp, smoothstep, t = 0 and 1 exact); `render_view` leaves the revision unchanged.

---

### Task 1: add_label

**Files:** `crates/session/src/{command.rs, document.rs or document/annotations, batch.rs}`, `docs/agent-api.md`; tests.

- [ ] **Step 1: Write the failing tests** for ruling 1.
- [ ] **Step 2-5:** run, implement, gate, commit `git commit -m "add_label"`.

### Task 2: Annotations in rendered images

**Files:** `crates/app/src/{headless.rs, host*.rs}`, `crates/render/src/annotations.rs` (shared layout if needed), `crates/api` render hosts, print path if shared; tests (headless render with a note shows text pixels; dimensions; screen note).

- [ ] **Step 1: Write the failing tests** for ruling 2.
- [ ] **Step 2-5:** run, implement, gate (+ build-web), commit `git commit -m "Notes and dimensions in rendered images"`.

### Task 3: Scene positions and going to a scene

**Files:** `crates/kernel/src/{page.rs, data.rs, scene.rs}`, `crates/session/src/{command.rs, document.rs, query.rs}`, Scenes panel (a "Positions" checkbox per scene storing the selected root instances, or all root instances when none selected), `docs/agent-api.md`; tests.

- [ ] **Step 1: Write the failing tests** for rulings 3-4.
- [ ] **Step 2-5:** run, implement, gate, commit `git commit -m "Scenes store instance positions"`.

### Task 4: Transitions and frames

**Files:** `crates/session/src/{command.rs, camera.rs}`, kernel model setting and save/load, `crates/app/src/{editor.rs, navigation.rs, ui/*}` (animation), api render params, `docs/agent-api.md`; tests.

- [ ] **Step 1: Write the failing tests** for rulings 5-6.
- [ ] **Step 2-5:** run, implement, gate (+ build-web), commit `git commit -m "Scene transitions and transition frames"`.

### Task 5: Exploded view

**Files:** session command and handler, `docs/agent-api.md`; tests.

- [ ] **Step 1: Write the failing tests** for ruling 7.
- [ ] **Step 2-5:** run, implement, gate, commit `git commit -m "explode_view"`.

### After the tasks (controller)

- Final review; push `dev`; CI green; roadmap row M17f done; spec bullet; memory; merge and push; next milestone (M17g).
