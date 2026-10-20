# M16a Cut, Copy and Paste Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Cut, Copy, Paste and Paste In Place of the selection (edges, faces, instances, guides) through the system clipboard, within a model and across models (and app processes), carrying geometry, materials, tags and component definitions; Edit menu, Cmd+X/C/V, editor commands and `tool_input` coverage.

**Architecture:** A clip is a small standalone model: the kernel extracts the selected entities into a fresh `Scene` (root geometry drawn from a stencil in world coordinates, copied instances with the definitions they reach, guides, and only the materials and tags used), and inserts a clip `Scene` into a model at a transform, remapping materials and tags by name and definitions by id (same document) or as new definitions. The session wraps the clip as an in-memory `.skr` archive (the file format's own reader and validation) plus a small header (base point, source document token), encoded as text so it travels on the system text clipboard. `Command::Paste { clip, at? }` is the undoable edit. The app owns the clipboard (system clipboard through `arboard`, an in-memory one for tests and headless runs), the Edit menu items, the keys and a Paste tool that carries the clip's outline with the cursor (Move-style inference) until a click.

**Tech Stack:** Rust 2024, egui 0.36.2. `arboard =3.6.1` and `base64 =0.23.1` are already in `Cargo.lock` (arboard through egui-winit; base64 is a workspace dependency); adding them as direct dependencies at those exact versions is allowed. No other new dependencies.

**Spec:** `docs/superpowers/roadmap.md` row M16a.

## Global Constraints

- Only `arboard =3.6.1` (app crate) and the workspace `base64` may be added as direct dependencies; `Cargo.lock` must gain no new package.
- Existing behaviour unchanged; golden images unchanged; `.skr` schema unchanged (11).
- New command variants get arms in `Document::execute`/`edit` and `session::batch::id_slots` (no `_` arm); new editor commands are listed in `EDITOR_COMMANDS` and its test.
- Clip text is untrusted input: every paste validates it fully (the `.skr` reader's checks, size cap 64 MiB of decoded bytes) and refuses with `invalid_input` and nothing changed; never panics.
- Tests never touch the real system clipboard (they use the in-memory clipboard).
- Gate per task: fmt, clippy `-D warnings`, `cargo test --workspace`, and `cargo clippy --workspace --all-targets --target x86_64-pc-windows-msvc -- -D warnings`; CI on push. `export PATH="$HOME/.cargo/bin:$PATH"`.
- Docs (`docs/agent-api.md`) updated in the task that changes behaviour. No emoji. Short commit messages, no attribution lines. Never `git add -A`.

## Spec notes (rulings)

1. **What a clip holds:** the selected edges and faces (as a stencil: positions, marks, paint, texture placement, holes, tags), the selected instances with every definition they reach (nested), the selected guides, and the materials and tags any of these use. Vertices are not selectable and are not copied alone. An empty selection makes Copy/Cut a no-op with a status note (editor) or `invalid_input` (session).
2. **Coordinates:** the clip is stored in world (root) coordinates: what was copied inside an open group is transformed by the context's transform. Its **base point** is the minimum corner of the clip's world bounding box.
3. **Paste:** places the clip so its base point lands at `at` (a point in the open context's coordinates), translated only (no rotation), into the open context. **Paste In Place** (`at` absent) puts the clip back at its world coordinates, transformed by the inverse of the open context's transform. Loose geometry merges with touching geometry as Move's copy does (sticky). After either, the pasted entities are selected. One undo step labelled `paste`.
4. **Remapping:** materials are matched by name (an existing material with the same name is used; otherwise the clip's material is added, textures included); tags likewise by name (SketchUp merges by name). Component definitions: when the clip came from this same document (its source token matches the document's) and the definition id still exists, the pasted instances use that definition; otherwise a new definition is made with the clip's name. Groups always get new definitions (as copying a group does today).
5. **Document token:** each document gets a token when it is created, opened or imported-as-document (from the time and process id; not saved). Pasting into a new or reopened model therefore makes new definitions.
6. **Clip text:** `sketchup-alt clip 1\n` followed by base64 (standard alphabet) of a zip archive holding `model.json` exactly as `.skr` writes it (textures as entries, as in `.skr`) and `clip.json` `{ "base": [x,y,z], "source": "<token>" }`. The `.skr` writer/reader get in-memory variants (`write_bytes`/`read_bytes`) that the path functions use, so the format and checks are shared. Text not starting with the header is "not a sketchup-alt clip".
7. **Commands:** session `Command::Paste { clip: String, at: Option<[f64;3]> }` (undoable edit). Editor commands (api `EditorCommand`, no undo of their own): `copy {}`, `cut {}` (copy, then `erase` of the same ids as one undo step labelled `cut`), `paste { at?: [f64;3] }` (with `at`: pastes there directly; without: starts the Paste tool with the cursor like Duplicate), `paste_in_place {}`. They act on the current selection and the app clipboard. A session helper `Document::clip(&[EntityId]) -> Result<String, CommandError>` produces the text.
8. **Window:** Edit menu gains Cut (Cmd+X), Copy (Cmd+C), Paste (Cmd+V), Paste In Place (no key) after Redo; Cut/Copy enabled with a selection, Paste/Paste In Place enabled when the clipboard holds a clip (checked when the menu opens, not every frame). With a text field focused, the keys and items act on the text field (add them to `yields_to_text`). The Paste tool shows the clip's edges as a preview following the cursor with Move's inference from the base point; a click pastes there (`Command::Paste`), Esc cancels and returns to the previous tool; typed distances are not supported in this milestone. Pasting text that is not a clip shows "The clipboard does not hold sketchup-alt geometry" and changes nothing.
9. **Across windows:** the app has one window per process; "across windows" means across app processes through the system clipboard. No in-process window list is added.
10. **tool_input:** `Editor::press` accepts Cmd+X, Cmd+C, Cmd+V chords (`x`, `c`, `v` with `cmd`, and `ctrl` on non-macOS where the other chords already do so).

## Review Focus

1. Copy inside an open rotated and scaled group, close it, Paste In Place at the root: the pasted geometry sits at the same world position (and the reverse: copy at root, paste in place inside the rotated group).
2. Copy a component instance and paste twice in the same model: all instances share one definition; paste into a new model: one new definition with its materials and tags, nested definitions included.
3. Undo of a paste removes everything it added (geometry, instances, new definitions, new materials, new tags); undo of cut restores the cut entities.
4. Clipboard text that is not ours, is truncated, has a bad base64 body, a damaged zip, a model.json that fails `.skr` checks, or exceeds the size cap: `invalid_input`, model unchanged, no panic.
5. Cmd+C / Cmd+V with a text field focused copy and paste text, not geometry.

---

### Task 1: Clips in the kernel (extract and insert)

**Files:** `crates/kernel/src/{clip.rs (new), lib.rs, transform.rs (expose what extraction needs), group.rs}`; tests `crates/kernel/tests/integration/clip.rs`.

**Interfaces (produces):**
- `Scene::extract(&self, mesh: MeshId, edges: &[EdgeId], faces: &[FaceId], instances: &[InstanceId], guides: &[GuideId], to_world: DAffine3) -> Result<Scene, KernelError>`: a new standalone scene whose root holds the entities transformed by `to_world`, with every definition reached, and only the materials and tags used (same names, colors, textures; ids may differ).
- `Scene::insert(&mut self, parent: MeshId, clip: &Scene, placement: DAffine3, reuse: &dyn Fn(DefinitionId) -> Option<DefinitionId>) -> Result<(Inserted, SceneChange), KernelError>` where `Inserted { edges: Vec<EdgeId>, faces: Vec<FaceId>, instances: Vec<InstanceId>, guides: Vec<GuideId> }`; materials and tags matched by name per ruling 4; `reuse` maps a clip component definition to an existing one (or `None` for a new one); groups always new. On error nothing is changed.

- [ ] **Step 1: Write the failing tests:** round trip of a face with a hole, paint, texture placement, soft/curve marks and a tag; instances with nested components and groups (component definitions shared within the clip stay shared); guides; materials and tags only those used; insert at a translation and at a rotation; insert matches materials and tags by name and adds missing ones; `reuse` makes instances share an existing definition; undo (applying the inverse change) restores the target exactly; extracting an empty selection gives an empty scene.
- [ ] **Step 2: Run** — expected: failures.
- [ ] **Step 3: Implement** using `stencil`/`draw_stencil`, `copy_instances_placed`/`duplicate` and `copy_guides` patterns.
- [ ] **Step 4: Run** the gate.
- [ ] **Step 5: Commit** `git commit -m "Clips in the kernel"`.

### Task 2: Clip text and the paste command

**Files:** `crates/session/src/{clip.rs (new), file.rs (write_bytes/read_bytes), command.rs, document.rs, batch.rs, lib.rs}`, `crates/api/tests/integration/dispatch.rs`, `docs/agent-api.md`; tests `crates/session/tests/integration/clip.rs`.

**Interfaces (consumes Task 1; produces):**
- `Document::clip(&self, ids: &[EntityId]) -> Result<String, CommandError>` (ruling 6 text).
- `Document::token(&self) -> &str` (ruling 5).
- `Command::Paste { clip: String, at: Option<[f64;3]> }`, name `paste`.
- `session::clip::outline(text: &str) -> Result<ClipOutline, CommandError>` with `ClipOutline { base: DVec3, segments: Vec<[DVec3; 2]> }` (world coordinates relative to nothing; the app offsets by the base) for the Paste tool preview, without building a model.

- [ ] **Step 1: Write the failing tests** for rulings 2-6 and Review Focus 1-4 (session level), including: `.skr` save/open still byte-compatible after the `write_bytes`/`read_bytes` split (existing file tests stay green); clip text round trip; `paste` selects the pasted entities; paste into an open group; same-document component reuse vs new document; every bad-input case of Review Focus 4.
- [ ] **Step 2: Run** — expected: failures.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** the gate.
- [ ] **Step 5: Commit** `git commit -m "Clip text and the paste command"`.

### Task 3: Clipboard, Edit menu, keys and the Paste tool

**Files:** `crates/app/{Cargo.toml, src/clipboard.rs (new), src/editor.rs (+ editor/input.rs), src/menus.rs, src/shortcuts.rs, src/ui/mod.rs, src/app.rs, src/host.rs}`, `crates/tools/src/{paste.rs (new), lib.rs}`, `crates/api/src/editor.rs`, `crates/api/src/mcp/tools.rs` if editor commands are listed there, `docs/agent-api.md`; tests in `crates/app/src/ui/tests/`, `crates/app/src/host/tool_input_tests.rs`, editor tests, `crates/tools/tests/scenery.rs`.

**Interfaces (consumes Task 2):** an app `Clipboard` trait (`get_text() -> Option<String>`, `set_text(String)`) with `SystemClipboard` (arboard; failures become a status message, never a panic) and `MemoryClipboard`; the Editor holds a `Box<dyn Clipboard>` (memory in tests and headless).

- [ ] **Step 1: Write the failing tests:** editor `copy`/`cut`/`paste {at}`/`paste_in_place` through the host (editor commands) with the memory clipboard; cut is one undo step; Paste tool previews and places on click, Esc cancels; menu items present with their enabled states; Cmd+X/C/V chords in `tool_input`; Review Focus 5 (text field focused); non-clip text gives the ruling 8 status and no change.
- [ ] **Step 2: Run** — expected: failures.
- [ ] **Step 3: Implement** rulings 7-10.
- [ ] **Step 4: Run** the gate.
- [ ] **Step 5: Commit** `git commit -m "Cut, copy and paste in the window"`.

### After the tasks (controller)

- Push `dev`; CI green. Roadmap row M16a done; spec bullet; memory; merge and push; next milestone (M16b).
