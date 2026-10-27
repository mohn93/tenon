# M10a Agent Sight and State Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Agents see and set the editor around the model: `pick` and `infer` queries say what lies under a pixel (or along a ray) and what a tool's click there would snap to; the `editor` query reports the tool, its phase and points, paint material, fly mode, view size, status bar, measurements box, import units, soften angle and open window; `set_tool`, `set_paint_material`, `set_import_units` and `cancel` set it; `history` lists undo and redo step labels; subscribers get `selection`, `camera` and `editor` notifications (at most one of each per frame); `quit {discard}` ends the window or headless app; and agent-api.md lists every window action with its command or as window chrome.

**Architecture:**
- `session`: undo steps carry a label (`Command::name`, the command's `type`; an atomic batch takes its edits' shared name, else `batch`); the `history` query lists them. `ImportUnits` moves here from the app (serde). `check_coordinates`, `unit_vector` and `Document::check_unsaved` become public for the API's checks.
- `tools`: `Tool::phase() -> Phase` (`Idle`, or `Active { points }`) on all eleven tools; `Scenery::path(k)` (each part's instances from the root); `infer_detail` (the inference plus the entities it snapped to, by part); `probe::pick` (every face along a ray, plus edges and guides within a screen or meter reach).
- `api` (now depending on `tools` and `kernel`): `EditorQuery` (`pick`, `infer`, `editor`) answered before `session::Query`; `probe` answers pick and infer from the document; `editor` module holds `ToolName`, `EditorState`, `EditorCommand`; `Host` gains `editor`, `editor_command` and `quit` with refusing defaults (`HeadlessHost` keeps them: it has no editor); `execute` routes editor commands, a new `quit` method; `Hub::notify_state` sends coalesced `selection`, `camera`, `editor` notifications and `Hub::close` waits for transports to write the last replies.
- `app`: `AppHost` (was `WindowHost`) over the `Editor`, an `Offscreen` renderer and a quit flag, used by the window and now by headless mode too (headless runs the window's `Editor`, 1280 x 800, scale 1); `editor_state` maps the Editor to `EditorState`; the Editor gains `phase`, `typed`, `flying_speed`, `soften_state`, `set_paint_material`, `cancel`; the window sends state notifications once per frame, exits on `quit` and flushes replies on exit.

**Tech Stack:** Rust 2024; serde, schemars, serde_json; no new external dependencies (the `api` crate gains path dependencies on `tools` and `kernel`).

**Spec:** `docs/superpowers/specs/2026-10-10-tenon-design.md` ("Agent control milestones", M10a; section 5 for the API).

## Global Constraints

- No emoji anywhere (code, comments, docs, commit messages). Comments succinct, in the style of the surrounding code.
- `cargo fmt --all` clean; `cargo clippy --workspace --all-targets -- -D warnings` clean; `cargo test --workspace` passes after every task. Cargo is at `$HOME/.cargo/bin/cargo` (not on the subagents' PATH).
- No new external dependencies.
- From the spec: queries `pick` (a pixel of a view, given width, height and an optional camera like `render_view`'s, or a ray `{origin, direction}`; hits front to back: entity id with its instance path from the root, point, normal and distance), `infer` (a pixel or a model point, optionally `from` a point; kind, point, the entities involved and the axis lock), `editor` (tool and phase with points so far, paint material, fly mode, viewport size, status text, measurements box text, import units, soften angle, open dialog); commands `set_tool {tool}`, `set_paint_material {material}`, `set_import_units {units}`, `cancel`; `history` lists undo and redo step labels; `subscribe` also sends `selection`, `camera` and `editor` notifications when they change, whoever changed them, coalesced to at most one of each per frame; method `quit {discard}` ends the app (window or headless), refusing with unsaved changes unless `discard`; every window action has a command or is window chrome listed in agent-api.md.
- Keep the interfaces M10b and M10c build on clean: `Phase`, `EditorState`, `Host::editor`/`editor_command`, `Hub::notify_state`, `AppHost`.
- Git: work on `dev`; other agents may share the checkout: run `git status` and `git diff` first and stage only the files you changed (never `git add -A` or `git add .`); short plain commit messages, no attribution lines.
- Do not launch the app window. Tests that start the binary use their own scratch sockets; never touch `~/.tenon/house.sock` or `h.sock`.

## Spec notes (rulings made in this plan)

- **Where each part lives.** Pure geometry questions live in `tools` (`probe::pick`, `infer_detail`, `Tool::phase`); the JSON shapes and checks in `api` (it now depends on `tools`, an internal crate); the editor itself stays in `app`. `pick`, `infer` and `editor` are query types the API answers itself before falling back to `session::Query` (`EditorQuery`); editor commands are answered by `execute` before `session::Command` (`EditorCommand`). They are not session commands: they change no model, undo history or revision, and a bare `Document` has no editor.
- **Coordinates.** Pixels and the view's camera are in root coordinates, like `render_view`; points, rays, normals and distances in `pick` and `infer` are in the open context's coordinates, like commands and every other result (the camera is carried into the context with `Camera::placed`, as the Editor does). Instance paths are always from the root, whatever is open.
- **Views for pixels.** A pixel is `[x, y]` from the view's top left (x right, y down), unrounded like the window's cursor. `width` and `height` are optional (both or neither, 1 to 4096); without them the view is the document camera's viewport: the window's 3D view, or 1280 x 800 headless. `camera` takes `render_view`'s fields (shared `View` struct). A pixel outside the view is `invalid_input`.
- **`pick`.** Every face the ray crosses in front of its origin, whatever lies in front (`Index::ray_hits` per placed mesh), plus every non-soft edge (soft edges are hidden and never picked in the window either) and guide whose point nearest the ray lies within `radius`: pixels on screen for a pixel (default 4, at most 1000), meters from the ray for a `ray` (default 0, so a ray meets faces only). No vertices (`infer` finds them). Each hit: `id`, `path`, `point`, `distance` along the ray from its origin (the eye in perspective, the near end of the view in parallel); faces add `normal` (front side, unit, by the inverse transpose of the placement) and `side` (`front` when the ray meets the front, i.e. normal against the ray). Sorted by distance; a component placed twice gives the same id twice with different paths. `camera`, `width` and `height` are refused with a ray.
- **`infer`.** Runs the window's inference (`infer_detail`, the same ranking and visibility as `infer_in`) with only `from` as the anchor: no reference edge (so no parallel or perpendicular snaps), no Shift or arrow-key lock, and the editor's state is not used. Default radius 8 pixels (the window's at scale 1). A `point` is projected with the view's camera and inferred at that pixel; behind the camera or outside the view it is `invalid_input`. Kinds: `endpoint`, `midpoint`, `origin`, `guide_point`, `intersection`, `on_edge`, `on_guide`, `axis`, `parallel`, `perpendicular`, `on_face`, `ground` and `nothing` (the tools' `Free`). There is no `center` kind: the tools infer no centers. "The axis lock" is the `axis` field (`x`, `y`, `z`) on an `axis` snap: the direction Shift would lock. `entities` (with paths): the vertex of an endpoint, the edge of a midpoint or on-edge point, the guide of a guide point or on-guide point, both edges or guide lines of an intersection (the one the point lies on first), the face of an on-face point; none otherwise.
- **Tool phases.** `Phase::Idle` or `Phase::Active { points }`: Line, the chain's first point and the point the next edge starts from (one point when they coincide); Rectangle, the first corner; Circle, the center; Arc, the start then the end; Push/Pull, where the face was clicked; Move, the base point; Rotate, the center then where the angle starts (a new `Turn.start` field: the click projected onto the rotation's plane); Tape Measure, the start. Select is `Active` with no points while its button is held (click or box, both on screen); Erase and Paint are always `Idle`. A Move or Rotate waiting for an array after a copy is `Idle`.
- **`editor` state.** `tool`, `phase`, `points`, `paint_material` (null: default), `import_units`, `flying`, `fly_speed`, `viewport`, `status` (the status bar line as the window shows it), `prompt`, `message`, `lock`, `measurement_label`, `measurement` (the typed text while typing, else the live measurement), `typing`, `soften_angle` (degrees) and `dialog` (`soften_edges` or null). The native file and unsaved-changes dialogs are modal and block the event loop, so the API cannot see them: requests wait until they close (documented). While flying, `tool` stays the active tool (the status bar says Fly).
- **Editor commands.** Only `execute` takes them (a `batch` of one is `-32602`, since `batch` parses session commands); they return `{"editor": <state after>, "revision"}`. `set_tool` is the toolbar's `select_tool` (a fresh tool, dropping typed text). `set_paint_material` keeps the tool (the Materials list's click also switches to Paint) and is `not_found` for an unknown material. `cancel` is Escape pressed until the tool is idle, but never closes the open group: it drops typed text, the lock, a pending press, and the tool's operation through `on_external_change` (so also a pending copy array or retyped angle). A host without an editor (`api::HeadlessHost`) refuses them, the `editor` query and `quit` with `invalid_input`.
- **History labels.** A step's label is `Command::name()`, the command's serde `type` (`add_edge`); an atomic batch is labeled by its edit commands' shared name, else `batch`. Window edits are labeled the same way, since the window runs the same commands. The `history` result adds `undo` and `redo` lists, next first (the first of `undo` is what `undo` undoes).
- **State notifications.** `Hub::notify_state(host)` compares the selection (ids), the `camera` query's result (which includes `lighting`) and the editor state with what subscribers were last told and sends `selection` (`{ids, revision}`), `camera` (the camera query without `type`) and `editor` (the state) for those that differ. The window calls it once per frame drawn (after the frame's panel actions); headless calls it after each round of requests (`process_blocking`). So each is coalesced and carries the state at that moment; a change undone within a frame sends nothing. The baseline is taken when the first connection subscribes; nothing is kept while nobody subscribes. In the window they follow the replies and `changed` notifications of the requests that caused them; a window drawing no frames (minimized) sends them at its next frame. The window's `process_api` also asks for a frame when only the editor state changed (`shown`).
- **`quit`.** A JSON-RPC method (params `{discard?}`, absent params allowed), not a command. The host checks `Document::check_unsaved(discard)`, sets a flag and replies `{}`; the window then exits without its own prompt, and headless stops serving. `Hub::close(wait)` drops unhandled requests and the connections' senders, then waits (2 s, `CLOSE_WAIT`) for transport writer threads (counted by a new `Writing` guard) to finish, so the reply reaches the agent; headless socket mode removes its socket file; stdio drops the hub and joins its writer as before. Requests already waiting in the same round as `quit` are still answered.
- **Headless runs the editor.** `app --headless` now uses `AppHost` over the window's `Editor` (viewport 1280 x 800, scale factor 1) instead of `api::HeadlessHost`, so editor queries and commands, phases and `quit` work the same headless. `api::HeadlessHost` stays as the bare host the API's own tests use.
- **`ImportUnits`** moves to `session` with serde (`meters`, `centimeters`, `millimeters`, `inches`, `feet`); `app::editor` re-exports it so the UI code is unchanged.
- **describe** keeps `schema_version` 1 (additions only) and adds the `quit` method and the `EditorQuery`, `PickHit`, `Inferred`, `EditorCommand` and `EditorState` schemas.
- **The control promise.** agent-api.md gets a "Window actions and the API" table mapping every menu item, button, panel control and gesture to its command or query, and names the window chrome (window size and position, panel sizes and scrolling, open menus, tooltips, expanded Outliner rows, the export dialog's suggested format, focus, the file dialogs). Fly mode and mouse navigation are window-only ways of reaching what `set_camera`/`orbit`/`pan`/`zoom` reach (their state is reported in `editor`). The spec's Goal paragraph says the same.
- **Unknown fields on unit variants.** serde accepts extra fields on unit variants of internally tagged enums (`{"type": "cancel", "x": 1}`), as it already does for `undo` and `stats`; left as it is (recorded as deferred).

## File Structure

| File | Responsibility | Task |
|---|---|---|
| `crates/session/src/command.rs` | `Command::name` | 1 |
| `crates/session/src/document.rs` | labeled undo steps (`Step`), history lists; public `check_coordinates`, `unit_vector` | 1, 4 |
| `crates/session/src/query.rs` | `QueryResult::History.undo/redo` | 1 |
| `crates/session/tests/contract.rs` | history label tests | 1 |
| `crates/tools/src/tool.rs` | `Phase`, `Tool::phase`, phase test | 2 |
| `crates/tools/src/{line,rectangle,circle,arc,push_pull,move_tool,rotate,tape,select,erase,paint}.rs` | each tool's `phase`; Rotate's `Turn.start` | 2 |
| `crates/tools/src/scenery.rs` | part paths (`Found`, `Layout.paths`, `Scenery::path`) | 3 |
| `crates/tools/src/inference.rs` | `infer_detail`, `Involved`, `nearest_face_part`, `Near::in_cone`, `Cone::along` | 3 |
| `crates/tools/src/probe.rs` (new) | `Hit`, `Reach`, `pick` | 3 |
| `crates/api/Cargo.toml`, `Cargo.lock` | `tools` and `kernel` path dependencies | 4 |
| `crates/api/src/probe.rs` (new) | `View`, `PickQuery`, `InferQuery`, results, `pick_query`, `infer_query` | 4 |
| `crates/api/src/dispatch.rs` | `EditorQuery`, routing, `Host` additions, `quit`, describe | 4, 5 |
| `crates/api/src/editor.rs` (new) | `ToolName`, `PhaseName`, `Dialog`, `EditorState`, `EditorCommand` | 5 |
| `crates/session/src/import.rs`, `document/files.rs`, `lib.rs` | `ImportUnits`; public `check_unsaved` | 5 |
| `crates/api/src/hub.rs`, `transport.rs` | `notify_state`, `close`, writer counting | 6 |
| `crates/app/src/editor/agent.rs` (new) | Editor's agent methods | 7 |
| `crates/app/src/host.rs` | `Offscreen`, `AppHost`, `editor_state`, `CLOSE_WAIT` | 7, 8 |
| `crates/app/src/headless.rs` | headless on the Editor, `quit`, socket removal | 7, 8 |
| `crates/app/src/app.rs` | `offscreen`/`quit` state, `shown`, per-frame notifications, exit on quit, close on exit | 7, 8 |
| `crates/api/tests/{probe,editor,hub}.rs`, `crates/app/tests/headless_agent.rs` (new) | API and end-to-end tests | 4-7 |
| `docs/agent-api.md`, the spec | every new query, command, method and notification; window actions | 1, 4, 7, 8 |

The code below was written and verified in this repository's working tree on `dev` at 705c1f0 (then reverted), task by task: after each task `cargo fmt --all --check` and `cargo clippy -p <crate> --all-targets -- -D warnings` passed and the tests named in the task passed; after Task 8 `cargo clippy --workspace --all-targets -- -D warnings` passed and `cargo test --workspace` passed: 1066 passed, 10 ignored, none failed (`--no-fail-fast`, summing every "test result" line; the tasks add 31 tests: 2, 1, 5, 6, 5, 3, 8, 1). The verified states are kept as trees at `refs/scratch/m10a/t1` .. `t8` (`git diff refs/scratch/m10a/t3 refs/scratch/m10a/t4` shows Task 4's change; except that Task 7 carries the docs written with Tasks 5 and 6: `git diff refs/scratch/m10a/t4 refs/scratch/m10a/t5 -- docs/agent-api.md` and `t6..t7`. Applying the tasks' diffs in order, docs included, gives `t8` exactly). Code for new files is given in full; changes to existing files are unified diffs against the state the previous task leaves. Where a source file carries its own unit tests, they are in its diff, so the step that runs them before the change fails to compile or finds no tests.

---

### Task 1: Undo step labels and the history lists

**Files:**
- Modify: `crates/session/src/command.rs`, `crates/session/src/document.rs`, `crates/session/src/query.rs`
- Test: `crates/session/tests/contract.rs`, `crates/session/tests/commands.rs` (an exact history result gains the lists)
- Docs: `docs/agent-api.md`

**Interfaces:**
- Consumes: nothing new.
- Produces:
  - `session::Command::name(&self) -> String`: the serde `type` ("add_edge").
  - `QueryResult::History { undo_depth: usize, redo_depth: usize, undo: Vec<String>, redo: Vec<String>, revision: u64 }`, lists next first.
  - Private: `struct Step { label: String, changes: SceneChange }`; `Document::record(&mut self, label: String, changes: SceneChange) -> Outcome`.

- [ ] **Step 1: Write the failing tests**

Append to `crates/session/tests/contract.rs`:

```diff
--- a/crates/session/tests/contract.rs
+++ b/crates/session/tests/contract.rs
@@ -207,3 +207,55 @@ fn truncation_boundary_is_exact() {
     assert_eq!(r["truncated"], false);
     assert_eq!(r["events"].as_array().unwrap().len(), 1024);
 }
+
+#[test]
+fn history_labels_each_step_by_its_command() {
+    let mut d = doc();
+    d.execute(edge([5.0, 0.0, 0.0], [6.0, 0.0, 0.0])).unwrap();
+    // An atomic batch of one kind of command is labeled by it.
+    d.execute_batch(square(), true).unwrap();
+    d.execute(cmd(
+        json!({"type": "add_guide_point", "position": [9, 9, 0]}),
+    ))
+    .unwrap();
+    // Neither the camera nor the selection is a step.
+    d.execute(cmd(json!({"type": "orbit", "yaw": 10}))).unwrap();
+    d.execute(cmd(json!({"type": "select", "ids": []})))
+        .unwrap();
+    d.execute(Command::Undo).unwrap();
+    let h = ask(&d, json!({"type": "history"}));
+    assert_eq!(h["undo"], json!(["add_edge", "add_edge"]));
+    assert_eq!(h["redo"], json!(["add_guide_point"]));
+    assert_eq!(
+        (h["undo_depth"].clone(), h["redo_depth"].clone()),
+        (json!(2), json!(1))
+    );
+
+    // A batch of differing commands is a "batch"; a new step clears the redo list.
+    let mixed = vec![
+        edge([0.0, 5.0, 0.0], [1.0, 5.0, 0.0]),
+        cmd(json!({"type": "add_guide_point", "position": [8, 8, 0]})),
+    ];
+    d.execute_batch(mixed, true).unwrap();
+    let h = ask(&d, json!({"type": "history"}));
+    assert_eq!(h["undo"], json!(["batch", "add_edge", "add_edge"]));
+    assert_eq!(h["redo"], json!([]));
+
+    // Undo and redo carry the label across.
+    d.execute(Command::Undo).unwrap();
+    d.execute(Command::Undo).unwrap();
+    d.execute(Command::Redo).unwrap();
+    let h = ask(&d, json!({"type": "history"}));
+    assert_eq!(h["undo"], json!(["add_edge", "add_edge"]));
+    assert_eq!(h["redo"], json!(["batch"]));
+}
+
+#[test]
+fn command_names_are_their_types() {
+    assert_eq!(edge([0.0; 3], [1.0, 0.0, 0.0]).name(), "add_edge");
+    assert_eq!(Command::PurgeDefinitions.name(), "purge_definitions");
+    assert_eq!(
+        cmd(json!({"type": "push_pull", "face": "f1", "distance": 1})).name(),
+        "push_pull"
+    );
+}
```

In `crates/session/tests/commands.rs` (`undo_and_redo_walk_history`), the exact history result gains the lists:

```diff
--- a/crates/session/tests/commands.rs
+++ b/crates/session/tests/commands.rs
@@ -72,7 +72,8 @@ fn undo_and_redo_walk_history() {
     assert_eq!(ask(&d, json!({"type": "stats"}))["faces"], 0);
     assert_eq!(
         ask(&d, json!({"type": "history"})),
-        json!({"type": "history", "undo_depth": 3, "redo_depth": 1, "revision": 5})
+        json!({"type": "history", "undo_depth": 3, "redo_depth": 1,
+               "undo": ["add_edge", "add_edge", "add_edge"], "redo": ["add_edge"], "revision": 5})
     );
     run(&mut d, json!({"type": "redo"})).unwrap();
     assert_eq!(ask(&d, json!({"type": "stats"}))["faces"], 1);
```

- [ ] **Step 2: Run them to see them fail**

Run: `$HOME/.cargo/bin/cargo test -p session --test contract --test commands`
Expected: FAIL to compile: no method `name` on `Command`.

- [ ] **Step 3: Label the steps**

```diff
--- a/crates/session/src/command.rs
+++ b/crates/session/src/command.rs
@@ -360,6 +360,15 @@ pub enum Command {
 }
 
 impl Command {
+    /// The command's `type` as agents write it ("add_edge", "push_pull"): the label of the undo
+    /// step it makes.
+    pub fn name(&self) -> String {
+        serde_json::to_value(self)
+            .ok()
+            .and_then(|v| v.get("type").and_then(|t| t.as_str()).map(str::to_owned))
+            .unwrap_or_default()
+    }
+
     /// True for commands that can change the model or the open context (everything except
     /// camera, selection, saving and exporting).
     pub fn changes_model(&self) -> bool {
```

```diff
--- a/crates/session/src/document.rs
+++ b/crates/session/src/document.rs
@@ -38,6 +38,12 @@ pub const MAX_COPIES: u32 = 1000;
 /// How many recent model changes `Query::Events` can return.
 const EVENT_LOG_LEN: usize = 1024;
 
+/// One undo step: the model change and the label of what made it (see `Command::name`).
+struct Step {
+    label: String,
+    changes: SceneChange,
+}
+
 /// The open model plus everything needed to edit it: camera, selection, the open context and
 /// undo history.
 pub struct Document {
@@ -48,8 +54,8 @@ pub struct Document {
     /// How the view is lit; saved with the camera.
     pub lighting: Lighting,
     selection: BTreeSet<EntityId>,
-    undo: Vec<SceneChange>,
-    redo: Vec<SceneChange>,
+    undo: Vec<Step>,
+    redo: Vec<Step>,
     revision: u64,
     events: VecDeque<ChangeEvent>,
     /// Changes up to this revision are missing from `events` (the model was replaced).
@@ -169,7 +175,7 @@ impl Document {
     /// Runs one command. Each modeling command is one undo step; camera commands are not recorded.
     pub fn execute(&mut self, command: Command) -> Result<Outcome, CommandError> {
         if let Some(changes) = self.edit(&command) {
-            let mut outcome = self.record(changes?);
+            let mut outcome = self.record(command.name(), changes?);
             outcome.import = self.import_report.take();
             outcome.soften = self.soften_report.take();
             outcome.positioned = self.positioned.take();
@@ -177,21 +183,21 @@ impl Document {
         }
         match command {
             Command::Undo => {
-                let changes = self.undo.pop().ok_or_else(|| {
+                let step = self.undo.pop().ok_or_else(|| {
                     CommandError::new(ErrorKind::NothingToUndo, "nothing to undo")
                 })?;
-                let inverse = changes.inverse();
+                let inverse = step.changes.inverse();
                 self.scene.apply(&inverse);
-                self.redo.push(changes);
+                self.redo.push(step);
                 Ok(self.changed(&inverse))
             }
             Command::Redo => {
-                let changes = self.redo.pop().ok_or_else(|| {
+                let step = self.redo.pop().ok_or_else(|| {
                     CommandError::new(ErrorKind::NothingToRedo, "nothing to redo")
                 })?;
-                self.scene.apply(&changes);
-                let outcome = self.changed(&changes);
-                self.undo.push(changes);
+                self.scene.apply(&step.changes);
+                let outcome = self.changed(&step.changes);
+                self.undo.push(step);
                 Ok(outcome)
             }
             Command::SetCamera { .. }
@@ -475,11 +481,14 @@ impl Document {
         let selection_before = self.selection.clone();
         let context_before = self.context.clone();
         let mut applied: Vec<SceneChange> = Vec::new();
+        // The edits' names: the step is labeled by theirs when they share one.
+        let mut names: Vec<String> = Vec::new();
         for (index, command) in commands.into_iter().enumerate() {
             let result = match self.edit(&command) {
                 Some(changes) => changes.map(|c| {
                     let summary = c.summary();
                     applied.push(c);
+                    names.push(command.name());
                     Outcome {
                         changes: summary,
                         import: self.import_report.take(),
@@ -524,7 +533,11 @@ impl Document {
                 }
             }
         }
-        self.record(SceneChange::join(applied));
+        let label = match names.first() {
+            Some(first) if names.iter().all(|n| n == first) => first.clone(),
+            _ => "batch".to_string(),
+        };
+        self.record(label, SceneChange::join(applied));
         Ok(outcomes)
     }
 
@@ -975,12 +988,13 @@ impl Document {
         }
     }
 
-    fn record(&mut self, changes: SceneChange) -> Outcome {
+    /// Records `changes` as an undo step labeled `label`, unless there are none.
+    fn record(&mut self, label: String, changes: SceneChange) -> Outcome {
         if changes.is_empty() {
             return Outcome::default();
         }
         let outcome = self.changed(&changes);
-        self.undo.push(changes);
+        self.undo.push(Step { label, changes });
         self.redo.clear();
         outcome
     }
@@ -1101,6 +1115,8 @@ impl Document {
             Query::History => QueryResult::History {
                 undo_depth: self.undo.len(),
                 redo_depth: self.redo.len(),
+                undo: self.undo.iter().rev().map(|s| s.label.clone()).collect(),
+                redo: self.redo.iter().rev().map(|s| s.label.clone()).collect(),
                 revision: self.revision,
             },
             Query::Selection => QueryResult::Selection {
```

```diff
--- a/crates/session/src/query.rs
+++ b/crates/session/src/query.rs
@@ -327,9 +327,14 @@ pub enum QueryResult {
         definitions: usize,
         revision: u64,
     },
+    /// Undo history: how many steps can be undone and redone, and their labels (the `type` of
+    /// the command that made each, or `batch` for an atomic batch of differing commands), the
+    /// next to undo or redo first.
     History {
         undo_depth: usize,
         redo_depth: usize,
+        undo: Vec<String>,
+        redo: Vec<String>,
         revision: u64,
     },
     Selection {
```

- [ ] **Step 4: Run the tests**

Run: `$HOME/.cargo/bin/cargo test -p session`
Expected: PASS.

- [ ] **Step 5: Document the history lists**

In `docs/agent-api.md`, replace the Queries row

```
| `{"type": "stats"}`, `{"type": "history"}` | counts for the open context (plus the number of definitions) and revision; undo/redo depth |
```

with

```
| `{"type": "stats"}` | counts for the open context (plus the number of definitions) and revision |
| `{"type": "history"}` | `undo_depth` and `redo_depth`, and the steps as labels, next first: `undo` (what `undo` would undo, then the step before it) and `redo` (what `redo` would redo first). A step's label is the `type` of the command that made it (`add_edge`, `push_pull`); an atomic batch is labeled by its edit commands' type when they share one, else `batch`. Window edits are labeled the same way. Plus `revision` |
```

- [ ] **Step 6: Check and commit**

Run: `$HOME/.cargo/bin/cargo fmt --all --check && $HOME/.cargo/bin/cargo clippy --workspace --all-targets -- -D warnings && $HOME/.cargo/bin/cargo test --workspace`
Expected: clean; all pass.

```bash
git status && git diff
git add crates/session/src/command.rs crates/session/src/document.rs crates/session/src/query.rs crates/session/tests/contract.rs crates/session/tests/commands.rs docs/agent-api.md
git commit -m "Session: undo steps are labeled; history lists them"
```

---

### Task 2: Tool phases

**Files:**
- Modify: `crates/tools/src/tool.rs` (with its test), `crates/tools/src/lib.rs`, and each tool: `line.rs`, `rectangle.rs`, `circle.rs`, `arc.rs`, `push_pull.rs`, `move_tool.rs`, `rotate.rs`, `tape.rs`, `select.rs`, `erase.rs`, `paint.rs` (all in `crates/tools/src/`)

**Interfaces:**
- Consumes: nothing new.
- Produces:
  - `tools::Phase` (re-exported from `tools::tool`): `#[derive(Clone, Debug, PartialEq)] pub enum Phase { Idle, Active { points: Vec<DVec3> } }`.
  - `Tool::phase(&self) -> Phase` (required, no default), implemented by all eleven tools as in the Spec notes.
  - Rotate's private `Turn` gains `start: Option<DVec3>`.

- [ ] **Step 1: Write the failing test**

The test is the test-module part of the `tool.rs` diff in Step 3: the two `use` lines after `use super::*;`, and `rounded`, `phases`, `active` and `phases_list_the_points_clicked_so_far` at the end. Add that part first.

- [ ] **Step 2: Run it to see it fail**

Run: `$HOME/.cargo/bin/cargo test -p tools --lib tool::`
Expected: FAIL to compile: no variant or type `Phase`, no method `phase`.

- [ ] **Step 3: Give every tool a phase**

```diff
--- a/crates/tools/src/tool.rs
+++ b/crates/tools/src/tool.rs
@@ -78,6 +78,16 @@ pub struct ToolOutput {
     pub sampled: Option<Option<MaterialId>>,
 }
 
+/// Where a tool is in its operation.
+#[derive(Clone, Debug, PartialEq)]
+pub enum Phase {
+    /// No operation in progress: the next click starts one.
+    Idle,
+    /// An operation in progress, with the points it was given so far, in the open context's
+    /// coordinates (none for the Select tool's click or box, which happen on screen).
+    Active { points: Vec<DVec3> },
+}
+
 /// An interactive way to build commands. Tools never edit the model themselves.
 pub trait Tool {
     fn name(&self) -> &'static str;
@@ -87,6 +97,8 @@ pub trait Tool {
     fn measurement(&self) -> (&'static str, Option<String>);
     /// The point axis inference starts from, if any.
     fn anchor(&self) -> Option<DVec3>;
+    /// Whether an operation is in progress, and the points it was given so far.
+    fn phase(&self) -> Phase;
     fn handle(&mut self, event: ToolEvent, ctx: &ToolContext) -> ToolOutput;
     /// The model changed outside the tool (agent edit, undo, redo): drop any operation in
     /// progress, since it may refer to geometry that no longer exists.
@@ -210,6 +222,8 @@ impl ToolKind {
 #[cfg(test)]
 mod tests {
     use super::*;
+    use crate::inference::SnapKind;
+    use kernel::FaceId;
 
     #[test]
     fn labels_match_tool_names_and_shortcuts_round_trip() {
@@ -220,4 +234,124 @@ mod tests {
         assert_eq!(ToolKind::from_shortcut('E'), Some(ToolKind::Erase));
         assert_eq!(ToolKind::from_shortcut('z'), None);
     }
+
+    /// `phase` with its points rounded to micrometers (Push/Pull's comes from a ray).
+    fn rounded(phase: Phase) -> Phase {
+        match phase {
+            Phase::Idle => Phase::Idle,
+            Phase::Active { points } => Phase::Active {
+                points: points.iter().map(|p| (*p * 1e6).round() / 1e6).collect(),
+            },
+        }
+    }
+
+    /// Clicks a fresh `kind` tool at each of `clicks` (points on a selected 2 m square face at
+    /// the origin, seen from the default camera) and returns its phase after each, then after
+    /// the button's release, rounded.
+    fn phases(kind: ToolKind, clicks: &[DVec3]) -> Vec<Phase> {
+        let mut model = Model::new();
+        let p = |x, y| DVec3::new(x, y, 0.0);
+        model
+            .add_polygon(&[p(0.0, 0.0), p(2.0, 0.0), p(2.0, 2.0), p(0.0, 2.0)])
+            .unwrap();
+        let face: FaceId = model.faces().map(|(id, _)| id).next().unwrap();
+        let selection = BTreeSet::from([EntityId::Face(face)]);
+        let camera = Camera::new(DVec2::new(800.0, 600.0));
+        let scenery = Scenery::single(&model);
+        let mut tool = kind.make();
+        let mut out = vec![rounded(tool.phase())];
+        for &point in clicks {
+            let ctx = ToolContext {
+                model: &model,
+                scenery: &scenery,
+                camera: &camera,
+                cursor: camera.project(point).unwrap(),
+                pick_radius: 10.0,
+                drag_threshold: 4.0,
+                shift: false,
+                selection: &selection,
+                material: None,
+                sample: false,
+            };
+            let hit = Inference {
+                point,
+                kind: SnapKind::OnFace,
+                face: Some(face),
+            };
+            tool.handle(ToolEvent::Click(hit), &ctx);
+            out.push(rounded(tool.phase()));
+            tool.handle(ToolEvent::Release(hit), &ctx);
+            out.push(rounded(tool.phase()));
+        }
+        out
+    }
+
+    fn active(points: &[DVec3]) -> Phase {
+        Phase::Active {
+            points: points.to_vec(),
+        }
+    }
+
+    #[test]
+    fn phases_list_the_points_clicked_so_far() {
+        let a = DVec3::new(0.5, 0.5, 0.0);
+        let b = DVec3::new(1.5, 0.5, 0.0);
+        let c = DVec3::new(1.5, 1.5, 0.0);
+        let idle = Phase::Idle;
+        assert_eq!(
+            phases(ToolKind::Line, &[a, b]),
+            vec![
+                idle.clone(),
+                active(&[a]),
+                active(&[a]),
+                active(&[a, b]),
+                active(&[a, b])
+            ]
+        );
+        for kind in [ToolKind::Rectangle, ToolKind::Circle] {
+            assert_eq!(
+                phases(kind, &[a]),
+                vec![idle.clone(), active(&[a]), active(&[a])],
+                "{kind:?}"
+            );
+        }
+        assert_eq!(
+            phases(ToolKind::Arc, &[a, b]),
+            vec![
+                idle.clone(),
+                active(&[a]),
+                active(&[a]),
+                active(&[a, b]),
+                active(&[a, b])
+            ]
+        );
+        for kind in [ToolKind::PushPull, ToolKind::Move, ToolKind::TapeMeasure] {
+            assert_eq!(
+                phases(kind, &[a]),
+                vec![idle.clone(), active(&[a]), active(&[a])],
+                "{kind:?}"
+            );
+        }
+        assert_eq!(
+            phases(ToolKind::Rotate, &[a, b]),
+            vec![
+                idle.clone(),
+                active(&[a]),
+                active(&[a]),
+                active(&[a, b]),
+                active(&[a, b])
+            ]
+        );
+        // A finished operation is idle again.
+        let finished = phases(ToolKind::Rectangle, &[a, c]);
+        assert_eq!(finished.last(), Some(&idle));
+        // The Select tool is active while its button is held; Erase and Paint act at once.
+        assert_eq!(
+            phases(ToolKind::Select, &[a]),
+            vec![idle.clone(), active(&[]), idle.clone()]
+        );
+        for kind in [ToolKind::Erase, ToolKind::Paint] {
+            assert_eq!(phases(kind, &[a]), vec![idle.clone(); 3], "{kind:?}");
+        }
+    }
 }
```

```diff
--- a/crates/tools/src/lib.rs
+++ b/crates/tools/src/lib.rs
@@ -34,4 +34,4 @@ pub use rotate::RotateTool;
 pub use scenery::{Layout, Owner, Part, Placed, Scenery};
 pub use select::SelectTool;
 pub use tape::TapeMeasureTool;
-pub use tool::{Remembered, Tool, ToolContext, ToolEvent, ToolKind, ToolOutput};
+pub use tool::{Phase, Remembered, Tool, ToolContext, ToolEvent, ToolKind, ToolOutput};
```

```diff
--- a/crates/tools/src/line.rs
+++ b/crates/tools/src/line.rs
@@ -5,7 +5,7 @@ use glam::DVec3;
 use kernel::geom::LENGTH_TOL;
 use session::Command;
 
-use crate::tool::{Tool, ToolContext, ToolEvent, ToolOutput};
+use crate::tool::{Phase, Tool, ToolContext, ToolEvent, ToolOutput};
 use crate::vcb::{format_length, parse_length};
 
 #[derive(Clone, Debug, Default)]
@@ -66,6 +66,17 @@ impl Tool for LineTool {
         self.last
     }
 
+    /// The chain's first point and the point the next edge starts from.
+    fn phase(&self) -> Phase {
+        match (self.first, self.last) {
+            (Some(first), Some(last)) if first != last => Phase::Active {
+                points: vec![first, last],
+            },
+            (_, Some(last)) => Phase::Active { points: vec![last] },
+            _ => Phase::Idle,
+        }
+    }
+
     fn handle(&mut self, event: ToolEvent, _ctx: &ToolContext) -> ToolOutput {
         let mut out = ToolOutput::default();
         match event {
```

```diff
--- a/crates/tools/src/rectangle.rs
+++ b/crates/tools/src/rectangle.rs
@@ -6,7 +6,7 @@ use kernel::geom::LENGTH_TOL;
 use session::{Command, rectangle_axes, rectangle_corners};
 
 use crate::inference::Inference;
-use crate::tool::{Tool, ToolContext, ToolEvent, ToolOutput};
+use crate::tool::{Phase, Tool, ToolContext, ToolEvent, ToolOutput};
 use crate::vcb::{format_length, parse_lengths, toward_cursor};
 
 #[derive(Clone, Debug, Default)]
@@ -101,6 +101,15 @@ impl Tool for RectangleTool {
         self.start.map(|(corner, _)| corner)
     }
 
+    fn phase(&self) -> Phase {
+        match self.start {
+            Some((corner, _)) => Phase::Active {
+                points: vec![corner],
+            },
+            None => Phase::Idle,
+        }
+    }
+
     fn handle(&mut self, event: ToolEvent, ctx: &ToolContext) -> ToolOutput {
         let mut out = ToolOutput::default();
         match event {
```

```diff
--- a/crates/tools/src/circle.rs
+++ b/crates/tools/src/circle.rs
@@ -7,7 +7,7 @@ use glam::DVec3;
 use session::{Command, DEFAULT_CIRCLE_SEGMENTS, circle_points};
 
 use crate::rectangle::{on_plane, plane_normal};
-use crate::tool::{Remembered, Tool, ToolContext, ToolEvent, ToolOutput};
+use crate::tool::{Phase, Remembered, Tool, ToolContext, ToolEvent, ToolOutput};
 use crate::vcb::{format_length, meant_as_count, parse_length, parse_segments};
 
 /// Shown for a side count that is not a whole number in range.
@@ -92,6 +92,15 @@ impl Tool for CircleTool {
         self.center.map(|(c, _)| c)
     }
 
+    fn phase(&self) -> Phase {
+        match self.center {
+            Some((center, _)) => Phase::Active {
+                points: vec![center],
+            },
+            None => Phase::Idle,
+        }
+    }
+
     fn handle(&mut self, event: ToolEvent, ctx: &ToolContext) -> ToolOutput {
         let mut out = ToolOutput::default();
         match event {
```

```diff
--- a/crates/tools/src/arc.rs
+++ b/crates/tools/src/arc.rs
@@ -9,7 +9,7 @@ use kernel::geom::LENGTH_TOL;
 use session::{Command, DEFAULT_ARC_SEGMENTS, MAX_COORDINATE, arc_from_bulge, arc_points};
 
 use crate::rectangle::{on_plane, plane_normal};
-use crate::tool::{Remembered, Tool, ToolContext, ToolEvent, ToolOutput};
+use crate::tool::{Phase, Remembered, Tool, ToolContext, ToolEvent, ToolOutput};
 use crate::vcb::{format_length, meant_as_count, parse_length, parse_segments};
 
 /// Shown for a segment count that is not a whole number in range.
@@ -149,6 +149,16 @@ impl Tool for ArcTool {
         }
     }
 
+    /// The start, then the end once clicked.
+    fn phase(&self) -> Phase {
+        match self.start {
+            Some((start, _)) => Phase::Active {
+                points: [Some(start), self.end].into_iter().flatten().collect(),
+            },
+            None => Phase::Idle,
+        }
+    }
+
     fn handle(&mut self, event: ToolEvent, ctx: &ToolContext) -> ToolOutput {
         let mut out = ToolOutput::default();
         match event {
```

```diff
--- a/crates/tools/src/push_pull.rs
+++ b/crates/tools/src/push_pull.rs
@@ -8,7 +8,7 @@ use session::Command;
 
 use crate::inference::{Inference, SnapKind, closest_on_line_to_ray, face_under_in};
 use crate::scenery::Owner;
-use crate::tool::{Tool, ToolContext, ToolEvent, ToolOutput};
+use crate::tool::{Phase, Tool, ToolContext, ToolEvent, ToolOutput};
 use crate::vcb::{format_length, parse_length, toward_cursor};
 
 /// A push/pull in progress.
@@ -104,6 +104,16 @@ impl Tool for PushPullTool {
         None
     }
 
+    /// Where the face was clicked.
+    fn phase(&self) -> Phase {
+        match &self.drag {
+            Some(d) => Phase::Active {
+                points: vec![d.start],
+            },
+            None => Phase::Idle,
+        }
+    }
+
     fn handle(&mut self, event: ToolEvent, ctx: &ToolContext) -> ToolOutput {
         let mut out = ToolOutput::default();
         match event {
```

```diff
--- a/crates/tools/src/move_tool.rs
+++ b/crates/tools/src/move_tool.rs
@@ -9,7 +9,7 @@ use kernel::geom::LENGTH_TOL;
 use session::{Command, MAX_COPIES};
 
 use crate::select::{box_outline, face_outline, guide_outline, pick, whole_curve};
-use crate::tool::{Tool, ToolContext, ToolEvent, ToolOutput};
+use crate::tool::{Phase, Tool, ToolContext, ToolEvent, ToolOutput};
 use crate::vcb::{Array, format_length, parse_array, parse_length};
 
 /// A move in progress.
@@ -136,6 +136,16 @@ impl Tool for MoveTool {
         self.drag.as_ref().map(|d| d.base)
     }
 
+    /// The base point.
+    fn phase(&self) -> Phase {
+        match &self.drag {
+            Some(d) => Phase::Active {
+                points: vec![d.base],
+            },
+            None => Phase::Idle,
+        }
+    }
+
     fn handle(&mut self, event: ToolEvent, ctx: &ToolContext) -> ToolOutput {
         let mut out = ToolOutput::default();
         match event {
```

```diff
--- a/crates/tools/src/rotate.rs
+++ b/crates/tools/src/rotate.rs
@@ -14,7 +14,7 @@ use crate::inference::{Inference, SnapKind};
 use crate::move_tool::outline;
 use crate::rectangle::{on_plane, plane_normal};
 use crate::select::{pick, whole_curve};
-use crate::tool::{Tool, ToolContext, ToolEvent, ToolOutput};
+use crate::tool::{Phase, Tool, ToolContext, ToolEvent, ToolOutput};
 use crate::vcb::{Array, format_angle, parse_angle, parse_array};
 
 /// Angles within this many degrees of a multiple of `STEP` snap to it.
@@ -35,6 +35,8 @@ struct Turn {
     outline: Vec<[DVec3; 2]>,
     /// Unit vector in the rotation's plane the angle is measured from, once clicked.
     reference: Option<DVec3>,
+    /// Where the angle starts: the click that gave `reference`, on the rotation's plane.
+    start: Option<DVec3>,
 }
 
 /// The last rotation made, which a typed angle or array replaces.
@@ -220,6 +222,16 @@ impl Tool for RotateTool {
         self.turn.as_ref().map(|t| t.center)
     }
 
+    /// The center, then where the angle starts (the click on the rotation's plane).
+    fn phase(&self) -> Phase {
+        match &self.turn {
+            Some(t) => Phase::Active {
+                points: [Some(t.center), t.start].into_iter().flatten().collect(),
+            },
+            None => Phase::Idle,
+        }
+    }
+
     fn handle(&mut self, event: ToolEvent, ctx: &ToolContext) -> ToolOutput {
         let mut out = ToolOutput::default();
         match event {
@@ -247,6 +259,7 @@ impl Tool for RotateTool {
                             outline: outline(ctx, &ids),
                             ids,
                             reference: None,
+                            start: None,
                         });
                         self.hover = Some(hit.point);
                     }
@@ -256,7 +269,10 @@ impl Tool for RotateTool {
                         .map(|p| flat(p - turn.center, turn.axis))
                         .filter(|v| v.length() >= LENGTH_TOL);
                     match from {
-                        Some(v) => turn.reference = Some(v.normalize()),
+                        Some(v) => {
+                            turn.reference = Some(v.normalize());
+                            turn.start = Some(turn.center + v);
+                        }
                         None => {
                             out.message = Some("Click away from the center".into());
                         }
```

```diff
--- a/crates/tools/src/tape.rs
+++ b/crates/tools/src/tape.rs
@@ -12,7 +12,7 @@ use kernel::geom::LENGTH_TOL;
 use session::Command;
 
 use crate::inference::{SnapKind, edge_under_in, guide_under_where};
-use crate::tool::{Tool, ToolContext, ToolEvent, ToolOutput};
+use crate::tool::{Phase, Tool, ToolContext, ToolEvent, ToolOutput};
 use crate::vcb::{format_length, parse_length};
 
 /// Where a measurement starts.
@@ -161,6 +161,16 @@ impl Tool for TapeMeasureTool {
         self.start.map(Start::point)
     }
 
+    /// Where the measurement starts.
+    fn phase(&self) -> Phase {
+        match self.start {
+            Some(start) => Phase::Active {
+                points: vec![start.point()],
+            },
+            None => Phase::Idle,
+        }
+    }
+
     fn handle(&mut self, event: ToolEvent, ctx: &ToolContext) -> ToolOutput {
         let mut out = ToolOutput::default();
         match event {
```

```diff
--- a/crates/tools/src/select.rs
+++ b/crates/tools/src/select.rs
@@ -12,7 +12,7 @@ use session::{Command, SelectMode};
 
 use crate::inference::{edge_under_where, face_under_in, guide_under_where};
 use crate::scenery::Owner;
-use crate::tool::{Tool, ToolContext, ToolEvent, ToolOutput};
+use crate::tool::{Phase, Tool, ToolContext, ToolEvent, ToolOutput};
 
 #[derive(Clone, Debug, Default)]
 pub struct SelectTool {
@@ -249,6 +249,14 @@ impl Tool for SelectTool {
         None
     }
 
+    /// Active while the button is held: a click or a box being dragged, both on screen.
+    fn phase(&self) -> Phase {
+        match self.press {
+            Some(_) => Phase::Active { points: Vec::new() },
+            None => Phase::Idle,
+        }
+    }
+
     fn handle(&mut self, event: ToolEvent, ctx: &ToolContext) -> ToolOutput {
         let mut out = ToolOutput::default();
         self.cursor = ctx.cursor;
```

```diff
--- a/crates/tools/src/erase.rs
+++ b/crates/tools/src/erase.rs
@@ -6,7 +6,7 @@ use kernel::EntityId;
 use session::Command;
 
 use crate::select::{box_outline, face_outline, guide_outline, pick};
-use crate::tool::{Tool, ToolContext, ToolEvent, ToolOutput};
+use crate::tool::{Phase, Tool, ToolContext, ToolEvent, ToolOutput};
 
 #[derive(Clone, Debug, Default)]
 pub struct EraseTool {
@@ -55,6 +55,11 @@ impl Tool for EraseTool {
         None
     }
 
+    /// Each click acts at once.
+    fn phase(&self) -> Phase {
+        Phase::Idle
+    }
+
     fn handle(&mut self, event: ToolEvent, ctx: &ToolContext) -> ToolOutput {
         let mut out = ToolOutput::default();
         match event {
```

```diff
--- a/crates/tools/src/paint.rs
+++ b/crates/tools/src/paint.rs
@@ -10,7 +10,7 @@ use session::Command;
 use crate::inference::face_under_in;
 use crate::scenery::Owner;
 use crate::select::{box_outline, face_outline};
-use crate::tool::{Tool, ToolContext, ToolEvent, ToolOutput};
+use crate::tool::{Phase, Tool, ToolContext, ToolEvent, ToolOutput};
 
 #[derive(Clone, Debug, Default)]
 pub struct PaintTool {
@@ -65,6 +65,11 @@ impl Tool for PaintTool {
         None
     }
 
+    /// Each click acts at once.
+    fn phase(&self) -> Phase {
+        Phase::Idle
+    }
+
     fn handle(&mut self, event: ToolEvent, ctx: &ToolContext) -> ToolOutput {
         let mut out = ToolOutput::default();
         match event {
```

- [ ] **Step 4: Run the tests**

Run: `$HOME/.cargo/bin/cargo test -p tools`
Expected: PASS (the phase test included).

- [ ] **Step 5: Check and commit**

Run: `$HOME/.cargo/bin/cargo fmt --all --check && $HOME/.cargo/bin/cargo clippy --workspace --all-targets -- -D warnings && $HOME/.cargo/bin/cargo test --workspace`
Expected: clean; all pass.

```bash
git status && git diff
git add crates/tools/src/tool.rs crates/tools/src/lib.rs crates/tools/src/line.rs crates/tools/src/rectangle.rs crates/tools/src/circle.rs crates/tools/src/arc.rs crates/tools/src/push_pull.rs crates/tools/src/move_tool.rs crates/tools/src/rotate.rs crates/tools/src/tape.rs crates/tools/src/select.rs crates/tools/src/erase.rs crates/tools/src/paint.rs
git commit -m "Tools: each tool reports its phase and the points clicked so far"
```

---

### Task 3: Picking along a ray and what inference snapped to

**Files:**
- Create: `crates/tools/src/probe.rs` (with its tests)
- Modify: `crates/tools/src/scenery.rs` (with a test), `crates/tools/src/inference.rs` (with a test), `crates/tools/src/lib.rs`

**Interfaces:**
- Consumes: `Scenery`, `Index::ray_hits(origin, dir, min_t, max_t) -> Vec<(FaceId, f64)>` (kernel, existing).
- Produces:
  - `Scenery::path(&self, k: usize) -> &[InstanceId]`: the instances from the root down to the one placing part `k` (empty for the root).
  - `tools::Involved = Vec<(usize, EntityId)>` (part index, entity).
  - `tools::infer_detail(scenery: &Scenery, camera: &Camera, cursor: DVec2, guides: &Guides, radius_px: f64) -> (Inference, Involved)`; `infer_in` now returns its `.0`.
  - `tools::Hit { id: EntityId, part: usize, point: DVec3, normal: Option<DVec3>, distance: f64 }`, `tools::Reach<'a> { Screen { camera: &'a Camera, cursor: DVec2, radius_px: f64 }, Around { radius: f64 } }`, `tools::pick(scenery: &Scenery, ray: &Ray, reach: Reach) -> Vec<Hit>` (sorted by distance).
  - Crate-private: `Near` and its fields `pub(crate)`, `Near::in_cone(scenery, cone)`, `Cone` `pub(crate)` with `Cone::around` and new `Cone::along(origin, dir, radius)`.

- [ ] **Step 1: Write the probe module with its tests**

Create `crates/tools/src/probe.rs`:

```rust
//! What lies along a ray: every face it meets, and the edges and guides passing near it, for
//! agents asking what is under a pixel.

use glam::{DVec2, DVec3};
use kernel::{EntityId, Guide};
use session::{Camera, Ray};

use crate::inference::{Cone, Near, closest_on_line_to_ray};
use crate::scenery::Scenery;

/// A face, edge or guide a pick met.
#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    /// The entity, an id in the mesh of its part.
    pub id: EntityId,
    /// The index of its part in the `Scenery` (see `Scenery::path`).
    pub part: usize,
    /// Where the ray meets a face, or the point of an edge or guide nearest the ray, in the
    /// open context's coordinates.
    pub point: DVec3,
    /// A face's unit normal (the direction its front side faces) in the open context's
    /// coordinates; None for edges and guides.
    pub normal: Option<DVec3>,
    /// How far along the ray `point` lies.
    pub distance: f64,
}

/// How near the ray an edge or guide must pass to be met.
#[derive(Clone, Copy, Debug)]
pub enum Reach<'a> {
    /// Within `radius_px` of `cursor` on screen, as `camera` (whose ray through the cursor is
    /// the one picked along) shows it.
    Screen {
        camera: &'a Camera,
        cursor: DVec2,
        radius_px: f64,
    },
    /// Within `radius` (meters) of the ray.
    Around { radius: f64 },
}

/// Everything `ray` (unit direction, in the open context's coordinates) meets in front of its
/// origin, nearest first: every face it crosses, whatever lies in front, and every edge (not
/// soft: soft edges are hidden) and guide passing within `reach`.
pub fn pick(scenery: &Scenery, ray: &Ray, reach: Reach) -> Vec<Hit> {
    let mut hits = Vec::new();
    for (k, part) in scenery.parts().enumerate() {
        if !part.may_meet_ray(ray.origin, ray.dir, f64::INFINITY) {
            continue;
        }
        let faces = part.model.index().ray_hits(
            part.back.transform_point3(ray.origin),
            part.back.transform_vector3(ray.dir),
            0.0,
            f64::INFINITY,
        );
        // Normals go into the open context's coordinates by the inverse transpose.
        let normals = part.back.matrix3.transpose();
        for (f, t) in faces {
            let normal = part
                .model
                .face(f)
                .and_then(|face| (normals * face.normal).try_normalize());
            hits.push(Hit {
                id: EntityId::Face(f),
                part: k,
                point: ray.origin + ray.dir * t,
                normal,
                distance: t,
            });
        }
    }

    let (cone, near_enough): (Cone, Box<dyn Fn(DVec3) -> bool>) = match reach {
        Reach::Screen {
            camera,
            cursor,
            radius_px,
        } => {
            let projector = camera.projector();
            (
                Cone::around(camera, cursor, radius_px),
                Box::new(move |p| {
                    projector
                        .project(p)
                        .is_some_and(|s| s.distance(cursor) <= radius_px)
                }),
            )
        }
        Reach::Around { radius } => {
            let (origin, dir) = (ray.origin, ray.dir);
            (
                Cone::along(origin, dir, radius),
                Box::new(move |p| {
                    let w = p - origin;
                    (w - dir * w.dot(dir)).length() <= radius
                }),
            )
        }
    };
    let mut offer = |id: EntityId, part: usize, point: DVec3| {
        let distance = (point - ray.origin).dot(ray.dir);
        if distance >= 0.0 && near_enough(point) {
            hits.push(Hit {
                id,
                part,
                point,
                normal: None,
                distance,
            });
        }
    };
    for (k, e, a, b) in Near::in_cone(scenery, cone).segments {
        offer(
            EntityId::Edge(e),
            k,
            closest_on_line_to_ray(a, b - a, ray, true),
        );
    }
    for &(k, id, g) in scenery.guides() {
        let point = match g {
            Guide::Point { position } => position,
            Guide::Line { point, direction } => {
                closest_on_line_to_ray(point, direction, ray, false)
            }
        };
        offer(EntityId::Guide(id), k, point);
    }
    hits.sort_by(|a, b| a.distance.total_cmp(&b.distance));
    hits
}

#[cfg(test)]
mod tests {
    use super::*;
    use kernel::{DefinitionKind, InstanceId, MeshId, Model, Scene};

    /// A root holding a unit square at z = 0, another at z = 1 and a guide point at
    /// (0.25, 0.25, 5); and a group holding a unit square at z = 0 from x = 5 to 6.
    fn scene() -> (Scene, InstanceId) {
        let p = |x, y, z| DVec3::new(x, y, z);
        let square = |m: &mut Model, x: f64, z: f64| {
            m.add_polygon(&[
                p(x, 0.0, z),
                p(x + 1.0, 0.0, z),
                p(x + 1.0, 1.0, z),
                p(x, 1.0, z),
            ])
            .unwrap();
        };
        let mut m = Model::new();
        square(&mut m, 5.0, 0.0);
        let mut s = Scene::from_root(m);
        let edges: Vec<_> = s.root().edges().map(|(id, _)| id).collect();
        let faces: Vec<_> = s.root().faces().map(|(id, _)| id).collect();
        let (i, _) = s
            .make_group(MeshId::Root, &edges, &faces, &[], DefinitionKind::Group, "")
            .unwrap();
        s.edit(MeshId::Root, |m| {
            let mut c = m.add_polygon(&[
                p(0.0, 0.0, 0.0),
                p(1.0, 0.0, 0.0),
                p(1.0, 1.0, 0.0),
                p(0.0, 1.0, 0.0),
            ])?;
            c.ops.extend(
                m.add_polygon(&[
                    p(0.0, 0.0, 1.0),
                    p(1.0, 0.0, 1.0),
                    p(1.0, 1.0, 1.0),
                    p(0.0, 1.0, 1.0),
                ])?
                .ops,
            );
            c.ops.extend(
                m.add_guide(Guide::Point {
                    position: p(0.25, 0.25, 5.0),
                })?
                .ops,
            );
            Ok(c)
        })
        .unwrap();
        (s, i)
    }

    fn down_at(x: f64, y: f64) -> Ray {
        Ray {
            origin: DVec3::new(x, y, 10.0),
            dir: DVec3::NEG_Z,
        }
    }

    /// The kind letter of each hit's id ("f", "e", "g") with its distance.
    fn kinds(hits: &[Hit]) -> Vec<(char, f64)> {
        hits.iter()
            .map(|h| (h.id.to_string().chars().next().unwrap(), h.distance))
            .collect()
    }

    #[test]
    fn a_ray_meets_every_face_front_to_back() {
        let (s, _) = scene();
        let v = Scenery::of(&s, &[]).unwrap();
        let hits = pick(&v, &down_at(0.5, 0.5), Reach::Around { radius: 0.0 });
        assert_eq!(kinds(&hits), vec![('f', 9.0), ('f', 10.0)]);
        assert_eq!(hits[0].point, DVec3::new(0.5, 0.5, 1.0));
        assert!(hits.iter().all(|h| v.path(h.part).is_empty()));
        let normal = hits[0].normal.unwrap();
        assert!((normal.z.abs() - 1.0).abs() < 1e-12, "{normal}");
    }

    #[test]
    fn faces_in_groups_carry_their_path_and_placement() {
        let (s, i) = scene();
        let v = Scenery::of(&s, &[]).unwrap();
        let hits = pick(&v, &down_at(5.5, 0.5), Reach::Around { radius: 0.0 });
        assert_eq!(kinds(&hits), vec![('f', 10.0)]);
        assert_eq!(v.path(hits[0].part), &[i]);
        assert_eq!(hits[0].point, DVec3::new(5.5, 0.5, 0.0));
        // Seen from inside the group, the same face is the open context's.
        let inside = Scenery::of(&s, &[i]).unwrap();
        let hits = pick(&inside, &down_at(5.5, 0.5), Reach::Around { radius: 0.0 });
        assert_eq!(kinds(&hits), vec![('f', 10.0)]);
        assert_eq!(hits[0].part, 0);
        assert_eq!(inside.path(0), &[i]);
    }

    #[test]
    fn edges_and_guides_within_reach_are_met_too() {
        let (s, _) = scene();
        let v = Scenery::of(&s, &[]).unwrap();
        let hits = pick(&v, &down_at(0.255, 0.25), Reach::Around { radius: 0.01 });
        assert_eq!(kinds(&hits), vec![('g', 5.0), ('f', 9.0), ('f', 10.0)]);
        let hits = pick(&v, &down_at(0.995, 0.5), Reach::Around { radius: 0.01 });
        let mut found = kinds(&hits);
        found.sort_by(|a, b| a.partial_cmp(b).unwrap());
        assert_eq!(
            found,
            vec![('e', 9.0), ('e', 10.0), ('f', 9.0), ('f', 10.0)]
        );
        // On screen: the edge passes within the radius of the cursor.
        let camera = Camera::new(DVec2::new(800.0, 600.0));
        let edge_mid = DVec3::new(1.0, 0.5, 1.0);
        let cursor = camera.project(edge_mid).unwrap() + DVec2::new(2.0, 0.0);
        let ray = camera.ray(cursor);
        let reach = |radius_px| Reach::Screen {
            camera: &camera,
            cursor,
            radius_px,
        };
        let near = pick(&v, &ray, reach(4.0));
        let edge = near.iter().find(|h| h.id.to_string().starts_with('e'));
        assert!(
            edge.is_some_and(|h| h.point.distance(edge_mid) < 0.05),
            "{near:?}"
        );
        let tight = pick(&v, &ray, reach(0.5));
        assert!(
            tight.iter().all(|h| !h.id.to_string().starts_with('e')),
            "{tight:?}"
        );
    }
}
```

- [ ] **Step 2: Run its tests to see them fail**

Run: `$HOME/.cargo/bin/cargo test -p tools --lib probe`
Expected: FAIL to compile: the module is not declared, and `Cone`, `Near`, `Scenery::path` are not reachable.

- [ ] **Step 3: Paths, detailed inference, and the module**

```diff
--- a/crates/tools/src/scenery.rs
+++ b/crates/tools/src/scenery.rs
@@ -169,6 +169,9 @@ pub struct Layout {
     meshes: Vec<MeshId>,
     /// Every placed mesh, the open context's first, then the others in walk order.
     parts: Vec<Placed>,
+    /// Each part's path: the instances from the root down to the one placing it (empty for
+    /// the root).
+    paths: Vec<Vec<InstanceId>>,
     /// Every guide of every part, placed in the open context's coordinates, with the index of
     /// the part it comes from, in part and id order.
     guides: Vec<(usize, GuideId, Guide)>,
@@ -191,27 +194,34 @@ impl Layout {
             &mut found,
         );
         // The open mesh first, then the others in walk order.
-        found.sort_by_key(|(_, _, _, owner)| *owner != Owner::Open);
+        found.sort_by_key(|f| f.owner != Owner::Open);
         let mut slots: HashMap<MeshId, usize> = HashMap::new();
         let mut meshes = Vec::new();
         let mut models = Vec::new();
+        let mut paths = Vec::new();
         let parts = found
             .into_iter()
-            .map(|(id, model, place, owner)| {
-                let mesh = *slots.entry(id).or_insert_with(|| {
-                    meshes.push(id);
-                    models.push(model);
+            .map(|f| {
+                let mesh = *slots.entry(f.id).or_insert_with(|| {
+                    meshes.push(f.id);
+                    models.push(f.model);
                     meshes.len() - 1
                 });
-                (mesh, place, owner)
+                paths.push(f.path);
+                (mesh, f.place, f.owner)
             })
             .collect();
-        Some(Layout::new(meshes, &models, parts))
+        Some(Layout::new(meshes, &models, parts, paths))
     }
 
     /// A layout of `parts` (each the index of its mesh in `meshes`, its placement and its
-    /// owner); `models` are the meshes themselves.
-    fn new(meshes: Vec<MeshId>, models: &[&Model], parts: Vec<(usize, DAffine3, Owner)>) -> Layout {
+    /// owner) reached by `paths`; `models` are the meshes themselves.
+    fn new(
+        meshes: Vec<MeshId>,
+        models: &[&Model],
+        parts: Vec<(usize, DAffine3, Owner)>,
+        paths: Vec<Vec<InstanceId>>,
+    ) -> Layout {
         let boxes: Vec<Option<[DVec3; 2]>> = models.iter().map(|m| mesh_box(m)).collect();
         let parts: Vec<Placed> = parts
             .into_iter()
@@ -248,6 +258,7 @@ impl Layout {
         Layout {
             meshes,
             parts,
+            paths,
             guides,
             boxes: instances
                 .into_iter()
@@ -276,7 +287,12 @@ impl<'a> Scenery<'a> {
         let parts = vec![(0, DAffine3::IDENTITY, Owner::Open)];
         Scenery {
             models: vec![model],
-            layout: Cow::Owned(Layout::new(vec![MeshId::Root], &[model], parts)),
+            layout: Cow::Owned(Layout::new(
+                vec![MeshId::Root],
+                &[model],
+                parts,
+                vec![Vec::new()],
+            )),
         }
     }
 
@@ -327,6 +343,12 @@ impl<'a> Scenery<'a> {
         })
     }
 
+    /// The instances from the root down to the one placing the `k`th part (empty for the
+    /// root's own mesh), whatever the open context.
+    pub fn path(&self, k: usize) -> &[InstanceId] {
+        &self.layout.paths[k]
+    }
+
     /// Every guide of every part, placed in the open context's coordinates, with the index of
     /// the part it comes from, in part and id order.
     pub fn guides(&self) -> &[(usize, GuideId, Guide)] {
@@ -367,6 +389,17 @@ impl<'a> Scenery<'a> {
     }
 }
 
+/// A mesh found placed in the scene.
+struct Found<'a> {
+    id: MeshId,
+    model: &'a Model,
+    /// Into the open context's coordinates.
+    place: DAffine3,
+    owner: Owner,
+    /// The instances from the root down to the one placing it.
+    path: Vec<InstanceId>,
+}
+
 /// Adds the mesh `id`, placed by `place` (into the open context's coordinates) and reached by
 /// `path`, and everything it places, each with its owner.
 fn collect<'a>(
@@ -375,7 +408,7 @@ fn collect<'a>(
     place: DAffine3,
     context: &[InstanceId],
     path: &mut Vec<InstanceId>,
-    found: &mut Vec<(MeshId, &'a Model, DAffine3, Owner)>,
+    found: &mut Vec<Found<'a>>,
 ) {
     let Some(model) = scene.mesh(id) else { return };
     let owner = if path.as_slice() == context {
@@ -385,7 +418,13 @@ fn collect<'a>(
     } else {
         Owner::Outside
     };
-    found.push((id, model, place, owner));
+    found.push(Found {
+        id,
+        model,
+        place,
+        owner,
+        path: path.clone(),
+    });
     for (iid, instance) in model.instances() {
         path.push(iid);
         collect(
@@ -442,6 +481,22 @@ mod tests {
         );
     }
 
+    #[test]
+    fn parts_know_their_path_from_the_root() {
+        let (s, i) = scene();
+        for context in [vec![], vec![i]] {
+            let v = Scenery::of(&s, &context).unwrap();
+            let paths: Vec<&[InstanceId]> = (0..v.len()).map(|k| v.path(k)).collect();
+            if context.is_empty() {
+                assert_eq!(paths, vec![&[][..], &[i][..]]);
+            } else {
+                assert_eq!(paths, vec![&[i][..], &[][..]]);
+            }
+        }
+        let m = Model::new();
+        assert!(Scenery::single(&m).path(0).is_empty());
+    }
+
     #[test]
     fn inside_an_instance_the_rest_is_outside_and_shifted() {
         let (s, i) = scene();
```

```diff
--- a/crates/tools/src/inference.rs
+++ b/crates/tools/src/inference.rs
@@ -3,7 +3,7 @@
 use std::cell::Cell;
 
 use glam::{DVec2, DVec3};
-use kernel::{EdgeId, FaceId, Guide, GuideId, Model, Shape, VertexId};
+use kernel::{EdgeId, EntityId, FaceId, Guide, GuideId, Model, Shape, VertexId};
 use session::{Camera, Ray};
 
 use crate::scenery::{Owner, Part, Scenery, Stretch};
@@ -127,26 +127,45 @@ pub fn infer_in(
     guides: &Guides,
     radius_px: f64,
 ) -> Inference {
+    infer_detail(scenery, camera, cursor, guides, radius_px).0
+}
+
+/// Entities a snap involves, each with the index of its part in the `Scenery`.
+pub type Involved = Vec<(usize, EntityId)>;
+
+/// [`infer_in`], also giving the entities the point snapped to: the vertex of an endpoint;
+/// the edge of a midpoint or of a point on an edge; the guide of a guide point or of a point on
+/// a guide line; the two edges or guide lines that cross at an intersection, the one the
+/// point lies on first; the face of a point on a face. None for the origin, a direction from
+/// the anchor, the ground, a free point or a locked direction.
+pub fn infer_detail(
+    scenery: &Scenery,
+    camera: &Camera,
+    cursor: DVec2,
+    guides: &Guides,
+    radius_px: f64,
+) -> (Inference, Involved) {
     let ray = camera.ray(cursor);
-    let face_hit = nearest_face_hit(scenery, &ray);
-    let face = face_hit.and_then(|(owner, f, _)| (owner == Owner::Open).then_some(f));
+    let face_hit = nearest_face_part(scenery, &ray);
+    let face = face_hit.and_then(|(k, f, _)| (scenery.part(k).owner == Owner::Open).then_some(f));
     let projector = camera.projector();
     let screen_dist = |p: DVec3| projector.project(p).map(|s| s.distance(cursor));
     let snapped = snap_point(scenery, camera, cursor, radius_px);
 
     if let (Some(anchor), Some(lock)) = (guides.anchor, guides.lock) {
         let point = match snapped {
-            Some((p, _)) => anchor + lock.dir * (p - anchor).dot(lock.dir),
+            Some((p, _, _)) => anchor + lock.dir * (p - anchor).dot(lock.dir),
             None => closest_on_line_to_ray(anchor, lock.dir, &ray, false),
         };
-        return Inference {
+        let hit = Inference {
             point,
             kind: lock.kind,
             face,
         };
+        return (hit, Vec::new());
     }
-    if let Some((point, kind)) = snapped {
-        return Inference { point, kind, face };
+    if let Some((point, kind, involved)) = snapped {
+        return (Inference { point, kind, face }, involved);
     }
 
     // Along a direction from the anchor. Direction lines are guides, so faces do not hide them.
@@ -169,20 +188,21 @@ pub fn infer_in(
             }
         }
         if let Some((_, hit)) = along {
-            return hit;
+            return (hit, Vec::new());
         }
     }
 
     // On a face (nearest along the ray).
-    if let Some((_, _, point)) = face_hit {
-        return Inference {
+    if let Some((k, f, point)) = face_hit {
+        let hit = Inference {
             point,
             kind: SnapKind::OnFace,
             face,
         };
+        return (hit, vec![(k, EntityId::Face(f))]);
     }
 
-    match camera.ground_at(cursor) {
+    let hit = match camera.ground_at(cursor) {
         Some(point) => Inference {
             point,
             kind: SnapKind::Ground,
@@ -193,18 +213,20 @@ pub fn infer_in(
             kind: SnapKind::Free,
             face,
         },
-    }
+    };
+    (hit, Vec::new())
 }
 
-/// The visible point snap under the cursor, if any: endpoint, midpoint, origin or guide point
-/// (nearest on screen), else the nearest apparent intersection of two edges or guide lines
-/// both visible there, else the nearest point on an edge, else on a guide line.
+/// The visible point snap under the cursor, if any, with the entities it involves (see
+/// `infer_detail`): endpoint, midpoint, origin or guide point (nearest on screen), else the
+/// nearest apparent intersection of two edges or guide lines both visible there, else the
+/// nearest point on an edge, else on a guide line.
 fn snap_point(
     scenery: &Scenery,
     camera: &Camera,
     cursor: DVec2,
     radius_px: f64,
-) -> Option<(DVec3, SnapKind)> {
+) -> Option<(DVec3, SnapKind, Involved)> {
     let ray = camera.ray(cursor);
     let projector = camera.projector();
     // How far from the cursor a point is on screen, if within the radius.
@@ -217,37 +239,46 @@ fn snap_point(
     let sight = Sight::new(scenery, camera);
     let seen = |p: &DVec3| sight.sees(*p);
     let near = Near::around(scenery, camera, cursor, radius_px);
-    let mut points: Vec<(f64, (DVec3, SnapKind))> = Vec::new();
-    let mut offer = |p: DVec3, kind: SnapKind| {
+    let mut points: Vec<(f64, (DVec3, SnapKind, Involved))> = Vec::new();
+    let mut offer = |p: DVec3, kind: SnapKind, involved: Involved| {
         if let Some(d) = near_enough(p) {
-            points.push((d, (p, kind)));
+            points.push((d, (p, kind, involved)));
         }
     };
-    for &(_, _, p) in &near.points {
-        offer(p, SnapKind::Endpoint);
-    }
-    for &(_, _, a, b) in &near.segments {
-        offer((a + b) * 0.5, SnapKind::Midpoint);
+    for &(k, v, p) in &near.points {
+        offer(p, SnapKind::Endpoint, vec![(k, EntityId::Vertex(v))]);
+    }
+    for &(k, e, a, b) in &near.segments {
+        offer(
+            (a + b) * 0.5,
+            SnapKind::Midpoint,
+            vec![(k, EntityId::Edge(e))],
+        );
     }
-    offer(DVec3::ZERO, SnapKind::Origin);
+    offer(DVec3::ZERO, SnapKind::Origin, Vec::new());
     let guides = scenery.guides();
-    for &(_, _, g) in guides {
+    for &(k, id, g) in guides {
         if let Guide::Point { position } = g {
-            offer(position, SnapKind::GuidePoint);
+            offer(
+                position,
+                SnapKind::GuidePoint,
+                vec![(k, EntityId::Guide(id))],
+            );
         }
     }
-    if let Some((_, (p, kind))) = nearest_visible(points, |(p, _)| seen(p)) {
-        return Some((p, kind));
+    if let Some((_, snap)) = nearest_visible(points, |(p, _, _)| seen(p)) {
+        return Some(snap);
     }
 
-    // Guide lines passing near the cursor, as the part of them in view.
-    let guide_lines: Vec<(DVec3, DVec3)> = guides
+    // Guide lines passing near the cursor, as the part of them in view, each with its part
+    // and id.
+    let guide_lines: Vec<(usize, GuideId, DVec3, DVec3)> = guides
         .iter()
-        .filter_map(|&(_, _, g)| match g {
-            Guide::Line { point, direction } => Some((point, direction)),
+        .filter_map(|&(k, id, g)| match g {
+            Guide::Line { point, direction } => Some((k, id, point, direction)),
             Guide::Point { .. } => None,
         })
-        .filter(|&(point, dir)| {
+        .filter(|&(_, _, point, dir)| {
             let p = closest_on_line_to_ray(point, dir, &ray, false);
             projector
                 .project(p)
@@ -260,16 +291,15 @@ fn snap_point(
     let lines: Vec<ScreenLine> = near
         .segments
         .iter()
-        .map(|&(_, _, a, b)| (a, b))
-        .chain(
-            guide_lines
-                .iter()
-                .filter_map(|&(point, dir)| camera.line_span(point, dir))
-                .map(|[a, b]| (a, b)),
-        )
-        .filter_map(|(a, b)| {
+        .map(|&(k, e, a, b)| ((k, EntityId::Edge(e)), (a, b)))
+        .chain(guide_lines.iter().filter_map(|&(k, id, point, dir)| {
+            let [a, b] = camera.line_span(point, dir)?;
+            Some(((k, EntityId::Guide(id)), (a, b)))
+        }))
+        .filter_map(|(who, (a, b))| {
             let (sa, sb) = (projector.project(a)?, projector.project(b)?);
             (distance_to_segment(cursor, sa, sb) <= 2.0 * radius_px).then_some(ScreenLine {
+                who,
                 ends: (a, b),
                 screen: (sa, sb),
             })
@@ -278,37 +308,45 @@ fn snap_point(
     let mut crossings = Vec::new();
     for (i, ab) in lines.iter().enumerate() {
         for cd in &lines[i + 1..] {
-            if let Some((p, far)) = apparent_crossing(camera, ab, cd)
+            if let Some((p, far, ab_nearer)) = apparent_crossing(camera, ab, cd)
                 && let Some(dist) = near_enough(p)
             {
-                crossings.push((dist, (p, far)));
+                let involved = if ab_nearer {
+                    vec![ab.who, cd.who]
+                } else {
+                    vec![cd.who, ab.who]
+                };
+                crossings.push((dist, (p, far, involved)));
             }
         }
     }
-    if let Some((_, (p, _))) = nearest_visible(crossings, |(p, far)| seen(far) && seen(p)) {
-        return Some((p, SnapKind::Intersection));
+    if let Some((_, (p, _, involved))) =
+        nearest_visible(crossings, |(p, far, _)| seen(far) && seen(p))
+    {
+        return Some((p, SnapKind::Intersection, involved));
     }
 
     let on_edge = near
         .segments
         .iter()
-        .filter_map(|&(_, _, a, b)| {
+        .filter_map(|&(k, e, a, b)| {
             let p = closest_on_line_to_ray(a, b - a, &ray, true);
-            near_enough(p).map(|d| (d, p))
+            near_enough(p).map(|d| (d, (p, (k, EntityId::Edge(e)))))
         })
         .collect();
-    if let Some((_, p)) = nearest_visible(on_edge, seen) {
-        return Some((p, SnapKind::OnEdge));
+    if let Some((_, (p, who))) = nearest_visible(on_edge, |(p, _)| seen(p)) {
+        return Some((p, SnapKind::OnEdge, vec![who]));
     }
 
     let on_guide = guide_lines
         .iter()
-        .filter_map(|&(point, dir)| {
+        .filter_map(|&(k, id, point, dir)| {
             let p = closest_on_line_to_ray(point, dir, &ray, false);
-            near_enough(p).map(|d| (d, p))
+            near_enough(p).map(|d| (d, (p, (k, EntityId::Guide(id)))))
         })
         .collect();
-    nearest_visible(on_guide, seen).map(|(_, p)| (p, SnapKind::OnGuide))
+    nearest_visible(on_guide, |(p, _)| seen(p))
+        .map(|(_, (p, who))| (p, SnapKind::OnGuide, vec![who]))
 }
 
 /// The nearest of `candidates` (each with its distance from the cursor on screen) that `seen`
@@ -359,8 +397,9 @@ pub fn guide_under_where(
         .map(|(_, (owner, id, g, _))| (owner, id, g))
 }
 
-/// A segment and where its ends are on screen.
+/// A segment, where its ends are on screen, and the edge or guide line it is (with its part).
 struct ScreenLine {
+    who: (usize, EntityId),
     ends: (DVec3, DVec3),
     screen: (DVec2, DVec2),
 }
@@ -378,9 +417,14 @@ fn distance_to_segment(p: DVec2, a: DVec2, b: DVec2) -> f64 {
 }
 
 /// Where segments a-b and c-d cross on screen away from their ends: the points under the
-/// crossing on the nearer and the farther segment, nearer first. Edges that meet in space share
-/// a vertex (the kernel splits crossing edges), so their crossing is an endpoint, not this.
-fn apparent_crossing(camera: &Camera, ab: &ScreenLine, cd: &ScreenLine) -> Option<(DVec3, DVec3)> {
+/// crossing on the nearer and the farther segment, nearer first, and whether a-b is the nearer.
+/// Edges that meet in space share a vertex (the kernel splits crossing edges), so their
+/// crossing is an endpoint, not this.
+fn apparent_crossing(
+    camera: &Camera,
+    ab: &ScreenLine,
+    cd: &ScreenLine,
+) -> Option<(DVec3, DVec3, bool)> {
     let ((a, b), (sa, sb)) = (ab.ends, ab.screen);
     let ((c, d), (sc, sd)) = (cd.ends, cd.screen);
     let (r, q) = (sb - sa, sd - sc);
@@ -399,9 +443,9 @@ fn apparent_crossing(camera: &Camera, ab: &ScreenLine, cd: &ScreenLine) -> Optio
     let on_cd = closest_on_line_to_ray(c, d - c, &ray, true);
     Some(
         if on_ab.distance(ray.origin) <= on_cd.distance(ray.origin) {
-            (on_ab, on_cd)
+            (on_ab, on_cd, true)
         } else {
-            (on_cd, on_ab)
+            (on_cd, on_ab, false)
         },
     )
 }
@@ -492,26 +536,29 @@ pub fn edge_under_where(
         .map(|(_, (owner, id, a, b, _))| (owner, id, a, b))
 }
 
-/// The vertices and edges that may lie within `radius_px` of the cursor on screen, with the
-/// index of the part each comes from and positions in the open context's coordinates, in
-/// part and id order so that ties between equally near candidates always go the same way.
-/// Soft edges are hidden, so they are left out: nothing snaps to them or picks them (their
-/// ends are vertices of the edges around them and still snap).
-struct Near {
-    points: Vec<(usize, VertexId, DVec3)>,
-    segments: Vec<(usize, EdgeId, DVec3, DVec3)>,
+/// The vertices and edges that may lie within `radius_px` of the cursor on screen (or in a
+/// cone), with the index of the part each comes from and positions in the open context's
+/// coordinates, in part and id order so that ties between equally near candidates always go
+/// the same way. Soft edges are hidden, so they are left out: nothing snaps to them or picks
+/// them (their ends are vertices of the edges around them and still snap).
+pub(crate) struct Near {
+    pub(crate) points: Vec<(usize, VertexId, DVec3)>,
+    pub(crate) segments: Vec<(usize, EdgeId, DVec3, DVec3)>,
 }
 
 impl Near {
     fn around(scenery: &Scenery, camera: &Camera, cursor: DVec2, radius_px: f64) -> Near {
-        let cone = Cone::around(camera, cursor, radius_px);
+        Near::in_cone(scenery, Cone::around(camera, cursor, radius_px))
+    }
+
+    /// Those that may lie in `cone`, in the open context's coordinates.
+    pub(crate) fn in_cone(scenery: &Scenery, cone: Cone) -> Near {
         let mut near = Near {
             points: Vec::new(),
             segments: Vec::new(),
         };
         for (k, part) in scenery.parts().enumerate() {
-            // Whatever lies near the cursor on screen lies in the cone, so a part whose box
-            // misses it has nothing to offer.
+            // A part whose box misses the cone has nothing in it.
             if !part.may_meet_cone(cone.origin, cone.dir, cone.r0, cone.slope) {
                 continue;
             }
@@ -543,7 +590,7 @@ impl Near {
 /// from 0, and `r0` behind the origin): what `Index::near_ray` and `Placed::may_meet_cone`
 /// search.
 #[derive(Clone, Copy, Debug, PartialEq)]
-struct Cone {
+pub(crate) struct Cone {
     origin: DVec3,
     dir: DVec3,
     r0: f64,
@@ -569,7 +616,7 @@ impl Cone {
     /// cone (its rim, an ellipse or other conic in perspective and a circle in a parallel
     /// view, holds no straight piece between two corners), so rounding in where a point lands
     /// on screen cannot carry a candidate out of it.
-    fn around(camera: &Camera, cursor: DVec2, radius_px: f64) -> Cone {
+    pub(crate) fn around(camera: &Camera, cursor: DVec2, radius_px: f64) -> Cone {
         let ray = camera.ray(cursor);
         let (mut r0, mut slope) = (0.0_f64, 0.0_f64);
         for (x, y) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
@@ -591,6 +638,17 @@ impl Cone {
         }
     }
 
+    /// The points within `radius` of the ray `origin + t * dir` (unit `dir`), and `radius`
+    /// behind its origin.
+    pub(crate) fn along(origin: DVec3, dir: DVec3, radius: f64) -> Cone {
+        Cone {
+            origin,
+            dir,
+            r0: radius,
+            slope: 0.0,
+        }
+    }
+
     /// A cone holding everything.
     fn whole(origin: DVec3, dir: DVec3) -> Cone {
         Cone {
@@ -715,8 +773,13 @@ pub(crate) fn closest_on_line_to_ray(origin: DVec3, dir: DVec3, ray: &Ray, segme
 
 /// The nearest face the ray meets in front of its origin: whose it is, the face, and where.
 fn nearest_face_hit(scenery: &Scenery, ray: &Ray) -> Option<(Owner, FaceId, DVec3)> {
-    let mut best: Option<(f64, Owner, FaceId)> = None;
-    for part in scenery.parts() {
+    nearest_face_part(scenery, ray).map(|(k, f, p)| (scenery.part(k).owner, f, p))
+}
+
+/// [`nearest_face_hit`] giving the index of the face's part instead of its owner.
+fn nearest_face_part(scenery: &Scenery, ray: &Ray) -> Option<(usize, FaceId, DVec3)> {
+    let mut best: Option<(f64, usize, FaceId)> = None;
+    for (k, part) in scenery.parts().enumerate() {
         let reach = best.map_or(f64::INFINITY, |(t, _, _)| t);
         if !part.may_meet_ray(ray.origin, ray.dir, reach) {
             continue;
@@ -728,10 +791,10 @@ fn nearest_face_hit(scenery: &Scenery, ray: &Ray) -> Option<(Owner, FaceId, DVec
             reach,
         ) && best.is_none_or(|(bt, _, _)| t < bt)
         {
-            best = Some((t, part.owner, f));
+            best = Some((t, k, f));
         }
     }
-    best.map(|(t, owner, f)| (owner, f, ray.origin + ray.dir * t))
+    best.map(|(t, k, f)| (k, f, ray.origin + ray.dir * t))
 }
 
 #[cfg(test)]
@@ -757,6 +820,59 @@ mod tests {
         }
     }
 
+    #[test]
+    fn detailed_inference_names_what_was_snapped_to() {
+        let (c, mut m) = (cam(), Model::new());
+        square(&mut m, 0.0);
+        m.add_guide(Guide::Line {
+            point: DVec3::new(-5.0, 0.5, 0.0),
+            direction: DVec3::X,
+        })
+        .unwrap();
+        let v = Scenery::single(&m);
+        let none = Guides::default();
+        let snap = |p: DVec3| infer_detail(&v, &c, px(&c, p), &none, R);
+        let vertex = |p: DVec3| m.vertices().find(|(_, x)| x.pos == p).unwrap().0;
+        let edge = |a: DVec3, b: DVec3| {
+            m.edges()
+                .find(|(_, e)| {
+                    let ends = e.v.map(|id| m.vertex(id).unwrap().pos);
+                    ends == [a, b] || ends == [b, a]
+                })
+                .unwrap()
+                .0
+        };
+        let (o, x) = (DVec3::ZERO, DVec3::new(2.0, 0.0, 0.0));
+        let (hit, involved) = snap(x);
+        assert_eq!(hit.kind, SnapKind::Endpoint);
+        assert_eq!(involved, vec![(0, EntityId::Vertex(vertex(x)))]);
+        let (hit, involved) = snap(DVec3::new(1.0, 0.0, 0.0));
+        assert_eq!(hit.kind, SnapKind::Midpoint);
+        assert_eq!(involved, vec![(0, EntityId::Edge(edge(o, x)))]);
+        let (hit, involved) = snap(DVec3::new(0.4, 0.0, 0.0));
+        assert_eq!(hit.kind, SnapKind::OnEdge);
+        assert_eq!(involved, vec![(0, EntityId::Edge(edge(o, x)))]);
+        let guide = m.guides().next().unwrap().0;
+        // The edge and the guide line cross in space here, so either may come first.
+        let (hit, mut involved) = snap(DVec3::new(2.0, 0.5, 0.0));
+        assert_eq!(hit.kind, SnapKind::Intersection);
+        involved.sort();
+        let side = edge(x, DVec3::new(2.0, 2.0, 0.0));
+        assert_eq!(
+            involved,
+            vec![(0, EntityId::Edge(side)), (0, EntityId::Guide(guide))]
+        );
+        let (hit, involved) = snap(DVec3::new(-3.0, 0.5, 0.0));
+        assert_eq!(hit.kind, SnapKind::OnGuide);
+        assert_eq!(involved, vec![(0, EntityId::Guide(guide))]);
+        let face = m.faces().next().unwrap().0;
+        let (hit, involved) = snap(DVec3::new(1.3, 1.4, 0.0));
+        assert_eq!(hit.kind, SnapKind::OnFace);
+        assert_eq!(involved, vec![(0, EntityId::Face(face))]);
+        let (hit, involved) = snap(DVec3::new(9.0, 9.0, 0.0));
+        assert_eq!((hit.kind, involved), (SnapKind::Ground, vec![]));
+    }
+
     #[test]
     fn snaps_to_a_nearby_endpoint_exactly() {
         let (c, mut m) = (cam(), Model::new());
```

```diff
--- a/crates/tools/src/lib.rs
+++ b/crates/tools/src/lib.rs
@@ -9,6 +9,7 @@ pub mod line;
 pub mod move_tool;
 pub mod overlay;
 pub mod paint;
+pub mod probe;
 pub mod push_pull;
 pub mod rectangle;
 pub mod rotate;
@@ -22,12 +23,13 @@ pub use arc::ArcTool;
 pub use circle::CircleTool;
 pub use erase::EraseTool;
 pub use inference::{
-    Axis, Guides, Inference, Lock, SnapKind, edge_under, edge_under_in, face_under, face_under_in,
-    guide_under_where, infer, infer_in, infer_with,
+    Axis, Guides, Inference, Involved, Lock, SnapKind, edge_under, edge_under_in, face_under,
+    face_under_in, guide_under_where, infer, infer_detail, infer_in, infer_with,
 };
 pub use line::LineTool;
 pub use move_tool::MoveTool;
 pub use paint::PaintTool;
+pub use probe::{Hit, Reach, pick};
 pub use push_pull::PushPullTool;
 pub use rectangle::RectangleTool;
 pub use rotate::RotateTool;
```

- [ ] **Step 4: Run the tests**

Run: `$HOME/.cargo/bin/cargo test -p tools`
Expected: PASS: the three probe tests, `parts_know_their_path_from_the_root`, `detailed_inference_names_what_was_snapped_to`, and the inference tests that compare against the code before (unchanged results).

- [ ] **Step 5: Check and commit**

Run: `$HOME/.cargo/bin/cargo fmt --all --check && $HOME/.cargo/bin/cargo clippy --workspace --all-targets -- -D warnings && $HOME/.cargo/bin/cargo test --workspace`
Expected: clean; all pass.

```bash
git status && git diff
git add crates/tools/src/probe.rs crates/tools/src/scenery.rs crates/tools/src/inference.rs crates/tools/src/lib.rs
git commit -m "Tools: pick along a ray; inference names what it snapped to; part paths"
```

---

### Task 4: The `pick` and `infer` queries

**Files:**
- Create: `crates/api/src/probe.rs`, `crates/api/tests/probe.rs`
- Modify: `crates/api/Cargo.toml` (and `Cargo.lock`, updated by cargo), `crates/api/src/dispatch.rs`, `crates/api/src/lib.rs`, `crates/session/src/document.rs`, `crates/session/src/lib.rs`
- Docs: `docs/agent-api.md`

**Interfaces:**
- Consumes: `tools::{pick, Reach, Hit, infer_detail, Guides, Scenery, SnapKind, Axis}` (Task 3); `Document::camera_for`.
- Produces:
  - `session::check_coordinates(&[[f64; 3]]) -> Result<(), CommandError>` and `session::unit_vector(DVec3) -> Option<DVec3>`, now public.
  - `api::View { eye, target, fov_y_deg, up, projection }` with `View::camera(&self, &Document) -> Result<Camera, CommandError>` (also used by `render_view`).
  - `api::{PickQuery, InferQuery, RaySpec, PickHit, Inferred, Placed, Snap}`; `probe::pick_query(&Document, &PickQuery) -> Result<Vec<PickHit>, CommandError>`, `probe::infer_query(&Document, &InferQuery) -> Result<Inferred, CommandError>`; constants `PICK_RADIUS` 4, `INFER_RADIUS` 8, `MAX_RADIUS` 1000.
  - `api::EditorQuery` (`#[serde(tag = "type")]`): `Pick(PickQuery)`, `Infer(InferQuery)` (Task 5 adds `Editor`); `dispatch::type_of(&Value) -> Option<&str>`.
  - Query results: `{"type": "pick", "hits": [PickHit]}`, `{"type": "infer", ...Inferred}`.

- [ ] **Step 1: Write the failing tests**

Create `crates/api/tests/probe.rs`:

```rust
//! The `pick` and `infer` queries over JSON-RPC.

use api::{HeadlessHost, codes, handle};
use serde_json::{Value, json};

fn call(host: &mut HeadlessHost, method: &str, params: Value) -> Value {
    let line = json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).to_string();
    serde_json::from_str(&handle(host, &line).expect("a reply")).unwrap()
}

fn query(host: &mut HeadlessHost, params: Value) -> Value {
    let reply = call(host, "query", params);
    assert!(reply["error"].is_null(), "{reply}");
    reply["result"].clone()
}

/// A 2 x 2 x 1 m box with its bottom corner at the origin.
fn boxed() -> HeadlessHost {
    let mut h = HeadlessHost::new();
    let r = call(
        &mut h,
        "execute",
        json!({"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [2, 2, 0]}),
    );
    let face = r["result"]["changes"]["created"]
        .as_array()
        .unwrap()
        .iter()
        .find(|id| id.as_str().unwrap().starts_with('f'))
        .unwrap()
        .clone();
    call(
        &mut h,
        "execute",
        json!({"type": "push_pull", "face": face, "distance": 1}),
    );
    h
}

/// Looking straight down at the box's middle from 10 m up, 100 x 100 pixels.
fn above() -> Value {
    json!({"eye": [1, 1, 10], "target": [1, 1, 0]})
}

#[test]
fn a_pixel_picks_every_face_front_to_back() {
    let mut h = boxed();
    let r = query(
        &mut h,
        json!({"type": "pick", "pixel": [50, 50], "width": 100, "height": 100, "camera": above()}),
    );
    assert_eq!(r["type"], "pick");
    let hits = r["hits"].as_array().unwrap();
    assert_eq!(hits.len(), 2, "{r}");
    assert_eq!(hits[0]["distance"], 9.0);
    assert_eq!(hits[0]["point"], json!([1.0, 1.0, 1.0]));
    assert_eq!(hits[0]["normal"], json!([0.0, 0.0, 1.0]));
    assert_eq!(hits[0]["side"], "front");
    assert_eq!(hits[0]["path"], json!([]));
    // The bottom faces down: the ray meets its back.
    assert_eq!(hits[1]["distance"], 10.0);
    assert_eq!(hits[1]["side"], "back");
    // The document camera did not move.
    assert_ne!(
        query(&mut h, json!({"type": "camera"}))["eye"],
        json!([1.0, 1.0, 10.0])
    );
}

#[test]
fn a_ray_picks_faces_and_with_a_radius_edges() {
    let mut h = boxed();
    let ray = json!({"origin": [1.99, 1, 10], "direction": [0, 0, -3]});
    let faces = query(&mut h, json!({"type": "pick", "ray": ray}));
    let ids = |r: &Value| -> Vec<String> {
        r["hits"]
            .as_array()
            .unwrap()
            .iter()
            .map(|h| h["id"].as_str().unwrap()[..1].to_string())
            .collect()
    };
    assert_eq!(ids(&faces), vec!["f", "f"]);
    let near = query(&mut h, json!({"type": "pick", "ray": ray, "radius": 0.02}));
    let mut kinds = ids(&near);
    kinds.sort();
    // The top and bottom edges along x = 2, and the side face x = 2 the ray runs beside
    // is not crossed.
    assert_eq!(kinds, vec!["e", "e", "f", "f"], "{near}");
}

#[test]
fn hits_inside_a_group_carry_the_path_from_the_root() {
    let mut h = boxed();
    let all = query(
        &mut h,
        json!({"type": "entities", "kinds": ["edge", "face"]}),
    );
    let ids: Vec<Value> = ["edges", "faces"]
        .iter()
        .flat_map(|k| all[k].as_array().unwrap().clone())
        .map(|e| e["id"].clone())
        .collect();
    let r = call(&mut h, "execute", json!({"type": "make_group", "ids": ids}));
    let group = r["result"]["changes"]["created"]
        .as_array()
        .unwrap()
        .iter()
        .find(|id| id.as_str().unwrap().starts_with('i'))
        .unwrap()
        .clone();
    let ray = json!({"origin": [1, 1, 10], "direction": [0, 0, -1]});
    let r = query(&mut h, json!({"type": "pick", "ray": ray}));
    assert_eq!(r["hits"][0]["path"], json!([group]));
    // Inside the group, the same face is the open context's, with the same path.
    call(
        &mut h,
        "execute",
        json!({"type": "open_instance", "id": group}),
    );
    let inside = query(&mut h, json!({"type": "pick", "ray": ray}));
    assert_eq!(inside["hits"][0]["id"], r["hits"][0]["id"]);
    assert_eq!(inside["hits"][0]["path"], json!([group]));
}

#[test]
fn infer_snaps_like_a_tool_click() {
    let mut h = boxed();
    let r = query(
        &mut h,
        json!({"type": "infer", "point": [1.98, 2, 1], "width": 100, "height": 100, "camera": above()}),
    );
    assert_eq!(r["type"], "infer");
    assert_eq!(r["kind"], "endpoint");
    assert_eq!(r["point"], json!([2.0, 2.0, 1.0]));
    let entities = r["entities"].as_array().unwrap();
    assert_eq!(entities.len(), 1);
    assert!(entities[0]["id"].as_str().unwrap().starts_with('v'));
    assert_eq!(entities[0]["path"], json!([]));

    let r = query(&mut h, json!({"type": "infer", "point": [1.3, 1.4, 1]}));
    assert_eq!(r["kind"], "on_face");
    assert!(r["entities"][0]["id"].as_str().unwrap().starts_with('f'));

    // From a point, as a tool's second click: along an axis.
    let r = query(
        &mut h,
        json!({"type": "infer", "point": [5, 3, 0], "from": [5, 0, 0]}),
    );
    assert_eq!(
        (r["kind"].clone(), r["axis"].clone()),
        (json!("axis"), json!("y"))
    );
    assert_eq!(r["entities"], json!([]));
}

#[test]
fn pick_and_infer_reject_what_they_cannot_use() {
    let mut h = boxed();
    let bad = [
        json!({"type": "pick"}),
        json!({"type": "pick", "pixel": [1, 1], "ray": {"origin": [0, 0, 1], "direction": [0, 0, -1]}}),
        json!({"type": "pick", "pixel": [5000, 1]}),
        json!({"type": "pick", "pixel": [1, 1], "width": 10}),
        json!({"type": "pick", "ray": {"origin": [0, 0, 1], "direction": [0, 0, 0]}}),
        json!({"type": "pick", "ray": {"origin": [0, 0, 1], "direction": [0, 0, -1]}, "width": 10, "height": 10}),
        json!({"type": "pick", "pixel": [1, 1], "radius": -1}),
        json!({"type": "infer", "point": [0, 0, 0], "pixel": [1, 1]}),
        // Behind the default camera.
        json!({"type": "infer", "point": [-100, -150, 80]}),
    ];
    for params in bad {
        let r = call(&mut h, "query", params.clone());
        assert_eq!(r["error"]["code"], codes::COMMAND_FAILED, "{params}: {r}");
        assert_eq!(r["error"]["data"]["kind"], "invalid_input", "{params}");
    }
    let r = call(
        &mut h,
        "query",
        json!({"type": "pick", "pixel": [1, 1], "radius_px": 2}),
    );
    assert_eq!(r["error"]["code"], codes::INVALID_PARAMS, "{r}");
}

#[test]
fn describe_gives_the_view_queries_schemas() {
    let d = api::describe();
    for name in ["EditorQuery", "PickHit", "Inferred"] {
        assert!(d["schemas"][name].is_object(), "{name}");
    }
    let text = d["schemas"]["EditorQuery"].to_string();
    assert!(
        text.contains("\"pick\"") && text.contains("\"infer\""),
        "{text}"
    );
}
```

- [ ] **Step 2: Run them to see them fail**

Run: `$HOME/.cargo/bin/cargo test -p api --test probe`
Expected: FAIL: every query comes back `-32602` (`unknown variant pick`), and `describe` has no `EditorQuery`.

- [ ] **Step 3: Answer the queries**

Add the path dependencies (cargo updates `Cargo.lock`):

```diff
--- a/crates/api/Cargo.toml
+++ b/crates/api/Cargo.toml
@@ -5,6 +5,8 @@ edition.workspace = true
 
 [dependencies]
 session = { path = "../session" }
+tools = { path = "../tools" }
+kernel = { path = "../kernel" }
 render = { path = "../render" }
 glam.workspace = true
 serde.workspace = true
```

Make the coordinate checks public:

```diff
--- a/crates/session/src/document.rs
+++ b/crates/session/src/document.rs
@@ -1333,7 +1333,7 @@ fn guide_ids(ids: &[EntityId]) -> Vec<GuideId> {
 /// `v` scaled to unit length, or None when it is zero or not finite. Very long and very
 /// short vectors work too (their squared length would overflow or underflow). Vectors of
 /// ordinary length are normalized directly, so a unit vector comes back bit for bit.
-pub(crate) fn unit_vector(v: DVec3) -> Option<DVec3> {
+pub fn unit_vector(v: DVec3) -> Option<DVec3> {
     let largest = v.abs().max_element();
     if !v.is_finite() || largest == 0.0 {
         return None;
@@ -1342,7 +1342,7 @@ pub(crate) fn unit_vector(v: DVec3) -> Option<DVec3> {
 }
 
 /// Rejects non-finite or out-of-range coordinates before they reach the kernel or camera.
-fn check_coordinates(points: &[[f64; 3]]) -> Result<(), CommandError> {
+pub fn check_coordinates(points: &[[f64; 3]]) -> Result<(), CommandError> {
     let ok = points
         .iter()
         .flatten()
```

```diff
--- a/crates/session/src/lib.rs
+++ b/crates/session/src/lib.rs
@@ -19,7 +19,7 @@ pub use circle::{DEFAULT_CIRCLE_SEGMENTS, MAX_CIRCLE_SEGMENTS, circle_points};
 pub use command::{
     BatchError, Command, CommandError, ErrorKind, Outcome, SelectMode, TextureSource,
 };
-pub use document::{Document, MAX_COORDINATE, MAX_COPIES};
+pub use document::{Document, MAX_COORDINATE, MAX_COPIES, check_coordinates, unit_vector};
 pub use export::ExportReport;
 pub use import::{
     ImportAs, ImportFormat, ImportReport, MAX_CONTEXT_IMPORT_FACES, MAX_IMPORT_BYTES,
```

Create `crates/api/src/probe.rs`:

```rust
//! The `pick` and `infer` queries: what lies under a pixel of a view or along a ray, and what
//! the tools' inference would snap to there. Pixels and cameras are the view's (root
//! coordinates, like `render_view`); points, rays and results are in the open context's
//! coordinates, like commands.

use glam::{DVec2, DVec3};
use kernel::EntityId;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use session::{
    Camera, CommandError, Document, ErrorKind, Projection, Ray, check_coordinates, unit_vector,
};
use tools::{Axis, Guides, Reach, Scenery, SnapKind, infer_detail, pick};

use crate::dispatch::MAX_IMAGE_SIDE;

/// How near a pixel an edge or guide must pass for `pick` to meet it, by default, in pixels.
pub const PICK_RADIUS: f64 = 4.0;

/// How far from a pixel `infer` snaps, by default, in pixels: the window's radius on a
/// display of scale 1.
pub const INFER_RADIUS: f64 = 8.0;

/// The largest pixel radius `pick` and `infer` take.
pub const MAX_RADIUS: f64 = 1000.0;

/// A camera for one request, as `set_camera` takes it: absent fields keep the document
/// camera's values. It never moves the document camera.
#[derive(Clone, Debug, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct View {
    pub eye: [f64; 3],
    pub target: [f64; 3],
    pub fov_y_deg: Option<f64>,
    pub up: Option<[f64; 3]>,
    pub projection: Option<Projection>,
}

impl View {
    /// The document camera moved to this view.
    pub fn camera(&self, doc: &Document) -> Result<Camera, CommandError> {
        doc.camera_for(
            self.eye,
            self.target,
            self.fov_y_deg,
            self.up,
            self.projection,
        )
    }
}

/// A ray in the open context's coordinates.
#[derive(Clone, Debug, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RaySpec {
    pub origin: [f64; 3],
    /// Any finite, non-zero length.
    pub direction: [f64; 3],
}

/// What lies under a pixel or along a ray.
#[derive(Clone, Debug, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PickQuery {
    /// A point of the view, in pixels from its top left corner (x right, y down). Give it or
    /// `ray`.
    pub pixel: Option<[f64; 2]>,
    /// The view's size in pixels (both or neither; default: the document camera's viewport,
    /// the window's 3D view).
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// The camera seeing the view (default: the document camera).
    pub camera: Option<View>,
    pub ray: Option<RaySpec>,
    /// How near edges and guides must pass: pixels on screen for a `pixel` (default 4), meters
    /// from the ray for a `ray` (default 0: faces only).
    pub radius: Option<f64>,
}

/// What the tools' inference would snap to under a pixel, or at a model point seen in a view.
#[derive(Clone, Debug, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InferQuery {
    /// A point of the view, in pixels from its top left corner. Give it or `point`.
    pub pixel: Option<[f64; 2]>,
    /// A model point, looked for where the view shows it.
    pub point: Option<[f64; 3]>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub camera: Option<View>,
    /// The point a tool is drawing from, as after a tool's first click: snaps along the axes
    /// from it are offered.
    pub from: Option<[f64; 3]>,
    /// How far from the pixel to snap, in pixels (default 8).
    pub radius: Option<f64>,
}

/// An entity and where it is: `path` lists the instances from the root down to the one whose
/// group or component holds it (empty for the root's own entities), as `open_path` takes it.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct Placed {
    pub id: EntityId,
    pub path: Vec<EntityId>,
}

/// A face, edge or guide a pick met.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct PickHit {
    pub id: EntityId,
    pub path: Vec<EntityId>,
    /// Where the ray meets a face, or the point of an edge or guide nearest the ray.
    pub point: [f64; 3],
    /// A face's unit normal: the direction its front side faces.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub normal: Option<[f64; 3]>,
    /// The side of a face the ray meets: `front` or `back`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub side: Option<String>,
    /// How far along the ray `point` is, from its origin (the eye in perspective, the near
    /// end of the view in parallel).
    pub distance: f64,
}

/// What a point snapped to, from strongest to weakest.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Snap {
    Endpoint,
    Midpoint,
    Origin,
    GuidePoint,
    Intersection,
    OnEdge,
    OnGuide,
    /// Along an axis through `from`.
    Axis,
    Parallel,
    Perpendicular,
    OnFace,
    /// The ground plane (z = 0 of the open context).
    Ground,
    /// Nothing: a point under the pixel at the depth of the camera's target.
    Nothing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AxisName {
    X,
    Y,
    Z,
}

/// What the tools' inference snapped to.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct Inferred {
    pub kind: Snap,
    /// For `axis`: which axis through `from` the point is held to (what Shift would lock).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub axis: Option<AxisName>,
    pub point: [f64; 3],
    /// What the point snapped to: the vertex of an endpoint; the edge of a midpoint or of a
    /// point on an edge; the guide of a guide point or of a point on a guide line; the two
    /// edges or guide lines crossing at an intersection; the face of a point on a face.
    pub entities: Vec<Placed>,
}

/// The camera seeing a view of `size` pixels (default: the document camera's viewport) from
/// `view` (default: the document camera).
fn view_camera(
    doc: &Document,
    view: Option<&View>,
    width: Option<u32>,
    height: Option<u32>,
) -> Result<Camera, CommandError> {
    let mut camera = match view {
        Some(v) => v.camera(doc)?,
        None => doc.camera.clone(),
    };
    match (width, height) {
        (None, None) => {}
        (Some(w), Some(h)) => {
            let sides = 1..=MAX_IMAGE_SIDE;
            if !sides.contains(&w) || !sides.contains(&h) {
                return Err(invalid(format!(
                    "width and height must be between 1 and {MAX_IMAGE_SIDE}"
                )));
            }
            camera.viewport = DVec2::new(w as f64, h as f64);
        }
        _ => return Err(invalid("give both width and height, or neither")),
    }
    Ok(camera)
}

/// `pixel` checked to lie in `camera`'s view.
fn pixel_in(camera: &Camera, pixel: [f64; 2]) -> Result<DVec2, CommandError> {
    let p = DVec2::from(pixel);
    let size = camera.viewport;
    if p.is_finite() && p.x >= 0.0 && p.y >= 0.0 && p.x <= size.x && p.y <= size.y {
        Ok(p)
    } else {
        Err(invalid(format!(
            "pixel must lie in the view, 0 to {} across and 0 to {} down",
            size.x, size.y
        )))
    }
}

/// A pixel radius: `radius`, or `default` when absent.
fn pixel_radius(radius: Option<f64>, default: f64) -> Result<f64, CommandError> {
    let r = radius.unwrap_or(default);
    if r.is_finite() && (0.0..=MAX_RADIUS).contains(&r) {
        Ok(r)
    } else {
        Err(invalid(format!(
            "radius must be between 0 and {MAX_RADIUS} pixels"
        )))
    }
}

fn invalid(message: impl Into<String>) -> CommandError {
    CommandError::new(ErrorKind::InvalidInput, message)
}

/// Everything placed, seen from the open context.
fn scenery(doc: &Document) -> Scenery<'_> {
    Scenery::of(doc.scene(), doc.context()).expect("the open context exists")
}

/// `camera` (in root coordinates) carried into the open context's.
fn local(doc: &Document, camera: &Camera) -> Camera {
    camera.placed(doc.context_transform().inverse())
}

fn path_of(scenery: &Scenery, part: usize) -> Vec<EntityId> {
    scenery
        .path(part)
        .iter()
        .map(|i| EntityId::Instance(*i))
        .collect()
}

/// Answers `pick`: every face the ray meets, front to back, and the edges and guides passing
/// within the radius.
pub fn pick_query(doc: &Document, q: &PickQuery) -> Result<Vec<PickHit>, CommandError> {
    let hits = match (&q.pixel, &q.ray) {
        (Some(pixel), None) => {
            let camera = view_camera(doc, q.camera.as_ref(), q.width, q.height)?;
            let cursor = pixel_in(&camera, *pixel)?;
            let radius_px = pixel_radius(q.radius, PICK_RADIUS)?;
            let (scenery, camera) = (scenery(doc), local(doc, &camera));
            let ray = camera.ray(cursor);
            let reach = Reach::Screen {
                camera: &camera,
                cursor,
                radius_px,
            };
            (pick(&scenery, &ray, reach), scenery, ray)
        }
        (None, Some(spec)) => {
            if q.camera.is_some() || q.width.is_some() || q.height.is_some() {
                return Err(invalid(
                    "camera, width and height go with a pixel, not a ray",
                ));
            }
            check_coordinates(&[spec.origin])?;
            let dir = unit_vector(DVec3::from(spec.direction))
                .ok_or_else(|| invalid("direction must be a finite, non-zero vector"))?;
            let radius = q.radius.unwrap_or(0.0);
            if !(radius.is_finite() && radius >= 0.0) {
                return Err(invalid(
                    "radius must be a finite number of meters, at least 0",
                ));
            }
            let scenery = scenery(doc);
            let ray = Ray {
                origin: DVec3::from(spec.origin),
                dir,
            };
            (pick(&scenery, &ray, Reach::Around { radius }), scenery, ray)
        }
        _ => return Err(invalid("give a pixel or a ray")),
    };
    let (hits, scenery, ray) = hits;
    Ok(hits
        .into_iter()
        .map(|h| PickHit {
            id: h.id,
            path: path_of(&scenery, h.part),
            point: h.point.to_array(),
            normal: h.normal.map(|n| n.to_array()),
            side: h.normal.map(|n| {
                if n.dot(ray.dir) <= 0.0 {
                    "front"
                } else {
                    "back"
                }
                .to_string()
            }),
            distance: h.distance,
        })
        .collect())
}

/// Answers `infer`: what a tool's click at the pixel (or where the view shows the point)
/// would snap to.
pub fn infer_query(doc: &Document, q: &InferQuery) -> Result<Inferred, CommandError> {
    let camera = view_camera(doc, q.camera.as_ref(), q.width, q.height)?;
    let (scenery, local) = (scenery(doc), local(doc, &camera));
    let cursor = match (&q.pixel, &q.point) {
        (Some(pixel), None) => pixel_in(&camera, *pixel)?,
        (None, Some(point)) => {
            check_coordinates(&[*point])?;
            local
                .project(DVec3::from(*point))
                .filter(|p| pixel_in(&camera, p.to_array()).is_ok())
                .ok_or_else(|| invalid("the point is not in view"))?
        }
        _ => return Err(invalid("give a pixel or a point")),
    };
    if let Some(from) = q.from {
        check_coordinates(&[from])?;
    }
    let radius_px = pixel_radius(q.radius, INFER_RADIUS)?;
    let guides = Guides {
        anchor: q.from.map(DVec3::from),
        ..Guides::default()
    };
    let (hit, involved) = infer_detail(&scenery, &local, cursor, &guides, radius_px);
    let (kind, axis) = match hit.kind {
        SnapKind::Endpoint => (Snap::Endpoint, None),
        SnapKind::Midpoint => (Snap::Midpoint, None),
        SnapKind::Origin => (Snap::Origin, None),
        SnapKind::GuidePoint => (Snap::GuidePoint, None),
        SnapKind::Intersection => (Snap::Intersection, None),
        SnapKind::OnEdge => (Snap::OnEdge, None),
        SnapKind::OnGuide => (Snap::OnGuide, None),
        SnapKind::Axis(a) => (
            Snap::Axis,
            Some(match a {
                Axis::X => AxisName::X,
                Axis::Y => AxisName::Y,
                Axis::Z => AxisName::Z,
            }),
        ),
        SnapKind::Parallel => (Snap::Parallel, None),
        SnapKind::Perpendicular => (Snap::Perpendicular, None),
        SnapKind::OnFace => (Snap::OnFace, None),
        SnapKind::Ground => (Snap::Ground, None),
        SnapKind::Free => (Snap::Nothing, None),
    };
    Ok(Inferred {
        kind,
        axis,
        point: hit.point.to_array(),
        entities: involved
            .into_iter()
            .map(|(k, id)| Placed {
                id,
                path: path_of(&scenery, k),
            })
            .collect(),
    })
}
```

Route the queries and share `View` with `render_view`:

```diff
--- a/crates/api/src/dispatch.rs
+++ b/crates/api/src/dispatch.rs
@@ -1,15 +1,16 @@
 //! Method dispatch: maps JSON-RPC methods onto session commands, queries and rendering.
 
 use base64::Engine;
-use schemars::schema_for;
+use schemars::{JsonSchema, schema_for};
 use serde::Deserialize;
 use serde::de::DeserializeOwned;
 use serde_json::{Value, json};
 use session::{
-    BatchError, Camera, ChangeEvent, Command, CommandError, Document, Lighting, Outcome,
-    Projection, Query, QueryResult,
+    BatchError, Camera, ChangeEvent, Command, CommandError, Document, Lighting, Outcome, Query,
+    QueryResult,
 };
 
+use crate::probe::{InferQuery, Inferred, PickHit, PickQuery, View, infer_query, pick_query};
 use crate::protocol::{RpcError, codes, failure, parse, success};
 
 /// Largest image side `render_view` accepts, in pixels.
@@ -36,6 +37,25 @@ pub trait Host {
     ) -> Option<Vec<u8>>;
 }
 
+/// Queries the API answers from the view around the document: what lies under a pixel or
+/// along a ray, and what the tools' inference would snap to there (see `probe`). Other
+/// queries are `session::Query`s.
+#[derive(Clone, Debug, PartialEq, Deserialize, JsonSchema)]
+#[serde(tag = "type", rename_all = "snake_case")]
+pub enum EditorQuery {
+    /// Every face a pixel's ray (or a given ray) meets, front to back, and the edges and
+    /// guides passing near it, each with its instance path from the root.
+    Pick(PickQuery),
+    /// What a tool's click at a pixel (or at a model point where the view shows it) would
+    /// snap to: kind, point and the entities involved.
+    Infer(InferQuery),
+}
+
+/// The `type` of a query or command, if it has one.
+fn type_of(params: &Value) -> Option<&str> {
+    params.get("type").and_then(Value::as_str)
+}
+
 /// Handles one request line and returns the reply line (None for notifications).
 /// `subscribe` is connection-level and handled by `Hub`.
 pub fn handle(host: &mut dyn Host, line: &str) -> Option<String> {
@@ -78,6 +98,10 @@ fn call(host: &mut dyn Host, method: &str, params: Value) -> Result<Value, RpcEr
             })?;
             Ok(json!({"outcomes": outcomes, "revision": host.document().revision()}))
         }
+        "query" if matches!(type_of(&params), Some("pick" | "infer")) => {
+            let query: EditorQuery = params_as(params)?;
+            editor_query(host, query)
+        }
         "query" => {
             let query: Query = params_as(params)?;
             let result = host.document().query(&query).map_err(command_failed)?;
@@ -96,16 +120,22 @@ fn call(host: &mut dyn Host, method: &str, params: Value) -> Result<Value, RpcEr
     }
 }
 
-fn render_view(host: &mut dyn Host, params: Value) -> Result<Value, RpcError> {
-    #[derive(Deserialize)]
-    #[serde(deny_unknown_fields)]
-    struct View {
-        eye: [f64; 3],
-        target: [f64; 3],
-        fov_y_deg: Option<f64>,
-        up: Option<[f64; 3]>,
-        projection: Option<Projection>,
+fn editor_query(host: &mut dyn Host, query: EditorQuery) -> Result<Value, RpcError> {
+    let doc = host.document();
+    match query {
+        EditorQuery::Pick(q) => {
+            let hits = pick_query(doc, &q).map_err(command_failed)?;
+            Ok(json!({"type": "pick", "hits": hits}))
+        }
+        EditorQuery::Infer(q) => {
+            let mut result = json!(infer_query(doc, &q).map_err(command_failed)?);
+            result["type"] = json!("infer");
+            Ok(result)
+        }
     }
+}
+
+fn render_view(host: &mut dyn Host, params: Value) -> Result<Value, RpcError> {
     #[derive(Deserialize)]
     #[serde(deny_unknown_fields)]
     struct Params {
@@ -124,9 +154,7 @@ fn render_view(host: &mut dyn Host, params: Value) -> Result<Value, RpcError> {
     }
     let doc = host.document();
     let camera = match p.camera {
-        Some(v) => doc
-            .camera_for(v.eye, v.target, v.fov_y_deg, v.up, v.projection)
-            .map_err(command_failed)?,
+        Some(v) => v.camera(doc).map_err(command_failed)?,
         None => doc.camera.clone(),
     };
     let lighting = p.lighting.unwrap_or(doc.lighting);
@@ -170,7 +198,10 @@ pub fn describe() -> Value {
                 "result": "{outcomes: [Outcome], revision}",
                 "notes": "atomic: one undo step, all-or-nothing; undo/redo not allowed"
             },
-            "query": {"params": "Query", "result": "QueryResult"},
+            "query": {
+                "params": "Query or EditorQuery",
+                "result": "QueryResult, or for EditorQuery: pick {type, hits: [PickHit]}, infer {type, ...Inferred}"
+            },
             "render_view": {
                 "params": "{width, height, camera?: {eye, target, fov_y_deg?, up?, projection?}, lighting?: \"day\" | \"night\"}",
                 "result": "{width, height, png_base64}",
@@ -191,6 +222,9 @@ pub fn describe() -> Value {
         "schemas": {
             "Command": schema_for!(Command),
             "Query": schema_for!(Query),
+            "EditorQuery": schema_for!(EditorQuery),
+            "PickHit": schema_for!(PickHit),
+            "Inferred": schema_for!(Inferred),
             "QueryResult": schema_for!(QueryResult),
             "Outcome": schema_for!(Outcome),
             "CommandError": schema_for!(CommandError),
```

```diff
--- a/crates/api/src/lib.rs
+++ b/crates/api/src/lib.rs
@@ -4,11 +4,13 @@
 mod dispatch;
 mod headless;
 mod hub;
+mod probe;
 mod protocol;
 mod transport;
 
-pub use dispatch::{Host, describe, handle};
+pub use dispatch::{EditorQuery, Host, describe, handle};
 pub use headless::HeadlessHost;
 pub use hub::{Hub, Inbox};
+pub use probe::{InferQuery, Inferred, PickHit, PickQuery, Placed, RaySpec, Snap, View};
 pub use protocol::{MAX_LINE_BYTES, codes};
 pub use transport::{listen_unix, serve_stdio, serve_stream};
```

- [ ] **Step 4: Run the tests**

Run: `$HOME/.cargo/bin/cargo test -p api`
Expected: PASS.

- [ ] **Step 5: Document picking and inference**

```diff
--- a/docs/agent-api.md
+++ b/docs/agent-api.md
@@ -58,6 +58,8 @@ Queries:
 | `{"type": "outliner"}` | the tree of placed groups and components in the whole model, plus `revision`; optional `depth` limits it; see Groups and components below |
 | `{"type": "file"}` | `path` of the file last opened or saved (`null` if never), `modified` since then, and `revision` |
 | `{"type": "events", "since": 3}` | model changes after revision 3, oldest first |
+| `{"type": "pick", "pixel": [320, 200], "width": 640, "height": 400}` | what lies under a pixel of a view, or along a `ray`: `hits`, front to back; see Picking and inference below |
+| `{"type": "infer", "pixel": [320, 200]}` | what a tool's click there would snap to: `kind`, `point` and the `entities` involved; see Picking and inference below |
 
 Unknown fields in commands, queries and params are rejected with `-32602` (a typo fails instead
 of silently doing nothing).
@@ -502,6 +504,65 @@ lists these controls) turns it on; Esc or F turns it off.
   the rest) still work. Switching to another window lets go of every held key and frees the
   pointer. There is no collision with the model.
 
+### Picking and inference
+
+`pick` and `infer` answer what an agent looking at a `render_view` image (or at the window)
+needs to act on a pixel. A pixel is `[x, y]` in pixels from the view's top left corner (x right, y
+down; the middle of the image's pixel (i, j) is `[i + 0.5, j + 0.5]`), in a view of `width` x
+`height` pixels (both or neither, 1 to 4096; default the window's 3D view, 1280 x 800 headless)
+seen by `camera` (`eye`, `target` and the optional `fov_y_deg`, `up` and `projection`, as
+`render_view` takes them; default the document camera, which never moves). So a pixel of a
+`render_view` image is picked by passing the same `width`, `height` and `camera`. A pixel outside
+the view is `invalid_input`. Points, rays, normals and distances are in the open context's
+coordinates, like commands; only the camera is in root coordinates.
+
+`{"type": "pick", "pixel": [x, y], ...}` or `{"type": "pick", "ray": {"origin": [...],
+"direction": [...]}}` returns every face the pixel's ray (or the given ray, any non-zero
+direction) meets in front of its origin, whatever lies in front of it, and every edge (soft
+edges are hidden and left out) and guide passing within `radius` of it, nearest first:
+
+```
+{"type": "pick", "hits": [
+  {"id": "f25", "path": [], "point": [1,1,1], "normal": [0,0,1], "side": "front", "distance": 9.0},
+  {"id": "f12", "path": ["i40"], "point": [1,1,0], "normal": [0,0,-1], "side": "back", "distance": 10.0}]}
+```
+
+- `path` lists the instances from the root down to the one whose group or component holds the
+  entity (`[]` for the root's own), whatever instance is open; `open_path` with it opens that
+  group, after which the id is the open context's.
+- `point` is where the ray meets a face, or the point of an edge or guide nearest the ray;
+  `distance` is how far along the ray it is, from its origin (the eye in perspective, the near
+  end of the view in parallel). Faces have `normal` (the direction their front faces) and `side`
+  (`front` or `back`: which side the ray meets). Vertices are not picked (`infer` finds them).
+- `radius` is in pixels on screen for a pixel (default 4, at most 1000) and in meters from the
+  ray for a ray (default 0: faces only). `camera`, `width` and `height` go with a pixel only.
+
+`{"type": "infer", "pixel": [x, y], ...}` or `{"type": "infer", "point": [x, y, z], ...}` (a
+model point, looked for where the view shows it; `invalid_input` when it is not in view) returns
+what a tool's click there would snap to, with the window's inference rules (points hidden behind
+faces are skipped; the order of preference is endpoint, midpoint, origin and guide point, then
+intersection, on an edge, on a guide line, along an axis from `from`, on a face, the ground):
+
+```
+{"type": "infer", "kind": "endpoint", "point": [2,2,1], "entities": [{"id": "v5", "path": []}]}
+```
+
+- `kind` is `endpoint`, `midpoint`, `origin`, `guide_point`, `intersection` (two edges or
+  guide lines crossing on screen; the point lies on the nearer), `on_edge`, `on_guide`, `axis`
+  (with `axis`: `x`, `y` or `z`, the axis through `from` the point is held to, which Shift would
+  lock), `on_face`, `ground` (z = 0 of the open context) or `nothing` (a point under the pixel at
+  the depth of the camera's target).
+- `entities` are what it snapped to, each with its `path` as in `pick`: the vertex of an
+  endpoint, the edge of a midpoint or of a point on an edge, the guide of a guide point or of a
+  point on a guide line, the two edges or guide lines of an intersection (the one the point lies
+  on first), the face of a point on a face; empty for the others.
+- `from` is the point a tool is drawing from, as after its first click: points along the red,
+  green and blue axes through it are offered. Without it there are no axis snaps. The editor's
+  own state (its tool, its last point, a Shift or arrow-key lock, the edge the cursor last
+  rested on for parallel snaps) is not used.
+- `radius` is how far from the pixel to snap, in pixels (default 8, the window's on a display of
+  scale 1; at most 1000).
+
 ### Files
 
 Models are saved as `.skr` files: a zip archive holding `document.json`, which records `format`
```

- [ ] **Step 6: Check and commit**

Run: `$HOME/.cargo/bin/cargo fmt --all --check && $HOME/.cargo/bin/cargo clippy --workspace --all-targets -- -D warnings && $HOME/.cargo/bin/cargo test --workspace`
Expected: clean; all pass.

```bash
git status && git diff
git add Cargo.lock crates/api/Cargo.toml crates/api/src/probe.rs crates/api/src/dispatch.rs crates/api/src/lib.rs crates/api/tests/probe.rs crates/session/src/document.rs crates/session/src/lib.rs docs/agent-api.md
git commit -m "API: pick and infer queries"
```

---

### Task 5: Editor state, editor commands and `quit` in the API

**Files:**
- Create: `crates/api/src/editor.rs`, `crates/api/tests/editor.rs`
- Modify: `crates/api/src/dispatch.rs`, `crates/api/src/lib.rs`, `crates/session/src/import.rs`, `crates/session/src/lib.rs`, `crates/session/src/document/files.rs`, `crates/app/src/editor.rs` (its `ImportUnits` moves to `session`)

**Interfaces:**
- Consumes: `tools::ToolKind`; `EditorQuery` and `type_of` (Task 4).
- Produces:
  - `session::ImportUnits` (serde snake_case; `ALL`, `label()`, `meters()`), re-exported by `app::editor` (`pub use session::ImportUnits;`).
  - `Document::check_unsaved(&self, discard: bool) -> Result<(), CommandError>`, now public.
  - `api::ToolName` (snake_case; `From<ToolKind>` both ways), `api::PhaseName { Idle, Active }`, `api::Dialog { SoftenEdges }`, `api::EditorState` (fields as in the Spec notes; `Clone, Debug, PartialEq, Serialize, JsonSchema`), `api::EditorCommand { SetTool { tool }, SetPaintMaterial { material: Option<MaterialId> }, SetImportUnits { units }, Cancel }`, `editor::EDITOR_COMMANDS`.
  - `Host` gains (with defaults refusing: `invalid_input`):
    `fn editor(&self) -> Option<EditorState>` (default None),
    `fn editor_command(&mut self, command: EditorCommand) -> Result<(), CommandError>`,
    `fn quit(&mut self, discard: bool) -> Result<(), CommandError>`.
  - `EditorQuery::Editor`; `execute` of an editor command returns `{"editor": EditorState, "revision"}`; method `quit` returns `{}`.

- [ ] **Step 1: Write the failing tests**

Create `crates/api/tests/editor.rs` (Task 6 appends one more test to it):

```rust
//! Editor commands, the `editor` query and `quit` over JSON-RPC, against a host with a simple
//! editor and against one without.

use api::{
    EditorCommand, EditorState, HeadlessHost, Host, PhaseName, ToolName, codes, describe, handle,
};
use serde_json::{Value, json};
use session::{
    BatchError, Camera, Command, CommandError, Document, ImportUnits, Lighting, Outcome,
};

/// A document with an editor that only records what it was told.
struct Editing {
    inner: HeadlessHost,
    state: EditorState,
    quit: bool,
}

impl Editing {
    fn new() -> Self {
        Editing {
            inner: HeadlessHost::new(),
            state: EditorState {
                tool: ToolName::Line,
                phase: PhaseName::Idle,
                points: Vec::new(),
                paint_material: None,
                import_units: ImportUnits::Meters,
                flying: false,
                fly_speed: 3.0,
                viewport: [1280.0, 800.0],
                status: "Line  |  Click to start a line".into(),
                prompt: "Click to start a line".into(),
                message: None,
                lock: None,
                measurement_label: "Length".into(),
                measurement: None,
                typing: false,
                soften_angle: 20,
                dialog: None,
            },
            quit: false,
        }
    }
}

impl Host for Editing {
    fn document(&self) -> &Document {
        self.inner.document()
    }

    fn execute(&mut self, command: Command) -> Result<Outcome, CommandError> {
        self.inner.execute(command)
    }

    fn execute_batch(
        &mut self,
        commands: Vec<Command>,
        atomic: bool,
    ) -> Result<Vec<Outcome>, BatchError> {
        self.inner.execute_batch(commands, atomic)
    }

    fn render_view(&mut self, _: &Camera, _: Lighting, _: u32, _: u32) -> Option<Vec<u8>> {
        None
    }

    fn editor(&self) -> Option<EditorState> {
        Some(self.state.clone())
    }

    fn editor_command(&mut self, command: EditorCommand) -> Result<(), CommandError> {
        match command {
            EditorCommand::SetTool { tool } => self.state.tool = tool,
            EditorCommand::SetPaintMaterial { material } => self.state.paint_material = material,
            EditorCommand::SetImportUnits { units } => self.state.import_units = units,
            EditorCommand::Cancel => self.state.phase = PhaseName::Idle,
        }
        Ok(())
    }

    fn quit(&mut self, discard: bool) -> Result<(), CommandError> {
        self.document().check_unsaved(discard)?;
        self.quit = true;
        Ok(())
    }
}

fn call(host: &mut dyn Host, method: &str, params: Value) -> Value {
    let line = json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).to_string();
    serde_json::from_str(&handle(host, &line).expect("a reply")).unwrap()
}

#[test]
fn editor_commands_set_the_editor_and_return_its_state() {
    let mut h = Editing::new();
    let r = call(
        &mut h,
        "execute",
        json!({"type": "set_tool", "tool": "push_pull"}),
    );
    assert_eq!(r["result"]["editor"]["tool"], "push_pull", "{r}");
    assert_eq!(r["result"]["revision"], 0);
    let r = call(
        &mut h,
        "execute",
        json!({"type": "set_import_units", "units": "millimeters"}),
    );
    assert_eq!(r["result"]["editor"]["import_units"], "millimeters");
    let r = call(
        &mut h,
        "execute",
        json!({"type": "set_paint_material", "material": "m3"}),
    );
    assert_eq!(r["result"]["editor"]["paint_material"], "m3");
    let r = call(&mut h, "execute", json!({"type": "set_paint_material"}));
    assert_eq!(r["result"]["editor"]["paint_material"], Value::Null);
    let r = call(&mut h, "execute", json!({"type": "cancel"}));
    assert_eq!(r["result"]["editor"]["phase"], "idle");

    let state = call(&mut h, "query", json!({"type": "editor"}))["result"].clone();
    assert_eq!(state["type"], "editor");
    assert_eq!(state["tool"], "push_pull");
    assert_eq!(state["viewport"], json!([1280.0, 800.0]));
    assert_eq!(state["dialog"], Value::Null);
}

#[test]
fn editor_commands_and_queries_check_their_params() {
    let mut h = Editing::new();
    for (method, params) in [
        ("execute", json!({"type": "set_tool", "tool": "lasso"})),
        ("execute", json!({"type": "set_tool"})),
        (
            "execute",
            json!({"type": "set_import_units", "units": "yards"}),
        ),
        (
            "batch",
            json!({"commands": [{"type": "set_tool", "tool": "line"}]}),
        ),
        ("quit", json!({"discard": "yes"})),
    ] {
        let r = call(&mut h, method, params.clone());
        assert_eq!(r["error"]["code"], codes::INVALID_PARAMS, "{params}: {r}");
    }
}

#[test]
fn quit_refuses_unsaved_changes_unless_discarded() {
    let mut h = Editing::new();
    let r = call(&mut h, "quit", Value::Null);
    assert_eq!(r["result"], json!({}));
    assert!(h.quit);

    let mut h = Editing::new();
    call(
        &mut h,
        "execute",
        json!({"type": "add_edge", "start": [0, 0, 0], "end": [1, 0, 0]}),
    );
    let r = call(&mut h, "quit", json!({}));
    assert_eq!(r["error"]["data"]["kind"], "unsaved_changes", "{r}");
    assert!(!h.quit);
    let r = call(&mut h, "quit", json!({"discard": true}));
    assert_eq!(r["result"], json!({}));
    assert!(h.quit);
}

#[test]
fn a_host_without_an_editor_says_so() {
    let mut h = HeadlessHost::new();
    for (method, params) in [
        ("execute", json!({"type": "set_tool", "tool": "line"})),
        ("query", json!({"type": "editor"})),
        ("quit", json!({"discard": true})),
    ] {
        let r = call(&mut h, method, params.clone());
        assert_eq!(r["error"]["code"], codes::COMMAND_FAILED, "{params}: {r}");
        assert_eq!(r["error"]["data"]["kind"], "invalid_input");
    }
}

#[test]
fn describe_lists_quit_and_the_editor_schemas() {
    let d = describe();
    assert!(d["methods"]["quit"].is_object());
    for name in ["EditorCommand", "EditorState", "EditorQuery"] {
        assert!(d["schemas"][name].is_object(), "{name}");
    }
}
```

- [ ] **Step 2: Run them to see them fail**

Run: `$HOME/.cargo/bin/cargo test -p api --test editor`
Expected: FAIL to compile: no `EditorCommand`, `EditorState`, `PhaseName`, `ToolName` in `api`, no `session::ImportUnits`.

- [ ] **Step 3: Move `ImportUnits`, add the editor types and route them**

```diff
--- a/crates/session/src/import.rs
+++ b/crates/session/src/import.rs
@@ -93,6 +93,50 @@ pub enum ImportAs {
     Context,
 }
 
+/// The length one unit of an imported OBJ or STL file stands for (the files carry no units):
+/// the window's File > Import Units, which sets the `import` command's `scale`.
+#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
+#[serde(rename_all = "snake_case")]
+pub enum ImportUnits {
+    #[default]
+    Meters,
+    Centimeters,
+    Millimeters,
+    Inches,
+    Feet,
+}
+
+impl ImportUnits {
+    pub const ALL: [ImportUnits; 5] = [
+        ImportUnits::Meters,
+        ImportUnits::Centimeters,
+        ImportUnits::Millimeters,
+        ImportUnits::Inches,
+        ImportUnits::Feet,
+    ];
+
+    pub fn label(self) -> &'static str {
+        match self {
+            ImportUnits::Meters => "Meters",
+            ImportUnits::Centimeters => "Centimeters",
+            ImportUnits::Millimeters => "Millimeters",
+            ImportUnits::Inches => "Inches",
+            ImportUnits::Feet => "Feet",
+        }
+    }
+
+    /// Meters per file unit: the import command's `scale`.
+    pub fn meters(self) -> f64 {
+        match self {
+            ImportUnits::Meters => 1.0,
+            ImportUnits::Centimeters => 0.01,
+            ImportUnits::Millimeters => 0.001,
+            ImportUnits::Inches => 0.0254,
+            ImportUnits::Feet => 0.3048,
+        }
+    }
+}
+
 /// What an `import` command read and made.
 #[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
 pub struct ImportReport {
```

```diff
--- a/crates/session/src/lib.rs
+++ b/crates/session/src/lib.rs
@@ -22,7 +22,7 @@ pub use command::{
 pub use document::{Document, MAX_COORDINATE, MAX_COPIES, check_coordinates, unit_vector};
 pub use export::ExportReport;
 pub use import::{
-    ImportAs, ImportFormat, ImportReport, MAX_CONTEXT_IMPORT_FACES, MAX_IMPORT_BYTES,
+    ImportAs, ImportFormat, ImportReport, ImportUnits, MAX_CONTEXT_IMPORT_FACES, MAX_IMPORT_BYTES,
     MAX_IMPORT_MATERIALS, MAX_IMPORT_POINTS, MAX_IMPORT_POLYGONS, UpAxis,
 };
 pub use kernel::{CurveKind, ImageFormat, MaterialId, Side, TexturePlacement};
```

```diff
--- a/crates/session/src/document/files.rs
+++ b/crates/session/src/document/files.rs
@@ -12,7 +12,9 @@ use crate::file;
 
 impl Document {
     /// Fails when the model has changes not saved, unless the command said to discard them.
-    pub(super) fn check_unsaved(&self, discard: bool) -> Result<(), CommandError> {
+    /// Refuses with `unsaved_changes` when the model changed since it was opened, saved or
+    /// started, unless `discard`: what `new`, `open` and quitting check first.
+    pub fn check_unsaved(&self, discard: bool) -> Result<(), CommandError> {
         if self.is_modified() && !discard {
             return Err(CommandError::new(
                 ErrorKind::UnsavedChanges,
```

```diff
--- a/crates/app/src/editor.rs
+++ b/crates/app/src/editor.rs
@@ -9,6 +9,7 @@ use std::time::Instant;
 use glam::{DAffine3, DVec2, DVec3};
 use kernel::{EntityId, InstanceId, MaterialId};
 use render::grid::LineVertex;
+pub use session::ImportUnits;
 use session::{
     BatchError, Camera, Command, CommandError, Document, ErrorKind, ImportAs, Lighting, Outcome,
     Projection, SelectMode, StandardView, TextureSource,
@@ -140,48 +141,6 @@ pub const EXPORT_FORMATS: [(&str, &str); 4] = [
     ("glTF", "gltf"),
 ];
 
-/// The length one unit of an imported OBJ or STL file stands for.
-#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
-pub enum ImportUnits {
-    #[default]
-    Meters,
-    Centimeters,
-    Millimeters,
-    Inches,
-    Feet,
-}
-
-impl ImportUnits {
-    pub const ALL: [ImportUnits; 5] = [
-        ImportUnits::Meters,
-        ImportUnits::Centimeters,
-        ImportUnits::Millimeters,
-        ImportUnits::Inches,
-        ImportUnits::Feet,
-    ];
-
-    pub fn label(self) -> &'static str {
-        match self {
-            ImportUnits::Meters => "Meters",
-            ImportUnits::Centimeters => "Centimeters",
-            ImportUnits::Millimeters => "Millimeters",
-            ImportUnits::Inches => "Inches",
-            ImportUnits::Feet => "Feet",
-        }
-    }
-
-    /// Meters per file unit: the import command's `scale`.
-    pub fn meters(self) -> f64 {
-        match self {
-            ImportUnits::Meters => 1.0,
-            ImportUnits::Centimeters => 0.01,
-            ImportUnits::Millimeters => 0.001,
-            ImportUnits::Inches => 0.0254,
-            ImportUnits::Feet => 0.3048,
-        }
-    }
-}
-
 /// The colors the Materials panel offers; new materials get them in turn.
 pub const MATERIAL_PALETTE: [[u8; 3]; 12] = [
     [196, 84, 64],
```

Create `crates/api/src/editor.rs`:

```rust
//! The editor around the document, as agents see and set it: the tool and what it is doing,
//! the material the Paint tool paints with, the units imports are read in, fly mode, the
//! status bar and the measurements box.

use kernel::MaterialId;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use session::ImportUnits;
use tools::ToolKind;

/// A tool, by name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ToolName {
    Select,
    Line,
    Rectangle,
    Circle,
    Arc,
    PushPull,
    Move,
    Rotate,
    TapeMeasure,
    Erase,
    Paint,
}

impl From<ToolKind> for ToolName {
    fn from(kind: ToolKind) -> Self {
        match kind {
            ToolKind::Select => ToolName::Select,
            ToolKind::Line => ToolName::Line,
            ToolKind::Rectangle => ToolName::Rectangle,
            ToolKind::Circle => ToolName::Circle,
            ToolKind::Arc => ToolName::Arc,
            ToolKind::PushPull => ToolName::PushPull,
            ToolKind::Move => ToolName::Move,
            ToolKind::Rotate => ToolName::Rotate,
            ToolKind::TapeMeasure => ToolName::TapeMeasure,
            ToolKind::Erase => ToolName::Erase,
            ToolKind::Paint => ToolName::Paint,
        }
    }
}

impl From<ToolName> for ToolKind {
    fn from(name: ToolName) -> Self {
        match name {
            ToolName::Select => ToolKind::Select,
            ToolName::Line => ToolKind::Line,
            ToolName::Rectangle => ToolKind::Rectangle,
            ToolName::Circle => ToolKind::Circle,
            ToolName::Arc => ToolKind::Arc,
            ToolName::PushPull => ToolKind::PushPull,
            ToolName::Move => ToolKind::Move,
            ToolName::Rotate => ToolKind::Rotate,
            ToolName::TapeMeasure => ToolKind::TapeMeasure,
            ToolName::Erase => ToolKind::Erase,
            ToolName::Paint => ToolKind::Paint,
        }
    }
}

/// Whether the tool is in the middle of an operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PhaseName {
    /// The next click starts an operation.
    Idle,
    /// An operation is in progress.
    Active,
}

/// A window the editor shows over the view.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Dialog {
    /// Edit > Soften Edges...
    SoftenEdges,
}

/// What the editor is doing: the `editor` query's result and `editor` notification.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct EditorState {
    /// The active tool.
    pub tool: ToolName,
    pub phase: PhaseName,
    /// The points the operation in progress was given so far (clicks), in the open
    /// context's coordinates; empty when idle (and for the Select tool, whose clicks and
    /// boxes are on screen).
    pub points: Vec<[f64; 3]>,
    /// The material the Paint tool paints with; null for the default material.
    pub paint_material: Option<MaterialId>,
    /// What one unit of an imported OBJ or STL file is (File > Import Units).
    pub import_units: ImportUnits,
    /// Whether fly mode is on (the window only).
    pub flying: bool,
    /// Fly mode's speed, meters a second.
    pub fly_speed: f64,
    /// The 3D view's size in pixels: the view `pick` and `infer` use by default.
    pub viewport: [f64; 2],
    /// The status bar as the window shows it: tool, prompt, lock, measurement and message.
    pub status: String,
    /// What to do next.
    pub prompt: String,
    /// Feedback from the last tool action or failed command.
    pub message: Option<String>,
    /// The direction lock in force, e.g. "Locked: red axis".
    pub lock: Option<String>,
    /// The measurements box's label, e.g. "Length"; empty when the tool measures nothing.
    pub measurement_label: String,
    /// The measurements box's text: what is typed while typing, else the live measurement.
    pub measurement: Option<String>,
    /// Whether text is being typed into the measurements box.
    pub typing: bool,
    /// The Soften Edges window's angle, in degrees.
    pub soften_angle: u32,
    /// The window open over the view, if any.
    pub dialog: Option<Dialog>,
}

/// Commands that set the editor rather than the model: no undo step, no revision. `execute`
/// takes them (not `batch`).
#[derive(Clone, Debug, PartialEq, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum EditorCommand {
    /// Switch to a tool (a fresh one, even if it is the active tool), dropping the operation in
    /// progress and anything typed.
    SetTool { tool: ToolName },
    /// The material the Paint tool paints with (null or absent: the default material). The
    /// tool does not change.
    SetPaintMaterial {
        #[serde(default)]
        material: Option<MaterialId>,
    },
    /// What one unit of an imported file is, for the window's File > Import.
    SetImportUnits { units: ImportUnits },
    /// Abort the tool's operation in progress, like Escape: drops typed text, the direction
    /// lock and the operation (and a copy array or a retyped angle it waits for). It never
    /// closes the open group (that is `close_instance`).
    Cancel,
}

/// The `type` of each editor command.
pub const EDITOR_COMMANDS: [&str; 4] = [
    "set_tool",
    "set_paint_material",
    "set_import_units",
    "cancel",
];
```

```diff
--- a/crates/api/src/dispatch.rs
+++ b/crates/api/src/dispatch.rs
@@ -6,10 +6,11 @@ use serde::Deserialize;
 use serde::de::DeserializeOwned;
 use serde_json::{Value, json};
 use session::{
-    BatchError, Camera, ChangeEvent, Command, CommandError, Document, Lighting, Outcome, Query,
-    QueryResult,
+    BatchError, Camera, ChangeEvent, Command, CommandError, Document, ErrorKind, Lighting, Outcome,
+    Query, QueryResult,
 };
 
+use crate::editor::{EDITOR_COMMANDS, EditorCommand, EditorState};
 use crate::probe::{InferQuery, Inferred, PickHit, PickQuery, View, infer_query, pick_query};
 use crate::protocol::{RpcError, codes, failure, parse, success};
 
@@ -35,13 +36,34 @@ pub trait Host {
         width: u32,
         height: u32,
     ) -> Option<Vec<u8>>;
+    /// The editor's state, or None for a program without an editor (a bare document).
+    fn editor(&self) -> Option<EditorState> {
+        None
+    }
+    /// Runs an editor command.
+    fn editor_command(&mut self, _command: EditorCommand) -> Result<(), CommandError> {
+        Err(no_editor())
+    }
+    /// Ends the program once the replies waiting are sent. Refuses with `unsaved_changes`
+    /// when the model changed since it was opened, saved or started, unless `discard`.
+    fn quit(&mut self, _discard: bool) -> Result<(), CommandError> {
+        Err(CommandError::new(
+            ErrorKind::InvalidInput,
+            "this program cannot be quit through the API",
+        ))
+    }
+}
+
+/// The error for editor commands and queries in a program without an editor.
+fn no_editor() -> CommandError {
+    CommandError::new(ErrorKind::InvalidInput, "this program has no editor")
 }
 
 /// Queries the API answers from the view around the document: what lies under a pixel or
 /// along a ray, and what the tools' inference would snap to there (see `probe`). Other
 /// queries are `session::Query`s.
 #[derive(Clone, Debug, PartialEq, Deserialize, JsonSchema)]
-#[serde(tag = "type", rename_all = "snake_case")]
+#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
 pub enum EditorQuery {
     /// Every face a pixel's ray (or a given ray) meets, front to back, and the edges and
     /// guides passing near it, each with its instance path from the root.
@@ -49,6 +71,8 @@ pub enum EditorQuery {
     /// What a tool's click at a pixel (or at a model point where the view shows it) would
     /// snap to: kind, point and the entities involved.
     Infer(InferQuery),
+    /// The editor's state (see `EditorState`).
+    Editor,
 }
 
 /// The `type` of a query or command, if it has one.
@@ -73,6 +97,11 @@ pub fn handle(host: &mut dyn Host, line: &str) -> Option<String> {
 
 fn call(host: &mut dyn Host, method: &str, params: Value) -> Result<Value, RpcError> {
     match method {
+        "execute" if type_of(&params).is_some_and(|t| EDITOR_COMMANDS.contains(&t)) => {
+            let command: EditorCommand = params_as(params)?;
+            host.editor_command(command).map_err(command_failed)?;
+            Ok(json!({"editor": host.editor(), "revision": host.document().revision()}))
+        }
         "execute" => {
             let command: Command = params_as(params)?;
             let outcome = host.execute(command).map_err(command_failed)?;
@@ -98,7 +127,7 @@ fn call(host: &mut dyn Host, method: &str, params: Value) -> Result<Value, RpcEr
             })?;
             Ok(json!({"outcomes": outcomes, "revision": host.document().revision()}))
         }
-        "query" if matches!(type_of(&params), Some("pick" | "infer")) => {
+        "query" if matches!(type_of(&params), Some("pick" | "infer" | "editor")) => {
             let query: EditorQuery = params_as(params)?;
             editor_query(host, query)
         }
@@ -108,6 +137,21 @@ fn call(host: &mut dyn Host, method: &str, params: Value) -> Result<Value, RpcEr
             Ok(json!(result))
         }
         "render_view" => render_view(host, params),
+        "quit" => {
+            #[derive(Deserialize)]
+            #[serde(deny_unknown_fields)]
+            struct Params {
+                #[serde(default)]
+                discard: bool,
+            }
+            let p: Params = if params.is_null() {
+                Params { discard: false }
+            } else {
+                params_as(params)?
+            };
+            host.quit(p.discard).map_err(command_failed)?;
+            Ok(json!({}))
+        }
         "describe" => Ok(describe()),
         "subscribe" => Err(RpcError::new(
             codes::INVALID_REQUEST,
@@ -132,6 +176,15 @@ fn editor_query(host: &mut dyn Host, query: EditorQuery) -> Result<Value, RpcErr
             result["type"] = json!("infer");
             Ok(result)
         }
+        EditorQuery::Editor => {
+            let state = host
+                .editor()
+                .ok_or_else(no_editor)
+                .map_err(command_failed)?;
+            let mut result = json!(state);
+            result["type"] = json!("editor");
+            Ok(result)
+        }
     }
 }
 
@@ -190,7 +243,7 @@ pub fn describe() -> Value {
         "units": "meters; Z is up; angles in degrees",
         "methods": {
             "execute": {
-                "params": "Command",
+                "params": "Command or EditorCommand (EditorCommand result: {editor: EditorState, revision})",
                 "result": "{changes: ChangeSummary, import?: ImportReport, export?: ExportReport, soften?: SoftenReport, positioned?: integer, revision}: the Outcome plus revision"
             },
             "batch": {
@@ -200,7 +253,7 @@ pub fn describe() -> Value {
             },
             "query": {
                 "params": "Query or EditorQuery",
-                "result": "QueryResult, or for EditorQuery: pick {type, hits: [PickHit]}, infer {type, ...Inferred}"
+                "result": "QueryResult, or for EditorQuery: pick {type, hits: [PickHit]}, infer {type, ...Inferred}, editor {type, ...EditorState}"
             },
             "render_view": {
                 "params": "{width, height, camera?: {eye, target, fov_y_deg?, up?, projection?}, lighting?: \"day\" | \"night\"}",
@@ -208,7 +261,12 @@ pub fn describe() -> Value {
                 "notes": "does not move the document camera or change its lighting"
             },
             "subscribe": {"params": null, "result": "{revision}", "notes": "connection only"},
-            "describe": {"params": null, "result": "this document"}
+            "describe": {"params": null, "result": "this document"},
+            "quit": {
+                "params": "{discard?: bool}",
+                "result": "{}",
+                "notes": "ends the program after replying; fails with unsaved_changes unless discard"
+            }
         },
         "notifications": {
             "changed": "{revision, changes: ChangeSummary} after every model change, in order",
@@ -225,6 +283,8 @@ pub fn describe() -> Value {
             "EditorQuery": schema_for!(EditorQuery),
             "PickHit": schema_for!(PickHit),
             "Inferred": schema_for!(Inferred),
+            "EditorCommand": schema_for!(EditorCommand),
+            "EditorState": schema_for!(EditorState),
             "QueryResult": schema_for!(QueryResult),
             "Outcome": schema_for!(Outcome),
             "CommandError": schema_for!(CommandError),
```

```diff
--- a/crates/api/src/lib.rs
+++ b/crates/api/src/lib.rs
@@ -2,6 +2,7 @@
 //! Every request runs through `session` commands and queries, the same path the UI uses.
 
 mod dispatch;
+mod editor;
 mod headless;
 mod hub;
 mod probe;
@@ -9,6 +10,7 @@ mod protocol;
 mod transport;
 
 pub use dispatch::{EditorQuery, Host, describe, handle};
+pub use editor::{Dialog, EditorCommand, EditorState, PhaseName, ToolName};
 pub use headless::HeadlessHost;
 pub use hub::{Hub, Inbox};
 pub use probe::{InferQuery, Inferred, PickHit, PickQuery, Placed, RaySpec, Snap, View};
```

- [ ] **Step 4: Run the tests**

Run: `$HOME/.cargo/bin/cargo test -p api && $HOME/.cargo/bin/cargo test -p app`
Expected: PASS (the app builds unchanged on the moved `ImportUnits`; its host keeps the defaults until Task 7).

- [ ] **Step 5: Check and commit**

No docs yet: the editor and `quit` are documented in Task 7, when the app answers them.

Run: `$HOME/.cargo/bin/cargo fmt --all --check && $HOME/.cargo/bin/cargo clippy --workspace --all-targets -- -D warnings && $HOME/.cargo/bin/cargo test --workspace`
Expected: clean; all pass.

```bash
git status && git diff
git add crates/api/src/editor.rs crates/api/src/dispatch.rs crates/api/src/lib.rs crates/api/tests/editor.rs crates/session/src/import.rs crates/session/src/lib.rs crates/session/src/document/files.rs crates/app/src/editor.rs
git commit -m "API: editor state query, editor commands and quit through the host"
```

---

### Task 6: Selection, camera and editor notifications; closing waits for replies

**Files:**
- Modify: `crates/api/src/hub.rs`, `crates/api/src/transport.rs`
- Test: `crates/api/tests/hub.rs`, `crates/api/tests/editor.rs`

**Interfaces:**
- Consumes: `Host::editor` (Task 5), `Query::Camera`.
- Produces:
  - `Hub::notify_state(&mut self, host: &dyn Host)`: sends `selection` (`{ids, revision}`), `camera` (the camera query's result without `type`) and `editor` (the state) notifications for those that changed since the last call, to subscribers only; the baseline is taken at the first `subscribe`.
  - `Hub::close(&mut self, wait: Duration) -> bool`: drops waiting requests and connections, waits for writer threads; true when all finished.
  - Crate-private: `Inbox::writing(&self) -> Writing` (a guard counted while a transport's writer thread runs), used by `serve_stream`.

- [ ] **Step 1: Write the failing tests**

```diff
--- a/crates/api/tests/hub.rs
+++ b/crates/api/tests/hub.rs
@@ -137,3 +137,87 @@ fn opening_a_file_tells_subscribers_to_read_the_model_again() {
     assert_eq!(note["method"], "resync", "{note}");
     assert_eq!(note["params"]["revision"], 2);
 }
+
+/// Every message waiting on `rx`.
+fn drain(rx: &Receiver<String>) -> Vec<Value> {
+    rx.try_iter()
+        .map(|l| serde_json::from_str(&l).unwrap())
+        .collect()
+}
+
+#[test]
+fn selection_and_camera_changes_are_notified_once_per_round() {
+    let mut host = HeadlessHost::new();
+    let mut hub = Hub::new(|| {});
+    let inbox = hub.inbox();
+    let (watcher, w_rx) = inbox.open();
+    let (writer, _r_rx) = inbox.open();
+    inbox.send_line(writer, request(1, "execute", edge(0.0)));
+    hub.process(&mut host);
+    // Nobody listens yet: nothing is kept or sent.
+    hub.notify_state(&host);
+
+    inbox.send_line(watcher, request(2, "subscribe", json!(null)));
+    hub.process(&mut host);
+    hub.notify_state(&host);
+    assert_eq!(drain(&w_rx).len(), 1, "only the subscribe reply");
+
+    let select = json!({"type": "select", "ids": ["e2"]});
+    let camera = |x: i32| json!({"type": "set_camera", "eye": [x, -10, 5], "target": [0, 0, 0]});
+    inbox.send_line(writer, request(3, "execute", select));
+    inbox.send_line(writer, request(4, "execute", camera(10)));
+    inbox.send_line(writer, request(5, "execute", camera(12)));
+    hub.process(&mut host);
+    hub.notify_state(&host);
+    let messages = drain(&w_rx);
+    let methods: Vec<&str> = messages
+        .iter()
+        .map(|m| m["method"].as_str().unwrap())
+        .collect();
+    assert_eq!(methods, vec!["selection", "camera"]);
+    assert_eq!(messages[0]["params"]["ids"], json!(["e2"]));
+    assert_eq!(messages[0]["params"]["revision"], 1);
+    assert_eq!(messages[1]["params"]["eye"], json!([12.0, -10.0, 5.0]));
+    assert!(messages[1]["params"]["type"].is_null());
+
+    // Changes made outside the API are notified the same way; nothing changed, nothing sent.
+    host.doc
+        .execute(session::Command::Select {
+            ids: Vec::new(),
+            mode: session::SelectMode::Replace,
+        })
+        .unwrap();
+    hub.notify_state(&host);
+    hub.notify_state(&host);
+    let messages = drain(&w_rx);
+    assert_eq!(messages.len(), 1);
+    assert_eq!(messages[0]["params"]["ids"], json!([]));
+}
+
+#[test]
+fn closing_waits_for_the_last_replies_to_be_written() {
+    use std::io::{BufRead, BufReader, Write};
+    use std::os::unix::net::UnixStream;
+    use std::time::Instant;
+
+    let mut host = HeadlessHost::new();
+    let mut hub = Hub::new(|| {});
+    let (served, client) = UnixStream::pair().unwrap();
+    api::serve_stream(hub.inbox(), served.try_clone().unwrap(), served);
+    writeln!(&client, "{}", request(1, "execute", edge(0.0))).unwrap();
+    // The request arrives from the reader thread; handling it bumps the revision.
+    let deadline = Instant::now() + Duration::from_secs(5);
+    while host.doc.revision() == 0 {
+        assert!(Instant::now() < deadline, "no request arrived");
+        hub.process(&mut host);
+        std::thread::sleep(Duration::from_millis(1));
+    }
+    assert!(hub.close(Duration::from_secs(5)));
+    client
+        .set_read_timeout(Some(Duration::from_millis(100)))
+        .unwrap();
+    let mut line = String::new();
+    BufReader::new(&client).read_line(&mut line).unwrap();
+    let reply: Value = serde_json::from_str(&line).unwrap();
+    assert_eq!(reply["result"]["revision"], 1);
+}
```

Append to `crates/api/tests/editor.rs`:

```diff
--- a/crates/api/tests/editor.rs
+++ b/crates/api/tests/editor.rs
@@ -189,3 +189,39 @@ fn describe_lists_quit_and_the_editor_schemas() {
         assert!(d["schemas"][name].is_object(), "{name}");
     }
 }
+
+#[test]
+fn subscribers_hear_when_the_editor_changes() {
+    use std::time::Duration;
+
+    let mut host = Editing::new();
+    let mut hub = api::Hub::new(|| {});
+    let inbox = hub.inbox();
+    let (conn, rx) = inbox.open();
+    let line = |id: u64, method: &str, params: Value| {
+        json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}).to_string()
+    };
+    inbox.send_line(conn, line(1, "subscribe", Value::Null));
+    hub.process(&mut host);
+    hub.notify_state(&host);
+    rx.recv_timeout(Duration::from_secs(1)).unwrap();
+    assert!(rx.try_recv().is_err());
+
+    // Changed by a request or by the program itself, it is told once per round.
+    inbox.send_line(
+        conn,
+        line(2, "execute", json!({"type": "set_tool", "tool": "move"})),
+    );
+    hub.process(&mut host);
+    host.state.import_units = ImportUnits::Feet;
+    hub.notify_state(&host);
+    let messages: Vec<Value> = rx
+        .try_iter()
+        .map(|l| serde_json::from_str(&l).unwrap())
+        .collect();
+    assert_eq!(messages.len(), 2, "{messages:?}");
+    assert_eq!(messages[0]["id"], 2);
+    assert_eq!(messages[1]["method"], "editor");
+    assert_eq!(messages[1]["params"]["tool"], "move");
+    assert_eq!(messages[1]["params"]["import_units"], "feet");
+}
```

- [ ] **Step 2: Run them to see them fail**

Run: `$HOME/.cargo/bin/cargo test -p api --test hub --test editor`
Expected: FAIL to compile: no method `notify_state` or `close` on `Hub`.

- [ ] **Step 3: Notify state and close**

```diff
--- a/crates/api/src/hub.rs
+++ b/crates/api/src/hub.rs
@@ -4,12 +4,15 @@
 //! that owns the document calls `Hub::process` on its own thread; that is the only place
 //! requests touch the document.
 
-use std::collections::{HashMap, HashSet};
-use std::sync::Arc;
+use std::collections::{BTreeSet, HashMap, HashSet};
 use std::sync::atomic::{AtomicU64, Ordering};
 use std::sync::mpsc::{Receiver, Sender, channel};
+use std::sync::{Arc, Condvar, Mutex};
+use std::time::Duration;
 
-use serde_json::json;
+use kernel::EntityId;
+use serde_json::{Value, json};
+use session::Query;
 
 use crate::dispatch::{Host, handle};
 use crate::protocol::{notification, parse, success};
@@ -27,6 +30,25 @@ pub struct Inbox {
     tx: Sender<Event>,
     next_conn: Arc<AtomicU64>,
     wake: Arc<dyn Fn() + Send + Sync>,
+    writers: Arc<Writers>,
+}
+
+/// How many transport threads are writing replies, so `Hub::close` can wait for them.
+#[derive(Default)]
+struct Writers {
+    running: Mutex<usize>,
+    done: Condvar,
+}
+
+/// Held by a transport's writer thread while it runs.
+pub(crate) struct Writing(Arc<Writers>);
+
+impl Drop for Writing {
+    fn drop(&mut self) {
+        let mut running = self.0.running.lock().unwrap_or_else(|e| e.into_inner());
+        *running -= 1;
+        self.0.done.notify_all();
+    }
 }
 
 impl Inbox {
@@ -51,6 +73,16 @@ impl Inbox {
         self.send(Event::Reply { conn, line });
     }
 
+    /// Counts a writer thread as running until the returned guard is dropped.
+    pub(crate) fn writing(&self) -> Writing {
+        *self
+            .writers
+            .running
+            .lock()
+            .unwrap_or_else(|e| e.into_inner()) += 1;
+        Writing(self.writers.clone())
+    }
+
     fn send(&self, event: Event) {
         if self.tx.send(event).is_ok() {
             (self.wake)();
@@ -65,6 +97,35 @@ pub struct Hub {
     subscribers: HashSet<u64>,
     /// Last revision subscribers were told about.
     notified: Option<u64>,
+    /// The selection, camera and editor subscribers were last told about (None while nobody
+    /// subscribes).
+    seen: Option<Seen>,
+}
+
+/// The state `selection`, `camera` and `editor` notifications report.
+struct Seen {
+    selection: BTreeSet<EntityId>,
+    /// The `camera` query's result without its `type`.
+    camera: Value,
+    editor: Option<Value>,
+}
+
+impl Seen {
+    fn of(host: &dyn Host) -> Seen {
+        let doc = host.document();
+        let mut camera = json!(
+            doc.query(&Query::Camera)
+                .expect("the camera query cannot fail")
+        );
+        if let Some(fields) = camera.as_object_mut() {
+            fields.remove("type");
+        }
+        Seen {
+            selection: doc.selection().clone(),
+            camera,
+            editor: host.editor().map(|e| json!(e)),
+        }
+    }
 }
 
 impl Hub {
@@ -78,10 +139,12 @@ impl Hub {
                 tx,
                 next_conn: Arc::new(AtomicU64::new(1)),
                 wake: Arc::new(wake),
+                writers: Arc::default(),
             },
             outs: HashMap::new(),
             subscribers: HashSet::new(),
             notified: None,
+            seen: None,
         }
     }
 
@@ -135,14 +198,69 @@ impl Hub {
                 .map(|e| notification("changed", json!(e)))
                 .collect()
         };
+        self.broadcast(&lines);
+        self.notified = Some(doc.revision());
+    }
+
+    /// Sends `selection`, `camera` and `editor` notifications for those that changed since
+    /// subscribers were last told, whoever changed them: at most one of each per call. Call it
+    /// once a frame (the window) or after each round of requests (headless).
+    pub fn notify_state(&mut self, host: &dyn Host) {
+        if self.subscribers.is_empty() {
+            self.seen = None;
+            return;
+        }
+        let now = Seen::of(host);
+        let Some(before) = self.seen.replace(now) else {
+            return;
+        };
+        let now = self.seen.as_ref().expect("just set");
+        let mut lines = Vec::new();
+        if now.selection != before.selection {
+            let ids: Vec<EntityId> = now.selection.iter().copied().collect();
+            let revision = host.document().revision();
+            lines.push(notification(
+                "selection",
+                json!({"ids": ids, "revision": revision}),
+            ));
+        }
+        if now.camera != before.camera {
+            lines.push(notification("camera", now.camera.clone()));
+        }
+        if now.editor != before.editor
+            && let Some(editor) = &now.editor
+        {
+            lines.push(notification("editor", editor.clone()));
+        }
+        self.broadcast(&lines);
+    }
+
+    /// Sends `lines` to every subscriber.
+    fn broadcast(&self, lines: &[String]) {
         for conn in &self.subscribers {
             if let Some(out) = self.outs.get(conn) {
-                for line in &lines {
+                for line in lines {
                     let _ = out.send(line.clone());
                 }
             }
         }
-        self.notified = Some(doc.revision());
+    }
+
+    /// Stops serving: drops requests not yet handled and every connection's sending side, so
+    /// each transport's writer thread ends once the lines queued for it are written, and waits
+    /// up to `wait` for them. True if they all finished. Call it before the program exits, so
+    /// the last replies (such as a `quit`'s) reach their clients.
+    pub fn close(&mut self, wait: Duration) -> bool {
+        while self.rx.try_recv().is_ok() {}
+        self.outs.clear();
+        self.subscribers.clear();
+        let writers = &self.inbox.writers;
+        let running = writers.running.lock().unwrap_or_else(|e| e.into_inner());
+        let (running, _) = writers
+            .done
+            .wait_timeout_while(running, wait, |n| *n > 0)
+            .unwrap_or_else(|e| e.into_inner());
+        *running == 0
     }
 
     fn on_event(&mut self, host: &mut dyn Host, event: Event) {
@@ -165,6 +283,9 @@ impl Hub {
                         // Notify about anything earlier before this connection joins.
                         self.notify(host);
                         self.subscribers.insert(conn);
+                        if self.seen.is_none() {
+                            self.seen = Some(Seen::of(host));
+                        }
                         req.id
                             .map(|id| success(&id, json!({"revision": host.document().revision()})))
                     }
```

```diff
--- a/crates/api/src/transport.rs
+++ b/crates/api/src/transport.rs
@@ -20,7 +20,9 @@ where
     W: Write + Send + 'static,
 {
     let (conn, replies) = inbox.open();
+    let writing = inbox.writing();
     let writer_thread = thread::spawn(move || {
+        let _writing = writing;
         for line in replies {
             if writer
                 .write_all(line.as_bytes())
```

- [ ] **Step 4: Run the tests**

Run: `$HOME/.cargo/bin/cargo test -p api`
Expected: PASS.

- [ ] **Step 5: Check and commit**

The notifications are documented in Task 7, with the editor they report.

Run: `$HOME/.cargo/bin/cargo fmt --all --check && $HOME/.cargo/bin/cargo clippy --workspace --all-targets -- -D warnings && $HOME/.cargo/bin/cargo test --workspace`
Expected: clean; all pass.

```bash
git status && git diff
git add crates/api/src/hub.rs crates/api/src/transport.rs crates/api/tests/hub.rs crates/api/tests/editor.rs
git commit -m "API: selection, camera and editor notifications; closing waits for the last replies"
```

---

### Task 7: The app's editor host, headless on the editor, quitting headless

**Files:**
- Create: `crates/app/src/editor/agent.rs` (with tests), `crates/app/tests/headless_agent.rs`
- Modify: `crates/app/src/editor.rs` (declares `agent`), `crates/app/src/host.rs` (rewritten, with tests), `crates/app/src/headless.rs` (rewritten), `crates/app/src/app.rs` (uses `AppHost` and `Offscreen`)
- Docs: `docs/agent-api.md`

**Interfaces:**
- Consumes: `Phase` (Task 2); `EditorState`, `EditorCommand`, `Host` additions (Task 5); `Hub::notify_state`, `Hub::close` (Task 6); `Document::check_unsaved`.
- Produces:
  - `Editor::phase(&self) -> Phase`, `Editor::typed(&self) -> &str`, `Editor::flying_speed(&self) -> f64`, `Editor::soften_state(&self) -> (u8, bool)` (angle, open), `Editor::set_paint_material(&mut self, Option<MaterialId>) -> Result<(), CommandError>` (keeps the tool; `not_found`), `Editor::cancel(&mut self)`.
  - `host::Offscreen` (`with(Snapshotter)`, `headless()`), `host::AppHost<'a> { editor: &'a mut Editor, offscreen: &'a mut Offscreen, quit: &'a mut bool }` implementing every `Host` method, `host::editor_state(&Editor) -> EditorState`.
  - `State.offscreen: Offscreen` (was `snapshotter`), `State.quit: bool` (acted on in Task 8).
  - Headless: `run` serves `AppHost` over an `Editor::new(1280 x 800, 1.0)`, calls `notify_state` after each round, stops on `quit`; socket mode then calls `Hub::close(CLOSE_WAIT)` and removes the socket file. `CLOSE_WAIT` (2 s) is in `headless.rs` here and moves to `host.rs` in Task 8.

- [ ] **Step 1: Write the failing end-to-end tests**

Create `crates/app/tests/headless_agent.rs`:

```rust
//! Runs the real binary headless and drives its editor the way an agent would, ending with
//! `quit` while the connection is still open.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

fn request(id: u64, method: &str, params: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})
}

/// Waits up to 30 seconds for `child` to exit on its own.
fn exits(child: &mut Child) -> ExitStatus {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return status;
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            panic!("the app did not quit within 30 seconds");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn an_agent_sets_the_editor_and_quits_over_stdio() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_app"))
        .args(["--headless", "--stdio"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("start app");
    let mut stdin = child.stdin.take().unwrap();
    let requests = [
        request(1, "subscribe", Value::Null),
        request(
            2,
            "execute",
            json!({"type": "set_tool", "tool": "rectangle"}),
        ),
        request(3, "query", json!({"type": "editor"})),
        request(
            4,
            "execute",
            json!({"type": "add_edge", "start": [0, 0, 0], "end": [1, 0, 0]}),
        ),
        request(5, "query", json!({"type": "history"})),
        request(6, "quit", Value::Null),
        request(7, "quit", json!({"discard": true})),
    ];
    for r in &requests {
        writeln!(stdin, "{r}").unwrap();
    }
    // Stdin stays open: only `quit` can end the app.
    let status = exits(&mut child);
    drop(stdin);
    assert!(status.success(), "exit status {status}");
    let messages: Vec<Value> = BufReader::new(child.stdout.take().unwrap())
        .lines()
        .map(|l| serde_json::from_str(&l.unwrap()).unwrap())
        .collect();
    let replies: HashMap<u64, &Value> = messages
        .iter()
        .filter_map(|m| m["id"].as_u64().map(|id| (id, m)))
        .collect();
    assert_eq!(replies.len(), 7, "{messages:?}");
    assert_eq!(replies[&2]["result"]["editor"]["tool"], "rectangle");
    let editor = &replies[&3]["result"];
    assert_eq!(editor["type"], "editor");
    assert_eq!(editor["phase"], "idle");
    assert_eq!(editor["viewport"], json!([1280.0, 800.0]));
    assert_eq!(replies[&5]["result"]["undo"], json!(["add_edge"]));
    assert_eq!(replies[&6]["error"]["data"]["kind"], "unsaved_changes");
    assert_eq!(replies[&7]["result"], json!({}));
    let tools: Vec<&Value> = messages
        .iter()
        .filter(|m| m["method"] == "editor")
        .map(|m| &m["params"]["tool"])
        .collect();
    assert_eq!(tools, vec!["rectangle"], "{messages:?}");
}

#[test]
fn quitting_over_a_socket_answers_and_removes_the_socket() {
    let dir = std::env::temp_dir().join(format!("skq-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("q.sock");
    let mut child = Command::new(env!("CARGO_BIN_EXE_app"))
        .args(["--headless", "--socket", path.to_str().unwrap()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("start app");
    let deadline = Instant::now() + Duration::from_secs(30);
    let stream = loop {
        if let Ok(s) = UnixStream::connect(&path) {
            break s;
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            panic!("the app never listened");
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    writeln!(&stream, "{}", request(1, "quit", Value::Null)).unwrap();
    let mut line = String::new();
    BufReader::new(&stream).read_line(&mut line).unwrap();
    let reply: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(reply["result"], json!({}), "{line}");
    assert!(exits(&mut child).success());
    assert!(!path.exists(), "the socket file is removed");
    std::fs::remove_dir_all(&dir).unwrap();
}
```

- [ ] **Step 2: Run them to see them fail**

Run: `$HOME/.cargo/bin/cargo test -p app --test headless_agent`
Expected: FAIL: `set_tool` and `quit` are refused (`this program has no editor`), and the stdio test times out waiting for the app to quit (it kills the app and panics).

- [ ] **Step 3: The Editor's agent methods**

Create `crates/app/src/editor/agent.rs`:

```rust
//! What agents read and set of the editor beyond the model (see `api::EditorState` and
//! `api::EditorCommand`).

use kernel::MaterialId;
use session::{CommandError, ErrorKind};
use tools::Phase;

use super::Editor;

impl Editor {
    /// Whether the tool is in an operation, and the points it was given so far.
    pub fn phase(&self) -> Phase {
        self.tool.phase()
    }

    /// The text typed into the measurements box, not yet submitted.
    pub fn typed(&self) -> &str {
        &self.typed
    }

    /// Fly mode's speed, meters a second.
    pub fn flying_speed(&self) -> f64 {
        self.fly.speed()
    }

    /// The Soften Edges window's angle in degrees, and whether the window is open.
    pub fn soften_state(&self) -> (u8, bool) {
        (self.soften.angle, self.soften.open)
    }

    /// Makes `material` (None: the default material) the one the Paint tool paints with,
    /// keeping the tool. `not_found` for a material the model does not have.
    pub fn set_paint_material(&mut self, material: Option<MaterialId>) -> Result<(), CommandError> {
        if let Some(m) = material
            && self.doc.scene().material(m).is_none()
        {
            return Err(CommandError::new(
                ErrorKind::NotFound,
                format!("no material {m}"),
            ));
        }
        self.material = material;
        Ok(())
    }

    /// Aborts the tool's operation, as Escape pressed until the tool is idle would, without
    /// closing the open group: drops typed text, the direction lock, a press the tool waits to
    /// see released, and the operation with anything it waits for (a copy array, a retyped
    /// angle).
    pub fn cancel(&mut self) {
        self.typed.clear();
        self.lock = None;
        self.shift_lock = false;
        self.tool_pressed = false;
        self.message = None;
        self.reset_tool();
        self.invalidate_hover();
    }
}

#[cfg(test)]
mod tests {
    use glam::{DVec2, DVec3};
    use session::Command;
    use tools::ToolKind;

    use super::*;
    use crate::editor::EditorKey;
    use crate::navigation::Button;

    fn editor() -> Editor {
        Editor::new(DVec2::new(800.0, 600.0), 1.0)
    }

    fn click_at(e: &mut Editor, p: DVec3) {
        let px = e.doc().camera.project(p).unwrap();
        e.cursor_moved(px);
        e.button(Button::Left, true);
        e.button(Button::Left, false);
    }

    #[test]
    fn the_phase_follows_the_tools_clicks() {
        let mut e = editor();
        assert_eq!(e.phase(), Phase::Idle);
        let a = DVec3::new(1.0, 1.0, 0.0);
        click_at(&mut e, a);
        let Phase::Active { points } = e.phase() else {
            panic!("a line is under way");
        };
        assert_eq!(points.len(), 1);
        assert!(points[0].distance(a) < 1e-9);
        e.key(EditorKey::Char('2'));
        assert_eq!(e.typed(), "2");
    }

    #[test]
    fn cancel_ends_the_operation_and_drops_typing_but_keeps_the_open_group() {
        let mut e = editor();
        e.execute(Command::AddRectangle {
            corner: [0.0, 0.0, 0.0],
            opposite: [2.0, 2.0, 0.0],
            normal: None,
        })
        .unwrap();
        let all: Vec<_> = e
            .doc()
            .model()
            .edges()
            .map(|(id, _)| kernel::EntityId::Edge(id))
            .chain(
                e.doc()
                    .model()
                    .faces()
                    .map(|(id, _)| kernel::EntityId::Face(id)),
            )
            .collect();
        let out = e.execute(Command::MakeGroup { ids: all }).unwrap();
        let group = out
            .changes
            .created
            .into_iter()
            .find(|id| matches!(id, kernel::EntityId::Instance(_)))
            .unwrap();
        e.execute(Command::OpenInstance { id: group }).unwrap();
        e.select_tool(ToolKind::Line);
        click_at(&mut e, DVec3::new(0.5, 0.5, 0.0));
        e.key(EditorKey::Char('3'));
        e.key(EditorKey::LockAxis(tools::Axis::X));
        assert!(matches!(e.phase(), Phase::Active { .. }));

        e.cancel();
        assert_eq!(e.phase(), Phase::Idle);
        assert_eq!(e.typed(), "");
        assert_eq!(e.status_parts().lock, None);
        assert_eq!(e.doc().context().len(), 1, "the group stays open");
        // Cancelling an idle tool changes nothing.
        e.cancel();
        assert_eq!(e.phase(), Phase::Idle);
    }

    #[test]
    fn the_paint_material_is_set_without_switching_tools() {
        let mut e = editor();
        let out = e
            .execute(Command::AddMaterial {
                name: Some("Brick".into()),
                color: Some([200, 80, 60]),
                opacity: None,
                emission: None,
                texture: None,
            })
            .unwrap();
        let brick = out.changes.materials[0];
        e.set_paint_material(Some(brick)).unwrap();
        assert_eq!(e.current_material(), Some(brick));
        assert_eq!(e.tool_kind(), ToolKind::Line);
        let missing = e.set_paint_material(Some(MaterialId(999))).unwrap_err();
        assert_eq!(missing.kind, ErrorKind::NotFound);
        assert_eq!(e.current_material(), Some(brick));
        e.set_paint_material(None).unwrap();
        assert_eq!(e.current_material(), None);
    }
}
```

```diff
--- a/crates/app/src/editor.rs
+++ b/crates/app/src/editor.rs
@@ -23,6 +23,7 @@ use tools::{
 use crate::fly::{self, Fly, FlyKey};
 use crate::navigation::{Button, LOGICAL_PX_PER_ZOOM_STEP, Navigation, Scroll};
 
+mod agent;
 mod soften;
 mod texture_position;
 
```

- [ ] **Step 4: The app host**

Replace `crates/app/src/host.rs` with:

```rust
//! The agent API's view of the app, in the window or headless: commands go through the Editor
//! (so the active tool stays consistent), editor commands set it, and `render_view` uses an
//! offscreen renderer.

use api::{Dialog, EditorCommand, EditorState, Host, PhaseName, ToolName};
use render::Snapshotter;
use session::{BatchError, Camera, Command, CommandError, Document, Lighting, Outcome};
use tools::Phase;

use crate::editor::Editor;

/// The offscreen renderer `render_view` uses: one on the window's GPU device, or, without a
/// window, one made on first use (None when there is no GPU).
pub struct Offscreen {
    snapshotter: Option<Snapshotter>,
    tried: bool,
}

impl Offscreen {
    /// `snapshotter`, already made.
    pub fn with(snapshotter: Snapshotter) -> Self {
        Offscreen {
            snapshotter: Some(snapshotter),
            tried: true,
        }
    }

    /// One made on its own GPU device when first needed.
    pub fn headless() -> Self {
        Offscreen {
            snapshotter: None,
            tried: false,
        }
    }

    fn get(&mut self) -> Option<&mut Snapshotter> {
        if !self.tried {
            self.tried = true;
            self.snapshotter = Snapshotter::new_headless();
        }
        self.snapshotter.as_mut()
    }
}

pub struct AppHost<'a> {
    pub editor: &'a mut Editor,
    pub offscreen: &'a mut Offscreen,
    /// Set when an agent asks the app to quit.
    pub quit: &'a mut bool,
}

impl Host for AppHost<'_> {
    fn document(&self) -> &Document {
        self.editor.doc()
    }

    fn execute(&mut self, command: Command) -> Result<Outcome, CommandError> {
        self.editor.execute(command)
    }

    fn execute_batch(
        &mut self,
        commands: Vec<Command>,
        atomic: bool,
    ) -> Result<Vec<Outcome>, BatchError> {
        self.editor.execute_batch(commands, atomic)
    }

    fn render_view(
        &mut self,
        camera: &Camera,
        lighting: Lighting,
        width: u32,
        height: u32,
    ) -> Option<Vec<u8>> {
        let doc = self.editor.doc();
        self.offscreen.get()?.capture(
            doc.scene(),
            doc.context(),
            doc.selection(),
            camera,
            lighting,
            width,
            height,
        )
    }

    fn editor(&self) -> Option<EditorState> {
        Some(editor_state(self.editor))
    }

    fn editor_command(&mut self, command: EditorCommand) -> Result<(), CommandError> {
        match command {
            EditorCommand::SetTool { tool } => self.editor.select_tool(tool.into()),
            EditorCommand::SetPaintMaterial { material } => {
                self.editor.set_paint_material(material)?
            }
            EditorCommand::SetImportUnits { units } => self.editor.set_import_units(units),
            EditorCommand::Cancel => self.editor.cancel(),
        }
        Ok(())
    }

    fn quit(&mut self, discard: bool) -> Result<(), CommandError> {
        self.editor.doc().check_unsaved(discard)?;
        *self.quit = true;
        Ok(())
    }
}

/// The editor's state as agents see it.
pub fn editor_state(editor: &Editor) -> EditorState {
    let status = editor.status_parts();
    let (phase, points) = match editor.phase() {
        Phase::Idle => (PhaseName::Idle, Vec::new()),
        Phase::Active { points } => (
            PhaseName::Active,
            points.iter().map(|p| p.to_array()).collect(),
        ),
    };
    let typed = editor.typed();
    let (soften_angle, soften_open) = editor.soften_state();
    let viewport = editor.doc().camera.viewport;
    EditorState {
        tool: ToolName::from(editor.tool_kind()),
        phase,
        points,
        paint_material: editor.current_material(),
        import_units: editor.import_units(),
        flying: editor.flying(),
        fly_speed: editor.flying_speed(),
        viewport: viewport.to_array(),
        status: status.to_string(),
        prompt: status.prompt.clone(),
        message: status.message.clone(),
        lock: status.lock.map(str::to_string),
        measurement_label: status.label.to_string(),
        measurement: if typed.is_empty() {
            status.value.clone()
        } else {
            Some(typed.to_string())
        },
        typing: !typed.is_empty(),
        soften_angle: soften_angle.into(),
        dialog: soften_open.then_some(Dialog::SoftenEdges),
    }
}

#[cfg(test)]
mod tests {
    use api::{ToolName, handle};
    use glam::{DVec2, DVec3};
    use serde_json::{Value, json};
    use session::ImportUnits;
    use tools::ToolKind;

    use super::*;
    use crate::editor::EditorKey;
    use crate::navigation::Button;

    fn editor() -> Editor {
        Editor::new(DVec2::new(800.0, 600.0), 1.0)
    }

    /// Sends one request through an `AppHost` and returns the reply.
    fn call(e: &mut Editor, quit: &mut bool, method: &str, params: Value) -> Value {
        let mut offscreen = Offscreen::headless();
        let mut host = AppHost {
            editor: e,
            offscreen: &mut offscreen,
            quit,
        };
        let line = json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params});
        serde_json::from_str(&handle(&mut host, &line.to_string()).unwrap()).unwrap()
    }

    #[test]
    fn the_state_shows_the_tool_its_points_and_the_measurements_box() {
        let mut e = editor();
        let a = DVec3::new(1.0, 1.0, 0.0);
        let px = e.doc().camera.project(a).unwrap();
        e.cursor_moved(px);
        e.button(Button::Left, true);
        e.button(Button::Left, false);
        e.key(EditorKey::Char('2'));
        let s = editor_state(&e);
        assert_eq!(s.tool, ToolName::Line);
        assert_eq!(s.phase, PhaseName::Active);
        assert_eq!(s.points.len(), 1);
        assert!(DVec3::from(s.points[0]).distance(a) < 1e-9);
        assert_eq!((s.typing, s.measurement.as_deref()), (true, Some("2")));
        assert_eq!(s.measurement_label, "Length");
        assert_eq!(s.prompt, "Click the next point, or type a length");
        assert!(s.status.starts_with("Line  |  "), "{}", s.status);
        assert_eq!(s.viewport, [800.0, 600.0]);
        assert_eq!((s.soften_angle, s.dialog), (20, None));
        e.show_soften(true);
        assert_eq!(editor_state(&e).dialog, Some(Dialog::SoftenEdges));
    }

    #[test]
    fn editor_commands_reach_the_editor() {
        let (mut e, mut quit) = (editor(), false);
        let r = call(
            &mut e,
            &mut quit,
            "execute",
            json!({"type": "set_tool", "tool": "tape_measure"}),
        );
        assert_eq!(r["result"]["editor"]["tool"], "tape_measure", "{r}");
        assert_eq!(e.tool_kind(), ToolKind::TapeMeasure);
        call(
            &mut e,
            &mut quit,
            "execute",
            json!({"type": "set_import_units", "units": "inches"}),
        );
        assert_eq!(e.import_units(), ImportUnits::Inches);
        let r = call(
            &mut e,
            &mut quit,
            "execute",
            json!({"type": "set_paint_material", "material": "m42"}),
        );
        assert_eq!(r["error"]["data"]["kind"], "not_found", "{r}");
        let r = call(&mut e, &mut quit, "query", json!({"type": "editor"}));
        assert_eq!(r["result"]["import_units"], "inches");
        assert!(!quit);
    }

    #[test]
    fn quit_is_refused_with_unsaved_changes_unless_discarded() {
        let (mut e, mut quit) = (editor(), false);
        e.execute(Command::AddEdge {
            start: [0.0; 3],
            end: [1.0, 0.0, 0.0],
        })
        .unwrap();
        let r = call(&mut e, &mut quit, "quit", Value::Null);
        assert_eq!(r["error"]["data"]["kind"], "unsaved_changes", "{r}");
        assert!(!quit);
        let r = call(&mut e, &mut quit, "quit", json!({"discard": true}));
        assert_eq!(r["result"], json!({}));
        assert!(quit);
    }
}
```

```diff
--- a/crates/app/src/app.rs
+++ b/crates/app/src/app.rs
@@ -23,7 +23,7 @@ use crate::awake::Awake;
 use crate::editor::{Editor, EditorKey};
 use crate::fly::FlyKey;
 use crate::gui::Gui;
-use crate::host::WindowHost;
+use crate::host::{AppHost, Offscreen};
 use crate::info::entity_info;
 use crate::navigation::{Button, Scroll};
 use crate::ui::{MaterialRow, TexturePosition, TextureRow, UiAction, View};
@@ -401,8 +401,11 @@ struct State {
     /// skips is logged once.
     skip_logged: Option<&'static str>,
     renderer: Renderer,
-    snapshotter: Snapshotter,
+    /// What `render_view` renders with, on the window's device.
+    offscreen: Offscreen,
     editor: Editor,
+    /// An agent asked the app to quit.
+    quit: bool,
     gui: Gui,
     /// Document revision, selection and open context the renderer's model buffers were built
     /// from.
@@ -453,7 +456,7 @@ impl State {
         let renderer = Renderer::new(&device, &queue, config.format);
         let gui = Gui::new(&window, &device, config.format);
         let view = Viewport::full((config.width, config.height));
-        let snapshotter = Snapshotter::with_device(&device, &queue);
+        let offscreen = Offscreen::with(Snapshotter::with_device(&device, &queue));
         let editor = Editor::new(
             DVec2::new(config.width as f64, config.height as f64),
             window.scale_factor(),
@@ -468,8 +471,9 @@ impl State {
             device_lost: lost,
             skip_logged: None,
             renderer,
-            snapshotter,
+            offscreen,
             editor,
+            quit: false,
             gui,
             drawn: None,
             outliner: None,
@@ -846,7 +850,7 @@ impl State {
         };
         eprintln!("GPU device made again");
         self.renderer = Renderer::new(&device, &queue, config.format);
-        self.snapshotter = Snapshotter::with_device(&device, &queue);
+        self.offscreen = Offscreen::with(Snapshotter::with_device(&device, &queue));
         self.gui = Gui::new(&self.window, &device, config.format);
         // The swatches belonged to the old panels' context.
         self.swatches.clear();
@@ -1051,9 +1055,10 @@ impl State {
             self.editor.doc().camera.clone(),
             self.editor.doc().lighting,
         );
-        let mut host = WindowHost {
+        let mut host = AppHost {
             editor: &mut self.editor,
-            snapshotter: &mut self.snapshotter,
+            offscreen: &mut self.offscreen,
+            quit: &mut self.quit,
         };
         hub.process(&mut host);
         let after = (
@@ -1073,9 +1078,10 @@ impl State {
 
     /// Tells API subscribers about edits made in the window.
     fn notify_api(&mut self, hub: &mut Hub) {
-        let host = WindowHost {
+        let host = AppHost {
             editor: &mut self.editor,
-            snapshotter: &mut self.snapshotter,
+            offscreen: &mut self.offscreen,
+            quit: &mut self.quit,
         };
         hub.notify(&host);
     }
```

- [ ] **Step 5: Headless on the editor**

Replace `crates/app/src/headless.rs` with:

```rust
//! Running without a window: the window's editor and the agent API on stdio or a Unix socket.

use std::path::PathBuf;
use std::time::Duration;

use api::{Hub, listen_unix, serve_stdio};
use glam::DVec2;
use session::Command;

use crate::cli::Transport;
use crate::editor::Editor;
use crate::host::{AppHost, Offscreen};

/// The size of the view headless, in pixels: what `pick` and `infer` use without one given.
const VIEWPORT: DVec2 = DVec2::new(1280.0, 800.0);

/// How long a quitting app waits for its last replies to be written.
pub const CLOSE_WAIT: Duration = Duration::from_secs(2);

/// Opens `file` if given, then serves until stdin closes (stdio), forever (socket), or until
/// an agent quits. Returns the process exit code.
pub fn run(transport: Transport, file: Option<PathBuf>) -> i32 {
    let mut editor = Editor::new(VIEWPORT, 1.0);
    if let Some(path) = file {
        let open = Command::Open {
            path: path.display().to_string(),
            // A fresh model: nothing to lose.
            discard: false,
        };
        if let Err(e) = editor.execute(open) {
            eprintln!("cannot open {}", e.message);
            return 1;
        }
    }
    let mut offscreen = Offscreen::headless();
    let mut hub = Hub::new(|| {});
    match transport {
        Transport::Stdio => {
            let writer = serve_stdio(hub.inbox());
            serve(&mut hub, &mut editor, &mut offscreen, |hub| {
                hub.open_connections() > 0
            });
            // Dropping the hub ends the writer once every queued reply is on stdout.
            drop(hub);
            let _ = writer.join();
            0
        }
        Transport::Socket(path) => {
            if let Err(e) = listen_unix(&path, hub.inbox()) {
                eprintln!("cannot listen on {}: {e}", path.display());
                return 1;
            }
            eprintln!("agent API listening on {}", path.display());
            serve(&mut hub, &mut editor, &mut offscreen, |_| true);
            hub.close(CLOSE_WAIT);
            let _ = std::fs::remove_file(&path);
            0
        }
    }
}

/// Handles requests as they come, round by round, until `go_on` says to stop or an agent
/// quits. After each round, subscribers hear about selection, camera and editor changes.
fn serve(
    hub: &mut Hub,
    editor: &mut Editor,
    offscreen: &mut Offscreen,
    go_on: impl Fn(&Hub) -> bool,
) {
    let mut quit = false;
    loop {
        let mut host = AppHost {
            editor: &mut *editor,
            offscreen: &mut *offscreen,
            quit: &mut quit,
        };
        if !hub.process_blocking(&mut host) {
            return;
        }
        hub.notify_state(&host);
        if quit || !go_on(hub) {
            return;
        }
    }
}
```

- [ ] **Step 6: Run the tests**

Run: `$HOME/.cargo/bin/cargo test -p app`
Expected: PASS: the three `editor::agent` tests, the three `host` tests and both `headless_agent` tests, plus the existing ones (`headless_stdio` still passes: closing stdin ends the app).

- [ ] **Step 7: Document the editor, `quit` and the notifications**

First the `quit` row of the Methods table, the `editor` row of the Queries table and the Editor section before Files:

```diff
--- a/docs/agent-api.md
+++ b/docs/agent-api.md
@@ -37,6 +37,7 @@ all commands and results.
 | `render_view` | `{"width": 640, "height": 400, "camera": {"eye": [...], "target": [...]}}` (camera optional; it also takes `fov_y_deg`, `up` and `projection` as `set_camera` does; `lighting`, `day` or `night`, is optional and defaults to the document's) | `{width, height, png_base64}` |
 | `subscribe` | none | `{revision}`, then `changed` notifications after every model change; a `resync` notification (`{revision}`) means changes were missed, so re-read with `{"type": "entities"}` (its `revision` tells you where you are) |
 | `describe` | none | the API description |
+| `quit` | `{"discard": true}` (optional; default false) | `{}`, then the app ends once its replies are sent (window or headless; a headless socket's file is removed). With unsaved changes and no `discard` it fails with `unsaved_changes` and goes on. In the window it ends without asking about unsaved changes |
 
 Queries:
 
@@ -60,6 +61,7 @@ Queries:
 | `{"type": "events", "since": 3}` | model changes after revision 3, oldest first |
 | `{"type": "pick", "pixel": [320, 200], "width": 640, "height": 400}` | what lies under a pixel of a view, or along a `ray`: `hits`, front to back; see Picking and inference below |
 | `{"type": "infer", "pixel": [320, 200]}` | what a tool's click there would snap to: `kind`, `point` and the `entities` involved; see Picking and inference below |
+| `{"type": "editor"}` | the editor around the model: tool, its phase and points, paint material, import units, fly mode, view size, status bar, measurements box, open window; see Editor below |
 
 Unknown fields in commands, queries and params are rejected with `-32602` (a typo fails instead
 of silently doing nothing).
@@ -563,6 +565,55 @@ intersection, on an edge, on a guide line, along an axis from `from`, on a face,
 - `radius` is how far from the pixel to snap, in pixels (default 8, the window's on a display of
   scale 1; at most 1000).
 
+### Editor
+
+The editor is what surrounds the model in the window (and in headless mode, which runs the same
+editor without one): the active tool and what it is doing, the material the Paint tool paints
+with, the units imports are read in, and the status bar. `{"type": "editor"}` returns:
+
+```
+{"type": "editor", "tool": "line", "phase": "active", "points": [[0,0,0]],
+ "paint_material": null, "import_units": "meters", "flying": false, "fly_speed": 3.0,
+ "viewport": [1280.0, 800.0], "status": "Line  |  Click the next point, or type a length  |  Length: 2 m",
+ "prompt": "Click the next point, or type a length", "message": null, "lock": null,
+ "measurement_label": "Length", "measurement": "2 m", "typing": false, "soften_angle": 20,
+ "dialog": null}
+```
+
+- `tool` is `select`, `line`, `rectangle`, `circle`, `arc`, `push_pull`, `move`, `rotate`,
+  `tape_measure`, `erase` or `paint`.
+- `phase` is `idle` (the next click starts an operation) or `active`, and `points` are the
+  points the operation was given so far, in the open context's coordinates: Line, the chain's
+  first point and the point the next edge starts from; Rectangle, the first corner; Circle, the
+  center; Arc, the start, then the end; Push/Pull, where the face was clicked; Move, the base
+  point; Rotate, the center, then where the angle starts; Tape Measure, where the measurement
+  starts. The Select tool is `active` with no points while its button is held (a click or a box,
+  both on screen); Erase and Paint are always `idle`.
+- `paint_material` is the material the Paint tool paints with (`null`: the default material);
+  `import_units` is what one unit of an imported OBJ or STL file is in the window's File > Import
+  (`meters`, `centimeters`, `millimeters`, `inches` or `feet`; the `import` command takes its own
+  `scale`).
+- `flying` and `fly_speed` (meters a second) are fly mode's (see Camera).
+- `viewport` is the window's 3D view in pixels (1280 x 800 headless): the view `pick` and
+  `infer` use when not given a size.
+- `status` is the status bar's line as the window shows it; `prompt`, `message` (feedback from
+  the last tool action or failed command), `lock` (e.g. `Locked: red axis`) are its parts.
+  `measurement_label` and `measurement` are the measurements box: the typed text while
+  `typing`, else the live measurement (`null` when there is none).
+- `soften_angle` is the Soften Edges window's angle in degrees, and `dialog` the window open
+  over the view: `soften_edges` or `null`. The window's file and unsaved-changes dialogs are
+  modal: while one is open the window handles no requests, which wait until it closes.
+
+Editor commands go through `execute` (not `batch`), are not undo steps and do not change the
+revision; each returns `{"editor": {...}, "revision": n}`, the state after it:
+
+| Command | Effect |
+|---|---|
+| `{"type": "set_tool", "tool": "rectangle"}` | Switches to the tool (a fresh one, even if it is the active tool), dropping the operation in progress and anything typed, as the toolbar does. |
+| `{"type": "set_paint_material", "material": "m6"}` | The material the Paint tool paints with; `null` or absent for the default material. The tool does not change (the window's Materials list also switches to Paint). An unknown material is `not_found`. |
+| `{"type": "set_import_units", "units": "millimeters"}` | What one unit of an imported file is, for the window's File > Import, for the rest of the session. |
+| `{"type": "cancel"}` | Aborts the tool's operation, like pressing Escape until the tool is idle: drops typed text, the direction lock and the operation (and a copy array or a retyped angle the tool waits for). It never closes the open group (`close_instance` does). |
+
 ### Files
 
 Models are saved as `.skr` files: a zip archive holding `document.json`, which records `format`
```

Then the `subscribe` row and the Notifications section before "Behaviors worth knowing" (these hunks are against the file with the first ones applied):

```diff
--- a/docs/agent-api.md
+++ b/docs/agent-api.md
@@ -35,7 +35,7 @@ all commands and results.
 | `batch` | `{"commands": [...], "atomic": true}` | `{outcomes, revision}`; atomic = one undo step, all or nothing |
 | `query` | a Query, see the table below | the QueryResult |
 | `render_view` | `{"width": 640, "height": 400, "camera": {"eye": [...], "target": [...]}}` (camera optional; it also takes `fov_y_deg`, `up` and `projection` as `set_camera` does; `lighting`, `day` or `night`, is optional and defaults to the document's) | `{width, height, png_base64}` |
-| `subscribe` | none | `{revision}`, then `changed` notifications after every model change; a `resync` notification (`{revision}`) means changes were missed, so re-read with `{"type": "entities"}` (its `revision` tells you where you are) |
+| `subscribe` | none | `{revision}`, then `changed` notifications after every model change; a `resync` notification (`{revision}`) means changes were missed, so re-read with `{"type": "entities"}` (its `revision` tells you where you are). Also `selection`, `camera` and `editor` notifications; see Notifications below |
 | `describe` | none | the API description |
 | `quit` | `{"discard": true}` (optional; default false) | `{}`, then the app ends once its replies are sent (window or headless; a headless socket's file is removed). With unsaved changes and no `discard` it fails with `unsaved_changes` and goes on. In the window it ends without asking about unsaved changes |
 
@@ -755,6 +755,26 @@ swapped for a pipe at that very instant could still block (local only, low risk)
 have at most 1,000,000 polygon corners in all (OBJ `f` corners, STL vertices) and ASCII STL keywords are read in any case;
 vertices left without an `endloop` are a `file` error.
 
+### Notifications
+
+After `subscribe`, a connection is told about changes without asking, whoever made them (an
+agent or the person at the window):
+
+| Notification | Params | When |
+|---|---|---|
+| `changed` | `{revision, changes}` | after every model change, in order (see Methods) |
+| `resync` | `{revision}` | changes were missed: read the model again |
+| `selection` | `{ids, revision}`, as the `selection` query returns | the selection changed |
+| `camera` | the `camera` query's result (without `type`) | the camera or the lighting changed: navigating in the window, fly mode, camera commands |
+| `editor` | the `editor` query's result (without `type`) | anything in it changed: the tool, its phase or points, the status bar or measurements box (which follow the cursor in the window), the paint material, import units, fly mode, the view's size, the Soften Edges window |
+
+`selection`, `camera` and `editor` are coalesced: at most one of each per frame the window draws
+(so a drag sends one per frame, not one per mouse event), or per round of requests in headless
+mode, each carrying the state at that moment; a change undone within the same frame sends
+nothing. In the window they follow the frame after the change, so they come after the replies
+and `changed` notifications of the requests that caused them; a window that draws no frames
+(minimized, or hidden on another desktop) sends them when it draws again.
+
 Behaviors worth knowing:
 
 - Replies come before the `changed` notifications they cause, and your own edits are notified too.
```

- [ ] **Step 8: Check and commit**

Run: `$HOME/.cargo/bin/cargo fmt --all --check && $HOME/.cargo/bin/cargo clippy --workspace --all-targets -- -D warnings && $HOME/.cargo/bin/cargo test --workspace`
Expected: clean; all pass.

```bash
git status && git diff
git add crates/app/src/editor/agent.rs crates/app/src/editor.rs crates/app/src/host.rs crates/app/src/headless.rs crates/app/src/app.rs crates/app/tests/headless_agent.rs docs/agent-api.md
git commit -m "App: agents read and set the editor; headless runs the editor and quits on request"
```

---

### Task 8: The window: per-frame notifications, quitting, the window actions list

**Files:**
- Modify: `crates/app/src/app.rs` (with a test), `crates/app/src/host.rs` (`CLOSE_WAIT` moves here), `crates/app/src/headless.rs`
- Docs: `docs/agent-api.md`, `docs/superpowers/specs/2026-10-10-tenon-design.md`

**Interfaces:**
- Consumes: everything above.
- Produces:
  - `host::CLOSE_WAIT: Duration` (2 s).
  - `app.rs`: `type Shown` and `fn shown(&Editor) -> Shown` (revision, selection, context, camera, lighting, editor state): `process_api` asks for a frame when it changes; `State::notify_frame(&mut self, hub)` calls `Hub::notify_state` once per `RedrawRequested`; `user_event` and `resumed` exit the event loop when `State.quit` is set; `exiting` calls `hub.close(CLOSE_WAIT)` before removing the socket.

- [ ] **Step 1: Write the failing test**

In `crates/app/src/app.rs`'s test module, before `only_a_ctrl_tap_switches_copying` (it is also in the Step 3 diff):

```rust
    #[test]
    fn a_request_that_changes_only_the_editor_still_shows() {
        let mut e = Editor::new(DVec2::new(800.0, 600.0), 1.0);
        let before = shown(&e);
        assert_eq!(shown(&e), before);
        e.set_import_units(crate::editor::ImportUnits::Feet);
        assert_ne!(shown(&e), before);
    }
```

- [ ] **Step 2: Run it to see it fail**

Run: `$HOME/.cargo/bin/cargo test -p app --bin app a_request_that`
Expected: FAIL to compile: no function `shown`.

- [ ] **Step 3: Wire the window**

```diff
--- a/crates/app/src/host.rs
+++ b/crates/app/src/host.rs
@@ -2,6 +2,8 @@
 //! (so the active tool stays consistent), editor commands set it, and `render_view` uses an
 //! offscreen renderer.
 
+use std::time::Duration;
+
 use api::{Dialog, EditorCommand, EditorState, Host, PhaseName, ToolName};
 use render::Snapshotter;
 use session::{BatchError, Camera, Command, CommandError, Document, Lighting, Outcome};
@@ -9,6 +11,9 @@ use tools::Phase;
 
 use crate::editor::Editor;
 
+/// How long a quitting app waits for its last replies to be written.
+pub const CLOSE_WAIT: Duration = Duration::from_secs(2);
+
 /// The offscreen renderer `render_view` uses: one on the window's GPU device, or, without a
 /// window, one made on first use (None when there is no GPU).
 pub struct Offscreen {
```

```diff
--- a/crates/app/src/headless.rs
+++ b/crates/app/src/headless.rs
@@ -1,7 +1,6 @@
 //! Running without a window: the window's editor and the agent API on stdio or a Unix socket.
 
 use std::path::PathBuf;
-use std::time::Duration;
 
 use api::{Hub, listen_unix, serve_stdio};
 use glam::DVec2;
@@ -9,14 +8,11 @@ use session::Command;
 
 use crate::cli::Transport;
 use crate::editor::Editor;
-use crate::host::{AppHost, Offscreen};
+use crate::host::{AppHost, CLOSE_WAIT, Offscreen};
 
 /// The size of the view headless, in pixels: what `pick` and `infer` use without one given.
 const VIEWPORT: DVec2 = DVec2::new(1280.0, 800.0);
 
-/// How long a quitting app waits for its last replies to be written.
-pub const CLOSE_WAIT: Duration = Duration::from_secs(2);
-
 /// Opens `file` if given, then serves until stdin closes (stdio), forever (socket), or until
 /// an agent quits. Returns the process exit code.
 pub fn run(transport: Transport, file: Option<PathBuf>) -> i32 {
```

```diff
--- a/crates/app/src/app.rs
+++ b/crates/app/src/app.rs
@@ -4,11 +4,11 @@ use std::sync::Arc;
 use std::sync::atomic::{AtomicBool, Ordering};
 use std::time::{Duration, Instant};
 
-use api::{Hub, listen_unix};
+use api::{EditorState, Hub, listen_unix};
 use glam::DVec2;
 use kernel::{EntityId, InstanceId, MaterialId, TextureImage};
 use render::{Renderer, Snapshotter, Viewport};
-use session::{Command, OutlinerTable};
+use session::{Camera, Command, Lighting, OutlinerTable};
 use tools::Axis;
 use winit::application::ApplicationHandler;
 use winit::dpi::LogicalSize;
@@ -23,7 +23,7 @@ use crate::awake::Awake;
 use crate::editor::{Editor, EditorKey};
 use crate::fly::FlyKey;
 use crate::gui::Gui;
-use crate::host::{AppHost, Offscreen};
+use crate::host::{AppHost, CLOSE_WAIT, Offscreen, editor_state};
 use crate::info::entity_info;
 use crate::navigation::{Button, Scroll};
 use crate::ui::{MaterialRow, TexturePosition, TextureRow, UiAction, View};
@@ -48,6 +48,30 @@ const HOVER_SETTLE: Duration = Duration::from_millis(50);
 /// The document revision, selection and open context the renderer's model buffers show.
 type Drawn = (u64, BTreeSet<EntityId>, Vec<InstanceId>);
 
+/// What the window shows that a request can change: the model (its revision), the selection,
+/// the open context, the camera, the lighting and the editor's state.
+type Shown = (
+    u64,
+    BTreeSet<EntityId>,
+    Vec<InstanceId>,
+    Camera,
+    Lighting,
+    EditorState,
+);
+
+/// What `editor`'s window shows that a request can change.
+fn shown(editor: &Editor) -> Shown {
+    let doc = editor.doc();
+    (
+        doc.revision(),
+        doc.selection().clone(),
+        doc.context().to_vec(),
+        doc.camera.clone(),
+        doc.lighting,
+        editor_state(editor),
+    )
+}
+
 /// What the renderer must redo before drawing a frame.
 #[derive(Debug, PartialEq)]
 enum SceneUpdate {
@@ -1045,30 +1069,17 @@ impl State {
     }
 
     /// Handles waiting API requests, answering each at once. If they changed the model,
-    /// selection, open context or camera, or may have changed the hover, a frame shows it:
-    /// at most about 30 a second while requests keep coming (see `api_repaint`).
+    /// selection, open context, camera or editor, or may have changed the hover, a frame shows
+    /// it: at most about 30 a second while requests keep coming (see `api_repaint`).
     fn process_api(&mut self, hub: &mut Hub) {
-        let before = (
-            self.editor.doc().revision(),
-            self.editor.doc().selection().clone(),
-            self.editor.doc().context().to_vec(),
-            self.editor.doc().camera.clone(),
-            self.editor.doc().lighting,
-        );
+        let before = shown(&self.editor);
         let mut host = AppHost {
             editor: &mut self.editor,
             offscreen: &mut self.offscreen,
             quit: &mut self.quit,
         };
         hub.process(&mut host);
-        let after = (
-            self.editor.doc().revision(),
-            self.editor.doc().selection().clone(),
-            self.editor.doc().context().to_vec(),
-            self.editor.doc().camera.clone(),
-            self.editor.doc().lighting,
-        );
-        if after != before || self.editor.hover_pending() {
+        if shown(&self.editor) != before || self.editor.hover_pending() {
             match api_repaint(Instant::now(), self.presented) {
                 Repaint::At(t) => self.repaint_at = Some(earliest(self.repaint_at, t)),
                 _ => self.window.request_redraw(),
@@ -1085,6 +1096,17 @@ impl State {
         };
         hub.notify(&host);
     }
+
+    /// Tells API subscribers what changed of the selection, camera and editor since the last
+    /// frame, once per frame drawn.
+    fn notify_frame(&mut self, hub: &mut Hub) {
+        let host = AppHost {
+            editor: &mut self.editor,
+            offscreen: &mut self.offscreen,
+            quit: &mut self.quit,
+        };
+        hub.notify_state(&host);
+    }
 }
 
 /// The letter of a Cmd/Ctrl shortcut: the layout's character when it is ASCII, else the
@@ -1254,6 +1276,8 @@ impl App {
 
 impl ApplicationHandler<ApiWake> for App {
     fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
+        // The last replies (a `quit`'s) reach their agents before the app ends.
+        self.hub.close(CLOSE_WAIT);
         if let Some(path) = self.socket.take() {
             let _ = std::fs::remove_file(path);
         }
@@ -1274,7 +1298,7 @@ impl ApplicationHandler<ApiWake> for App {
         }
     }
 
-    fn user_event(&mut self, _event_loop: &ActiveEventLoop, _event: ApiWake) {
+    fn user_event(&mut self, event_loop: &ActiveEventLoop, _event: ApiWake) {
         // Wake before the work, not after: a napped app runs a request (a save, say) many
         // times slower, and an agent's first request often arrives with its connection.
         let state = &mut self.state;
@@ -1285,6 +1309,10 @@ impl ApplicationHandler<ApiWake> for App {
             }
             hub.open_connections() > 0
         });
+        // An agent asked to quit; any unsaved changes were refused or discarded already.
+        if self.state.as_ref().is_some_and(|s| s.quit) {
+            event_loop.exit();
+        }
     }
 
     fn resumed(&mut self, event_loop: &ActiveEventLoop) {
@@ -1309,6 +1337,9 @@ impl ApplicationHandler<ApiWake> for App {
         state.window.request_redraw();
         // Requests that arrived before the window existed.
         state.process_api(&mut self.hub);
+        if state.quit {
+            event_loop.exit();
+        }
         self.state = Some(state);
     }
 
@@ -1349,6 +1380,7 @@ impl ApplicationHandler<ApiWake> for App {
                 if s.render() {
                     s.notify_api(&mut self.hub);
                 }
+                s.notify_frame(&mut self.hub);
                 false
             }
             WindowEvent::ModifiersChanged(m) => {
@@ -1513,6 +1545,15 @@ mod tests {
     use super::*;
     use winit::keyboard::SmolStr;
 
+    #[test]
+    fn a_request_that_changes_only_the_editor_still_shows() {
+        let mut e = Editor::new(DVec2::new(800.0, 600.0), 1.0);
+        let before = shown(&e);
+        assert_eq!(shown(&e), before);
+        e.set_import_units(crate::editor::ImportUnits::Feet);
+        assert_ne!(shown(&e), before);
+    }
+
     #[test]
     fn only_a_ctrl_tap_switches_copying() {
         let none = ModifiersState::empty();
```

- [ ] **Step 4: Run the tests**

Run: `$HOME/.cargo/bin/cargo test -p app`
Expected: PASS. The window itself is not launched: its wiring is the same calls the headless tests exercise (`process`, `notify_state`, `close`), placed per frame and on exit.

- [ ] **Step 5: List the window actions, and say so in the spec**

```diff
--- a/docs/agent-api.md
+++ b/docs/agent-api.md
@@ -786,6 +786,43 @@ Behaviors worth knowing:
 - A failed atomic batch is rolled back but still consumes entity ids, so later ids may skip numbers.
 - An `id` that is not a string, number or null is an invalid request (`-32600`, reply id null).
 
+## Window actions and the API
+
+Everything a person can do in the window is either a command or query here, or window chrome. The
+tools turn clicks and typing into the same commands with explicit values (the status bar's
+measurements become `distance`, `offset`, `angle` and so on), and `pick` and `infer` say what lies
+under a pixel and where a click would snap.
+
+| In the window | Through the API |
+|---|---|
+| File > New, Open..., Save, Save As... (Cmd+N, Cmd+O, Cmd+S, Shift+Cmd+S), the unsaved-changes prompt | `new`, `open` (`discard`), `save` (`path`); the prompt is `unsaved_changes` |
+| File > Import... (Cmd+I), File > Import Units | `import` (`scale`, `as`, `up`, `merge_coplanar`); `set_import_units`, `editor.import_units` |
+| File > Export... (Shift+Cmd+E) | `export` (`format`) |
+| Quit (Cmd+Q, closing the window) | `quit` (`discard`) |
+| Edit > Erase Selection (Delete), Edit > Delete Guides | `erase` with the `selection` query's ids; `delete_guides` |
+| Edit > Soften Edges...: angle, Soften, Harden; Entity Info's Soft box | `soften_by_angle` (`max_angle_deg`), `soften_edges` (`soft`); the window's angle and whether it is open: `editor.soften_angle`, `editor.dialog` |
+| Undo, Redo (Cmd+Z, Shift+Cmd+Z) | `undo`, `redo`; `history` lists the steps |
+| Make group (G), Make component (Shift+G), Explode, Close group (Escape) | `make_group`, `make_component` (`name`), `explode`, `close_instance` |
+| Camera menu: Zoom Extents (Shift+Z), standard views, Perspective / Parallel, Night lighting | `zoom_extents`, `standard_view`, `set_projection`, `set_lighting` |
+| Orbit, pan and zoom with the mouse or trackpad, fly mode (F) | `set_camera`, `orbit`, `pan`, `zoom` reach the same views; fly mode itself is the window's (`editor.flying`, `editor.fly_speed`) |
+| The toolbar's tools and their keys | `set_tool`; `editor.tool`, `editor.phase`, `editor.points` |
+| Select: click, Shift-click, box (inside or crossing), double-click to open a group | `select` (`mode`), `pick` for what is under a pixel, `entities` with `within` for a box, `open_instance` / `open_path` |
+| Line, Rectangle, Circle, Arc, Push/Pull, Move (Ctrl copies, `5x`, `/5`), Rotate, Tape Measure, Erase, Paint (Cmd samples) | `add_edge`, `add_rectangle`, `add_circle` (`segments`), `add_arc`, `push_pull`, `move`, `copy` (`count`), `rotate` (`copies`), `add_guide_line` / `add_guide_point`, `erase`, `paint` (`side`); a sampled material is the `entity` query's |
+| Inference snapping, Shift and arrow-key locks, the marker under the cursor | `infer` (with `from`; `axis` is what Shift would lock) |
+| Typing into the measurements box, Escape | the commands' explicit values; `cancel`; `editor.measurement`, `editor.typing` |
+| The status bar | `editor.status` (and `prompt`, `message`, `lock`) |
+| Materials list: choose (for Paint), Add, Delete, name, color, opacity, Glow, Texture..., Remove, texture size | `set_paint_material`, `add_material`, `delete_material`, `change_material` (`name`, `color`, `opacity`, `emission`, `texture`); `materials` |
+| Entity Info: what is selected, texture Offset / Rotation / Scale, Rotate 90, Reset | `entity`, `entities`, `curve`; `position_texture` (`offset`, `rotation_deg`, `scale`, `reset`) |
+| Outliner: the tree, click to select, Shift/Cmd-click to toggle, double-click to open, the Model row | `outliner`; `open_path` then `select` (`toggle`); `open_path` (`[]` for the root) |
+| The window's title (file name, unsaved mark) | `file` |
+| The 3D view's size | `editor.viewport` (the default view of `pick` and `infer`); `render_view` takes any size |
+
+Window chrome, which changes nothing in the model, its view or the editor, has no command: the
+window's size and position, the panels' sizes and scroll positions, which menus are open,
+tooltips, which Outliner rows are expanded, the export dialog's suggested format, and keyboard
+focus. The file dialogs are chrome too (commands take paths); while one, or the unsaved-changes
+prompt, is open, the window answers no requests.
+
 ## Example
 
 ```
```

```diff
--- a/docs/superpowers/specs/2026-10-10-tenon-design.md
+++ b/docs/superpowers/specs/2026-10-10-tenon-design.md
@@ -7,7 +7,7 @@ Status: Approved; amended 2026-10-10 (agent control, M2.5; M3/M4 scope); 2026-10
 
 A desktop 3D modeler in Rust with SketchUp-style direct modeling: draw edges, faces form automatically, push/pull faces, inference snapping, groups and components. macOS first; Windows and Linux supported through wgpu without dedicated work.
 
-The app is fully controllable by agents: every action a user can take is a serializable command available through a local API, the model can be queried as structured data, and the app can run headless (see section 5).
+The app is fully controllable by agents: every action a user can take is a serializable command available through a local API (except window chrome such as panel sizes and open menus, listed in `docs/agent-api.md`), the model can be queried as structured data, and the app can run headless (see section 5).
 
 ## Non-goals (v1)
 
```

- [ ] **Step 6: Check and commit**

Run: `$HOME/.cargo/bin/cargo fmt --all --check && $HOME/.cargo/bin/cargo clippy --workspace --all-targets -- -D warnings && $HOME/.cargo/bin/cargo test --workspace && $HOME/.cargo/bin/cargo build --release -p app`
Expected: clean; all pass; release build succeeds.

```bash
git status && git diff
git add crates/app/src/app.rs crates/app/src/host.rs crates/app/src/headless.rs docs/agent-api.md docs/superpowers/specs/2026-10-10-tenon-design.md
git commit -m "App: the window notifies once a frame and quits on request; window actions listed"
```

## After the last task

- [ ] `$HOME/.cargo/bin/cargo test --workspace`, `$HOME/.cargo/bin/cargo clippy --workspace --all-targets -- -D warnings`, `$HOME/.cargo/bin/cargo fmt --all --check`, `$HOME/.cargo/bin/cargo build --release -p app`.
- [ ] Final whole-branch review, one fix wave, then the roadmap row `done <first>..<last>` ("Roadmap: M10a done") and the M10a section in the project memory's follow-ups file.
- [ ] Delete the scratch refs: `git for-each-ref --format='%(refname)' refs/scratch/m10a | xargs -n1 git update-ref -d`.
