# M6c Tape Measure and Guides Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Guide lines and guide points can be added, snapped to, selected, moved, erased and saved, by agents (`add_guide_line`, `add_guide_point`, `delete_guides`) and in the window (the Tape Measure tool, `t`).

**Architecture:**
- **kernel:**
  - `Guide` entities live per mesh (`GuideId`, `"g<n>"`) and are recorded as `Op::AddGuide` and `Op::RemoveGuide`.
  - `guides.rs` adds, erases, moves (keeping the id) and copies them, and moves them between meshes.
  - Groups take guides in, copies of groups copy them, and explode lifts them out.
  - Files store them as `GuideData`.
- **session:**
  - New commands `add_guide_line`, `add_guide_point` and `delete_guides`.
  - Erase, select, move, rotate, copy and make_group take guide ids.
  - `entities` and `entity` report guides.
  - `.skr` schema 2; version 1 files still open.
  - `Camera::line_span` and `near`.
- **render:** guides are collected as `GuideShape`s and drawn each frame as dashed lines (clipped in f64) and point crosses, depth-tested.
- **tools:**
  - Inference snaps to guide points, onto guide lines, and to guide crossings.
  - `guide_under_where`.
  - Select picks and box-selects guides; Erase and Move outline them.
  - `TapeMeasureTool` (`t`).
- **app:** Ctrl reaches the Tape Measure; Edit > Delete Guides; KeyT fallback; Entity Info shows guides.

**Tech Stack:** Rust 2024; no dependency changes.

**Spec:** `docs/superpowers/specs/2026-10-10-tenon-design.md`:
- section 2 (v1 tools include Tape Measure, guides);
- section 5 (commands).

Task 3 adds the M6c milestone line.

## Global Constraints

- Every new capability is a `session::Command` reachable through the API, covered by a command-driven test. Tools only emit commands.
- Each modeling command is one undo step. Command inputs and the points they produce must be finite and within +/-`MAX_COORDINATE` (1e6 m).
- No dependency changes.
- Git:
  - work on `dev`;
  - stage only the files you changed;
  - short plain commit messages, no emoji;
  - run `cargo fmt --all` before each commit;
  - `cargo clippy --workspace --all-targets -- -D warnings` stays clean.
- Do not launch the app window.

## Spec notes (rulings made in this plan)

- **Guides live in the kernel `Model`, per mesh, not at session level.** `kernel::Guide`
  (`Line { point, direction }` with a unit direction, or `Point { position }`) is stored in a
  `BTreeMap<GuideId, Guide>` of each mesh, with `Op::AddGuide` / `Op::RemoveGuide`. Why:
  SketchUp guides belong to the context they are drawn in; undo, redo, events, atomic batches,
  the change summary and `.skr` files then work unchanged (the undo unit is the kernel
  `SceneChange`; a session-side store would need a second undo channel). The roadmap's
  "session-level guides" is read as "document data reachable through session commands".
- **Ids follow the existing scheme**: `GuideId` from the shared id counter, `EntityId::Guide`,
  serialized `"g<n>"` (schema pattern `^[vefig]...`). Unique across meshes like every id.
- **Commands**: `add_guide_line {point, direction}` (direction any non-zero length, stored
  normalized), `add_guide_point {position}`, `delete_guides` (every guide of the open context;
  none there = success, no undo step, no error). Each is one undo step. Points within
  +/-1e6 m; zero direction `invalid_input`. Why separate add commands: explicit shapes, simple
  schemas; one `delete_guides` mirrors SketchUp's Edit > Delete Guides (open context only, as
  every command addresses the open context).
- **Existing commands take guide ids** instead of new guide-specific ones: `erase`, `select`,
  `move`, `rotate` (incl. `copies`), `copy`, `make_group`/`make_component` (guides move into
  the definition keeping their ids), `explode` (guides come out placed, with new ids, like
  instances). A moved/turned guide keeps its id and is reported `modified` (remove + add under
  the same id, no new Op). Copying a group copies its guides. Guides alone cannot be grouped
  (`invalid_input`, all ids listed): a definition with only guides has no bounds.
- **Queries**: `entities` gains a `guides` list and the kind `guide`; `entity` returns
  `{"type":"guide","id","kind":"line"|"point","point","direction"?}`. `within` contains a
  guide point by its position; a guide line is in no box. `stats` does not count guides (keeps
  its shape). No separate `guides` query: `entities` with `kinds: ["guide"]` lists them.
- **Files**: `.skr` `schema_version` bumped to 2; `MeshData.guides` (`GuideData {id, point,
  direction?}`) is `#[serde(default, skip_serializing_if = "Vec::is_empty")]`, so version 1
  files (no key) still open. Loading checks finite numbers, unit directions, no shared ids,
  and the +/-1e6 m limit on guide points. The API `describe` schema_version stays 1 (additive).
- **Not exported**: export writes faces only, so guides never reach OBJ/STL/glTF (a model with
  only guides is "nothing to export").
- **Inference**: new `SnapKind::GuidePoint` (ranked with endpoint/midpoint/origin, nearest
  wins) and `SnapKind::OnGuide` (after on-edge, before directions). Intersections now include
  guide line x edge and guide line x guide line (apparent crossings, existing `Intersection`
  kind; a true crossing is the same point). Guide/face intersections are out of scope. Guides
  are scanned linearly (not in the BVH): they are few. Guides of every placed mesh are snapped
  to (inside instances and outside the open one), and faces hide them (visibility test).
  Markers: guide point uses the endpoint color, on-guide the on-edge color.
- **`Camera::line_span(point, dir)`** (session): the drawable part of an infinite line, in
  front of 2x the near plane and within `LINE_REACH` = 1000 eye-to-target distances (depth and
  along the line). Used by inference crossings, picking outlines, box selection and rendering.
  Also `Camera::near()` (proj() uses it).
- **Picking and selection**: Select pick order is open edge > open guide > instance edge >
  face. Guides are picked only in the open context. Box select: a window holds guide points
  inside it; guide lines are never inside a window, and a crossing box (dragged left) selects
  guide lines whose on-screen span touches it. Erase tool and Delete erase guides; Move/Rotate
  tools move them through the same commands (their outline is drawn); Entity Info shows
  "1 guide line / Direction: x, y, z", "1 guide point / Position: x, y, z", and "N guides" in
  mixed counts.
- **Rendering**: guides are not in the cached edge buffer. `scene_geometry` collects
  `GuideShape`s in root coordinates (with color: guide, faded outside the open instance, blue
  when selected); the renderer rebuilds their line vertices each frame for the camera
  (`guides::guide_lines`) and draws them with the edge pipeline (depth-tested, so faces hide
  them). Guide lines are clipped in f64 (`line_span`, then to the viewport) and dashed with
  8 px dashes and gaps on screen (dash ends unprojected onto the line in f64, so no f32 error
  from huge segments); guide points are 5 px X crosses. Why per frame: an infinite line has no
  fixed segment that is both precise in f32 and long enough.
- **Tape Measure tool** (`t`, free before; toolbar after Rotate, tooltip "Tape Measure (T)";
  KeyT added to the non-ASCII physical-key fallback):
  - Click 1 on an edge (`OnEdge`) or guide line (`OnGuide`) starts a line measurement
    (distance square to that line); anywhere else (endpoint, midpoint, guide point, face,
    ground) a point measurement. Click 2 adds a guide line parallel to the line through the
    end (point = the start foot + offset), or a guide point at the end. A zero distance is
    refused with a message.
  - Measurements box "Length" shows the live distance. Typed length: offset in the cursor's
    direction (negative = the other way); right after a guide was added, a typed length
    replaces it (Undo + add, the M4b/M6b retype pattern).
  - Ctrl toggles guide creation (SketchUp's "+" cursor): measuring only reports
    "Distance: 2.828 m" on the second click and refuses typed lengths (no model rescale; the
    SketchUp resize feature is deferred). Ctrl also drops the retype chance.
  - Preview: the measure segment, plus the guide line to be (its view span) in line mode.
  - The anchor is the start point, so axis snaps and locks work from it.
- **Edit menu with "Delete Guides"** (enabled when the open context has guides), next to File,
  rather than a toolbar button. The ui tests' screen grew from 1000x700 to the window's default
  1280x800: the toolbar (now with Tape Measure) needs about 1150 points (overflow on narrower
  windows deferred).
- **Ctrl routing**: `Editor::toggle_copy` now also reaches the Tape Measure; `copies()` (which
  also gates "x" arrays) is unchanged.

## File Structure

| File | Responsibility |
|---|---|
| `crates/kernel/src/guides.rs` (new) | add/erase/transform/copy guides; `Scene::move_guides` |
| `crates/kernel/src/{ids,model,data,group,scene,lib}.rs` | `GuideId`, `Guide`, guide ops, `GuideData`, guides through groups and explode |
| `crates/session/src/{command,query,document,file,lib}.rs`, `document/groups.rs` | guide commands, guide queries, guides in edit commands, schema 2 |
| `crates/session/src/camera.rs` | `near`, `line_span` |
| `crates/render/src/guides.rs` (new) | `GuideShape`, per-frame dashed guide lines and point crosses |
| `crates/render/src/{scene,renderer,lib}.rs` | collect guides; draw them each frame |
| `crates/tools/src/inference.rs` | guide snaps and crossings, `guide_under_where` |
| `crates/tools/src/{select,erase,move_tool,overlay}.rs` | pick, box-select, outline guides; marker colors |
| `crates/tools/src/tape.rs` (new) | `TapeMeasureTool` |
| `crates/tools/src/{tool,lib}.rs` | `ToolKind::TapeMeasure` (`t`) |
| `crates/app/src/{editor,ui,app,info}.rs` | Ctrl for the tape, Delete Guides, Edit menu, KeyT, Entity Info |
| `docs/agent-api.md`, spec | documentation |

The code below was written and verified in a scratch copy of `dev` at 261b074. Each task was checked on its own with `cargo fmt --all --check`, clippy with `-D warnings`, and the full test suite. Workspace test totals after each task: 586, 597, 605 (from 574).

Code for new files is given in full. Changes to existing files are given as unified diffs against the state the previous task leaves.

---

### Task 1: Guides as model data and commands

**Files:**
- Create: `crates/kernel/src/guides.rs`
- Modify:
  - kernel: `ids.rs` (`GuideId`, `EntityId::Guide` "g"), `model.rs` (`Guide`, `Op::AddGuide`/`RemoveGuide`, guides map, validation), `data.rs` (`GuideData`, load checks), `group.rs` (explode lifts guides out; duplicate copies them), `scene.rs`, `lib.rs`
  - session: `command.rs` (`AddGuideLine`, `AddGuidePoint`, `DeleteGuides`), `query.rs` (guide kinds and info), `document.rs` (guides in edit commands), `document/groups.rs` (guides-only refusal, guides move into groups), `file.rs` (`SCHEMA_VERSION` 2), `lib.rs`
  - compile-only arms: `crates/tools/src/{erase,move_tool}.rs`, `crates/app/src/{info,editor}.rs`
  - `docs/agent-api.md`
- Test: `crates/kernel/tests/guides.rs` (new, 5), `crates/session/tests/guides.rs` (new, 7), `crates/api/tests/dispatch.rs` (`describe` lists the new commands)

**Interfaces:**
- Consumes: `Model::record/alloc/bump/take_changes`, `Scene::edit`, `Document::{edit_mesh, parts, copy_placed_changes, check_placed}`, `check_coordinates`.
- Produces:
  - `kernel::GuideId(pub u64)`, `kernel::EntityId::Guide(GuideId)`
  - `pub enum kernel::Guide { Line { point: DVec3, direction: DVec3 }, Point { position: DVec3 } }` with `transformed(&self, t: DAffine3) -> Guide` and `anchor(&self) -> DVec3`
  - `kernel::Op::AddGuide(GuideId, Guide)`, `Op::RemoveGuide(GuideId, Guide)`
  - `Model::guide(&self, GuideId) -> Option<&Guide>`, `Model::guides(&self) -> impl Iterator<Item = (GuideId, &Guide)>`
  - `Model::add_guide(&mut self, guide: Guide) -> Result<ChangeSet, KernelError>`, `erase_guides(&mut self, &[GuideId]) -> ChangeSet`, `transform_guides(&mut self, &[GuideId], DAffine3) -> ChangeSet`, `copy_guides(&mut self, &[GuideId], &[DAffine3]) -> ChangeSet`
  - `Scene::move_guides(&mut self, from: MeshId, to: MeshId, ids: &[GuideId]) -> SceneChange`
  - `kernel::GuideData { pub id: u64, pub point: [f64; 3], pub direction: Option<[f64; 3]> }`, `MeshData.guides`
  - `session::Command::{AddGuideLine { point: [f64; 3], direction: [f64; 3] }, AddGuidePoint { position: [f64; 3] }, DeleteGuides}`
  - `session::{GuideKind, GuideInfo}`, `EntityKind::Guide`, `QueryResult::Guide`, `Entities.guides`
  - `session::file::SCHEMA_VERSION = 2`

- [ ] **Step 1: Write the failing tests**

Create `crates/kernel/tests/guides.rs`:
```rust
use glam::{DAffine3, DVec3};
use kernel::{
    DefinitionKind, EntityId, Guide, GuideData, GuideId, MeshId, Model, Scene, SceneData,
};

fn p(x: f64, y: f64, z: f64) -> DVec3 {
    DVec3::new(x, y, z)
}

fn line(point: DVec3, direction: DVec3) -> Guide {
    Guide::Line { point, direction }
}

/// The id of the one guide a change created.
fn created_guide(created: &[EntityId]) -> GuideId {
    match created {
        [EntityId::Guide(g)] => *g,
        other => panic!("expected one guide, got {other:?}"),
    }
}

#[test]
fn guides_are_added_erased_moved_and_copied_as_undoable_changes() {
    let mut m = Model::new();
    let c = m
        .add_guide(line(p(1.0, 0.0, 0.0), p(0.0, 3.0, 0.0)))
        .unwrap();
    let g = created_guide(&c.summary().created);
    assert_eq!(
        m.guide(g),
        Some(&line(p(1.0, 0.0, 0.0), DVec3::Y)),
        "the direction is normalized"
    );
    let point = created_guide(
        &m.add_guide(Guide::Point {
            position: p(2.0, 2.0, 0.0),
        })
        .unwrap()
        .summary()
        .created,
    );
    m.validate().unwrap();
    let before = m.clone();

    let moved = m.transform_guides(&[g, point], DAffine3::from_translation(p(0.0, 0.0, 1.0)));
    let s = moved.summary();
    assert_eq!(s.modified, vec![EntityId::Guide(g), EntityId::Guide(point)]);
    assert!(s.created.is_empty() && s.deleted.is_empty());
    assert_eq!(m.guide(g), Some(&line(p(1.0, 0.0, 1.0), DVec3::Y)));
    m.apply(&moved.inverse());
    assert_eq!(m, before);

    let copies = m.copy_guides(
        &[point],
        &[
            DAffine3::from_translation(DVec3::X),
            DAffine3::from_translation(DVec3::X * 2.0),
        ],
    );
    assert_eq!(copies.summary().created.len(), 2);
    assert_eq!(m.guides().count(), 4);
    m.apply(&copies.inverse());
    assert_eq!(m, before);

    let erased = m.erase_guides(&[g, GuideId(999)]);
    assert_eq!(erased.summary().deleted, vec![EntityId::Guide(g)]);
    m.apply(&erased.inverse());
    assert_eq!(m, before);
}

#[test]
fn guides_need_finite_numbers_and_a_direction() {
    let mut m = Model::new();
    assert!(m.add_guide(line(DVec3::ZERO, DVec3::ZERO)).is_err());
    assert!(m.add_guide(line(p(f64::NAN, 0.0, 0.0), DVec3::X)).is_err());
    assert!(
        m.add_guide(Guide::Point {
            position: p(0.0, f64::INFINITY, 0.0)
        })
        .is_err()
    );
    assert_eq!(m.guides().count(), 0);
}

/// A root holding a square and a guide line along its front side, offset 1 m.
fn scene_with_guides() -> (Scene, GuideId) {
    let mut m = Model::new();
    m.add_polygon(&[
        p(0.0, 0.0, 0.0),
        p(2.0, 0.0, 0.0),
        p(2.0, 2.0, 0.0),
        p(0.0, 2.0, 0.0),
    ])
    .unwrap();
    let mut s = Scene::from_root(m);
    let g = s
        .edit(MeshId::Root, |m| {
            m.add_guide(line(p(0.0, -1.0, 0.0), DVec3::X))
        })
        .unwrap()
        .summary()
        .created;
    (s, created_guide(&g))
}

#[test]
fn guides_round_trip_through_data_and_old_data_has_none() {
    let (s, g) = scene_with_guides();
    let json = serde_json::to_string(&s.to_data()).unwrap();
    assert!(json.contains("\"guides\""), "{json}");
    let back = Scene::from_data(&serde_json::from_str(&json).unwrap()).unwrap();
    assert_eq!(back, s);
    assert_eq!(back.root().guide(g), s.root().guide(g));

    // Data written before guides existed has no "guides" key, and none are written when
    // there are none.
    let old = Scene::from_root(Model::new());
    let json = serde_json::to_string(&old.to_data()).unwrap();
    assert!(!json.contains("guides"), "{json}");
    let data: SceneData = serde_json::from_str(&json).unwrap();
    assert_eq!(Scene::from_data(&data).unwrap(), old);
}

#[test]
fn broken_guides_are_rejected() {
    let (s, _) = scene_with_guides();
    let rejects = |f: &dyn Fn(&mut SceneData), expected: &str| {
        let mut data = s.to_data();
        f(&mut data);
        let err = Scene::from_data(&data).unwrap_err();
        assert!(err.contains(expected), "{err:?} lacks {expected:?}");
    };
    rejects(
        &|d| d.root.guides[0].direction = Some([0.0, 2.0, 0.0]),
        "not a unit vector",
    );
    rejects(
        &|d| d.root.guides[0].point = [f64::NAN, 0.0, 0.0],
        "not finite",
    );
    rejects(
        &|d| {
            let v = d.root.vertices[0].id;
            d.root.guides.push(GuideData {
                id: v,
                point: [0.0; 3],
                direction: None,
            });
        },
        "share an id",
    );
}

#[test]
fn grouping_keeps_guides_and_copies_and_explodes_carry_them() {
    let (mut s, g) = scene_with_guides();
    let edges: Vec<_> = s.root().edges().map(|(id, _)| id).collect();
    let faces: Vec<_> = s.root().faces().map(|(id, _)| id).collect();
    let (i, _) = s
        .make_group(MeshId::Root, &edges, &faces, &[], DefinitionKind::Group, "")
        .unwrap();
    let d = MeshId::Definition(s.root().instance(i).unwrap().definition);
    let moved = s.move_guides(MeshId::Root, d, &[g]);
    assert_eq!(s.root().guides().count(), 0);
    assert!(
        s.mesh(d).unwrap().guide(g).is_some(),
        "the guide keeps its id"
    );
    s.validate().unwrap();
    s.apply(&moved.inverse());
    assert!(s.root().guide(g).is_some());
    s.apply(&moved);

    // A copied group gets a copy of the guide; exploding it brings the guide out, placed.
    s.copy_instances(MeshId::Root, &[i], p(5.0, 0.0, 0.0), 1)
        .unwrap();
    let copy = s
        .root()
        .instances()
        .map(|(id, _)| id)
        .find(|c| *c != i)
        .unwrap();
    let inner = s
        .mesh(MeshId::Definition(
            s.root().instance(copy).unwrap().definition,
        ))
        .unwrap();
    assert_eq!(inner.guides().count(), 1);
    assert!(inner.guide(g).is_none(), "copies get new ids");
    s.explode(MeshId::Root, copy).unwrap();
    let out: Vec<_> = s.root().guides().map(|(_, g)| *g).collect();
    assert_eq!(out, vec![line(p(5.0, -1.0, 0.0), DVec3::X)]);
    s.validate().unwrap();
}
```

Create `crates/session/tests/guides.rs`:
```rust
//! Guide lines and guide points, driven through JSON commands.

use std::io::Write;
use std::path::PathBuf;

use glam::DVec2;
use serde_json::{Value, json};
use session::{Command, Document, ErrorKind, Query};

fn doc() -> Document {
    Document::new(DVec2::new(800.0, 600.0))
}

fn run(d: &mut Document, v: Value) -> Result<Value, (ErrorKind, Vec<String>)> {
    let cmd: Command = serde_json::from_value(v).expect("valid command json");
    d.execute(cmd)
        .map(|o| serde_json::to_value(o).unwrap())
        .map_err(|e| (e.kind, e.ids.iter().map(|i| i.to_string()).collect()))
}

fn ask(d: &Document, q: Value) -> Value {
    let q: Query = serde_json::from_value(q).unwrap();
    serde_json::to_value(d.query(&q).unwrap()).unwrap()
}

/// The guides of the open context as returned by `entities`.
fn guides(d: &Document) -> Vec<Value> {
    ask(d, json!({"type": "entities", "kinds": ["guide"]}))["guides"]
        .as_array()
        .unwrap()
        .clone()
}

fn undo_depth(d: &Document) -> u64 {
    ask(d, json!({"type": "history"}))["undo_depth"]
        .as_u64()
        .unwrap()
}

/// A guide line along X through (0, -1, 0) (g0) and a guide point at (1, 1, 0) (g1).
fn two_guides(d: &mut Document) {
    let line = run(
        d,
        json!({"type": "add_guide_line", "point": [0, -1, 0], "direction": [3, 0, 0]}),
    )
    .unwrap();
    assert_eq!(line["changes"]["created"], json!(["g0"]));
    let point = run(d, json!({"type": "add_guide_point", "position": [1, 1, 0]})).unwrap();
    assert_eq!(point["changes"]["created"], json!(["g1"]));
}

#[test]
fn guides_are_added_listed_and_undone_one_step_each() {
    let mut d = doc();
    two_guides(&mut d);
    assert_eq!(undo_depth(&d), 2);
    assert_eq!(
        guides(&d),
        vec![
            json!({"id": "g0", "kind": "line", "point": [0.0, -1.0, 0.0], "direction": [1.0, 0.0, 0.0]}),
            json!({"id": "g1", "kind": "point", "point": [1.0, 1.0, 0.0]}),
        ]
    );
    let one = ask(&d, json!({"type": "entity", "id": "g1"}));
    assert_eq!(one["type"], "guide");
    assert_eq!(one["kind"], "point");
    // Guides are no geometry: no vertices, edges or faces.
    let stats = ask(&d, json!({"type": "stats"}));
    assert_eq!(
        (stats["vertices"].as_u64(), stats["edges"].as_u64()),
        (Some(0), Some(0))
    );

    run(&mut d, json!({"type": "undo"})).unwrap();
    assert_eq!(guides(&d).len(), 1);
    run(&mut d, json!({"type": "redo"})).unwrap();
    assert_eq!(guides(&d).len(), 2);
}

#[test]
fn bad_guides_are_refused() {
    let mut d = doc();
    for bad in [
        json!({"type": "add_guide_line", "point": [0, 0, 0], "direction": [0, 0, 0]}),
        json!({"type": "add_guide_line", "point": [2e6, 0, 0], "direction": [1, 0, 0]}),
        json!({"type": "add_guide_point", "position": [0, -2e6, 0]}),
    ] {
        assert_eq!(run(&mut d, bad).unwrap_err().0, ErrorKind::InvalidInput);
    }
    assert!(guides(&d).is_empty());
    assert_eq!(undo_depth(&d), 0);
}

#[test]
fn guides_are_selected_erased_and_deleted_all_at_once() {
    let mut d = doc();
    two_guides(&mut d);
    run(&mut d, json!({"type": "select", "ids": ["g0", "g1"]})).unwrap();
    let erased = run(&mut d, json!({"type": "erase", "ids": ["g0"]})).unwrap();
    assert_eq!(erased["changes"]["deleted"], json!(["g0"]));
    assert_eq!(
        ask(&d, json!({"type": "selection"}))["ids"],
        json!(["g1"]),
        "an erased guide leaves the selection"
    );
    assert_eq!(
        run(&mut d, json!({"type": "erase", "ids": ["g0"]})).unwrap_err(),
        (ErrorKind::NotFound, vec!["g0".into()])
    );

    run(
        &mut d,
        json!({"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [2, 2, 0]}),
    )
    .unwrap();
    let depth = undo_depth(&d);
    let deleted = run(&mut d, json!({"type": "delete_guides"})).unwrap();
    assert_eq!(deleted["changes"]["deleted"], json!(["g1"]));
    assert!(guides(&d).is_empty());
    assert_eq!(
        ask(&d, json!({"type": "stats"}))["faces"],
        1,
        "geometry stays"
    );
    assert_eq!(undo_depth(&d), depth + 1);
    // Nothing left to delete: no error and no undo step.
    run(&mut d, json!({"type": "delete_guides"})).unwrap();
    assert_eq!(undo_depth(&d), depth + 1);
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert_eq!(guides(&d).len(), 1);
}

#[test]
fn guides_move_turn_and_copy_with_the_rest() {
    let mut d = doc();
    two_guides(&mut d);
    let moved = run(
        &mut d,
        json!({"type": "move", "ids": ["g0", "g1"], "offset": [0, 0, 2]}),
    )
    .unwrap();
    assert_eq!(moved["changes"]["modified"], json!(["g0", "g1"]));
    assert_eq!(guides(&d)[0]["point"], json!([0.0, -1.0, 2.0]));
    assert_eq!(guides(&d)[1]["point"], json!([1.0, 1.0, 2.0]));

    run(
        &mut d,
        json!({"type": "rotate", "ids": ["g0"], "center": [0, 0, 0], "angle": 90}),
    )
    .unwrap();
    let g0 = &guides(&d)[0];
    assert_eq!(g0["point"], json!([1.0, 0.0, 2.0]));
    assert_eq!(g0["direction"], json!([0.0, 1.0, 0.0]));

    let copied = run(
        &mut d,
        json!({"type": "copy", "ids": ["g1"], "offset": [1, 0, 0], "count": 2}),
    )
    .unwrap();
    assert_eq!(copied["changes"]["created"].as_array().unwrap().len(), 2);
    let points: Vec<Value> = guides(&d).iter().map(|g| g["point"].clone()).collect();
    assert!(points.contains(&json!([3.0, 1.0, 2.0])), "{points:?}");

    assert_eq!(
        run(
            &mut d,
            json!({"type": "move", "ids": ["g1"], "offset": [1e6, 0, 0]})
        )
        .unwrap_err()
        .0,
        ErrorKind::InvalidInput,
        "a guide cannot leave the coordinate limit"
    );
}

#[test]
fn guides_go_into_groups_and_come_out_when_exploded() {
    let mut d = doc();
    two_guides(&mut d);
    assert_eq!(
        run(&mut d, json!({"type": "make_group", "ids": ["g0"]}))
            .unwrap_err()
            .0,
        ErrorKind::InvalidInput,
        "guides alone make no group"
    );
    run(
        &mut d,
        json!({"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [2, 2, 0]}),
    )
    .unwrap();
    let face = ask(&d, json!({"type": "entities", "kinds": ["face"]}))["faces"][0]["id"].clone();
    run(&mut d, json!({"type": "make_group", "ids": [face, "g1"]})).unwrap();
    let group = ask(&d, json!({"type": "selection"}))["ids"][0].clone();
    assert_eq!(guides(&d).len(), 1, "only g0 stays at the root");
    run(&mut d, json!({"type": "open_instance", "id": group})).unwrap();
    assert_eq!(guides(&d)[0]["id"], "g1", "the guide keeps its id inside");
    run(&mut d, json!({"type": "close_instance"})).unwrap();

    run(&mut d, json!({"type": "explode", "ids": [group]})).unwrap();
    let points: Vec<Value> = guides(&d).iter().map(|g| g["point"].clone()).collect();
    assert_eq!(points.len(), 2);
    assert!(points.contains(&json!([1.0, 1.0, 0.0])));
}

#[test]
fn within_finds_guide_points_but_never_guide_lines() {
    let mut d = doc();
    two_guides(&mut d);
    let found = ask(
        &d,
        json!({"type": "entities", "within": {"min": [-5, -5, -5], "max": [5, 5, 5]}}),
    );
    let ids: Vec<&Value> = found["guides"]
        .as_array()
        .unwrap()
        .iter()
        .map(|g| &g["id"])
        .collect();
    assert_eq!(ids, vec![&json!("g1")]);
}

/// A fresh path in the test scratch directory.
fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("session-guides");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    let _ = std::fs::remove_file(&path);
    path
}

/// Rewrites the document inside a saved file.
fn edit_file(path: &PathBuf, f: impl FnOnce(&mut Value)) {
    let mut zip = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
    let mut document: Value =
        serde_json::from_reader(zip.by_name("document.json").unwrap()).unwrap();
    f(&mut document);
    let mut zip = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
    zip.start_file("document.json", zip::write::SimpleFileOptions::default())
        .unwrap();
    zip.write_all(&serde_json::to_vec(&document).unwrap())
        .unwrap();
    zip.finish().unwrap();
}

#[test]
fn guides_are_saved_but_not_exported_and_old_files_still_open() {
    let mut d = doc();
    two_guides(&mut d);
    let path = scratch("guides.skr");
    let path_text = path.to_str().unwrap();
    run(&mut d, json!({"type": "save", "path": path_text})).unwrap();
    let mut e = doc();
    run(&mut e, json!({"type": "open", "path": path_text})).unwrap();
    assert_eq!(guides(&e), guides(&d));

    let obj = scratch("guides.obj");
    assert_eq!(
        run(
            &mut d,
            json!({"type": "export", "path": obj.to_str().unwrap()})
        )
        .unwrap_err()
        .0,
        ErrorKind::InvalidInput,
        "guides are not faces: nothing to export"
    );

    // A version 1 file (before guides) opens.
    edit_file(&path, |doc| {
        doc["schema_version"] = json!(1);
        doc["model"]["root"]
            .as_object_mut()
            .unwrap()
            .remove("guides");
    });
    let mut old = doc();
    run(&mut old, json!({"type": "open", "path": path_text})).unwrap();
    assert!(guides(&old).is_empty());

    // A guide beyond the coordinate limit does not.
    run(&mut d, json!({"type": "save", "path": path_text})).unwrap();
    edit_file(&path, |doc| {
        doc["model"]["root"]["guides"][1]["point"] = json!([2e6, 0, 0]);
    });
    let mut far = doc();
    assert_eq!(
        run(&mut far, json!({"type": "open", "path": path_text}))
            .unwrap_err()
            .0,
        ErrorKind::File
    );
}
```

Apply to `crates/api/tests/dispatch.rs`:
```diff
--- a/crates/api/tests/dispatch.rs
+++ b/crates/api/tests/dispatch.rs
@@ -126,8 +126,11 @@ fn describe_lists_methods_and_schemas() {
         "erase",
         "move",
         "copy",
         "rotate",
+        "add_guide_line",
+        "add_guide_point",
+        "delete_guides",
         "select",
         "undo",
         "redo",
         "set_camera",
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p session --test guides`
Expected: runtime panics: ``valid command json: Error("unknown variant `add_guide_line`, expected one of `add_edge`, ...``. (`cargo test -p kernel --test guides` fails to compile: ``unresolved imports `kernel::Guide` ``.)

- [ ] **Step 3: Implement**

Create `crates/kernel/src/guides.rs`:
```rust
//! Guide lines and guide points: construction geometry that tools snap to. Guides never merge
//! with edges or each other and bound no faces.

use std::collections::BTreeSet;

use glam::DAffine3;

use crate::edit::KernelError;
use crate::ids::GuideId;
use crate::model::{ChangeSet, Guide, Model};
use crate::scene::{MeshId, Scene, SceneChange};

impl Model {
    /// Adds a guide as one ChangeSet; a guide line's direction is normalized. Non-finite
    /// numbers and a zero direction are `NotFinite`.
    pub fn add_guide(&mut self, guide: Guide) -> Result<ChangeSet, KernelError> {
        let guide = match guide {
            Guide::Line { point, direction } => Guide::Line {
                point,
                direction: direction.try_normalize().ok_or(KernelError::NotFinite)?,
            },
            g => g,
        };
        if !guide.anchor().is_finite() {
            return Err(KernelError::NotFinite);
        }
        self.insert_guide(guide);
        Ok(self.take_changes())
    }

    /// Removes guides as one ChangeSet; unknown ids are skipped.
    pub fn erase_guides(&mut self, ids: &[GuideId]) -> ChangeSet {
        for id in ids.iter().collect::<BTreeSet<_>>() {
            if self.guide(*id).is_some() {
                self.remove_guide(*id);
            }
        }
        self.take_changes()
    }

    /// Moves guides by the rigid motion `t` as one ChangeSet, keeping their ids (each is
    /// reported as modified); unknown ids are skipped.
    pub fn transform_guides(&mut self, ids: &[GuideId], t: DAffine3) -> ChangeSet {
        for id in ids.iter().collect::<BTreeSet<_>>() {
            if self.guide(*id).is_some() {
                let g = self.remove_guide(*id);
                self.insert_guide_as(*id, g.transformed(t));
            }
        }
        self.take_changes()
    }

    /// Adds a copy of each guide per rigid motion in `places`, with new ids, as one ChangeSet;
    /// unknown ids are skipped.
    pub fn copy_guides(&mut self, ids: &[GuideId], places: &[DAffine3]) -> ChangeSet {
        let originals: Vec<Guide> = ids
            .iter()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .filter_map(|id| self.guide(*id).copied())
            .collect();
        for t in places {
            for g in &originals {
                self.insert_guide(g.transformed(*t));
            }
        }
        self.take_changes()
    }
}

impl Scene {
    /// Moves guides from mesh `from` to mesh `to`, keeping their ids and positions (for a
    /// group made where they were drawn). Unknown ids are skipped.
    pub fn move_guides(&mut self, from: MeshId, to: MeshId, ids: &[GuideId]) -> SceneChange {
        let mut moved = Vec::new();
        let left = self
            .edit(from, |m| {
                for id in ids.iter().collect::<BTreeSet<_>>() {
                    if m.guide(*id).is_some() {
                        moved.push((*id, m.remove_guide(*id)));
                    }
                }
                Ok(m.take_changes())
            })
            .expect("removing guides cannot fail");
        let placed = self
            .edit(to, |m| {
                for (id, g) in moved {
                    m.insert_guide_as(id, g);
                }
                Ok(m.take_changes())
            })
            .expect("adding guides cannot fail");
        SceneChange::join([left, placed])
    }
}
```

Apply to `crates/kernel/src/ids.rs`:
```diff
--- a/crates/kernel/src/ids.rs
+++ b/crates/kernel/src/ids.rs
@@ -16,18 +16,21 @@ macro_rules! id_type {
 id_type!(VertexId);
 id_type!(EdgeId);
 id_type!(FaceId);
 id_type!(InstanceId);
+id_type!(GuideId);
 
-/// Any kernel entity. Serialized as a short string: "v12", "e3", "f7", "i4".
+/// Any kernel entity. Serialized as a short string: "v12", "e3", "f7", "i4", "g9".
 #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
 #[serde(try_from = "String", into = "String")]
 pub enum EntityId {
     Vertex(VertexId),
     Edge(EdgeId),
     Face(FaceId),
     /// A placed group or component.
     Instance(InstanceId),
+    /// A guide line or guide point (see `Guide`).
+    Guide(GuideId),
 }
 
 impl fmt::Display for EntityId {
     fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
@@ -35,8 +38,9 @@ impl fmt::Display for EntityId {
             EntityId::Vertex(id) => write!(f, "v{}", id.0),
             EntityId::Edge(id) => write!(f, "e{}", id.0),
             EntityId::Face(id) => write!(f, "f{}", id.0),
             EntityId::Instance(id) => write!(f, "i{}", id.0),
+            EntityId::Guide(id) => write!(f, "g{}", id.0),
         }
     }
 }
 
@@ -49,8 +53,9 @@ impl FromStr for EntityId {
             'v' => Ok(EntityId::Vertex(VertexId(n))),
             'e' => Ok(EntityId::Edge(EdgeId(n))),
             'f' => Ok(EntityId::Face(FaceId(n))),
             'i' => Ok(EntityId::Instance(InstanceId(n))),
+            'g' => Ok(EntityId::Guide(GuideId(n))),
             _ => Err(format!("invalid entity id {s:?}")),
         }
     }
 }
@@ -69,19 +74,20 @@ fn parse_prefixed(s: &str) -> Option<(char, u64)> {
     }
     Some((kind, digits.parse().ok()?))
 }
 
-/// Ids appear in JSON as strings: "v12" (vertex), "e3" (edge), "f7" (face), "i4" (instance).
+/// Ids appear in JSON as strings: "v12" (vertex), "e3" (edge), "f7" (face), "i4" (instance),
+/// "g9" (guide).
 impl schemars::JsonSchema for EntityId {
     fn schema_name() -> std::borrow::Cow<'static, str> {
         "EntityId".into()
     }
 
     fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
         schemars::json_schema!({
             "type": "string",
-            "pattern": "^[vefi](0|[1-9][0-9]*)$",
-            "description": "Entity id: v<n> vertex, e<n> edge, f<n> face, i<n> instance"
+            "pattern": "^[vefig](0|[1-9][0-9]*)$",
+            "description": "Entity id: v<n> vertex, e<n> edge, f<n> face, i<n> instance, g<n> guide"
         })
     }
 }
 
@@ -159,13 +165,15 @@ mod tests {
             EntityId::Vertex(VertexId(12)),
             EntityId::Edge(EdgeId(0)),
             EntityId::Face(FaceId(7)),
             EntityId::Instance(InstanceId(4)),
+            EntityId::Guide(GuideId(9)),
         ] {
             assert_eq!(id.to_string().parse::<EntityId>().unwrap(), id);
         }
         assert_eq!(EntityId::Face(FaceId(7)).to_string(), "f7");
         assert_eq!(EntityId::Instance(InstanceId(4)).to_string(), "i4");
+        assert_eq!(EntityId::Guide(GuideId(9)).to_string(), "g9");
         assert_eq!("d3".parse::<DefinitionId>(), Ok(DefinitionId(3)));
         assert_eq!(DefinitionId(3).to_string(), "d3");
         for s in ["", "d", "f3", "d03", "d-1"] {
             assert!(s.parse::<DefinitionId>().is_err(), "{s:?} parsed");
```

Apply to `crates/kernel/src/model.rs`:
```diff
--- a/crates/kernel/src/model.rs
+++ b/crates/kernel/src/model.rs
@@ -4,9 +4,9 @@ use std::sync::OnceLock;
 use glam::{DAffine3, DVec3};
 use serde::{Deserialize, Serialize};
 
 use crate::geom::{LENGTH_TOL, vector_area};
-use crate::ids::{DefinitionId, EdgeId, EntityId, FaceId, InstanceId, VertexId};
+use crate::ids::{DefinitionId, EdgeId, EntityId, FaceId, GuideId, InstanceId, VertexId};
 use crate::index::Index;
 
 #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
 pub struct Vertex {
@@ -34,8 +34,40 @@ pub struct Instance {
     /// Maps the definition's coordinates into the coordinates of the mesh holding the instance.
     pub transform: DAffine3,
 }
 
+/// Construction geometry drawn by the Tape Measure: not part of any face, never exported.
+#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
+pub enum Guide {
+    /// The infinite line through `point` along the unit vector `direction`.
+    Line { point: DVec3, direction: DVec3 },
+    /// A single point.
+    Point { position: DVec3 },
+}
+
+impl Guide {
+    /// The guide moved by a rigid motion `t`.
+    pub fn transformed(&self, t: DAffine3) -> Guide {
+        match *self {
+            Guide::Line { point, direction } => Guide::Line {
+                point: t.transform_point3(point),
+                direction: t.transform_vector3(direction).normalize(),
+            },
+            Guide::Point { position } => Guide::Point {
+                position: t.transform_point3(position),
+            },
+        }
+    }
+
+    /// The point a guide line passes through, or the guide point itself.
+    pub fn anchor(&self) -> DVec3 {
+        match *self {
+            Guide::Line { point, .. } => point,
+            Guide::Point { position } => position,
+        }
+    }
+}
+
 /// One primitive, invertible change to the model.
 #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
 pub enum Op {
     AddVertex(VertexId, Vertex),
@@ -49,8 +81,10 @@ pub enum Op {
     AddInstance(InstanceId, Instance),
     RemoveInstance(InstanceId, Instance),
     /// An instance's transform changed from the first to the second.
     TransformInstance(InstanceId, DAffine3, DAffine3),
+    AddGuide(GuideId, Guide),
+    RemoveGuide(GuideId, Guide),
 }
 
 impl Op {
     fn inverse(&self) -> Op {
@@ -64,8 +98,10 @@ impl Op {
             Op::MoveVertex(id, from, to) => Op::MoveVertex(id, to, from),
             Op::AddInstance(id, i) => Op::RemoveInstance(id, i),
             Op::RemoveInstance(id, i) => Op::AddInstance(id, i),
             Op::TransformInstance(id, from, to) => Op::TransformInstance(id, to, from),
+            Op::AddGuide(id, g) => Op::RemoveGuide(id, g),
+            Op::RemoveGuide(id, g) => Op::AddGuide(id, g),
         }
     }
 
     /// The entity this op touches and whether it exists before and after the op.
@@ -80,8 +116,10 @@ impl Op {
             Op::RemoveFace(id, _) => (EntityId::Face(*id), true, false),
             Op::AddInstance(id, _) => (EntityId::Instance(*id), false, true),
             Op::RemoveInstance(id, _) => (EntityId::Instance(*id), true, false),
             Op::TransformInstance(id, _, _) => (EntityId::Instance(*id), true, true),
+            Op::AddGuide(id, _) => (EntityId::Guide(*id), false, true),
+            Op::RemoveGuide(id, _) => (EntityId::Guide(*id), true, false),
         }
     }
 }
 
@@ -130,16 +168,17 @@ impl ChangeSet {
         s
     }
 }
 
-/// The geometry of one mesh, plus the instances placed in it. Every mutation is recorded as an
+/// The geometry of one mesh, plus the instances and guides placed in it. Every mutation is recorded as an
 /// `Op` until `take_changes`.
 #[derive(Clone, Debug, Default)]
 pub struct Model {
     vertices: BTreeMap<VertexId, Vertex>,
     edges: BTreeMap<EdgeId, Edge>,
     faces: BTreeMap<FaceId, Face>,
     instances: BTreeMap<InstanceId, Instance>,
+    guides: BTreeMap<GuideId, Guide>,
     next_id: u64,
     pending: Vec<Op>,
     /// Built on first use after a change; see `Model::index`.
     index: OnceLock<Index>,
@@ -151,8 +190,9 @@ impl PartialEq for Model {
         self.vertices == other.vertices
             && self.edges == other.edges
             && self.faces == other.faces
             && self.instances == other.instances
+            && self.guides == other.guides
     }
 }
 
 impl Model {
@@ -166,22 +206,25 @@ impl Model {
         vertices: BTreeMap<VertexId, Vertex>,
         edges: BTreeMap<EdgeId, Edge>,
         faces: BTreeMap<FaceId, Face>,
         instances: BTreeMap<InstanceId, Instance>,
+        guides: BTreeMap<GuideId, Guide>,
     ) -> Model {
         let next_id = vertices
             .keys()
             .map(|k| k.0)
             .chain(edges.keys().map(|k| k.0))
             .chain(faces.keys().map(|k| k.0))
             .chain(instances.keys().map(|k| k.0))
+            .chain(guides.keys().map(|k| k.0))
             .max()
             .map_or(0, |m| m.saturating_add(1));
         Model {
             vertices,
             edges,
             faces,
             instances,
+            guides,
             next_id,
             pending: Vec::new(),
             index: OnceLock::new(),
         }
@@ -225,9 +268,17 @@ impl Model {
     pub fn instances(&self) -> impl Iterator<Item = (InstanceId, &Instance)> {
         self.instances.iter().map(|(k, v)| (*k, v))
     }
 
-    /// True when the mesh holds no vertices and no instances.
+    pub fn guide(&self, id: GuideId) -> Option<&Guide> {
+        self.guides.get(&id)
+    }
+
+    pub fn guides(&self) -> impl Iterator<Item = (GuideId, &Guide)> {
+        self.guides.iter().map(|(k, v)| (*k, v))
+    }
+
+    /// True when the mesh holds no vertices and no instances (guides do not count).
     pub fn is_empty(&self) -> bool {
         self.vertices.is_empty() && self.instances.is_empty()
     }
 
@@ -324,8 +375,15 @@ impl Model {
                 if let Some(i) = self.instances.get_mut(&id) {
                     i.transform = to;
                 }
             }
+            Op::AddGuide(id, g) => {
+                self.bump(id.0);
+                self.guides.insert(id, g);
+            }
+            Op::RemoveGuide(id, _) => {
+                self.guides.remove(&id);
+            }
         }
     }
 
     fn bump(&mut self, used: u64) {
@@ -413,8 +471,26 @@ impl Model {
         self.record(Op::RemoveInstance(id, i.clone()));
         i
     }
 
+    pub(crate) fn insert_guide(&mut self, guide: Guide) -> GuideId {
+        let id = GuideId(self.alloc());
+        self.record(Op::AddGuide(id, guide));
+        id
+    }
+
+    /// Adds a guide under an id it had elsewhere (a guide moved between meshes) or before
+    /// (a guide moved in place).
+    pub(crate) fn insert_guide_as(&mut self, id: GuideId, guide: Guide) {
+        self.record(Op::AddGuide(id, guide));
+    }
+
+    pub(crate) fn remove_guide(&mut self, id: GuideId) -> Guide {
+        let g = self.guides[&id];
+        self.record(Op::RemoveGuide(id, g));
+        g
+    }
+
     /// Moves instances by `offset` as one ChangeSet. Each listed instance moves once; unknown
     /// ids are skipped.
     pub fn move_instances(&mut self, ids: &[InstanceId], offset: DVec3) -> ChangeSet {
         self.transform_instances(ids, DAffine3::from_translation(offset))
@@ -485,8 +561,23 @@ impl Model {
             if !i.transform.is_finite() {
                 return Err(format!("instance {id:?} has a non-finite transform"));
             }
         }
+        for (id, g) in &self.guides {
+            let ok = match g {
+                Guide::Line { point, direction } => {
+                    point.is_finite()
+                        && direction.is_finite()
+                        && (direction.length() - 1.0).abs() <= 1e-9
+                }
+                Guide::Point { position } => position.is_finite(),
+            };
+            if !ok {
+                return Err(format!(
+                    "guide {id:?} is not finite or its direction is not a unit vector"
+                ));
+            }
+        }
         for (id, f) in &self.faces {
             if (f.normal.length() - 1.0).abs() > 1e-9 {
                 return Err(format!("face {id:?} normal is not unit length"));
             }
```

Apply to `crates/kernel/src/data.rs`:
```diff
--- a/crates/kernel/src/data.rs
+++ b/crates/kernel/src/data.rs
@@ -4,10 +4,10 @@ use std::collections::BTreeMap;
 
 use glam::{DAffine3, DVec3};
 use serde::{Deserialize, Serialize};
 
-use crate::ids::{DefinitionId, EdgeId, FaceId, InstanceId, VertexId};
-use crate::model::{Edge, Face, Instance, Model, Vertex};
+use crate::ids::{DefinitionId, EdgeId, FaceId, GuideId, InstanceId, VertexId};
+use crate::model::{Edge, Face, Guide, Instance, Model, Vertex};
 use crate::scene::{Definition, DefinitionKind, Scene};
 
 /// The largest id a file may use: every id up to it is exact as a JSON (double) number.
 pub const MAX_ID: u64 = 1 << 53;
@@ -32,8 +32,11 @@ pub struct MeshData {
     pub vertices: Vec<VertexData>,
     pub edges: Vec<EdgeData>,
     pub faces: Vec<FaceData>,
     pub instances: Vec<InstanceData>,
+    /// Absent in files written before guides existed (and when there are none).
+    #[serde(default, skip_serializing_if = "Vec::is_empty")]
+    pub guides: Vec<GuideData>,
 }
 
 #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
 #[serde(deny_unknown_fields)]
@@ -66,8 +69,19 @@ pub struct InstanceData {
     /// Column-major 3x4: x axis, y axis, z axis, translation.
     pub transform: [f64; 12],
 }
 
+/// A guide line through `point` along `direction` (a unit vector), or, without `direction`, a
+/// guide point at `point`.
+#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
+#[serde(deny_unknown_fields)]
+pub struct GuideData {
+    pub id: u64,
+    pub point: [f64; 3],
+    #[serde(default, skip_serializing_if = "Option::is_none")]
+    pub direction: Option<[f64; 3]>,
+}
+
 #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
 #[serde(deny_unknown_fields)]
 pub struct DefinitionData {
     pub id: u64,
@@ -159,8 +173,23 @@ fn mesh_data(m: &Model) -> MeshData {
                 definition: i.definition.0,
                 transform: i.transform.to_cols_array(),
             })
             .collect(),
+        guides: m
+            .guides()
+            .map(|(id, g)| match *g {
+                Guide::Line { point, direction } => GuideData {
+                    id: id.0,
+                    point: point.to_array(),
+                    direction: Some(direction.to_array()),
+                },
+                Guide::Point { position } => GuideData {
+                    id: id.0,
+                    point: position.to_array(),
+                    direction: None,
+                },
+            })
+            .collect(),
     }
 }
 
 fn check_id(what: &str, id: u64) -> Result<(), String> {
@@ -231,17 +260,41 @@ fn mesh(data: &MeshData) -> Result<Model, String> {
         if instances.insert(InstanceId(i.id), instance).is_some() {
             return Err(format!("instance i{} appears twice", i.id));
         }
     }
-    let ids = vertices.len() + edges.len() + faces.len() + instances.len();
+    let mut guides = BTreeMap::new();
+    for g in &data.guides {
+        check_id("guide g", g.id)?;
+        finite(&g.point, format!("guide g{}", g.id))?;
+        let point = DVec3::from(g.point);
+        let guide = match g.direction {
+            None => Guide::Point { position: point },
+            Some(d) => {
+                finite(&d, format!("guide g{}", g.id))?;
+                let direction = DVec3::from(d);
+                if (direction.length() - 1.0).abs() > 1e-9 {
+                    return Err(format!(
+                        "guide g{} has a direction that is not a unit vector",
+                        g.id
+                    ));
+                }
+                Guide::Line { point, direction }
+            }
+        };
+        if guides.insert(GuideId(g.id), guide).is_some() {
+            return Err(format!("guide g{} appears twice", g.id));
+        }
+    }
+    let ids = vertices.len() + edges.len() + faces.len() + instances.len() + guides.len();
     let distinct: std::collections::BTreeSet<u64> = vertices
         .keys()
         .map(|k| k.0)
         .chain(edges.keys().map(|k| k.0))
         .chain(faces.keys().map(|k| k.0))
         .chain(instances.keys().map(|k| k.0))
+        .chain(guides.keys().map(|k| k.0))
         .collect();
     if distinct.len() != ids {
         return Err("two entities of different kinds share an id".into());
     }
-    Ok(Model::from_parts(vertices, edges, faces, instances))
+    Ok(Model::from_parts(vertices, edges, faces, instances, guides))
 }
```

Apply to `crates/kernel/src/group.rs`:
```diff
--- a/crates/kernel/src/group.rs
+++ b/crates/kernel/src/group.rs
@@ -6,9 +6,9 @@ use glam::{DAffine3, DVec3};
 
 use crate::edit::{KernelError, loop_has_side};
 use crate::geom::LENGTH_TOL;
 use crate::ids::{DefinitionId, EdgeId, FaceId, InstanceId, VertexId};
-use crate::model::{Instance, Model};
+use crate::model::{Guide, Instance, Model};
 use crate::scene::{DefinitionKind, MeshId, Scene, SceneChange};
 use crate::transform::Stencil;
 
 impl Scene {
@@ -61,9 +61,9 @@ impl Scene {
 
 impl Scene {
     /// Replaces an instance in mesh `parent` by its definition's geometry, placed by the
     /// instance's transform and drawn like new edges, so it merges with what it touches.
-    /// Instances inside it become instances in `parent`, with new ids; a group inside a
+    /// Instances and guides inside it come out into `parent`, with new ids; a group inside a
     /// component gets a copy of its definition. A group's definition goes with its last
     /// instance; a component's stays.
     pub fn explode(
         &mut self,
@@ -87,8 +87,13 @@ impl Scene {
             .mesh
             .instances()
             .map(|(_, i)| i.clone())
             .collect();
+        let guides: Vec<Guide> = definition
+            .mesh
+            .guides()
+            .map(|(_, g)| g.transformed(placed.transform))
+            .collect();
         let mut copied = SceneChange::default();
         let mut children = Vec::new();
         for i in inner {
             let definition = match self.definition(i.definition).map(|d| d.kind) {
@@ -109,8 +114,11 @@ impl Scene {
             m.draw_stencil(&stencil, &[placed.transform]);
             for child in children {
                 m.insert_instance(child);
             }
+            for g in guides {
+                m.insert_guide(g);
+            }
             Ok(m.take_changes())
         })?;
         Ok(SceneChange::join([copied, exploded, self.prune()]))
     }
@@ -181,15 +189,16 @@ impl Scene {
         })?;
         Ok(SceneChange::join([steps, placed]))
     }
 
-    /// A new group definition holding a copy of `id`'s geometry, with new ids; groups inside it
-    /// are copied too, components inside are placed again.
+    /// A new group definition holding a copy of `id`'s geometry and guides, with new ids;
+    /// groups inside it are copied too, components inside are placed again.
     fn duplicate(&mut self, id: DefinitionId) -> (DefinitionId, SceneChange) {
         let original = self.definition(id).expect("copy of a missing definition");
         let (name, kind) = (original.name.clone(), original.kind);
         let stencil = whole(&original.mesh);
         let children: Vec<Instance> = original.mesh.instances().map(|(_, i)| i.clone()).collect();
+        let guides: Vec<Guide> = original.mesh.guides().map(|(_, g)| *g).collect();
         let mut change = SceneChange::default();
         let mut placed = Vec::new();
         for child in children {
             let definition = match self.definition(child.definition).map(|d| d.kind) {
@@ -211,8 +220,11 @@ impl Scene {
                 m.draw_stencil(&stencil, &[DAffine3::IDENTITY]);
                 for i in placed {
                     m.insert_instance(i);
                 }
+                for g in guides {
+                    m.insert_guide(g);
+                }
                 Ok(m.take_changes())
             })
             .expect("drawing a copy cannot fail");
         (copy, SceneChange::join([change, added.into(), filled]))
```

Apply to `crates/kernel/src/scene.rs`:
```diff
--- a/crates/kernel/src/scene.rs
+++ b/crates/kernel/src/scene.rs
@@ -439,5 +439,6 @@ fn entity_ids(m: &Model) -> impl Iterator<Item = EntityId> + '_ {
         .map(|(id, _)| EntityId::Vertex(id))
         .chain(m.edges().map(|(id, _)| EntityId::Edge(id)))
         .chain(m.faces().map(|(id, _)| EntityId::Face(id)))
         .chain(m.instances().map(|(id, _)| EntityId::Instance(id)))
+        .chain(m.guides().map(|(id, _)| EntityId::Guide(id)))
 }
```

Apply to `crates/kernel/src/lib.rs`:
```diff
--- a/crates/kernel/src/lib.rs
+++ b/crates/kernel/src/lib.rs
@@ -6,8 +6,9 @@ mod edit;
 mod erase;
 mod faces;
 pub mod geom;
 mod group;
+mod guides;
 mod ids;
 mod index;
 mod mesh;
 mod model;
@@ -18,12 +19,13 @@ mod transform;
 mod triangles;
 mod weld;
 
 pub use data::{
-    DefinitionData, EdgeData, FaceData, InstanceData, MAX_ID, MeshData, SceneData, VertexData,
+    DefinitionData, EdgeData, FaceData, GuideData, InstanceData, MAX_ID, MeshData, SceneData,
+    VertexData,
 };
 pub use edit::KernelError;
-pub use ids::{DefinitionId, EdgeId, EntityId, FaceId, InstanceId, VertexId};
+pub use ids::{DefinitionId, EdgeId, EntityId, FaceId, GuideId, InstanceId, VertexId};
 pub use index::{Index, Shape};
-pub use model::{ChangeSet, ChangeSummary, Edge, Face, Instance, Model, Op, Vertex};
+pub use model::{ChangeSet, ChangeSummary, Edge, Face, Guide, Instance, Model, Op, Vertex};
 pub use scene::{Definition, DefinitionKind, MeshId, Scene, SceneChange, Step};
 pub use triangles::Triangle;
```

Apply to `crates/session/src/command.rs`:
```diff
--- a/crates/session/src/command.rs
+++ b/crates/session/src/command.rs
@@ -44,22 +44,36 @@ pub enum Command {
     PushPull {
         face: EntityId,
         distance: f64,
     },
-    /// Erase edges, faces and instances as one undo step. An erased edge takes the faces it
-    /// bounds with it, except that two faces in one plane meeting there merge into one.
-    /// Vertices go away with their last edge and cannot be erased directly. A group's
+    /// Add a guide line: the infinite line through `point` along `direction` (any non-zero
+    /// length), as one undo step. Guides are construction geometry: tools snap to them, they
+    /// bound no faces and are not exported.
+    AddGuideLine {
+        point: [f64; 3],
+        direction: [f64; 3],
+    },
+    /// Add a guide point at `position`, as one undo step.
+    AddGuidePoint {
+        position: [f64; 3],
+    },
+    /// Erase every guide of the open context, as one undo step (nothing to erase is not an
+    /// error and makes no undo step).
+    DeleteGuides,
+    /// Erase edges, faces, instances and guides as one undo step. An erased edge takes the
+    /// faces it bounds with it, except that two faces in one plane meeting there merge into
+    /// one. Vertices go away with their last edge and cannot be erased directly. A group's
     /// definition goes with its last instance.
     Erase {
         ids: Vec<EntityId>,
     },
-    /// Move vertices, edges, faces and instances by `offset` meters as one undo step. Faces
-    /// that share some of the moved vertices stretch and may tilt, but must stay flat.
+    /// Move vertices, edges, faces, instances and guides by `offset` meters as one undo step.
+    /// Faces that share some of the moved vertices stretch and may tilt, but must stay flat.
     Move {
         ids: Vec<EntityId>,
         offset: [f64; 3],
     },
-    /// Copy edges, faces and instances `count` times (default 1), copy k shifted by
+    /// Copy edges, faces, instances and guides `count` times (default 1), copy k shifted by
     /// `k * offset`, as one undo step. Copies merge with geometry they touch; copied faces keep
     /// their orientation. A copied component instance places the same definition; a copied
     /// group gets a definition of its own.
     Copy {
@@ -67,11 +81,11 @@ pub enum Command {
         offset: [f64; 3],
         #[serde(default)]
         count: Option<u32>,
     },
-    /// Rotate vertices, edges, faces (their vertices) and instances by `angle` degrees about
-    /// the line through `center` along `axis` (default Z), counter-clockwise looking down the
-    /// axis, as one undo step. Faces that share some of the turned vertices stretch and may
+    /// Rotate vertices, edges, faces (their vertices), instances and guides by `angle` degrees
+    /// about the line through `center` along `axis` (default Z), counter-clockwise looking
+    /// down the axis, as one undo step. Faces that share some of the turned vertices stretch and may
     /// tilt, but must stay flat. With `copies`, the originals stay and that many copies are
     /// made instead, copy k turned by `k * angle`, under the rules of `Copy`.
     Rotate {
         ids: Vec<EntityId>,
@@ -81,11 +95,11 @@ pub enum Command {
         angle: f64,
         #[serde(default)]
         copies: Option<u32>,
     },
-    /// Turn edges, faces (with their edges) and instances into a group: they move into a new
-    /// definition, placed by one new instance where they were. The instance becomes the
-    /// selection.
+    /// Turn edges, faces (with their edges), instances and guides into a group: they move into
+    /// a new definition, placed by one new instance where they were. Guides alone cannot be
+    /// grouped. The instance becomes the selection.
     MakeGroup {
         ids: Vec<EntityId>,
     },
     /// Like `MakeGroup`, but the definition is a named component: copies of its instances
@@ -93,10 +107,10 @@ pub enum Command {
     MakeComponent {
         ids: Vec<EntityId>,
         name: String,
     },
-    /// Replace instances by their geometry, which merges with what it touches. Instances
-    /// inside them come out as instances. What comes out becomes the selection.
+    /// Replace instances by their geometry, which merges with what it touches. Instances and
+    /// guides inside them come out as instances and guides. What comes out becomes the selection.
     Explode {
         ids: Vec<EntityId>,
     },
     /// Edit the definition an instance of the open context places: its entities become the
@@ -143,9 +157,9 @@ pub enum Command {
         path: String,
         #[serde(default)]
         format: Option<ExportFormat>,
     },
-    /// Change the selection of edges, faces and instances. Not recorded in undo history.
+    /// Change the selection of edges, faces, instances and guides. Not recorded in undo history.
     Select {
         ids: Vec<EntityId>,
         #[serde(default)]
         mode: SelectMode,
```

Apply to `crates/session/src/query.rs`:
```diff
--- a/crates/session/src/query.rs
+++ b/crates/session/src/query.rs
@@ -6,20 +6,21 @@ use serde::{Deserialize, Serialize};
 /// context (see `Command::OpenInstance`).
 #[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
 #[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
 pub enum Query {
-    /// Vertices, edges, faces and instances. An entity is returned when `ids` is empty or lists
-    /// it, `kinds` is empty or lists its kind, and `within` is absent or contains all its
-    /// vertices (an instance: its bounding box); unknown ids are simply absent from the result.
+    /// Vertices, edges, faces, instances and guides. An entity is returned when `ids` is empty
+    /// or lists it, `kinds` is empty or lists its kind, and `within` is absent or contains all
+    /// its vertices (an instance: its bounding box; a guide point: its position; a guide line
+    /// is in no box); unknown ids are simply absent from the result.
     Entities {
         #[serde(default)]
         ids: Vec<EntityId>,
         #[serde(default)]
         kinds: Vec<EntityKind>,
         #[serde(default)]
         within: Option<Bounds>,
     },
-    /// One entity by id ("v3", "e7", "f2", "i4").
+    /// One entity by id ("v3", "e7", "f2", "i4", "g9").
     Entity {
         id: EntityId,
     },
     Camera,
@@ -60,8 +61,9 @@ pub enum EntityKind {
     Vertex,
     Edge,
     Face,
     Instance,
+    Guide,
 }
 
 #[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
 pub struct VertexInfo {
@@ -100,8 +102,27 @@ pub struct InstanceInfo {
     /// an empty definition.
     pub bounds: Option<Bounds>,
 }
 
+/// Whether a guide is a line or a point.
+#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
+#[serde(rename_all = "snake_case")]
+pub enum GuideKind {
+    Line,
+    Point,
+}
+
+/// A guide line (through `point` along the unit vector `direction`) or a guide point (at
+/// `point`, no `direction`).
+#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
+pub struct GuideInfo {
+    pub id: EntityId,
+    pub kind: GuideKind,
+    pub point: [f64; 3],
+    #[serde(default, skip_serializing_if = "Option::is_none")]
+    pub direction: Option<[f64; 3]>,
+}
+
 #[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
 pub struct DefinitionInfo {
     pub id: DefinitionId,
     pub name: String,
@@ -119,14 +140,16 @@ pub enum QueryResult {
         vertices: Vec<VertexInfo>,
         edges: Vec<EdgeInfo>,
         faces: Vec<FaceInfo>,
         instances: Vec<InstanceInfo>,
+        guides: Vec<GuideInfo>,
         revision: u64,
     },
     Vertex(VertexInfo),
     Edge(EdgeInfo),
     Face(FaceInfo),
     Instance(InstanceInfo),
+    Guide(GuideInfo),
     Camera {
         eye: [f64; 3],
         target: [f64; 3],
         fov_y_deg: f64,
```

Apply to `crates/session/src/document.rs`:
```diff
--- a/crates/session/src/document.rs
+++ b/crates/session/src/document.rs
@@ -2,17 +2,18 @@ use std::collections::{BTreeSet, VecDeque};
 use std::path::{Path, PathBuf};
 
 use glam::{DAffine3, DVec2, DVec3};
 use kernel::{
-    ChangeSet, EdgeId, EntityId, FaceId, Instance, InstanceId, KernelError, MeshId, Model, Scene,
-    SceneChange, VertexId,
+    ChangeSet, EdgeId, EntityId, FaceId, Guide, GuideId, Instance, InstanceId, KernelError, MeshId,
+    Model, Scene, SceneChange, VertexId,
 };
 
 use crate::camera::Camera;
 use crate::circle::{DEFAULT_CIRCLE_SEGMENTS, circle_points};
 use crate::command::{BatchError, Command, CommandError, ErrorKind, Outcome, SelectMode};
 use crate::query::{
-    ChangeEvent, DefinitionInfo, EdgeInfo, EntityKind, FaceInfo, Query, QueryResult, VertexInfo,
+    ChangeEvent, DefinitionInfo, EdgeInfo, EntityKind, FaceInfo, GuideInfo, GuideKind, Query,
+    QueryResult, VertexInfo,
 };
 use crate::rectangle::rectangle_corners;
 use crate::rotate::rotation;
 
@@ -218,8 +219,11 @@ impl Document {
             }
             Command::AddEdge { .. }
             | Command::AddRectangle { .. }
             | Command::AddCircle { .. }
+            | Command::AddGuideLine { .. }
+            | Command::AddGuidePoint { .. }
+            | Command::DeleteGuides
             | Command::PushPull { .. }
             | Command::Erase { .. }
             | Command::Move { .. }
             | Command::Copy { .. }
@@ -250,8 +254,22 @@ impl Document {
                 radius,
                 normal,
                 segments,
             } => self.add_circle_changes(*center, *radius, *normal, *segments),
+            Command::AddGuideLine { point, direction } => {
+                self.add_guide_line_changes(*point, *direction)
+            }
+            Command::AddGuidePoint { position } => check_coordinates(&[*position]).and_then(|()| {
+                self.add_guide(Guide::Point {
+                    position: DVec3::from(*position),
+                })
+            }),
+            Command::DeleteGuides => {
+                let all: Vec<GuideId> = self.model().guides().map(|(id, _)| id).collect();
+                Ok(self
+                    .edit_mesh(|m| Ok(m.erase_guides(&all)))
+                    .expect("erasing guides cannot fail"))
+            }
             Command::PushPull { face, distance } => self.push_pull_changes(*face, *distance),
             Command::Erase { ids } => self.erase_changes(ids),
             Command::Move { ids, offset } => self.move_changes(ids, *offset),
             Command::Copy { ids, offset, count } => {
@@ -412,8 +430,31 @@ impl Document {
         self.edit_mesh(|m| m.add_edge(DVec3::from(start), DVec3::from(end)))
             .map_err(|e| CommandError::new(ErrorKind::InvalidInput, e.to_string()))
     }
 
+    fn add_guide_line_changes(
+        &mut self,
+        point: [f64; 3],
+        direction: [f64; 3],
+    ) -> Result<SceneChange, CommandError> {
+        check_coordinates(&[point, direction])?;
+        let Some(direction) = DVec3::from(direction).try_normalize() else {
+            return Err(CommandError::new(
+                ErrorKind::InvalidInput,
+                "direction must be a non-zero vector",
+            ));
+        };
+        self.add_guide(Guide::Line {
+            point: DVec3::from(point),
+            direction,
+        })
+    }
+
+    fn add_guide(&mut self, guide: Guide) -> Result<SceneChange, CommandError> {
+        self.edit_mesh(|m| m.add_guide(guide))
+            .map_err(|e| CommandError::new(ErrorKind::InvalidInput, e.to_string()))
+    }
+
     fn add_circle_changes(
         &mut self,
         center: [f64; 3],
         radius: f64,
@@ -490,9 +531,9 @@ impl Document {
     ) -> Result<SceneChange, CommandError> {
         if ids.is_empty() {
             return Err(CommandError::new(
                 ErrorKind::InvalidInput,
-                "ids must list at least one vertex, edge, face or instance",
+                "ids must list at least one vertex, edge, face, instance or guide",
             ));
         }
         let missing: Vec<EntityId> = ids.iter().copied().filter(|id| !self.exists(*id)).collect();
         if !missing.is_empty() {
@@ -501,20 +542,16 @@ impl Document {
             );
         }
         check_coordinates(&[offset])?;
         let vertices = self.vertex_ids(ids);
-        let instances: Vec<InstanceId> = ids
-            .iter()
-            .filter_map(|id| match id {
-                EntityId::Instance(i) => Some(*i),
-                _ => None,
-            })
-            .collect();
+        let (instances, guides) = (instance_ids(ids), guide_ids(ids));
         let offset = DVec3::from(offset);
-        self.check_placed(&vertices, &instances, &[DAffine3::from_translation(offset)])?;
+        let shift = DAffine3::from_translation(offset);
+        self.check_placed(&vertices, &instances, &guides, &[shift])?;
         self.edit_mesh(|m| {
             let mut c = m.move_vertices(&vertices, offset)?;
             c.ops.extend(m.move_instances(&instances, offset).ops);
+            c.ops.extend(m.transform_guides(&guides, shift).ops);
             Ok(c)
         })
         .map_err(|e| self.kernel_error(e, ids))
     }
@@ -529,9 +566,9 @@ impl Document {
     ) -> Result<SceneChange, CommandError> {
         let invalid = |m: &str| CommandError::new(ErrorKind::InvalidInput, m);
         if ids.is_empty() {
             return Err(invalid(
-                "ids must list at least one vertex, edge, face or instance",
+                "ids must list at least one vertex, edge, face, instance or guide",
             ));
         }
         check_coordinates(&[center, axis.unwrap_or([0.0, 0.0, 1.0])])?;
         let Some(axis) = DVec3::from(axis.unwrap_or([0.0, 0.0, 1.0])).try_normalize() else {
@@ -561,9 +598,14 @@ impl Document {
             }
             let places: Vec<DAffine3> = (1..=count)
                 .map(|k| rotation(center, axis, angle * k as f64))
                 .collect();
-            self.check_placed(&self.vertex_ids(ids), &parts.instances, &places)?;
+            self.check_placed(
+                &self.vertex_ids(ids),
+                &parts.instances,
+                &parts.guides,
+                &places,
+            )?;
             return self.copy_placed_changes(ids, &parts, &places);
         }
         let missing: Vec<EntityId> = ids.iter().copied().filter(|id| !self.exists(*id)).collect();
         if !missing.is_empty() {
@@ -571,20 +613,15 @@ impl Document {
                 CommandError::new(ErrorKind::NotFound, "no such entities").with_ids(missing)
             );
         }
         let vertices = self.vertex_ids(ids);
-        let instances: Vec<InstanceId> = ids
-            .iter()
-            .filter_map(|id| match id {
-                EntityId::Instance(i) => Some(*i),
-                _ => None,
-            })
-            .collect();
+        let (instances, guides) = (instance_ids(ids), guide_ids(ids));
         let turn = rotation(center, axis, angle);
-        self.check_placed(&vertices, &instances, &[turn])?;
+        self.check_placed(&vertices, &instances, &guides, &[turn])?;
         self.edit_mesh(|m| {
             let mut c = m.transform_vertices(&vertices, turn)?;
             c.ops.extend(m.transform_instances(&instances, turn).ops);
+            c.ops.extend(m.transform_guides(&guides, turn).ops);
             Ok(c)
         })
         .map_err(|e| self.kernel_error(e, ids))
     }
@@ -600,9 +637,14 @@ impl Document {
         let offset = DVec3::from(offset);
         let places: Vec<DAffine3> = (1..=count)
             .map(|k| DAffine3::from_translation(offset * k as f64))
             .collect();
-        self.check_placed(&self.vertex_ids(ids), &parts.instances, &places)?;
+        self.check_placed(
+            &self.vertex_ids(ids),
+            &parts.instances,
+            &parts.guides,
+            &places,
+        )?;
         if offset.length() < kernel::geom::LENGTH_TOL {
             return Err(self.kernel_error(KernelError::ZeroDistance, ids));
         }
         self.copy_placed_changes(ids, &parts, &places)
@@ -612,9 +654,9 @@ impl Document {
     fn copy_parts(&self, ids: &[EntityId], count: u32) -> Result<Parts, CommandError> {
         if ids.is_empty() {
             return Err(CommandError::new(
                 ErrorKind::InvalidInput,
-                "ids must list at least one edge, face or instance",
+                "ids must list at least one edge, face, instance or guide",
             ));
         }
         if !(1..=MAX_COPIES).contains(&count) {
             return Err(CommandError::new(
@@ -652,25 +694,35 @@ impl Document {
                     return Err(self.kernel_error(e, ids));
                 }
             }
         };
-        Ok(SceneChange::join([geometry, placed]))
+        let guides = self
+            .edit_mesh(|m| Ok(m.copy_guides(&parts.guides, places)))
+            .expect("copying guides cannot fail");
+        Ok(SceneChange::join([geometry, placed, guides]))
     }
 
-    /// Rejects moves, rotations or copies that would put a vertex or an instance's bounding
-    /// box (its origin when it has nothing in it) beyond `MAX_COORDINATE` under any of
-    /// `places`.
+    /// Rejects moves, rotations or copies that would put a vertex, an instance's bounding
+    /// box (its origin when it has nothing in it) or a guide's point beyond `MAX_COORDINATE`
+    /// under any of `places`.
     fn check_placed(
         &self,
         vertices: &BTreeSet<VertexId>,
         instances: &[InstanceId],
+        guides: &[GuideId],
         places: &[DAffine3],
     ) -> Result<(), CommandError> {
         let m = self.model();
         let mut moved: Vec<[f64; 3]> = Vec::new();
         for t in places {
             let at = |p: DVec3| t.transform_point3(p).to_array();
             moved.extend(vertices.iter().map(|v| at(m.position(*v).unwrap())));
+            moved.extend(
+                guides
+                    .iter()
+                    .filter_map(|g| m.guide(*g))
+                    .map(|g| at(g.anchor())),
+            );
             for i in instances.iter().filter_map(|i| m.instance(*i)) {
                 let placed = Instance {
                     definition: i.definition,
                     transform: *t * i.transform,
@@ -697,9 +749,9 @@ impl Document {
             match *id {
                 EntityId::Vertex(v) => {
                     out.insert(v);
                 }
-                EntityId::Instance(_) => {}
+                EntityId::Instance(_) | EntityId::Guide(_) => {}
                 EntityId::Edge(e) => out.extend(m.edge(e).map(|e| e.v).into_iter().flatten()),
                 EntityId::Face(f) => {
                     if let Some(f) = m.face(f) {
                         out.extend(f.outer.iter().chain(f.holes.iter().flatten()));
@@ -722,21 +774,26 @@ impl Document {
     fn erase_changes(&mut self, ids: &[EntityId]) -> Result<SceneChange, CommandError> {
         if ids.is_empty() {
             return Err(CommandError::new(
                 ErrorKind::InvalidInput,
-                "ids must list at least one edge, face or instance",
+                "ids must list at least one edge, face, instance or guide",
             ));
         }
         let parts = self.parts(ids)?;
         let erased = self
-            .edit_mesh(|m| Ok(m.erase(&parts.edges, &parts.faces)))
+            .edit_mesh(|m| {
+                let mut c = m.erase(&parts.edges, &parts.faces);
+                c.ops.extend(m.erase_guides(&parts.guides).ops);
+                Ok(c)
+            })
             .expect("erase cannot fail");
         let mesh = self.active();
         let removed = self.scene.erase_instances(mesh, &parts.instances);
         Ok(SceneChange::join([erased, removed]))
     }
 
-    /// Splits ids (each counted once) into edges, faces and instances of the open context.
+    /// Splits ids (each counted once) into edges, faces, instances and guides of the open
+    /// context.
     /// Vertex ids are invalid input and unknown ids are not found; both errors list the
     /// offending ids.
     fn parts(&self, ids: &[EntityId]) -> Result<Parts, CommandError> {
         let vertices: Vec<EntityId> = ids
@@ -746,9 +803,9 @@ impl Document {
             .collect();
         if !vertices.is_empty() {
             return Err(CommandError::new(
                 ErrorKind::InvalidInput,
-                "only edges, faces and instances are accepted here, not vertices",
+                "only edges, faces, instances and guides are accepted here, not vertices",
             )
             .with_ids(vertices));
         }
         let missing: Vec<EntityId> = ids.iter().copied().filter(|id| !self.exists(*id)).collect();
@@ -762,8 +819,9 @@ impl Document {
             match *id {
                 EntityId::Edge(e) => parts.edges.push(e),
                 EntityId::Face(f) => parts.faces.push(f),
                 EntityId::Instance(i) => parts.instances.push(i),
+                EntityId::Guide(g) => parts.guides.push(g),
                 EntityId::Vertex(_) => {}
             }
         }
         Ok(parts)
@@ -776,8 +834,9 @@ impl Document {
             EntityId::Vertex(v) => m.vertex(v).is_some(),
             EntityId::Edge(e) => m.edge(e).is_some(),
             EntityId::Face(f) => m.face(f).is_some(),
             EntityId::Instance(i) => m.instance(i).is_some(),
+            EntityId::Guide(g) => m.guide(g).is_some(),
         }
     }
 
     fn record(&mut self, changes: SceneChange) -> Outcome {
@@ -846,8 +905,13 @@ impl Document {
                         .instances()
                         .filter(|(id, _)| wanted(EntityId::Instance(*id), EntityKind::Instance))
                         .map(|(id, _)| self.instance_info(id))
                         .collect(),
+                    guides: m
+                        .guides()
+                        .filter(|(id, _)| wanted(EntityId::Guide(*id), EntityKind::Guide))
+                        .map(|(id, _)| self.guide_info(id))
+                        .collect(),
                     revision: self.revision,
                 }
             }
             Query::Entity { id } => {
@@ -871,8 +935,11 @@ impl Document {
                         m.instance(i)
                             .map(|_| self.instance_info(i))
                             .ok_or_else(missing)?,
                     ),
+                    EntityId::Guide(g) => QueryResult::Guide(
+                        m.guide(g).map(|_| self.guide_info(g)).ok_or_else(missing)?,
+                    ),
                 }
             }
             Query::Camera => QueryResult::Camera {
                 eye: self.camera.eye.to_array(),
@@ -938,9 +1005,9 @@ impl Document {
         })
     }
 
     /// Positions of the vertices an entity is made of; for an instance, the corners of its
-    /// bounding box.
+    /// bounding box; for a guide point, its position (none for a guide line).
     fn vertices_of(&self, id: EntityId) -> Vec<[f64; 3]> {
         let m = self.model();
         let ids: Vec<VertexId> = match id {
             EntityId::Vertex(v) => vec![v],
@@ -951,8 +1018,14 @@ impl Document {
                     .flatten()
                     .map(|p| p.to_array())
                     .collect();
             }
+            EntityId::Guide(g) => {
+                return match m.guide(g) {
+                    Some(Guide::Point { position }) => vec![position.to_array()],
+                    _ => Vec::new(),
+                };
+            }
             EntityId::Edge(e) => m.edge(e).map(|e| e.v.to_vec()).unwrap_or_default(),
             EntityId::Face(f) => m
                 .face(f)
                 .map(|f| {
@@ -985,8 +1058,21 @@ impl Document {
             end: self.model().position(b).unwrap().to_array(),
         }
     }
 
+    fn guide_info(&self, id: GuideId) -> GuideInfo {
+        let (kind, point, direction) = match *self.model().guide(id).unwrap() {
+            Guide::Line { point, direction } => (GuideKind::Line, point, Some(direction)),
+            Guide::Point { position } => (GuideKind::Point, position, None),
+        };
+        GuideInfo {
+            id: EntityId::Guide(id),
+            kind,
+            point: point.to_array(),
+            direction: direction.map(|d| d.to_array()),
+        }
+    }
+
     fn face_info(&self, id: FaceId) -> FaceInfo {
         let f = self.model().face(id).unwrap();
         let ids = |l: &Vec<VertexId>| l.iter().map(|v| EntityId::Vertex(*v)).collect();
         FaceInfo {
@@ -998,14 +1084,35 @@ impl Document {
         }
     }
 }
 
-/// Edges, faces and instances named by a command.
+/// Edges, faces, instances and guides named by a command.
 #[derive(Default)]
 struct Parts {
     edges: Vec<EdgeId>,
     faces: Vec<FaceId>,
     instances: Vec<InstanceId>,
+    guides: Vec<GuideId>,
+}
+
+/// The instances among `ids`.
+fn instance_ids(ids: &[EntityId]) -> Vec<InstanceId> {
+    ids.iter()
+        .filter_map(|id| match id {
+            EntityId::Instance(i) => Some(*i),
+            _ => None,
+        })
+        .collect()
+}
+
+/// The guides among `ids`.
+fn guide_ids(ids: &[EntityId]) -> Vec<GuideId> {
+    ids.iter()
+        .filter_map(|id| match id {
+            EntityId::Guide(g) => Some(*g),
+            _ => None,
+        })
+        .collect()
 }
 
 /// Rejects non-finite or out-of-range coordinates before they reach the kernel or camera.
 fn check_coordinates(points: &[[f64; 3]]) -> Result<(), CommandError> {
```

Apply to `crates/session/src/document/groups.rs`:
```diff
--- a/crates/session/src/document/groups.rs
+++ b/crates/session/src/document/groups.rs
@@ -1,9 +1,9 @@
 //! Groups, components and the open context.
 
 use std::collections::BTreeSet;
 
-use kernel::{DefinitionKind, EntityId, KernelError, SceneChange};
+use kernel::{DefinitionKind, EntityId, KernelError, MeshId, SceneChange};
 
 use super::{Document, MAX_COORDINATE, check_coordinates};
 use crate::command::{CommandError, ErrorKind};
 use crate::query::{Bounds, InstanceInfo};
@@ -31,8 +31,15 @@ impl Document {
             }
             Some(n) => (DefinitionKind::Component, n.as_str()),
         };
         let parts = self.parts(ids)?;
+        if parts.edges.is_empty() && parts.faces.is_empty() && parts.instances.is_empty() {
+            return Err(CommandError::new(
+                ErrorKind::InvalidInput,
+                "guides alone cannot be grouped: include an edge, face or instance",
+            )
+            .with_ids(ids.to_vec()));
+        }
         let mesh = self.active();
         let (instance, change) = self
             .scene
             .make_group(
@@ -43,10 +50,14 @@ impl Document {
                 kind,
                 name,
             )
             .map_err(|e| invalid(e, ids))?;
+        let definition = self.model().instance(instance).expect("placed").definition;
+        let guides = self
+            .scene
+            .move_guides(mesh, MeshId::Definition(definition), &parts.guides);
         self.selection = BTreeSet::from([EntityId::Instance(instance)]);
-        Ok(change)
+        Ok(SceneChange::join([change, guides]))
     }
 
     /// Explodes instances and selects what comes out of them.
     pub(super) fn explode_changes(
```

Apply to `crates/session/src/file.rs`:
```diff
--- a/crates/session/src/file.rs
+++ b/crates/session/src/file.rs
@@ -13,10 +13,11 @@ use crate::camera::Camera;
 use crate::document::MAX_COORDINATE;
 
 /// The `format` every file records.
 pub const FORMAT: &str = "tenon";
-/// The schema version this build writes and the newest it reads.
-pub const SCHEMA_VERSION: u32 = 1;
+/// The schema version this build writes and the newest it reads. Version 2 added guides;
+/// version 1 files (no guides) still open.
+pub const SCHEMA_VERSION: u32 = 2;
 /// The archive entry holding the document.
 const ENTRY: &str = "document.json";
 /// The largest `document.json` this build reads, uncompressed.
 const MAX_DOCUMENT_BYTES: u64 = 256 * 1024 * 1024;
@@ -133,9 +134,9 @@ fn read_limited(reader: impl Read, limit: u64) -> Result<Vec<u8>, String> {
     }
     Ok(bytes)
 }
 
-/// Fails, naming the entity, when a vertex or an instance's position is beyond
+/// Fails, naming the entity, when a vertex, an instance's position or a guide's point is beyond
 /// `MAX_COORDINATE`, the limit commands keep to.
 fn check_coordinates(model: &SceneData) -> Result<(), String> {
     let meshes = std::iter::once(&model.root).chain(model.definitions.iter().map(|d| &d.mesh));
     let within = |xs: &[f64]| xs.iter().all(|x| x.abs() <= MAX_COORDINATE);
@@ -143,16 +144,20 @@ fn check_coordinates(model: &SceneData) -> Result<(), String> {
     for mesh in meshes {
         let MeshData {
             vertices,
             instances,
+            guides,
             ..
         } = mesh;
         if let Some(v) = vertices.iter().find(|v| !within(&v.position)) {
             return Err(beyond(format!("vertex v{}", v.id)));
         }
         if let Some(i) = instances.iter().find(|i| !within(&i.transform[9..])) {
             return Err(beyond(format!("instance i{}'s position", i.id)));
         }
+        if let Some(g) = guides.iter().find(|g| !within(&g.point)) {
+            return Err(beyond(format!("guide g{}", g.id)));
+        }
     }
     Ok(())
 }
 
```

Apply to `crates/session/src/lib.rs`:
```diff
--- a/crates/session/src/lib.rs
+++ b/crates/session/src/lib.rs
@@ -14,9 +14,9 @@ pub use camera::{Camera, Ray};
 pub use circle::{DEFAULT_CIRCLE_SEGMENTS, MAX_CIRCLE_SEGMENTS, circle_points};
 pub use command::{BatchError, Command, CommandError, ErrorKind, Outcome, SelectMode};
 pub use document::{Document, MAX_COORDINATE, MAX_COPIES};
 pub use query::{
-    Bounds, ChangeEvent, DefinitionInfo, EdgeInfo, EntityKind, FaceInfo, InstanceInfo, Query,
-    QueryResult, VertexInfo,
+    Bounds, ChangeEvent, DefinitionInfo, EdgeInfo, EntityKind, FaceInfo, GuideInfo, GuideKind,
+    InstanceInfo, Query, QueryResult, VertexInfo,
 };
 pub use rectangle::{rectangle_axes, rectangle_corners};
 pub use rotate::rotation;
```

Apply to `crates/tools/src/erase.rs`:
```diff
--- a/crates/tools/src/erase.rs
+++ b/crates/tools/src/erase.rs
@@ -29,9 +29,9 @@ fn outline(ctx: &ToolContext, id: EntityId) -> Vec<[DVec3; 2]> {
             .into_iter()
             .collect(),
         EntityId::Face(f) => m.face(f).map(|f| face_outline(m, f)).unwrap_or_default(),
         EntityId::Instance(_) => box_outline(ctx, id),
-        EntityId::Vertex(_) => Vec::new(),
+        EntityId::Vertex(_) | EntityId::Guide(_) => Vec::new(),
     }
 }
 
 impl Tool for EraseTool {
```

Apply to `crates/tools/src/move_tool.rs`:
```diff
--- a/crates/tools/src/move_tool.rs
+++ b/crates/tools/src/move_tool.rs
@@ -94,9 +94,9 @@ pub(crate) fn outline(ctx: &ToolContext, ids: &[EntityId]) -> Vec<[DVec3; 2]> {
                     out.extend(face_outline(m, f));
                 }
             }
             EntityId::Instance(_) => out.extend(box_outline(ctx, *id)),
-            EntityId::Vertex(_) => {}
+            EntityId::Vertex(_) | EntityId::Guide(_) => {}
         }
     }
     out
 }
```

Apply to `crates/app/src/info.rs`:
```diff
--- a/crates/app/src/info.rs
+++ b/crates/app/src/info.rs
@@ -34,9 +34,9 @@ fn selection_info(doc: &Document) -> Vec<String> {
         match id {
             EntityId::Edge(e) => edges.push(*e),
             EntityId::Face(f) => faces.push(*f),
             EntityId::Instance(i) => instances.push(*i),
-            EntityId::Vertex(_) => {}
+            EntityId::Vertex(_) | EntityId::Guide(_) => {}
         }
     }
     if let ([], [], [i]) = (&edges[..], &faces[..], &instances[..]) {
         let instance = m.instance(*i);
```

Apply to `crates/app/src/editor.rs`:
```diff
--- a/crates/app/src/editor.rs
+++ b/crates/app/src/editor.rs
@@ -657,10 +657,10 @@ impl Editor {
         }
         self.preview = out.preview;
     }
 
-    /// Selects the edges, faces and instances a command created in the open context (a copied
-    /// group's new contents live in its new definition and are left out).
+    /// Selects the edges, faces, instances and guides a command created in the open context (a
+    /// copied group's new contents live in its new definition and are left out).
     fn select_created(&mut self, outcome: Outcome) {
         let m = self.doc.model();
         let ids = outcome
             .changes
@@ -669,8 +669,9 @@ impl Editor {
             .filter(|id| match *id {
                 EntityId::Edge(e) => m.edge(e).is_some(),
                 EntityId::Face(f) => m.face(f).is_some(),
                 EntityId::Instance(i) => m.instance(i).is_some(),
+                EntityId::Guide(g) => m.guide(g).is_some(),
                 EntityId::Vertex(_) => false,
             })
             .collect();
         self.run(Command::Select {
@@ -1215,9 +1216,9 @@ mod tests {
                     .v
                     .iter()
                     .all(|v| model.position(*v).unwrap().x > 3.5),
                 EntityId::Face(f) => model.face(f).unwrap().outer.len() == 4,
-                EntityId::Vertex(_) | EntityId::Instance(_) => false,
+                EntityId::Vertex(_) | EntityId::Instance(_) | EntityId::Guide(_) => false,
             }
         }));
         // Typing another array must not undo anything.
         type_text(&mut e, "3x");
```

Apply to `docs/agent-api.md`:
```diff
--- a/docs/agent-api.md
+++ b/docs/agent-api.md
@@ -41,12 +41,12 @@ all commands and results.
 Queries:
 
 | Query | Result |
 |---|---|
-| `{"type": "entities"}` | every vertex, edge, face and instance of the open context, plus `revision` |
-| `{"type": "entities", "kinds": ["face"]}` | only the listed kinds (`vertex`, `edge`, `face`, `instance`) |
+| `{"type": "entities"}` | every vertex, edge, face, instance and guide of the open context, plus `revision` |
+| `{"type": "entities", "kinds": ["face"]}` | only the listed kinds (`vertex`, `edge`, `face`, `instance`, `guide`) |
 | `{"type": "entities", "ids": ["v0", "e2"]}` | only the listed ids; unknown ids are absent, not an error |
-| `{"type": "entities", "within": {"min": [0,0,0], "max": [2,2,1]}}` | only entities whose vertices (an instance: its bounding box) all lie in the box, bounds included; an instance with nothing in it is in no box. `ids`, `kinds` and `within` combine (all must match); empty lists mean no filter |
+| `{"type": "entities", "within": {"min": [0,0,0], "max": [2,2,1]}}` | only entities whose vertices (an instance: its bounding box; a guide point: its position) all lie in the box, bounds included; an instance with nothing in it and a guide line are in no box. `ids`, `kinds` and `within` combine (all must match); empty lists mean no filter |
 | `{"type": "entity", "id": "f7"}` | one entity, or a not-found error |
 | `{"type": "stats"}`, `{"type": "camera"}`, `{"type": "history"}` | counts for the open context (plus the number of definitions) and revision, camera, undo/redo depth |
 | `{"type": "selection"}` | the selected edge, face and instance ids, plus `revision` |
 | `{"type": "definitions"}` | every group and component definition: `id`, `name`, `kind`, how many `instances` place it, `edges`, `faces` |
@@ -58,12 +58,13 @@ Unknown fields in commands, queries and params are rejected with `-32602` (a typ
 of silently doing nothing).
 
 Commands: `add_edge`, `add_rectangle`, `add_circle`, `push_pull`, `erase`, `move`, `copy`, `rotate`, `make_group`,
 `make_component`, `explode`, `open_instance`, `close_instance`, `rename_definition`,
-`purge_definitions`, `select`, `undo`, `redo`, `set_camera`, `new`, `open`, `save`, `export`.
+`purge_definitions`, `add_guide_line`, `add_guide_point`, `delete_guides`, `select`, `undo`,
+`redo`, `set_camera`, `new`, `open`, `save`, `export`.
 Units are meters, Z is up, angles in degrees, coordinates within +/-1,000,000 m.
 Entity ids are strings: `v12` (vertex), `e3` (edge), `f7` (face), `i4` (instance of a group or
-component). Definitions are `d5`. Ids are unique across the whole model, including inside
+component), `g9` (guide line or guide point). Definitions are `d5`. Ids are unique across the whole model, including inside
 groups and components.
 
 Modeling commands, each one undo step:
 
@@ -74,16 +75,16 @@ Modeling commands, each one undo step:
 | `{"type": "add_circle", "center": [0,0,0], "radius": 1.5}` | A circle as a regular polygon with `segments` sides (default 24, 3 to 360), in the plane through `center` with `normal` (default `[0,0,1]`). The first corner lies along the plane's u axis (as for `add_rectangle`) and the corners run counter-clockwise around the normal. The radius must be positive, and each side (2r sin(pi/segments)) must be at least 0.01 mm, ten times the merge tolerance. |
 | `{"type": "move", "ids": ["f8"], "offset": [0, 0, 1]}` | Moves vertices, edges and faces (their vertices) and instances by `offset` (an instance must keep its bounding box, or its origin when it holds nothing, within the coordinate limit; so must copies). Faces that share only some moved vertices stretch and may tilt but must stay flat: a move that would bend one is `invalid_input` naming that face. A move that would collapse an edge, turn a face over, make a face cross itself, push a hole out of its face or land a vertex on another is `invalid_input` naming the ids. Moved geometry does not merge with what it ends up touching. Unknown ids are `not_found`. |
 | `{"type": "copy", "ids": ["f8"], "offset": [3, 0, 0], "count": 4}` | Copies edges, faces and instances `count` times (default 1, at most 100); copy k is shifted by `k * offset`. Copies are drawn like new edges, so they merge with what they touch: a copy has the faces its original had, made even where their sides already exist as bare edges, plus faces it closes together with existing edges. A copy drawn on a face splits it and every piece stays a face (also under an opening of the copy); elsewhere an opening in a copied face stays open and a loop the original had no face in gets none. Copied faces keep the orientation of their originals. A copied component instance places the same definition; a copied group gets a definition of its own. Vertex ids are `invalid_input`. |
 | `{"type": "rotate", "ids": ["f8"], "center": [0, 0, 0], "angle": 90}` | Turns vertices, edges and faces (their vertices) and instances by `angle` degrees about the line through `center` along `axis` (default `[0,0,1]`), counter-clockwise looking down the axis (right-hand rule), under the rules of `move`: faces that share only some turned vertices must stay flat, and so on; geometry turned whole keeps its shape. Multiples of 90 degrees are exact. An instance turns with its transform. A zero `axis` and whole turns (0, 360, ..., or within 1e-9 degrees of one) are `invalid_input`. With `"copies": n` (1 to 100) the originals stay and n copies are made under the rules of `copy`, copy k turned by `k * angle` (a radial array; vertex ids are then `invalid_input`, and so is a count where a copy would complete a whole turn and land on the original, e.g. angle 90 with 4 copies). Vertices on the axis do not move, and a rotation that moves nothing changes nothing. |
-| `{"type": "erase", "ids": ["e2", "f8"]}` | Erases edges, faces and instances (a group's definition goes with its last instance). An erased face leaves its edges. An erased edge takes the faces it bounds with it, except that two faces in one plane meeting at it merge into one. Vertices go with their last edge; a vertex left between two collinear edges is healed away. Faces are erased before edges, so erasing a face together with the seam it shares with a coplanar neighbour removes the neighbour too: the seam then bounds only one face. Vertex ids are `invalid_input` and unknown ids are `not_found`, both with `ids`; empty `ids` is `invalid_input`. |
-| `{"type": "make_group", "ids": ["e2", "f8", "i4"]}` | Moves edges, faces (with their edges) and instances into a new group definition and places one instance of it where they were; the instance becomes the selection. Edges that still bound a face left behind stay outside too. Vertex ids are `invalid_input`. |
+| `{"type": "erase", "ids": ["e2", "f8"]}` | Erases edges, faces, instances and guides (a group's definition goes with its last instance). An erased face leaves its edges. An erased edge takes the faces it bounds with it, except that two faces in one plane meeting at it merge into one. Vertices go with their last edge; a vertex left between two collinear edges is healed away. Faces are erased before edges, so erasing a face together with the seam it shares with a coplanar neighbour removes the neighbour too: the seam then bounds only one face. Vertex ids are `invalid_input` and unknown ids are `not_found`, both with `ids`; empty `ids` is `invalid_input`. |
+| `{"type": "make_group", "ids": ["e2", "f8", "i4"]}` | Moves edges, faces (with their edges), instances and guides (keeping their ids) into a new group definition and places one instance of it where they were; the instance becomes the selection. Edges that still bound a face left behind stay outside too. Vertex ids are `invalid_input`, and so are ids naming only guides. |
 | `{"type": "make_component", "ids": ["f8"], "name": "Chair"}` | Like `make_group`, but makes a named component: its copies share the definition, so editing one edits them all. An empty name is `invalid_input`. |
-| `{"type": "explode", "ids": ["i4"]}` | Replaces instances by their geometry, which merges with what it touches; instances inside come out as instances (with new ids). What comes out becomes the selection. Ids that are not instances are `invalid_input`, and so is an instance whose geometry would land beyond the coordinate limit (naming it). |
+| `{"type": "explode", "ids": ["i4"]}` | Replaces instances by their geometry, which merges with what it touches; instances and guides inside come out as instances and guides (with new ids). What comes out becomes the selection. Ids that are not instances are `invalid_input`, and so is an instance whose geometry would land beyond the coordinate limit (naming it). |
 | `{"type": "push_pull", "face": "f8", "distance": 1.5}` | Moves a face along its normal (negative: against it). A face on a flat sheet extrudes into a solid and stays as its base. A face on a solid extrudes a bump, or cuts a recess when pushed in; pushed in exactly as far as a parallel face that holds it (the back of a wall), it cuts an opening through. A face whose neighbors all run along the normal, like a box top, moves and stretches them; moved back flush with the surface around it (a recess or bump pushed back), it closes up and the seams heal. |
 
-`{"type": "select", "ids": ["f8"], "mode": "replace"}` changes the selection (edges, faces and
-instances; `mode` is `replace` (default), `add`, `remove` or `toggle`). It is not an undo step and
+`{"type": "select", "ids": ["f8"], "mode": "replace"}` changes the selection (edges, faces,
+instances and guides; `mode` is `replace` (default), `add`, `remove` or `toggle`). It is not an undo step and
 does not change the revision. The window shows the selection and agents see what the user
 selected with the `selection` query. Vertex ids are `invalid_input` and unknown ids are
 `not_found`, both with `ids`. Empty `ids` with mode `replace` clears the selection.
 
@@ -103,8 +104,26 @@ pushed face, with nothing else on it inside that area: that cuts an opening (a w
 wall, a hole through a slab). The floor of a recess pushed on to the back opens the same way.
 When a face moves, its vertices are reported as `modified`; the edges and faces using them changed
 shape too.
 
+### Guides
+
+Guides are construction geometry, as drawn by the Tape Measure tool: guide lines (infinite) and
+guide points. Tools snap to them; they never merge with edges, bound no faces and are not
+exported. They belong to the open context they are added in (the root, or a group or component
+definition) and are saved with the model.
+
+| Command | Effect |
+|---|---|
+| `{"type": "add_guide_line", "point": [0, -1, 0], "direction": [1, 0, 0]}` | A guide line through `point` along `direction` (any non-zero length; stored as a unit vector). One undo step. |
+| `{"type": "add_guide_point", "position": [1, 1, 0]}` | A guide point. One undo step. |
+| `{"type": "delete_guides"}` | Erases every guide of the open context as one undo step; with none there it changes nothing (no error, no undo step). |
+
+`erase`, `select`, `move`, `rotate` and `copy` take guide ids like any other (a moved or turned
+guide keeps its id and is reported as `modified`). A guide's `entity` result, and each entry of
+the `guides` list in `entities`, is `{"id": "g9", "kind": "line", "point": [...], "direction":
+[...]}` (`kind` `point` has no `direction`). `stats` counts no guides.
+
 ### Groups and components
 
 Commands and queries work on the open context: the root at first. `{"type": "open_instance",
 "id": "i4"}` opens an instance of the open context for editing, and `{"type": "close_instance"}`
@@ -135,9 +154,10 @@ whose last instance was erased, and what only it placed (one undo step).
 
 ### Files
 
 Models are saved as `.skr` files: a zip archive holding `document.json`, which records `format`
-(`"tenon"`), `schema_version` (1), the camera and the model with every id.
+(`"tenon"`), `schema_version` (2; version 1 files, written before guides, still open), the
+camera and the model with every id.
 
 | Command | Effect |
 |---|---|
 | `{"type": "save", "path": "/abs/house.skr"}` | Writes the file (the old file is replaced only once the new one is complete). Without `path` it saves to the file last opened or saved; a model never saved then gives `invalid_input`. |
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p kernel --test guides && cargo test -p session --test guides && cargo test -p api --test dispatch`
Expected: all pass (5 and 7).

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
Expected: clean; 586 tests pass.

- [ ] **Step 5: Commit**

```bash
git status
git diff --stat
git add crates/kernel/tests/guides.rs crates/session/tests/guides.rs crates/api/tests/dispatch.rs crates/kernel/src/guides.rs crates/kernel/src/ids.rs crates/kernel/src/model.rs crates/kernel/src/data.rs crates/kernel/src/group.rs crates/kernel/src/scene.rs crates/kernel/src/lib.rs crates/session/src/command.rs crates/session/src/query.rs crates/session/src/document.rs crates/session/src/document/groups.rs crates/session/src/file.rs crates/session/src/lib.rs crates/tools/src/erase.rs crates/tools/src/move_tool.rs crates/app/src/info.rs crates/app/src/editor.rs docs/agent-api.md
git commit -m "Guides as model data: guide lines and points, add/delete commands, saved in .skr"
```

### Task 2: Guides in the window

**Files:**
- Create: `crates/render/src/guides.rs`
- Modify:
  - `crates/session/src/camera.rs` (`LINE_REACH`, `near()`, `line_span()`)
  - `crates/tools/src/inference.rs` (`SnapKind::GuidePoint`/`OnGuide`, guide snaps and crossings, `guide_under_where`), `select.rs` (pick and box-select guides, `guide_outline`), `erase.rs`, `move_tool.rs`, `overlay.rs`, `lib.rs`
  - `crates/render/src/scene.rs` (`ModelGeometry.guides`), `renderer.rs` (draws guides per frame), `lib.rs`
  - `crates/app/src/info.rs` (guides in Entity Info), `crates/app/src/editor.rs` (test)
- Test: test-module hunks of `camera.rs`, `inference.rs`, `select.rs`, `render/src/scene.rs`, the new `render/src/guides.rs`, `info.rs`, `editor.rs`; and `crates/render/tests/headless.rs` (`a_guide_line_is_drawn_dashed_across_the_view`, GPU, skips without an adapter)

**Interfaces:**
- Consumes (Task 1): `Guide`, `Model::guides`, `EntityId::Guide`, `Command::AddGuide*`; also `Scenery`, `visible`, `apparent_crossing`, `closest_on_line_to_ray`.
- Produces:
  - `session::camera::LINE_REACH: f64 = 1000.0`; `Camera::near(&self) -> f64`; `Camera::line_span(&self, point: DVec3, dir: DVec3) -> Option<[DVec3; 2]>` (ends in line-parameter order)
  - `tools::SnapKind::{GuidePoint, OnGuide}`
  - `tools::guide_under_where(scenery: &Scenery, camera: &Camera, cursor: DVec2, radius_px: f64, keep: impl Fn(Owner) -> bool) -> Option<(Owner, GuideId, Guide)>`
  - `pub(crate) fn tools::select::guide_outline(ctx: &ToolContext, g: &Guide) -> Vec<[DVec3; 2]>`
  - `render::guides::{GUIDE_COLOR, FADED_GUIDE_COLOR, GuideShape { point, direction: Option<DVec3>, color }, guide_lines(&[GuideShape], &Camera) -> Vec<LineVertex>}`; `ModelGeometry.guides`

- [ ] **Step 1: Write the failing tests**

Most tests are test-module hunks of the Step 3 diffs (and the new `render/src/guides.rs`). Apply those hunks first, along with the headless test below.

Apply to `crates/render/tests/headless.rs`:
```diff
--- a/crates/render/tests/headless.rs
+++ b/crates/render/tests/headless.rs
@@ -379,4 +379,31 @@ fn scene_stays_inside_its_viewport() {
     let face = cam.project(DVec3::new(0.5, 0.5, 0.0)).unwrap();
     assert!(face.x < (W / 2) as f64);
     assert!(grayish(pixel(&px, face.x as u32, face.y as u32)));
 }
+
+#[test]
+fn a_guide_line_is_drawn_dashed_across_the_view() {
+    let c = camera();
+    let mut m = Model::new();
+    // A horizontal guide line across the view through the target, in front of the background.
+    m.add_guide(kernel::Guide::Line {
+        point: c.target,
+        direction: c.right(),
+    })
+    .unwrap();
+    let Some(pixels) = render_scene(&c, &m, &[]) else {
+        return;
+    };
+    // The line runs along the boundary between the two middle rows, so multisampling shares
+    // it between them: a column is part of a dash when either row is clearly darkened.
+    let dark = |x: u32| (H / 2 - 1..=H / 2).any(|y| pixel(&pixels, x, y)[0] < 200);
+    let along: Vec<bool> = (0..W).map(dark).collect();
+    let lit = along.iter().filter(|b| **b).count();
+    // Dashes and gaps of about 8 px: roughly half the row, broken many times.
+    assert!(
+        lit > W as usize / 4 && lit < W as usize * 3 / 4,
+        "{lit} guide pixels"
+    );
+    let breaks = along.windows(2).filter(|w| w[0] != w[1]).count();
+    assert!(breaks > 20, "{breaks} dash ends");
+}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p tools --lib inference`
Expected: compile errors: ``no variant or associated item named `GuidePoint` found for enum `SnapKind` `` and ``cannot find function `guide_under_where` ``.

- [ ] **Step 3: Implement**

Apply to `crates/session/src/camera.rs`:
```diff
--- a/crates/session/src/camera.rs
+++ b/crates/session/src/camera.rs
@@ -10,8 +10,10 @@ pub const ORBIT_RAD_PER_PX: f64 = 0.006;
 pub const ZOOM_STEP: f64 = 0.85;
 pub const MIN_ZOOM_DISTANCE: f64 = 0.01;
 /// Ground hits farther than distance() * this are ignored as anchors.
 pub const MAX_ANCHOR_FACTOR: f64 = 50.0;
+/// How far (in eye-to-target distances) `Camera::line_span` reaches along an infinite line.
+pub const LINE_REACH: f64 = 1000.0;
 
 #[derive(Clone, Copy, Debug, PartialEq)]
 pub struct Ray {
     pub origin: DVec3,
@@ -84,10 +86,40 @@ impl Camera {
 
     /// Reversed-Z perspective with an infinite far plane: depth is 1 at the near plane and
     /// approaches 0 with distance, so nothing far away is ever clipped.
     pub fn proj(&self) -> DMat4 {
-        let near = (self.distance() * 1e-3).max(1e-5);
-        directx::perspective_infinite_reverse(self.fov_y, self.aspect(), near)
+        directx::perspective_infinite_reverse(self.fov_y, self.aspect(), self.near())
+    }
+
+    /// Distance from the eye to the near plane.
+    pub fn near(&self) -> f64 {
+        (self.distance() * 1e-3).max(1e-5)
+    }
+
+    /// The part of the infinite line through `point` along `dir` worth drawing or snapping
+    /// to: in front of the near plane (with a margin), and within `LINE_REACH` eye-to-target
+    /// distances of the eye in depth and along the line. Points beyond look the same as the
+    /// end. None when no part of the line is in front of the camera (or `dir` is zero).
+    pub fn line_span(&self, point: DVec3, dir: DVec3) -> Option<[DVec3; 2]> {
+        let dir = dir.try_normalize()?;
+        let reach = self.distance() * LINE_REACH;
+        let near = self.near() * 2.0;
+        // Along the line, around the point nearest the eye.
+        let foot = (self.eye - point).dot(dir);
+        let (mut lo, mut hi) = (foot - reach, foot + reach);
+        // In depth: near <= depth0 + t * slope <= reach.
+        let depth0 = (point - self.eye).dot(self.forward());
+        let slope = dir.dot(self.forward());
+        if slope.abs() < 1e-12 {
+            if !(near..=reach).contains(&depth0) {
+                return None;
+            }
+        } else {
+            let (a, b) = ((near - depth0) / slope, (reach - depth0) / slope);
+            lo = lo.max(a.min(b));
+            hi = hi.min(a.max(b));
+        }
+        (lo < hi).then(|| [point + dir * lo, point + dir * hi])
     }
 
     pub fn view_proj_f32(&self) -> Mat4 {
         (self.proj() * self.view()).as_mat4()
@@ -195,8 +227,27 @@ mod tests {
         let p = c.project(c.target).unwrap();
         assert!(p.abs_diff_eq(DVec2::new(400.0, 300.0), 1e-6));
     }
 
+    #[test]
+    fn line_spans_stay_in_front_of_the_camera() {
+        let c = cam();
+        // A line through the target across the view: both ends project, far to either side.
+        let [a, b] = c.line_span(c.target, c.right()).unwrap();
+        assert!(c.project(a).is_some() && c.project(b).is_some());
+        assert!(a.distance(b) > c.distance() * 100.0);
+        // A line through the eye along the view (given pointing backwards) starts just past
+        // the near plane.
+        let [far, near] = c.line_span(c.eye, -c.forward()).unwrap();
+        let depth = |p: DVec3| (p - c.eye).dot(c.forward());
+        assert!(depth(near) > c.near() && depth(near) < c.near() * 3.0);
+        assert!((depth(far) - c.distance() * LINE_REACH).abs() < 1e-6);
+        // A line wholly behind the camera, across the view, has no span.
+        let behind = c.eye - c.forward();
+        assert!(c.line_span(behind, c.right()).is_none());
+        assert!(c.line_span(c.target, DVec3::ZERO).is_none());
+    }
+
     #[test]
     fn ray_through_projected_point_hits_point() {
         let c = cam();
         let world = DVec3::new(3.0, -2.0, 1.5);
```

Create `crates/render/src/guides.rs`:
```rust
//! Guide lines and guide points, drawn anew each frame for the camera: a guide line is
//! infinite, so only the part in view is drawn, dashed; a guide point is a small cross of
//! constant size on screen.

use glam::{DVec2, DVec3};
use session::{Camera, Ray};

use crate::grid::LineVertex;

pub const GUIDE_COLOR: [f32; 4] = [0.25, 0.25, 0.32, 1.0];
/// Guides outside the open instance, while one is open.
pub const FADED_GUIDE_COLOR: [f32; 4] = [0.68, 0.68, 0.72, 1.0];

/// Length of a dash and of the gap after it, in physical pixels.
const DASH_PX: f64 = 8.0;
/// Half the size of a guide point's cross, in physical pixels.
const CROSS_PX: f64 = 5.0;
/// How far beyond the viewport (physical pixels) dashes are still drawn.
const MARGIN_PX: f64 = 16.0;

/// One guide placed in root coordinates: a line through `point` along the unit `direction`,
/// or, without a direction, a point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GuideShape {
    pub point: DVec3,
    pub direction: Option<DVec3>,
    pub color: [f32; 4],
}

/// Line pairs drawing `guides` as `camera` sees them.
pub fn guide_lines(guides: &[GuideShape], camera: &Camera) -> Vec<LineVertex> {
    let mut out = Vec::new();
    let mut push = |a: DVec3, b: DVec3, color: [f32; 4]| {
        for p in [a, b] {
            out.push(LineVertex {
                pos: p.as_vec3().to_array(),
                color,
            });
        }
    };
    for g in guides {
        match g.direction {
            Some(dir) => {
                for [a, b] in dashes(camera, g.point, dir) {
                    push(a, b, g.color);
                }
            }
            None => {
                let depth = (g.point - camera.eye).dot(camera.forward());
                if depth <= camera.near() {
                    continue;
                }
                let size = CROSS_PX * 2.0 * depth * (camera.fov_y * 0.5).tan() / camera.viewport.y;
                let (r, u) = (camera.right() * size, camera.up() * size);
                push(g.point - r - u, g.point + r + u, g.color);
                push(g.point - r + u, g.point + r - u, g.color);
            }
        }
    }
    out
}

/// Dashes along the part of the line through `point` along `dir` that is on screen, each
/// `DASH_PX` long on screen, as world segments on the line.
fn dashes(camera: &Camera, point: DVec3, dir: DVec3) -> Vec<[DVec3; 2]> {
    let Some([a, b]) = camera.line_span(point, dir) else {
        return Vec::new();
    };
    let (Some(sa), Some(sb)) = (camera.project(a), camera.project(b)) else {
        return Vec::new();
    };
    let lo = DVec2::splat(-MARGIN_PX);
    let hi = camera.viewport + MARGIN_PX;
    let Some((sa, sb)) = clip(sa, sb, lo, hi) else {
        return Vec::new();
    };
    let n = (sa.distance(sb) / DASH_PX).ceil().max(1.0) as usize;
    let on_line = |s: DVec2| closest_on_line(point, dir, &camera.ray(s));
    (0..n)
        .step_by(2)
        .map(|k| {
            let at = |k: usize| sa.lerp(sb, k.min(n) as f64 / n as f64);
            [on_line(at(k)), on_line(at(k + 1))]
        })
        .collect()
}

/// The part of the 2D segment p-q inside the box `lo`..`hi` (Liang-Barsky), if any.
fn clip(p: DVec2, q: DVec2, lo: DVec2, hi: DVec2) -> Option<(DVec2, DVec2)> {
    let d = q - p;
    let (mut t0, mut t1) = (0.0f64, 1.0f64);
    for (dp, from_lo, to_hi) in [(d.x, p.x - lo.x, hi.x - p.x), (d.y, p.y - lo.y, hi.y - p.y)] {
        for (num, den) in [(from_lo, -dp), (to_hi, dp)] {
            if den == 0.0 {
                if num < 0.0 {
                    return None;
                }
            } else {
                let t = num / den;
                if den < 0.0 {
                    t0 = t0.max(t);
                } else {
                    t1 = t1.min(t);
                }
            }
        }
    }
    (t0 < t1).then(|| (p + d * t0, p + d * t1))
}

/// The point on the line `origin + t * dir` closest to the ray.
fn closest_on_line(origin: DVec3, dir: DVec3, ray: &Ray) -> DVec3 {
    let w0 = origin - ray.origin;
    let (a, b, c) = (dir.dot(dir), dir.dot(ray.dir), ray.dir.dot(ray.dir));
    let (d, e) = (dir.dot(w0), ray.dir.dot(w0));
    let denom = a * c - b * b;
    let t = if denom.abs() < 1e-12 * a * c {
        0.0
    } else {
        (b * e - c * d) / denom
    };
    origin + dir * t
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cam() -> Camera {
        Camera::new(DVec2::new(800.0, 600.0))
    }

    #[test]
    fn a_guide_line_is_dashed_across_the_view_and_stays_on_its_line() {
        let c = cam();
        let line = GuideShape {
            point: DVec3::new(0.0, 1.0, 0.0),
            direction: Some(DVec3::X),
            color: GUIDE_COLOR,
        };
        let v = guide_lines(&[line], &c);
        assert!(v.len() > 40, "{} vertices", v.len());
        for p in &v {
            assert!((p.pos[1] - 1.0).abs() < 1e-5 && p.pos[2].abs() < 1e-5);
        }
        // Each dash is about DASH_PX long on screen.
        let s = |k: usize| c.project(DVec3::from(v[k].pos.map(f64::from))).unwrap();
        let len = s(0).distance(s(1));
        assert!((len - DASH_PX).abs() < 0.5, "{len}");
    }

    #[test]
    fn guide_points_are_crosses_and_guides_out_of_view_are_skipped() {
        let c = cam();
        let point = GuideShape {
            point: DVec3::new(1.0, 1.0, 0.0),
            direction: None,
            color: GUIDE_COLOR,
        };
        let v = guide_lines(&[point], &c);
        assert_eq!(v.len(), 4);
        let s = |k: usize| c.project(DVec3::from(v[k].pos.map(f64::from))).unwrap();
        let diagonal = 2.0 * CROSS_PX * 2f64.sqrt();
        assert!((s(0).distance(s(1)) - diagonal).abs() < 0.5);
        let behind = GuideShape {
            point: c.eye - c.forward() * 5.0,
            ..point
        };
        assert!(guide_lines(&[behind], &c).is_empty());
        let off_screen = GuideShape {
            point: c.target + c.up() * c.distance() * 10.0,
            direction: Some(c.right()),
            color: GUIDE_COLOR,
        };
        assert!(guide_lines(&[off_screen], &c).is_empty());
    }
}
```

Apply to `crates/render/src/scene.rs`:
```diff
--- a/crates/render/src/scene.rs
+++ b/crates/render/src/scene.rs
@@ -3,11 +3,12 @@
 use std::collections::BTreeSet;
 
 use bytemuck::{Pod, Zeroable};
 use glam::{DAffine3, DVec3};
-use kernel::{EntityId, InstanceId, MeshId, Scene};
+use kernel::{EntityId, Guide, InstanceId, MeshId, Scene};
 
 use crate::grid::LineVertex;
+use crate::guides::{FADED_GUIDE_COLOR, GUIDE_COLOR, GuideShape};
 
 #[repr(C)]
 #[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
 pub struct FaceVertex {
@@ -24,20 +25,22 @@ pub const SELECTED_EDGE_COLOR: [f32; 4] = [0.10, 0.35, 0.95, 1.0];
 pub const FADED_EDGE_COLOR: [f32; 4] = [0.62, 0.62, 0.62, 1.0];
 /// The box around the open instance.
 pub const CONTEXT_BOX_COLOR: [f32; 4] = [0.45, 0.45, 0.45, 1.0];
 
-/// Triangles for every face and line pairs for every edge.
+/// Triangles for every face, line pairs for every edge, and the guides (drawn per frame, see
+/// `guides::guide_lines`).
 #[derive(Debug, Default, PartialEq)]
 pub struct ModelGeometry {
     pub faces: Vec<FaceVertex>,
     pub edges: Vec<LineVertex>,
+    pub guides: Vec<GuideShape>,
 }
 
 /// Geometry for the whole scene in root coordinates, every instance placed by its transform.
 /// `context` is the path of open instances (see `session::Document::context`) and `selection`
 /// holds ids of the open context: selected entities there are highlighted, and a selected
-/// instance highlights everything it places. While an instance is open, edges outside it are
-/// faded and its bounding box is outlined.
+/// instance highlights everything it places. While an instance is open, edges and guides
+/// outside it are faded and its bounding box is outlined.
 pub fn scene_geometry(
     scene: &Scene,
     context: &[InstanceId],
     selection: &BTreeSet<EntityId>,
@@ -147,8 +150,26 @@ impl Walk<'_> {
                     });
                 }
             }
         }
+        for (gid, guide) in model.guides() {
+            let color = if selected(EntityId::Guide(gid)) {
+                SELECTED_EDGE_COLOR
+            } else if inside {
+                GUIDE_COLOR
+            } else {
+                FADED_GUIDE_COLOR
+            };
+            let (point, direction) = match guide.transformed(t) {
+                Guide::Line { point, direction } => (point, Some(direction)),
+                Guide::Point { position } => (position, None),
+            };
+            self.g.guides.push(GuideShape {
+                point,
+                direction,
+                color,
+            });
+        }
         for (iid, instance) in model.instances() {
             let lit = selected(EntityId::Instance(iid));
             self.path.push(iid);
             self.mesh(
@@ -278,8 +299,55 @@ mod tests {
             .any(|v| (v.pos[2] - sin).abs() < 1e-6);
         assert!(lifted_box, "the open instance's box turns with it");
     }
 
+    #[test]
+    fn guides_are_placed_and_lit_when_selected() {
+        let (mut s, first, _) = two_squares();
+        let d = s.root().instance(first).unwrap().definition;
+        let inner = s
+            .edit(MeshId::Definition(d), |m| {
+                m.add_guide(Guide::Point {
+                    position: DVec3::new(0.5, 0.5, 0.0),
+                })
+            })
+            .unwrap()
+            .summary()
+            .created[0];
+        let root = s
+            .edit(MeshId::Root, |m| {
+                m.add_guide(Guide::Line {
+                    point: DVec3::ZERO,
+                    direction: DVec3::Y,
+                })
+            })
+            .unwrap()
+            .summary()
+            .created[0];
+        let g = scene_geometry(&s, &[], &BTreeSet::from([root]));
+        // The component is placed twice, so its guide point is drawn twice.
+        assert_eq!(g.guides.len(), 3);
+        let line = g.guides.iter().find(|s| s.direction.is_some()).unwrap();
+        assert_eq!(line.color, SELECTED_EDGE_COLOR);
+        let mut points: Vec<f64> = g
+            .guides
+            .iter()
+            .filter(|s| s.direction.is_none())
+            .map(|s| s.point.x)
+            .collect();
+        points.sort_by(f64::total_cmp);
+        assert_eq!(points, vec![0.5, 3.5]);
+        // Inside the first square, the root's guide line is faded.
+        let open = scene_geometry(&s, &[first], &BTreeSet::from([inner]));
+        let faded = open.guides.iter().filter(|s| s.color == FADED_GUIDE_COLOR);
+        assert_eq!(
+            faded.count(),
+            2,
+            "the root's line and the other square's point"
+        );
+        assert!(open.guides.iter().any(|s| s.color == SELECTED_EDGE_COLOR));
+    }
+
     #[test]
     fn a_selected_instance_lights_up_what_it_places() {
         let (s, first, _) = two_squares();
         let selection = BTreeSet::from([EntityId::Instance(first)]);
```

Apply to `crates/render/src/renderer.rs`:
```diff
--- a/crates/render/src/renderer.rs
+++ b/crates/render/src/renderer.rs
@@ -4,8 +4,9 @@ use kernel::{EntityId, InstanceId, Scene};
 use session::Camera;
 use wgpu::util::DeviceExt;
 
 use crate::grid::{BACKGROUND, LineVertex, flat_ground_lines, vertical_axis_lines};
+use crate::guides::{GuideShape, guide_lines};
 use crate::scene::{FaceVertex, scene_geometry};
 
 pub const SAMPLE_COUNT: u32 = 4;
 pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
@@ -71,8 +72,10 @@ pub struct Renderer {
     flat_ground: Mesh,
     vertical_axis: Mesh,
     faces: Option<Mesh>,
     edges: Option<Mesh>,
+    /// Guides in root coordinates; their lines depend on the camera, so they are built per frame.
+    guides: Vec<GuideShape>,
     targets: Option<Targets>,
 }
 
 /// How a pipeline uses the depth buffer.
@@ -249,8 +252,9 @@ impl Renderer {
             flat_ground,
             vertical_axis,
             faces: None,
             edges: None,
+            guides: Vec::new(),
             targets: None,
         }
     }
 
@@ -265,8 +269,9 @@ impl Renderer {
     ) {
         let g = scene_geometry(scene, context, selection);
         self.faces = mesh(&self.device, "faces", &g.faces);
         self.edges = mesh(&self.device, "edges", &g.edges);
+        self.guides = g.guides;
     }
 
     /// Draws grid, axes, model and `overlay` (line pairs drawn on top of everything) over the
     /// whole target.
@@ -302,8 +307,9 @@ impl Renderer {
         };
         self.queue
             .write_buffer(&self.uniforms, 0, bytemuck::bytes_of(&uniforms));
         let overlay = mesh(&self.device, "overlay", overlay);
+        let guides = mesh(&self.device, "guides", &guide_lines(&self.guides, camera));
 
         let [r, g, b, a] = BACKGROUND.map(f64::from);
         let mut encoder = self
             .device
@@ -346,8 +352,9 @@ impl Renderer {
                     (&self.flat_ground_pipeline, Some(&self.flat_ground)),
                     (&self.ground_pipeline, Some(&self.vertical_axis)),
                     (&self.face_pipeline, self.faces.as_ref()),
                     (&self.edge_pipeline, self.edges.as_ref()),
+                    (&self.edge_pipeline, guides.as_ref()),
                     (&self.overlay_pipeline, overlay.as_ref()),
                 ];
                 for (pipeline, mesh) in layers {
                     if let Some(mesh) = mesh {
```

Apply to `crates/render/src/lib.rs`:
```diff
--- a/crates/render/src/lib.rs
+++ b/crates/render/src/lib.rs
@@ -1,5 +1,6 @@
 pub mod grid;
+pub mod guides;
 pub mod renderer;
 pub mod scene;
 pub mod snapshot;
 
```

Apply to `crates/tools/src/inference.rs`:
```diff
--- a/crates/tools/src/inference.rs
+++ b/crates/tools/src/inference.rs
@@ -1,8 +1,8 @@
 //! Snapping the cursor to meaningful 3D points.
 
 use glam::{DVec2, DVec3};
-use kernel::{EdgeId, FaceId, Model, Shape, VertexId};
+use kernel::{EdgeId, FaceId, Guide, GuideId, Model, Shape, VertexId};
 use session::{Camera, Ray};
 
 use crate::scenery::{Owner, Scenery};
 
@@ -30,11 +30,16 @@ impl Axis {
 pub enum SnapKind {
     Endpoint,
     Midpoint,
     Origin,
-    /// Where two edges cross on screen without meeting in space: the point on the nearer one.
+    /// A guide point.
+    GuidePoint,
+    /// Where two edges or guide lines cross on screen (or in space): the point on the nearer
+    /// one.
     Intersection,
     OnEdge,
+    /// On a guide line.
+    OnGuide,
     /// Along a red/green/blue axis through the point the tool is drawing from.
     Axis(Axis),
     /// Parallel to the reference edge, through the point the tool is drawing from.
     Parallel,
@@ -107,10 +112,10 @@ pub fn infer_with(
 /// Finds the 3D point under `cursor` (physical pixels) among everything `scenery` holds, in
 /// the open context's coordinates (`camera` must be in them too). Snap candidates count only
 /// within `radius_px` of the cursor.
 ///
-/// Ranking: endpoint, midpoint and origin (nearest on screen wins) > intersection > on-edge >
-/// directions from the anchor (axes, parallel or perpendicular to the reference edge; nearest
+/// Ranking: endpoint, midpoint, origin and guide point (nearest on screen wins) > intersection
+/// (of edges and guide lines) > on-edge > on-guide-line > directions from the anchor (axes, parallel or perpendicular to the reference edge; nearest
 /// wins) > on-face > ground > free. Points and edges hidden behind a face are skipped. With a
 /// lock, the result is always on the locked line.
 pub fn infer_in(
     scenery: &Scenery,
@@ -186,11 +191,11 @@ pub fn infer_in(
         },
     }
 }
 
-/// The visible point snap under the cursor, if any: endpoint, midpoint or origin (nearest on
-/// screen), else the nearest apparent intersection of two edges both visible there, else the
-/// nearest point on an edge.
+/// The visible point snap under the cursor, if any: endpoint, midpoint, origin or guide point
+/// (nearest on screen), else the nearest apparent intersection of two edges or guide lines
+/// both visible there, else the nearest point on an edge, else on a guide line.
 fn snap_point(
     scenery: &Scenery,
     camera: &Camera,
     cursor: DVec2,
@@ -219,15 +224,46 @@ fn snap_point(
     for &(_, _, a, b) in &near.segments {
         offer((a + b) * 0.5, SnapKind::Midpoint);
     }
     offer(DVec3::ZERO, SnapKind::Origin);
+    let guides = placed_guides(scenery);
+    for &(_, _, g) in &guides {
+        if let Guide::Point { position } = g {
+            offer(position, SnapKind::GuidePoint);
+        }
+    }
     if let Some((_, p, kind)) = best {
         return Some((p, kind));
     }
 
+    // Guide lines passing near the cursor, as the part of them in view.
+    let guide_lines: Vec<(DVec3, DVec3)> = guides
+        .iter()
+        .filter_map(|&(_, _, g)| match g {
+            Guide::Line { point, direction } => Some((point, direction)),
+            Guide::Point { .. } => None,
+        })
+        .filter(|&(point, dir)| {
+            let p = closest_on_line_to_ray(point, dir, &ray, false);
+            camera
+                .project(p)
+                .is_some_and(|s| s.distance(cursor) <= radius_px)
+        })
+        .collect();
+    let lines: Vec<(DVec3, DVec3)> = near
+        .segments
+        .iter()
+        .map(|&(_, _, a, b)| (a, b))
+        .chain(
+            guide_lines
+                .iter()
+                .filter_map(|&(point, dir)| camera.line_span(point, dir))
+                .map(|[a, b]| (a, b)),
+        )
+        .collect();
     let mut crossing: Option<(f64, DVec3)> = None;
-    for (i, &(_, _, a, b)) in near.segments.iter().enumerate() {
-        for &(_, _, c, d) in &near.segments[i + 1..] {
+    for (i, &(a, b)) in lines.iter().enumerate() {
+        for &(c, d) in &lines[i + 1..] {
             if let Some((p, far)) = apparent_crossing(camera, (a, b), (c, d))
                 && visible(scenery, camera.eye, far)
                 && let Some(dist) = within(p)
                 && crossing.is_none_or(|(bd, _)| dist < bd)
@@ -248,9 +284,70 @@ fn snap_point(
         {
             on_edge = Some((d, p));
         }
     }
-    on_edge.map(|(_, p)| (p, SnapKind::OnEdge))
+    if let Some((_, p)) = on_edge {
+        return Some((p, SnapKind::OnEdge));
+    }
+
+    let mut on_guide: Option<(f64, DVec3)> = None;
+    for &(point, dir) in &guide_lines {
+        let p = closest_on_line_to_ray(point, dir, &ray, false);
+        if let Some(d) = within(p)
+            && on_guide.is_none_or(|(bd, _)| d < bd)
+        {
+            on_guide = Some((d, p));
+        }
+    }
+    on_guide.map(|(_, p)| (p, SnapKind::OnGuide))
+}
+
+/// Every guide of every part, placed in the open context's coordinates, with the index of the
+/// part it comes from, in part and id order. Guides are few, so they are scanned, not indexed.
+fn placed_guides(scenery: &Scenery) -> Vec<(usize, GuideId, Guide)> {
+    scenery
+        .parts
+        .iter()
+        .enumerate()
+        .flat_map(|(k, part)| {
+            part.model
+                .guides()
+                .map(move |(id, g)| (k, id, g.transformed(part.place)))
+        })
+        .collect()
+}
+
+/// The visible guide nearest the cursor on screen whose owner passes `keep`, if one lies
+/// within `radius_px`: whose it is, the guide (an id in its own mesh) and the guide placed in
+/// the open context's coordinates.
+pub fn guide_under_where(
+    scenery: &Scenery,
+    camera: &Camera,
+    cursor: DVec2,
+    radius_px: f64,
+    keep: impl Fn(Owner) -> bool,
+) -> Option<(Owner, GuideId, Guide)> {
+    let ray = camera.ray(cursor);
+    let mut best: Option<(f64, (Owner, GuideId, Guide))> = None;
+    for (part, id, g) in placed_guides(scenery) {
+        let owner = scenery.parts[part].owner;
+        if !keep(owner) {
+            continue;
+        }
+        let p = match g {
+            Guide::Point { position } => position,
+            Guide::Line { point, direction } => {
+                closest_on_line_to_ray(point, direction, &ray, false)
+            }
+        };
+        let Some(d) = camera.project(p).map(|s| s.distance(cursor)) else {
+            continue;
+        };
+        if d <= radius_px && best.is_none_or(|(bd, _)| d < bd) && visible(scenery, camera.eye, p) {
+            best = Some((d, (owner, id, g)));
+        }
+    }
+    best.map(|(_, hit)| hit)
 }
 
 /// Where segments a-b and c-d cross on screen away from their ends: the points under the
 /// crossing on the nearer and the farther segment, nearer first. Edges that meet in space share
@@ -799,5 +896,83 @@ mod tests {
         // An endpoint under the cursor sets the height.
         let snapped = infer_with(&m, &c, px(&c, DVec3::new(3.0, 3.0, 2.0)), &g, R);
         assert_eq!(snapped.point, DVec3::new(1.0, 1.0, 2.0));
     }
+
+    fn with_guides() -> Model {
+        let mut m = Model::new();
+        m.add_guide(Guide::Point {
+            position: DVec3::new(2.0, 1.0, 0.0),
+        })
+        .unwrap();
+        m.add_guide(Guide::Line {
+            point: DVec3::new(0.0, 3.0, 0.0),
+            direction: DVec3::X,
+        })
+        .unwrap();
+        m.add_guide(Guide::Line {
+            point: DVec3::new(5.0, 0.0, 0.0),
+            direction: DVec3::Y,
+        })
+        .unwrap();
+        m
+    }
+
+    #[test]
+    fn snaps_to_guide_points_and_onto_guide_lines() {
+        let (c, m) = (cam(), with_guides());
+        let point = DVec3::new(2.0, 1.0, 0.0);
+        let hit = infer(&m, &c, px(&c, point) + DVec2::new(3.0, -2.0), None, R);
+        assert_eq!((hit.point, hit.kind), (point, SnapKind::GuidePoint));
+        let hit = infer(
+            &m,
+            &c,
+            px(&c, DVec3::new(2.6, 3.0, 0.0)) + DVec2::new(0.0, 3.0),
+            None,
+            R,
+        );
+        assert_eq!(hit.kind, SnapKind::OnGuide);
+        assert!((hit.point.y - 3.0).abs() < 1e-9 && hit.point.z.abs() < 1e-9);
+        assert!((hit.point.x - 2.6).abs() < 0.1);
+    }
+
+    #[test]
+    fn guide_lines_cross_each_other_and_edges() {
+        let (c, mut m) = (cam(), with_guides());
+        let both = DVec3::new(5.0, 3.0, 0.0);
+        let hit = infer(&m, &c, px(&c, both) + DVec2::new(2.0, 1.0), None, R);
+        assert_eq!(hit.kind, SnapKind::Intersection);
+        assert!(hit.point.distance(both) < 1e-6, "{:?}", hit.point);
+        m.add_edge(DVec3::new(1.0, -3.0, 0.0), DVec3::new(1.0, 7.0, 0.0))
+            .unwrap();
+        let cross = DVec3::new(1.0, 3.0, 0.0);
+        let hit = infer(&m, &c, px(&c, cross) + DVec2::new(-1.0, 2.0), None, R);
+        assert_eq!(hit.kind, SnapKind::Intersection);
+        assert!(hit.point.distance(cross) < 1e-6, "{:?}", hit.point);
+        // Along the edge away from the guide, the edge wins over nothing.
+        let hit = infer(&m, &c, px(&c, DVec3::new(1.0, 0.5, 0.0)), None, R);
+        assert_eq!(hit.kind, SnapKind::OnEdge);
+    }
+
+    #[test]
+    fn finds_the_guide_under_the_cursor() {
+        let (c, m) = (cam(), with_guides());
+        let scenery = Scenery::single(&m);
+        let ids: Vec<GuideId> = m.guides().map(|(id, _)| id).collect();
+        let under = |p: DVec3| {
+            guide_under_where(&scenery, &c, px(&c, p) + DVec2::new(2.0, 2.0), R, |_| true)
+                .map(|(_, id, _)| id)
+        };
+        assert_eq!(under(DVec3::new(2.0, 1.0, 0.0)), Some(ids[0]));
+        assert_eq!(under(DVec3::new(-2.0, 3.0, 0.0)), Some(ids[1]));
+        assert_eq!(under(DVec3::new(5.0, -4.0, 0.0)), Some(ids[2]));
+        assert_eq!(under(DVec3::new(-3.0, -3.0, 0.0)), None);
+        // Guides behind a face are not found.
+        let mut hidden = with_guides();
+        let lid = [(-9.0, -9.0), (9.0, -9.0), (9.0, 9.0), (-9.0, 9.0)]
+            .map(|(x, y)| DVec3::new(x, y, 1.0));
+        hidden.add_polygon(&lid).unwrap();
+        let scenery = Scenery::single(&hidden);
+        let p = px(&c, DVec3::new(2.0, 1.0, 0.0));
+        assert!(guide_under_where(&scenery, &c, p, R, |_| true).is_none());
+    }
 }
```

Apply to `crates/tools/src/select.rs`:
```diff
--- a/crates/tools/src/select.rs
+++ b/crates/tools/src/select.rs
@@ -1,16 +1,16 @@
-//! The Select tool: click an edge, face or group to select it, or drag a box. Dragging right
+//! The Select tool: click an edge, face, guide or group to select it, or drag a box. Dragging right
 //! selects what lies entirely inside the box; dragging left also selects what the box touches.
 //! With Shift held, clicked or boxed entities are toggled instead of replacing the selection.
 //! Double-clicking a group opens it for editing; double-clicking outside it, or Escape, closes
 //! it.
 
 use glam::{DVec2, DVec3};
 use kernel::geom::point_in_polygon;
-use kernel::{EntityId, Face, Model};
+use kernel::{EntityId, Face, Guide, Model};
 use session::{Command, SelectMode};
 
-use crate::inference::{edge_under_where, face_under_in};
+use crate::inference::{edge_under_where, face_under_in, guide_under_where};
 use crate::scenery::Owner;
 use crate::tool::{Tool, ToolContext, ToolEvent, ToolOutput};
 
 #[derive(Clone, Debug, Default)]
@@ -25,19 +25,25 @@ impl SelectTool {
         Self::default()
     }
 }
 
-/// What a click picks: the open context's edge under the cursor, else an instance's edge
-/// (which picks the instance), else the face under it (the open context's face, or the
-/// instance). Geometry outside the open instance hides what is behind it but is not picked,
+/// What a click picks: the open context's edge under the cursor, else its guide, else an
+/// instance's edge (which picks the instance), else the face under it (the open context's
+/// face, or the instance). Geometry outside the open instance hides what is behind it but is not picked,
 /// and its edges do not outrank the open context's even when they lie on top of them.
 pub(crate) fn pick(ctx: &ToolContext) -> Option<EntityId> {
     let edge_of = |keep: fn(Owner) -> bool| {
         edge_under_where(ctx.scenery, ctx.camera, ctx.cursor, ctx.pick_radius, keep)
     };
     if let Some((_, e, _, _)) = edge_of(|o| o == Owner::Open) {
         return Some(EntityId::Edge(e));
     }
+    let open = |o: Owner| o == Owner::Open;
+    if let Some((_, g, _)) =
+        guide_under_where(ctx.scenery, ctx.camera, ctx.cursor, ctx.pick_radius, open)
+    {
+        return Some(EntityId::Guide(g));
+    }
     if let Some((Owner::Instance(i), ..)) = edge_of(|o| matches!(o, Owner::Instance(_))) {
         return Some(EntityId::Instance(i));
     }
     match face_under_in(ctx.scenery, ctx.camera, ctx.cursor)? {
@@ -73,11 +79,12 @@ pub(crate) fn box_outline(ctx: &ToolContext, id: EntityId) -> Vec<[DVec3; 2]> {
         .map(|(a, b)| [c[a], c[b]])
         .collect()
 }
 
-/// Edges, faces and instances selected by the screen box from `a` to `b`: inside it entirely
-/// when dragging right (`b.x >= a.x`), touching it when dragging left. Entities with a point
-/// behind the camera are skipped.
+/// Edges, faces, instances and guides selected by the screen box from `a` to `b`: inside it
+/// entirely when dragging right (`b.x >= a.x`), touching it when dragging left. Entities with
+/// a point behind the camera are skipped. Guide lines are never inside a box; a box dragged
+/// left selects those crossing it.
 fn boxed(ctx: &ToolContext, a: DVec2, b: DVec2) -> Vec<EntityId> {
     let (lo, hi) = (a.min(b), a.max(b));
     let window = b.x >= a.x;
     let m = ctx.model;
@@ -145,8 +152,24 @@ fn boxed(ctx: &ToolContext, a: DVec2, b: DVec2) -> Vec<EntityId> {
         if hit {
             ids.push(id);
         }
     }
+    for (id, g) in m.guides() {
+        let hit = match *g {
+            Guide::Point { position } => ctx.camera.project(position).is_some_and(inside),
+            Guide::Line { point, direction } => {
+                !window
+                    && ctx
+                        .camera
+                        .line_span(point, direction)
+                        .and_then(|[a, b]| Some((ctx.camera.project(a)?, ctx.camera.project(b)?)))
+                        .is_some_and(|(p, q)| touches(p, q))
+            }
+        };
+        if hit {
+            ids.push(EntityId::Guide(id));
+        }
+    }
     for (id, f) in m.faces() {
         let all = face_loops(f).all(|l| screen(l).is_some_and(|pts| !pts.is_empty()));
         let hit = if window {
             all && face_loops(f).all(|l| screen(l).is_some_and(|pts| chosen(true, &pts)))
@@ -262,8 +285,31 @@ impl Tool for SelectTool {
         self.press = None;
     }
 }
 
+/// Segments drawing a guide of the open context: the part of a guide line in view, or an X
+/// as wide as the pick radius on screen around a guide point.
+pub(crate) fn guide_outline(ctx: &ToolContext, g: &Guide) -> Vec<[DVec3; 2]> {
+    match *g {
+        Guide::Line { point, direction } => {
+            ctx.camera.line_span(point, direction).into_iter().collect()
+        }
+        Guide::Point { position } => {
+            let c = ctx.camera;
+            let depth = (position - c.eye).dot(c.forward());
+            if depth <= c.near() {
+                return Vec::new();
+            }
+            let size = ctx.pick_radius * 2.0 * depth * (c.fov_y * 0.5).tan() / c.viewport.y;
+            let (r, u) = (c.right() * size, c.up() * size);
+            vec![
+                [position - r - u, position + r + u],
+                [position - r + u, position + r - u],
+            ]
+        }
+    }
+}
+
 /// Positions of the vertices of a face's loops, for outlines.
 pub(crate) fn face_outline(model: &Model, f: &Face) -> Vec<[DVec3; 2]> {
     let mut out = Vec::new();
     for l in face_loops(f) {
@@ -432,8 +478,48 @@ mod tests {
         let out = r.drag(a, b);
         assert_eq!(out.commands, select(&["f8"], SelectMode::Replace));
     }
 
+    #[test]
+    fn guides_are_clicked_after_edges_and_boxed() {
+        let mut r = Rig::new();
+        r.model
+            .add_guide(Guide::Point {
+                position: DVec3::new(3.0, 1.0, 0.0),
+            })
+            .unwrap();
+        r.model
+            .add_guide(Guide::Line {
+                point: DVec3::new(0.0, 3.0, 0.0),
+                direction: DVec3::X,
+            })
+            .unwrap();
+        let point = r.click(DVec3::new(3.0, 1.0, 0.0));
+        assert_eq!(point.commands, select(&["g9"], SelectMode::Replace));
+        let line = r.click(DVec3::new(-1.0, 3.0, 0.0));
+        assert_eq!(line.commands, select(&["g10"], SelectMode::Replace));
+        // Where the guide line runs over the square's edge, the edge wins.
+        r.model
+            .add_guide(Guide::Line {
+                point: DVec3::ZERO,
+                direction: DVec3::X,
+            })
+            .unwrap();
+        let edge = r.click(DVec3::new(1.0, 0.0, 0.0));
+        assert_eq!(edge.commands, select(&["e2"], SelectMode::Replace));
+
+        // A window around the guide point holds it; one across the guide line does not hold
+        // the line, but the same box dragged left selects it.
+        let p = r.px(DVec3::new(3.0, 1.0, 0.0));
+        let window = r.drag(p - 5.0, p + 5.0);
+        assert_eq!(window.commands, select(&["g9"], SelectMode::Replace));
+        let q = r.px(DVec3::new(-1.0, 3.0, 0.0));
+        let (a, b) = (q - 6.0, q + 6.0);
+        assert_eq!(r.drag(a, b).commands, select(&[], SelectMode::Replace));
+        let crossing = r.drag(DVec2::new(b.x, a.y), DVec2::new(a.x, b.y));
+        assert_eq!(crossing.commands, select(&["g10"], SelectMode::Replace));
+    }
+
     #[test]
     fn dragging_draws_a_box_until_release() {
         let mut r = Rig::new();
         r.cursor = DVec2::new(100.0, 100.0);
```

Apply to `crates/tools/src/erase.rs`:
```diff
--- a/crates/tools/src/erase.rs
+++ b/crates/tools/src/erase.rs
@@ -1,12 +1,12 @@
-//! The Erase tool: click an edge, face or group to erase it. What is under the cursor is
+//! The Erase tool: click an edge, face, guide or group to erase it. What is under the cursor is
 //! outlined.
 
 use glam::DVec3;
 use kernel::EntityId;
 use session::Command;
 
-use crate::select::{box_outline, face_outline, pick};
+use crate::select::{box_outline, face_outline, guide_outline, pick};
 use crate::tool::{Tool, ToolContext, ToolEvent, ToolOutput};
 
 #[derive(Clone, Debug, Default)]
 pub struct EraseTool {
@@ -29,9 +29,13 @@ fn outline(ctx: &ToolContext, id: EntityId) -> Vec<[DVec3; 2]> {
             .into_iter()
             .collect(),
         EntityId::Face(f) => m.face(f).map(|f| face_outline(m, f)).unwrap_or_default(),
         EntityId::Instance(_) => box_outline(ctx, id),
-        EntityId::Vertex(_) | EntityId::Guide(_) => Vec::new(),
+        EntityId::Guide(g) => m
+            .guide(g)
+            .map(|g| guide_outline(ctx, g))
+            .unwrap_or_default(),
+        EntityId::Vertex(_) => Vec::new(),
     }
 }
 
 impl Tool for EraseTool {
@@ -39,9 +43,9 @@ impl Tool for EraseTool {
         "Erase"
     }
 
     fn prompt(&self) -> &'static str {
-        "Click an edge, face or group to erase it"
+        "Click an edge, face, guide or group to erase it"
     }
 
     fn measurement(&self) -> (&'static str, Option<String>) {
         ("", None)
```

Apply to `crates/tools/src/move_tool.rs`:
```diff
--- a/crates/tools/src/move_tool.rs
+++ b/crates/tools/src/move_tool.rs
@@ -7,9 +7,9 @@ use glam::DVec3;
 use kernel::EntityId;
 use kernel::geom::LENGTH_TOL;
 use session::{Command, MAX_COPIES};
 
-use crate::select::{box_outline, face_outline, pick};
+use crate::select::{box_outline, face_outline, guide_outline, pick};
 use crate::tool::{Tool, ToolContext, ToolEvent, ToolOutput};
 use crate::vcb::{Array, format_length, parse_array, parse_length};
 
 /// A move in progress.
@@ -75,9 +75,9 @@ impl MoveTool {
         });
     }
 }
 
-/// Segments outlining edges, faces and instances (their bounding boxes).
+/// Segments outlining edges, faces, instances (their bounding boxes) and guides.
 pub(crate) fn outline(ctx: &ToolContext, ids: &[EntityId]) -> Vec<[DVec3; 2]> {
     let m = ctx.model;
     let mut out = Vec::new();
     for id in ids {
@@ -94,9 +94,14 @@ pub(crate) fn outline(ctx: &ToolContext, ids: &[EntityId]) -> Vec<[DVec3; 2]> {
                     out.extend(face_outline(m, f));
                 }
             }
             EntityId::Instance(_) => out.extend(box_outline(ctx, *id)),
-            EntityId::Vertex(_) | EntityId::Guide(_) => {}
+            EntityId::Guide(g) => {
+                if let Some(g) = m.guide(g) {
+                    out.extend(guide_outline(ctx, g));
+                }
+            }
+            EntityId::Vertex(_) => {}
         }
     }
     out
 }
```

Apply to `crates/tools/src/overlay.rs`:
```diff
--- a/crates/tools/src/overlay.rs
+++ b/crates/tools/src/overlay.rs
@@ -54,12 +54,12 @@ pub fn overlay(
         .collect();
 
     let Some(hit) = hit else { return out };
     let color = match hit.kind {
-        SnapKind::Endpoint | SnapKind::Origin => ENDPOINT,
+        SnapKind::Endpoint | SnapKind::Origin | SnapKind::GuidePoint => ENDPOINT,
         SnapKind::Midpoint => MIDPOINT,
         SnapKind::Intersection => INTERSECTION,
-        SnapKind::OnEdge => ON_EDGE,
+        SnapKind::OnEdge | SnapKind::OnGuide => ON_EDGE,
         SnapKind::OnFace => ON_FACE,
         _ => return out,
     };
     let depth = (hit.point - camera.eye).dot(camera.forward()).max(1e-6);
```

Apply to `crates/tools/src/lib.rs`:
```diff
--- a/crates/tools/src/lib.rs
+++ b/crates/tools/src/lib.rs
@@ -18,9 +18,9 @@ pub mod vcb;
 pub use circle::CircleTool;
 pub use erase::EraseTool;
 pub use inference::{
     Axis, Guides, Inference, Lock, SnapKind, edge_under, edge_under_in, face_under, face_under_in,
-    infer, infer_in, infer_with,
+    guide_under_where, infer, infer_in, infer_with,
 };
 pub use line::LineTool;
 pub use move_tool::MoveTool;
 pub use push_pull::PushPullTool;
```

Apply to `crates/app/src/info.rs`:
```diff
--- a/crates/app/src/info.rs
+++ b/crates/app/src/info.rs
@@ -1,13 +1,14 @@
 //! Text for the Entity Info panel: what is selected and its measurements.
 
 use glam::{DAffine3, DQuat};
-use kernel::{DefinitionKind, EntityId, Model};
+use kernel::{DefinitionKind, EntityId, Guide, Model};
 use session::Document;
 use tools::vcb::format_length;
 
 /// Lines describing the selection: counts, plus length, area or normal for a single entity,
-/// or the kind, name and rotation of a single instance. While a group or component is open, a first line
+/// the direction or position of a single guide, or the kind, name and rotation of a single
+/// instance. While a group or component is open, a first line
 /// says which.
 pub fn entity_info(doc: &Document) -> Vec<String> {
     let mut lines = editing(doc).into_iter().collect::<Vec<_>>();
     lines.extend(selection_info(doc));
@@ -29,17 +30,38 @@ fn editing(doc: &Document) -> Option<String> {
 
 fn selection_info(doc: &Document) -> Vec<String> {
     let m = doc.model();
     let (mut edges, mut faces, mut instances) = (Vec::new(), Vec::new(), Vec::new());
+    let mut guides = Vec::new();
     for id in doc.selection() {
         match id {
             EntityId::Edge(e) => edges.push(*e),
             EntityId::Face(f) => faces.push(*f),
             EntityId::Instance(i) => instances.push(*i),
-            EntityId::Vertex(_) | EntityId::Guide(_) => {}
+            EntityId::Guide(g) => guides.push(*g),
+            EntityId::Vertex(_) => {}
         }
     }
-    if let ([], [], [i]) = (&edges[..], &faces[..], &instances[..]) {
+    if let ([], [], [], [g]) = (&edges[..], &faces[..], &instances[..], &guides[..]) {
+        let xyz = |v: glam::DVec3| v.to_array().map(number).join(", ");
+        return match m.guide(*g) {
+            Some(Guide::Line { direction, .. }) => {
+                vec![
+                    "1 guide line".into(),
+                    format!("Direction: {}", xyz(*direction)),
+                ]
+            }
+            Some(Guide::Point { position }) => {
+                vec![
+                    "1 guide point".into(),
+                    format!("Position: {}", xyz(*position)),
+                ]
+            }
+            None => Vec::new(),
+        };
+    }
+    let guides = guides.len();
+    if let ([], [], [i], 0) = (&edges[..], &faces[..], &instances[..], guides) {
         let instance = m.instance(*i);
         let d = instance.and_then(|i| doc.scene().definition(i.definition));
         let mut lines = match d {
             Some(d) if d.kind == DefinitionKind::Component => {
@@ -50,14 +72,14 @@ fn selection_info(doc: &Document) -> Vec<String> {
         lines.extend(instance.and_then(|i| rotation(i.transform)));
         return lines;
     }
     match (&edges[..], &faces[..]) {
-        ([], []) if instances.is_empty() => vec!["Nothing selected".into()],
-        ([e], []) if instances.is_empty() => vec![
+        ([], []) if instances.is_empty() && guides == 0 => vec!["Nothing selected".into()],
+        ([e], []) if instances.is_empty() && guides == 0 => vec![
             "1 edge".into(),
             format!("Length: {}", format_length(edge_length(m, *e))),
         ],
-        ([], [f]) if instances.is_empty() => {
+        ([], [f]) if instances.is_empty() && guides == 0 => {
             let face = m.face(*f).unwrap();
             let n = face.normal.to_array().map(number);
             vec![
                 "1 face".into(),
@@ -75,8 +97,11 @@ fn selection_info(doc: &Document) -> Vec<String> {
             }
             if !instances.is_empty() {
                 parts.push(count(instances.len(), "instance"));
             }
+            if guides > 0 {
+                parts.push(count(guides, "guide"));
+            }
             vec![parts.join(", ")]
         }
     }
 }
@@ -162,8 +187,28 @@ mod tests {
         select(&mut d, &["e2", "e4", "f8"]);
         assert_eq!(entity_info(&d), ["2 edges, 1 face"]);
     }
 
+    #[test]
+    fn describes_guides() {
+        let mut d = doc_with_square();
+        d.execute(Command::AddGuideLine {
+            point: [0.0, -1.0, 0.0],
+            direction: [0.0, 2.0, 0.0],
+        })
+        .unwrap();
+        d.execute(Command::AddGuidePoint {
+            position: [1.5, 2.0, 0.25],
+        })
+        .unwrap();
+        select(&mut d, &["g9"]);
+        assert_eq!(entity_info(&d), ["1 guide line", "Direction: 0, 1, 0"]);
+        select(&mut d, &["g10"]);
+        assert_eq!(entity_info(&d), ["1 guide point", "Position: 1.5, 2, 0.25"]);
+        select(&mut d, &["g9", "g10", "e2"]);
+        assert_eq!(entity_info(&d), ["1 edge, 2 guides"]);
+    }
+
     #[test]
     fn describes_groups_and_components() {
         let mut d = doc_with_square();
         let ids = ["e2", "e4", "e6", "e7", "f8"]
```

Apply to `crates/app/src/editor.rs`:
```diff
--- a/crates/app/src/editor.rs
+++ b/crates/app/src/editor.rs
@@ -1051,8 +1051,48 @@ mod tests {
         e.undo();
         assert_eq!(stats(&e), (4, 1));
     }
 
+    #[test]
+    fn lines_snap_to_guides_and_select_picks_and_erases_them() {
+        let mut e = editor();
+        e.execute(Command::AddGuidePoint {
+            position: [1.0, 1.0, 0.0],
+        })
+        .unwrap();
+        e.execute(Command::AddGuideLine {
+            point: [0.0, 3.0, 0.0],
+            direction: [1.0, 0.0, 0.0],
+        })
+        .unwrap();
+        let point = DVec3::new(1.0, 1.0, 0.0);
+        e.cursor_moved(e.doc().camera.project(point).unwrap() + DVec2::new(2.0, 1.0));
+        assert_eq!(
+            e.hit.map(|h| (h.point, h.kind)),
+            Some((point, SnapKind::GuidePoint))
+        );
+        // A line drawn from the guide point to the guide line ends on it.
+        click(&mut e);
+        click_at(&mut e, DVec3::new(1.0, 3.0, 0.0));
+        e.key(EditorKey::Escape);
+        let ends = edge_ends(&e);
+        assert_eq!(ends.len(), 1);
+        assert_eq!(ends[0][0], point);
+        assert!(
+            ends[0][1].abs_diff_eq(DVec3::new(1.0, 3.0, 0.0), 1e-9),
+            "the line ends on the guide line: {ends:?}"
+        );
+
+        e.key(EditorKey::Char(' '));
+        click_at(&mut e, DVec3::new(-2.0, 3.0, 0.0));
+        assert_eq!(
+            e.doc().selection().iter().copied().collect::<Vec<_>>(),
+            vec!["g1".parse().unwrap()]
+        );
+        assert!(e.key(EditorKey::Delete));
+        assert_eq!(e.doc().model().guides().count(), 1);
+    }
+
     #[test]
     fn backspace_erases_the_selection_only_when_nothing_is_typed() {
         let mut e = editor();
         square(&mut e);
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p session --lib camera && cargo test -p tools --lib && cargo test -p render && cargo test -p app --bins`
Expected: all pass.

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
Expected: clean; 597 tests pass.

- [ ] **Step 5: Commit**

```bash
git status
git diff --stat
git add crates/render/tests/headless.rs crates/session/src/camera.rs crates/render/src/guides.rs crates/render/src/scene.rs crates/render/src/renderer.rs crates/render/src/lib.rs crates/tools/src/inference.rs crates/tools/src/select.rs crates/tools/src/erase.rs crates/tools/src/move_tool.rs crates/tools/src/overlay.rs crates/tools/src/lib.rs crates/app/src/info.rs crates/app/src/editor.rs
git commit -m "Guides in the window: snaps, picking, box selection, dashed rendering, Entity Info"
```

### Task 3: Tape Measure tool and Edit > Delete Guides

**Files:**
- Create: `crates/tools/src/tape.rs`
- Modify:
  - `crates/tools/src/tool.rs` (`ToolKind::TapeMeasure`, `ALL` has 9, label "Tape Measure", shortcut `t`), `crates/tools/src/lib.rs`
  - `crates/app/src/editor.rs` (Ctrl reaches the Tape Measure, `delete_guides()`), `ui.rs` (`UiAction::DeleteGuides`, `View.has_guides`, Edit menu; test screen 1280x800), `app.rs` (`has_guides`, the action, KeyT fallback)
  - `docs/superpowers/specs/2026-10-10-tenon-design.md` (status line, section 5 commands, M6c milestone line)
- Test: test-module hunks of the new `tape.rs` (6), `editor.rs` (1), `ui.rs` (`the_edit_menu_deletes_guides`; existing `View` literals gain `has_guides`), `app.rs` (KeyT in `non_ascii_layouts_fall_back_to_the_physical_key`). `tool.rs`'s round-trip test covers the new kind.

**Interfaces:**
- Consumes (Tasks 1-2): `guide_under_where`, `SnapKind::{OnEdge, OnGuide}`, `Camera::line_span`, `Command::{AddGuideLine, AddGuidePoint, DeleteGuides, Undo}`; also `edge_under_in`, `vcb::{parse_length, format_length}`, `LENGTH_TOL`.
- Produces:
  - `tools::TapeMeasureTool` (`new()`, `Default`, `impl Tool`; name "Tape Measure", measurement label "Length")
  - `tools::ToolKind::TapeMeasure`
  - `Editor::delete_guides(&mut self) -> bool`; `UiAction::DeleteGuides`; `View.has_guides: bool`

- [ ] **Step 1: Write the failing tests**

The tests are test-module hunks of the Step 3 diffs, plus the test module of the new `tape.rs`. Apply the `editor.rs` and `ui.rs` test hunks first.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p app --bins`
Expected: compile errors: ``no variant or associated item named `TapeMeasure` found for enum `ToolKind` `` and no `UiAction::DeleteGuides`.

- [ ] **Step 3: Implement**

Create `crates/tools/src/tape.rs`:
```rust
//! The Tape Measure tool: measures distances and draws guides. Click where to measure from,
//! then where to measure to; the measurements box shows the distance. Measured from a point,
//! the second click adds a guide point there; measured from an edge or a guide line (clicked
//! anywhere along it), the distance is taken square to that line and the second click adds a
//! guide line parallel to it through the end. A typed length sets the distance, in the
//! direction of the cursor; typed right after a guide was added, it moves that guide to the
//! new distance. Ctrl switches guide creation off (measuring only) and on again.

use glam::DVec3;
use kernel::Guide;
use kernel::geom::LENGTH_TOL;
use session::Command;

use crate::inference::{SnapKind, edge_under_in, guide_under_where};
use crate::tool::{Tool, ToolContext, ToolEvent, ToolOutput};
use crate::vcb::{format_length, parse_length};

/// Where a measurement starts.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Start {
    /// At a point: distances are straight to the cursor.
    Point(DVec3),
    /// On the line through `point` (where it was clicked) along the unit `dir`: distances are
    /// square to it.
    Line { point: DVec3, dir: DVec3 },
}

impl Start {
    fn point(self) -> DVec3 {
        match self {
            Start::Point(p) | Start::Line { point: p, .. } => p,
        }
    }

    /// The offset from the start to `p`: to the point, or square to the line.
    fn offset(self, p: DVec3) -> DVec3 {
        match self {
            Start::Point(s) => p - s,
            Start::Line { point, dir } => {
                let d = p - point;
                d - dir * d.dot(dir)
            }
        }
    }

    /// The guide `offset` away from the start: a guide point, or a parallel guide line.
    fn guide(self, offset: DVec3) -> Command {
        match self {
            Start::Point(s) => Command::AddGuidePoint {
                position: (s + offset).to_array(),
            },
            Start::Line { point, dir } => Command::AddGuideLine {
                point: (point + offset).to_array(),
                direction: dir.to_array(),
            },
        }
    }
}

#[derive(Clone, Debug)]
pub struct TapeMeasureTool {
    start: Option<Start>,
    /// The point under the cursor.
    hover: Option<DVec3>,
    /// Whether the second click adds a guide (Ctrl toggles it).
    guides: bool,
    /// The start and unit offset direction of the guide just added, for a typed distance to
    /// replace it.
    last: Option<(Start, DVec3)>,
}

impl Default for TapeMeasureTool {
    fn default() -> Self {
        TapeMeasureTool {
            start: None,
            hover: None,
            guides: true,
            last: None,
        }
    }
}

impl TapeMeasureTool {
    pub fn new() -> Self {
        Self::default()
    }

    /// The live distance from the start to the cursor.
    fn distance(&self) -> Option<f64> {
        Some(self.start?.offset(self.hover?).length())
    }

    /// Ends the measurement at `offset` from the start: adds the guide, or reports the
    /// distance when measuring only.
    fn finish(&mut self, start: Start, offset: DVec3, out: &mut ToolOutput) {
        let length = offset.length();
        if length < LENGTH_TOL {
            out.message = Some(match start {
                Start::Point(_) => "Click away from the start point".into(),
                Start::Line { .. } => "Click away from the line to place a guide".into(),
            });
            return;
        }
        if self.guides {
            out.commands.push(start.guide(offset));
            self.last = Some((start, offset / length));
        } else {
            out.message = Some(format!("Distance: {}", format_length(length)));
        }
        self.start = None;
    }

    /// Where a click at `hit` starts measuring: along the edge or guide line under the
    /// cursor when it snapped onto one, else at the point.
    fn start_at(hit: &crate::inference::Inference, ctx: &ToolContext) -> Start {
        let line = match hit.kind {
            SnapKind::OnEdge => edge_under_in(ctx.scenery, ctx.camera, ctx.cursor, ctx.pick_radius)
                .and_then(|(_, _, a, b)| (b - a).try_normalize()),
            SnapKind::OnGuide => {
                guide_under_where(ctx.scenery, ctx.camera, ctx.cursor, ctx.pick_radius, |_| {
                    true
                })
                .and_then(|(_, _, g)| match g {
                    Guide::Line { direction, .. } => Some(direction),
                    Guide::Point { .. } => None,
                })
            }
            _ => None,
        };
        match line {
            Some(dir) => Start::Line {
                point: hit.point,
                dir,
            },
            None => Start::Point(hit.point),
        }
    }
}

impl Tool for TapeMeasureTool {
    fn name(&self) -> &'static str {
        "Tape Measure"
    }

    fn prompt(&self) -> &'static str {
        match (self.start, self.guides) {
            (None, true) => {
                "Click a point, or an edge for a parallel guide; Ctrl switches guides off"
            }
            (None, false) => "Click where to measure from; Ctrl switches guides on",
            (Some(_), true) => "Click where to place the guide, or type the distance",
            (Some(_), false) => "Click where to measure to",
        }
    }

    fn measurement(&self) -> (&'static str, Option<String>) {
        ("Length", self.distance().map(format_length))
    }

    fn anchor(&self) -> Option<DVec3> {
        self.start.map(Start::point)
    }

    fn handle(&mut self, event: ToolEvent, ctx: &ToolContext) -> ToolOutput {
        let mut out = ToolOutput::default();
        match event {
            ToolEvent::Move(hit) => self.hover = Some(hit.point),
            ToolEvent::Click(hit) => {
                self.hover = Some(hit.point);
                match self.start {
                    None => {
                        self.start = Some(Self::start_at(&hit, ctx));
                        self.last = None;
                    }
                    Some(start) => self.finish(start, start.offset(hit.point), &mut out),
                }
            }
            ToolEvent::Value(text) => {
                let Some(length) = parse_length(&text) else {
                    out.message = Some(format!("Not a length: {text}"));
                    return out;
                };
                if length.abs() < LENGTH_TOL {
                    out.message = Some("The distance must not be zero".into());
                    return out;
                }
                match (self.start, self.last) {
                    (Some(start), _) => {
                        if !self.guides {
                            out.message =
                                Some("Measuring only: Ctrl switches guides on to place one".into());
                        } else if let Some(dir) =
                            self.hover.and_then(|h| start.offset(h).try_normalize())
                        {
                            self.finish(start, dir * length, &mut out);
                        } else {
                            out.message = Some("Move the cursor to show the direction".into());
                        }
                    }
                    // Right after a guide was added: move it to the typed distance.
                    (None, Some((start, dir))) => {
                        out.commands.push(Command::Undo);
                        self.finish(start, dir * length, &mut out);
                    }
                    (None, None) => {
                        out.message = Some("Click where to measure from first".into());
                    }
                }
            }
            ToolEvent::ToggleCopy => {
                self.guides = !self.guides;
                self.last = None;
                out.message = Some(if self.guides {
                    "Guides on".into()
                } else {
                    "Measuring only".into()
                });
            }
            ToolEvent::Cancel => self.on_external_change(),
            ToolEvent::Release(_) | ToolEvent::DoubleClick(_) => {}
        }
        if let (Some(start), Some(hover)) = (self.start, self.hover) {
            let offset = start.offset(hover);
            out.preview.push([hover - offset, hover]);
            if let Start::Line { dir, .. } = start
                && self.guides
                && offset.length() >= LENGTH_TOL
            {
                out.preview.extend(ctx.camera.line_span(hover, dir));
            }
        }
        out
    }

    fn on_external_change(&mut self) {
        self.start = None;
        self.last = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inference::{Inference, infer};
    use crate::scenery::Scenery;
    use glam::DVec2;
    use kernel::Model;
    use session::Camera;

    /// An edge from (0, 0, 0) to (4, 0, 0) and a guide line along Y through (6, 0, 0), seen by
    /// the default camera.
    struct Rig {
        tool: TapeMeasureTool,
        model: Model,
        camera: Camera,
        cursor: DVec2,
    }

    impl Rig {
        fn new() -> Self {
            let mut model = Model::new();
            model
                .add_edge(DVec3::ZERO, DVec3::new(4.0, 0.0, 0.0))
                .unwrap();
            model
                .add_guide(Guide::Line {
                    point: DVec3::new(6.0, 0.0, 0.0),
                    direction: DVec3::Y,
                })
                .unwrap();
            Rig {
                tool: TapeMeasureTool::new(),
                model,
                camera: Camera::new(DVec2::new(800.0, 600.0)),
                cursor: DVec2::ZERO,
            }
        }

        fn send(&mut self, event: ToolEvent) -> ToolOutput {
            let scenery = Scenery::single(&self.model);
            let ctx = ToolContext {
                model: &self.model,
                scenery: &scenery,
                camera: &self.camera,
                cursor: self.cursor,
                pick_radius: 10.0,
                drag_threshold: 4.0,
                shift: false,
                selection: &Default::default(),
            };
            self.tool.handle(event, &ctx)
        }

        /// The inference with the cursor over `p`.
        fn at(&mut self, p: DVec3) -> Inference {
            self.cursor = self.camera.project(p).unwrap();
            infer(
                &self.model,
                &self.camera,
                self.cursor,
                self.tool.anchor(),
                10.0,
            )
        }

        fn click(&mut self, p: DVec3) -> ToolOutput {
            let hit = self.at(p);
            self.send(ToolEvent::Click(hit))
        }

        fn hover(&mut self, p: DVec3) -> ToolOutput {
            let hit = self.at(p);
            self.send(ToolEvent::Move(hit))
        }
    }

    /// Asserts that `commands` end with a guide line through about `point` along `direction`.
    fn assert_line(commands: &[Command], point: [f64; 3], direction: [f64; 3]) {
        let Some(Command::AddGuideLine {
            point: p,
            direction: d,
        }) = commands.last()
        else {
            panic!("no guide line in {commands:?}");
        };
        assert!(
            DVec3::from(*p).abs_diff_eq(DVec3::from(point), 1e-9) && *d == direction,
            "{commands:?}"
        );
    }

    #[test]
    fn from_an_edge_a_parallel_guide_line_goes_through_the_second_click() {
        let mut r = Rig::new();
        r.click(DVec3::new(1.5, 0.0, 0.0));
        assert_eq!(
            r.tool.start.map(|s| matches!(s, Start::Line { .. })),
            Some(true)
        );
        r.hover(DVec3::new(2.0, 2.5, 0.0));
        let (label, value) = r.tool.measurement();
        assert_eq!((label, value.as_deref()), ("Length", Some("2.5 m")));
        let out = r.click(DVec3::new(2.0, 2.5, 0.0));
        let [Command::AddGuideLine { point, direction }] = out.commands[..] else {
            panic!("{:?}", out.commands);
        };
        assert!((point[1] - 2.5).abs() < 1e-9 && point[2].abs() < 1e-9);
        assert!((point[0] - 1.5).abs() < 1e-6, "square to the edge");
        assert_eq!(direction, [1.0, 0.0, 0.0]);
        assert_eq!(r.tool.anchor(), None);
    }

    #[test]
    fn from_a_point_the_second_click_adds_a_guide_point() {
        let mut r = Rig::new();
        r.click(DVec3::new(4.0, 0.0, 0.0));
        assert_eq!(r.tool.start, Some(Start::Point(DVec3::new(4.0, 0.0, 0.0))));
        let out = r.click(DVec3::ZERO);
        assert_eq!(
            out.commands,
            vec![Command::AddGuidePoint { position: [0.0; 3] }]
        );
        // Clicking the start point again adds nothing.
        r.click(DVec3::new(4.0, 0.0, 0.0));
        let out = r.click(DVec3::new(4.0, 0.0, 0.0));
        assert!(out.commands.is_empty());
        assert!(out.message.unwrap().contains("away"));
    }

    #[test]
    fn from_a_guide_line_a_parallel_guide_line_too() {
        let mut r = Rig::new();
        r.click(DVec3::new(6.0, 3.0, 0.0));
        let out = r.send(ToolEvent::Value("1.5".into()));
        assert!(out.message.unwrap().contains("Move the cursor"));
        r.hover(DVec3::new(8.0, 3.0, 0.0));
        let out = r.send(ToolEvent::Value("1.5".into()));
        assert_eq!(out.commands.len(), 1);
        assert_line(&out.commands, [7.5, 3.0, 0.0], [0.0, 1.0, 0.0]);
    }

    #[test]
    fn a_typed_length_follows_the_cursor_and_retyping_replaces_the_guide() {
        let mut r = Rig::new();
        r.click(DVec3::new(2.5, 0.0, 0.0));
        r.hover(DVec3::new(2.5, -1.0, 0.0));
        let out = r.send(ToolEvent::Value("2".into()));
        assert_eq!(out.commands.len(), 1);
        assert_line(&out.commands, [2.5, -2.0, 0.0], [1.0, 0.0, 0.0]);
        let out = r.send(ToolEvent::Value("3".into()));
        assert_eq!((out.commands.len(), &out.commands[0]), (2, &Command::Undo));
        assert_line(&out.commands, [2.5, -3.0, 0.0], [1.0, 0.0, 0.0]);
        // A new measurement ends the chance to retype.
        r.click(DVec3::new(4.0, 0.0, 0.0));
        r.send(ToolEvent::Cancel);
        let out = r.send(ToolEvent::Value("3".into()));
        assert!(out.commands.is_empty());
    }

    #[test]
    fn ctrl_switches_to_measuring_only() {
        let mut r = Rig::new();
        let out = r.send(ToolEvent::ToggleCopy);
        assert_eq!(out.message.as_deref(), Some("Measuring only"));
        r.click(DVec3::ZERO);
        let out = r.click(DVec3::new(4.0, 0.0, 0.0));
        assert!(out.commands.is_empty());
        assert_eq!(out.message.as_deref(), Some("Distance: 4 m"));
        r.click(DVec3::ZERO);
        r.hover(DVec3::new(4.0, 0.0, 0.0));
        let out = r.send(ToolEvent::Value("2".into()));
        assert!(out.commands.is_empty() && out.message.is_some());
        r.send(ToolEvent::ToggleCopy);
        let out = r.send(ToolEvent::Value("2".into()));
        assert_eq!(
            out.commands,
            vec![Command::AddGuidePoint {
                position: [2.0, 0.0, 0.0]
            }]
        );
    }

    #[test]
    fn the_preview_shows_the_measure_and_the_guide_to_be() {
        let mut r = Rig::new();
        r.click(DVec3::new(1.0, 0.0, 0.0));
        let out = r.hover(DVec3::new(1.0, 2.0, 0.0));
        assert_eq!(out.preview.len(), 2, "the measure and the guide line");
        assert!(out.preview[0][0].abs_diff_eq(DVec3::new(1.0, 0.0, 0.0), 1e-6));
        let out = r.send(ToolEvent::Cancel);
        assert!(out.preview.is_empty());
        assert_eq!(r.tool.anchor(), None);
    }
}
```

Apply to `crates/tools/src/tool.rs`:
```diff
--- a/crates/tools/src/tool.rs
+++ b/crates/tools/src/tool.rs
@@ -15,8 +15,9 @@ use crate::push_pull::PushPullTool;
 use crate::rectangle::RectangleTool;
 use crate::rotate::RotateTool;
 use crate::scenery::Scenery;
 use crate::select::SelectTool;
+use crate::tape::TapeMeasureTool;
 
 /// What a tool may look at while handling an event. Points, the camera and entity ids are in
 /// the open context's coordinates (see `session::Document::context`).
 pub struct ToolContext<'a> {
@@ -52,9 +53,9 @@ pub enum ToolEvent {
     Value(String),
     /// Escape: abandon the operation in progress.
     Cancel,
     /// The copy modifier was tapped (Ctrl): tools that can copy switch between moving and
-    /// copying.
+    /// copying; the Tape Measure switches guide creation on and off.
     ToggleCopy,
 }
 
 /// What a tool wants done after an event.
@@ -91,21 +92,23 @@ pub enum ToolKind {
     Circle,
     PushPull,
     Move,
     Rotate,
+    TapeMeasure,
     Erase,
 }
 
 impl ToolKind {
     /// Every tool, in toolbar order.
-    pub const ALL: [ToolKind; 8] = [
+    pub const ALL: [ToolKind; 9] = [
         ToolKind::Select,
         ToolKind::Line,
         ToolKind::Rectangle,
         ToolKind::Circle,
         ToolKind::PushPull,
         ToolKind::Move,
         ToolKind::Rotate,
+        ToolKind::TapeMeasure,
         ToolKind::Erase,
     ];
 
     pub fn make(self) -> Box<dyn Tool> {
@@ -116,8 +119,9 @@ impl ToolKind {
             ToolKind::Circle => Box::new(CircleTool::new()),
             ToolKind::PushPull => Box::new(PushPullTool::new()),
             ToolKind::Move => Box::new(MoveTool::new()),
             ToolKind::Rotate => Box::new(RotateTool::new()),
+            ToolKind::TapeMeasure => Box::new(TapeMeasureTool::new()),
             ToolKind::Erase => Box::new(EraseTool::new()),
         }
     }
 
@@ -130,8 +134,9 @@ impl ToolKind {
             ToolKind::Circle => "Circle",
             ToolKind::PushPull => "Push/Pull",
             ToolKind::Move => "Move",
             ToolKind::Rotate => "Rotate",
+            ToolKind::TapeMeasure => "Tape Measure",
             ToolKind::Erase => "Erase",
         }
     }
 
@@ -144,8 +149,9 @@ impl ToolKind {
             ToolKind::Circle => 'c',
             ToolKind::PushPull => 'p',
             ToolKind::Move => 'm',
             ToolKind::Rotate => 'q',
+            ToolKind::TapeMeasure => 't',
             ToolKind::Erase => 'e',
         }
     }
 
```

Apply to `crates/tools/src/lib.rs`:
```diff
--- a/crates/tools/src/lib.rs
+++ b/crates/tools/src/lib.rs
@@ -11,8 +11,9 @@ pub mod push_pull;
 pub mod rectangle;
 pub mod rotate;
 pub mod scenery;
 pub mod select;
+pub mod tape;
 pub mod tool;
 pub mod vcb;
 
 pub use circle::CircleTool;
@@ -27,5 +28,6 @@ pub use push_pull::PushPullTool;
 pub use rectangle::RectangleTool;
 pub use rotate::RotateTool;
 pub use scenery::{Owner, Part, Scenery};
 pub use select::SelectTool;
+pub use tape::TapeMeasureTool;
 pub use tool::{Tool, ToolContext, ToolEvent, ToolKind, ToolOutput};
```

Apply to `crates/app/src/editor.rs`:
```diff
--- a/crates/app/src/editor.rs
+++ b/crates/app/src/editor.rs
@@ -235,12 +235,12 @@ impl Editor {
     pub fn cursor_left(&mut self) -> bool {
         self.hit.take().is_some()
     }
 
-    /// The copy modifier (Ctrl) was tapped. Only the Move and Rotate tools copy; elsewhere it
-    /// does nothing and returns false.
+    /// The copy modifier (Ctrl) was tapped. The Move and Rotate tools switch copying, the Tape
+    /// Measure switches guide creation; elsewhere it does nothing and returns false.
     pub fn toggle_copy(&mut self) -> bool {
-        if !copies(self.kind) {
+        if !copies(self.kind) && self.kind != ToolKind::TapeMeasure {
             return false;
         }
         self.send(ToolEvent::ToggleCopy);
         true
@@ -419,8 +419,16 @@ impl Editor {
         }
         self.run_selection_command(Command::Explode { ids })
     }
 
+    /// Erases every guide of the open context as one undo step. False if there are none.
+    pub fn delete_guides(&mut self) -> bool {
+        if self.doc.model().guides().next().is_none() {
+            return false;
+        }
+        self.run_selection_command(Command::DeleteGuides)
+    }
+
     /// Closes the open group or component. False if none is open.
     pub fn close_instance(&mut self) -> bool {
         if self.doc.context().is_empty() {
             return false;
@@ -1091,8 +1099,53 @@ mod tests {
         assert!(e.key(EditorKey::Delete));
         assert_eq!(e.doc().model().guides().count(), 1);
     }
 
+    #[test]
+    fn t_measures_from_an_edge_to_a_parallel_guide_and_ctrl_measures_only() {
+        let mut e = editor();
+        square(&mut e);
+        assert!(e.key(EditorKey::Char('t')));
+        assert_eq!(e.tool_kind(), ToolKind::TapeMeasure);
+        // From the bottom side of the square, 1.5 m out (the guide follows the cursor's side).
+        click_at(&mut e, DVec3::new(2.5, 1.0, 0.0));
+        move_to(&mut e, DVec3::new(2.5, -0.5, 0.0));
+        assert!(e.status().contains("Length: 1.5 m"), "{}", e.status());
+        type_text(&mut e, "2");
+        let guides: Vec<kernel::Guide> = e.doc().model().guides().map(|(_, g)| *g).collect();
+        let [kernel::Guide::Line { point, direction }] = guides[..] else {
+            panic!("{guides:?}");
+        };
+        assert!((point.y + 1.0).abs() < 1e-9, "{point:?}");
+        assert_eq!(direction, DVec3::X);
+        // Retyping moves that guide: one undo step replaced by another.
+        type_text(&mut e, "3");
+        let lines: Vec<f64> = e
+            .doc()
+            .model()
+            .guides()
+            .map(|(_, g)| g.anchor().y)
+            .collect();
+        assert_eq!(lines.len(), 1);
+        assert!((lines[0] + 2.0).abs() < 1e-9, "{lines:?}");
+
+        assert!(e.toggle_copy());
+        click_at(&mut e, DVec3::new(1.0, 1.0, 0.0));
+        click_at(&mut e, DVec3::new(3.0, 3.0, 0.0));
+        assert!(e.status().contains("Distance: 2.828 m"), "{}", e.status());
+        assert_eq!(
+            e.doc().model().guides().count(),
+            1,
+            "no guide when measuring"
+        );
+
+        assert!(e.delete_guides());
+        assert_eq!(e.doc().model().guides().count(), 0);
+        assert!(!e.delete_guides());
+        e.undo();
+        assert_eq!(e.doc().model().guides().count(), 1);
+    }
+
     #[test]
     fn backspace_erases_the_selection_only_when_nothing_is_typed() {
         let mut e = editor();
         square(&mut e);
```

Apply to `crates/app/src/ui.rs`:
```diff
--- a/crates/app/src/ui.rs
+++ b/crates/app/src/ui.rs
@@ -20,8 +20,9 @@ pub enum UiAction {
     OpenFile,
     SaveFile,
     SaveFileAs,
     ExportFile,
+    DeleteGuides,
 }
 
 /// What the panels show.
 pub struct View<'a> {
@@ -32,8 +33,10 @@ pub struct View<'a> {
     /// Whether a group or component is selected.
     pub has_instance_selected: bool,
     /// Whether a group or component is open for editing.
     pub inside_instance: bool,
+    /// Whether the open context has guides to delete.
+    pub has_guides: bool,
 }
 
 /// Lays out the panels for one frame. Returns the actions clicked and the rectangle left for
 /// the 3D view, in points.
@@ -57,8 +60,16 @@ pub fn draw(ui: &mut Ui, view: &View) -> (Vec<UiAction>, egui::Rect) {
                         actions.push(action);
                     }
                 }
             });
+            ui.menu_button("Edit", |ui| {
+                if ui
+                    .add_enabled(view.has_guides, egui::Button::new("Delete Guides"))
+                    .clicked()
+                {
+                    actions.push(UiAction::DeleteGuides);
+                }
+            });
             ui.separator();
             for kind in ToolKind::ALL {
                 let key = match kind.shortcut() {
                     ' ' => "Space".to_string(),
@@ -154,11 +165,12 @@ mod tests {
             .drop_without_applying_deltas();
         out.unwrap()
     }
 
+    /// The window's default size (see `app.rs`); the toolbar needs about 1150 points.
     fn screen() -> egui::RawInput {
         egui::RawInput {
-            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1000.0, 700.0))),
+            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1280.0, 800.0))),
             ..Default::default()
         }
     }
 
@@ -201,8 +213,9 @@ mod tests {
             info: &info,
             has_selection: false,
             has_instance_selected: false,
             inside_instance: false,
+            has_guides: false,
         };
         let (_, central) = run(&ctx, screen(), &view);
         let file = Pos2::new(14.0, central.min.y / 2.0);
         let mut clicked = Vec::new();
@@ -236,8 +249,47 @@ mod tests {
             assert!(clicked.contains(&action), "{action:?} in {clicked:?}");
         }
     }
 
+    #[test]
+    fn the_edit_menu_deletes_guides() {
+        let ctx = egui::Context::default();
+        let status = status();
+        let info = Vec::new();
+        let view = View {
+            tool: ToolKind::Line,
+            status: &status,
+            info: &info,
+            has_selection: false,
+            has_instance_selected: false,
+            inside_instance: false,
+            has_guides: true,
+        };
+        let (_, central) = run(&ctx, screen(), &view);
+        let row = central.min.y / 2.0;
+        let mut clicked = Vec::new();
+        // Open each menu right of File in turn, then click just below it.
+        for x in (40..90).step_by(6) {
+            click(&ctx, &view, Pos2::new(x as f32, row));
+            let _ = run(&ctx, screen(), &view);
+            clicked.extend(click(
+                &ctx,
+                &view,
+                Pos2::new(x as f32 + 10.0, central.min.y + 8.0),
+            ));
+            let mut escape = screen();
+            escape.events = vec![egui::Event::Key {
+                key: egui::Key::Escape,
+                physical_key: None,
+                pressed: true,
+                repeat: false,
+                modifiers: Default::default(),
+            }];
+            let _ = run(&ctx, escape, &view);
+        }
+        assert!(clicked.contains(&UiAction::DeleteGuides), "{clicked:?}");
+    }
+
     #[test]
     fn panels_leave_a_central_view_inside_the_window() {
         let ctx = egui::Context::default();
         let status = status();
@@ -248,17 +300,18 @@ mod tests {
             info: &info,
             has_selection: false,
             has_instance_selected: false,
             inside_instance: false,
+            has_guides: false,
         };
         let (actions, rect) = run(&ctx, screen(), &view);
         assert!(actions.is_empty());
         assert!(
             rect.min.x == 0.0 && rect.min.y > 0.0,
             "toolbar above: {rect:?}"
         );
-        assert!(rect.max.x < 1000.0, "info panel on the right: {rect:?}");
-        assert!(rect.max.y < 700.0, "status bar below: {rect:?}");
+        assert!(rect.max.x < 1280.0, "info panel on the right: {rect:?}");
+        assert!(rect.max.y < 800.0, "status bar below: {rect:?}");
         assert!(rect.width() > 600.0 && rect.height() > 500.0, "{rect:?}");
     }
 
     #[test]
@@ -272,14 +325,15 @@ mod tests {
             info: &info,
             has_selection: true,
             has_instance_selected: true,
             inside_instance: true,
+            has_guides: true,
         };
         let (_, central) = run(&ctx, screen(), &view);
         let row = central.min.y / 2.0;
         // Click along the toolbar row until a press lands on the Rectangle button.
         let mut clicked = Vec::new();
-        for x in (2..1000).step_by(4) {
+        for x in (2..1280).step_by(4) {
             let pos = Pos2::new(x as f32, row);
             let pointer = |pressed: bool| egui::Event::PointerButton {
                 pos,
                 button: egui::PointerButton::Primary,
```

Apply to `crates/app/src/app.rs`:
```diff
--- a/crates/app/src/app.rs
+++ b/crates/app/src/app.rs
@@ -294,8 +294,11 @@ impl State {
             }
             UiAction::CloseInstance => {
                 self.editor.close_instance();
             }
+            UiAction::DeleteGuides => {
+                self.editor.delete_guides();
+            }
         }
     }
 
     fn resize(&mut self, width: u32, height: u32) {
@@ -345,8 +348,9 @@ impl State {
                 .selection()
                 .iter()
                 .any(|id| matches!(id, EntityId::Instance(_))),
             inside_instance: !self.editor.doc().context().is_empty(),
+            has_guides: self.editor.doc().model().guides().next().is_some(),
         };
         let mut gui_frame = self.gui.frame(&self.window, &panels);
         // Applied once the frame is presented: some open modal dialogs, which must not run
         // while this frame's surface texture is held.
@@ -535,8 +539,9 @@ fn physical_char(key: PhysicalKey) -> Option<char> {
         KeyCode::KeyG => 'g',
         KeyCode::KeyS => 's',
         KeyCode::KeyC => 'c',
         KeyCode::KeyQ => 'q',
+        KeyCode::KeyT => 't',
         _ => return None,
     })
 }
 
@@ -675,9 +680,10 @@ impl ApplicationHandler<ApiWake> for App {
                 s.modifiers = m.state();
                 let locked = s
                     .editor
                     .set_modifiers(m.state().shift_key(), m.state().alt_key());
-                // Tapping Ctrl switches tools that can copy (Move) between moving and copying.
+                // Tapping Ctrl switches tools that can copy (Move, Rotate) between moving and
+                // copying, and the Tape Measure between adding guides and measuring only.
                 let toggled = ctrl_pressed && s.editor.toggle_copy();
                 locked || toggled
             }
             WindowEvent::Focused(false) => s.editor.focus_lost(),
@@ -882,8 +888,10 @@ mod tests {
         let s = PhysicalKey::Code(KeyCode::KeyS);
         assert_eq!(ch(editor_key(&arabic, s, false)), Some('s'));
         let q = PhysicalKey::Code(KeyCode::KeyQ);
         assert_eq!(ch(editor_key(&arabic, q, false)), Some('q'));
+        let t = PhysicalKey::Code(KeyCode::KeyT);
+        assert_eq!(ch(editor_key(&arabic, t, false)), Some('t'));
         let ascii = Key::Character(SmolStr::new("g"));
         assert_eq!(ch(editor_key(&ascii, g, true)), Some('g'));
     }
 }
```

Apply to `docs/superpowers/specs/2026-10-10-tenon-design.md`:
```diff
--- a/docs/superpowers/specs/2026-10-10-tenon-design.md
+++ b/docs/superpowers/specs/2026-10-10-tenon-design.md
@@ -1,8 +1,8 @@
 # Tenon — Design
 
 Date: 2026-10-10
-Status: Approved; amended 2026-10-10 (agent control, M2.5; M3/M4 scope); 2026-10-11 (M4 split into M4a-c; M4b scope; M4c scope; M5 split into M5a-c; M5b tools); 2026-10-12 (M5c files, M5d export, M6a circles, M6b rotate)
+Status: Approved; amended 2026-10-10 (agent control, M2.5; M3/M4 scope); 2026-10-11 (M4 split into M4a-c; M4b scope; M4c scope; M5 split into M5a-c; M5b tools); 2026-10-12 (M5c files, M5d export, M6a circles, M6b rotate, M6c guides)
 
 ## Goal
 
 A desktop 3D modeler in Rust with SketchUp-style direct modeling: draw edges, faces form automatically, push/pull faces, inference snapping, groups and components. macOS first; Windows and Linux supported through wgpu without dedicated work.
@@ -150,12 +150,12 @@ Unit tests for buffer building and camera math; a small set of headless golden-i
 Design rule for every milestone: the interface is one client of the command layer, and the API is another. No feature may exist only in the interface.
 
 ### Commands
 
-- `session::Command` is a serde-serializable enum covering every state change: modeling (add edge, draw rectangle/circle, push/pull face by distance, move/copy/rotate entities, erase, make group/component, explode), editing context (open/close an instance), selection, camera (set, orbit, zoom extents, standard views), undo/redo, file (new, open, save, import, export).
+- `session::Command` is a serde-serializable enum covering every state change: modeling (add edge, draw rectangle/circle, push/pull face by distance, move/copy/rotate entities, erase, make group/component, explode), guides (add a guide line or point, delete guides), editing context (open/close an instance), selection, camera (set, orbit, zoom extents, standard views), undo/redo, file (new, open, save, import, export).
 - Commands take explicit values (points, distances, entity IDs), never screen coordinates or cursor state.
 - `Document::execute(Command) -> Result<Outcome, CommandError>`, where `Outcome` carries the net `ChangeSummary` (created/deleted/modified ids as strings). Each modeling command is one undo step; camera commands are not recorded. Errors are structured (`kind` + `message` + offending `ids`), never panics. Raw `ChangeSet`s stay internal.
-- Entity IDs are the kernel's stable ids, serialized as strings (`"v12"`, `"e3"`, `"f7"`, `"i4"` for an instance); definitions are `"d<n>"`.
+- Entity IDs are the kernel's stable ids, serialized as strings (`"v12"`, `"e3"`, `"f7"`, `"i4"` for an instance, `"g9"` for a guide); definitions are `"d<n>"`.
 
 ### Queries
 
 Read-only, structured: list/get entities (vertices, edges, faces, groups, instances) with geometry, bounding-box and type filters, selection, camera, model stats, undo history. Agents do not need screenshots to know the model state.
@@ -195,8 +195,9 @@ Every milestone's acceptance includes: new capabilities are reachable as `sessio
 - **M5c**: files: `.skr` save/open/new as commands (zip with versioned JSON keeping every id), opening a file given on the command line, File menu with native dialogs and shortcuts, unsaved-changes prompts; rename and purge definitions.
 - **M5d**: export OBJ, STL, glTF as commands and from the File menu (faces as triangles in root coordinates; OBJ and glTF Y-up, STL Z-up).
 - **M6a**: circles: an `add_circle` command (center, radius, plane normal, 3-360 sides, default 24) and a Circle tool (center, then rim or typed radius; "24s" sets the side count). `c` selects the Circle tool and Shift+G makes a component. Circles are plain edge chains until M6d tags them as curves.
 - **M6b**: rotate: a `rotate` command (vertices, edges, faces and instances about an axis through a center, angle in degrees, exact quarter turns; optional `copies` for radial arrays) and a Rotate tool (`q`: center on a face turns about its normal, else about Z; reference click, then angle click with 15-degree snapping, or a typed angle; Ctrl copies, "5x" and "/5" arrays). Turned placements work everywhere: render normals, picking, box select, inference and the tools' camera inside an open turned instance, bounds, Entity Info (rotation line), explode, files and export.
+- **M6c**: guides and the Tape Measure: guide lines (infinite) and guide points are model entities (`g<n>` ids) of the context they are drawn in; `add_guide_line`, `add_guide_point` and `delete_guides` commands, and `erase`, `select`, `move`, `rotate`, `copy`, `make_group` and `explode` take guides; saved in `.skr` (schema version 2; version 1 still opens), never exported. Inference snaps to guide points, onto guide lines, and where guide lines cross edges or each other. Guides are drawn dashed (guide points as small crosses), picked, box-selected and erased like edges, and shown in Entity Info. Tape Measure tool (`t`): from a point the second click adds a guide point, from an edge or guide line a parallel guide line; typed distances, retyped right after to adjust; Ctrl switches to measuring only. Edit > Delete Guides.
 
 ## Risks
 
 - **Sticky geometry robustness**: edge merging and face detection under floating-point error is the main risk. Mitigation: robust predicates, explicit tolerances, invariant checks in every test, fuzz tests on random edge sequences.
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p tools --lib tape && cargo test -p app --bins`
Expected: all pass (6 in `tape`).

Run: `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
Expected: clean; 605 tests pass.

- [ ] **Step 5: Commit**

```bash
git status
git diff --stat
git add crates/tools/src/tape.rs crates/tools/src/tool.rs crates/tools/src/lib.rs crates/app/src/editor.rs crates/app/src/ui.rs crates/app/src/app.rs docs/superpowers/specs/2026-10-10-tenon-design.md
git commit -m "Tape Measure tool (t): guide points and parallel guide lines, typed distances, Ctrl measures only; Edit > Delete Guides"
```

## Self-review notes

- **Spec coverage:**
  - Tape Measure and guides (section 2 v1 tools): Tasks 1-3;
  - guide commands (section 5): Task 1;
  - snapping to guides (inference): Task 2.
- **Type names used across tasks:** `Guide`, `GuideId`, `EntityId::Guide`, `GuideData`, `Command::{AddGuideLine, AddGuidePoint, DeleteGuides}`, `GuideInfo`, `Camera::line_span`, `SnapKind::{GuidePoint, OnGuide}`, `guide_under_where`, `GuideShape`, `guide_lines`, `TapeMeasureTool`, `ToolKind::TapeMeasure`.
- **Deferred:**
  - guide x face snaps, perpendicular foot onto guides, guides as reference directions;
  - Tape Measure resizing the model;
  - guides in the BVH;
  - explode coordinate check for guides;
  - picking guides inside closed groups;
  - toolbar overflow below about 1150 points;
  - dash and cross sizes not scaled by the display scale factor.
