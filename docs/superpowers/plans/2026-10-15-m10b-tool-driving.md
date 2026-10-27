# M10b Tool Driving and Parity Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** An agent can do anything a user does with the tools without touching the window: a `tool_input {events}` method feeds the active tool pointer, key, modifier and typed events through the window's own editor code (inference, locks, typed values, Ctrl copy, Cmd sampling, double-click opening) and reports the tool's phase, preview, status and the outcome of every command it ran; and the remaining parity gaps close as commands: `orbit`/`zoom` about a given point, `select` by a screen box (inside or crossing), `add_arc` by start, end and a third point or a bulge, `position_texture` with a relative `rotate_by`, and `undo`/`redo` with a count.

**Architecture:**
- `session`: `Undo`/`Redo` gain `count` (each step its own revision; all or nothing); `Orbit`/`Zoom` gain `about` (a root point; `Camera::turn` and `Camera::zoom`, the navigation code's own); `AddArc` takes `center`+`angle`, or `end`+`through` (`arc_through`), or `end`+`bulge` (`arc_from_bulge`, the Arc tool's); `PositionTexture` and `kernel::PlacementEdit` gain `rotate_by`, which the Entity Info Rotate 90 button now uses.
- `tools`: the Select tool's box test becomes public, `select_box(scenery, camera, a, b, crossing)`.
- `api`: `execute` resolves `{"type": "select", "box": {...}}` (`ScreenCommand`) into ids with `select_box` and runs `select`; new module `input` with the `tool_input` wire types, their checks and the result; `Host::tool_input` (refusing by default, so `api::HeadlessHost` has none).
- `app`: the Editor notes every command it runs while `tool_input` runs (`execute_recorded`); `editor/input.rs` drives the Editor's own input methods (`cursor_moved`, `button`, `double_click`, `key`, `set_modifiers`, `set_sample_modifier`, `toggle_copy`, `undo`, `redo`) event by event, as the window's event loop does; `AppHost::tool_input` serves window and headless alike.

**Tech Stack:** Rust 2024; serde, schemars, serde_json, glam; no new dependencies.

**Spec:** `docs/superpowers/specs/2026-10-10-tenon-design.md` ("Agent control milestones", M10b; section 5 for the API). M10a's plan (`docs/superpowers/plans/2026-10-15-m10a-agent-state.md`) has the interfaces this builds on (`EditorState`, `Host::editor`, `AppHost`, `editor_state`, `snap` kinds of `infer`).

## Global Constraints

- No emoji anywhere (code, comments, docs, commit messages). Comments succinct, in the style of the surrounding code.
- `cargo fmt --all` clean; `cargo clippy --workspace --all-targets -- -D warnings` clean; `cargo test --workspace` passes after every task. Cargo is at `$HOME/.cargo/bin/cargo` (not on the subagents' PATH).
- No new external dependencies. Disk is limited: no extra worktrees or target directories.
- From the spec: method `tool_input {events}` feeds the active tool the events a user would make, without the window: `move`, `down`, `up`, `click`, `double_click` at a pixel (with view width and height) or a model point, `drag` paths, `key` (with modifiers) and `type` (measurements box text then Enter), each run through the same inference, locks and typed-value handling as the window; the result reports the tool's phase, preview (inferred point, rubber-band geometry), status text and the outcome of any command it issued. Parity gaps closed as commands: `orbit`/`zoom` about a given model point; `select` by screen box with window or crossing mode; `add_arc` by start, end and a third point (or bulge); `position_texture` with `rotate_by` (relative); `undo`/`redo` with a count.
- `tool_input` must drive the same tool code paths the window uses (the Editor's input methods), not a reimplementation, headless and in the window; every tool is covered by tests (Line, Rectangle, Circle, Arc, Push/Pull, Move incl. copy, Rotate, Tape Measure, Erase, Paint, Select incl. box and double-click open), typed values, Shift/axis locks and modifier keys.
- Git: work on `dev`; other agents may share the checkout: run `git status` and `git diff` first and stage only the files you changed (never `git add -A` or `git add .`); short plain commit messages, no attribution lines. Never push or merge.
- Do not launch the app window. Tests that start the binary use stdio or their own scratch sockets; never touch `~/.tenon/house.sock` or `h.sock` or their processes.

## Spec notes (rulings made in this plan)

- **`tool_input` is a method**, not a command (like `quit`): params `{events, width?, height?}`, result `{editor, inference, preview, commands, revision}`. It changes the model only through the commands the tools run, which are undo steps, labeled and notified as usual. A host without an editor (`api::HeadlessHost`) refuses it with `invalid_input` ("this program has no editor"); `app --headless` and the window run it on the same Editor.
- **Same code paths.** Each event calls the Editor methods the window's event loop calls for the same user action: `move` is `cursor_moved`; `down`/`up` are `cursor_moved` then `button`; `click` is down then up; `double_click` is two clicks then `double_click` (the window's sequence); `drag` is down, moves, up; `key` maps to `EditorKey` as `editor_key` does in `app.rs` (an uppercase letter exactly when Shift is held), Cmd/Ctrl+Z to `undo`/`redo` as the window does, and the key `ctrl` to `toggle_copy` (the window's Ctrl tap); `modifiers` is `set_modifiers` and `set_sample_modifier`; `type` puts the text in the measurements box and presses Enter (`key(Enter)`). After each event the hover is worked out (`flush_hover`), as a frame drawn between events would; navigation (middle drag, Alt+left, Shift+middle pan) works as in the window.
- **Where an event happens.** A `pixel` is in a view of `width` x `height` (both or neither, 1 to 4096; default the 3D view, `editor.viewport`) and is scaled to the 3D view; the view must have the 3D view's proportions within 1%, else `invalid_input` before anything runs. A `point` (open context coordinates) is projected by the document camera at the event's turn (the camera may have moved) and must be in sight. Either way the tools infer from that pixel, so a point near an endpoint snaps to it; exact values come from typing.
- **Checks and failures.** Malformed params are `-32602`. Shape errors (both or neither of `pixel`/`point`, a drag of fewer than two places, an unknown key, a Cmd key other than Z, empty text or control characters, a size out of range, a point beyond the coordinate limit) are `invalid_input` naming the event (`event 2: ...`), and nothing runs. An event that cannot run at its turn (a pixel outside the view, a point out of sight) stops the rest with `invalid_input`; the events before it stay done (their commands are undo steps; `history` and the `editor` query show them).
- **Keys.** One character, or `enter`, `backspace`, `delete`, `escape`, `space`, `right`/`left`/`up` (the arrow keys' axis locks), `down` (unlock), `ctrl` (a Ctrl tap). With `cmd` or `ctrl` only `z` (undo; Shift: redo); the window's file shortcuts are commands already (`new`, `open`, `save`, `import`, `export`, `quit`) and open native dialogs, so they are refused. With `alt` a key does nothing, as in the window. Fly mode's moving keys are not keys here (the eye only moves in the window; `set_camera` places it).
- **Modifiers** held by a `modifiers` event (exactly those listed) last until the next `modifiers` event or the end of the call, when all are let go as if released: so a Shift lock ends with the call, while arrow-key locks last until the operation ends. A `key` event's own modifiers are held only for that key. `ctrl` and `cmd` act alike (the window takes either for sampling and undo). A button left down at the end of a call stays down for a later call.
- **Result.** `editor` is the `editor` query's state after the events; `inference` the snap under the cursor (`infer`'s kinds and `axis`; null while flying); `preview` the tool's rubber-band segments in open context coordinates; `commands` every command the Editor ran during the call, in order, with `outcome` or `error`: the tool's, a copy's selection of what it made, an array's undo, a shortcut's (`make_group`, `make_component`, `erase`, `zoom_extents`), `undo`/`redo`. A copy or turn that changed nothing is reported with its (empty) outcome; the editor's message says why it was dropped. While flying, clicks and typing do nothing, as in the window.
- **Window.** The window's Editor runs `tool_input` between frames like any request; the cursor stays where the events left it until the user's mouse moves, and the next frame draws the marker there (`drive` leaves the hover marked out of date). The user's own key and mouse input in between interleaves with the agent's, as two people at one keyboard would.
- **`select` by a box** is `{"type": "select", "box": {"from", "to", "crossing"?, "width"?, "height"?}, "mode"?}` through `execute` only (`ScreenCommand`; in a `batch` the session refuses `box` as an unknown field, `-32602`). It selects what the Select tool's box would (`tools::select_box`, the tool's own code), seen by the document camera; `crossing` is explicit (the tool derives it from the drag direction). Corners outside the view are `invalid_input`; `ids` with `box` is `-32602`.
- **`orbit`/`zoom` `about`** is a root point (the camera and its target are in root coordinates). `orbit` turns about it with `Camera::turn` (the middle-drag code); `zoom` uses `Camera::zoom` (the scroll wheel's zoom toward a point) with `steps = -ln(factor) / ln(ZOOM_STEP)`, so the point keeps its pixel and the eye comes to `1/factor` of its distance; in perspective the target moves to the point's depth, as the wheel leaves it.
- **`add_arc`** takes exactly one of: `center` + `angle` (as before); `end` + `through` (the three points set the plane, the reported normal makes start, through, end counter-clockwise, angle in (0, 360); `normal` refused; collinear or coincident points refused); `end` + `bulge` (the Arc tool's `arc_from_bulge`, normal default Z, start and end in one plane with it, bulge non-zero). `center` and `angle` become optional fields.
- **`rotate_by`** turns each face's texture that many degrees further (after any `rotation_deg`; both together are refused). Entity Info's Rotate 90 now is one `position_texture` with `rotate_by: 90` instead of a batch grouping faces by rotation (same result and one undo step).
- **`undo`/`redo` `count`** (default 1, 0 refused) undoes that many steps, each its own revision and `changed` event; the outcome's `changes` are their net effect. With fewer steps than `count` it fails (`nothing_to_undo` / `nothing_to_redo`, "only 2 steps to undo") and changes nothing.
- **describe** keeps `schema_version` 1 (additions only): the `tool_input` method and the `ToolInput`, `ToolInputResult` and `ScreenCommand` schemas.

## File Structure

| File | Responsibility | Task |
|---|---|---|
| `crates/session/src/command.rs` | `count`, `about`, `AddArc` forms, `rotate_by` | 1, 2 |
| `crates/session/src/document.rs`, `document/camera.rs`, `document/placement.rs` | counted undo/redo, camera `about`, arc forms, `rotate_by` check | 1, 2 |
| `crates/session/src/arc.rs`, `lib.rs` | `arc_through`, `arc_spec` | 2 |
| `crates/kernel/src/texture.rs` | `PlacementEdit.rotate_by` | 2 |
| `crates/tools/src/{move_tool,rotate,tape,arc}.rs` | new field shapes | 1, 2 |
| `crates/app/src/editor/texture_position.rs`, `info.rs` | Rotate 90 by `rotate_by`; field shapes | 2 |
| `crates/tools/src/select.rs`, `lib.rs` | `select_box` | 3 |
| `crates/api/src/probe.rs` | `ScreenBox`, `ScreenCommand`, `box_ids`, `snap_name` | 3, 4 |
| `crates/api/src/input.rs` (new) | `tool_input` wire types, checks, result | 4 |
| `crates/api/src/dispatch.rs`, `lib.rs` | routing, `Host::tool_input`, describe, exports | 3, 4 |
| `crates/app/src/editor.rs` | `issued`, `execute_recorded` | 1, 5 |
| `crates/app/src/editor/input.rs` (new) | `Editor::drive` | 5 |
| `crates/app/src/host.rs`, `host/tool_input_tests.rs` (new) | `AppHost::tool_input`; tests | 5, 6 |
| `crates/app/tests/headless_agent.rs` | headless end to end | 6 |
| `docs/agent-api.md` | every new command field and method, the window actions table | 1, 2, 3, 5 |

The code below was written and verified in this repository's working tree on `dev` at e74b60b (then reverted), task by task: after each task `cargo fmt --all --check` and `cargo clippy --workspace --all-targets -- -D warnings` passed and the tests named in the task passed; after Task 6 `cargo test --workspace --no-fail-fast` passed: 1097 passed, 10 ignored, none failed (summing every "test result" line; the tasks add 23 tests: 2, 3, 1, 3, 4, 10). The verified states are kept as trees at `refs/scratch/m10b/t1` .. `t6` (`git diff refs/scratch/m10b/t2 refs/scratch/m10b/t3` shows Task 3's change; restrict such diffs to the task's paths, since the trees predate the plan and roadmap commits). Applying the tasks' diffs in order gives `t6` exactly. Code for new files is given in full; changes to existing files are unified diffs against the state the previous task leaves (`git apply` them, or `git diff refs/scratch/m10b/tN-1 refs/scratch/m10b/tN -- <paths> | git apply`). Where a source file carries its own unit tests (`session/src/arc.rs`, `api/src/input.rs`), they are in its diff.

---

### Task 1: Counted undo and redo; orbit and zoom about a point

**Files:**
- Modify or create: `crates/app/src/editor.rs`
- Modify or create: `crates/session/src/command.rs`
- Modify or create: `crates/session/src/document.rs`
- Modify or create: `crates/session/src/document/camera.rs`
- Modify or create: `crates/tools/src/move_tool.rs`
- Modify or create: `crates/tools/src/rotate.rs`
- Modify or create: `crates/tools/src/tape.rs`
- Modify or create: `docs/agent-api.md`
- Test: `crates/api/tests/hub.rs`
- Test: `crates/render/tests/perf.rs`
- Test: `crates/session/tests/camera.rs`
- Test: `crates/session/tests/commands.rs`
- Test: `crates/session/tests/contract.rs`

**Interfaces:**
- Consumes: nothing new.
- Produces:
  - `Command::Undo { count: Option<u32> }`, `Command::Redo { count: Option<u32> }` (were `Undo {}`, `Redo {}`; every construction becomes `Command::Undo { count: None }`, every pattern `Command::Undo { .. }`).
  - `Command::Orbit { yaw, pitch, about: Option<[f64; 3]> }`, `Command::Zoom { factor, about: Option<[f64; 3]> }` (root coordinates, like the camera).
  - Private in `session::document`: `step_count(Option<u32>) -> Result<usize, CommandError>`, `too_few(&str, usize) -> String`.

Verify the result against the scratch tree: `git diff refs/scratch/m10b/t1 -- <paths>` is empty after the task (the tree predates the plan commit, so restrict it to the task's paths).

- [ ] **Step 1: Write the failing tests**

```diff
diff --git a/crates/api/tests/hub.rs b/crates/api/tests/hub.rs
index b7ce717..dfa9678 100644
--- a/crates/api/tests/hub.rs
+++ b/crates/api/tests/hub.rs
@@ -53,7 +53,9 @@ fn subscribers_get_every_change_in_order() {
     assert_eq!(next(&w_rx)["params"]["revision"], 2);
 
     // Edits made outside the API (e.g. in the window) are announced by notify().
-    host.doc.execute(session::Command::Undo {}).unwrap();
+    host.doc
+        .execute(session::Command::Undo { count: None })
+        .unwrap();
     hub.notify(&host);
     let undo = next(&w_rx);
     assert_eq!(undo["params"]["revision"], 3);
diff --git a/crates/render/tests/perf.rs b/crates/render/tests/perf.rs
index b8f6853..51cd0cc 100644
--- a/crates/render/tests/perf.rs
+++ b/crates/render/tests/perf.rs
@@ -289,7 +289,7 @@ fn time_house_renders() {
                 },
             );
         } else {
-            run(&mut d, Command::Undo {});
+            run(&mut d, Command::Undo { count: None });
         }
         let t = Instant::now();
         snap.capture(
diff --git a/crates/session/tests/camera.rs b/crates/session/tests/camera.rs
index ced1288..6dcb230 100644
--- a/crates/session/tests/camera.rs
+++ b/crates/session/tests/camera.rs
@@ -129,6 +129,45 @@ fn zoom_divides_the_distance_by_the_factor() {
     );
 }
 
+#[test]
+fn orbit_and_zoom_turn_and_zoom_about_a_given_point() {
+    let mut d = doc();
+    front(&mut d);
+    // A quarter turn about (2, 0, 0): the eye swings from (0, -10, 0) to (12, -2, 0).
+    run(
+        &mut d,
+        json!({"type": "orbit", "yaw": 90, "about": [2, 0, 0]}),
+    )
+    .unwrap();
+    let (eye, target, up) = view(&d);
+    assert!(close(eye, DVec3::new(12.0, -2.0, 0.0)), "{eye}");
+    assert!(close(target, DVec3::new(2.0, -2.0, 0.0)), "{target}");
+    assert!(close(up, DVec3::Z));
+
+    // Zooming about a point keeps it on the same pixel and divides the eye's distance to it.
+    front(&mut d);
+    let p = DVec3::new(1.0, -4.0, 0.5);
+    let px = d.camera.project(p).unwrap();
+    let before = d.camera.eye.distance(p);
+    run(
+        &mut d,
+        json!({"type": "zoom", "factor": 2, "about": p.to_array()}),
+    )
+    .unwrap();
+    assert!(d.camera.project(p).unwrap().abs_diff_eq(px, 1e-6));
+    assert!((d.camera.eye.distance(p) - before / 2.0).abs() < 1e-9);
+    // In perspective the target comes to the point's depth.
+    assert!((d.camera.distance() - d.camera.depth(p)).abs() < 1e-9);
+
+    for bad in [
+        json!({"type": "orbit", "yaw": 10, "about": [1e9, 0, 0]}),
+        json!({"type": "zoom", "factor": 2, "about": [0, f64::MAX, 0]}),
+    ] {
+        assert_eq!(run(&mut d, bad), Err(ErrorKind::InvalidInput));
+    }
+    assert_eq!(d.revision(), 0, "camera commands are not edits");
+}
+
 #[test]
 fn standard_views_look_along_the_axes_from_the_same_distance() {
     let mut d = doc();
diff --git a/crates/session/tests/commands.rs b/crates/session/tests/commands.rs
index 02b3e9d..ad452aa 100644
--- a/crates/session/tests/commands.rs
+++ b/crates/session/tests/commands.rs
@@ -171,7 +171,7 @@ fn not_found_error_serializes_requested_ids_and_other_errors_omit_them() {
         json!({"kind": "not_found", "message": err.message, "ids": ["f99"]})
     );
     let mut d = doc();
-    let err = d.execute(Command::Undo {}).unwrap_err();
+    let err = d.execute(Command::Undo { count: None }).unwrap_err();
     let v = serde_json::to_value(&err).unwrap();
     assert!(v.get("ids").is_none());
     let back: session::CommandError = serde_json::from_value(v).unwrap();
diff --git a/crates/session/tests/contract.rs b/crates/session/tests/contract.rs
index 1f1087b..e0b5651 100644
--- a/crates/session/tests/contract.rs
+++ b/crates/session/tests/contract.rs
@@ -72,7 +72,7 @@ fn events_list_changes_after_a_revision() {
     for c in square() {
         d.execute(c).unwrap();
     }
-    d.execute(Command::Undo {}).unwrap();
+    d.execute(Command::Undo { count: None }).unwrap();
     let r = ask(&d, json!({"type": "events", "since": 3}));
     assert_eq!(r["revision"], 5);
     assert_eq!(r["truncated"], false);
@@ -134,7 +134,7 @@ fn atomic_batch_is_one_undo_step() {
     assert_eq!(ask(&d, json!({"type": "stats"}))["faces"], 1);
     assert_eq!(ask(&d, json!({"type": "history"}))["undo_depth"], 1);
     assert_eq!(d.revision(), 1);
-    d.execute(Command::Undo {}).unwrap();
+    d.execute(Command::Undo { count: None }).unwrap();
     assert_eq!(ask(&d, json!({"type": "stats"}))["edges"], 0);
 }
 
@@ -161,7 +161,9 @@ fn failed_atomic_batch_changes_nothing() {
 #[test]
 fn atomic_batch_rejects_undo() {
     let mut d = doc();
-    let err = d.execute_batch(vec![Command::Undo {}], true).unwrap_err();
+    let err = d
+        .execute_batch(vec![Command::Undo { count: None }], true)
+        .unwrap_err();
     assert_eq!(err.error.kind, ErrorKind::InvalidInput);
 }
 
@@ -222,7 +224,7 @@ fn history_labels_each_step_by_its_command() {
     d.execute(cmd(json!({"type": "orbit", "yaw": 10}))).unwrap();
     d.execute(cmd(json!({"type": "select", "ids": []})))
         .unwrap();
-    d.execute(Command::Undo {}).unwrap();
+    d.execute(Command::Undo { count: None }).unwrap();
     let h = ask(&d, json!({"type": "history"}));
     assert_eq!(h["undo"], json!(["add_edge", "add_edge"]));
     assert_eq!(h["redo"], json!(["add_guide_point"]));
@@ -242,9 +244,9 @@ fn history_labels_each_step_by_its_command() {
     assert_eq!(h["redo"], json!([]));
 
     // Undo and redo carry the label across.
-    d.execute(Command::Undo {}).unwrap();
-    d.execute(Command::Undo {}).unwrap();
-    d.execute(Command::Redo {}).unwrap();
+    d.execute(Command::Undo { count: None }).unwrap();
+    d.execute(Command::Undo { count: None }).unwrap();
+    d.execute(Command::Redo { count: None }).unwrap();
     let h = ask(&d, json!({"type": "history"}));
     assert_eq!(h["undo"], json!(["add_edge", "add_edge"]));
     assert_eq!(h["redo"], json!(["batch"]));
@@ -259,3 +261,59 @@ fn command_names_are_their_types() {
         "push_pull"
     );
 }
+
+#[test]
+fn undo_and_redo_take_a_count() {
+    let mut d = doc();
+    d.execute_batch(square(), false).unwrap();
+    let r0 = d.revision();
+    let out = d.execute(cmd(json!({"type": "undo", "count": 3}))).unwrap();
+    assert_eq!(d.revision(), r0 + 3, "one revision a step");
+    let gone = |ids: &[kernel::EntityId]| {
+        ids.iter()
+            .filter(|id| !matches!(id, kernel::EntityId::Vertex(_)))
+            .count()
+    };
+    assert_eq!(
+        gone(&out.changes.deleted),
+        3 + 1,
+        "three edges and the face"
+    );
+    let h = ask(&d, json!({"type": "history"}));
+    assert_eq!(
+        (h["undo_depth"].clone(), h["redo_depth"].clone()),
+        (json!(1), json!(3))
+    );
+
+    // More steps than there are undoes nothing.
+    let e = d
+        .execute(cmd(json!({"type": "undo", "count": 2})))
+        .unwrap_err();
+    assert_eq!(
+        (e.kind, e.message.as_str()),
+        (ErrorKind::NothingToUndo, "only 1 step to undo")
+    );
+    let e = d
+        .execute(cmd(json!({"type": "redo", "count": 4})))
+        .unwrap_err();
+    assert_eq!(
+        (e.kind, e.message.as_str()),
+        (ErrorKind::NothingToRedo, "only 3 steps to redo")
+    );
+    let e = d
+        .execute(cmd(json!({"type": "undo", "count": 0})))
+        .unwrap_err();
+    assert_eq!(e.kind, ErrorKind::InvalidInput);
+    assert_eq!(d.revision(), r0 + 3);
+
+    let out = d.execute(cmd(json!({"type": "redo", "count": 3}))).unwrap();
+    assert_eq!(gone(&out.changes.created), 3 + 1);
+    let h = ask(&d, json!({"type": "history"}));
+    assert_eq!(
+        (h["undo_depth"].clone(), h["redo_depth"].clone()),
+        (json!(4), json!(0))
+    );
+    // Without a count, one step.
+    d.execute(cmd(json!({"type": "undo"}))).unwrap();
+    assert_eq!(ask(&d, json!({"type": "history"}))["undo_depth"], 3);
+}
```

- [ ] **Step 2: Run them to see them fail**

Run: `$HOME/.cargo/bin/cargo test -p session --test contract --test camera`
Expected: compile errors (`count` and `about` are unknown fields of the variants).

- [ ] **Step 3: Implement**

```diff
diff --git a/crates/app/src/editor.rs b/crates/app/src/editor.rs
index e85b183..d5146e5 100644
--- a/crates/app/src/editor.rs
+++ b/crates/app/src/editor.rs
@@ -1114,14 +1114,14 @@ impl Editor {
 
     pub fn undo(&mut self) -> bool {
         self.reset_tool();
-        self.run(Command::Undo {});
+        self.run(Command::Undo { count: None });
         self.invalidate_hover();
         true
     }
 
     pub fn redo(&mut self) -> bool {
         self.reset_tool();
-        self.run(Command::Redo {});
+        self.run(Command::Redo { count: None });
         self.invalidate_hover();
         true
     }
@@ -1262,7 +1262,7 @@ impl Editor {
         let mut undone = false;
         let selected: Vec<EntityId> = self.doc.selection().iter().copied().collect();
         for command in out.commands {
-            let is_undo = matches!(command, Command::Undo {});
+            let is_undo = matches!(command, Command::Undo { .. });
             let is_copy = matches!(
                 command,
                 Command::Copy { .. }
@@ -1284,7 +1284,7 @@ impl Editor {
                 });
             }
             let Some(outcome) = outcome.filter(|_| !unchanged) else {
-                if undone && let Ok(redone) = self.doc.execute(Command::Redo {}) {
+                if undone && let Ok(redone) = self.doc.execute(Command::Redo { count: None }) {
                     if is_copy && !redone.changes.created.is_empty() {
                         self.select_created(redone);
                     } else {
@@ -2814,7 +2814,7 @@ mod tests {
         assert_eq!(box_max(&e, group), DVec3::new(10.0, 2.0, 1.0));
         type_text(&mut e, "-90");
         assert_eq!(box_min(&e, group), DVec3::new(10.0, -2.0, 0.0));
-        e.execute(Command::Undo {}).unwrap();
+        e.execute(Command::Undo { count: None }).unwrap();
         assert_eq!(
             box_min(&e, group),
             DVec3::new(10.0, 0.0, 0.0),
diff --git a/crates/session/src/command.rs b/crates/session/src/command.rs
index 4f036d7..5750e91 100644
--- a/crates/session/src/command.rs
+++ b/crates/session/src/command.rs
@@ -17,10 +17,7 @@ use crate::import::{ImportAs, ImportFormat, ImportReport, UpAxis};
 #[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
 pub enum Command {
     /// Draw a straight edge. Sticky-geometry rules apply (merging, splitting, auto faces).
-    AddEdge {
-        start: [f64; 3],
-        end: [f64; 3],
-    },
+    AddEdge { start: [f64; 3], end: [f64; 3] },
     /// Draw a rectangle from one corner to the opposite one, as one undo step. Its sides follow
     /// the world axes on axis planes. `normal` picks the plane; without it the corners must
     /// differ in exactly two coordinates.
@@ -56,10 +53,7 @@ pub enum Command {
     },
     /// Move a face along its normal by `distance` meters (negative: against the normal),
     /// extruding new sides or stretching the faces around it.
-    PushPull {
-        face: EntityId,
-        distance: f64,
-    },
+    PushPull { face: EntityId, distance: f64 },
     /// Add a guide line: the infinite line through `point` along `direction` (any non-zero
     /// length), as one undo step. Guides are construction geometry: tools snap to them, they
     /// bound no faces and are not exported.
@@ -68,9 +62,7 @@ pub enum Command {
         direction: [f64; 3],
     },
     /// Add a guide point at `position`, as one undo step.
-    AddGuidePoint {
-        position: [f64; 3],
-    },
+    AddGuidePoint { position: [f64; 3] },
     /// Erase every guide of the open context, as one undo step (nothing to erase is not an
     /// error and makes no undo step).
     DeleteGuides {},
@@ -78,9 +70,7 @@ pub enum Command {
     /// faces it bounds with it, except that two faces in one plane meeting there merge into
     /// one. Vertices go away with their last edge and cannot be erased directly. A group's
     /// definition goes with its last instance.
-    Erase {
-        ids: Vec<EntityId>,
-    },
+    Erase { ids: Vec<EntityId> },
     /// Move vertices, edges, faces, instances and guides by `offset` meters as one undo step.
     /// Faces that share some of the moved vertices stretch and may tilt, but must stay flat.
     Move {
@@ -114,35 +104,24 @@ pub enum Command {
     /// Turn edges, faces (with their edges), instances and guides into a group: they move into
     /// a new definition, placed by one new instance where they were. Guides alone cannot be
     /// grouped. The instance becomes the selection.
-    MakeGroup {
-        ids: Vec<EntityId>,
-    },
+    MakeGroup { ids: Vec<EntityId> },
     /// Like `MakeGroup`, but the definition is a named component: copies of its instances
     /// share it, so editing one edits them all.
-    MakeComponent {
-        ids: Vec<EntityId>,
-        name: String,
-    },
+    MakeComponent { ids: Vec<EntityId>, name: String },
     /// Replace instances by their geometry, which merges with what it touches. Instances and
     /// guides inside them come out as instances and guides. What comes out becomes the selection.
-    Explode {
-        ids: Vec<EntityId>,
-    },
+    Explode { ids: Vec<EntityId> },
     /// Edit the definition an instance of the open context places: its entities become the
     /// ones commands and queries see, in its own coordinates. Clears the selection. Not
     /// recorded in undo history.
-    OpenInstance {
-        id: EntityId,
-    },
+    OpenInstance { id: EntityId },
     /// Go back to editing the context the open instance is in. Clears the selection. Not
     /// recorded in undo history.
     CloseInstance {},
     /// Make `path` the open context: instances from the root, each placed in the definition
     /// of the one before (empty: the root), as the `outliner` query lists them. Clears the
     /// selection unless `path` is already open. Not recorded in undo history.
-    OpenPath {
-        path: Vec<EntityId>,
-    },
+    OpenPath { path: Vec<EntityId> },
     /// Rename a group or component definition. A component's name must not be empty.
     RenameDefinition {
         definition: DefinitionId,
@@ -194,9 +173,7 @@ pub enum Command {
     },
     /// Remove a material. Faces and instances painted with it, anywhere in the model, go back
     /// to the default material.
-    DeleteMaterial {
-        material: MaterialId,
-    },
+    DeleteMaterial { material: MaterialId },
     /// Paint faces and instances of the open context with `material` (absent or null: the
     /// default material), as one undo step. On faces `side` (default `front`) picks the side
     /// the normal points to, the other one, or both. An instance's material shows on the face
@@ -212,10 +189,7 @@ pub enum Command {
     /// edges in `ids` and the edges around the faces in `ids`. A soft edge is hidden and the
     /// faces on its sides are shaded as one smooth surface. Only an edge between exactly two
     /// faces can be soft; others are left as they are and counted in the outcome's `soften`.
-    SoftenEdges {
-        ids: Vec<EntityId>,
-        soft: bool,
-    },
+    SoftenEdges { ids: Vec<EntityId>, soft: bool },
     /// Soften each edge between two faces whose normals differ by at most `max_angle_deg`
     /// (0 to 180), as one undo step: among the edges in `ids` and the edges around the faces
     /// in `ids`, or among every edge of the open context when `ids` is empty or absent. Other
@@ -298,8 +272,17 @@ pub enum Command {
         #[serde(default)]
         mode: SelectMode,
     },
-    Undo {},
-    Redo {},
+    /// Undo the last `count` steps (default 1), each its own revision. With fewer steps to
+    /// undo than `count`, fails (`nothing_to_undo`) and undoes nothing.
+    Undo {
+        #[serde(default)]
+        count: Option<u32>,
+    },
+    /// Redo the last `count` undone steps (default 1), like `undo`.
+    Redo {
+        #[serde(default)]
+        count: Option<u32>,
+    },
     /// Place the camera: from `eye` looking at `target`, with the vertical field of view
     /// `fov_y_deg` (1 to 170; default: unchanged) and `projection` (default: unchanged). A
     /// view straight down or up takes the screen's up direction from the horizontal part of
@@ -314,14 +297,17 @@ pub enum Command {
         #[serde(default)]
         projection: Option<Projection>,
     },
-    /// Turn the camera about its target: `yaw` degrees about the vertical (counter-clockwise
-    /// seen from above), then `pitch` degrees raising the eye (negative lowers it). The view
-    /// stops 0.01 rad short of straight down or up.
+    /// Turn the camera about its target, or about the root point `about` (as dragging with the
+    /// middle button turns it about the point under the cursor): `yaw` degrees about the
+    /// vertical (counter-clockwise seen from above), then `pitch` degrees raising the eye
+    /// (negative lowers it). The view stops 0.01 rad short of straight down or up.
     Orbit {
         #[serde(default)]
         yaw: f64,
         #[serde(default)]
         pitch: f64,
+        #[serde(default)]
+        about: Option<[f64; 3]>,
     },
     /// Move the eye and target together, `right` and `up` meters along the screen's right
     /// and up directions.
@@ -333,30 +319,28 @@ pub enum Command {
     },
     /// Move the eye toward the target to `1 / factor` of its distance (factor above 1 zooms
     /// in, below 1 out; never nearer than 0.01 m). In a parallel view the view height shrinks
-    /// the same way.
+    /// the same way. With `about` (a root point), zoom toward it as the scroll wheel zooms
+    /// toward the point under the cursor: it stays where it is on screen, the eye moves to
+    /// `1 / factor` of its distance from it and, in perspective, the target moves to its depth.
     Zoom {
         factor: f64,
+        #[serde(default)]
+        about: Option<[f64; 3]>,
     },
     /// Keep the view direction and fit the whole model (every vertex and placed instance, in
     /// root coordinates, whatever instance is open) in the view. Guides do not count. With
     /// nothing to fit, the camera stays.
     ZoomExtents {},
     /// Look at the target from a standard direction, from the same distance.
-    StandardView {
-        view: StandardView,
-    },
+    StandardView { view: StandardView },
     /// Switch between perspective and parallel projection. The view height at the target
     /// stays the same.
-    SetProjection {
-        projection: Projection,
-    },
+    SetProjection { projection: Projection },
     /// Light the view by `day` (a headlight and ambient light, glowing faces adding theirs) or
     /// by `night` (both dimmed, a dark background: glowing faces light the scene). A view
     /// setting like the camera's: saved with the model, but not an undo step and not a change
     /// to the revision.
-    SetLighting {
-        mode: Lighting,
-    },
+    SetLighting { mode: Lighting },
 }
 
 impl Command {
diff --git a/crates/session/src/document.rs b/crates/session/src/document.rs
index 345518b..480c643 100644
--- a/crates/session/src/document.rs
+++ b/crates/session/src/document.rs
@@ -182,23 +182,48 @@ impl Document {
             return Ok(outcome);
         }
         match command {
-            Command::Undo {} => {
-                let step = self.undo.pop().ok_or_else(|| {
-                    CommandError::new(ErrorKind::NothingToUndo, "nothing to undo")
-                })?;
-                let inverse = step.changes.inverse();
-                self.scene.apply(&inverse);
-                self.redo.push(step);
-                Ok(self.changed(&inverse))
+            Command::Undo { count } => {
+                let n = step_count(count)?;
+                if self.undo.len() < n {
+                    return Err(CommandError::new(
+                        ErrorKind::NothingToUndo,
+                        too_few("undo", self.undo.len()),
+                    ));
+                }
+                let mut done = Vec::new();
+                for _ in 0..n {
+                    let step = self.undo.pop().expect("counted");
+                    let inverse = step.changes.inverse();
+                    self.scene.apply(&inverse);
+                    self.redo.push(step);
+                    self.changed(&inverse);
+                    done.push(inverse);
+                }
+                Ok(Outcome {
+                    changes: SceneChange::join(done).summary(),
+                    ..Outcome::default()
+                })
             }
-            Command::Redo {} => {
-                let step = self.redo.pop().ok_or_else(|| {
-                    CommandError::new(ErrorKind::NothingToRedo, "nothing to redo")
-                })?;
-                self.scene.apply(&step.changes);
-                let outcome = self.changed(&step.changes);
-                self.undo.push(step);
-                Ok(outcome)
+            Command::Redo { count } => {
+                let n = step_count(count)?;
+                if self.redo.len() < n {
+                    return Err(CommandError::new(
+                        ErrorKind::NothingToRedo,
+                        too_few("redo", self.redo.len()),
+                    ));
+                }
+                let mut done = Vec::new();
+                for _ in 0..n {
+                    let step = self.redo.pop().expect("counted");
+                    self.scene.apply(&step.changes);
+                    self.changed(&step.changes);
+                    done.push(step.changes.clone());
+                    self.undo.push(step);
+                }
+                Ok(Outcome {
+                    changes: SceneChange::join(done).summary(),
+                    ..Outcome::default()
+                })
             }
             Command::SetCamera { .. }
             | Command::Orbit { .. }
@@ -426,8 +451,8 @@ impl Document {
             | Command::Open { .. }
             | Command::Save { .. }
             | Command::Export { .. } => return None,
-            Command::Undo {}
-            | Command::Redo {}
+            Command::Undo { .. }
+            | Command::Redo { .. }
             | Command::SetCamera { .. }
             | Command::Orbit { .. }
             | Command::Pan { .. }
@@ -499,8 +524,8 @@ impl Document {
                 }),
                 None if matches!(
                     command,
-                    Command::Undo {}
-                        | Command::Redo {}
+                    Command::Undo { .. }
+                        | Command::Redo { .. }
                         | Command::New { .. }
                         | Command::Open { .. }
                         | Command::Save { .. }
@@ -1356,3 +1381,23 @@ pub fn check_coordinates(points: &[[f64; 3]]) -> Result<(), CommandError> {
         ))
     }
 }
+
+/// The steps an `undo` or `redo` of `count` takes: 1 by default, never 0.
+fn step_count(count: Option<u32>) -> Result<usize, CommandError> {
+    match count.unwrap_or(1) {
+        0 => Err(CommandError::new(
+            ErrorKind::InvalidInput,
+            "count must be at least 1",
+        )),
+        n => Ok(n as usize),
+    }
+}
+
+/// Why an `undo` or `redo` of more steps than there are fails.
+fn too_few(what: &str, left: usize) -> String {
+    match left {
+        0 => format!("nothing to {what}"),
+        1 => format!("only 1 step to {what}"),
+        n => format!("only {n} steps to {what}"),
+    }
+}
diff --git a/crates/session/src/document/camera.rs b/crates/session/src/document/camera.rs
index c184fb1..6084fc4 100644
--- a/crates/session/src/document/camera.rs
+++ b/crates/session/src/document/camera.rs
@@ -5,7 +5,7 @@ use glam::DVec3;
 use kernel::MeshId;
 
 use super::{Document, check_coordinates, unit_vector};
-use crate::camera::{Camera, Projection, UP, VERTICAL_TOLERANCE};
+use crate::camera::{Camera, Projection, UP, VERTICAL_TOLERANCE, ZOOM_STEP};
 use crate::command::{Command, CommandError, ErrorKind};
 
 impl Document {
@@ -21,10 +21,16 @@ impl Document {
                 up,
                 projection,
             } => return self.camera_for(eye, target, fov_y_deg, up, projection),
-            Command::Orbit { yaw, pitch } => {
+            Command::Orbit { yaw, pitch, about } => {
                 finite(&[yaw, pitch], "yaw and pitch")?;
-                let target = camera.target;
-                camera.turn(target, yaw.to_radians(), -pitch.to_radians());
+                let pivot = match about {
+                    Some(p) => {
+                        check_coordinates(&[p])?;
+                        DVec3::from(p)
+                    }
+                    None => camera.target,
+                };
+                camera.turn(pivot, yaw.to_radians(), -pitch.to_radians());
             }
             Command::Pan { right, up } => {
                 finite(&[right, up], "right and up")?;
@@ -32,14 +38,21 @@ impl Document {
                 camera.eye += offset;
                 camera.target += offset;
             }
-            Command::Zoom { factor } => {
+            Command::Zoom { factor, about } => {
                 if !(factor.is_finite() && factor > 0.0) {
                     return Err(CommandError::new(
                         ErrorKind::InvalidInput,
                         "factor must be a positive number",
                     ));
                 }
-                camera.zoom_by(factor);
+                match about {
+                    Some(p) => {
+                        check_coordinates(&[p])?;
+                        // `zoom` takes scroll steps: `ZOOM_STEP^steps` of the distance.
+                        camera.zoom(DVec3::from(p), -factor.ln() / ZOOM_STEP.ln());
+                    }
+                    None => camera.zoom_by(factor),
+                }
             }
             Command::ZoomExtents {} => {
                 if let Some([lo, hi]) = self.scene.bounds(MeshId::Root) {
diff --git a/crates/tools/src/move_tool.rs b/crates/tools/src/move_tool.rs
index 5e940cd..5002fec 100644
--- a/crates/tools/src/move_tool.rs
+++ b/crates/tools/src/move_tool.rs
@@ -67,7 +67,7 @@ impl MoveTool {
             out.message = Some(format!("At most {MAX_COPIES} copies"));
             return;
         }
-        out.commands.push(Command::Undo {});
+        out.commands.push(Command::Undo { count: None });
         out.commands.push(Command::Copy {
             ids,
             offset: spacing.to_array(),
@@ -361,7 +361,7 @@ mod tests {
         assert_eq!(
             times.commands,
             vec![
-                Command::Undo {},
+                Command::Undo { count: None },
                 Command::Copy {
                     ids: ids(&["f8"]),
                     offset: [3.0, 0.0, 0.0],
@@ -373,7 +373,7 @@ mod tests {
         assert_eq!(
             divide.commands,
             vec![
-                Command::Undo {},
+                Command::Undo { count: None },
                 Command::Copy {
                     ids: ids(&["f8"]),
                     offset: [1.0, 0.0, 0.0],
diff --git a/crates/tools/src/rotate.rs b/crates/tools/src/rotate.rs
index ac10d07..4525c67 100644
--- a/crates/tools/src/rotate.rs
+++ b/crates/tools/src/rotate.rs
@@ -112,7 +112,7 @@ impl RotateTool {
         };
         let angle = if last.angle < 0.0 { -angle } else { angle };
         last.angle = angle;
-        out.commands.push(Command::Undo {});
+        out.commands.push(Command::Undo { count: None });
         out.commands.push(command(last, angle, Some(last.count)));
     }
 
@@ -130,7 +130,7 @@ impl RotateTool {
             out.message = Some(format!("At most {MAX_COPIES} copies"));
             return;
         }
-        out.commands.push(Command::Undo {});
+        out.commands.push(Command::Undo { count: None });
         out.commands.push(command(last, step, Some(count)));
         if let Some(last) = self.last.as_mut() {
             last.count = count;
@@ -512,7 +512,7 @@ mod tests {
         assert_eq!(
             again.commands,
             vec![
-                Command::Undo {},
+                Command::Undo { count: None },
                 rotate(&["f8"], [1.0, 1.0, 0.0], -60.0, None)
             ]
         );
@@ -544,12 +544,18 @@ mod tests {
         let times = r.send(ToolEvent::Value("3x".into()));
         assert_eq!(
             times.commands,
-            vec![Command::Undo {}, rotate(&["f8"], [0.0; 3], 90.0, Some(3))]
+            vec![
+                Command::Undo { count: None },
+                rotate(&["f8"], [0.0; 3], 90.0, Some(3))
+            ]
         );
         let divide = r.send(ToolEvent::Value("/3".into()));
         assert_eq!(
             divide.commands,
-            vec![Command::Undo {}, rotate(&["f8"], [0.0; 3], 30.0, Some(3))]
+            vec![
+                Command::Undo { count: None },
+                rotate(&["f8"], [0.0; 3], 30.0, Some(3))
+            ]
         );
         let big = r.send(ToolEvent::Value("1001x".into()));
         assert_eq!(big.message.as_deref(), Some("At most 1000 copies"));
@@ -563,7 +569,10 @@ mod tests {
         let again = r.send(ToolEvent::Value("45".into()));
         assert_eq!(
             again.commands,
-            vec![Command::Undo {}, rotate(&["f8"], [0.0; 3], -45.0, None)]
+            vec![
+                Command::Undo { count: None },
+                rotate(&["f8"], [0.0; 3], -45.0, None)
+            ]
         );
         let flipped = r.send(ToolEvent::Value("-20".into()));
         assert_eq!(turned(&flipped), 20.0, "a typed minus flips it");
@@ -579,7 +588,10 @@ mod tests {
         let out = r.send(ToolEvent::Value("45".into()));
         assert_eq!(
             out.commands,
-            vec![Command::Undo {}, rotate(&["f8"], [0.0; 3], 45.0, Some(3))]
+            vec![
+                Command::Undo { count: None },
+                rotate(&["f8"], [0.0; 3], 45.0, Some(3))
+            ]
         );
         let five = r.send(ToolEvent::Value("5x".into()));
         assert_eq!(turned(&five), 45.0);
diff --git a/crates/tools/src/tape.rs b/crates/tools/src/tape.rs
index 647379a..85680ad 100644
--- a/crates/tools/src/tape.rs
+++ b/crates/tools/src/tape.rs
@@ -209,7 +209,7 @@ impl Tool for TapeMeasureTool {
                     }
                     // Right after a guide was added: move it to the typed distance.
                     (None, Some((start, dir))) => {
-                        out.commands.push(Command::Undo {});
+                        out.commands.push(Command::Undo { count: None });
                         self.finish(start, dir * length, &mut out);
                     }
                     (None, None) => {
@@ -402,7 +402,7 @@ mod tests {
         let out = r.send(ToolEvent::Value("3".into()));
         assert_eq!(
             (out.commands.len(), &out.commands[0]),
-            (2, &Command::Undo {})
+            (2, &Command::Undo { count: None })
         );
         assert_line(&out.commands, [2.5, -3.0, 0.0], [1.0, 0.0, 0.0]);
         // A new measurement ends the chance to retype.
diff --git a/docs/agent-api.md b/docs/agent-api.md
index e079de3..547c936 100644
--- a/docs/agent-api.md
+++ b/docs/agent-api.md
@@ -105,6 +105,12 @@ not restore the selection. The `selection` query's `revision` is the model revis
 change when only the selection changes. Subscribers get a `selection` notification when it
 changes (see Notifications).
 
+`{"type": "undo"}` undoes the last step and `{"type": "redo"}` redoes the last step undone;
+`"count": 3` takes three steps (default 1, at least 1), each its own revision, and the outcome's
+`changes` are their net effect. With fewer steps than `count` it fails (`nothing_to_undo`,
+`nothing_to_redo`: "only 1 step to undo") and changes nothing. A new edit clears what could be
+redone. Undo and redo are not allowed in an atomic batch.
+
 Find the face to push from the `created` ids of the command that made it, or with
 `{"type": "entities", "kinds": ["face"]}`. Faces created by push/pull point out of the solid.
 `distance` is measured along the face's own normal, and faces in one plane can point either way
@@ -458,9 +464,9 @@ fails with `invalid_input` and leaves the camera as it was.
 | Command | Effect |
 |---|---|
 | `{"type": "set_camera", "eye": [10,-10,8], "target": [0,0,0]}` | Places the camera. Optional `fov_y_deg` (1 to 170) and `projection` keep their values when absent. A view straight down or up takes the screen's up from the horizontal part of `up` (default `[0,1,0]`; any length, however large or small; a vertical `up` is `invalid_input`); any other view has `[0,0,1]` up and ignores `up`. `eye` equal to `target` is `invalid_input`. To restore a view, pass back the `eye`, `target`, `fov_y_deg`, `up` and `projection` the `camera` query returned. |
-| `{"type": "orbit", "yaw": 30, "pitch": 10}` | Turns the eye about the target: `yaw` degrees about the vertical (counter-clockwise seen from above), then `pitch` degrees raising the eye (negative lowers it); both default to 0. The view stops 0.01 rad (0.57 degrees) short of straight down or up. From a Top or Bottom view, `yaw` spins the view about the vertical and only a pitch that moves the eye toward the horizon moves it (from Top a negative pitch, from Bottom a positive one). |
+| `{"type": "orbit", "yaw": 30, "pitch": 10}` | Turns the eye about the target, or about `about` (a root point, as a middle-button drag turns the view about the point under the cursor): `yaw` degrees about the vertical (counter-clockwise seen from above), then `pitch` degrees raising the eye (negative lowers it); both default to 0. The view stops 0.01 rad (0.57 degrees) short of straight down or up. From a Top or Bottom view, `yaw` spins the view about the vertical and only a pitch that moves the eye toward the horizon moves it (from Top a negative pitch, from Bottom a positive one). |
 | `{"type": "pan", "right": 2, "up": 0.5}` | Moves the eye and target together, meters along the screen's right and up directions (each defaults to 0). |
-| `{"type": "zoom", "factor": 2}` | Moves the eye toward the target to `1/factor` of its distance (above 1 zooms in, below 1 out), never nearer than 0.01 m. `factor` must be positive. In a parallel view `view_height` shrinks the same way. |
+| `{"type": "zoom", "factor": 2}` | Moves the eye toward the target to `1/factor` of its distance (above 1 zooms in, below 1 out), never nearer than 0.01 m. `factor` must be positive. In a parallel view `view_height` shrinks the same way. With `"about": [x, y, z]` (a root point) it zooms toward that point as the scroll wheel zooms toward the point under the cursor: the point stays on the same pixel and the eye comes to `1/factor` of its distance from it; in perspective the target moves to the point's depth, so later orbits and zooms work there. |
 | `{"type": "zoom_extents"}` | Keeps the view direction and fits the whole model in the view: every vertex and placed instance of the root, in root coordinates, whatever instance is open (guides do not count). Perspective fits the sphere around the model's bounding box; parallel fits the box itself with 5% to spare. With nothing to fit, the camera stays (no error). |
 | `{"type": "standard_view", "view": "top"}` | Looks at the target from a standard direction, from the same distance: `top` (from +Z, `up` +Y), `bottom` (from -Z, `up` +Y), `front` (from -Y, looking along +Y), `back` (from +Y), `left` (from -X), `right` (from +X), `iso` (from the front left above, along (1,1,-1)). The projection does not change. |
 | `{"type": "set_projection", "projection": "parallel"}` | Switches between `perspective` and `parallel`, keeping the eye, target and `view_height`. |
```

- [ ] **Step 4: Run the tests**

Run: `$HOME/.cargo/bin/cargo test -p session --test contract --test camera` then `$HOME/.cargo/bin/cargo build --workspace --tests`
Expected: PASS (`undo_and_redo_take_a_count`, `orbit_and_zoom_turn_and_zoom_about_a_given_point`); the workspace builds.
Then `$HOME/.cargo/bin/cargo fmt --all --check` and `$HOME/.cargo/bin/cargo clippy --workspace --all-targets -- -D warnings`: clean.

- [ ] **Step 5: Commit**

```bash
git status && git diff --stat
git add crates/api/tests/hub.rs crates/app/src/editor.rs crates/render/tests/perf.rs crates/session/src/command.rs crates/session/src/document.rs crates/session/src/document/camera.rs crates/session/tests/camera.rs crates/session/tests/commands.rs crates/session/tests/contract.rs crates/tools/src/move_tool.rs crates/tools/src/rotate.rs crates/tools/src/tape.rs docs/agent-api.md
git commit -m "M10b: undo and redo take a count; orbit and zoom about a point"
```

---

### Task 2: Arcs through three points or by bulge; relative texture rotation

**Files:**
- Modify or create: `crates/app/src/editor/texture_position.rs`
- Modify or create: `crates/app/src/info.rs`
- Modify or create: `crates/kernel/src/texture.rs`
- Modify or create: `crates/session/src/arc.rs`
- Modify or create: `crates/session/src/command.rs`
- Modify or create: `crates/session/src/document.rs`
- Modify or create: `crates/session/src/document/placement.rs`
- Modify or create: `crates/session/src/lib.rs`
- Modify or create: `crates/tools/src/arc.rs`
- Modify or create: `docs/agent-api.md`
- Test: `crates/kernel/tests/texture_placement.rs`
- Test: `crates/render/tests/golden.rs`
- Test: `crates/session/tests/curves.rs`
- Test: `crates/session/tests/texture_position.rs`

**Interfaces:**
- Consumes: Task 1's tree.
- Produces:
  - `Command::AddArc { center: Option<[f64; 3]>, start: [f64; 3], angle: Option<f64>, end: Option<[f64; 3]>, through: Option<[f64; 3]>, bulge: Option<f64>, normal: Option<[f64; 3]>, segments: Option<u32> }`.
  - `session::arc_through(start: DVec3, end: DVec3, through: DVec3) -> Option<(DVec3 /*center*/, f64 /*angle deg*/, DVec3 /*unit normal*/)>` (re-exported from `session`); private `arc::arc_spec(..) -> Result<(DVec3, DVec3, f64, Option<DVec3>), CommandError>`.
  - `kernel::PlacementEdit.rotate_by: Option<f64>` (applied after `rotation_deg`), `Command::PositionTexture.rotate_by: Option<f64>` (refused together with `rotation_deg`).
  - The Entity Info Rotate 90 button now runs one `position_texture` with `rotate_by: 90` (`Editor::rotate_texture_90` through `position_faces`).

Verify the result against the scratch tree: `git diff refs/scratch/m10b/t2 -- <paths>` is empty after the task (the tree predates the plan commit, so restrict it to the task's paths).

- [ ] **Step 1: Write the failing tests**

```diff
diff --git a/crates/kernel/tests/texture_placement.rs b/crates/kernel/tests/texture_placement.rs
index a8cdb44..78d64cf 100644
--- a/crates/kernel/tests/texture_placement.rs
+++ b/crates/kernel/tests/texture_placement.rs
@@ -38,6 +38,7 @@ fn sets(t: TexturePlacement) -> PlacementEdit {
     PlacementEdit {
         offset: Some(t.offset),
         rotation_deg: Some(t.rotation_deg),
+        rotate_by: None,
         scale: Some(t.scale),
         reset: false,
     }
@@ -114,6 +115,17 @@ fn position_texture_sets_the_given_fields_and_undo_restores_them() {
         ..PlacementEdit::default()
     };
     let (turn, n) = m.position_texture(&[f], edit).unwrap();
+    // A relative turn adds to it.
+    let mut by = m.clone();
+    let quarter = PlacementEdit {
+        rotate_by: Some(100.0),
+        ..PlacementEdit::default()
+    };
+    by.position_texture(&[f], quarter).unwrap();
+    assert_eq!(
+        by.face(f).unwrap().texture,
+        Some(placement([0.0, 0.0], 10.0, [1.0, 1.0]))
+    );
     assert_eq!(n, 1);
     assert_eq!(
         m.face(f).unwrap().texture,
diff --git a/crates/render/tests/golden.rs b/crates/render/tests/golden.rs
index e2e7361..bfd76af 100644
--- a/crates/render/tests/golden.rs
+++ b/crates/render/tests/golden.rs
@@ -830,6 +830,7 @@ fn golden_positioned_textures() {
             ids: vec![front],
             offset: None,
             rotation_deg: Some(90.0),
+            rotate_by: None,
             scale: Some([0.5, 0.5]),
             reset: false,
         },
@@ -840,6 +841,7 @@ fn golden_positioned_textures() {
             ids: vec![back],
             offset: Some([0.25, 0.0]),
             rotation_deg: None,
+            rotate_by: None,
             scale: None,
             reset: false,
         },
diff --git a/crates/session/tests/curves.rs b/crates/session/tests/curves.rs
index 61547db..19af268 100644
--- a/crates/session/tests/curves.rs
+++ b/crates/session/tests/curves.rs
@@ -273,6 +273,57 @@ fn an_arc_is_one_curve_from_its_start_around_its_center() {
     assert!((c["angle"].as_f64().unwrap() - 180.0).abs() < 1e-9);
 }
 
+#[test]
+fn an_arc_is_drawn_through_three_points_or_by_its_bulge() {
+    let mut d = doc();
+    // Start, end and a point between them on the far side: a half circle around (1, 0, 0).
+    let out = run(
+        &mut d,
+        json!({"type": "add_arc", "start": [0, 0, 0], "end": [2, 0, 0], "through": [1, 1, 0],
+               "segments": 4}),
+    )
+    .unwrap();
+    let c = curve(&d, &created(&out, 'e')[0]);
+    assert!(near(&c["center"], [1.0, 0.0, 0.0]));
+    assert!((c["radius"].as_f64().unwrap() - 1.0).abs() < 1e-9);
+    assert!((c["angle"].as_f64().unwrap() - 180.0).abs() < 1e-9);
+    let at = |v: &Value| ask(&d, json!({"type": "entity", "id": v})).unwrap()["position"].clone();
+    assert!(
+        near(&at(&c["vertices"][2]), [1.0, 1.0, 0.0]),
+        "passes through it"
+    );
+    run(&mut d, json!({"type": "undo"})).unwrap();
+
+    // The same arc by its bulge: 1 m to the left of start to end, looking down +Z.
+    run(
+        &mut d,
+        json!({"type": "add_arc", "start": [0, 0, 0], "end": [2, 0, 0], "bulge": 1, "segments": 4}),
+    )
+    .unwrap();
+    let c = curve(&d, &edges(&d)[0]["id"]);
+    assert!(near(&c["center"], [1.0, 0.0, 0.0]));
+    let at = |v: &Value| ask(&d, json!({"type": "entity", "id": v})).unwrap()["position"].clone();
+    assert!(near(&at(&c["vertices"][2]), [1.0, 1.0, 0.0]));
+
+    for bad in [
+        // Mixed or missing forms.
+        json!({"type": "add_arc", "start": [0, 0, 0], "end": [2, 0, 0]}),
+        json!({"type": "add_arc", "center": [0, 0, 0], "start": [1, 0, 0], "angle": 90, "end": [0, 1, 0]}),
+        json!({"type": "add_arc", "start": [0, 0, 0], "end": [2, 0, 0], "through": [1, 1, 0], "bulge": 1}),
+        json!({"type": "add_arc", "start": [0, 0, 0], "end": [2, 0, 0], "through": [1, 1, 0], "normal": [0, 0, 1]}),
+        // Three points on a line, or an end on the start.
+        json!({"type": "add_arc", "start": [0, 0, 0], "end": [2, 0, 0], "through": [1, 0, 0]}),
+        json!({"type": "add_arc", "start": [0, 0, 0], "end": [0, 0, 0], "through": [1, 1, 0]}),
+        // A bulge of 0, or an end off the plane of the normal.
+        json!({"type": "add_arc", "start": [0, 0, 0], "end": [2, 0, 0], "bulge": 0}),
+        json!({"type": "add_arc", "start": [0, 0, 0], "end": [2, 0, 1], "bulge": 1}),
+        json!({"type": "add_arc", "start": [0, 0, 0], "end": [2e6, 0, 0], "bulge": 1}),
+    ] {
+        let (kind, _) = run(&mut d, bad.clone()).unwrap_err();
+        assert_eq!(kind, ErrorKind::InvalidInput, "{bad}");
+    }
+}
+
 #[test]
 fn bad_arcs_are_refused() {
     let mut d = doc();
diff --git a/crates/session/tests/texture_position.rs b/crates/session/tests/texture_position.rs
index 6360423..53cbe41 100644
--- a/crates/session/tests/texture_position.rs
+++ b/crates/session/tests/texture_position.rs
@@ -141,6 +141,49 @@ fn position_texture_sets_fields_reports_them_and_is_one_undo_step() {
     );
 }
 
+#[test]
+fn rotate_by_turns_each_face_from_its_own_rotation() {
+    let mut d = doc();
+    let f = rectangle(&mut d);
+    let out = run(
+        &mut d,
+        json!({"type": "add_rectangle", "corner": [3, 0, 0], "opposite": [4, 1, 0]}),
+    )
+    .unwrap();
+    let g = out["changes"]["created"]
+        .as_array()
+        .unwrap()
+        .iter()
+        .find(|id| id.as_str().unwrap().starts_with('f'))
+        .unwrap()
+        .clone();
+    run(
+        &mut d,
+        json!({"type": "position_texture", "ids": [f], "rotation_deg": 300}),
+    )
+    .unwrap();
+    let out = run(
+        &mut d,
+        json!({"type": "position_texture", "ids": [f, g], "rotate_by": 90}),
+    )
+    .unwrap();
+    assert_eq!(out["positioned"], 2);
+    let rotation = |d: &Document, id: &Value| {
+        ask(d, json!({"type": "entity", "id": id}))["texture_position"]["rotation_deg"].clone()
+    };
+    assert_eq!(
+        (rotation(&d, &f), rotation(&d, &g)),
+        (json!(30.0), json!(90.0))
+    );
+    let (kind, message) = run(
+        &mut d,
+        json!({"type": "position_texture", "ids": [f], "rotation_deg": 10, "rotate_by": 10}),
+    )
+    .unwrap_err();
+    assert_eq!(kind, ErrorKind::InvalidInput);
+    assert_eq!(message, "give rotation_deg or rotate_by, not both");
+}
+
 #[test]
 fn bad_position_texture_commands_are_refused_and_change_nothing() {
     let mut d = doc();
```

- [ ] **Step 2: Run them to see them fail**

Run: `$HOME/.cargo/bin/cargo test -p session --test curves --test texture_position` and `$HOME/.cargo/bin/cargo test -p kernel --test texture_placement`
Expected: compile errors (`end`, `through`, `bulge`, `rotate_by` unknown).

- [ ] **Step 3: Implement**

```diff
diff --git a/crates/app/src/editor/texture_position.rs b/crates/app/src/editor/texture_position.rs
index 06de9ea..ad5a39b 100644
--- a/crates/app/src/editor/texture_position.rs
+++ b/crates/app/src/editor/texture_position.rs
@@ -1,7 +1,5 @@
 //! Entity Info's texture position fields for selected faces.
 
-use std::collections::BTreeMap;
-
 use kernel::{EntityId, TexturePlacement};
 use session::Command;
 
@@ -30,6 +28,7 @@ impl Editor {
                 ids: Vec::new(),
                 offset: Some(millimeters.map(|mm| f64::from(mm) / 1000.0)),
                 rotation_deg: None,
+                rotate_by: None,
                 scale: None,
                 reset: false,
             },
@@ -44,6 +43,7 @@ impl Editor {
                 ids: Vec::new(),
                 offset: None,
                 rotation_deg: Some(f64::from(tenths) / 10.0),
+                rotate_by: None,
                 scale: None,
                 reset: false,
             },
@@ -58,6 +58,7 @@ impl Editor {
                 ids: Vec::new(),
                 offset: None,
                 rotation_deg: None,
+                rotate_by: None,
                 scale: Some(thousandths.map(|t| f64::from(t) / 1000.0)),
                 reset: false,
             },
@@ -73,6 +74,7 @@ impl Editor {
                 ids: Vec::new(),
                 offset: None,
                 rotation_deg: None,
+                rotate_by: None,
                 scale: None,
                 reset: true,
             },
@@ -80,38 +82,20 @@ impl Editor {
     }
 
     /// Turns the texture of each selected face a further quarter counter-clockwise, as one
-    /// undo step (faces turned alike share a command in one atomic batch).
+    /// undo step.
     pub fn rotate_texture_90(&mut self) -> bool {
-        let m = self.doc.model();
-        let mut by_rotation: BTreeMap<u64, (f64, Vec<EntityId>)> = BTreeMap::new();
-        for id in self.doc.selection() {
-            let EntityId::Face(f) = id else { continue };
-            let Some(face) = m.face(*f) else { continue };
-            let r = face.texture.unwrap_or_default().rotation_deg + 90.0;
-            by_rotation
-                .entry(r.to_bits())
-                .or_insert_with(|| (r, Vec::new()))
-                .1
-                .push(*id);
-        }
-        if by_rotation.is_empty() {
-            return false;
-        }
-        let commands = by_rotation
-            .into_values()
-            .map(|(r, ids)| Command::PositionTexture {
-                ids,
+        let faces = self.doc.selection().iter().copied().collect();
+        self.position_faces(
+            faces,
+            Command::PositionTexture {
+                ids: Vec::new(),
                 offset: None,
-                rotation_deg: Some(r),
+                rotation_deg: None,
+                rotate_by: Some(90.0),
                 scale: None,
                 reset: false,
-            })
-            .collect();
-        self.message = None;
-        if let Err(e) = self.execute_batch(commands, true) {
-            self.message = Some(e.error.message);
-        }
-        true
+            },
+        )
     }
 
     /// Runs a `position_texture` command on the faces of `ids` (its `ids` filled in). False
diff --git a/crates/app/src/info.rs b/crates/app/src/info.rs
index e1d8728..5834dbe 100644
--- a/crates/app/src/info.rs
+++ b/crates/app/src/info.rs
@@ -309,9 +309,12 @@ mod tests {
         assert_eq!(entity_info(&d), ["2 edges"]);
 
         d.execute(Command::AddArc {
-            center: [10.0, 0.0, 0.0],
+            center: Some([10.0, 0.0, 0.0]),
             start: [11.0, 0.0, 0.0],
-            angle: 90.0,
+            angle: Some(90.0),
+            end: None,
+            through: None,
+            bulge: None,
             normal: None,
             segments: Some(3),
         })
diff --git a/crates/kernel/src/texture.rs b/crates/kernel/src/texture.rs
index 75f2451..f4b4173 100644
--- a/crates/kernel/src/texture.rs
+++ b/crates/kernel/src/texture.rs
@@ -291,6 +291,8 @@ pub struct PlacementEdit {
     pub offset: Option<[f64; 2]>,
     /// Any finite angle; stored from 0 up to 360.
     pub rotation_deg: Option<f64>,
+    /// Turns each face's texture further by this many degrees, after `rotation_deg`.
+    pub rotate_by: Option<f64>,
     pub scale: Option<[f64; 2]>,
     pub reset: bool,
 }
@@ -309,6 +311,9 @@ impl PlacementEdit {
         if let Some(r) = self.rotation_deg {
             t.rotation_deg = normal_angle(r);
         }
+        if let Some(r) = self.rotate_by {
+            t.rotation_deg = normal_angle(t.rotation_deg + r);
+        }
         if let Some(scale) = self.scale {
             t.scale = scale;
         }
diff --git a/crates/session/src/arc.rs b/crates/session/src/arc.rs
index 4d1cfbc..bf5edd5 100644
--- a/crates/session/src/arc.rs
+++ b/crates/session/src/arc.rs
@@ -98,10 +98,112 @@ pub fn arc_from_bulge(start: DVec3, end: DVec3, bulge: f64, normal: DVec3) -> Op
     Some((center, angle.to_degrees()))
 }
 
+/// The center, angle (degrees, counter-clockwise around the normal) and unit normal of the
+/// arc from `start` to `end` through `through`: the normal makes start, through, end run
+/// counter-clockwise, so the angle is between 0 and 360. None when the three points are
+/// (nearly) on one line or `end` is on `start`.
+pub fn arc_through(start: DVec3, end: DVec3, through: DVec3) -> Option<(DVec3, f64, DVec3)> {
+    let chord = end - start;
+    if chord.length() < 10.0 * LENGTH_TOL {
+        return None;
+    }
+    let off_line = chord.cross(through - start).length() / chord.length();
+    if !off_line.is_finite() || off_line < LENGTH_TOL {
+        return None;
+    }
+    // The circumcenter, with `through` as the origin.
+    let (a, b) = (start - through, end - through);
+    let axb = a.cross(b);
+    let center = through
+        + (b * a.length_squared() - a * b.length_squared()).cross(axb)
+            / (2.0 * axb.length_squared());
+    let normal = unit_vector((through - start).cross(end - through))?;
+    let (u, v) = (start - center, end - center);
+    let mut angle = normal.dot(u.cross(v)).atan2(u.dot(v));
+    if angle <= 0.0 {
+        angle += std::f64::consts::TAU;
+    }
+    Some((center, angle.to_degrees(), normal))
+}
+
+/// What `add_arc` was given, as the center, start, angle and normal `arc_points` takes: a
+/// `center` and `angle`; an `end` and a point it passes `through`; or an `end` and a `bulge`
+/// (the height of the arc's middle above the chord, positive to the left of start to end
+/// looking down the normal, as the Arc tool pulls it).
+pub fn arc_spec(
+    center: Option<[f64; 3]>,
+    start: [f64; 3],
+    angle: Option<f64>,
+    end: Option<[f64; 3]>,
+    through: Option<[f64; 3]>,
+    bulge: Option<f64>,
+    normal: Option<[f64; 3]>,
+) -> Result<(DVec3, DVec3, f64, Option<DVec3>), CommandError> {
+    let invalid = |m: &str| CommandError::new(ErrorKind::InvalidInput, m);
+    let start = DVec3::from(start);
+    match (center, angle, end, through, bulge) {
+        (Some(c), Some(a), None, None, None) => {
+            Ok((DVec3::from(c), start, a, normal.map(DVec3::from)))
+        }
+        (None, None, Some(e), Some(t), None) => {
+            if normal.is_some() {
+                return Err(invalid(
+                    "normal goes with center or bulge: three points set their own plane",
+                ));
+            }
+            let (c, a, n) =
+                arc_through(start, DVec3::from(e), DVec3::from(t)).ok_or_else(|| {
+                    invalid("start, through and end must be apart and not on one line")
+                })?;
+            Ok((c, start, a, Some(n)))
+        }
+        (None, None, Some(e), None, Some(b)) => {
+            let n = match normal {
+                None => DVec3::Z,
+                Some(n) => unit_vector(DVec3::from(n))
+                    .ok_or_else(|| invalid("normal must be a finite, non-zero vector"))?,
+            };
+            let end = DVec3::from(e);
+            if (end - start).dot(n).abs() > LENGTH_TOL {
+                return Err(invalid(
+                    "start and end must lie in one plane with the normal",
+                ));
+            }
+            let (c, a) = arc_from_bulge(start, end, b, n)
+                .ok_or_else(|| invalid("end must be away from start and bulge must not be 0"))?;
+            Ok((c, start, a, Some(n)))
+        }
+        _ => Err(invalid(
+            "give center and angle, or end with through or bulge",
+        )),
+    }
+}
+
 #[cfg(test)]
 mod tests {
     use super::*;
 
+    #[test]
+    fn three_points_give_the_arc_through_them() {
+        let (s, t, e) = (DVec3::X, DVec3::Y, -DVec3::X);
+        let (center, angle, normal) = arc_through(s, e, t).unwrap();
+        assert!(center.distance(DVec3::ZERO) < 1e-12);
+        assert!((angle - 180.0).abs() < 1e-9 && normal.abs_diff_eq(DVec3::Z, 1e-12));
+        // Through the other side: the same half circle, clockwise seen from above.
+        let (_, angle, normal) = arc_through(s, e, -DVec3::Y).unwrap();
+        assert!((angle - 180.0).abs() < 1e-9 && normal.abs_diff_eq(-DVec3::Z, 1e-12));
+        // A long way round: three quarters of a circle in a vertical plane.
+        let (s, e) = (DVec3::new(0.0, 0.0, 1.0), DVec3::new(0.0, 1.0, 0.0));
+        let t = DVec3::new(0.0, -1.0, 0.0);
+        let (center, angle, normal) = arc_through(s, e, t).unwrap();
+        let points = arc_points(center, s, angle, Some(normal), 3).unwrap();
+        assert!((angle - 270.0).abs() < 1e-9, "{angle}");
+        assert!(points[3].distance(e) < 1e-9);
+        assert!(points.iter().all(|p| (p.length() - 1.0).abs() < 1e-9));
+        assert!(arc_through(s, s * 2.0, s * 3.0).is_none(), "on one line");
+        assert!(arc_through(s, s, t).is_none(), "end on start");
+    }
+
     #[test]
     fn a_bulge_as_high_as_half_the_chord_is_a_half_circle() {
         let (s, e) = (DVec3::new(-1.0, 0.0, 0.0), DVec3::new(1.0, 0.0, 0.0));
diff --git a/crates/session/src/command.rs b/crates/session/src/command.rs
index 5750e91..4894d05 100644
--- a/crates/session/src/command.rs
+++ b/crates/session/src/command.rs
@@ -38,14 +38,26 @@ pub enum Command {
         #[serde(default)]
         segments: Option<u32>,
     },
-    /// Draw an arc around `center` from `start`, turning `angle` degrees counter-clockwise
-    /// around `normal` (default Z; negative angles turn clockwise), as `segments` straight
-    /// edges (default 12, 2 to 360) tagged as one curve, in one undo step. `start` must lie in
-    /// the plane through `center` with that normal.
+    /// Draw an arc, as `segments` straight edges (default 12, 2 to 360) tagged as one curve,
+    /// in one undo step, given in one of three ways: around `center` from `start`, turning
+    /// `angle` degrees counter-clockwise around `normal` (default Z; negative angles turn
+    /// clockwise; `start` must lie in the plane through `center` with that normal); from
+    /// `start` to `end` passing through `through` (the three points set the plane; no
+    /// `normal`); or from `start` to `end` with its middle `bulge` meters from the chord, to
+    /// the left of start to end looking down `normal` (default Z; negative: to the right), as
+    /// the Arc tool draws it.
     AddArc {
-        center: [f64; 3],
+        #[serde(default)]
+        center: Option<[f64; 3]>,
         start: [f64; 3],
-        angle: f64,
+        #[serde(default)]
+        angle: Option<f64>,
+        #[serde(default)]
+        end: Option<[f64; 3]>,
+        #[serde(default)]
+        through: Option<[f64; 3]>,
+        #[serde(default)]
+        bulge: Option<f64>,
         #[serde(default)]
         normal: Option<[f64; 3]>,
         #[serde(default)]
@@ -201,9 +213,10 @@ pub enum Command {
     },
     /// Move, turn and scale the texture on faces of the open context, as one undo step: on
     /// top of the projection, `offset` [u, v] in meters, `rotation_deg` counter-clockwise
-    /// seen from the face's front, and `scale` [u, v] (multipliers, 0.001 to 1000). Fields
-    /// left out stay as they are; `reset: true` first clears the placement. The outcome's
-    /// `positioned` counts the faces that changed.
+    /// seen from the face's front (or `rotate_by`: that many degrees more than each face's
+    /// own), and `scale` [u, v] (multipliers, 0.001 to 1000). Fields left out stay as they
+    /// are; `reset: true` first clears the placement. The outcome's `positioned` counts the
+    /// faces that changed.
     PositionTexture {
         ids: Vec<EntityId>,
         #[serde(default)]
@@ -211,6 +224,8 @@ pub enum Command {
         #[serde(default)]
         rotation_deg: Option<f64>,
         #[serde(default)]
+        rotate_by: Option<f64>,
+        #[serde(default)]
         scale: Option<[f64; 2]>,
         #[serde(default)]
         reset: bool,
diff --git a/crates/session/src/document.rs b/crates/session/src/document.rs
index 480c643..d2ce315 100644
--- a/crates/session/src/document.rs
+++ b/crates/session/src/document.rs
@@ -7,7 +7,7 @@ use kernel::{
     MaterialEdit, MeshId, Model, PlacementEdit, Scene, SceneChange, SoftenReport, VertexId,
 };
 
-use crate::arc::{DEFAULT_ARC_SEGMENTS, arc_points};
+use crate::arc::{DEFAULT_ARC_SEGMENTS, arc_points, arc_spec};
 use crate::camera::{Camera, Lighting};
 use crate::circle::{DEFAULT_CIRCLE_SEGMENTS, circle_points};
 use crate::command::{BatchError, Command, CommandError, ErrorKind, Outcome, SelectMode};
@@ -337,9 +337,22 @@ impl Document {
                 center,
                 start,
                 angle,
+                end,
+                through,
+                bulge,
                 normal,
                 segments,
-            } => self.add_arc_changes(*center, *start, *angle, *normal, *segments),
+            } => {
+                let points: Vec<[f64; 3]> = [Some(*start), *center, *end, *through]
+                    .into_iter()
+                    .flatten()
+                    .collect();
+                check_coordinates(&points).and_then(|()| {
+                    let (center, start, angle, normal) =
+                        arc_spec(*center, *start, *angle, *end, *through, *bulge, *normal)?;
+                    self.add_arc_changes(center, start, angle, normal, *segments)
+                })
+            }
             Command::AddGuideLine { point, direction } => {
                 self.add_guide_line_changes(*point, *direction)
             }
@@ -419,6 +432,7 @@ impl Document {
                 ids,
                 offset,
                 rotation_deg,
+                rotate_by,
                 scale,
                 reset,
             } => self.position_texture_changes(
@@ -426,6 +440,7 @@ impl Document {
                 PlacementEdit {
                     offset: *offset,
                     rotation_deg: *rotation_deg,
+                    rotate_by: *rotate_by,
                     scale: *scale,
                     reset: *reset,
                 },
@@ -621,18 +636,17 @@ impl Document {
 
     fn add_arc_changes(
         &mut self,
-        center: [f64; 3],
-        start: [f64; 3],
+        center: DVec3,
+        start: DVec3,
         angle: f64,
-        normal: Option<[f64; 3]>,
+        normal: Option<DVec3>,
         segments: Option<u32>,
     ) -> Result<SceneChange, CommandError> {
-        check_coordinates(&[center, start])?;
         let points = arc_points(
-            DVec3::from(center),
-            DVec3::from(start),
+            center,
+            start,
             angle,
-            normal.map(DVec3::from),
+            normal,
             segments.unwrap_or(DEFAULT_ARC_SEGMENTS),
         )?;
         check_coordinates(&points.iter().map(|p| p.to_array()).collect::<Vec<_>>())?;
diff --git a/crates/session/src/document/placement.rs b/crates/session/src/document/placement.rs
index 3841710..eaad83c 100644
--- a/crates/session/src/document/placement.rs
+++ b/crates/session/src/document/placement.rs
@@ -34,7 +34,13 @@ impl Document {
         if edit == PlacementEdit::default() {
             return Err(CommandError::new(
                 ErrorKind::InvalidInput,
-                "give offset, rotation_deg, scale or reset",
+                "give offset, rotation_deg, rotate_by, scale or reset",
+            ));
+        }
+        if edit.rotation_deg.is_some() && edit.rotate_by.is_some() {
+            return Err(CommandError::new(
+                ErrorKind::InvalidInput,
+                "give rotation_deg or rotate_by, not both",
             ));
         }
         let faces = self.parts(ids)?.faces;
diff --git a/crates/session/src/lib.rs b/crates/session/src/lib.rs
index 4f6ee68..998a694 100644
--- a/crates/session/src/lib.rs
+++ b/crates/session/src/lib.rs
@@ -13,7 +13,7 @@ mod rectangle;
 mod rotate;
 pub mod texture;
 
-pub use arc::{DEFAULT_ARC_SEGMENTS, arc_from_bulge, arc_points};
+pub use arc::{DEFAULT_ARC_SEGMENTS, arc_from_bulge, arc_points, arc_through};
 pub use camera::{Camera, Lighting, Projection, Projector, Ray, StandardView};
 pub use circle::{DEFAULT_CIRCLE_SEGMENTS, MAX_CIRCLE_SEGMENTS, circle_points};
 pub use command::{
diff --git a/crates/tools/src/arc.rs b/crates/tools/src/arc.rs
index 849cd6c..120931f 100644
--- a/crates/tools/src/arc.rs
+++ b/crates/tools/src/arc.rs
@@ -88,9 +88,12 @@ impl ArcTool {
             return Err(too_shallow());
         }
         Ok(Command::AddArc {
-            center: center.to_array(),
+            center: Some(center.to_array()),
             start: start.to_array(),
-            angle,
+            angle: Some(angle),
+            end: None,
+            through: None,
+            bulge: None,
             normal: Some(normal.to_array()),
             segments: Some(self.segments),
         })
@@ -223,9 +226,9 @@ impl Tool for ArcTool {
         match (self.start, self.end, self.hover) {
             (Some(_), Some(_), _) => {
                 if let Some(Command::AddArc {
-                    center,
+                    center: Some(center),
                     start,
-                    angle,
+                    angle: Some(angle),
                     normal,
                     ..
                 }) = self.bulge().and_then(|b| self.command(b).ok())
@@ -297,11 +300,12 @@ mod tests {
     /// The arc's center, start, angle and segments.
     fn parts(c: &Command) -> ([f64; 3], [f64; 3], f64, u32) {
         let Command::AddArc {
-            center,
+            center: Some(center),
             start,
-            angle,
+            angle: Some(angle),
             normal,
             segments,
+            ..
         } = c
         else {
             panic!("{c:?}");
diff --git a/docs/agent-api.md b/docs/agent-api.md
index 547c936..10b09ee 100644
--- a/docs/agent-api.md
+++ b/docs/agent-api.md
@@ -85,6 +85,8 @@ Modeling commands, each one undo step:
 | `{"type": "add_rectangle", "corner": [0,0,0], "opposite": [2,3,0]}` | A rectangle with sides along the axes. Without `normal` the corners must differ in exactly two coordinates. With `"normal": [nx,ny,nz]` it lies in the plane through `corner` with that normal; `opposite` must be in that plane. Sides then run along u = `Z x normal` (horizontal; X on horizontal planes) and v = `normal x u`, so directions are fixed only up to sign: on a plane with normal +Y, u = -X and v = +Z; with normal -Y, u = +X. |
 | `{"type": "add_circle", "center": [0,0,0], "radius": 1.5}` | A circle as a regular polygon with `segments` sides (default 24, 3 to 360), in the plane through `center` with `normal` (default `[0,0,1]`). The first corner lies along the plane's u axis (as for `add_rectangle`) and the corners run counter-clockwise around the normal. The radius must be positive, and each side (2r sin(pi/segments)) must be at least 0.01 mm, ten times the merge tolerance. Its edges form one curve (see Curves). |
 | `{"type": "add_arc", "center": [0,0,0], "start": [2,0,0], "angle": 90}` | An arc around `center` from `start`, turning `angle` degrees counter-clockwise around `normal` (default `[0,0,1]`; a negative angle turns clockwise), as `segments` equal straight edges (default 12 whatever the angle, 2 to 360) that form one curve. `start` must lie in the plane through `center` with that normal (within 0.001 mm); the radius is its distance from `center`. The angle must be more than 0 and less than 360 degrees in size (a whole circle is `add_circle`), and each segment at least 0.01 mm long. Its points snap and split like `add_edge`'s, so an arc ending on existing edges can close a face. |
+| `{"type": "add_arc", "start": [0,0,0], "end": [2,0,0], "through": [1,1,0]}` | The arc from `start` to `end` passing through `through`, the way three clicks pick it: the three points set its plane, so `normal` is not allowed (the curve query reports the normal about which start, through and end run counter-clockwise). The points must be apart and not on one line. `segments` as above. |
+| `{"type": "add_arc", "start": [0,0,0], "end": [2,0,0], "bulge": 1}` | The arc from `start` to `end` whose middle lies `bulge` meters from the chord, to the left of start-to-end looking down `normal` (default `[0,0,1]`; negative: to the right), as the Arc tool pulls it: a bulge of half the chord is a half circle. `start` and `end` must lie in one plane with the normal and apart; the bulge must not be 0. Give `center` with `angle`, or `end` with `through` or `bulge`; any other mix is `invalid_input`. |
 | `{"type": "move", "ids": ["f8"], "offset": [0, 0, 1]}` | Moves vertices, edges and faces (their vertices), instances and guides by `offset` (an instance must keep its bounding box, or its origin when it holds nothing, and a guide its point, within the coordinate limit; so must copies). Faces that share only some moved vertices stretch and may tilt but must stay flat: a move that would bend one is `invalid_input` naming that face. A move that would collapse an edge, turn a face over, make a face cross itself, push a hole out of its face or land a vertex on another is `invalid_input` naming the ids. Moved geometry does not merge with what it ends up touching. Unknown ids are `not_found`. |
 | `{"type": "copy", "ids": ["f8"], "offset": [3, 0, 0], "count": 4}` | Copies edges, faces, instances and guides `count` times (default 1, at most 1,000); copy k is shifted by `k * offset`. Copies are drawn like new edges, so they merge with what they touch: a copy has the faces its original had, made even where their sides already exist as bare edges, plus faces it closes together with existing edges. A copy drawn on a face splits it and every piece stays a face (also under an opening of the copy); elsewhere an opening in a copied face stays open and a loop the original had no face in gets none. Copied faces keep the orientation of their originals. A copied component instance places the same definition; a copied group gets a definition of its own. Vertex ids are `invalid_input`. |
 | `{"type": "rotate", "ids": ["f8"], "center": [0, 0, 0], "angle": 90}` | Turns vertices, edges and faces (their vertices), instances and guides by `angle` degrees about the line through `center` along `axis` (default `[0,0,1]`), counter-clockwise looking down the axis (right-hand rule), under the rules of `move`: faces that share only some turned vertices must stay flat, and so on; geometry turned whole keeps its shape. Multiples of 90 degrees are exact. An instance turns with its transform. The `axis` is a direction of any length; a zero or non-finite one and whole turns (0, 360, ..., or within 1e-9 degrees of one) are `invalid_input`. With `"copies": n` (1 to 1,000) the originals stay and n copies are made under the rules of `copy`, copy k turned by `k * angle` (a radial array; vertex ids are then `invalid_input`, and so is a count where a copy would complete a whole turn and land on the original, e.g. angle 90 with 4 copies). Vertices on the axis do not move, and a rotation that moves nothing changes nothing. |
@@ -264,7 +266,7 @@ The `materials` query adds `"texture": {"width": 0.6, "height": 0.3, "pixel_widt
 
 | Command | Effect |
 |---|---|
-| `{"type": "position_texture", "ids": ["f8", "f9"], "offset": [0.2, 0], "rotation_deg": 45, "scale": [2, 2]}` | Sets the texture position of the faces in `ids`, as one undo step: `offset` [u, v] moves the texture that many meters along the face's `u` and `v` (within +/-1e6 m), `rotation_deg` turns it counter-clockwise seen from the face's front, about the moved origin (any angle; kept from 0 up to 360), and `scale` [u, v] multiplies the size one copy covers along the image's own width and height (each 0.001 to 1000). Fields left out stay as they are; `"reset": true` first clears the position (then applies any fields given). At least one field or `reset` is required. Ids other than faces are `invalid_input` (naming them), as is an empty list or a value out of range; unknown ids are `not_found`. |
+| `{"type": "position_texture", "ids": ["f8", "f9"], "offset": [0.2, 0], "rotation_deg": 45, "scale": [2, 2]}` | Sets the texture position of the faces in `ids`, as one undo step: `offset` [u, v] moves the texture that many meters along the face's `u` and `v` (within +/-1e6 m), `rotation_deg` turns it counter-clockwise seen from the face's front, about the moved origin (any angle; kept from 0 up to 360), or `rotate_by` turns each face's texture that many degrees further than it was (not both), and `scale` [u, v] multiplies the size one copy covers along the image's own width and height (each 0.001 to 1000). Fields left out stay as they are; `"reset": true` first clears the position (then applies any fields given). At least one field or `reset` is required. Ids other than faces are `invalid_input` (naming them), as is an empty list or a value out of range; unknown ids are `not_found`. |
 
 The outcome reports `"positioned": n`, the faces whose position changed (also listed in
 `changes.modified`); a command that changes nothing makes no undo step. A face's `entity` result
@@ -287,7 +289,7 @@ every selected face when a drag ends or a typed value is entered (the faces sele
 edit began, even if the selection changed before the value was entered; if the fields go away
 first, a dragged or arrow-stepped value still applies and text still being typed is dropped),
 and Rotate 90 (turns each
-selected face's texture a further quarter counter-clockwise, as one undo step) and Reset.
+selected face's texture a further quarter counter-clockwise: `rotate_by: 90`, one undo step) and Reset.
 
 #### Lights
 
@@ -827,7 +829,7 @@ under a pixel and where a click would snap.
 | Typing into the measurements box, Escape | the commands' explicit values; `cancel`; `editor.measurement`, `editor.typing` |
 | The status bar | `editor.status` (and `prompt`, `message`, `lock`) |
 | Materials list: choose (for Paint), Add, Delete, name, color, opacity, Glow, Texture..., Remove, texture size | `set_paint_material`, `add_material`, `delete_material`, `change_material` (`name`, `color`, `opacity`, `emission`, `texture`); `materials` |
-| Entity Info: what is selected, texture Offset / Rotation / Scale, Rotate 90, Reset | `entity`, `entities`, `curve`; `position_texture` (`offset`, `rotation_deg`, `scale`, `reset`) |
+| Entity Info: what is selected, texture Offset / Rotation / Scale, Rotate 90, Reset | `entity`, `entities`, `curve`; `position_texture` (`offset`, `rotation_deg`, `rotate_by: 90`, `scale`, `reset`) |
 | Outliner: the tree, click to select, Shift/Cmd-click to toggle, double-click to open, the Model row | `outliner`; `open_path` then `select` (`toggle`); `open_path` (`[]` for the root) |
 | The window's title (file name, unsaved mark) | `file` |
 | The 3D view's size | `editor.viewport` (the default view of `pick` and `infer`); `render_view` takes any size |
```

- [ ] **Step 4: Run the tests**

Run: `$HOME/.cargo/bin/cargo test -p session` , `$HOME/.cargo/bin/cargo test -p kernel --test texture_placement`, `$HOME/.cargo/bin/cargo test -p app --bin app texture_position`
Expected: PASS, including `arc::tests::three_points_give_the_arc_through_them`, `an_arc_is_drawn_through_three_points_or_by_its_bulge`, `rotate_by_turns_each_face_from_its_own_rotation` and the unchanged `rotate_90_turns_each_face_a_further_quarter_in_one_undo_step`.
Then `$HOME/.cargo/bin/cargo fmt --all --check` and `$HOME/.cargo/bin/cargo clippy --workspace --all-targets -- -D warnings`: clean.

- [ ] **Step 5: Commit**

```bash
git status && git diff --stat
git add crates/app/src/editor/texture_position.rs crates/app/src/info.rs crates/kernel/src/texture.rs crates/kernel/tests/texture_placement.rs crates/render/tests/golden.rs crates/session/src/arc.rs crates/session/src/command.rs crates/session/src/document.rs crates/session/src/document/placement.rs crates/session/src/lib.rs crates/session/tests/curves.rs crates/session/tests/texture_position.rs crates/tools/src/arc.rs docs/agent-api.md
git commit -m "M10b: add_arc by three points or bulge; position_texture rotate_by"
```

---

### Task 3: Select by a box on screen

**Files:**
- Modify or create: `crates/api/src/dispatch.rs`
- Modify or create: `crates/api/src/lib.rs`
- Modify or create: `crates/api/src/probe.rs`
- Modify or create: `crates/tools/src/lib.rs`
- Modify or create: `crates/tools/src/select.rs`
- Modify or create: `docs/agent-api.md`
- Test: `crates/api/tests/probe.rs`

**Interfaces:**
- Consumes: Task 2's tree.
- Produces:
  - `tools::select_box(scenery: &Scenery, camera: &Camera, a: DVec2, b: DVec2, crossing: bool) -> Vec<EntityId>` (was the private `boxed(ctx, a, b)`; the Select tool passes `crossing = cursor.x < start.x`; `box_corners` takes `&Scenery`).
  - `api::ScreenBox { from: [f64; 2], to: [f64; 2], crossing: bool, width: Option<u32>, height: Option<u32> }`, `api::ScreenCommand::Select { screen_box (serde "box"), mode: SelectMode }`, `api::probe::box_ids(&Document, &ScreenBox) -> Result<Vec<EntityId>, CommandError>`.
  - `execute` with `type: "select"` and a `box` key is parsed as `ScreenCommand` and runs `Command::Select { ids, mode }`; `describe` lists the `ScreenCommand` schema.

Verify the result against the scratch tree: `git diff refs/scratch/m10b/t3 -- <paths>` is empty after the task (the tree predates the plan commit, so restrict it to the task's paths).

- [ ] **Step 1: Write the failing tests**

```diff
diff --git a/crates/api/tests/probe.rs b/crates/api/tests/probe.rs
index 80f2250..2ea03dd 100644
--- a/crates/api/tests/probe.rs
+++ b/crates/api/tests/probe.rs
@@ -247,6 +247,70 @@ fn pick_and_infer_reject_what_they_cannot_use() {
     assert_eq!(r["error"]["code"], codes::INVALID_PARAMS, "{r}");
 }
 
+#[test]
+fn select_takes_a_box_on_screen_inside_or_crossing() {
+    let mut h = boxed();
+    let r = call(
+        &mut h,
+        "execute",
+        json!({"type": "set_camera", "eye": [1, 1, 10], "target": [1, 1, 0]}),
+    );
+    assert!(r["error"].is_null(), "{r}");
+    let select = |h: &mut HeadlessHost, b: Value| {
+        let r = call(h, "execute", json!({"type": "select", "box": b}));
+        assert!(r["error"].is_null(), "{r}");
+        let ids = query(h, json!({"type": "selection"}))["ids"].clone();
+        ids.as_array()
+            .unwrap()
+            .iter()
+            .map(|id| id.as_str().unwrap()[..1].to_string())
+            .collect::<Vec<_>>()
+    };
+    let count = |ids: &[String], k: &str| ids.iter().filter(|i| *i == k).count();
+    // The whole view: every edge and face lies inside.
+    let all = select(
+        &mut h,
+        json!({"from": [0, 0], "to": [100, 100], "width": 100, "height": 100}),
+    );
+    assert_eq!((count(&all, "e"), count(&all, "f")), (12, 6), "{all:?}");
+    // A small box in the middle holds nothing whole, but crosses into the top and bottom faces.
+    let small = json!({"from": [55, 55], "to": [45, 45], "width": 100, "height": 100});
+    assert!(select(&mut h, small.clone()).is_empty());
+    let mut crossing = small;
+    crossing["crossing"] = json!(true);
+    let touched = select(&mut h, crossing);
+    assert_eq!(
+        (count(&touched, "e"), count(&touched, "f")),
+        (0, 2),
+        "{touched:?}"
+    );
+    // `mode` combines it with the selection; a box outside the view or with ids is refused.
+    let r = call(
+        &mut h,
+        "execute",
+        json!({"type": "select", "box": {"from": [0, 0], "to": [100, 100], "width": 100,
+               "height": 100}, "mode": "remove"}),
+    );
+    assert!(r["error"].is_null(), "{r}");
+    assert_eq!(
+        query(&mut h, json!({"type": "selection"}))["ids"],
+        json!([])
+    );
+    let r = call(
+        &mut h,
+        "execute",
+        json!({"type": "select", "box": {"from": [0, 0], "to": [101, 100], "width": 100,
+               "height": 100}}),
+    );
+    assert_eq!(r["error"]["data"]["kind"], "invalid_input", "{r}");
+    let r = call(
+        &mut h,
+        "execute",
+        json!({"type": "select", "ids": [], "box": {"from": [0, 0], "to": [1, 1]}}),
+    );
+    assert_eq!(r["error"]["code"], codes::INVALID_PARAMS, "{r}");
+}
+
 #[test]
 fn describe_gives_the_view_queries_schemas() {
     let d = api::describe();
```

- [ ] **Step 2: Run them to see them fail**

Run: `$HOME/.cargo/bin/cargo test -p api --test probe select_takes_a_box`
Expected: FAIL (`unknown field box`, -32602).

- [ ] **Step 3: Implement**

```diff
diff --git a/crates/api/src/dispatch.rs b/crates/api/src/dispatch.rs
index b6974f0..a8cafe0 100644
--- a/crates/api/src/dispatch.rs
+++ b/crates/api/src/dispatch.rs
@@ -11,7 +11,9 @@ use session::{
 };
 
 use crate::editor::{EDITOR_COMMANDS, EditorCommand, EditorState};
-use crate::probe::{InferQuery, Inferred, PickHit, PickQuery, View, infer_query, pick_query};
+use crate::probe::{
+    InferQuery, Inferred, PickHit, PickQuery, ScreenCommand, View, box_ids, infer_query, pick_query,
+};
 use crate::protocol::{RpcError, codes, failure, parse, success};
 
 /// Largest image side `render_view` accepts, in pixels.
@@ -119,6 +121,16 @@ fn call(host: &mut dyn Host, method: &str, params: Value) -> Result<Value, RpcEr
             host.editor_command(command).map_err(command_failed)?;
             Ok(json!({"editor": host.editor(), "revision": host.document().revision()}))
         }
+        "execute" if type_of(&params) == Some("select") && params.get("box").is_some() => {
+            let ScreenCommand::Select { screen_box, mode } = params_as(params)?;
+            let ids = box_ids(host.document(), &screen_box).map_err(command_failed)?;
+            let outcome = host
+                .execute(Command::Select { ids, mode })
+                .map_err(command_failed)?;
+            let mut result = json!(outcome);
+            result["revision"] = json!(host.document().revision());
+            Ok(result)
+        }
         "execute" => {
             let command: Command = params_as(params)?;
             let outcome = host.execute(command).map_err(command_failed)?;
@@ -260,7 +272,7 @@ pub fn describe() -> Value {
         "units": "meters; Z is up; angles in degrees",
         "methods": {
             "execute": {
-                "params": "Command or EditorCommand (EditorCommand result: {editor: EditorState, revision})",
+                "params": "Command, ScreenCommand (select by a box on screen) or EditorCommand (EditorCommand result: {editor: EditorState, revision})",
                 "result": "{changes: ChangeSummary, import?: ImportReport, export?: ExportReport, soften?: SoftenReport, positioned?: integer, revision}: the Outcome plus revision"
             },
             "batch": {
@@ -304,6 +316,7 @@ pub fn describe() -> Value {
             "EditorQuery": schema_for!(EditorQuery),
             "PickHit": schema_for!(PickHit),
             "Inferred": schema_for!(Inferred),
+            "ScreenCommand": schema_for!(ScreenCommand),
             "EditorCommand": schema_for!(EditorCommand),
             "EditorState": schema_for!(EditorState),
             "QueryResult": schema_for!(QueryResult),
diff --git a/crates/api/src/lib.rs b/crates/api/src/lib.rs
index 73335be..7d1571e 100644
--- a/crates/api/src/lib.rs
+++ b/crates/api/src/lib.rs
@@ -13,6 +13,9 @@ pub use dispatch::{EditorQuery, Host, describe, handle};
 pub use editor::{Dialog, EditorCommand, EditorState, PhaseName, ToolName};
 pub use headless::HeadlessHost;
 pub use hub::{Hub, Inbox};
-pub use probe::{InferQuery, Inferred, PickHit, PickQuery, Placed, RaySpec, Snap, View};
+pub use probe::{
+    AxisName, InferQuery, Inferred, PickHit, PickQuery, Placed, RaySpec, ScreenBox, ScreenCommand,
+    Snap, View,
+};
 pub use protocol::{MAX_LINE_BYTES, codes};
 pub use transport::{listen_unix, serve_stdio, serve_stream};
diff --git a/crates/api/src/probe.rs b/crates/api/src/probe.rs
index fab61a8..ae9e248 100644
--- a/crates/api/src/probe.rs
+++ b/crates/api/src/probe.rs
@@ -7,10 +7,11 @@ use glam::{DVec2, DVec3};
 use kernel::EntityId;
 use schemars::JsonSchema;
 use serde::{Deserialize, Serialize};
+use session::SelectMode;
 use session::{
     Camera, CommandError, Document, ErrorKind, Projection, Ray, check_coordinates, unit_vector,
 };
-use tools::{Axis, Guides, Reach, Scenery, SnapKind, infer_detail, pick};
+use tools::{Axis, Guides, Reach, Scenery, SnapKind, infer_detail, pick, select_box};
 
 use crate::dispatch::MAX_IMAGE_SIDE;
 
@@ -155,6 +156,46 @@ pub enum AxisName {
     Z,
 }
 
+/// A box on screen, as the Select tool drags it.
+#[derive(Clone, Debug, PartialEq, Deserialize, JsonSchema)]
+#[serde(deny_unknown_fields)]
+pub struct ScreenBox {
+    /// One corner, in pixels from the view's top left (x right, y down).
+    pub from: [f64; 2],
+    /// The opposite corner.
+    pub to: [f64; 2],
+    /// Select what the box touches (the window's box dragged left), not only what lies
+    /// entirely inside it (dragged right). Default false.
+    #[serde(default)]
+    pub crossing: bool,
+    /// The view's size in pixels (both or neither; default: the document camera's viewport).
+    pub width: Option<u32>,
+    pub height: Option<u32>,
+}
+
+/// Commands the API resolves from the view before running them as session commands.
+#[derive(Clone, Debug, PartialEq, Deserialize, JsonSchema)]
+#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
+pub enum ScreenCommand {
+    /// `select` by a box on screen: what the Select tool's box selects of the open context,
+    /// seen by the document camera, combined with the selection by `mode`.
+    Select {
+        #[serde(rename = "box")]
+        screen_box: ScreenBox,
+        #[serde(default)]
+        mode: SelectMode,
+    },
+}
+
+/// The ids a screen box selects (see `ScreenCommand::Select`).
+pub fn box_ids(doc: &Document, b: &ScreenBox) -> Result<Vec<EntityId>, CommandError> {
+    let camera = view_camera(doc, None, b.width, b.height)?;
+    let from = pixel_in(&camera, b.from)?;
+    let to = pixel_in(&camera, b.to)?;
+    let (scenery, local) = (scenery(doc), local(doc, &camera));
+    Ok(select_box(&scenery, &local, from, to, b.crossing))
+}
+
 /// What the tools' inference snapped to.
 #[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
 pub struct Inferred {
diff --git a/crates/tools/src/lib.rs b/crates/tools/src/lib.rs
index ab7e18f..65d4af2 100644
--- a/crates/tools/src/lib.rs
+++ b/crates/tools/src/lib.rs
@@ -34,6 +34,6 @@ pub use push_pull::PushPullTool;
 pub use rectangle::RectangleTool;
 pub use rotate::RotateTool;
 pub use scenery::{Layout, Owner, Part, Placed, Scenery};
-pub use select::SelectTool;
+pub use select::{SelectTool, select_box};
 pub use tape::TapeMeasureTool;
 pub use tool::{Phase, Remembered, Tool, ToolContext, ToolEvent, ToolKind, ToolOutput};
diff --git a/crates/tools/src/select.rs b/crates/tools/src/select.rs
index 7d7378a..d4d9562 100644
--- a/crates/tools/src/select.rs
+++ b/crates/tools/src/select.rs
@@ -8,10 +8,10 @@
 use glam::{DVec2, DVec3};
 use kernel::geom::point_in_polygon;
 use kernel::{EntityId, Face, Guide, Model};
-use session::{Command, SelectMode};
+use session::{Camera, Command, SelectMode};
 
 use crate::inference::{edge_under_where, face_under_in, guide_under_where};
-use crate::scenery::Owner;
+use crate::scenery::{Owner, Scenery};
 use crate::tool::{Phase, Tool, ToolContext, ToolEvent, ToolOutput};
 
 #[derive(Clone, Debug, Default)]
@@ -73,11 +73,11 @@ fn partly_selected(ids: &[EntityId], ctx: &ToolContext) -> bool {
 }
 
 /// The corners of an instance's bounding box (bit 0 picks x, bit 1 y, bit 2 z).
-fn box_corners(ctx: &ToolContext, id: EntityId) -> Option<[DVec3; 8]> {
+fn box_corners(scenery: &Scenery, id: EntityId) -> Option<[DVec3; 8]> {
     let EntityId::Instance(i) = id else {
         return None;
     };
-    let [lo, hi] = ctx.scenery.instance_box(i)?;
+    let [lo, hi] = scenery.instance_box(i)?;
     Some(std::array::from_fn(|k| {
         DVec3::new(
             if k & 1 == 0 { lo.x } else { hi.x },
@@ -89,7 +89,7 @@ fn box_corners(ctx: &ToolContext, id: EntityId) -> Option<[DVec3; 8]> {
 
 /// The twelve edges of an instance's bounding box, for outlines.
 pub(crate) fn box_outline(ctx: &ToolContext, id: EntityId) -> Vec<[DVec3; 2]> {
-    let Some(c) = box_corners(ctx, id) else {
+    let Some(c) = box_corners(ctx.scenery, id) else {
         return Vec::new();
     };
     (0..8)
@@ -99,18 +99,25 @@ pub(crate) fn box_outline(ctx: &ToolContext, id: EntityId) -> Vec<[DVec3; 2]> {
         .collect()
 }
 
-/// Edges, faces, instances and guides selected by the screen box from `a` to `b`: inside it
-/// entirely when dragging right (`b.x >= a.x`), touching it when dragging left. Entities with
-/// a point behind the camera are skipped. Guide lines are never inside a box; a box dragged
-/// left selects those crossing it.
-fn boxed(ctx: &ToolContext, a: DVec2, b: DVec2) -> Vec<EntityId> {
+/// The open context's edges, faces, instances and guides a screen box with corners `a` and
+/// `b` selects, as `camera` (in the open context's coordinates) sees `scenery`: those inside it
+/// entirely, or with `crossing` those touching it (the Select tool's box dragged right, or
+/// left). Entities with a point behind the camera are skipped. Guide lines are never inside a
+/// box; a crossing box selects those crossing it.
+pub fn select_box(
+    scenery: &Scenery,
+    camera: &Camera,
+    a: DVec2,
+    b: DVec2,
+    crossing: bool,
+) -> Vec<EntityId> {
     let (lo, hi) = (a.min(b), a.max(b));
-    let window = b.x >= a.x;
-    let m = ctx.model;
+    let window = !crossing;
+    let m = scenery.open();
     let screen = |points: &[kernel::VertexId]| -> Option<Vec<DVec2>> {
         points
             .iter()
-            .map(|v| m.position(*v).and_then(|p| ctx.camera.project(p)))
+            .map(|v| m.position(*v).and_then(|p| camera.project(p)))
             .collect()
     };
     let inside = |s: DVec2| s.cmpge(lo).all() && s.cmple(hi).all();
@@ -150,12 +157,12 @@ fn boxed(ctx: &ToolContext, a: DVec2, b: DVec2) -> Vec<EntityId> {
     // screen rectangle overlaps the selection box.
     for (id, _) in m.instances() {
         let id = EntityId::Instance(id);
-        let Some(corners) = box_corners(ctx, id) else {
+        let Some(corners) = box_corners(scenery, id) else {
             continue;
         };
         let Some(pts) = corners
             .iter()
-            .map(|p| ctx.camera.project(*p))
+            .map(|p| camera.project(*p))
             .collect::<Option<Vec<DVec2>>>()
         else {
             continue;
@@ -174,13 +181,12 @@ fn boxed(ctx: &ToolContext, a: DVec2, b: DVec2) -> Vec<EntityId> {
     }
     for (id, g) in m.guides() {
         let hit = match *g {
-            Guide::Point { position } => ctx.camera.project(position).is_some_and(inside),
+            Guide::Point { position } => camera.project(position).is_some_and(inside),
             Guide::Line { point, direction } => {
                 !window
-                    && ctx
-                        .camera
+                    && camera
                         .line_span(point, direction)
-                        .and_then(|[a, b]| Some((ctx.camera.project(a)?, ctx.camera.project(b)?)))
+                        .and_then(|[a, b]| Some((camera.project(a)?, camera.project(b)?)))
                         .is_some_and(|(p, q)| touches(p, q))
             }
         };
@@ -269,7 +275,9 @@ impl Tool for SelectTool {
                     let ids = if clicked {
                         pick(ctx).map_or(Vec::new(), |id| whole_curve(ctx.model, id))
                     } else {
-                        boxed(ctx, start, ctx.cursor)
+                        // Dragged left: what the box touches.
+                        let crossing = ctx.cursor.x < start.x;
+                        select_box(ctx.scenery, ctx.camera, start, ctx.cursor, crossing)
                     };
                     let mode = if !ctx.shift {
                         SelectMode::Replace
diff --git a/docs/agent-api.md b/docs/agent-api.md
index 10b09ee..28ec567 100644
--- a/docs/agent-api.md
+++ b/docs/agent-api.md
@@ -102,6 +102,14 @@ does not change the revision. The window shows the selection and agents see what
 selected with the `selection` query. Vertex ids are `invalid_input` and unknown ids are
 `not_found`, both with `ids`. Empty `ids` with mode `replace` clears the selection.
 
+`{"type": "select", "box": {"from": [100, 80], "to": [300, 240]}, "mode": "add"}` selects by a box
+on screen instead of `ids`, as the Select tool's box does: of the open context, the edges, faces,
+instances (by their bounding boxes) and guide points lying entirely inside it, or with
+`"crossing": true` everything it touches (the window's box dragged left), guide lines too.
+Corners are pixels of a view seen by the document camera (`width` and `height`, both or neither,
+default the 3D view's size); a corner outside the view is `invalid_input`. Only `execute` takes a
+box (not `batch`).
+
 The selection drops any entity that stops existing, through erase, merges, undo or redo. Undo does
 not restore the selection. The `selection` query's `revision` is the model revision: it does not
 change when only the selection changes. Subscribers get a `selection` notification when it
@@ -823,7 +831,7 @@ under a pixel and where a click would snap.
 | Camera menu: Zoom Extents (Shift+Z), standard views, Perspective / Parallel, Night lighting | `zoom_extents`, `standard_view`, `set_projection`, `set_lighting` |
 | Orbit, pan and zoom with the mouse or trackpad, fly mode (F, Escape) and its speed | `set_camera`, `orbit`, `pan`, `zoom` reach the same views; `set_fly_mode` (`on`, `speed`), `cancel`; `editor.flying`, `editor.fly_speed`. Flying itself (moving keys, mouse look) moves the camera, which `set_camera` places directly |
 | The toolbar's tools and their keys | `set_tool`; `editor.tool`, `editor.phase`, `editor.points` |
-| Select: click, Shift-click, box (inside or crossing), double-click to open a group | `select` (`mode`), `pick` for what is under a pixel, `entities` with `within` for a box, `open_instance` / `open_path` |
+| Select: click, Shift-click, box (inside or crossing), double-click to open a group | `select` (`mode`; `box` with `crossing`), `pick` for what is under a pixel, `open_instance` / `open_path` |
 | Line, Rectangle, Circle, Arc, Push/Pull, Move (Ctrl copies, `5x`, `/5`), Rotate, Tape Measure, Erase, Paint (Cmd samples) | `add_edge`, `add_rectangle`, `add_circle` (`segments`), `add_arc`, `push_pull`, `move`, `copy` (`count`), `rotate` (`copies`), `add_guide_line` / `add_guide_point`, `erase`, `paint` (`side`); a sampled material is the `entity` query's |
 | Inference snapping, Shift and arrow-key locks, the marker under the cursor | `infer` (with `from`; `axis` is what Shift would lock) |
 | Typing into the measurements box, Escape | the commands' explicit values; `cancel`; `editor.measurement`, `editor.typing` |
```

- [ ] **Step 4: Run the tests**

Run: `$HOME/.cargo/bin/cargo test -p api --test probe` and `$HOME/.cargo/bin/cargo test -p tools`
Expected: PASS; the Select tool's own box tests unchanged.
Then `$HOME/.cargo/bin/cargo fmt --all --check` and `$HOME/.cargo/bin/cargo clippy --workspace --all-targets -- -D warnings`: clean.

- [ ] **Step 5: Commit**

```bash
git status && git diff --stat
git add crates/api/src/dispatch.rs crates/api/src/lib.rs crates/api/src/probe.rs crates/api/tests/probe.rs crates/tools/src/lib.rs crates/tools/src/select.rs docs/agent-api.md
git commit -m "M10b: select by a screen box, inside or crossing"
```

---

### Task 4: The `tool_input` method in the API

**Files:**
- Modify or create: `crates/api/src/dispatch.rs`
- Modify or create: `crates/api/src/input.rs`
- Modify or create: `crates/api/src/lib.rs`
- Modify or create: `crates/api/src/probe.rs`
- Test: `crates/api/tests/editor.rs`

**Interfaces:**
- Consumes: Task 3's tree; `api::EditorState`, `api::Snap`, `api::AxisName`.
- Produces (all re-exported from `api`):
  - Wire types: `ToolInputParams { events: Vec<InputEvent>, width: Option<u32>, height: Option<u32> }`, `InputEvent` (`move`, `down`, `up`, `click`, `double_click`, `drag`, `key`, `modifiers`, `type`), `MouseButton { Left (default), Middle, Right }`.
  - Checked types: `Spot { Pixel(DVec2), Point(DVec3) }`, `Key { Char(char), Enter, Backspace, Delete, Escape, Right, Left, Up, Down, CtrlTap }`, `Mods { shift, alt, cmd }` (ctrl folded into cmd), `Input { Move(Spot), Down(Spot, MouseButton), Up(Spot, MouseButton), Click(Spot, MouseButton), DoubleClick(Spot), Drag(Vec<Spot>, MouseButton), Key(Key, Mods), Modifiers(Mods), Type(String) }`, `ToolInput { events: Vec<Input>, view: Option<DVec2> }`; `input::check(ToolInputParams) -> Result<ToolInput, CommandError>`.
  - Results: `Hover { kind: Snap, axis: Option<AxisName>, point }`, `Issued { command: Command, outcome: Option<Outcome>, error: Option<CommandError> }`, `ToolInputResult { editor: EditorState, inference: Option<Hover>, preview: Vec<[[f64; 3]; 2]>, commands: Vec<Issued>, revision: u64 }`.
  - `Host::tool_input(&mut self, ToolInput) -> Result<ToolInputResult, CommandError>` (default refuses: "this program has no editor"); `api::snap_name(SnapKind) -> (Snap, Option<AxisName>)` (shared with `infer`); method `tool_input`; describe's `tool_input` method and `ToolInput`, `ToolInputResult` schemas.

Verify the result against the scratch tree: `git diff refs/scratch/m10b/t4 -- <paths>` is empty after the task (the tree predates the plan commit, so restrict it to the task's paths).

- [ ] **Step 1: Write the failing tests**

```diff
diff --git a/crates/api/tests/editor.rs b/crates/api/tests/editor.rs
index ad97057..a89cc11 100644
--- a/crates/api/tests/editor.rs
+++ b/crates/api/tests/editor.rs
@@ -2,7 +2,8 @@
 //! editor and against one without.
 
 use api::{
-    EditorCommand, EditorState, HeadlessHost, Host, PhaseName, ToolName, codes, describe, handle,
+    EditorCommand, EditorState, HeadlessHost, Host, Input, PhaseName, ToolInput, ToolInputResult,
+    ToolName, codes, describe, handle,
 };
 use serde_json::{Value, json};
 use session::{
@@ -14,6 +15,8 @@ struct Editing {
     inner: HeadlessHost,
     state: EditorState,
     quit: bool,
+    /// The events `tool_input` was given.
+    events: Vec<Input>,
 }
 
 impl Editing {
@@ -40,6 +43,7 @@ impl Editing {
                 dialog: None,
             },
             quit: false,
+            events: Vec::new(),
         }
     }
 }
@@ -85,6 +89,17 @@ impl Host for Editing {
         Ok(())
     }
 
+    fn tool_input(&mut self, input: ToolInput) -> Result<ToolInputResult, CommandError> {
+        self.events.extend(input.events);
+        Ok(ToolInputResult {
+            editor: self.state.clone(),
+            inference: None,
+            preview: Vec::new(),
+            commands: Vec::new(),
+            revision: self.document().revision(),
+        })
+    }
+
     fn quit(&mut self, discard: bool) -> Result<(), CommandError> {
         self.document().check_unsaved(discard)?;
         self.quit = true;
@@ -230,7 +245,15 @@ fn describe_lists_quit_and_the_editor_schemas() {
     for name in ["selection", "camera", "editor"] {
         assert!(d["notifications"][name].is_string(), "{name}");
     }
-    for name in ["EditorCommand", "EditorState", "EditorQuery"] {
+    assert!(d["methods"]["tool_input"].is_object());
+    for name in [
+        "EditorCommand",
+        "EditorState",
+        "EditorQuery",
+        "ToolInput",
+        "ToolInputResult",
+        "ScreenCommand",
+    ] {
         assert!(d["schemas"][name].is_object(), "{name}");
     }
 }
@@ -379,3 +402,37 @@ fn requests_after_an_accepted_quit_are_refused() {
     let r = call(&mut host, "quit", json!({"discard": true}));
     assert_eq!(r["error"]["message"], "the program is quitting", "{r}");
 }
+
+#[test]
+fn tool_input_reaches_the_editor_checked() {
+    let mut h = Editing::new();
+    let r = call(
+        &mut h,
+        "tool_input",
+        json!({"events": [{"type": "click", "point": [1, 2, 0]}, {"type": "type", "text": "3"}]}),
+    );
+    assert_eq!(r["result"]["editor"]["tool"], "line", "{r}");
+    assert_eq!(r["result"]["commands"], json!([]));
+    assert_eq!(h.events.len(), 2);
+    // A malformed event is -32602; one that makes no sense is invalid_input, and runs nothing.
+    let r = call(
+        &mut h,
+        "tool_input",
+        json!({"events": [{"type": "hover", "pixel": [1, 1]}]}),
+    );
+    assert_eq!(r["error"]["code"], codes::INVALID_PARAMS, "{r}");
+    let r = call(
+        &mut h,
+        "tool_input",
+        json!({"events": [{"type": "move", "pixel": [1, 1]}, {"type": "key", "key": "tab"}]}),
+    );
+    assert_eq!(r["error"]["message"], "event 1: unknown key \"tab\"", "{r}");
+    assert_eq!(h.events.len(), 2);
+    // Without an editor there is nothing to drive.
+    let r = call(
+        &mut HeadlessHost::new(),
+        "tool_input",
+        json!({"events": [{"type": "click", "pixel": [1, 1]}]}),
+    );
+    assert_eq!(r["error"]["message"], "this program has no editor", "{r}");
+}
```

- [ ] **Step 2: Run them to see them fail**

Run: `$HOME/.cargo/bin/cargo test -p api --test editor`
Expected: compile errors (`Input`, `ToolInput`, `ToolInputResult` not found in `api`).

- [ ] **Step 3: Implement**

```diff
diff --git a/crates/api/src/dispatch.rs b/crates/api/src/dispatch.rs
index a8cafe0..57f0c81 100644
--- a/crates/api/src/dispatch.rs
+++ b/crates/api/src/dispatch.rs
@@ -11,6 +11,7 @@ use session::{
 };
 
 use crate::editor::{EDITOR_COMMANDS, EditorCommand, EditorState};
+use crate::input::{ToolInput, ToolInputParams, ToolInputResult, check};
 use crate::probe::{
     InferQuery, Inferred, PickHit, PickQuery, ScreenCommand, View, box_ids, infer_query, pick_query,
 };
@@ -46,6 +47,11 @@ pub trait Host {
     fn editor_command(&mut self, _command: EditorCommand) -> Result<(), CommandError> {
         Err(no_editor())
     }
+    /// Feeds the active tool the events, as the window would, and reports the editor after
+    /// them and the commands they ran.
+    fn tool_input(&mut self, _input: ToolInput) -> Result<ToolInputResult, CommandError> {
+        Err(no_editor())
+    }
     /// Ends the program once the replies waiting are sent. Refuses with `unsaved_changes`
     /// when the model changed since it was opened, saved or started, unless `discard`.
     fn quit(&mut self, _discard: bool) -> Result<(), CommandError> {
@@ -166,6 +172,12 @@ fn call(host: &mut dyn Host, method: &str, params: Value) -> Result<Value, RpcEr
             Ok(json!(result))
         }
         "render_view" => render_view(host, params),
+        "tool_input" => {
+            let p: ToolInputParams = params_as(params)?;
+            let input = check(p).map_err(command_failed)?;
+            let result = host.tool_input(input).map_err(command_failed)?;
+            Ok(json!(result))
+        }
         "quit" => {
             #[derive(Deserialize)]
             #[serde(deny_unknown_fields)]
@@ -289,6 +301,11 @@ pub fn describe() -> Value {
                 "result": "{width, height, png_base64}",
                 "notes": "does not move the document camera or change its lighting"
             },
+            "tool_input": {
+                "params": "ToolInput: {events: [InputEvent], width?, height?}",
+                "result": "ToolInputResult: {editor, inference, preview, commands: [{command, outcome?, error?}], revision}",
+                "notes": "runs the events through the window's editor and active tool (inference, locks, typed values); a model point is looked for where the view shows it at its turn; an event that cannot run stops the rest (the ones before it stay done); modifiers are let go at the end"
+            },
             "subscribe": {"params": null, "result": "{revision}", "notes": "connection only"},
             "describe": {"params": null, "result": "this document"},
             "quit": {
@@ -319,6 +336,8 @@ pub fn describe() -> Value {
             "ScreenCommand": schema_for!(ScreenCommand),
             "EditorCommand": schema_for!(EditorCommand),
             "EditorState": schema_for!(EditorState),
+            "ToolInput": schema_for!(ToolInputParams),
+            "ToolInputResult": schema_for!(ToolInputResult),
             "QueryResult": schema_for!(QueryResult),
             "Outcome": schema_for!(Outcome),
             "CommandError": schema_for!(CommandError),
diff --git a/crates/api/src/input.rs b/crates/api/src/input.rs
new file mode 100644
index 0000000..2056cd2
--- /dev/null
+++ b/crates/api/src/input.rs
@@ -0,0 +1,463 @@
+//! `tool_input`: the pointer and keyboard events a user would make in the 3D view, fed to the
+//! active tool through the editor the window uses (its inference, locks and typed values).
+//! The API checks their shape; the host runs them.
+
+use glam::{DVec2, DVec3};
+use schemars::JsonSchema;
+use serde::{Deserialize, Serialize};
+use session::{Command, CommandError, ErrorKind, Outcome, check_coordinates};
+
+use crate::dispatch::MAX_IMAGE_SIDE;
+use crate::editor::EditorState;
+use crate::probe::{AxisName, Snap};
+
+/// The `tool_input` method's params.
+#[derive(Clone, Debug, PartialEq, Deserialize, JsonSchema)]
+#[serde(deny_unknown_fields)]
+pub struct ToolInputParams {
+    /// Run in order, each as the window would handle it.
+    pub events: Vec<InputEvent>,
+    /// The size in pixels of the view the events' pixels are in (both or neither; default
+    /// the 3D view's size, `editor.viewport`). Pixels are scaled to the 3D view, which must
+    /// have the same proportions.
+    pub width: Option<u32>,
+    pub height: Option<u32>,
+}
+
+/// A mouse button.
+#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, JsonSchema)]
+#[serde(rename_all = "snake_case")]
+pub enum MouseButton {
+    /// The tools' button (with `alt` held: orbit).
+    #[default]
+    Left,
+    /// Orbits about the point under the cursor (with `shift`: pans).
+    Middle,
+    Right,
+}
+
+/// One thing a user does. Pointer events take a `pixel` of the view or a model `point` (open
+/// context coordinates), looked for where the view shows it at the event's turn; the tools then
+/// infer what is near that pixel, as for a click there.
+#[derive(Clone, Debug, PartialEq, Deserialize, JsonSchema)]
+#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
+pub enum InputEvent {
+    /// The cursor moves there: the tool's preview and the inference follow.
+    Move {
+        pixel: Option<[f64; 2]>,
+        point: Option<[f64; 3]>,
+    },
+    /// The button goes down there.
+    Down {
+        pixel: Option<[f64; 2]>,
+        point: Option<[f64; 3]>,
+        #[serde(default)]
+        button: MouseButton,
+    },
+    /// The button comes up there.
+    Up {
+        pixel: Option<[f64; 2]>,
+        point: Option<[f64; 3]>,
+        #[serde(default)]
+        button: MouseButton,
+    },
+    /// Move, down and up in one place.
+    Click {
+        pixel: Option<[f64; 2]>,
+        point: Option<[f64; 3]>,
+        #[serde(default)]
+        button: MouseButton,
+    },
+    /// Two left clicks in one place, then the double-click (the Select tool opens a group).
+    DoubleClick {
+        pixel: Option<[f64; 2]>,
+        point: Option<[f64; 3]>,
+    },
+    /// Down at the first place, moving through the others, up at the last (at least two):
+    /// `pixels` or `points`.
+    Drag {
+        #[serde(default)]
+        pixels: Vec<[f64; 2]>,
+        #[serde(default)]
+        points: Vec<[f64; 3]>,
+        #[serde(default)]
+        button: MouseButton,
+    },
+    /// A key pressed and released with the modifiers given held: one character (an uppercase
+    /// letter holds Shift: "G" makes a component), or `enter`, `backspace`, `delete`,
+    /// `escape`, `space`, `right` / `left` / `up` (lock the red / green / blue axis), `down`
+    /// (unlock), or `ctrl` (Ctrl tapped alone: Move and Rotate switch copying, the Tape Measure
+    /// guide making). With `cmd` (or `ctrl`) only `z` (undo; with `shift` redo) is taken.
+    Key {
+        key: String,
+        #[serde(default)]
+        shift: bool,
+        #[serde(default)]
+        alt: bool,
+        #[serde(default)]
+        ctrl: bool,
+        #[serde(default)]
+        cmd: bool,
+    },
+    /// Hold exactly these modifiers until the next `modifiers` event or the end of the call
+    /// (then all are let go): Shift locks the direction the cursor snaps along, Cmd or Ctrl
+    /// makes a Paint click sample, Shift toggles in the Select tool.
+    Modifiers {
+        #[serde(default)]
+        shift: bool,
+        #[serde(default)]
+        alt: bool,
+        #[serde(default)]
+        ctrl: bool,
+        #[serde(default)]
+        cmd: bool,
+    },
+    /// Text typed into the measurements box (replacing anything typed), then Enter: lengths,
+    /// "2,3", "24s", "5x", "/5", angles.
+    Type { text: String },
+}
+
+/// Where a pointer event happens: a pixel of the requested view, or a model point.
+#[derive(Clone, Copy, Debug, PartialEq)]
+pub enum Spot {
+    Pixel(DVec2),
+    Point(DVec3),
+}
+
+/// A key the editor takes.
+#[derive(Clone, Copy, Debug, PartialEq, Eq)]
+pub enum Key {
+    /// A character, its case already set by Shift.
+    Char(char),
+    Enter,
+    Backspace,
+    Delete,
+    Escape,
+    /// The arrow keys: lock the red, green or blue axis, or unlock.
+    Right,
+    Left,
+    Up,
+    Down,
+    /// Ctrl pressed and released alone.
+    CtrlTap,
+}
+
+/// Modifier keys held. Ctrl and Cmd act alike (the window takes either).
+#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
+pub struct Mods {
+    pub shift: bool,
+    pub alt: bool,
+    pub cmd: bool,
+}
+
+/// A checked event.
+#[derive(Clone, Debug, PartialEq)]
+pub enum Input {
+    Move(Spot),
+    Down(Spot, MouseButton),
+    Up(Spot, MouseButton),
+    Click(Spot, MouseButton),
+    DoubleClick(Spot),
+    Drag(Vec<Spot>, MouseButton),
+    Key(Key, Mods),
+    Modifiers(Mods),
+    Type(String),
+}
+
+/// Checked `tool_input` params, for the host to run.
+#[derive(Clone, Debug, PartialEq)]
+pub struct ToolInput {
+    pub events: Vec<Input>,
+    /// The view the pixels are in; None: the 3D view itself.
+    pub view: Option<DVec2>,
+}
+
+/// What the cursor snaps to, as the window's marker shows it.
+#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
+pub struct Hover {
+    pub kind: Snap,
+    /// For `axis`: the axis the point is held to.
+    #[serde(skip_serializing_if = "Option::is_none")]
+    pub axis: Option<AxisName>,
+    /// In the open context's coordinates.
+    pub point: [f64; 3],
+}
+
+/// A command the events made the editor run, with its outcome or why it failed.
+#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
+pub struct Issued {
+    pub command: Command,
+    #[serde(skip_serializing_if = "Option::is_none")]
+    pub outcome: Option<Outcome>,
+    #[serde(skip_serializing_if = "Option::is_none")]
+    pub error: Option<CommandError>,
+}
+
+/// The `tool_input` result: the editor after the events.
+#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
+pub struct ToolInputResult {
+    /// The tool, its phase and points, status bar and measurements box.
+    pub editor: EditorState,
+    /// What the cursor snaps to now (null while flying).
+    pub inference: Option<Hover>,
+    /// The tool's rubber-band geometry: segments in the open context's coordinates.
+    pub preview: Vec<[[f64; 3]; 2]>,
+    /// Every command the events ran, in order (tools, shortcuts, undo).
+    pub commands: Vec<Issued>,
+    pub revision: u64,
+}
+
+fn invalid(message: impl Into<String>) -> CommandError {
+    CommandError::new(ErrorKind::InvalidInput, message)
+}
+
+/// `params` checked: each event's shape, points within the coordinate limit, key names.
+/// Pixels are checked against the view by the host.
+pub fn check(params: ToolInputParams) -> Result<ToolInput, CommandError> {
+    let view = match (params.width, params.height) {
+        (None, None) => None,
+        (Some(w), Some(h)) => {
+            let sides = 1..=MAX_IMAGE_SIDE;
+            if !sides.contains(&w) || !sides.contains(&h) {
+                return Err(invalid(format!(
+                    "width and height must be between 1 and {MAX_IMAGE_SIDE}"
+                )));
+            }
+            Some(DVec2::new(w as f64, h as f64))
+        }
+        _ => return Err(invalid("give both width and height, or neither")),
+    };
+    let events = params
+        .events
+        .into_iter()
+        .enumerate()
+        .map(|(i, e)| event(e).map_err(|e| invalid(format!("event {i}: {}", e.message))))
+        .collect::<Result<_, _>>()?;
+    Ok(ToolInput { events, view })
+}
+
+fn event(e: InputEvent) -> Result<Input, CommandError> {
+    Ok(match e {
+        InputEvent::Move { pixel, point } => Input::Move(spot(pixel, point)?),
+        InputEvent::Down {
+            pixel,
+            point,
+            button,
+        } => Input::Down(spot(pixel, point)?, button),
+        InputEvent::Up {
+            pixel,
+            point,
+            button,
+        } => Input::Up(spot(pixel, point)?, button),
+        InputEvent::Click {
+            pixel,
+            point,
+            button,
+        } => Input::Click(spot(pixel, point)?, button),
+        InputEvent::DoubleClick { pixel, point } => Input::DoubleClick(spot(pixel, point)?),
+        InputEvent::Drag {
+            pixels,
+            points,
+            button,
+        } => {
+            let path: Vec<Spot> = match (pixels.is_empty(), points.is_empty()) {
+                (false, true) => pixels
+                    .into_iter()
+                    .map(|p| spot(Some(p), None))
+                    .collect::<Result<_, _>>()?,
+                (true, false) => points
+                    .into_iter()
+                    .map(|p| spot(None, Some(p)))
+                    .collect::<Result<_, _>>()?,
+                _ => return Err(invalid("give pixels or points")),
+            };
+            if path.len() < 2 {
+                return Err(invalid("a drag needs at least two places"));
+            }
+            Input::Drag(path, button)
+        }
+        InputEvent::Key {
+            key,
+            shift,
+            alt,
+            ctrl,
+            cmd,
+        } => {
+            let mods = Mods {
+                shift,
+                alt,
+                cmd: cmd || ctrl,
+            };
+            let (key, mods) = key_named(&key, mods)?;
+            if mods.cmd && !matches!(key, Key::Char('z' | 'Z')) {
+                return Err(invalid(
+                    "with cmd or ctrl only z (undo, with shift redo) is taken; file actions \
+                     are commands",
+                ));
+            }
+            Input::Key(key, mods)
+        }
+        InputEvent::Modifiers {
+            shift,
+            alt,
+            ctrl,
+            cmd,
+        } => Input::Modifiers(Mods {
+            shift,
+            alt,
+            cmd: cmd || ctrl,
+        }),
+        InputEvent::Type { text } => {
+            if text.is_empty() || text.chars().any(char::is_control) {
+                return Err(invalid(
+                    "text must be non-empty, without control characters",
+                ));
+            }
+            Input::Type(text)
+        }
+    })
+}
+
+/// A pixel (finite, not negative) or a point within the coordinate limit; exactly one.
+fn spot(pixel: Option<[f64; 2]>, point: Option<[f64; 3]>) -> Result<Spot, CommandError> {
+    match (pixel, point) {
+        (Some(p), None) => {
+            let p = DVec2::from(p);
+            if p.is_finite() && p.x >= 0.0 && p.y >= 0.0 {
+                Ok(Spot::Pixel(p))
+            } else {
+                Err(invalid("a pixel must be finite and not negative"))
+            }
+        }
+        (None, Some(p)) => {
+            check_coordinates(&[p])?;
+            Ok(Spot::Point(DVec3::from(p)))
+        }
+        _ => Err(invalid("give a pixel or a point")),
+    }
+}
+
+/// The key `name` names, with Shift added for an uppercase letter and the letter's case set
+/// by Shift (as the window does, so Caps Lock never matters).
+fn key_named(name: &str, mut mods: Mods) -> Result<(Key, Mods), CommandError> {
+    let key = match name {
+        "enter" => Key::Enter,
+        "backspace" => Key::Backspace,
+        "delete" => Key::Delete,
+        "escape" => Key::Escape,
+        "space" => Key::Char(' '),
+        "right" => Key::Right,
+        "left" => Key::Left,
+        "up" => Key::Up,
+        "down" => Key::Down,
+        "ctrl" => Key::CtrlTap,
+        _ => {
+            let mut chars = name.chars();
+            let (Some(c), None) = (chars.next(), chars.next()) else {
+                return Err(invalid(format!("unknown key {name:?}")));
+            };
+            if c.is_control() {
+                return Err(invalid(format!("unknown key {name:?}")));
+            }
+            if c.is_ascii_uppercase() {
+                mods.shift = true;
+            }
+            Key::Char(if !c.is_ascii_alphabetic() {
+                c
+            } else if mods.shift {
+                c.to_ascii_uppercase()
+            } else {
+                c.to_ascii_lowercase()
+            })
+        }
+    };
+    Ok((key, mods))
+}
+
+#[cfg(test)]
+mod tests {
+    use super::*;
+    use serde_json::json;
+
+    fn checked(v: serde_json::Value) -> Result<ToolInput, CommandError> {
+        check(serde_json::from_value(v).unwrap())
+    }
+
+    #[test]
+    fn events_are_checked_and_keys_named() {
+        let t = checked(json!({"events": [
+            {"type": "click", "pixel": [10, 20]},
+            {"type": "move", "point": [1, 2, 3]},
+            {"type": "key", "key": "G"},
+            {"type": "key", "key": "z", "shift": true, "ctrl": true},
+            {"type": "key", "key": "right"},
+            {"type": "drag", "pixels": [[0, 0], [5, 5]], "button": "middle"},
+            {"type": "type", "text": "2,3"}
+        ], "width": 640, "height": 400}))
+        .unwrap();
+        assert_eq!(t.view, Some(DVec2::new(640.0, 400.0)));
+        assert_eq!(
+            t.events[0],
+            Input::Click(Spot::Pixel(DVec2::new(10.0, 20.0)), MouseButton::Left)
+        );
+        let shift = Mods {
+            shift: true,
+            ..Mods::default()
+        };
+        assert_eq!(t.events[2], Input::Key(Key::Char('G'), shift));
+        let redo = Mods {
+            shift: true,
+            alt: false,
+            cmd: true,
+        };
+        assert_eq!(t.events[3], Input::Key(Key::Char('Z'), redo));
+        assert_eq!(t.events[4], Input::Key(Key::Right, Mods::default()));
+        assert!(matches!(t.events[5], Input::Drag(ref p, MouseButton::Middle) if p.len() == 2));
+    }
+
+    #[test]
+    fn bad_events_name_their_place() {
+        for (bad, message) in [
+            (json!({"type": "click"}), "event 0: give a pixel or a point"),
+            (
+                json!({"type": "click", "pixel": [1, 1], "point": [0, 0, 0]}),
+                "event 0: give a pixel or a point",
+            ),
+            (
+                json!({"type": "move", "pixel": [-1, 0]}),
+                "event 0: a pixel must be finite and not negative",
+            ),
+            (
+                json!({"type": "drag", "pixels": [[0, 0]]}),
+                "event 0: a drag needs at least two places",
+            ),
+            (
+                json!({"type": "key", "key": "f1"}),
+                "event 0: unknown key \"f1\"",
+            ),
+            (
+                json!({"type": "key", "key": "s", "cmd": true}),
+                "event 0: with cmd or ctrl only z (undo, with shift redo) is taken; file \
+                 actions are commands",
+            ),
+            (
+                json!({"type": "type", "text": ""}),
+                "event 0: text must be non-empty, without control characters",
+            ),
+        ] {
+            let e = checked(json!({"events": [bad]})).unwrap_err();
+            assert_eq!(
+                (e.kind, e.message.as_str()),
+                (ErrorKind::InvalidInput, message)
+            );
+        }
+        let e = checked(json!({"events": [], "width": 10})).unwrap_err();
+        assert_eq!(e.message, "give both width and height, or neither");
+        let e = checked(json!({"events": [{"type": "move", "point": [2e6, 0, 0]}]})).unwrap_err();
+        assert!(
+            e.message.starts_with("event 0: coordinates"),
+            "{}",
+            e.message
+        );
+    }
+}
diff --git a/crates/api/src/lib.rs b/crates/api/src/lib.rs
index 7d1571e..b467de3 100644
--- a/crates/api/src/lib.rs
+++ b/crates/api/src/lib.rs
@@ -5,6 +5,7 @@ mod dispatch;
 mod editor;
 mod headless;
 mod hub;
+mod input;
 mod probe;
 mod protocol;
 mod transport;
@@ -13,9 +14,13 @@ pub use dispatch::{EditorQuery, Host, describe, handle};
 pub use editor::{Dialog, EditorCommand, EditorState, PhaseName, ToolName};
 pub use headless::HeadlessHost;
 pub use hub::{Hub, Inbox};
+pub use input::{
+    Hover, Input, InputEvent, Issued, Key, Mods, MouseButton, Spot, ToolInput, ToolInputParams,
+    ToolInputResult,
+};
 pub use probe::{
     AxisName, InferQuery, Inferred, PickHit, PickQuery, Placed, RaySpec, ScreenBox, ScreenCommand,
-    Snap, View,
+    Snap, View, snap_name,
 };
 pub use protocol::{MAX_LINE_BYTES, codes};
 pub use transport::{listen_unix, serve_stdio, serve_stream};
diff --git a/crates/api/src/probe.rs b/crates/api/src/probe.rs
index ae9e248..7e21945 100644
--- a/crates/api/src/probe.rs
+++ b/crates/api/src/probe.rs
@@ -348,6 +348,32 @@ pub fn pick_query(doc: &Document, q: &PickQuery) -> Result<Vec<PickHit>, Command
         .collect())
 }
 
+/// The name agents see for a snap, and the axis of an axis snap.
+pub fn snap_name(kind: SnapKind) -> (Snap, Option<AxisName>) {
+    match kind {
+        SnapKind::Endpoint => (Snap::Endpoint, None),
+        SnapKind::Midpoint => (Snap::Midpoint, None),
+        SnapKind::Origin => (Snap::Origin, None),
+        SnapKind::GuidePoint => (Snap::GuidePoint, None),
+        SnapKind::Intersection => (Snap::Intersection, None),
+        SnapKind::OnEdge => (Snap::OnEdge, None),
+        SnapKind::OnGuide => (Snap::OnGuide, None),
+        SnapKind::Axis(a) => (
+            Snap::Axis,
+            Some(match a {
+                Axis::X => AxisName::X,
+                Axis::Y => AxisName::Y,
+                Axis::Z => AxisName::Z,
+            }),
+        ),
+        SnapKind::Parallel => (Snap::Parallel, None),
+        SnapKind::Perpendicular => (Snap::Perpendicular, None),
+        SnapKind::OnFace => (Snap::OnFace, None),
+        SnapKind::Ground => (Snap::Ground, None),
+        SnapKind::Free => (Snap::Nothing, None),
+    }
+}
+
 /// Answers `infer`: what a tool's click at the pixel (or where the view shows the point)
 /// would snap to.
 pub fn infer_query(doc: &Document, q: &InferQuery) -> Result<Inferred, CommandError> {
@@ -373,28 +399,7 @@ pub fn infer_query(doc: &Document, q: &InferQuery) -> Result<Inferred, CommandEr
         ..Guides::default()
     };
     let (hit, involved) = infer_detail(&scenery, &local, cursor, &guides, radius_px);
-    let (kind, axis) = match hit.kind {
-        SnapKind::Endpoint => (Snap::Endpoint, None),
-        SnapKind::Midpoint => (Snap::Midpoint, None),
-        SnapKind::Origin => (Snap::Origin, None),
-        SnapKind::GuidePoint => (Snap::GuidePoint, None),
-        SnapKind::Intersection => (Snap::Intersection, None),
-        SnapKind::OnEdge => (Snap::OnEdge, None),
-        SnapKind::OnGuide => (Snap::OnGuide, None),
-        SnapKind::Axis(a) => (
-            Snap::Axis,
-            Some(match a {
-                Axis::X => AxisName::X,
-                Axis::Y => AxisName::Y,
-                Axis::Z => AxisName::Z,
-            }),
-        ),
-        SnapKind::Parallel => (Snap::Parallel, None),
-        SnapKind::Perpendicular => (Snap::Perpendicular, None),
-        SnapKind::OnFace => (Snap::OnFace, None),
-        SnapKind::Ground => (Snap::Ground, None),
-        SnapKind::Free => (Snap::Nothing, None),
-    };
+    let (kind, axis) = snap_name(hit.kind);
     Ok(Inferred {
         kind,
         axis,
```

- [ ] **Step 4: Run the tests**

Run: `$HOME/.cargo/bin/cargo test -p api`
Expected: PASS, including `input::tests::*` and `tool_input_reaches_the_editor_checked`.
Then `$HOME/.cargo/bin/cargo fmt --all --check` and `$HOME/.cargo/bin/cargo clippy --workspace --all-targets -- -D warnings`: clean.

- [ ] **Step 5: Commit**

```bash
git status && git diff --stat
git add crates/api/src/dispatch.rs crates/api/src/input.rs crates/api/src/lib.rs crates/api/src/probe.rs crates/api/tests/editor.rs
git commit -m "M10b: tool_input method: events checked, run by the host"
```

---

### Task 5: The editor runs `tool_input`

**Files:**
- Modify or create: `crates/app/src/editor.rs`
- Modify or create: `crates/app/src/editor/input.rs`
- Modify or create: `crates/app/src/host.rs`
- Modify or create: `docs/agent-api.md`
- Test: `crates/app/src/host/tool_input_tests.rs`

**Interfaces:**
- Consumes: Task 4's `api::{ToolInput, Input, Spot, Key, Mods, MouseButton, ToolInputResult, Hover, Issued, snap_name}`.
- Produces:
  - `Editor.issued: Option<Vec<(Command, Result<Outcome, CommandError>)>>`, `Editor::execute_recorded(&mut self, Command) -> Result<Outcome, CommandError>` (used by `run` and by `apply`'s redo after a refused array), so every command the Editor runs is noted while `tool_input` runs.
  - `crates/app/src/editor/input.rs`: `pub struct Driven { issued, hover: Option<Inference>, preview: Vec<[DVec3; 2]> }`, `Editor::drive(&mut self, ToolInput) -> Result<Driven, CommandError>`; private `input`, `move_to`, `hold`, `press`, `button_of`, `editor_key`.
  - `AppHost::tool_input` (window and headless); test module `crates/app/src/host/tool_input_tests.rs` with the helpers Task 6 uses (`editor`, `call`, `ok`, `input`, `exec`, `ask`, `ran`, `vec3`, `near`, `created`, `square`, `count`, `input_in`).
  - docs/agent-api.md: the `tool_input` row, "Driving the tools", the window actions table.

Verify the result against the scratch tree: `git diff refs/scratch/m10b/t5 -- <paths>` is empty after the task (the tree predates the plan commit, so restrict it to the task's paths).

- [ ] **Step 1: Write the failing tests**

```diff
diff --git a/crates/app/src/host/tool_input_tests.rs b/crates/app/src/host/tool_input_tests.rs
new file mode 100644
index 0000000..914f937
--- /dev/null
+++ b/crates/app/src/host/tool_input_tests.rs
@@ -0,0 +1,212 @@
+//! `tool_input` over JSON-RPC through the app's host: every tool driven the way a user drives
+//! it in the window, with typed values, locks and modifier keys.
+
+use api::handle;
+use glam::{DVec2, DVec3};
+use serde_json::{Value, json};
+
+use super::*;
+
+fn editor() -> Editor {
+    Editor::new(DVec2::new(800.0, 600.0), 1.0)
+}
+
+/// One request through an `AppHost`; the whole reply.
+fn call(e: &mut Editor, method: &str, params: Value) -> Value {
+    let (mut offscreen, mut quit) = (Offscreen::headless(), false);
+    let mut host = AppHost {
+        editor: e,
+        offscreen: &mut offscreen,
+        quit: &mut quit,
+    };
+    let line = json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params});
+    serde_json::from_str(&handle(&mut host, &line.to_string()).unwrap()).unwrap()
+}
+
+/// The result of a request that must succeed.
+fn ok(e: &mut Editor, method: &str, params: Value) -> Value {
+    let r = call(e, method, params);
+    assert!(r["error"].is_null(), "{r}");
+    r["result"].clone()
+}
+
+/// Runs `events` and returns the result.
+fn input(e: &mut Editor, events: Value) -> Value {
+    ok(e, "tool_input", json!({"events": events}))
+}
+
+fn exec(e: &mut Editor, command: Value) -> Value {
+    ok(e, "execute", command)
+}
+
+fn ask(e: &mut Editor, query: Value) -> Value {
+    ok(e, "query", query)
+}
+
+/// The `type`s of the commands a result says were run.
+fn ran(r: &Value) -> Vec<String> {
+    r["commands"]
+        .as_array()
+        .unwrap()
+        .iter()
+        .map(|c| c["command"]["type"].as_str().unwrap().to_string())
+        .collect()
+}
+
+fn vec3(v: &Value) -> DVec3 {
+    DVec3::new(
+        v[0].as_f64().unwrap(),
+        v[1].as_f64().unwrap(),
+        v[2].as_f64().unwrap(),
+    )
+}
+
+fn near(v: &Value, p: [f64; 3]) -> bool {
+    vec3(v).distance(DVec3::from(p)) < 1e-6
+}
+
+/// The ids among `created` whose kind starts with `prefix`.
+fn created(outcome: &Value, prefix: char) -> Vec<Value> {
+    outcome["changes"]["created"]
+        .as_array()
+        .unwrap()
+        .iter()
+        .filter(|id| id.as_str().unwrap().starts_with(prefix))
+        .cloned()
+        .collect()
+}
+
+/// A 2 x 2 m square face on the ground at the origin; returns its face id.
+fn square(e: &mut Editor) -> Value {
+    let r = exec(
+        e,
+        json!({"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [2, 2, 0]}),
+    );
+    created(&r, 'f')[0].clone()
+}
+
+fn count(e: &mut Editor, kind: &str) -> u64 {
+    ask(e, json!({"type": "stats"}))[kind].as_u64().unwrap()
+}
+
+#[test]
+fn the_line_tool_draws_by_clicks_and_a_typed_length() {
+    let mut e = editor();
+    let r = input(
+        &mut e,
+        json!([
+            {"type": "click", "point": [0, 0, 0]},
+            {"type": "move", "point": [1, 0, 0]}
+        ]),
+    );
+    assert_eq!(r["editor"]["phase"], "active", "{r}");
+    assert!(near(&r["editor"]["points"][0], [0.0, 0.0, 0.0]));
+    assert_eq!(r["inference"]["kind"], "axis", "{r}");
+    assert_eq!(r["inference"]["axis"], "x");
+    assert_eq!(r["preview"].as_array().unwrap().len(), 1, "the rubber band");
+    assert_eq!(r["commands"], json!([]));
+
+    let r = input(&mut e, json!([{"type": "type", "text": "3"}]));
+    assert_eq!(ran(&r), ["add_edge"]);
+    let c = &r["commands"][0];
+    assert!(near(&c["command"]["end"], [3.0, 0.0, 0.0]), "{c}");
+    assert_eq!(created(&c["outcome"], 'e').len(), 1);
+    assert_eq!(r["revision"], 1);
+    // The chain goes on from the end; Escape ends it.
+    assert!(near(&r["editor"]["points"][1], [3.0, 0.0, 0.0]), "{r}");
+    let r = input(&mut e, json!([{"type": "key", "key": "escape"}]));
+    assert_eq!(r["editor"]["phase"], "idle");
+}
+
+#[test]
+fn keys_switch_tools_group_and_undo() {
+    let mut e = editor();
+    let face = square(&mut e);
+    let r = input(&mut e, json!([{"type": "key", "key": "r"}]));
+    assert_eq!(r["editor"]["tool"], "rectangle");
+    exec(&mut e, json!({"type": "select", "ids": [face]}));
+    // "G" (Shift+G) makes a component of the selection.
+    let r = input(&mut e, json!([{"type": "key", "key": "G"}]));
+    assert_eq!(ran(&r), ["make_component"], "{r}");
+    assert_eq!(r["commands"][0]["command"]["name"], "Component 1");
+    let r = input(
+        &mut e,
+        json!([
+            {"type": "key", "key": "z", "cmd": true},
+            {"type": "key", "key": "z", "cmd": true, "shift": true}
+        ]),
+    );
+    assert_eq!(ran(&r), ["undo", "redo"], "{r}");
+    assert_eq!(count(&mut e, "instances"), 1);
+}
+
+#[test]
+fn pixels_scale_from_the_view_given_and_bad_events_stop_the_rest() {
+    let mut e = editor();
+    let a = e.doc().camera.project(DVec3::ZERO).unwrap();
+    // The same spot in a view half the size.
+    let r = input_in(
+        &mut e,
+        json!([{"type": "click", "pixel": (a / 2.0).to_array()}]),
+        400,
+        300,
+    );
+    assert!(near(&r["editor"]["points"][0], [0.0, 0.0, 0.0]), "{r}");
+
+    let r = call(
+        &mut e,
+        "tool_input",
+        json!({"events": [{"type": "click", "pixel": [1, 1]}], "width": 400, "height": 400}),
+    );
+    assert_eq!(r["error"]["data"]["kind"], "invalid_input", "{r}");
+    // The first event runs; the second is outside the view and stops the third.
+    let r = call(
+        &mut e,
+        "tool_input",
+        json!({"events": [
+            {"type": "key", "key": "escape"},
+            {"type": "move", "pixel": [900, 10]},
+            {"type": "key", "key": "r"}
+        ]}),
+    );
+    assert_eq!(
+        r["error"]["message"],
+        "event 1: pixel must lie in the view, 0 to 800 across and 0 to 600 down",
+        "{r}"
+    );
+    assert_eq!(e.phase(), tools::Phase::Idle);
+    assert_eq!(e.tool_kind(), tools::ToolKind::Line);
+    // A point behind the camera is not in view.
+    let behind = e.doc().camera.eye * 2.0 - e.doc().camera.target;
+    let r = call(
+        &mut e,
+        "tool_input",
+        json!({"events": [{"type": "move", "point": behind.to_array()}]}),
+    );
+    assert_eq!(
+        r["error"]["message"], "event 0: the point is not in view",
+        "{r}"
+    );
+}
+
+fn input_in(e: &mut Editor, events: Value, width: u32, height: u32) -> Value {
+    ok(
+        e,
+        "tool_input",
+        json!({"events": events, "width": width, "height": height}),
+    )
+}
+
+#[test]
+fn a_middle_drag_orbits_and_draws_nothing() {
+    let mut e = editor();
+    square(&mut e);
+    let eye = e.doc().camera.eye;
+    let r = input(
+        &mut e,
+        json!([{"type": "drag", "pixels": [[400, 300], [500, 300]], "button": "middle"}]),
+    );
+    assert_eq!(ran(&r), Vec::<String>::new());
+    assert_eq!(r["editor"]["phase"], "idle");
+    assert!(e.doc().camera.eye.distance(eye) > 0.1, "the view turned");
+}
```

- [ ] **Step 2: Run them to see them fail**

Run: `$HOME/.cargo/bin/cargo test -p app --bin app tool_input_tests`
Expected: 0 tests run: the test module is declared in `host.rs` with the implementation (Step 3).

- [ ] **Step 3: Implement**

```diff
diff --git a/crates/app/src/editor.rs b/crates/app/src/editor.rs
index d5146e5..39a6f8c 100644
--- a/crates/app/src/editor.rs
+++ b/crates/app/src/editor.rs
@@ -24,6 +24,7 @@ use crate::fly::{self, Fly, FlyKey};
 use crate::navigation::{Button, LOGICAL_PX_PER_ZOOM_STEP, Navigation, Scroll};
 
 mod agent;
+mod input;
 mod soften;
 mod texture_position;
 
@@ -132,6 +133,8 @@ pub struct Editor {
     flying: bool,
     /// The Soften Edges window.
     soften: SoftenWindow,
+    /// While an agent's `tool_input` runs: every command run, with its result.
+    issued: Option<Vec<(Command, Result<Outcome, CommandError>)>>,
 }
 
 /// The formats File > Export offers, as (filter name, extension).
@@ -190,6 +193,7 @@ impl Editor {
             fly: Fly::default(),
             flying: false,
             soften: SoftenWindow::default(),
+            issued: None,
         }
     }
 
@@ -1284,7 +1288,7 @@ impl Editor {
                 });
             }
             let Some(outcome) = outcome.filter(|_| !unchanged) else {
-                if undone && let Ok(redone) = self.doc.execute(Command::Redo { count: None }) {
+                if undone && let Ok(redone) = self.execute_recorded(Command::Redo { count: None }) {
                     if is_copy && !redone.changes.created.is_empty() {
                         self.select_created(redone);
                     } else {
@@ -1335,9 +1339,19 @@ impl Editor {
         self.preview.clear();
     }
 
+    /// Runs a command on the document, noting it and its result while `tool_input` runs.
+    fn execute_recorded(&mut self, command: Command) -> Result<Outcome, CommandError> {
+        let noted = self.issued.is_some().then(|| command.clone());
+        let result = self.doc.execute(command);
+        if let (Some(command), Some(issued)) = (noted, self.issued.as_mut()) {
+            issued.push((command, result.clone()));
+        }
+        result
+    }
+
     /// Runs a command; a failure becomes the status message.
     fn run(&mut self, command: Command) -> Option<Outcome> {
-        match self.doc.execute(command) {
+        match self.execute_recorded(command) {
             Ok(outcome) => Some(outcome),
             Err(e) if matches!(e.kind, ErrorKind::NothingToUndo | ErrorKind::NothingToRedo) => None,
             Err(e) => {
diff --git a/crates/app/src/editor/input.rs b/crates/app/src/editor/input.rs
new file mode 100644
index 0000000..f58af90
--- /dev/null
+++ b/crates/app/src/editor/input.rs
@@ -0,0 +1,206 @@
+//! `tool_input`: an agent's pointer and keyboard events run through the same Editor methods
+//! the window calls for a user's (see `api::Input`).
+
+use api::{Input, Key, Mods, MouseButton, Spot, ToolInput};
+use glam::{DVec2, DVec3};
+use session::{Command, CommandError, ErrorKind, Outcome};
+use tools::{Axis, Inference};
+
+use super::{Editor, EditorKey};
+use crate::navigation::Button;
+
+/// How far the view `tool_input` pixels are in may differ in proportions from the 3D view.
+const ASPECT_TOLERANCE: f64 = 0.01;
+
+/// What a run of events left: the commands they ran, the inference under the cursor and the
+/// tool's preview (both in the open context's coordinates).
+pub struct Driven {
+    pub issued: Vec<(Command, Result<Outcome, CommandError>)>,
+    pub hover: Option<Inference>,
+    pub preview: Vec<[DVec3; 2]>,
+}
+
+fn invalid(message: impl Into<String>) -> CommandError {
+    CommandError::new(ErrorKind::InvalidInput, message)
+}
+
+impl Editor {
+    /// Runs `input`'s events in order as the window runs a user's: each pointer event moves
+    /// the cursor first, and the hover is worked out after each, as a frame drawn between them
+    /// would. Modifiers held are let go at the end. An event that cannot run (a pixel outside
+    /// the view, a point out of sight) stops the rest with `invalid_input`; those before it
+    /// stay done.
+    pub fn drive(&mut self, input: ToolInput) -> Result<Driven, CommandError> {
+        let viewport = self.doc.camera.viewport;
+        let view = input.view.unwrap_or(viewport);
+        let (asked, ours) = (view.x / view.y, viewport.x / viewport.y);
+        if ((asked - ours) / ours).abs() > ASPECT_TOLERANCE {
+            return Err(invalid(format!(
+                "the view must have the proportions of the 3D view, {} x {} pixels",
+                viewport.x, viewport.y
+            )));
+        }
+        self.issued = Some(Vec::new());
+        let mut held = Mods::default();
+        let mut result = Ok(());
+        for (i, event) in input.events.into_iter().enumerate() {
+            if let Err(message) = self.input(event, view, &mut held) {
+                result = Err(invalid(format!("event {i}: {message}")));
+                break;
+            }
+            self.flush_hover();
+        }
+        self.hold(Mods::default());
+        self.flush_hover();
+        let issued = self.issued.take().unwrap_or_default();
+        let driven = Driven {
+            issued,
+            hover: if self.flying { None } else { self.hit },
+            preview: if self.flying {
+                Vec::new()
+            } else {
+                self.preview.clone()
+            },
+        };
+        // The window draws the marker where the events left the cursor at its next frame.
+        self.invalidate_hover();
+        result.map(|()| driven)
+    }
+
+    /// Runs one event; `held` are the modifiers a `modifiers` event holds.
+    fn input(&mut self, event: Input, view: DVec2, held: &mut Mods) -> Result<(), String> {
+        match event {
+            Input::Move(at) => {
+                self.move_to(at, view)?;
+            }
+            Input::Down(at, button) => {
+                self.move_to(at, view)?;
+                self.button(button_of(button), true);
+            }
+            Input::Up(at, button) => {
+                self.move_to(at, view)?;
+                self.button(button_of(button), false);
+            }
+            Input::Click(at, button) => {
+                self.move_to(at, view)?;
+                self.button(button_of(button), true);
+                self.button(button_of(button), false);
+            }
+            Input::DoubleClick(at) => {
+                self.move_to(at, view)?;
+                for _ in 0..2 {
+                    self.button(Button::Left, true);
+                    self.button(Button::Left, false);
+                }
+                self.double_click();
+            }
+            Input::Drag(path, button) => {
+                let button = button_of(button);
+                self.move_to(path[0], view)?;
+                self.button(button, true);
+                for at in &path[1..] {
+                    self.flush_hover();
+                    self.move_to(*at, view)?;
+                }
+                self.button(button, false);
+            }
+            Input::Key(key, mods) => self.press(key, mods, *held),
+            Input::Modifiers(mods) => {
+                *held = mods;
+                self.hold(mods);
+            }
+            Input::Type(text) => {
+                if !self.flying {
+                    self.flush_hover();
+                    self.typed = text;
+                    self.key(EditorKey::Enter);
+                }
+            }
+        }
+        Ok(())
+    }
+
+    /// Moves the cursor to `at`: a pixel of `view` scaled to the 3D view, or where the view
+    /// shows a model point.
+    fn move_to(&mut self, at: Spot, view: DVec2) -> Result<(), String> {
+        let viewport = self.doc.camera.viewport;
+        let cursor = match at {
+            Spot::Pixel(p) => {
+                if p.x > view.x || p.y > view.y {
+                    return Err(format!(
+                        "pixel must lie in the view, 0 to {} across and 0 to {} down",
+                        view.x, view.y
+                    ));
+                }
+                p * viewport / view
+            }
+            Spot::Point(p) => self
+                .local_camera()
+                .project(p)
+                .filter(|c| c.x >= 0.0 && c.y >= 0.0 && c.x <= viewport.x && c.y <= viewport.y)
+                .ok_or("the point is not in view")?,
+        };
+        self.cursor_moved(cursor);
+        Ok(())
+    }
+
+    /// Holds exactly `mods`, as the window's modifier changes do.
+    fn hold(&mut self, mods: Mods) {
+        self.set_modifiers(mods.shift, mods.alt);
+        self.set_sample_modifier(mods.cmd);
+    }
+
+    /// Presses `key` with `mods` held on top of `held`, as the window's key handling does.
+    fn press(&mut self, key: Key, mods: Mods, held: Mods) {
+        let with = Mods {
+            shift: held.shift || mods.shift,
+            alt: held.alt || mods.alt,
+            cmd: held.cmd || mods.cmd,
+        };
+        self.hold(with);
+        match key {
+            Key::CtrlTap => {
+                self.toggle_copy();
+            }
+            Key::Char('z' | 'Z') if with.cmd => {
+                if with.shift {
+                    self.redo();
+                } else {
+                    self.undo();
+                }
+            }
+            // Alt is the orbit modifier: Alt with a key does nothing in the window.
+            _ if with.alt || with.cmd => {}
+            key => {
+                self.key(editor_key(key, with.shift));
+            }
+        }
+        self.hold(held);
+    }
+}
+
+fn button_of(button: MouseButton) -> Button {
+    match button {
+        MouseButton::Left => Button::Left,
+        MouseButton::Middle => Button::Middle,
+        MouseButton::Right => Button::Other,
+    }
+}
+
+/// The editor key for `key`; a letter is uppercase exactly when Shift is held.
+fn editor_key(key: Key, shift: bool) -> EditorKey {
+    match key {
+        Key::Char(c) if c.is_ascii_alphabetic() && shift => EditorKey::Char(c.to_ascii_uppercase()),
+        Key::Char(c) if c.is_ascii_alphabetic() => EditorKey::Char(c.to_ascii_lowercase()),
+        Key::Char(c) => EditorKey::Char(c),
+        Key::Enter => EditorKey::Enter,
+        Key::Backspace => EditorKey::Backspace,
+        Key::Delete => EditorKey::Delete,
+        Key::Escape => EditorKey::Escape,
+        Key::Right => EditorKey::LockAxis(Axis::X),
+        Key::Left => EditorKey::LockAxis(Axis::Y),
+        Key::Up => EditorKey::LockAxis(Axis::Z),
+        Key::Down => EditorKey::Unlock,
+        Key::CtrlTap => unreachable!("handled by press"),
+    }
+}
diff --git a/crates/app/src/host.rs b/crates/app/src/host.rs
index 5dcb56b..243cb15 100644
--- a/crates/app/src/host.rs
+++ b/crates/app/src/host.rs
@@ -4,7 +4,10 @@
 
 use std::time::Duration;
 
-use api::{Dialog, EditorCommand, EditorState, Host, PhaseName, ToolName};
+use api::{
+    Dialog, EditorCommand, EditorState, Host, Hover, Issued, PhaseName, ToolInput, ToolInputResult,
+    ToolName, snap_name,
+};
 use render::Snapshotter;
 use session::{BatchError, Camera, Command, CommandError, Document, Lighting, Outcome};
 use tools::Phase;
@@ -113,6 +116,43 @@ impl Host for AppHost<'_> {
         Ok(())
     }
 
+    fn tool_input(&mut self, input: ToolInput) -> Result<ToolInputResult, CommandError> {
+        let driven = self.editor.drive(input)?;
+        Ok(ToolInputResult {
+            editor: editor_state(self.editor),
+            inference: driven.hover.map(|h| {
+                let (kind, axis) = snap_name(h.kind);
+                Hover {
+                    kind,
+                    axis,
+                    point: h.point.to_array(),
+                }
+            }),
+            preview: driven
+                .preview
+                .iter()
+                .map(|[a, b]| [a.to_array(), b.to_array()])
+                .collect(),
+            commands: driven
+                .issued
+                .into_iter()
+                .map(|(command, result)| match result {
+                    Ok(outcome) => Issued {
+                        command,
+                        outcome: Some(outcome),
+                        error: None,
+                    },
+                    Err(error) => Issued {
+                        command,
+                        outcome: None,
+                        error: Some(error),
+                    },
+                })
+                .collect(),
+            revision: self.editor.doc().revision(),
+        })
+    }
+
     fn quit(&mut self, discard: bool) -> Result<(), CommandError> {
         self.editor.doc().check_unsaved(discard)?;
         *self.quit = true;
@@ -162,6 +202,9 @@ pub fn editor_state(editor: &Editor) -> EditorState {
     }
 }
 
+#[cfg(test)]
+mod tool_input_tests;
+
 #[cfg(test)]
 mod tests {
     use api::{ToolName, handle};
diff --git a/docs/agent-api.md b/docs/agent-api.md
index 28ec567..ed12071 100644
--- a/docs/agent-api.md
+++ b/docs/agent-api.md
@@ -36,6 +36,7 @@ all commands and results.
 | `query` | a Query, see the table below | the QueryResult |
 | `render_view` | `{"width": 640, "height": 400, "camera": {"eye": [...], "target": [...]}}` (camera optional; it also takes `fov_y_deg`, `up` and `projection` as `set_camera` does; `lighting`, `day` or `night`, is optional and defaults to the document's) | `{width, height, png_base64}` |
 | `subscribe` | none | `{revision}`, then `changed` notifications after every model change; a `resync` notification (`{revision}`) means changes were missed, so re-read with `{"type": "entities"}` (its `revision` tells you where you are). Also `selection`, `camera` and `editor` notifications; see Notifications below |
+| `tool_input` | `{"events": [{"type": "click", "point": [0,0,0]}, {"type": "type", "text": "3"}], "width": 640, "height": 400}` (size optional) | `{editor, inference, preview, commands, revision}`: the active tool fed the events as the window feeds a user's; see Driving the tools below |
 | `describe` | none | the API description |
 | `quit` | `{"discard": true}` (optional; default false) | `{}`, then the app ends once its replies are sent (window or headless; a headless socket's file is removed). With unsaved changes and no `discard` it fails with `unsaved_changes` and goes on. In the window it ends without asking about unsaved changes. Once it is accepted, every later request, including those already waiting with it, is refused (`-32000`, `invalid_input`, "the program is quitting"), so no edit is made that would not be saved. Camera changes do not count as unsaved (see Camera), so `quit` without `discard` does not keep a view moved since the last save; `save` first to keep it. Params go by name: `[true]` is `-32602` |
 
@@ -638,6 +639,67 @@ revision; each returns `{"editor": {...}, "revision": n}`, the state after it:
 | `{"type": "set_dialog", "dialog": "soften_edges"}` | Opens the Soften Edges window over the view (`null` or absent closes it), as Edit > Soften Edges... and its close button do. |
 | `{"type": "set_soften_angle", "angle": 30}` | The Soften Edges window's angle, in whole degrees (0 to 180; above 180 is `invalid_input`, a negative or fractional angle `-32602`), which its Soften button uses. |
 
+### Driving the tools
+
+`tool_input` feeds the active tool the events a user makes in the 3D view, through the same
+editor code the window runs: the same inference and snapping, Shift and arrow-key locks, typed
+values ("3", "2,3", "24s", "5x", "/5", angles), Ctrl copying, Cmd sampling and double-click
+opening. Every tool behaviour is reachable this way, in the window and headless alike.
+
+```
+{"jsonrpc": "2.0", "id": 9, "method": "tool_input", "params": {"events": [
+  {"type": "click", "point": [0, 0, 0]},
+  {"type": "move", "point": [1, 0, 0]},
+  {"type": "type", "text": "3"}]}}
+```
+
+Events, run in order:
+
+| Event | What a user does |
+|---|---|
+| `{"type": "move", "pixel": [320, 200]}` | Moves the cursor there: the inference and the tool's preview follow. |
+| `{"type": "down", "point": [1, 2, 0], "button": "left"}`, `{"type": "up", ...}` | Presses or releases a button there (`left` default, `middle`, `right`). A middle drag orbits and Shift+middle pans, as in the window; Alt+left orbits. A button left down stays down for a later call. |
+| `{"type": "click", "point": [1, 2, 0]}` | Moves there, presses and releases. |
+| `{"type": "double_click", "point": [1, 2, 0]}` | Two left clicks there, then the double-click: the Select tool opens the group under it, or closes the open one when there is nothing of it there. |
+| `{"type": "drag", "pixels": [[100, 100], [200, 150], [300, 300]]}` (or `points`) | Presses at the first place, moves through the others, releases at the last (at least two): a Select box (dragged right: inside; left: crossing), or a navigation drag with `"button": "middle"`. |
+| `{"type": "key", "key": "r", "shift": false, "alt": false, "ctrl": false, "cmd": false}` | A key pressed with those modifiers held: one character (tool shortcuts, "g" group, "G" or Shift+g component, "Z" zoom extents, "f" fly, or a measurement's first character), or `enter`, `backspace`, `delete`, `escape`, `space`, `right` / `left` / `up` (lock the red / green / blue axis during an operation, or unlock it), `down` (unlock), `ctrl` (Ctrl tapped alone: Move and Rotate switch copying, the Tape Measure guide making). An uppercase letter holds Shift. With `cmd` or `ctrl` only `z` is taken (undo; with `shift` redo); the window's file shortcuts are commands (`new`, `open`, `save`, `import`, `export`, `quit`) and other keys are `invalid_input`. With `alt` a key does nothing, as in the window. |
+| `{"type": "modifiers", "shift": true}` | Holds exactly these modifiers (`shift`, `alt`, `ctrl`, `cmd`; absent ones are let go): Shift over an axis, parallel or perpendicular snap locks the direction, Shift toggles in the Select tool, Cmd or Ctrl makes a Paint click sample the material. |
+| `{"type": "type", "text": "2,3"}` | Types the text into the measurements box, replacing anything typed, and presses Enter. Unlike a key, a first letter is text here, not a shortcut. |
+
+- **Where**: a `pixel` is in a view of `width` x `height` (both or neither; default the 3D view,
+  `editor.viewport`, 1280 x 800 headless), scaled to the 3D view, which must have the same
+  proportions (within 1%). A `point` is in the open context's coordinates and is looked for
+  where the document camera shows it at the event's turn (the camera may have moved). Either
+  way the tool infers from that pixel, as for a click there: a point near an endpoint snaps to
+  it, as `infer` reports. Exact values come from typing them.
+- **Modifiers** held by `modifiers` are let go at the end of the call, as if the keys were
+  released (a Shift lock ends with it; arrow-key locks last until the operation ends). In the
+  window the user's own key presses also change them.
+- **Frames**: after each event the hover is worked out as a frame drawn between events would.
+- **Errors**: malformed events are `-32602`; an event that makes no sense (both or neither of
+  `pixel` and `point`, a drag of one place, an unknown key, empty text, a size out of range) is
+  `invalid_input` naming it (`event 2: ...`) and nothing runs. An event that cannot run at its
+  turn (a pixel outside the view, a point not in sight) stops the rest with `invalid_input`;
+  the events before it stay done. While flying, clicks and typing do nothing, as in the window.
+
+The result is the editor after the events:
+
+```
+{"editor": {...EditorState...},
+ "inference": {"kind": "axis", "axis": "x", "point": [3, 0, 0]},
+ "preview": [[[0, 0, 0], [3, 0, 0]]],
+ "commands": [{"command": {"type": "add_edge", "start": [0,0,0], "end": [3,0,0]},
+               "outcome": {"changes": {...}}}],
+ "revision": 1}
+```
+
+`inference` is what the cursor snaps to now (kinds as `infer` names them; `null` while flying),
+`preview` the tool's rubber-band segments (open context coordinates), and `commands` every
+command the events ran, in order, each with its `outcome` or `error` (a CommandError): the
+tool's, a copy's selection of what it made, a shortcut's (`make_group`), an undo. The window
+also shows a failed command's message in `editor.message`. Subscribers get the usual `changed`,
+`selection`, `camera` and `editor` notifications.
+
 ### Files
 
 Models are saved as `.skr` files: a zip archive holding `document.json`, which records `format`
@@ -829,12 +891,12 @@ under a pixel and where a click would snap.
 | Undo, Redo (Cmd+Z, Shift+Cmd+Z) | `undo`, `redo`; `history` lists the steps |
 | Make group (G), Make component (Shift+G), Explode, Close group (Escape) | `make_group`, `make_component` (`name`), `explode`, `close_instance` |
 | Camera menu: Zoom Extents (Shift+Z), standard views, Perspective / Parallel, Night lighting | `zoom_extents`, `standard_view`, `set_projection`, `set_lighting` |
-| Orbit, pan and zoom with the mouse or trackpad, fly mode (F, Escape) and its speed | `set_camera`, `orbit`, `pan`, `zoom` reach the same views; `set_fly_mode` (`on`, `speed`), `cancel`; `editor.flying`, `editor.fly_speed`. Flying itself (moving keys, mouse look) moves the camera, which `set_camera` places directly |
-| The toolbar's tools and their keys | `set_tool`; `editor.tool`, `editor.phase`, `editor.points` |
-| Select: click, Shift-click, box (inside or crossing), double-click to open a group | `select` (`mode`; `box` with `crossing`), `pick` for what is under a pixel, `open_instance` / `open_path` |
-| Line, Rectangle, Circle, Arc, Push/Pull, Move (Ctrl copies, `5x`, `/5`), Rotate, Tape Measure, Erase, Paint (Cmd samples) | `add_edge`, `add_rectangle`, `add_circle` (`segments`), `add_arc`, `push_pull`, `move`, `copy` (`count`), `rotate` (`copies`), `add_guide_line` / `add_guide_point`, `erase`, `paint` (`side`); a sampled material is the `entity` query's |
-| Inference snapping, Shift and arrow-key locks, the marker under the cursor | `infer` (with `from`; `axis` is what Shift would lock) |
-| Typing into the measurements box, Escape | the commands' explicit values; `cancel`; `editor.measurement`, `editor.typing` |
+| Orbit, pan and zoom with the mouse or trackpad (about the point under the cursor), fly mode (F, Escape) and its speed | `orbit` and `zoom` (`about` a point), `pan`, `set_camera`; a middle `drag` in `tool_input`; `set_fly_mode` (`on`, `speed`), `cancel`; `editor.flying`, `editor.fly_speed`. Flying itself (moving keys, mouse look) moves the camera, which `set_camera` places directly |
+| The toolbar's tools and their keys | `set_tool` or a `tool_input` key; `editor.tool`, `editor.phase`, `editor.points` |
+| Select: click, Shift-click, box (inside or crossing), double-click to open a group | `tool_input` (`click`, `modifiers`, `drag`, `double_click`); `select` (`mode`; `box` with `crossing`), `pick` for what is under a pixel, `open_instance` / `open_path` |
+| Line, Rectangle, Circle, Arc, Push/Pull, Move (Ctrl copies, `5x`, `/5`), Rotate, Tape Measure, Erase, Paint (Cmd samples) | `tool_input`, or the commands they make: `add_edge`, `add_rectangle`, `add_circle` (`segments`), `add_arc`, `push_pull`, `move`, `copy` (`count`), `rotate` (`copies`), `add_guide_line` / `add_guide_point`, `erase`, `paint` (`side`); a sampled material is the `entity` query's |
+| Inference snapping, Shift and arrow-key locks, the marker under the cursor | `tool_input` (`modifiers`, arrow `key`s; `inference` in its result); `infer` (with `from`; `axis` is what Shift would lock) |
+| Typing into the measurements box, Escape | `tool_input` (`type`, `key`); the commands' explicit values; `cancel`; `editor.measurement`, `editor.typing` |
 | The status bar | `editor.status` (and `prompt`, `message`, `lock`) |
 | Materials list: choose (for Paint), Add, Delete, name, color, opacity, Glow, Texture..., Remove, texture size | `set_paint_material`, `add_material`, `delete_material`, `change_material` (`name`, `color`, `opacity`, `emission`, `texture`); `materials` |
 | Entity Info: what is selected, texture Offset / Rotation / Scale, Rotate 90, Reset | `entity`, `entities`, `curve`; `position_texture` (`offset`, `rotation_deg`, `rotate_by: 90`, `scale`, `reset`) |
```

- [ ] **Step 4: Run the tests**

Run: `$HOME/.cargo/bin/cargo test -p app --bin app` and `$HOME/.cargo/bin/cargo clippy -p app --all-targets -- -D warnings`
Expected: PASS (4 new tests).
Then `$HOME/.cargo/bin/cargo fmt --all --check` and `$HOME/.cargo/bin/cargo clippy --workspace --all-targets -- -D warnings`: clean.

- [ ] **Step 5: Commit**

```bash
git status && git diff --stat
git add crates/app/src/editor.rs crates/app/src/editor/input.rs crates/app/src/host.rs crates/app/src/host/tool_input_tests.rs docs/agent-api.md
git commit -m "M10b: the editor runs tool_input events as the window does"
```

---

### Task 6: Every tool through `tool_input`

**Files:**
- Test: `crates/app/src/host/tool_input_tests.rs`
- Test: `crates/app/tests/headless_agent.rs`

**Interfaces:**
- Consumes: Task 5's test helpers and `AppHost::tool_input`.
- Produces: tests only (helpers `tool`, `select_all` added): Line (clicks, typed length, Escape), Rectangle ("2,3"), Circle ("8s", radius), Arc (two clicks, typed bulge), Push/Pull (typed distance), Move (Ctrl copy, "4x" array, move), Rotate (typed angle, Ctrl copy, "3x"), Tape Measure (parallel guide by typed distance, Ctrl measures only), Erase, Paint (and Cmd sampling), Select (click, Shift toggle, box drag, double-click opens a group), Shift and arrow-key locks, and an end-to-end headless run over stdio drawing a box.

Verify the result against the scratch tree: `git diff refs/scratch/m10b/t6 -- <paths>` is empty after the task (the tree predates the plan commit, so restrict it to the task's paths).

- [ ] **Step 1: Write the tests**

They exercise code that already exists, so they pass once written; a failure is a bug to fix in the tool path, not in the test.

```diff
diff --git a/crates/app/src/host/tool_input_tests.rs b/crates/app/src/host/tool_input_tests.rs
index 914f937..e6c603e 100644
--- a/crates/app/src/host/tool_input_tests.rs
+++ b/crates/app/src/host/tool_input_tests.rs
@@ -43,6 +43,10 @@ fn ask(e: &mut Editor, query: Value) -> Value {
     ok(e, "query", query)
 }
 
+fn tool(e: &mut Editor, name: &str) {
+    exec(e, json!({"type": "set_tool", "tool": name}));
+}
+
 /// The `type`s of the commands a result says were run.
 fn ran(r: &Value) -> Vec<String> {
     r["commands"]
@@ -85,6 +89,16 @@ fn square(e: &mut Editor) -> Value {
     created(&r, 'f')[0].clone()
 }
 
+fn select_all(e: &mut Editor) {
+    let all = ask(e, json!({"type": "entities"}));
+    let ids: Vec<Value> = ["edges", "faces", "instances"]
+        .iter()
+        .flat_map(|k| all[k].as_array().cloned().unwrap_or_default())
+        .map(|x| x["id"].clone())
+        .collect();
+    exec(e, json!({"type": "select", "ids": ids}));
+}
+
 fn count(e: &mut Editor, kind: &str) -> u64 {
     ask(e, json!({"type": "stats"}))[kind].as_u64().unwrap()
 }
@@ -118,6 +132,352 @@ fn the_line_tool_draws_by_clicks_and_a_typed_length() {
     assert_eq!(r["editor"]["phase"], "idle");
 }
 
+#[test]
+fn rectangle_and_circle_take_typed_sizes_and_sides() {
+    let mut e = editor();
+    tool(&mut e, "rectangle");
+    let r = input(
+        &mut e,
+        json!([
+            {"type": "click", "point": [0, 0, 0]},
+            {"type": "move", "point": [1, 1, 0]},
+            {"type": "type", "text": "2,3"}
+        ]),
+    );
+    assert_eq!(ran(&r), ["add_rectangle"]);
+    assert!(
+        near(&r["commands"][0]["command"]["opposite"], [2.0, 3.0, 0.0]),
+        "{r}"
+    );
+
+    tool(&mut e, "circle");
+    let r = input(
+        &mut e,
+        json!([
+            {"type": "type", "text": "8s"},
+            {"type": "click", "point": [6, 0, 0]},
+            {"type": "move", "point": [7, 0, 0]},
+            {"type": "type", "text": "1.5"}
+        ]),
+    );
+    assert_eq!(ran(&r), ["add_circle"]);
+    let c = &r["commands"][0]["command"];
+    assert_eq!(
+        (c["radius"].clone(), c["segments"].clone()),
+        (json!(1.5), json!(8))
+    );
+    assert!(near(&c["center"], [6.0, 0.0, 0.0]));
+}
+
+#[test]
+fn the_arc_tool_takes_two_clicks_and_a_typed_bulge() {
+    let mut e = editor();
+    tool(&mut e, "arc");
+    let r = input(
+        &mut e,
+        json!([
+            {"type": "click", "point": [0, 0, 0]},
+            {"type": "click", "point": [2, 0, 0]},
+            {"type": "move", "point": [1, 0.5, 0]}
+        ]),
+    );
+    assert_eq!(r["editor"]["points"].as_array().unwrap().len(), 2, "{r}");
+    assert_eq!(r["editor"]["measurement_label"], "Bulge");
+    let r = input(&mut e, json!([{"type": "type", "text": "1"}]));
+    assert_eq!(ran(&r), ["add_arc"], "{r}");
+    let edges = created(&r["commands"][0]["outcome"], 'e');
+    assert_eq!(edges.len(), 12);
+    let curve = ask(&mut e, json!({"type": "curve", "id": edges[0]}));
+    assert!(
+        near(&curve["center"], [1.0, 0.0, 0.0]),
+        "a half circle: {curve}"
+    );
+    assert!((curve["angle"].as_f64().unwrap() - 180.0).abs() < 1e-9);
+}
+
+#[test]
+fn push_pull_extrudes_a_clicked_face_by_a_typed_distance() {
+    let mut e = editor();
+    square(&mut e);
+    tool(&mut e, "push_pull");
+    let r = input(
+        &mut e,
+        json!([
+            {"type": "click", "point": [1, 1, 0]},
+            {"type": "move", "point": [1, 1, 0.5]},
+            {"type": "type", "text": "1.5"}
+        ]),
+    );
+    assert_eq!(ran(&r), ["push_pull"], "{r}");
+    assert_eq!(
+        r["commands"][0]["command"]["distance"]
+            .as_f64()
+            .unwrap()
+            .abs(),
+        1.5
+    );
+    assert_eq!(count(&mut e, "faces"), 6);
+    let top = ask(
+        &mut e,
+        json!({"type": "pick", "ray": {"origin": [1, 1, 10], "direction": [0, 0, -1]}}),
+    );
+    assert_eq!(top["hits"][0]["point"], json!([1.0, 1.0, 1.5]), "{top}");
+}
+
+#[test]
+fn move_copies_after_a_ctrl_tap_and_arrays_replace_the_copy() {
+    let mut e = editor();
+    square(&mut e);
+    select_all(&mut e);
+    tool(&mut e, "move");
+    let r = input(
+        &mut e,
+        json!([
+            {"type": "click", "point": [0, 0, 0]},
+            {"type": "key", "key": "ctrl"},
+            {"type": "move", "point": [1, 0, 0]},
+            {"type": "type", "text": "3"}
+        ]),
+    );
+    assert_eq!(ran(&r), ["copy", "select"], "{r}");
+    assert_eq!(
+        r["commands"][0]["command"]["offset"],
+        json!([3.0, 0.0, 0.0])
+    );
+    assert_eq!(count(&mut e, "faces"), 2);
+    // "4x" makes four copies at that spacing in place of the one.
+    let r = input(&mut e, json!([{"type": "type", "text": "4x"}]));
+    assert_eq!(ran(&r), ["undo", "copy", "select"], "{r}");
+    assert_eq!(r["commands"][1]["command"]["count"], 4);
+    assert_eq!(count(&mut e, "faces"), 5);
+
+    // Ctrl again moves instead: from a corner to a point 5 m along Y.
+    select_all(&mut e);
+    let r = input(
+        &mut e,
+        json!([
+            {"type": "key", "key": "ctrl"},
+            {"type": "click", "point": [0, 0, 0]},
+            {"type": "click", "point": [0, 5, 0]}
+        ]),
+    );
+    assert_eq!(ran(&r), ["move"], "{r}");
+    assert!(
+        near(&r["commands"][0]["command"]["offset"], [0.0, 5.0, 0.0]),
+        "{r}"
+    );
+}
+
+#[test]
+fn rotate_turns_the_selection_by_a_typed_angle_and_copies() {
+    let mut e = editor();
+    square(&mut e);
+    select_all(&mut e);
+    tool(&mut e, "rotate");
+    let r = input(
+        &mut e,
+        json!([
+            {"type": "click", "point": [5, 5, 0]},
+            {"type": "click", "point": [6, 5, 0]},
+            {"type": "type", "text": "90"}
+        ]),
+    );
+    assert_eq!(ran(&r), ["rotate"], "{r}");
+    let c = &r["commands"][0]["command"];
+    assert!(near(&c["center"], [5.0, 5.0, 0.0]), "{c}");
+    assert_eq!(c["angle"].as_f64().unwrap().abs(), 90.0);
+    // Ctrl copies; "3x" then makes a radial array of three.
+    select_all(&mut e);
+    let r = input(
+        &mut e,
+        json!([
+            {"type": "key", "key": "ctrl"},
+            {"type": "click", "point": [5, 5, 0]},
+            {"type": "click", "point": [6, 5, 0]},
+            {"type": "type", "text": "45"},
+            {"type": "type", "text": "3x"}
+        ]),
+    );
+    assert_eq!(
+        ran(&r),
+        ["rotate", "select", "undo", "rotate", "select"],
+        "{r}"
+    );
+    assert_eq!(r["commands"][3]["command"]["copies"], 3);
+    assert_eq!(count(&mut e, "faces"), 4);
+}
+
+#[test]
+fn the_tape_measure_adds_guides_and_ctrl_only_measures() {
+    let mut e = editor();
+    exec(
+        &mut e,
+        json!({"type": "add_edge", "start": [0, 0, 0], "end": [4, 0, 0]}),
+    );
+    tool(&mut e, "tape_measure");
+    // From the edge: a parallel guide line at the typed distance.
+    let r = input(
+        &mut e,
+        json!([
+            {"type": "click", "point": [1, 0, 0]},
+            {"type": "move", "point": [1, 1, 0]},
+            {"type": "type", "text": "1.25"}
+        ]),
+    );
+    assert_eq!(ran(&r), ["add_guide_line"], "{r}");
+    assert!(
+        near(&r["commands"][0]["command"]["point"], [1.0, 1.25, 0.0]),
+        "{r}"
+    );
+    // Ctrl: measure only.
+    let r = input(
+        &mut e,
+        json!([
+            {"type": "key", "key": "ctrl"},
+            {"type": "click", "point": [0, 0, 0]},
+            {"type": "click", "point": [4, 0, 0]}
+        ]),
+    );
+    assert_eq!(ran(&r), Vec::<String>::new(), "{r}");
+    assert_eq!(r["editor"]["message"], "Distance: 4 m", "{r}");
+}
+
+#[test]
+fn erase_and_paint_act_on_what_is_clicked_and_cmd_samples() {
+    let mut e = editor();
+    let face = square(&mut e);
+    let m = exec(
+        &mut e,
+        json!({"type": "add_material", "name": "Brick", "color": [200, 80, 60]}),
+    )["changes"]["materials"][0]
+        .clone();
+    exec(&mut e, json!({"type": "set_paint_material", "material": m}));
+    tool(&mut e, "paint");
+    let r = input(&mut e, json!([{"type": "click", "point": [1, 1, 0]}]));
+    assert_eq!(ran(&r), ["paint"], "{r}");
+    assert_eq!(r["commands"][0]["command"]["ids"], json!([face]));
+    // With Cmd held a click picks up the material clicked.
+    exec(
+        &mut e,
+        json!({"type": "set_paint_material", "material": null}),
+    );
+    let r = input(
+        &mut e,
+        json!([
+            {"type": "modifiers", "cmd": true},
+            {"type": "click", "point": [1, 1, 0]}
+        ]),
+    );
+    assert_eq!(ran(&r), Vec::<String>::new());
+    assert_eq!(r["editor"]["paint_material"], m, "{r}");
+    assert_eq!(r["editor"]["message"], "Painting with Brick");
+
+    tool(&mut e, "erase");
+    let r = input(&mut e, json!([{"type": "click", "point": [1, 0, 0]}]));
+    assert_eq!(ran(&r), ["erase"], "{r}");
+    assert_eq!(count(&mut e, "faces"), 0, "the edge took its face");
+}
+
+#[test]
+fn select_clicks_toggles_with_shift_boxes_and_double_click_opens() {
+    let mut e = editor();
+    let face = square(&mut e);
+    exec(
+        &mut e,
+        json!({"type": "add_edge", "start": [4, 0, 0], "end": [4, 2, 0]}),
+    );
+    tool(&mut e, "select");
+    let r = input(&mut e, json!([{"type": "click", "point": [1, 1, 0]}]));
+    assert_eq!(ran(&r), ["select"]);
+    assert_eq!(
+        ask(&mut e, json!({"type": "selection"}))["ids"],
+        json!([face])
+    );
+    // Shift-click toggles the edge in.
+    let r = input(
+        &mut e,
+        json!([
+            {"type": "modifiers", "shift": true},
+            {"type": "click", "point": [4, 1, 0]}
+        ]),
+    );
+    assert_eq!(r["commands"][0]["command"]["mode"], "toggle", "{r}");
+    assert_eq!(
+        ask(&mut e, json!({"type": "selection"}))["ids"]
+            .as_array()
+            .unwrap()
+            .len(),
+        2
+    );
+    // A box dragged right around everything selects it all.
+    let r = input(
+        &mut e,
+        json!([{"type": "drag", "pixels": [[1, 1], [400, 300], [799, 599]]}]),
+    );
+    assert_eq!(ran(&r), ["select"], "{r}");
+    assert_eq!(
+        ask(&mut e, json!({"type": "selection"}))["ids"]
+            .as_array()
+            .unwrap()
+            .len(),
+        6
+    );
+
+    // Group the square; a double-click opens the group.
+    exec(&mut e, json!({"type": "select", "ids": [face]}));
+    exec(&mut e, json!({"type": "make_group", "ids": [face]}));
+    let r = input(
+        &mut e,
+        json!([{"type": "double_click", "point": [1, 1, 0]}]),
+    );
+    assert_eq!(ran(&r), ["select", "select", "open_instance"], "{r}");
+    assert_eq!(
+        ask(&mut e, json!({"type": "context"}))["path"]
+            .as_array()
+            .unwrap()
+            .len(),
+        1
+    );
+}
+
+#[test]
+fn shift_and_arrow_keys_lock_the_direction() {
+    let mut e = editor();
+    // Shift pressed over the red axis holds the cursor to it while it clicks off the axis.
+    let r = input(
+        &mut e,
+        json!([
+            {"type": "click", "point": [0, 0, 0]},
+            {"type": "move", "point": [2, 0, 0]},
+            {"type": "modifiers", "shift": true},
+            {"type": "click", "point": [2, 1, 0]}
+        ]),
+    );
+    assert_eq!(ran(&r), ["add_edge"], "{r}");
+    let end = vec3(&r["commands"][0]["command"]["end"]);
+    assert!(
+        end.x > 1.0 && end.y == 0.0 && end.z == 0.0,
+        "on the red axis: {r}"
+    );
+    // The lock ends with the call, when Shift is let go.
+    assert_eq!(r["editor"]["lock"], Value::Null);
+
+    // The up arrow locks the blue axis and stays until the operation ends.
+    let r = input(&mut e, json!([{"type": "key", "key": "up"}]));
+    assert_eq!(r["editor"]["lock"], "Locked: blue axis", "{r}");
+    let r = input(
+        &mut e,
+        json!([
+            {"type": "move", "point": [3, 1, 0]},
+            {"type": "type", "text": "1"}
+        ]),
+    );
+    assert!(
+        near(&r["commands"][0]["command"]["end"], [end.x, 0.0, 1.0]),
+        "{r}"
+    );
+}
+
 #[test]
 fn keys_switch_tools_group_and_undo() {
     let mut e = editor();
diff --git a/crates/app/tests/headless_agent.rs b/crates/app/tests/headless_agent.rs
index c555e39..579351d 100644
--- a/crates/app/tests/headless_agent.rs
+++ b/crates/app/tests/headless_agent.rs
@@ -119,3 +119,65 @@ fn quitting_over_a_socket_answers_and_removes_the_socket() {
     assert!(!path.exists(), "the socket file is removed");
     std::fs::remove_dir_all(&dir).unwrap();
 }
+
+#[test]
+fn an_agent_draws_a_box_with_the_tools_over_stdio() {
+    let mut child = Command::new(env!("CARGO_BIN_EXE_app"))
+        .args(["--headless", "--stdio"])
+        .stdin(Stdio::piped())
+        .stdout(Stdio::piped())
+        .stderr(Stdio::inherit())
+        .spawn()
+        .expect("start app");
+    let mut stdin = child.stdin.take().unwrap();
+    let requests = [
+        request(1, "subscribe", Value::Null),
+        request(
+            2,
+            "execute",
+            json!({"type": "set_tool", "tool": "rectangle"}),
+        ),
+        request(
+            3,
+            "tool_input",
+            json!({"events": [
+                {"type": "click", "point": [0, 0, 0]},
+                {"type": "move", "point": [1, 1, 0]},
+                {"type": "type", "text": "2,3"},
+                {"type": "key", "key": "p"},
+                {"type": "click", "point": [1, 1, 0]},
+                {"type": "move", "point": [1, 1, 0.5]},
+                {"type": "type", "text": "1"}
+            ]}),
+        ),
+        request(4, "query", json!({"type": "stats"})),
+        request(5, "quit", json!({"discard": true})),
+    ];
+    for r in &requests {
+        writeln!(stdin, "{r}").unwrap();
+    }
+    let status = exits(&mut child);
+    drop(stdin);
+    assert!(status.success(), "exit status {status}");
+    let messages: Vec<Value> = BufReader::new(child.stdout.take().unwrap())
+        .lines()
+        .map(|l| serde_json::from_str(&l.unwrap()).unwrap())
+        .collect();
+    let replies: HashMap<u64, &Value> = messages
+        .iter()
+        .filter_map(|m| m["id"].as_u64().map(|id| (id, m)))
+        .collect();
+    let drawn = &replies[&3]["result"];
+    let ran: Vec<&str> = drawn["commands"]
+        .as_array()
+        .unwrap()
+        .iter()
+        .map(|c| c["command"]["type"].as_str().unwrap())
+        .collect();
+    assert_eq!(ran, ["add_rectangle", "push_pull"], "{drawn}");
+    assert_eq!(drawn["editor"]["tool"], "push_pull");
+    assert_eq!(replies[&4]["result"]["faces"], 6);
+    // Subscribers hear of both edits.
+    let changed = messages.iter().filter(|m| m["method"] == "changed").count();
+    assert_eq!(changed, 2, "{messages:?}");
+}
```

- [ ] **Step 2: Run the tests**

Run: `$HOME/.cargo/bin/cargo test -p app --bin app tool_input_tests` and `$HOME/.cargo/bin/cargo test -p app --test headless_agent`
Expected: PASS (13 and 3 tests).
Then `$HOME/.cargo/bin/cargo fmt --all --check` and `$HOME/.cargo/bin/cargo clippy --workspace --all-targets -- -D warnings`: clean.

- [ ] **Step 3: Commit**

```bash
git status && git diff --stat
git add crates/app/src/host/tool_input_tests.rs crates/app/tests/headless_agent.rs
git commit -m "M10b: tool_input tests for every tool, locks and modifiers; headless box"
```

---

## After the last task

- [ ] `$HOME/.cargo/bin/cargo test --workspace`, `$HOME/.cargo/bin/cargo clippy --workspace --all-targets -- -D warnings`, `$HOME/.cargo/bin/cargo fmt --all --check`, `$HOME/.cargo/bin/cargo build --release -p app`.
- [ ] Final whole-branch review, one fix wave, then the roadmap row `done <first>..<last>` ("Roadmap: M10b done") and the M10b section in the project memory's follow-ups file.
- [ ] Delete the scratch refs: `git for-each-ref --format='%(refname)' refs/scratch/m10b | xargs -n1 git update-ref -d`.
