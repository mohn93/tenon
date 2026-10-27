# M9d Texture Positioning Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A face can carry a texture placement (offset, rotation and scale on top of M9b's projection), set through `position_texture` as one undo step, reported in the `face` entity data, kept through splits, copies and groups, saved in `.skr` schema 9, written into glTF and OBJ texture coordinates, shown in the window with back sides no longer mirrored, and edited from Entity Info.

**Architecture:**
- Kernel `texture.rs`: `TexturePlacement { offset: [f64; 2], rotation_deg, scale: [f64; 2] }` with `place` (projected meters to placed meters) and `turned_over`; `PlacementEdit` (the command's optional fields plus `reset`); `Face::texture_position(p)` (projection plus placement, the front's); `back_texture_position` (u mirrored). `Face.texture: Option<TexturePlacement>` (never the default), changed through `Op::PlaceTexture` so undo restores it; `Model::position_texture` is the edit. Splits (`faces.rs` reconcile), stencils (copy, rotate copies, groups, explode) and push/pull caps carry it; `Face::reversed` turns it over. `FaceData.texture_position` in plain data; `Scene::validate` checks ranges. `Triangle.uvs` become placed positions, so export needs no change of its own.
- Session: `Command::PositionTexture`, `document/placement.rs`, `Outcome.positioned`, `FaceInfo.texture_position`; `file.rs` writes schema 9 only when a face has a placement (`LIGHTS_SCHEMA_VERSION` = 8 is the old top).
- Render: `MeshParts` positions come from `Face::texture_position`; `both_uv` mirrors the back side's u. No vertex format or shader change.
- App: `Editor::selection_texture_position` and setters (`editor/texture_position.rs`), Entity Info rows Offset, Rotation, Scale and the Rotate 90 and Reset buttons in `ui.rs`, wired in `app.rs`.

**Tech Stack:** Rust 2024; glam 0.34, wgpu 30, egui 0.36. No dependency changes.

**Spec:** `docs/superpowers/specs/2026-10-10-tenon-design.md` ("Post-v1 milestones", M9d; M9b for the projection it builds on).

## Global Constraints

- No emoji anywhere (code, comments, docs, commit messages). Comments succinct, in the style of the surrounding code.
- Every new capability is a `session::Command` reachable through the API and covered by a command-driven test. Each modeling command is one undo step.
- `cargo fmt --all` clean; `cargo clippy --workspace --all-targets -- -D warnings` clean; `cargo test --workspace` passes after every task.
- No dependency changes.
- From the spec: `offset` [u, v] in meters, `rotation_deg` counterclockwise seen from the face's front, `scale` [u, v] multipliers (default 1); `position_texture {ids, offset?, rotation_deg?, scale?, reset?}` on faces of the open context, one undo step reporting how many faces changed; `.skr` schema 9, older files open without placements.
- Git: work on `dev`; stage only the files you changed (other agents may share the checkout: check `git status` and `git diff` first); short plain commit messages; never push.
- Do not launch the app window.

## Spec notes (rulings made in this plan)

- **Where it is stored.** `kernel::Face.texture: Option<TexturePlacement>`, one per face for both sides and whatever material the sides have (a face repainted later keeps it). None means the texture lies as projected; a placement equal to the default (offset 0, rotation 0, scale 1) is always stored as none, so "reset" and "set back to defaults" are the same state and files and entity data omit it. Changes go through a new primitive `Op::PlaceTexture(face, from, to)`, so undo, redo and atomic batches restore it exactly.
- **How it composes with the projection.** With `b` the M9b texture position in meters (`texture_position(normal, p)`: along u and v), the placed position is `S^-1 R(-rotation) (b - offset)`, `S` = diag(scale): the texture is moved by `offset`, turned counter-clockwise (seen from the front, u to the right, v up) about the moved origin, and each copy covers `scale` times its width and height. The renderer and the exporters divide the placed position by the material's texture size exactly as before.
- **Ranges.** `offset` components finite and within +/-1e6 m (`kernel::MAX_TEXTURE_OFFSET`); `rotation_deg` any finite angle, stored brought into 0 up to 360 (`kernel::normal_angle`: -90 is 270, 450 is 90); `scale` components from 0.001 to 1000 (`MIN_TEXTURE_SCALE`, `MAX_TEXTURE_SCALE`; no negative scales, so no mirroring). Out of range is `invalid_input` naming the field ("texture offset", "texture rotation", "texture scale") and nothing changes.
- **Command.** `position_texture {ids, offset?, rotation_deg?, scale?, reset?}`: fields given replace the face's (others kept); `reset: true` clears the placement first, then applies any fields given. `ids` must be faces of the open context (other kinds `invalid_input` naming them; unknown `not_found`; empty `invalid_input`); a command with no field and no reset is `invalid_input` ("give offset, rotation_deg, scale or reset"). The outcome's `positioned` counts the faces that changed (they are also in `changes.modified`); when nothing changes there is no undo step.
- **Back side.** The back's texture position is the front's placed position with u negated (`kernel::back_texture_position`), so seen from behind the image is not mirrored and the placement shows as the same physical move and turn (a counter-clockwise turn on the front reads clockwise from behind, like a decal seen through glass would if it were printed on both faces). This changes M9b's look of every textured back side, as the spec asks; the stored `textured` golden image shows no back side and still matches.
- **Turning a face over.** `Face::reversed` (push/pull orientation, `orient`) converts the placement so each side keeps showing what it showed: on a sloping or vertical face u flips, so offset u and the rotation are negated; on a level face v flips, so offset v is negated and the rotation becomes `-rotation - 180`. A face without a placement stays without.
- **What new faces get.** Pieces of a face split by drawing keep its placement (they take the container face's paint and placement in `reconcile`), and so does a face merged by erasing the edge between two faces (the container's, as paint already does). Copies (`copy`, rotate copies), faces moved into a group or component and out of one by `explode` keep theirs (the stencil carries it); duplicated definitions keep theirs (cloned meshes). A push/pull cap keeps the pushed face's placement: it is that face moved (turned over with it when the pushed sheet turns over). Push/pull sides and recess walls, faces filling openings, imported faces and every other new face have none. Moving or rotating faces keeps the numbers: the pattern stays anchored to the definition's coordinates as in M9b (a moved face does not carry its pattern along), as the projection already behaves.
- **Face entity JSON.** `"texture_position": {"offset": [u, v], "rotation_deg": r, "scale": [u, v]}` on a face's `entity` result and its `entities` entry, absent when none. The `.skr` face object uses the same key and shape.
- **Files.** `schema_version` 9 (`SCHEMA_VERSION`) only when some face in any mesh has a placement; otherwise 8 (`LIGHTS_SCHEMA_VERSION`, new name for the old top), 7, 6 or 5 as before, so models without placements are written exactly as before. Files of version 8 and older open without placements; a stored default placement reads as none; an out-of-range placement fails the open naming the face.
- **Export.** `Triangle.uvs` carries the front's placed positions, so glTF `TEXCOORD_0` and OBJ `vt` are placed with no exporter change (back sides are still not exported).
- **Window.** With only faces selected, Entity Info shows "Texture position" with the first selected face's (lowest id) placement, or the defaults: Offset (two fields, m), Rotation (degrees, 0 to 360), Scale (two fields), then Rotate 90 and Reset. Each field applies only that field to every selected face (so faces with different rotations keep them when the offset is edited), once per drag or typed value. `UiAction` carries millimeters, tenths of a degree and thousandths so it stays `Eq`. Rotate 90 adds 90 degrees to each face's own rotation: faces sharing a rotation share one `position_texture`, all in one atomic batch (one undo step).
- **Kernel API.** `kernel::{TexturePlacement, PlacementEdit, back_texture_position, normal_angle, MIN_TEXTURE_SCALE, MAX_TEXTURE_SCALE, MAX_TEXTURE_OFFSET}`, `Face::texture_position(&self, p: DVec3) -> DVec2`, `Model::position_texture(&mut self, &[FaceId], PlacementEdit) -> Result<(ChangeSet, usize), KernelError>`, `KernelError::TexturePlacement(String)`.

## File Structure

| File | Responsibility | Task |
|---|---|---|
| `crates/kernel/src/texture.rs` | `TexturePlacement`, `PlacementEdit`, limits, `check_placement`, `normal_angle`, `is_level`, `back_texture_position`, `Face::texture_position`, `Model::position_texture` | 1 |
| `crates/kernel/src/model.rs` | `Face.texture`, `Face::reversed` turning it over, `Op::PlaceTexture`, `Model::set_texture` | 1 |
| `crates/kernel/src/faces.rs`, `transform.rs`, `push_pull.rs` | split pieces, stencil copies and push/pull caps keep the placement | 1 |
| `crates/kernel/src/data.rs`, `scene.rs`, `edit.rs`, `triangles.rs`, `lib.rs` | `FaceData.texture_position`, validation, `KernelError::TexturePlacement`, placed `Triangle.uvs`, re-exports | 1 |
| `crates/kernel/tests/texture_placement.rs` (new), `crates/kernel/tests/triangles.rs` | kernel tests | 1 |
| `crates/session/src/command.rs`, `document.rs`, `document/placement.rs` (new), `query.rs`, `lib.rs` | `position_texture`, `Outcome.positioned`, `FaceInfo.texture_position` | 2 |
| `crates/session/src/file.rs` | schema 9, `LIGHTS_SCHEMA_VERSION` | 2 |
| `crates/session/tests/texture_position.rs` (new), `export.rs`, `lights.rs`, `crates/api/tests/dispatch.rs` | command, file and export tests | 2 |
| `crates/render/src/scene.rs` | placed positions, unmirrored back side, unit test | 3 |
| `crates/render/tests/golden.rs`, `crates/render/tests/golden/positioned.png` (new) | golden image of a turned texture and a back side | 3 |
| `crates/app/src/editor/texture_position.rs` (new), `editor.rs` | editor queries and setters, Rotate 90 batch | 4 |
| `crates/app/src/ui.rs`, `app.rs` | Entity Info fields and buttons, `UiAction`s, wiring, UI test | 4 |
| `docs/agent-api.md` | `position_texture`, Texture positions, files, export | 5 |

The code below was written and verified in a scratch worktree of `dev` at ff9d0af: after each task `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace` passed. Workspace test totals after each task: 1001, 1007, 1009, 1012, 1012 (from 992). Code for new files is given in full; changes to existing files are unified diffs against the state the previous task leaves. Test files are in Step 1; source files, including the unit tests inside them, are in Step 3, so where a task's tests use new code, expect the Step 2 run to fail to compile.

---

### Task 1: Kernel: texture placements on faces

**Files:**
- Modify: `crates/kernel/src/texture.rs`, `crates/kernel/src/model.rs`, `crates/kernel/src/faces.rs`, `crates/kernel/src/transform.rs`, `crates/kernel/src/push_pull.rs`, `crates/kernel/src/data.rs`, `crates/kernel/src/scene.rs`, `crates/kernel/src/edit.rs`, `crates/kernel/src/triangles.rs`, `crates/kernel/src/lib.rs`
- Test: `crates/kernel/tests/texture_placement.rs` (new), `crates/kernel/tests/triangles.rs`

**Interfaces:**
- Consumes: `texture_position`, `texture_axes` (M9b); `Model::record`, `take_changes`, `set_paint`, the stencil (`Stencil`, `StencilFace`, `draw_stencil`), `reconcile` (existing).
- Produces:
  - `kernel::TexturePlacement { offset: [f64; 2], rotation_deg: f64, scale: [f64; 2] }` (`Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema, Default`; `deny_unknown_fields`), `is_default(&self) -> bool`, `place(&self, p: DVec2) -> DVec2`, `pub(crate) turned_over(&self, level: bool) -> TexturePlacement`
  - `kernel::PlacementEdit { offset: Option<[f64; 2]>, rotation_deg: Option<f64>, scale: Option<[f64; 2]>, reset: bool }` (`Clone, Copy, Debug, Default, PartialEq`), `applied(&self, Option<TexturePlacement>) -> TexturePlacement`
  - `kernel::{MIN_TEXTURE_SCALE = 0.001, MAX_TEXTURE_SCALE = 1000.0, MAX_TEXTURE_OFFSET = 1.0e6}`, `kernel::normal_angle(deg: f64) -> f64`, `kernel::back_texture_position(front: DVec2) -> DVec2`
  - `Face.texture: Option<TexturePlacement>`, `Face::texture_position(&self, p: DVec3) -> DVec2`
  - `Op::PlaceTexture(FaceId, Option<TexturePlacement>, Option<TexturePlacement>)`, `pub(crate) Model::set_texture(&mut self, FaceId, Option<TexturePlacement>)`
  - `Model::position_texture(&mut self, faces: &[FaceId], edit: PlacementEdit) -> Result<(ChangeSet, usize), KernelError>`
  - `KernelError::TexturePlacement(String)`
  - `FaceData.texture_position: Option<TexturePlacement>` (serde default, skipped when none)
  - `Triangle.uvs` now the placed front positions

- [ ] **Step 1: Write the failing tests**

Create `crates/kernel/tests/texture_placement.rs`:

````rust
//! Texture placements on faces: the placed and back-side texture positions, the
//! `position_texture` edit, what faces keep through splits, copies, groups and push/pull, and
//! plain data.

use glam::{DVec2, DVec3};
use kernel::{
    DefinitionKind, Face, FaceId, KernelError, MeshId, Model, PlacementEdit, Scene, SceneData,
    TexturePlacement, back_texture_position, normal_angle,
};

fn p(x: f64, y: f64, z: f64) -> DVec3 {
    DVec3::new(x, y, z)
}

/// A 2 x 1 m rectangle on the ground; its face (normal +z or -z, as the kernel turns it).
fn ground() -> (Model, FaceId) {
    let mut m = Model::new();
    m.add_polygon(&[
        p(0.0, 0.0, 0.0),
        p(2.0, 0.0, 0.0),
        p(2.0, 1.0, 0.0),
        p(0.0, 1.0, 0.0),
    ])
    .unwrap();
    let f = m.faces().next().unwrap().0;
    (m, f)
}

fn placement(offset: [f64; 2], rotation_deg: f64, scale: [f64; 2]) -> TexturePlacement {
    TexturePlacement {
        offset,
        rotation_deg,
        scale,
    }
}

fn sets(t: TexturePlacement) -> PlacementEdit {
    PlacementEdit {
        offset: Some(t.offset),
        rotation_deg: Some(t.rotation_deg),
        scale: Some(t.scale),
        reset: false,
    }
}

#[test]
fn a_placement_moves_turns_and_scales_the_projection() {
    // Moved 1 m along u: the texture's origin is at u = 1.
    let moved = placement([1.0, 0.0], 0.0, [1.0, 1.0]);
    assert!(
        moved
            .place(DVec2::new(1.0, 0.0))
            .abs_diff_eq(DVec2::ZERO, 1e-12)
    );
    // Turned a quarter counter-clockwise: the image's width now runs along v.
    let turned = placement([0.0, 0.0], 90.0, [1.0, 1.0]);
    assert!(
        turned
            .place(DVec2::new(0.0, 1.0))
            .abs_diff_eq(DVec2::X, 1e-12)
    );
    assert!(
        turned
            .place(DVec2::new(-1.0, 0.0))
            .abs_diff_eq(DVec2::Y, 1e-12)
    );
    // Twice as wide: 2 m along u is one image width (in meters of the texture's own size).
    let wide = placement([0.0, 0.0], 0.0, [2.0, 1.0]);
    assert!(
        wide.place(DVec2::new(2.0, 3.0))
            .abs_diff_eq(DVec2::new(1.0, 3.0), 1e-12)
    );
    // The back side mirrors u.
    assert_eq!(
        back_texture_position(DVec2::new(1.5, 2.0)),
        DVec2::new(-1.5, 2.0)
    );
    assert_eq!(normal_angle(-90.0), 270.0);
    assert_eq!(normal_angle(450.0), 90.0);
    assert_eq!(normal_angle(-1e-20), 0.0);
    assert!(normal_angle(-0.0).is_sign_positive());
}

#[test]
fn turning_a_face_over_keeps_what_each_side_shows() {
    let t = placement([0.3, -0.7], 30.0, [2.0, 0.5]);
    // A level face and a sloping one.
    for normal in [DVec3::Z, p(0.0, -1.0, 1.0).normalize()] {
        let mut face = Face::new(Vec::new(), Vec::new(), normal);
        face.texture = Some(t);
        let flipped = face.reversed();
        for q in [p(0.0, 0.0, 0.0), p(1.0, 2.0, 0.5), p(-3.0, 0.25, 4.0)] {
            let (front, back) = (
                face.texture_position(q),
                back_texture_position(face.texture_position(q)),
            );
            let flipped_front = flipped.texture_position(q);
            assert!(flipped_front.abs_diff_eq(back, 1e-9), "{normal} {q}");
            assert!(
                back_texture_position(flipped_front).abs_diff_eq(front, 1e-9),
                "{normal} {q}"
            );
        }
        assert_eq!(flipped.reversed().texture, Some(t));
    }
}

#[test]
fn position_texture_sets_the_given_fields_and_undo_restores_them() {
    let (mut m, f) = ground();
    let before = m.clone();
    let edit = PlacementEdit {
        rotation_deg: Some(-90.0),
        ..PlacementEdit::default()
    };
    let (turn, n) = m.position_texture(&[f], edit).unwrap();
    assert_eq!(n, 1);
    assert_eq!(
        m.face(f).unwrap().texture,
        Some(placement([0.0, 0.0], 270.0, [1.0, 1.0]))
    );
    // Other fields are kept.
    let edit = PlacementEdit {
        offset: Some([0.5, 0.25]),
        ..PlacementEdit::default()
    };
    let (moved, _) = m.position_texture(&[f], edit).unwrap();
    assert_eq!(
        m.face(f).unwrap().texture,
        Some(placement([0.5, 0.25], 270.0, [1.0, 1.0]))
    );
    // The same again changes nothing.
    let (none, n) = m.position_texture(&[f], edit).unwrap();
    assert!(none.is_empty());
    assert_eq!(n, 0);
    // Reset, then the fields given.
    let reset = PlacementEdit {
        scale: Some([2.0, 2.0]),
        reset: true,
        ..PlacementEdit::default()
    };
    m.position_texture(&[f], reset).unwrap();
    assert_eq!(
        m.face(f).unwrap().texture,
        Some(placement([0.0, 0.0], 0.0, [2.0, 2.0]))
    );
    // A placement back at the default is stored as none.
    let (cleared, n) = m
        .position_texture(
            &[f],
            PlacementEdit {
                reset: true,
                ..PlacementEdit::default()
            },
        )
        .unwrap();
    assert_eq!((m.face(f).unwrap().texture, n), (None, 1));
    m.apply(&cleared.inverse());
    assert!(m.face(f).unwrap().texture.is_some());
    let mut undone = before.clone();
    undone.apply(&turn);
    undone.apply(&moved);
    undone.apply(&moved.inverse());
    undone.apply(&turn.inverse());
    assert_eq!(undone, before);
    m.validate().unwrap();
}

#[test]
fn out_of_range_placements_are_refused_and_change_nothing() {
    let (mut m, f) = ground();
    m.position_texture(&[f], sets(placement([1.0, 1.0], 45.0, [1.0, 1.0])))
        .unwrap();
    let before = m.clone();
    for bad in [
        PlacementEdit {
            offset: Some([f64::NAN, 0.0]),
            ..PlacementEdit::default()
        },
        PlacementEdit {
            offset: Some([0.0, 2.0e6]),
            ..PlacementEdit::default()
        },
        PlacementEdit {
            rotation_deg: Some(f64::INFINITY),
            ..PlacementEdit::default()
        },
        PlacementEdit {
            scale: Some([0.0, 1.0]),
            ..PlacementEdit::default()
        },
        PlacementEdit {
            scale: Some([1.0, -2.0]),
            ..PlacementEdit::default()
        },
        PlacementEdit {
            scale: Some([1.0, 1001.0]),
            ..PlacementEdit::default()
        },
    ] {
        let err = m.position_texture(&[f], bad).unwrap_err();
        assert!(
            matches!(&err, KernelError::TexturePlacement(why) if why.contains("texture")),
            "{bad:?}: {err:?}"
        );
        assert_eq!(m, before);
    }
}

#[test]
fn split_pieces_keep_the_placement_and_new_faces_have_none() {
    let (mut m, f) = ground();
    let t = placement([0.1, 0.2], 30.0, [0.5, 0.5]);
    m.position_texture(&[f], sets(t)).unwrap();
    // A line across splits the face in two; both keep the placement.
    m.add_edge(p(1.0, 0.0, 0.0), p(1.0, 1.0, 0.0)).unwrap();
    assert_eq!(m.faces().count(), 2);
    assert!(m.faces().all(|(_, g)| g.texture == Some(t)));
    // A new rectangle apart from it has none.
    m.add_polygon(&[
        p(5.0, 0.0, 0.0),
        p(6.0, 0.0, 0.0),
        p(6.0, 1.0, 0.0),
        p(5.0, 1.0, 0.0),
    ])
    .unwrap();
    assert_eq!(m.faces().filter(|(_, g)| g.texture.is_none()).count(), 1);
    m.validate().unwrap();
}

#[test]
fn copies_keep_the_placement_and_push_pull_caps_keep_it_too() {
    let (mut m, f) = ground();
    let t = placement([0.0, 0.0], 90.0, [1.0, 2.0]);
    m.position_texture(&[f], sets(t)).unwrap();
    let edges: Vec<_> = m.edges().map(|(id, _)| id).collect();
    m.copy(&edges, &[f], p(0.0, 5.0, 0.0), 2).unwrap();
    assert_eq!(m.faces().count(), 3);
    assert!(m.faces().all(|(_, g)| g.texture == Some(t)));

    // Pulled up into a box: the cap keeps the placement (turned over with the cap when the
    // sheet turns over), the sides have none.
    let (mut m, f) = ground();
    m.position_texture(&[f], sets(t)).unwrap();
    let face = m.face(f).unwrap().clone();
    m.push_pull(f, -face.normal.z.signum()).unwrap();
    let placed: Vec<&Face> = m
        .faces()
        .map(|(_, g)| g)
        .filter(|g| g.texture.is_some())
        .collect();
    assert_eq!(placed.len(), 2, "the base and the cap");
    assert_eq!(m.faces().filter(|(_, g)| g.texture.is_none()).count(), 4);
    m.validate().unwrap();
}

#[test]
fn groups_explode_and_definitions_keep_placements() {
    let (m, f) = ground();
    let mut s = Scene::from_root(m);
    let t = placement([0.5, 0.0], 0.0, [3.0, 3.0]);
    s.edit(MeshId::Root, |m| Ok(m.position_texture(&[f], sets(t))?.0))
        .unwrap();
    let edges: Vec<_> = s.root().edges().map(|(id, _)| id).collect();
    let (instance, _) = s
        .make_group(
            MeshId::Root,
            &edges,
            &[f],
            &[],
            DefinitionKind::Component,
            "Tile",
        )
        .unwrap();
    let (_, d) = s.definitions().next().unwrap();
    assert!(d.mesh.faces().all(|(_, g)| g.texture == Some(t)));
    s.explode(MeshId::Root, instance).unwrap();
    assert!(s.root().faces().all(|(_, g)| g.texture == Some(t)));
    s.validate().unwrap();
}

#[test]
fn placements_are_kept_in_plain_data_and_checked_on_load() {
    let (m, f) = ground();
    let mut s = Scene::from_root(m);
    let t = placement([0.25, -1.5], 22.5, [0.5, 4.0]);
    s.edit(MeshId::Root, |m| Ok(m.position_texture(&[f], sets(t))?.0))
        .unwrap();
    let data = s.to_data();
    let json = serde_json::to_string(&data).unwrap();
    assert!(
        json.contains(
            r#""texture_position":{"offset":[0.25,-1.5],"rotation_deg":22.5,"scale":[0.5,4.0]}"#
        ),
        "{json}"
    );
    let back: SceneData = serde_json::from_str(&json).unwrap();
    assert_eq!(Scene::from_data(&back).unwrap(), s);

    // Faces written before placements existed have none; a default placement reads as none.
    let mut old = data.clone();
    old.root.faces[0].texture_position = None;
    let plain = Scene::from_data(&old).unwrap();
    assert!(plain.root().faces().all(|(_, g)| g.texture.is_none()));
    let mut default = data.clone();
    default.root.faces[0].texture_position = Some(TexturePlacement::default());
    let read = Scene::from_data(&default).unwrap();
    assert!(read.root().faces().all(|(_, g)| g.texture.is_none()));

    for bad in [
        placement([0.0, 0.0], 360.0, [1.0, 1.0]),
        placement([0.0, 0.0], 0.0, [0.0, 1.0]),
        placement([f64::INFINITY, 0.0], 0.0, [1.0, 1.0]),
    ] {
        let mut d = data.clone();
        d.root.faces[0].texture_position = Some(bad);
        assert!(
            Scene::from_data(&d).unwrap_err().contains("texture"),
            "{bad:?}"
        );
    }
}

#[test]
fn triangles_carry_placed_texture_positions() {
    let (m, f) = ground();
    let mut s = Scene::from_root(m);
    let t = placement([1.0, 0.0], 90.0, [1.0, 1.0]);
    s.edit(MeshId::Root, |m| Ok(m.position_texture(&[f], sets(t))?.0))
        .unwrap();
    let face = s.root().face(f).unwrap().clone();
    for tri in s.triangles() {
        for (c, uv) in tri.corners.iter().zip(tri.uvs) {
            assert!(uv.abs_diff_eq(face.texture_position(*c), 1e-12));
        }
    }
    // On a level face u runs along x: the corner at x = 2, y = 0 sits 1 m past the moved
    // origin along u, which a quarter turn of the texture puts 1 m down its height.
    let corner = p(2.0, 0.0, 0.0);
    assert!(
        face.texture_position(corner)
            .abs_diff_eq(DVec2::new(0.0, -1.0), 1e-12)
    );
}
````

Apply to `crates/kernel/tests/triangles.rs`:

````diff
diff --git a/crates/kernel/tests/triangles.rs b/crates/kernel/tests/triangles.rs
index a731371..364ec20 100644
--- a/crates/kernel/tests/triangles.rs
+++ b/crates/kernel/tests/triangles.rs
@@ -195,6 +195,7 @@ fn mesh_with_an_empty_face(first: u64, x: f64) -> kernel::MeshData {
         normal: [0.0, 0.0, 1.0],
         front: None,
         back: None,
+        texture_position: None,
     };
     MeshData {
         vertices: vec![
````

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p kernel --test texture_placement`
Expected: does not compile (`TexturePlacement`, `PlacementEdit`, `position_texture`, `texture_position` on `FaceData` unknown). `crates/kernel/tests/triangles.rs` fails to compile the same way until Step 3 adds the field.

- [ ] **Step 3: Implement**

`crates/kernel/src/texture.rs` gets the placement type, its checks and the edit; `model.rs` the face field and its op; `faces.rs`, `transform.rs` and `push_pull.rs` pass the placement on where paint already passes.

Apply to `crates/kernel/src/data.rs`:

````diff
diff --git a/crates/kernel/src/data.rs b/crates/kernel/src/data.rs
index 3634aea..c06068b 100644
--- a/crates/kernel/src/data.rs
+++ b/crates/kernel/src/data.rs
@@ -12,7 +12,7 @@ use crate::ids::{
 use crate::material::Material;
 use crate::model::{Edge, Face, Guide, Instance, Model, Vertex};
 use crate::scene::{Definition, DefinitionKind, Scene};
-use crate::texture::{Texture, TextureImage};
+use crate::texture::{Texture, TextureImage, TexturePlacement};
 
 /// Ids in a file are below this, and its `next_id` is at most this, so every id is exact as a
 /// JSON (double) number and a scene that loads also saves and loads again.
@@ -124,6 +124,9 @@ pub struct FaceData {
     pub front: Option<u64>,
     #[serde(default, skip_serializing_if = "Option::is_none")]
     pub back: Option<u64>,
+    /// The texture placement (absent: none, as in files written before placements existed).
+    #[serde(default, skip_serializing_if = "Option::is_none")]
+    pub texture_position: Option<TexturePlacement>,
 }
 
 #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
@@ -280,6 +283,7 @@ fn mesh_data(m: &Model) -> MeshData {
                 normal: f.normal.to_array(),
                 front: f.front.map(|m| m.0),
                 back: f.back.map(|m| m.0),
+                texture_position: f.texture,
             })
             .collect(),
         instances: m
@@ -363,6 +367,8 @@ fn mesh(data: &MeshData) -> Result<Model, String> {
             normal: DVec3::from(f.normal),
             front: f.front.map(MaterialId),
             back: f.back.map(MaterialId),
+            // A default placement is the same as none.
+            texture: f.texture_position.filter(|t| !t.is_default()),
         };
         if faces.insert(FaceId(f.id), face).is_some() {
             return Err(format!("face f{} appears twice", f.id));
````

Apply to `crates/kernel/src/edit.rs`:

````diff
diff --git a/crates/kernel/src/edit.rs b/crates/kernel/src/edit.rs
index 137b33f..5fa7090 100644
--- a/crates/kernel/src/edit.rs
+++ b/crates/kernel/src/edit.rs
@@ -39,6 +39,8 @@ pub enum KernelError {
     /// A material emission is not a finite number from 0 to `MAX_EMISSION`; the text says
     /// which.
     MaterialEmission(String),
+    /// A texture placement's offset, rotation or scale is out of range; the text says which.
+    TexturePlacement(String),
     /// An import has no polygon that makes a face.
     NothingToImport,
     /// An import would make more faces than `MAX_IMPORT_FACES`.
@@ -74,7 +76,8 @@ impl fmt::Display for KernelError {
             KernelError::MaterialName(why)
             | KernelError::MaterialOpacity(why)
             | KernelError::MaterialTexture(why)
-            | KernelError::MaterialEmission(why) => {
+            | KernelError::MaterialEmission(why)
+            | KernelError::TexturePlacement(why) => {
                 write!(f, "{why}")
             }
             KernelError::NothingToImport => {
````

Apply to `crates/kernel/src/faces.rs`:

````diff
diff --git a/crates/kernel/src/faces.rs b/crates/kernel/src/faces.rs
index 0862187..8bf891f 100644
--- a/crates/kernel/src/faces.rs
+++ b/crates/kernel/src/faces.rs
@@ -11,6 +11,7 @@ use crate::ids::{EdgeId, FaceId, MaterialId, VertexId};
 use crate::index::{Index, Shape};
 use crate::model::{Face, Model};
 use crate::regions::{PlaneGraph, Region};
+use crate::texture::TexturePlacement;
 
 impl Model {
     /// Updates the faces of every plane spanned by one of `new_edges` and an edge touching it.
@@ -211,7 +212,9 @@ impl Model {
                 });
                 let keep = in_old_face || (uses_new && (!in_old_hole || apart));
                 let normal = container.map_or(plane.normal, |i| old[i].1.normal);
-                let paint = container.map_or([None, None], |i| old[i].1.paint());
+                let paint = container.map_or(([None, None], None), |i| {
+                    (old[i].1.paint(), old[i].1.texture)
+                });
                 keep.then_some((r, normal, paint))
             })
             .collect();
@@ -228,7 +231,7 @@ impl Model {
         plane: Plane,
     ) {
         let mut matched: BTreeSet<FaceId> = BTreeSet::new();
-        for (r, normal, [front, back]) in wanted {
+        for (r, normal, ([front, back], texture)) in wanted {
             let outer: BTreeSet<VertexId> = r.outer.iter().copied().collect();
             // Faces in one plane may face opposite ways (push/pull flips a base face), so an
             // old face matches by vertex set and keeps its own orientation.
@@ -266,6 +269,7 @@ impl Model {
                         normal,
                         front,
                         back,
+                        texture,
                     });
                 }
             }
@@ -278,8 +282,9 @@ impl Model {
     }
 }
 
-/// A face's front and back materials.
-type Paint = [Option<MaterialId>; 2];
+/// A face's front and back materials and its texture placement, which the pieces split from it
+/// keep.
+type Paint = ([Option<MaterialId>; 2], Option<TexturePlacement>);
 
 /// Above this many planes, `create_faces` finds each plane's edges with an index.
 const FEW_PLANES: usize = 4;
````

Apply to `crates/kernel/src/lib.rs`:

````diff
diff --git a/crates/kernel/src/lib.rs b/crates/kernel/src/lib.rs
index 1d69211..4e3a58a 100644
--- a/crates/kernel/src/lib.rs
+++ b/crates/kernel/src/lib.rs
@@ -42,7 +42,8 @@ pub use scene::{Definition, DefinitionKind, MeshId, Scene, SceneChange, Step};
 pub use smooth::{ShadedTriangle, SmoothNormals};
 pub use soften::SoftenReport;
 pub use texture::{
-    ImageFormat, MAX_TEXTURE_SIZE, MAX_TEXTURES, MIN_TEXTURE_SIZE, Texture, TextureImage,
-    texture_axes, texture_position,
+    ImageFormat, MAX_TEXTURE_OFFSET, MAX_TEXTURE_SCALE, MAX_TEXTURE_SIZE, MAX_TEXTURES,
+    MIN_TEXTURE_SCALE, MIN_TEXTURE_SIZE, PlacementEdit, Texture, TextureImage, TexturePlacement,
+    back_texture_position, normal_angle, texture_axes, texture_position,
 };
 pub use triangles::Triangle;
````

Apply to `crates/kernel/src/model.rs`:

````diff
diff --git a/crates/kernel/src/model.rs b/crates/kernel/src/model.rs
index b1b5acb..1fe6cfd 100644
--- a/crates/kernel/src/model.rs
+++ b/crates/kernel/src/model.rs
@@ -10,6 +10,7 @@ use crate::ids::{
     CurveId, DefinitionId, EdgeId, EntityId, FaceId, GuideId, InstanceId, MaterialId, VertexId,
 };
 use crate::index::Index;
+use crate::texture::{TexturePlacement, is_level};
 
 #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
 pub struct Vertex {
@@ -53,6 +54,10 @@ pub struct Face {
     /// The material of the other side.
     #[serde(default, skip_serializing_if = "Option::is_none")]
     pub back: Option<MaterialId>,
+    /// Where the texture lies on top of the projection; None for the projection as it is.
+    /// Never the default placement.
+    #[serde(default, skip_serializing_if = "Option::is_none")]
+    pub texture: Option<TexturePlacement>,
 }
 
 impl Face {
@@ -64,11 +69,12 @@ impl Face {
             normal,
             front: None,
             back: None,
+            texture: None,
         }
     }
 
-    /// The same face turned over: loops reversed, normal negated, and the materials swapped so
-    /// each stays on the side it was on.
+    /// The same face turned over: loops reversed, normal negated, and the materials swapped and
+    /// the texture placement turned over so each side looks as it did.
     pub fn reversed(&self) -> Face {
         let rev = |l: &Vec<VertexId>| l.iter().rev().copied().collect::<Vec<_>>();
         Face {
@@ -77,6 +83,7 @@ impl Face {
             normal: -self.normal,
             front: self.back,
             back: self.front,
+            texture: self.texture.map(|t| t.turned_over(is_level(self.normal))),
         }
     }
 
@@ -164,6 +171,8 @@ pub enum Op {
     PaintFace(FaceId, [Option<MaterialId>; 2], [Option<MaterialId>; 2]),
     /// An instance's material changed from the first to the second.
     PaintInstance(InstanceId, Option<MaterialId>, Option<MaterialId>),
+    /// A face's texture placement changed from the first to the second.
+    PlaceTexture(FaceId, Option<TexturePlacement>, Option<TexturePlacement>),
 }
 
 impl Op {
@@ -184,6 +193,7 @@ impl Op {
             Op::RemoveGuide(id, g) => Op::AddGuide(id, g),
             Op::PaintFace(id, from, to) => Op::PaintFace(id, to, from),
             Op::PaintInstance(id, from, to) => Op::PaintInstance(id, to, from),
+            Op::PlaceTexture(id, from, to) => Op::PlaceTexture(id, to, from),
         }
     }
 
@@ -205,6 +215,7 @@ impl Op {
             Op::RemoveGuide(id, _) => (EntityId::Guide(*id), true, false),
             Op::PaintFace(id, _, _) => (EntityId::Face(*id), true, true),
             Op::PaintInstance(id, _, _) => (EntityId::Instance(*id), true, true),
+            Op::PlaceTexture(id, _, _) => (EntityId::Face(*id), true, true),
         }
     }
 }
@@ -605,6 +616,11 @@ impl Model {
                     i.material = to;
                 }
             }
+            Op::PlaceTexture(id, _, to) => {
+                if let Some(f) = self.faces.get_mut(&id) {
+                    f.texture = to;
+                }
+            }
         }
     }
 
@@ -846,6 +862,16 @@ impl Model {
         }
     }
 
+    /// Sets a face's texture placement (a default placement is stored as none), recording a
+    /// change only when it differs.
+    pub(crate) fn set_texture(&mut self, id: FaceId, texture: Option<TexturePlacement>) {
+        let texture = texture.filter(|t| !t.is_default());
+        let from = self.faces[&id].texture;
+        if from != texture {
+            self.record(Op::PlaceTexture(id, from, texture));
+        }
+    }
+
     /// Sets an instance's material, recording a change only when it differs.
     pub(crate) fn set_instance_material(&mut self, id: InstanceId, material: Option<MaterialId>) {
         let from = self.instances[&id].material;
````

Apply to `crates/kernel/src/push_pull.rs`:

````diff
diff --git a/crates/kernel/src/push_pull.rs b/crates/kernel/src/push_pull.rs
index 37eea40..abb3a98 100644
--- a/crates/kernel/src/push_pull.rs
+++ b/crates/kernel/src/push_pull.rs
@@ -256,10 +256,12 @@ impl Model {
         let cap_origin = points[0][0] + offset;
         // A flat sheet pulled against its normal turns over: the side facing the pull direction
         // is the one that ends up outside, so the cap and sides take the materials swapped.
-        let new_paint = if cap_normal == -n {
-            face.reversed().paint()
+        // The cap is the pushed face moved: it keeps the texture placement too.
+        let (new_paint, cap_texture) = if cap_normal == -n {
+            let r = face.reversed();
+            (r.paint(), r.texture)
         } else {
-            face.paint()
+            (face.paint(), face.texture)
         };
         let created: Vec<FaceId> = self
             .faces()
@@ -282,6 +284,7 @@ impl Model {
             if in_cap {
                 self.orient(f, cap_normal);
                 self.set_paint(f, new_paint);
+                self.set_texture(f, cap_texture);
                 continue;
             }
             // A side face lies inside a side's swept rectangle. Other new faces in a side's
````

Apply to `crates/kernel/src/scene.rs`:

````diff
diff --git a/crates/kernel/src/scene.rs b/crates/kernel/src/scene.rs
index 6f8f280..c922d11 100644
--- a/crates/kernel/src/scene.rs
+++ b/crates/kernel/src/scene.rs
@@ -441,8 +441,8 @@ impl Scene {
     /// itself (directly or deeper), that no number is the id of two things (entities in any
     /// mesh, curves, definitions and materials all take ids from one counter), that every
     /// material painted exists, that material names are valid and distinct, that material
-    /// opacities are from 0 to 1 and emissions from 0 to `MAX_EMISSION`, and that textures are
-    /// valid and at most `MAX_TEXTURES`.
+    /// opacities are from 0 to 1 and emissions from 0 to `MAX_EMISSION`, that textures are
+    /// valid and at most `MAX_TEXTURES`, and that face texture placements are in range.
     pub fn validate(&self) -> Result<(), String> {
         let mut names = BTreeSet::new();
         for (id, m) in &self.materials {
@@ -496,6 +496,9 @@ impl Scene {
                 if missing(&f.front) || missing(&f.back) {
                     return Err(format!("face {id:?} is painted with a missing material"));
                 }
+                if let Some(t) = &f.texture {
+                    crate::texture::check_placement(t).map_err(|e| format!("face {id:?}: {e}"))?;
+                }
             }
             for id in entity_ids(m) {
                 let (prefix, n) = match id {
````

Apply to `crates/kernel/src/texture.rs`:

````diff
diff --git a/crates/kernel/src/texture.rs b/crates/kernel/src/texture.rs
index 2b03b3d..6eb6bed 100644
--- a/crates/kernel/src/texture.rs
+++ b/crates/kernel/src/texture.rs
@@ -3,13 +3,20 @@
 //! A face shows its material's texture laid flat in its own plane, in the coordinates of the
 //! definition holding it: the image's width runs along the face's horizontal direction (x on
 //! level faces) and its height across, repeating every `width` by `height` meters. Faces in one
-//! plane continue each other's pattern, and a component carries its texture with it.
+//! plane continue each other's pattern, and a component carries its texture with it. A face's
+//! placement moves, turns and scales its texture on top of that, and the back side shows the
+//! texture as seen from behind.
 
+use std::collections::BTreeSet;
 use std::sync::Arc;
 
 use glam::{DVec2, DVec3};
 use serde::{Deserialize, Serialize};
 
+use crate::edit::KernelError;
+use crate::ids::FaceId;
+use crate::model::{ChangeSet, Face, Model};
+
 /// The most materials with a texture a scene holds.
 pub const MAX_TEXTURES: usize = 256;
 /// A texture's least width and height on the model, in meters.
@@ -93,11 +100,10 @@ pub(crate) fn check_texture(t: &Texture) -> Result<(), String> {
 /// u is horizontal, or x on a level face; v lies in the face across u, so that u, v and the
 /// normal turn right-handed (v points up on a wall).
 pub fn texture_axes(normal: DVec3) -> (DVec3, DVec3) {
-    let across = DVec3::Z.cross(normal);
-    let u = if across.length() < LEVEL {
+    let u = if is_level(normal) {
         DVec3::X
     } else {
-        across.normalize()
+        DVec3::Z.cross(normal).normalize()
     };
     (u, normal.cross(u).normalize_or_zero())
 }
@@ -109,3 +115,194 @@ pub fn texture_position(normal: DVec3, p: DVec3) -> DVec2 {
     let (u, v) = texture_axes(normal);
     DVec2::new(p.dot(u), p.dot(v))
 }
+
+/// The least and greatest scale a texture placement takes along u or v.
+pub const MIN_TEXTURE_SCALE: f64 = 0.001;
+pub const MAX_TEXTURE_SCALE: f64 = 1000.0;
+/// The farthest a texture placement moves the texture along u or v, in meters.
+pub const MAX_TEXTURE_OFFSET: f64 = 1.0e6;
+
+/// Where a face's texture lies on top of the projection (see [`texture_position`]): moved by
+/// `offset` meters along u and v, turned `rotation_deg` counter-clockwise seen from the face's
+/// front about the moved origin, and `scale` times as large along its own width and height.
+#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
+#[serde(deny_unknown_fields)]
+pub struct TexturePlacement {
+    pub offset: [f64; 2],
+    /// From 0 up to (not including) 360.
+    pub rotation_deg: f64,
+    pub scale: [f64; 2],
+}
+
+impl Default for TexturePlacement {
+    fn default() -> Self {
+        TexturePlacement {
+            offset: [0.0, 0.0],
+            rotation_deg: 0.0,
+            scale: [1.0, 1.0],
+        }
+    }
+}
+
+impl TexturePlacement {
+    /// True for the placement that leaves the projection as it is.
+    pub fn is_default(&self) -> bool {
+        *self == TexturePlacement::default()
+    }
+
+    /// A projected texture position (meters along u and v) as a position on the placed texture:
+    /// meters along the image's width and height, as if it had its own size.
+    pub fn place(&self, p: DVec2) -> DVec2 {
+        let (sin, cos) = self.rotation_deg.to_radians().sin_cos();
+        let q = p - DVec2::from(self.offset);
+        // Turning the texture one way turns the points on it the other.
+        let turned = DVec2::new(cos * q.x + sin * q.y, cos * q.y - sin * q.x);
+        turned / DVec2::from(self.scale)
+    }
+
+    /// The placement that shows the same texture on each side once the face is turned over
+    /// (its normal negated): the projection's u flips on a sloping face, v on a level one
+    /// (see [`texture_axes`]), and the back mirrors u (see [`back_texture_position`]).
+    pub(crate) fn turned_over(&self, level: bool) -> TexturePlacement {
+        let [x, y] = self.offset;
+        if level {
+            TexturePlacement {
+                offset: [x, -y],
+                rotation_deg: normal_angle(-self.rotation_deg - 180.0),
+                scale: self.scale,
+            }
+        } else {
+            TexturePlacement {
+                offset: [-x, y],
+                rotation_deg: normal_angle(-self.rotation_deg),
+                scale: self.scale,
+            }
+        }
+    }
+}
+
+/// An angle in degrees brought into 0 up to (not including) 360.
+pub fn normal_angle(deg: f64) -> f64 {
+    let a = deg.rem_euclid(360.0) + 0.0;
+    if a >= 360.0 { 0.0 } else { a }
+}
+
+/// Fails, saying why, when a placement's numbers are not finite, its offset is beyond
+/// [`MAX_TEXTURE_OFFSET`], its rotation is not from 0 up to 360 or a scale is not from
+/// [`MIN_TEXTURE_SCALE`] to [`MAX_TEXTURE_SCALE`].
+pub(crate) fn check_placement(t: &TexturePlacement) -> Result<(), String> {
+    if !t
+        .offset
+        .iter()
+        .all(|x| x.is_finite() && x.abs() <= MAX_TEXTURE_OFFSET)
+    {
+        return Err(format!(
+            "a texture offset must be from -{MAX_TEXTURE_OFFSET} to {MAX_TEXTURE_OFFSET} m, not \
+             {:?}",
+            t.offset
+        ));
+    }
+    if !(t.rotation_deg.is_finite() && (0.0..360.0).contains(&t.rotation_deg)) {
+        return Err(format!(
+            "a texture rotation must be from 0 up to 360 degrees, not {}",
+            t.rotation_deg
+        ));
+    }
+    if !t
+        .scale
+        .iter()
+        .all(|s| s.is_finite() && (MIN_TEXTURE_SCALE..=MAX_TEXTURE_SCALE).contains(s))
+    {
+        return Err(format!(
+            "a texture scale must be from {MIN_TEXTURE_SCALE} to {MAX_TEXTURE_SCALE}, not {:?}",
+            t.scale
+        ));
+    }
+    Ok(())
+}
+
+/// True when a face with unit `normal` lays its texture along x (see [`texture_axes`]).
+pub(crate) fn is_level(normal: DVec3) -> bool {
+    DVec3::Z.cross(normal).length() < LEVEL
+}
+
+/// Where a point lies on the back side's texture, from where it lies on the front's: u
+/// mirrored, so the back reads as the image seen from behind, not mirrored.
+pub fn back_texture_position(front: DVec2) -> DVec2 {
+    DVec2::new(-front.x, front.y)
+}
+
+impl Face {
+    /// Where point `p` of the face lies on its front's texture, in meters along the texture's
+    /// width and height: the projection (see [`texture_position`]) with the face's placement
+    /// on top. The back side's is [`back_texture_position`] of it.
+    pub fn texture_position(&self, p: DVec3) -> DVec2 {
+        let projected = texture_position(self.normal, p);
+        match &self.texture {
+            Some(t) => t.place(projected),
+            None => projected,
+        }
+    }
+}
+
+/// What [`Model::position_texture`] sets on each face: the fields given, on top of the face's
+/// placement or, with `reset`, of none; absent fields stay as they are.
+#[derive(Clone, Copy, Debug, Default, PartialEq)]
+pub struct PlacementEdit {
+    pub offset: Option<[f64; 2]>,
+    /// Any finite angle; stored from 0 up to 360.
+    pub rotation_deg: Option<f64>,
+    pub scale: Option<[f64; 2]>,
+    pub reset: bool,
+}
+
+impl PlacementEdit {
+    /// `placement` with this edit applied.
+    pub fn applied(&self, placement: Option<TexturePlacement>) -> TexturePlacement {
+        let mut t = if self.reset {
+            TexturePlacement::default()
+        } else {
+            placement.unwrap_or_default()
+        };
+        if let Some(offset) = self.offset {
+            t.offset = offset;
+        }
+        if let Some(r) = self.rotation_deg {
+            t.rotation_deg = normal_angle(r);
+        }
+        if let Some(scale) = self.scale {
+            t.scale = scale;
+        }
+        t
+    }
+}
+
+impl Model {
+    /// Sets the texture placement of `faces` as one ChangeSet, returning how many changed.
+    /// Unknown ids are ignored. Fails, changing nothing, when a given field is out of range
+    /// (see [`TexturePlacement`]).
+    pub fn position_texture(
+        &mut self,
+        faces: &[FaceId],
+        edit: PlacementEdit,
+    ) -> Result<(ChangeSet, usize), KernelError> {
+        check_placement(
+            &PlacementEdit {
+                reset: true,
+                ..edit
+            }
+            .applied(None),
+        )
+        .map_err(KernelError::TexturePlacement)?;
+        let mut changed = 0;
+        for &id in faces.iter().collect::<BTreeSet<_>>() {
+            let Some(face) = self.face(id) else { continue };
+            let to = Some(edit.applied(face.texture)).filter(|t| !t.is_default());
+            if to != face.texture {
+                self.set_texture(id, to);
+                changed += 1;
+            }
+        }
+        Ok((self.take_changes(), changed))
+    }
+}
````

Apply to `crates/kernel/src/transform.rs`:

````diff
diff --git a/crates/kernel/src/transform.rs b/crates/kernel/src/transform.rs
index 6bf2f94..5327d0e 100644
--- a/crates/kernel/src/transform.rs
+++ b/crates/kernel/src/transform.rs
@@ -11,6 +11,7 @@ use crate::geom::{
 use crate::ids::{CurveId, EdgeId, FaceId, MaterialId, VertexId};
 use crate::index::{Index, Shape};
 use crate::model::{ChangeSet, Face, Model};
+use crate::texture::TexturePlacement;
 
 impl Model {
     /// Moves `vertices` by `offset` as one ChangeSet. Faces that use some of them stretch and
@@ -210,6 +211,7 @@ impl Model {
                 outer: ring(&f.outer),
                 normal: f.normal,
                 paint: f.paint(),
+                texture: f.texture,
                 openings: Vec::new(),
                 filled: Vec::new(),
             };
@@ -345,13 +347,15 @@ impl Model {
                 })
             })
             .collect();
-        // Give each piece of a stencil face that face's orientation and materials.
+        // Give each piece of a stencil face that face's orientation, materials and texture
+        // placement.
         for (f, t) in placed.iter().zip(&placed_tests) {
             for id in candidates(&t.outer) {
                 let in_hole = t.openings.iter().chain(&t.filled).any(|h| within(id, h));
                 if within(id, &t.outer) && !in_hole {
                     self.orient(id, f.normal);
                     self.set_paint(id, f.paint);
+                    self.set_texture(id, f.texture);
                 }
             }
         }
@@ -475,6 +479,8 @@ struct StencilFace {
     normal: DVec3,
     /// Front and back materials.
     paint: [Option<MaterialId>; 2],
+    /// The texture placement, kept as it is wherever the face is drawn.
+    texture: Option<TexturePlacement>,
     /// Holes no face fills.
     openings: Vec<Vec<DVec3>>,
     /// Holes another face fills.
@@ -488,6 +494,7 @@ impl StencilFace {
             outer: ring(&self.outer),
             normal: t.transform_vector3(self.normal).normalize(),
             paint: self.paint,
+            texture: self.texture,
             openings: self.openings.iter().map(ring).collect(),
             filled: self.filled.iter().map(ring).collect(),
         }
````

Apply to `crates/kernel/src/triangles.rs`:

````diff
diff --git a/crates/kernel/src/triangles.rs b/crates/kernel/src/triangles.rs
index 9b5d18d..ab40387 100644
--- a/crates/kernel/src/triangles.rs
+++ b/crates/kernel/src/triangles.rs
@@ -4,7 +4,6 @@ use glam::{DAffine3, DVec2, DVec3};
 
 use crate::ids::MaterialId;
 use crate::scene::{MeshId, Scene};
-use crate::texture::texture_position;
 
 /// One triangle of a face, wound counter-clockwise around `normal` (the face's front).
 #[derive(Clone, Copy, Debug, PartialEq)]
@@ -14,8 +13,9 @@ pub struct Triangle {
     /// The normal at each corner: the face's own, or across soft edges the smooth one (see
     /// `Model::corner_normal`).
     pub normals: [DVec3; 3],
-    /// Where each corner lies on the face's texture, in meters (see `texture_position`): in the
-    /// plane of the face as its definition holds it, so a placed copy carries its pattern.
+    /// Where each corner lies on the front's texture, in meters (see `Face::texture_position`:
+    /// the projection with the face's placement on top): in the plane of the face as its
+    /// definition holds it, so a placed copy carries its pattern.
     pub uvs: [DVec2; 3],
     /// The material shown on the front and on the back: the face's own, or else that of the
     /// nearest painted instance placing it; None for the default material.
@@ -72,7 +72,7 @@ impl Scene {
             for tri in tris {
                 let [a, b, c] = tri.corners.map(|p| place.transform_point3(p));
                 let [na, nb, nc] = tri.normals.map(|n| (normal_map * n).normalize());
-                let [ua, ub, uc] = tri.corners.map(|p| texture_position(face.normal, p));
+                let [ua, ub, uc] = tri.corners.map(|p| face.texture_position(p));
                 out.push(Triangle {
                     corners: if mirrored { [a, c, b] } else { [a, b, c] },
                     normal,
````

- [ ] **Step 4: Run the tests**

Run: `cargo test -p kernel`
Expected: all pass (9 new in `texture_placement`).

- [ ] **Step 5: Check and commit**

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
git status
git add crates/kernel/src/texture.rs crates/kernel/src/model.rs crates/kernel/src/faces.rs crates/kernel/src/transform.rs crates/kernel/src/push_pull.rs crates/kernel/src/data.rs crates/kernel/src/scene.rs crates/kernel/src/edit.rs crates/kernel/src/triangles.rs crates/kernel/src/lib.rs crates/kernel/tests/texture_placement.rs crates/kernel/tests/triangles.rs
git commit -m "Kernel: texture placements on faces"
```

---

### Task 2: position_texture, face texture_position, .skr schema 9 and placed export UVs

**Files:**
- Create: `crates/session/src/document/placement.rs`
- Modify: `crates/session/src/command.rs`, `crates/session/src/document.rs`, `crates/session/src/query.rs`, `crates/session/src/lib.rs`, `crates/session/src/file.rs`
- Test: `crates/session/tests/texture_position.rs` (new), `crates/session/tests/export.rs`, `crates/session/tests/lights.rs`, `crates/api/tests/dispatch.rs`

**Interfaces:**
- Consumes: `Model::position_texture`, `PlacementEdit`, `TexturePlacement`, `Face::texture_position` (Task 1); `Document::parts`, `edit_mesh` (existing).
- Produces:
  - `Command::PositionTexture { ids: Vec<EntityId>, offset: Option<[f64; 2]>, rotation_deg: Option<f64>, scale: Option<[f64; 2]>, reset: bool }` (JSON `position_texture`; `reset` defaults to false)
  - `Outcome.positioned: Option<usize>` (absent for other commands)
  - `FaceInfo.texture_position: Option<TexturePlacement>`; `session::TexturePlacement` re-exported
  - `session::file::{SCHEMA_VERSION = 9, LIGHTS_SCHEMA_VERSION = 8}`

- [ ] **Step 1: Write the failing tests**

Apply to `crates/api/tests/dispatch.rs`:

````diff
diff --git a/crates/api/tests/dispatch.rs b/crates/api/tests/dispatch.rs
index 19bb08b..10917b7 100644
--- a/crates/api/tests/dispatch.rs
+++ b/crates/api/tests/dispatch.rs
@@ -184,6 +184,7 @@ fn describe_lists_methods_and_schemas() {
         "change_material",
         "delete_material",
         "paint",
+        "position_texture",
         "soften_edges",
         "soften_by_angle",
         "new",
````

Apply to `crates/session/tests/export.rs`:

````diff
diff --git a/crates/session/tests/export.rs b/crates/session/tests/export.rs
index 619f6ef..d2bb77b 100644
--- a/crates/session/tests/export.rs
+++ b/crates/session/tests/export.rs
@@ -921,3 +921,105 @@ fn mtl_of_glowing_materials_gives_their_emissive_color() {
     assert_eq!(library.matches("Ke ").count(), 1, "only the lamp glows");
     assert!(!library.contains("map_Ke"), "{library}");
 }
+
+/// A 2 x 1 m rectangle on the ground painted with a 2 x 1 m texture, its texture moved and
+/// turned a quarter. Returns the face as the kernel holds it.
+fn positioned_tile(d: &mut Document) -> kernel::Face {
+    let path = scratch("positioned-tile.png");
+    image::RgbaImage::from_fn(4, 2, |x, _| image::Rgba([60 * x as u8, 0, 200, 255]))
+        .save(&path)
+        .unwrap();
+    let out = run(
+        d,
+        json!({"type": "add_material", "name": "Tiles",
+               "texture": {"path": path.to_str().unwrap(), "width": 2}}),
+    )
+    .unwrap();
+    let material = out["changes"]["materials"][0].clone();
+    run(
+        d,
+        json!({"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [2, 1, 0]}),
+    )
+    .unwrap();
+    let face = ask(d, json!({"type": "entities", "kinds": ["face"]}))["faces"][0]["id"].clone();
+    run(
+        d,
+        json!({"type": "paint", "ids": [face], "material": material}),
+    )
+    .unwrap();
+    run(
+        d,
+        json!({"type": "position_texture", "ids": [face], "offset": [0.5, 0.25],
+               "rotation_deg": 90}),
+    )
+    .unwrap();
+    d.model().faces().next().unwrap().1.clone()
+}
+
+#[test]
+fn obj_writes_placed_texture_coordinates() {
+    let mut d = doc();
+    let face = positioned_tile(&mut d);
+    let obj = String::from_utf8(export(&mut d, "positioned.obj")).unwrap();
+    let numbers = |prefix: &str| -> Vec<Vec<f64>> {
+        obj.lines()
+            .filter_map(|l| l.strip_prefix(prefix))
+            .map(|l| l.split(' ').map(|x| x.parse().unwrap()).collect())
+            .collect()
+    };
+    let (v, vt) = (numbers("v "), numbers("vt "));
+    let mut corners = 0;
+    for f in obj.lines().filter_map(|l| l.strip_prefix("f ")) {
+        for corner in f.split(' ') {
+            let ids: Vec<usize> = corner.split('/').map(|k| k.parse().unwrap()).collect();
+            // Y-up back to the model's Z-up.
+            let p = &v[ids[0] - 1];
+            let model = DVec3::new(p[0], -p[2], p[1]);
+            let expected = face.texture_position(model) / DVec2::new(2.0, 1.0);
+            let st = &vt[ids[1] - 1];
+            assert!(
+                (st[0] - expected.x).abs() < 1e-9 && (st[1] - expected.y).abs() < 1e-9,
+                "{model} {st:?} {expected}"
+            );
+            corners += 1;
+        }
+    }
+    assert_eq!(corners, 6);
+    // Turned a quarter: along the ground's x the texture's height changes, not its width.
+    assert!(
+        (face.texture_position(DVec3::new(1.0, 0.0, 0.0)) - face.texture_position(DVec3::ZERO))
+            .abs_diff_eq(DVec2::new(0.0, -1.0), 1e-12)
+    );
+}
+
+#[test]
+fn gltf_writes_placed_texture_coordinates() {
+    let mut d = doc();
+    let face = positioned_tile(&mut d);
+    let (doc, buffer) = glb_parts(&export(&mut d, "positioned.glb"));
+    let accessor = |k: &Value| {
+        let a = &doc["accessors"][k.as_u64().unwrap() as usize];
+        let view = &doc["bufferViews"][a["bufferView"].as_u64().unwrap() as usize];
+        let at =
+            (view["byteOffset"].as_u64().unwrap() + a["byteOffset"].as_u64().unwrap()) as usize;
+        let width = if a["type"] == "VEC2" { 8 } else { 12 };
+        floats(&buffer[at..at + width * a["count"].as_u64().unwrap() as usize])
+    };
+    let primitive = &doc["meshes"][0]["primitives"][0];
+    let positions = accessor(&primitive["attributes"]["POSITION"]);
+    let st = accessor(&primitive["attributes"]["TEXCOORD_0"]);
+    assert_eq!(st.len(), 6 * 2);
+    for (p, s) in positions
+        .as_chunks::<3>()
+        .0
+        .iter()
+        .zip(st.as_chunks::<2>().0)
+    {
+        let model = DVec3::new(f64::from(p[0]), -f64::from(p[2]), f64::from(p[1]));
+        let placed = face.texture_position(model) / DVec2::new(2.0, 1.0);
+        // v counted down from the image's top, up to whole repeats.
+        let (ds, dt) = (f64::from(s[0]) - placed.x, f64::from(s[1]) + placed.y);
+        assert!((ds - ds.round()).abs() < 1e-6, "{model} {s:?} {placed}");
+        assert!((dt - dt.round()).abs() < 1e-6, "{model} {s:?} {placed}");
+    }
+}
````

Apply to `crates/session/tests/lights.rs`:

````diff
diff --git a/crates/session/tests/lights.rs b/crates/session/tests/lights.rs
index b87ef60..1c14cbf 100644
--- a/crates/session/tests/lights.rs
+++ b/crates/session/tests/lights.rs
@@ -133,7 +133,10 @@ fn glowing_materials_are_saved_as_version_8() {
     )
     .unwrap();
     let document = stored(&path);
-    assert_eq!(document["schema_version"], session::file::SCHEMA_VERSION);
+    assert_eq!(
+        document["schema_version"],
+        session::file::LIGHTS_SCHEMA_VERSION
+    );
     assert_eq!(document["schema_version"], 8);
     assert_eq!(document["model"]["materials"][0]["emission"], 3.0);
     let mut e = doc();
````

Create `crates/session/tests/texture_position.rs`:

````rust
//! Texture positions on faces, driven through JSON commands and queries.

use std::io::Write;
use std::path::PathBuf;

use glam::DVec2;
use serde_json::{Value, json};
use session::{Command, Document, ErrorKind, Query};

fn doc() -> Document {
    Document::new(DVec2::new(800.0, 600.0))
}

fn run(d: &mut Document, v: Value) -> Result<Value, (ErrorKind, String)> {
    let cmd: Command = serde_json::from_value(v).expect("valid command json");
    d.execute(cmd)
        .map(|o| serde_json::to_value(o).unwrap())
        .map_err(|e| (e.kind, e.message))
}

fn ask(d: &Document, q: Value) -> Value {
    let q: Query = serde_json::from_value(q).unwrap();
    serde_json::to_value(d.query(&q).unwrap()).unwrap()
}

fn faces(d: &Document) -> Vec<Value> {
    ask(d, json!({"type": "entities", "kinds": ["face"]}))["faces"]
        .as_array()
        .unwrap()
        .clone()
}

fn revision(d: &Document) -> Value {
    ask(d, json!({"type": "history"}))["revision"].clone()
}

/// A 2 x 1 m rectangle on the ground; returns its face id.
fn rectangle(d: &mut Document) -> Value {
    let out = run(
        d,
        json!({"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [2, 1, 0]}),
    )
    .unwrap();
    out["changes"]["created"]
        .as_array()
        .unwrap()
        .iter()
        .find(|id| id.as_str().unwrap().starts_with('f'))
        .unwrap()
        .clone()
}

/// A fresh path in the test scratch directory.
fn scratch(name: &str) -> PathBuf {
    // The process id keeps concurrent test runs (sharing a target directory) apart.
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("session-texture-position-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    let _ = std::fs::remove_file(&path);
    path
}

/// The `document.json` saved in a `.skr` file.
fn stored(path: &PathBuf) -> Value {
    let mut zip = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
    serde_json::from_reader(zip.by_name("document.json").unwrap()).unwrap()
}

#[test]
fn position_texture_sets_fields_reports_them_and_is_one_undo_step() {
    let mut d = doc();
    let f = rectangle(&mut d);
    let out = run(
        &mut d,
        json!({"type": "position_texture", "ids": [f], "offset": [0.5, 0.25],
               "rotation_deg": -90}),
    )
    .unwrap();
    assert_eq!(out["positioned"], 1);
    assert_eq!(out["changes"]["modified"], json!([f]));
    let face = ask(&d, json!({"type": "entity", "id": f}));
    assert_eq!(
        face["texture_position"],
        json!({"offset": [0.5, 0.25], "rotation_deg": 270.0, "scale": [1.0, 1.0]})
    );
    // Fields left out stay.
    run(
        &mut d,
        json!({"type": "position_texture", "ids": [f], "scale": [2, 0.5]}),
    )
    .unwrap();
    assert_eq!(
        ask(&d, json!({"type": "entity", "id": f}))["texture_position"],
        json!({"offset": [0.5, 0.25], "rotation_deg": 270.0, "scale": [2.0, 0.5]})
    );
    // The same again changes nothing and makes no undo step.
    let history = ask(&d, json!({"type": "history"}));
    let out = run(
        &mut d,
        json!({"type": "position_texture", "ids": [f], "scale": [2, 0.5]}),
    )
    .unwrap();
    assert_eq!(out["positioned"], 0);
    assert_eq!(ask(&d, json!({"type": "history"})), history);

    run(&mut d, json!({"type": "undo"})).unwrap();
    assert_eq!(
        ask(&d, json!({"type": "entity", "id": f}))["texture_position"]["scale"],
        json!([1.0, 1.0])
    );
    run(&mut d, json!({"type": "undo"})).unwrap();
    assert!(
        ask(&d, json!({"type": "entity", "id": f}))
            .get("texture_position")
            .is_none(),
        "absent when the texture lies as projected"
    );
    run(&mut d, json!({"type": "redo"})).unwrap();
    run(&mut d, json!({"type": "redo"})).unwrap();
    // Reset clears it, then applies what is given.
    run(
        &mut d,
        json!({"type": "position_texture", "ids": [f], "reset": true, "rotation_deg": 90}),
    )
    .unwrap();
    assert_eq!(
        ask(&d, json!({"type": "entity", "id": f}))["texture_position"],
        json!({"offset": [0.0, 0.0], "rotation_deg": 90.0, "scale": [1.0, 1.0]})
    );
    let out = run(
        &mut d,
        json!({"type": "position_texture", "ids": [f], "reset": true}),
    )
    .unwrap();
    assert_eq!(out["positioned"], 1);
    assert!(
        ask(&d, json!({"type": "entity", "id": f}))
            .get("texture_position")
            .is_none()
    );
}

#[test]
fn bad_position_texture_commands_are_refused_and_change_nothing() {
    let mut d = doc();
    let f = rectangle(&mut d);
    let edge = ask(&d, json!({"type": "entities", "kinds": ["edge"]}))["edges"][0]["id"].clone();
    let before = revision(&d);
    for (command, kind, words) in [
        (
            json!({"type": "position_texture", "ids": [], "rotation_deg": 10}),
            ErrorKind::InvalidInput,
            "at least one face",
        ),
        (
            json!({"type": "position_texture", "ids": [edge], "rotation_deg": 10}),
            ErrorKind::InvalidInput,
            "only faces",
        ),
        (
            json!({"type": "position_texture", "ids": [f]}),
            ErrorKind::InvalidInput,
            "give offset",
        ),
        (
            json!({"type": "position_texture", "ids": [f], "scale": [0, 1]}),
            ErrorKind::InvalidInput,
            "scale",
        ),
        (
            json!({"type": "position_texture", "ids": [f], "offset": [2e6, 0]}),
            ErrorKind::InvalidInput,
            "offset",
        ),
        (
            json!({"type": "position_texture", "ids": ["f999"], "rotation_deg": 10}),
            ErrorKind::NotFound,
            "no such",
        ),
    ] {
        let (k, message) = run(&mut d, command.clone()).unwrap_err();
        assert_eq!(k, kind, "{command}: {message}");
        assert!(message.contains(words), "{command}: {message}");
    }
    assert!(
        serde_json::from_value::<Command>(
            json!({"type": "position_texture", "ids": [f], "angle": 10})
        )
        .is_err(),
        "unknown fields are refused"
    );
    assert_eq!(revision(&d), before);
}

#[test]
fn split_and_copied_faces_keep_their_texture_position() {
    let mut d = doc();
    let f = rectangle(&mut d);
    run(
        &mut d,
        json!({"type": "position_texture", "ids": [f], "rotation_deg": 45}),
    )
    .unwrap();
    // A line across the face splits it; both pieces keep the placement.
    run(
        &mut d,
        json!({"type": "add_edge", "start": [1, 0, 0], "end": [1, 1, 0]}),
    )
    .unwrap();
    let split = faces(&d);
    assert_eq!(split.len(), 2);
    assert!(
        split
            .iter()
            .all(|f| f["texture_position"]["rotation_deg"] == 45.0)
    );
    let ids: Vec<Value> = split.iter().map(|f| f["id"].clone()).collect();
    run(
        &mut d,
        json!({"type": "copy", "ids": ids, "offset": [0, 3, 0]}),
    )
    .unwrap();
    let copied = faces(&d);
    assert_eq!(copied.len(), 4);
    assert!(
        copied
            .iter()
            .all(|f| f["texture_position"]["rotation_deg"] == 45.0)
    );
    // A new face has none.
    run(
        &mut d,
        json!({"type": "add_rectangle", "corner": [5, 0, 0], "opposite": [6, 1, 0]}),
    )
    .unwrap();
    assert_eq!(
        faces(&d)
            .iter()
            .filter(|f| f.get("texture_position").is_none())
            .count(),
        1
    );
}

#[test]
fn texture_positions_are_saved_as_version_9_and_older_files_open_without_them() {
    let mut d = doc();
    let f = rectangle(&mut d);
    run(
        &mut d,
        json!({"type": "position_texture", "ids": [f], "offset": [0.1, 0.2],
               "rotation_deg": 30, "scale": [2, 3]}),
    )
    .unwrap();
    let path = scratch("positioned.skr");
    run(
        &mut d,
        json!({"type": "save", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    let document = stored(&path);
    assert_eq!(document["schema_version"], session::file::SCHEMA_VERSION);
    assert_eq!(document["schema_version"], 9);
    assert_eq!(
        document["model"]["root"]["faces"][0]["texture_position"],
        json!({"offset": [0.1, 0.2], "rotation_deg": 30.0, "scale": [2.0, 3.0]})
    );
    let mut e = doc();
    run(
        &mut e,
        json!({"type": "open", "path": path.to_str().unwrap()}),
    )
    .unwrap();
    assert_eq!(faces(&e), faces(&d));

    // A file from before texture positions (schema 5, no field) opens without them.
    let mut old = document.clone();
    old["schema_version"] = json!(5);
    old["model"]["root"]["faces"][0]
        .as_object_mut()
        .unwrap()
        .remove("texture_position");
    let old_path = scratch("old.skr");
    let mut zip = zip::ZipWriter::new(std::fs::File::create(&old_path).unwrap());
    zip.start_file("document.json", zip::write::SimpleFileOptions::default())
        .unwrap();
    zip.write_all(&serde_json::to_vec(&old).unwrap()).unwrap();
    zip.finish().unwrap();
    let mut o = doc();
    run(
        &mut o,
        json!({"type": "open", "path": old_path.to_str().unwrap()}),
    )
    .unwrap();
    assert!(
        faces(&o)
            .iter()
            .all(|f| f.get("texture_position").is_none())
    );

    // Without placements the model saves as before they existed.
    run(
        &mut d,
        json!({"type": "position_texture", "ids": [f], "reset": true}),
    )
    .unwrap();
    run(&mut d, json!({"type": "save"})).unwrap();
    assert_eq!(
        stored(&path)["schema_version"],
        session::file::OPAQUE_SCHEMA_VERSION
    );
}
````

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p session --test texture_position --test export --test lights` and `cargo test -p api --test dispatch describe`
Expected: the `texture_position` tests and the two new `export` tests panic with "valid command json" (there is no `position_texture` command yet); `lights` does not compile (`LIGHTS_SCHEMA_VERSION` unknown); the `describe` test fails with "Command schema lacks position_texture".

- [ ] **Step 3: Implement**

Apply to `crates/session/src/command.rs`:

````diff
diff --git a/crates/session/src/command.rs b/crates/session/src/command.rs
index 4937af4..39fe711 100644
--- a/crates/session/src/command.rs
+++ b/crates/session/src/command.rs
@@ -225,6 +225,22 @@ pub enum Command {
         ids: Vec<EntityId>,
         max_angle_deg: f64,
     },
+    /// Move, turn and scale the texture on faces of the open context, as one undo step: on
+    /// top of the projection, `offset` [u, v] in meters, `rotation_deg` counter-clockwise
+    /// seen from the face's front, and `scale` [u, v] (multipliers, 0.001 to 1000). Fields
+    /// left out stay as they are; `reset: true` first clears the placement. The outcome's
+    /// `positioned` counts the faces that changed.
+    PositionTexture {
+        ids: Vec<EntityId>,
+        #[serde(default)]
+        offset: Option<[f64; 2]>,
+        #[serde(default)]
+        rotation_deg: Option<f64>,
+        #[serde(default)]
+        scale: Option<[f64; 2]>,
+        #[serde(default)]
+        reset: bool,
+    },
     /// Start an empty model with the default view. Clears undo history; not an undo step.
     /// With changes not saved, fails (`unsaved_changes`) unless `discard` is true.
     New {
@@ -422,6 +438,9 @@ pub struct Outcome {
     /// commands.
     #[serde(default, skip_serializing_if = "Option::is_none")]
     pub soften: Option<SoftenReport>,
+    /// How many faces a `position_texture` changed; absent for other commands.
+    #[serde(default, skip_serializing_if = "Option::is_none")]
+    pub positioned: Option<usize>,
 }
 
 #[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
````

Apply to `crates/session/src/document.rs`:

````diff
diff --git a/crates/session/src/document.rs b/crates/session/src/document.rs
index 459f80b..7ed7617 100644
--- a/crates/session/src/document.rs
+++ b/crates/session/src/document.rs
@@ -4,7 +4,7 @@ use std::path::{Path, PathBuf};
 use glam::{DAffine3, DVec2, DVec3};
 use kernel::{
     ChangeSet, EdgeId, EntityId, FaceId, Guide, GuideId, Instance, InstanceId, KernelError,
-    MaterialEdit, MeshId, Model, Scene, SceneChange, SoftenReport, VertexId,
+    MaterialEdit, MeshId, Model, PlacementEdit, Scene, SceneChange, SoftenReport, VertexId,
 };
 
 use crate::arc::{DEFAULT_ARC_SEGMENTS, arc_points};
@@ -24,6 +24,7 @@ mod files;
 mod groups;
 mod import;
 mod materials;
+mod placement;
 mod soften;
 
 use import::ImportOptions;
@@ -61,6 +62,8 @@ pub struct Document {
     import_report: Option<ImportReport>,
     /// What the last soften edit did, until its outcome takes it.
     soften_report: Option<SoftenReport>,
+    /// How many faces the last `position_texture` changed, until its outcome takes it.
+    positioned: Option<usize>,
 }
 
 impl Document {
@@ -80,6 +83,7 @@ impl Document {
             clean_revision: 0,
             import_report: None,
             soften_report: None,
+            positioned: None,
         }
     }
 
@@ -168,6 +172,7 @@ impl Document {
             let mut outcome = self.record(changes?);
             outcome.import = self.import_report.take();
             outcome.soften = self.soften_report.take();
+            outcome.positioned = self.positioned.take();
             return Ok(outcome);
         }
         match command {
@@ -273,6 +278,7 @@ impl Document {
             | Command::Paint { .. }
             | Command::SoftenEdges { .. }
             | Command::SoftenByAngle { .. }
+            | Command::PositionTexture { .. }
             | Command::Import { .. } => {
                 unreachable!("edits are handled by Document::edit")
             }
@@ -378,6 +384,21 @@ impl Document {
             Command::SoftenByAngle { ids, max_angle_deg } => {
                 self.soften_by_angle_changes(ids, *max_angle_deg)
             }
+            Command::PositionTexture {
+                ids,
+                offset,
+                rotation_deg,
+                scale,
+                reset,
+            } => self.position_texture_changes(
+                ids,
+                PlacementEdit {
+                    offset: *offset,
+                    rotation_deg: *rotation_deg,
+                    scale: *scale,
+                    reset: *reset,
+                },
+            ),
             Command::Import {
                 path,
                 format,
@@ -464,6 +485,7 @@ impl Document {
                         import: self.import_report.take(),
                         export: None,
                         soften: self.soften_report.take(),
+                        positioned: self.positioned.take(),
                     }
                 }),
                 None if matches!(
@@ -1258,6 +1280,7 @@ impl Document {
             area: self.model().area(id).unwrap(),
             front_material: f.front,
             back_material: f.back,
+            texture_position: f.texture,
         }
     }
 }
````

Create `crates/session/src/document/placement.rs`:

````rust
//! Texture positions on faces.

use kernel::{EntityId, PlacementEdit, SceneChange};

use super::Document;
use crate::command::{CommandError, ErrorKind};

impl Document {
    /// Sets the texture placement of the faces in `ids`. Other kinds of entity are invalid
    /// input, as is an empty list or an edit that sets nothing.
    pub(super) fn position_texture_changes(
        &mut self,
        ids: &[EntityId],
        edit: PlacementEdit,
    ) -> Result<SceneChange, CommandError> {
        if ids.is_empty() {
            return Err(CommandError::new(
                ErrorKind::InvalidInput,
                "ids must list at least one face",
            ));
        }
        let others: Vec<EntityId> = ids
            .iter()
            .copied()
            .filter(|id| !matches!(id, EntityId::Face(_)))
            .collect();
        if !others.is_empty() {
            return Err(CommandError::new(
                ErrorKind::InvalidInput,
                "only faces have a texture position",
            )
            .with_ids(others));
        }
        if edit == PlacementEdit::default() {
            return Err(CommandError::new(
                ErrorKind::InvalidInput,
                "give offset, rotation_deg, scale or reset",
            ));
        }
        let faces = self.parts(ids)?.faces;
        let mut count = 0;
        let change = self
            .edit_mesh(|m| {
                let (changes, n) = m.position_texture(&faces, edit)?;
                count = n;
                Ok(changes)
            })
            .map_err(|e| CommandError::new(ErrorKind::InvalidInput, e.to_string()))?;
        self.positioned = Some(count);
        Ok(change)
    }
}
````

Apply to `crates/session/src/file.rs`:

````diff
diff --git a/crates/session/src/file.rs b/crates/session/src/file.rs
index ebe5472..95643be 100644
--- a/crates/session/src/file.rs
+++ b/crates/session/src/file.rs
@@ -17,20 +17,23 @@ use crate::texture::{MAX_TEXTURE_BYTES, decode};
 
 /// The `format` every file records.
 pub const FORMAT: &str = "tenon";
-/// The newest schema version this build reads, and the one it writes when a material glows or
-/// the view is lit by night. Version 2 added guides, version 3 curves and soft edges, version 4
-/// the camera's up direction and projection, version 5 materials, version 6 material opacity,
-/// version 7 textures and version 8 material emission and the lighting mode; older files still
-/// open.
-pub const SCHEMA_VERSION: u32 = 8;
+/// The newest schema version this build reads, and the one it writes when a face has a texture
+/// position. Version 2 added guides, version 3 curves and soft edges, version 4 the camera's up
+/// direction and projection, version 5 materials, version 6 material opacity, version 7
+/// textures, version 8 material emission and the lighting mode, and version 9 texture
+/// positions on faces; older files still open.
+pub const SCHEMA_VERSION: u32 = 9;
+/// The version written when a material glows or the view is lit by night but no face has a
+/// texture position.
+pub const LIGHTS_SCHEMA_VERSION: u32 = 8;
 /// The version written when a material has a texture but none glows and the view is lit by
 /// day.
 pub const TEXTURED_SCHEMA_VERSION: u32 = 7;
 /// The version written when a material is see-through but none has a texture or glows.
 pub const TRANSLUCENT_SCHEMA_VERSION: u32 = 6;
-/// The version written when every material is opaque and none has a texture or glows: such a
-/// model needs nothing versions 6 to 8 added, so its file is the same as before and older
-/// builds open it.
+/// The version written when every material is opaque and none has a texture or glows, and no
+/// face has a texture position: such a model needs nothing versions 6 to 9 added, so its file
+/// is the same as before and older builds open it.
 pub const OPAQUE_SCHEMA_VERSION: u32 = 5;
 /// The archive entry holding the document.
 const ENTRY: &str = "document.json";
@@ -81,6 +84,9 @@ pub(crate) fn write(
             "the model has used ids past {MAX_ID}, more than a file can hold"
         ));
     }
+    let positioned = std::iter::once(&model.root)
+        .chain(model.definitions.iter().map(|d| &d.mesh))
+        .any(|m| m.faces.iter().any(|f| f.texture_position.is_some()));
     let needs_lights_schema =
         model.materials.iter().any(|m| m.emission > 0.0) || lighting != Lighting::Day;
     let textured = model.materials.iter().any(|m| m.texture.is_some());
@@ -95,8 +101,10 @@ pub(crate) fn write(
         .collect();
     let stored = Stored {
         format: FORMAT.into(),
-        schema_version: if needs_lights_schema {
+        schema_version: if positioned {
             SCHEMA_VERSION
+        } else if needs_lights_schema {
+            LIGHTS_SCHEMA_VERSION
         } else if textured {
             TEXTURED_SCHEMA_VERSION
         } else if translucent {
````

Apply to `crates/session/src/lib.rs`:

````diff
diff --git a/crates/session/src/lib.rs b/crates/session/src/lib.rs
index 0a9ee9f..e9ea3e1 100644
--- a/crates/session/src/lib.rs
+++ b/crates/session/src/lib.rs
@@ -25,7 +25,7 @@ pub use import::{
     ImportAs, ImportFormat, ImportReport, MAX_CONTEXT_IMPORT_FACES, MAX_IMPORT_BYTES,
     MAX_IMPORT_MATERIALS, MAX_IMPORT_POINTS, MAX_IMPORT_POLYGONS, UpAxis,
 };
-pub use kernel::{CurveKind, ImageFormat, MaterialId, Side};
+pub use kernel::{CurveKind, ImageFormat, MaterialId, Side, TexturePlacement};
 pub use query::{
     Bounds, ChangeEvent, CurveInfo, DefinitionInfo, EdgeInfo, EntityKind, FaceInfo, GuideInfo,
     GuideKind, InstanceInfo, MaterialInfo, OutlinerNode, OutlinerRow, OutlinerTable, Query,
````

Apply to `crates/session/src/query.rs`:

````diff
diff --git a/crates/session/src/query.rs b/crates/session/src/query.rs
index 4b491ae..1981d05 100644
--- a/crates/session/src/query.rs
+++ b/crates/session/src/query.rs
@@ -2,6 +2,7 @@ use std::collections::BTreeMap;
 
 use kernel::{
     ChangeSummary, CurveKind, DefinitionId, DefinitionKind, EntityId, ImageFormat, MaterialId,
+    TexturePlacement,
 };
 use schemars::JsonSchema;
 use serde::{Deserialize, Serialize};
@@ -146,6 +147,10 @@ pub struct FaceInfo {
     /// The material of the other side; absent for the default material.
     #[serde(default, skip_serializing_if = "Option::is_none")]
     pub back_material: Option<MaterialId>,
+    /// Where the texture lies on top of its projection (see `position_texture`); absent when
+    /// it lies as projected.
+    #[serde(default, skip_serializing_if = "Option::is_none")]
+    pub texture_position: Option<TexturePlacement>,
 }
 
 /// A placed group or component.
````

- [ ] **Step 4: Run the tests**

Run: `cargo test -p session --test texture_position --test export --test lights` and `cargo test -p api`
Expected: all pass.

- [ ] **Step 5: Check and commit**

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
git status
git add crates/session/src/command.rs crates/session/src/document.rs crates/session/src/document/placement.rs crates/session/src/query.rs crates/session/src/lib.rs crates/session/src/file.rs crates/session/tests/texture_position.rs crates/session/tests/export.rs crates/session/tests/lights.rs crates/api/tests/dispatch.rs
git commit -m "position_texture command, face texture_position, .skr schema 9, placed export UVs"
```

---

### Task 3: Render: placed textures, back sides not mirrored; positioned golden image

**Files:**
- Modify: `crates/render/src/scene.rs`
- Test: `crates/render/tests/golden.rs`, `crates/render/tests/golden/positioned.png` (new, written by the test)

**Interfaces:**
- Consumes: `Face::texture_position`, `back_texture_position`, `PlacementEdit`, `Model::position_texture` (Task 1); `Command::PositionTexture` (Task 2); `quadrants_png`, `check`, `look`, `created_face` (golden.rs, existing).
- Produces: nothing new for later tasks (vertex format unchanged: `BaseFaceVertex.uv` keeps its layout; its back half is now mirrored).

- [ ] **Step 1: Write the failing tests**

The golden test renders two free-standing 1.5 x 1.5 m walls painted on both sides with the 16 px quadrant image at 1 m: the left one shows its front with the texture turned a quarter counter-clockwise and halved (scale 0.5), the right one shows its back with the texture moved 0.25 m along u.

Apply to `crates/render/tests/golden.rs`:

````diff
diff --git a/crates/render/tests/golden.rs b/crates/render/tests/golden.rs
index edcaf9c..1fa9425 100644
--- a/crates/render/tests/golden.rs
+++ b/crates/render/tests/golden.rs
@@ -760,6 +760,94 @@ fn golden_textured_box_and_ground() {
     check("textured", &d);
 }
 
+/// A free-standing 1.5 x 1.5 m wall at `x` in the plane y = 0, turned so its front faces -y
+/// (toward the camera of `golden_positioned_textures`) or +y; returns its face.
+fn wall(doc: &mut Document, x: f64, front_to_camera: bool) -> EntityId {
+    let made = run(
+        doc,
+        Command::AddRectangle {
+            corner: [x, 0.0, 0.0],
+            opposite: [x + 1.5, 0.0, 1.5],
+            normal: None,
+        },
+    );
+    let face = created_face(&made);
+    let EntityId::Face(id) = face else {
+        unreachable!()
+    };
+    let toward = doc.model().face(id).expect("face").normal.y < 0.0;
+    if toward != front_to_camera {
+        // Half a turn about its middle shows the other side.
+        run(
+            doc,
+            Command::Rotate {
+                ids: vec![face],
+                center: [x + 0.75, 0.0, 0.0],
+                axis: None,
+                angle: 180.0,
+                copies: None,
+            },
+        );
+    }
+    face
+}
+
+#[test]
+fn golden_positioned_textures() {
+    let mut d = doc();
+    // Left: a wall's front with the texture turned a quarter counter-clockwise and halved.
+    // Right: a wall's back with the texture moved a quarter along: it reads as seen from
+    // behind, not mirrored (white lines along the tops and left sides of the copies).
+    let front = wall(&mut d, 0.0, true);
+    let back = wall(&mut d, 2.0, false);
+    let tiles = run(
+        &mut d,
+        Command::AddMaterial {
+            name: Some("Tiles".into()),
+            color: None,
+            opacity: None,
+            texture: Some(session::TextureSource {
+                path: Some(quadrants_png().display().to_string()),
+                width: Some(1.0),
+                height: Some(1.0),
+            }),
+            emission: None,
+        },
+    )
+    .changes
+    .materials[0];
+    run(
+        &mut d,
+        Command::Paint {
+            ids: vec![front, back],
+            material: Some(tiles),
+            side: Side::Both,
+        },
+    );
+    run(
+        &mut d,
+        Command::PositionTexture {
+            ids: vec![front],
+            offset: None,
+            rotation_deg: Some(90.0),
+            scale: Some([0.5, 0.5]),
+            reset: false,
+        },
+    );
+    run(
+        &mut d,
+        Command::PositionTexture {
+            ids: vec![back],
+            offset: Some([0.25, 0.0]),
+            rotation_deg: None,
+            scale: None,
+            reset: false,
+        },
+    );
+    look(&mut d, [1.75, -4.5, 0.75], [1.75, 0.0, 0.75]);
+    check("positioned", &d);
+}
+
 /// A material glowing at `emission`.
 fn add_lamp_material(doc: &mut Document, name: &str, color: [u8; 3], emission: f32) -> MaterialId {
     let outcome = run(
@@ -876,6 +964,7 @@ fn every_golden_image_has_a_test() {
         "glass",
         "textured",
         "night",
+        "positioned",
     ];
     let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden");
     let mut stored: Vec<String> = std::fs::read_dir(&dir)
````

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p render --test golden golden_positioned`
Expected: FAIL, "positioned: no readable golden image" (the image is stored in Step 4). The unit test `placed_textures_turn_and_back_sides_read_unmirrored` is added with the source in Step 3.

- [ ] **Step 3: Implement**

The unit test at the end of the diff checks, for each triangle, the map from a viewer's right and up to the image's width and height on each side: on the front the quarter turn sends a step right down the image's height, and on the back the same turn reads clockwise with a positive determinant (not mirrored). Before this change the back's map was the front's mirrored, so it fails without the `back_texture_position` line.

Apply to `crates/render/src/scene.rs`:

````diff
diff --git a/crates/render/src/scene.rs b/crates/render/src/scene.rs
index e006ca7..0cc19d1 100644
--- a/crates/render/src/scene.rs
+++ b/crates/render/src/scene.rs
@@ -16,7 +16,7 @@ use bytemuck::{Pod, Zeroable};
 use glam::{DAffine3, DVec2, DVec3};
 use kernel::{
     EdgeId, EntityId, FaceId, Guide, GuideId, InstanceId, MaterialId, MeshId, Model, Scene,
-    texture_position,
+    back_texture_position,
 };
 
 use crate::grid::LineVertex;
@@ -59,8 +59,10 @@ pub struct BaseFaceVertex {
     /// is drawn in the translucent pass (see `SceneBase::glass`).
     pub alpha: [f32; 2],
     /// The texture coordinates of the front (first two) and back sides: the corner's texture
-    /// position over its side's texture size, v counted down from the image's top, shifted by
-    /// whole repeats per triangle so they stay small. Zero on a side without a texture.
+    /// position on that side (see `kernel::Face::texture_position`; the back's is mirrored so
+    /// it reads as seen from behind) over its side's texture size, v counted down from the
+    /// image's top, shifted by whole repeats per triangle so they stay small. Zero on a side
+    /// without a texture.
     pub uv: [f32; 4],
     /// The texture array layers of the front and back sides (see `texture::layers`); -1 for
     /// none.
@@ -158,9 +160,13 @@ fn face_glow(front: &Look, back: &Look) -> (f32, [f32; 3]) {
     (side.emission, side.color)
 }
 
-/// The corners of a triangle with both sides' texture coordinates (see `BaseFaceVertex::uv`).
+/// The corners of a triangle with both sides' texture coordinates (see `BaseFaceVertex::uv`),
+/// from the corners' positions on the front's texture.
 fn both_uv(front: &Look, back: &Look, positions: [DVec2; 3]) -> [[f32; 4]; 3] {
-    let (f, b) = (front.uv(positions), back.uv(positions));
+    let (f, b) = (
+        front.uv(positions),
+        back.uv(positions.map(back_texture_position)),
+    );
     [0, 1, 2].map(|k| [f[k][0], f[k][1], b[k][0], b[k][1]])
 }
 
@@ -438,7 +444,8 @@ struct MeshParts {
     corners: Vec<DVec3>,
     /// The normal at each corner: the face's own, or across soft edges the smooth one.
     normals: Vec<DVec3>,
-    /// Each corner's position on its face's texture, in meters (see `texture_position`).
+    /// Each corner's position on its face's front texture, in meters (see
+    /// `kernel::Face::texture_position`).
     positions: Vec<DVec2>,
     faces: Vec<FacePart>,
     /// The ends of the hard edges (soft edges are not drawn).
@@ -475,7 +482,7 @@ impl MeshParts {
             for tri in model.shaded_triangles(id, &smooth).unwrap_or_default() {
                 corners.extend(tri.corners);
                 normals.extend(tri.normals);
-                positions.extend(tri.corners.map(|p| texture_position(face.normal, p)));
+                positions.extend(tri.corners.map(|p| face.texture_position(p)));
             }
             faces.push(FacePart {
                 id,
@@ -1305,6 +1312,71 @@ mod tests {
         assert_eq!(g, single_pass::scene_geometry(&s, &[], &BTreeSet::new()));
     }
 
+    #[test]
+    fn placed_textures_turn_and_back_sides_read_unmirrored() {
+        // A 1 m wall in the xz plane with the same 1 m texture on both sides, turned a quarter
+        // counter-clockwise.
+        let mut m = Model::new();
+        let pts =
+            [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)].map(|(x, z)| DVec3::new(x, 0.0, z));
+        m.add_polygon(&pts).unwrap();
+        let face = m.faces().next().unwrap().0;
+        let turn = kernel::PlacementEdit {
+            rotation_deg: Some(90.0),
+            ..kernel::PlacementEdit::default()
+        };
+        m.position_texture(&[face], turn).unwrap();
+        let mut s = Scene::from_root(m);
+        let (tiles, _) = s
+            .add_material_with_texture("Tiles", [1, 2, 3], 1.0, Some(texture(1.0, 1.0)))
+            .unwrap();
+        s.paint(MeshId::Root, &[face], &[], kernel::Side::Both, Some(tiles))
+            .unwrap();
+        let n = s.root().face(face).unwrap().normal;
+        let g = scene_geometry(&s, &[], &BTreeSet::new());
+        assert_eq!(g.faces.len(), 6);
+        // For a viewer in front of a side, upright: how a step right and a step up move across
+        // the image (its width and its height, up the image). `right` is the viewer's right.
+        let image_steps = |tri: &[FaceVertex], k: usize, right: DVec3| {
+            let at = |v: &FaceVertex| {
+                let p = DVec3::from(v.pos.map(f64::from));
+                (
+                    DVec2::new(p.dot(right), p.z),
+                    // v counts down from the image's top.
+                    DVec2::new(f64::from(v.uv[k]), -f64::from(v.uv[k + 1])),
+                )
+            };
+            let ((p0, t0), (p1, t1), (p2, t2)) = (at(&tri[0]), at(&tri[1]), at(&tri[2]));
+            let screen = glam::DMat2::from_cols(p1 - p0, p2 - p0);
+            let image = glam::DMat2::from_cols(t1 - t0, t2 - t0);
+            image * screen.inverse()
+        };
+        for tri in g.faces.chunks(3) {
+            // From the front: the image's width runs up and its height to the left.
+            let front = image_steps(tri, 0, DVec3::Z.cross(n));
+            assert!(
+                front.x_axis.abs_diff_eq(DVec2::new(0.0, -1.0), 1e-6),
+                "{front}"
+            );
+            assert!(
+                front.y_axis.abs_diff_eq(DVec2::new(1.0, 0.0), 1e-6),
+                "{front}"
+            );
+            // From behind: the same turn seen from the other side (clockwise), not mirrored.
+            let back = image_steps(tri, 2, n.cross(DVec3::Z));
+            assert!(
+                back.x_axis.abs_diff_eq(DVec2::new(0.0, 1.0), 1e-6),
+                "{back}"
+            );
+            assert!(
+                back.y_axis.abs_diff_eq(DVec2::new(-1.0, 0.0), 1e-6),
+                "{back}"
+            );
+            assert!(back.determinant() > 0.0 && front.determinant() > 0.0);
+        }
+        assert_eq!(g, single_pass::scene_geometry(&s, &[], &BTreeSet::new()));
+    }
+
     /// The single pass `scene_geometry` made before the base and the selection flags were
     /// split, kept to check that the split changes nothing.
     mod single_pass {
@@ -1389,7 +1461,7 @@ mod tests {
                         DEFAULT_BACK,
                     );
                     for tri in model.shaded_triangles(fid, &smooth).unwrap_or_default() {
-                        let positions = tri.corners.map(|p| texture_position(face.normal, p));
+                        let positions = tri.corners.map(|p| face.texture_position(p));
                         let uv = both_uv(&front, &back, positions);
                         let mut corners =
                             [0, 1, 2].map(|k| (tri.corners[k], tri.normals[k], uv[k]));
````

- [ ] **Step 4: Store the golden image and check it by eye**

Run: `UPDATE_GOLDEN=1 cargo test -p render --test golden golden_positioned`, then open `crates/render/tests/golden/positioned.png` (160 x 120).
Expected: on the left wall, three by three half-meter copies, each with yellow top left, blue top right, red bottom left and green bottom right, white lines along their left sides and bottoms (the quadrant image turned a quarter counter-clockwise); on the right wall, 1 m copies upright and not mirrored: red top left, yellow top right, green bottom left, blue bottom right, white lines along their tops and left sides, shifted a quarter meter sideways. Then run `cargo test -p render` (all pass, every other golden image unchanged, `every_golden_image_has_a_test` lists `positioned`).

- [ ] **Step 5: Check and commit**

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
git status
git add crates/render/src/scene.rs crates/render/tests/golden.rs crates/render/tests/golden/positioned.png
git commit -m "Render: placed textures, back sides not mirrored; positioned golden"
```

---

### Task 4: Entity Info texture position fields, Rotate 90 and Reset

**Files:**
- Create: `crates/app/src/editor/texture_position.rs`
- Modify: `crates/app/src/editor.rs`, `crates/app/src/ui.rs`, `crates/app/src/app.rs`

**Interfaces:**
- Consumes: `Command::PositionTexture` (Task 2), `kernel::TexturePlacement`, `MAX_TEXTURE_OFFSET`, `MIN_TEXTURE_SCALE`, `MAX_TEXTURE_SCALE` (Task 1); `Editor::run`, `execute_batch`, `reset_tool`, `invalidate_hover` (existing); the ui test helpers `soften_view`, `panel_actions`, `status` (existing).
- Produces:
  - `Editor::selection_texture_position(&self) -> Option<TexturePlacement>`
  - `Editor::set_texture_offset(&mut self, millimeters: [i32; 2]) -> bool`, `set_texture_rotation(&mut self, tenths: i32) -> bool`, `set_texture_scale(&mut self, thousandths: [u32; 2]) -> bool`, `reset_texture_position(&mut self) -> bool`, `rotate_texture_90(&mut self) -> bool`
  - `UiAction::{SetTextureOffset([i32; 2]), SetTextureRotation(i32), SetTextureScale([u32; 2]), RotateTexture90, ResetTexturePosition}`
  - `View.texture_position: Option<TexturePlacement>`

- [ ] **Step 1: Make the changes**

The editor's tests are in the new `editor/texture_position.rs`, the UI test (`entity_info_shows_texture_position_fields_for_selected_faces`) in `ui.rs`'s test module; write those first and see them fail to compile, then the rest.

Apply to `crates/app/src/app.rs`:

````diff
diff --git a/crates/app/src/app.rs b/crates/app/src/app.rs
index 566613d..492d2ff 100644
--- a/crates/app/src/app.rs
+++ b/crates/app/src/app.rs
@@ -714,6 +714,21 @@ impl State {
             UiAction::SetSoft(soft) => {
                 self.editor.set_selection_soft(soft);
             }
+            UiAction::SetTextureOffset(millimeters) => {
+                self.editor.set_texture_offset(millimeters);
+            }
+            UiAction::SetTextureRotation(tenths) => {
+                self.editor.set_texture_rotation(tenths);
+            }
+            UiAction::SetTextureScale(thousandths) => {
+                self.editor.set_texture_scale(thousandths);
+            }
+            UiAction::RotateTexture90 => {
+                self.editor.rotate_texture_90();
+            }
+            UiAction::ResetTexturePosition => {
+                self.editor.reset_texture_position();
+            }
         }
     }
 
@@ -801,6 +816,7 @@ impl State {
             selection: self.editor.doc().selection(),
             soften: self.editor.soften_window(),
             edges_soft: self.editor.selection_soft(),
+            texture_position: self.editor.selection_texture_position(),
         };
         let mut gui_frame = self.gui.frame(&self.window, &panels);
         gui_frame.hide_cursor = self.capture == Capture::Grabbed;
````

Apply to `crates/app/src/editor.rs`:

````diff
diff --git a/crates/app/src/editor.rs b/crates/app/src/editor.rs
index 9c05b24..e132f25 100644
--- a/crates/app/src/editor.rs
+++ b/crates/app/src/editor.rs
@@ -23,6 +23,7 @@ use crate::fly::{self, Fly, FlyKey};
 use crate::navigation::{Button, LOGICAL_PX_PER_ZOOM_STEP, Navigation, Scroll};
 
 mod soften;
+mod texture_position;
 
 use soften::SoftenWindow;
````

Create `crates/app/src/editor/texture_position.rs`:

````rust
//! Entity Info's texture position fields for selected faces.

use std::collections::BTreeMap;

use kernel::{EntityId, TexturePlacement};
use session::Command;

use super::Editor;

impl Editor {
    /// The texture placement Entity Info shows when only faces are selected: the first
    /// selected face's (the default placement when it has none). None otherwise.
    pub fn selection_texture_position(&self) -> Option<TexturePlacement> {
        let selection = self.doc.selection();
        let m = self.doc.model();
        let mut first = None;
        for id in selection {
            let EntityId::Face(f) = id else { return None };
            let face = m.face(*f)?;
            first.get_or_insert(face.texture.unwrap_or_default());
        }
        first
    }

    /// Sets the texture offset of the selected faces, in millimeters along u and v.
    pub fn set_texture_offset(&mut self, millimeters: [i32; 2]) -> bool {
        self.position_selected(Command::PositionTexture {
            ids: Vec::new(),
            offset: Some(millimeters.map(|mm| f64::from(mm) / 1000.0)),
            rotation_deg: None,
            scale: None,
            reset: false,
        })
    }

    /// Sets the texture rotation of the selected faces, in tenths of a degree.
    pub fn set_texture_rotation(&mut self, tenths: i32) -> bool {
        self.position_selected(Command::PositionTexture {
            ids: Vec::new(),
            offset: None,
            rotation_deg: Some(f64::from(tenths) / 10.0),
            scale: None,
            reset: false,
        })
    }

    /// Sets the texture scale of the selected faces, in thousandths.
    pub fn set_texture_scale(&mut self, thousandths: [u32; 2]) -> bool {
        self.position_selected(Command::PositionTexture {
            ids: Vec::new(),
            offset: None,
            rotation_deg: None,
            scale: Some(thousandths.map(|t| f64::from(t) / 1000.0)),
            reset: false,
        })
    }

    /// Clears the texture placement of the selected faces.
    pub fn reset_texture_position(&mut self) -> bool {
        self.position_selected(Command::PositionTexture {
            ids: Vec::new(),
            offset: None,
            rotation_deg: None,
            scale: None,
            reset: true,
        })
    }

    /// Turns the texture of each selected face a further quarter counter-clockwise, as one
    /// undo step (faces turned alike share a command in one atomic batch).
    pub fn rotate_texture_90(&mut self) -> bool {
        let m = self.doc.model();
        let mut by_rotation: BTreeMap<u64, (f64, Vec<EntityId>)> = BTreeMap::new();
        for id in self.doc.selection() {
            let EntityId::Face(f) = id else { continue };
            let Some(face) = m.face(*f) else { continue };
            let r = face.texture.unwrap_or_default().rotation_deg + 90.0;
            by_rotation
                .entry(r.to_bits())
                .or_insert_with(|| (r, Vec::new()))
                .1
                .push(*id);
        }
        if by_rotation.is_empty() {
            return false;
        }
        let commands = by_rotation
            .into_values()
            .map(|(r, ids)| Command::PositionTexture {
                ids,
                offset: None,
                rotation_deg: Some(r),
                scale: None,
                reset: false,
            })
            .collect();
        self.message = None;
        if let Err(e) = self.execute_batch(commands, true) {
            self.message = Some(e.error.message);
        }
        true
    }

    /// Runs a `position_texture` command on the selected faces (its `ids` filled in). False
    /// when no face is selected.
    fn position_selected(&mut self, mut command: Command) -> bool {
        let faces: Vec<EntityId> = self
            .doc
            .selection()
            .iter()
            .copied()
            .filter(|id| matches!(id, EntityId::Face(_)))
            .collect();
        if faces.is_empty() {
            return false;
        }
        if let Command::PositionTexture { ids, .. } = &mut command {
            *ids = faces;
        }
        self.message = None;
        self.reset_tool();
        self.run(command);
        self.invalidate_hover();
        true
    }
}

#[cfg(test)]
mod tests {
    use glam::DVec2;
    use session::SelectMode;

    use super::*;

    fn editor() -> Editor {
        Editor::new(DVec2::new(800.0, 600.0), 1.0)
    }

    /// Two 1 m squares on the ground; returns their faces.
    fn two_squares(e: &mut Editor) -> Vec<EntityId> {
        for x in [0.0, 2.0] {
            e.execute(Command::AddRectangle {
                corner: [x, 0.0, 0.0],
                opposite: [x + 1.0, 1.0, 0.0],
                normal: None,
            })
            .unwrap();
        }
        e.doc()
            .model()
            .faces()
            .map(|(id, _)| EntityId::Face(id))
            .collect()
    }

    fn select(e: &mut Editor, ids: Vec<EntityId>) {
        e.execute(Command::Select {
            ids,
            mode: SelectMode::Replace,
        })
        .unwrap();
    }

    fn placement(e: &Editor, face: EntityId) -> Option<TexturePlacement> {
        let EntityId::Face(f) = face else {
            unreachable!()
        };
        e.doc().model().face(f).unwrap().texture
    }

    #[test]
    fn the_fields_show_the_first_face_and_set_every_selected_face() {
        let mut e = editor();
        let faces = two_squares(&mut e);
        assert_eq!(e.selection_texture_position(), None, "nothing selected");
        select(&mut e, faces.clone());
        assert_eq!(
            e.selection_texture_position(),
            Some(TexturePlacement::default())
        );
        assert!(e.set_texture_offset([250, -500]));
        assert!(e.set_texture_rotation(455));
        assert!(e.set_texture_scale([2000, 500]));
        let expected = TexturePlacement {
            offset: [0.25, -0.5],
            rotation_deg: 45.5,
            scale: [2.0, 0.5],
        };
        for f in &faces {
            assert_eq!(placement(&e, *f), Some(expected));
        }
        assert_eq!(e.selection_texture_position(), Some(expected));
        // A bad value says why in the status and changes nothing.
        assert!(e.set_texture_scale([0, 1000]));
        assert!(e.status_parts().message.unwrap().contains("scale"));
        assert_eq!(placement(&e, faces[0]), Some(expected));
        assert!(e.reset_texture_position());
        assert!(faces.iter().all(|f| placement(&e, *f).is_none()));
        e.undo();
        assert_eq!(placement(&e, faces[1]), Some(expected));

        // Not faces only: no fields, and the setters do nothing.
        let edge = e
            .doc()
            .model()
            .edges()
            .map(|(id, _)| EntityId::Edge(id))
            .next()
            .unwrap();
        select(&mut e, vec![faces[0], edge]);
        assert_eq!(e.selection_texture_position(), None);
        select(&mut e, vec![edge]);
        assert!(!e.reset_texture_position());
    }

    fn undo_depth(e: &Editor) -> usize {
        match e.doc().query(&session::Query::History).unwrap() {
            session::QueryResult::History { undo_depth, .. } => undo_depth,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn rotate_90_turns_each_face_a_further_quarter_in_one_undo_step() {
        let mut e = editor();
        let faces = two_squares(&mut e);
        select(&mut e, vec![faces[0]]);
        e.set_texture_rotation(300);
        select(&mut e, faces.clone());
        let depth = undo_depth(&e);
        assert!(e.rotate_texture_90());
        assert_eq!(undo_depth(&e), depth + 1);
        assert_eq!(placement(&e, faces[0]).unwrap().rotation_deg, 120.0);
        assert_eq!(placement(&e, faces[1]).unwrap().rotation_deg, 90.0);
        e.undo();
        assert_eq!(placement(&e, faces[0]).unwrap().rotation_deg, 30.0);
        assert_eq!(placement(&e, faces[1]), None);
        // Past a full turn it comes back round.
        for _ in 0..4 {
            e.rotate_texture_90();
        }
        assert_eq!(placement(&e, faces[0]).unwrap().rotation_deg, 30.0);
        assert_eq!(placement(&e, faces[1]), None);
    }
}
````

Apply to `crates/app/src/ui.rs`:

````diff
diff --git a/crates/app/src/ui.rs b/crates/app/src/ui.rs
index a531b0d..eadb035 100644
--- a/crates/app/src/ui.rs
+++ b/crates/app/src/ui.rs
@@ -5,7 +5,7 @@ use std::collections::BTreeSet;
 
 use egui::collapsing_header::CollapsingState;
 use egui::{Align, Color32, Layout, Panel, RichText, Sense, Ui, Vec2};
-use kernel::{DefinitionKind, EntityId};
+use kernel::{DefinitionKind, EntityId, TexturePlacement};
 use session::{Lighting, MaterialId, OutlinerRow, OutlinerTable, Projection, StandardView};
 use tools::ToolKind;
 
@@ -72,6 +72,16 @@ pub enum UiAction {
     Harden,
     /// Make the selected edges soft or hard (Entity Info's Soft checkbox).
     SetSoft(bool),
+    /// Set the texture offset of the selected faces, in millimeters along u and v.
+    SetTextureOffset([i32; 2]),
+    /// Set the texture rotation of the selected faces, in tenths of a degree.
+    SetTextureRotation(i32),
+    /// Set the texture scale of the selected faces, in thousandths.
+    SetTextureScale([u32; 2]),
+    /// Turn the texture of each selected face a further quarter counter-clockwise.
+    RotateTexture90,
+    /// Clear the texture placement of the selected faces.
+    ResetTexturePosition,
 }
 
 /// A material as the Materials list shows it.
@@ -136,6 +146,8 @@ pub struct View<'a> {
     pub soften: Option<u8>,
     /// Whether the selected edges are all soft, when only edges are selected.
     pub edges_soft: Option<bool>,
+    /// The first selected face's texture placement, when only faces are selected.
+    pub texture_position: Option<TexturePlacement>,
 }
 
 /// The fly mode controls, shown over the Camera menu's Fly mode item.
@@ -399,12 +411,103 @@ fn right_panel(ui: &mut Ui, view: &View, actions: &mut Vec<UiAction>) {
             actions.push(UiAction::SetSoft(checked));
         }
     }
+    if let Some(t) = &view.texture_position {
+        texture_position(ui, t, actions);
+    }
     ui.separator();
     materials(ui, view, actions);
     ui.separator();
     outliner(ui, view, actions);
 }
 
+/// Entity Info's texture position for selected faces: offset, rotation and scale fields, each
+/// applied to every selected face when a drag ends or a typed value is entered, and the
+/// Rotate 90 and Reset buttons.
+fn texture_position(ui: &mut Ui, t: &TexturePlacement, actions: &mut Vec<UiAction>) {
+    ui.label("Texture position")
+        .on_hover_text("Where the texture lies on the selected faces");
+    let offset_range = -kernel::MAX_TEXTURE_OFFSET..=kernel::MAX_TEXTURE_OFFSET;
+    let mm = |m: f64| (m * 1000.0).round() as i32;
+    if let Some(offset) = drag_row(ui, "texture-offset", "Offset", t.offset, offset_range, 0.01)
+        && offset.map(mm) != t.offset.map(mm)
+    {
+        actions.push(UiAction::SetTextureOffset(offset.map(mm)));
+    }
+    let tenths = |deg: f64| (deg * 10.0).round() as i32;
+    if let Some([deg]) = drag_row(
+        ui,
+        "texture-rotation",
+        "Rotation",
+        [t.rotation_deg],
+        0.0..=360.0,
+        0.5,
+    ) && tenths(deg) != tenths(t.rotation_deg)
+    {
+        actions.push(UiAction::SetTextureRotation(tenths(deg)));
+    }
+    let scale_range = kernel::MIN_TEXTURE_SCALE..=kernel::MAX_TEXTURE_SCALE;
+    let thousandths = |s: f64| (s * 1000.0).round() as u32;
+    if let Some(scale) = drag_row(ui, "texture-scale", "Scale", t.scale, scale_range, 0.01)
+        && scale.map(thousandths) != t.scale.map(thousandths)
+    {
+        actions.push(UiAction::SetTextureScale(scale.map(thousandths)));
+    }
+    ui.horizontal(|ui| {
+        if ui
+            .button("Rotate 90")
+            .on_hover_text("Turn the texture a quarter counter-clockwise")
+            .clicked()
+        {
+            actions.push(UiAction::RotateTexture90);
+        }
+        if ui
+            .button("Reset")
+            .on_hover_text("Lay the texture as projected")
+            .clicked()
+        {
+            actions.push(UiAction::ResetTexturePosition);
+        }
+    });
+}
+
+/// A row of `label` and one field per value, changing by `speed` per point dragged. Like the
+/// texture size, the values being dragged live in egui's memory under `key` until the drag
+/// ends; returns the values when a drag ends or a typed value is entered.
+fn drag_row<const N: usize>(
+    ui: &mut Ui,
+    key: &str,
+    label: &str,
+    shown: [f64; N],
+    range: std::ops::RangeInclusive<f64>,
+    speed: f64,
+) -> Option<[f64; N]> {
+    let key = egui::Id::new(key);
+    let mut values = ui.data(|d| d.get_temp::<[f64; N]>(key)).unwrap_or(shown);
+    let (dragged, done) = ui
+        .horizontal(|ui| {
+            ui.label(label);
+            let (mut dragged, mut done) = (false, false);
+            for value in &mut values {
+                let field = ui.add(
+                    egui::DragValue::new(value)
+                        .range(range.clone())
+                        .speed(speed)
+                        .max_decimals(3),
+                );
+                dragged |= field.dragged();
+                done |= field.drag_stopped() || field.changed();
+            }
+            (dragged, done)
+        })
+        .inner;
+    if dragged {
+        ui.data_mut(|d| d.insert_temp(key, values));
+        return None;
+    }
+    ui.data_mut(|d| d.remove::<[f64; N]>(key));
+    done.then_some(values)
+}
+
 /// A small square of `color` that can be clicked.
 fn swatch(ui: &mut Ui, color: Color32) -> egui::Response {
     let (rect, response) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::click());
@@ -840,6 +943,7 @@ mod tests {
             selection: &NO_SELECTION,
             soften: None,
             edges_soft: None,
+            texture_position: None,
         };
         let (_, central) = run(&ctx, screen(), &view);
         let file = Pos2::new(14.0, central.min.y / 2.0);
@@ -929,6 +1033,7 @@ mod tests {
             selection: &NO_SELECTION,
             soften: None,
             edges_soft: None,
+            texture_position: None,
         };
         // The Camera menu sits right after File and Edit.
         let clicked = menu_actions(&view, 80..140);
@@ -983,6 +1088,7 @@ mod tests {
             selection: &NO_SELECTION,
             soften: None,
             edges_soft: None,
+            texture_position: None,
         };
         // The Edit button sits right after File (which ends before x = 40).
         menu_actions(&view, 40..80)
@@ -1034,6 +1140,7 @@ mod tests {
             selection: &NO_SELECTION,
             soften,
             edges_soft,
+            texture_position: None,
         }
     }
 
@@ -1104,6 +1211,53 @@ mod tests {
         );
     }
 
+    #[test]
+    fn entity_info_shows_texture_position_fields_for_selected_faces() {
+        let status = status();
+        let info = vec!["2 faces".to_string()];
+        let mut view = soften_view(&status, &info, None, None);
+        view.texture_position = Some(TexturePlacement::default());
+        let columns = [160.0, 120.0, 80.0];
+        let clicked: Vec<UiAction> = columns
+            .into_iter()
+            .flat_map(|x| panel_actions(&view, 1280.0 - x, None))
+            .collect();
+        for action in [UiAction::RotateTexture90, UiAction::ResetTexturePosition] {
+            assert!(clicked.contains(&action), "{action:?} in {clicked:?}");
+        }
+        // Typing 2 into each field sets that field alone, in its own units.
+        let typed: Vec<UiAction> = columns
+            .into_iter()
+            .flat_map(|x| panel_actions(&view, 1280.0 - x, Some("2")))
+            .collect();
+        for action in [
+            UiAction::SetTextureOffset([2000, 0]),
+            UiAction::SetTextureOffset([0, 2000]),
+            UiAction::SetTextureRotation(20),
+            UiAction::SetTextureScale([2000, 1000]),
+            UiAction::SetTextureScale([1000, 2000]),
+        ] {
+            assert!(typed.contains(&action), "{action:?} in {typed:?}");
+        }
+        // Without faces selected there are no texture fields.
+        view.texture_position = None;
+        let clicked: Vec<UiAction> = columns
+            .into_iter()
+            .flat_map(|x| panel_actions(&view, 1280.0 - x, Some("2")))
+            .collect();
+        assert!(
+            !clicked.iter().any(|a| matches!(
+                a,
+                UiAction::SetTextureOffset(_)
+                    | UiAction::SetTextureRotation(_)
+                    | UiAction::SetTextureScale(_)
+                    | UiAction::RotateTexture90
+                    | UiAction::ResetTexturePosition
+            )),
+            "{clicked:?}"
+        );
+    }
+
     #[test]
     fn panels_leave_a_central_view_inside_the_window() {
         let ctx = egui::Context::default();
@@ -1128,6 +1282,7 @@ mod tests {
             selection: &NO_SELECTION,
             soften: None,
             edges_soft: None,
+            texture_position: None,
         };
         let (actions, rect) = run(&ctx, screen(), &view);
         assert!(actions.is_empty());
@@ -1171,6 +1326,7 @@ mod tests {
             selection: &NO_SELECTION,
             soften: None,
             edges_soft: None,
+            texture_position: None,
         };
         f(&view)
     }
@@ -1339,6 +1495,7 @@ mod tests {
             selection: &NO_SELECTION,
             soften: None,
             edges_soft: None,
+            texture_position: None,
         };
         // Down the panel's left part: swatches and names, Add, the name field, the palette.
         let clicked = panel_actions(&view, 1280.0 - 160.0, None);
@@ -1469,6 +1626,7 @@ mod tests {
             selection: &NO_SELECTION,
             soften: None,
             edges_soft: None,
+            texture_position: None,
         }
     }
 
@@ -1759,6 +1917,7 @@ mod tests {
             selection,
             soften: None,
             edges_soft: None,
+            texture_position: None,
         }
     }
````

- [ ] **Step 2: Run the tests**

Run: `cargo test -p app texture_position`
Expected: 3 pass (`the_fields_show_the_first_face_and_set_every_selected_face`, `rotate_90_turns_each_face_a_further_quarter_in_one_undo_step`, `entity_info_shows_texture_position_fields_for_selected_faces`; the UI test takes about 5 s, like the other panel tests).

- [ ] **Step 3: Check and commit**

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
git status
git add crates/app/src/editor/texture_position.rs crates/app/src/editor.rs crates/app/src/ui.rs crates/app/src/app.rs
git commit -m "Entity Info texture position fields, Rotate 90 and Reset"
```

---

### Task 5: Agent API docs

**Files:**
- Modify: `docs/agent-api.md` (the repo's only API doc; the spec and `docs/testing.md` need no change)

**Interfaces:**
- Consumes: the behavior of Tasks 1 to 4.
- Produces: nothing for code.

- [ ] **Step 1: Make the changes**

Add `position_texture` to the command list and the `execute` row, replace the mirrored-back sentence under Textures, add a "Texture positions" subsection before Lights, and update Files (schema 9, open errors) and Export (texture coordinates):

Apply to `docs/agent-api.md`:

````diff
diff --git a/docs/agent-api.md b/docs/agent-api.md
index 26b09df..6429b49 100644
--- a/docs/agent-api.md
+++ b/docs/agent-api.md
@@ -31,7 +31,7 @@ all commands and results.
 
 | Method | Params | Result |
 |---|---|---|
-| `execute` | a Command, e.g. `{"type": "add_edge", "start": [0,0,0], "end": [1,0,0]}` | `{changes, revision}`, plus `import` for an `import` (see Import below) and `soften` for `soften_edges` and `soften_by_angle` |
+| `execute` | a Command, e.g. `{"type": "add_edge", "start": [0,0,0], "end": [1,0,0]}` | `{changes, revision}`, plus `import` for an `import` (see Import below), `soften` for `soften_edges` and `soften_by_angle`, and `positioned` for `position_texture` |
 | `batch` | `{"commands": [...], "atomic": true}` | `{outcomes, revision}`; atomic = one undo step, all or nothing |
 | `query` | a Query, see the table below | the QueryResult |
 | `render_view` | `{"width": 640, "height": 400, "camera": {"eye": [...], "target": [...]}}` (camera optional; it also takes `fov_y_deg`, `up` and `projection` as `set_camera` does; `lighting`, `day` or `night`, is optional and defaults to the document's) | `{width, height, png_base64}` |
@@ -64,7 +64,7 @@ of silently doing nothing).
 Commands: `add_edge`, `add_rectangle`, `add_circle`, `add_arc`, `push_pull`, `erase`, `move`, `copy`, `rotate`, `make_group`,
 `make_component`, `explode`, `open_instance`, `close_instance`, `open_path`, `rename_definition`,
 `purge_definitions`, `add_guide_line`, `add_guide_point`, `delete_guides`, `add_material`,
-`change_material`, `delete_material`, `paint`, `soften_edges`, `soften_by_angle`, `select`, `undo`,
+`change_material`, `delete_material`, `paint`, `position_texture`, `soften_edges`, `soften_by_angle`, `select`, `undo`,
 `redo`, `set_camera`, `orbit`, `pan`, `zoom`, `zoom_extents`, `standard_view`, `set_projection`,
 `set_lighting`, `new`, `open`, `save`, `export`, `import`.
 Units are meters, Z is up, angles in degrees, coordinates within +/-1,000,000 m.
@@ -237,13 +237,39 @@ runs along the face's horizontal direction (`u` = z x normal, normalized; along
 face) and its height across it (`v` = normal x u, up on a wall), repeating every `width` by
 `height` meters with a copy's bottom left corner at the definition's origin. Faces in one plane
 continue each other's pattern, and a component carries its pattern with it when moved, turned or
-scaled. Both sides of a face use the same placement, so from behind the image reads mirrored.
+scaled. A face's texture position (below) moves, turns and scales its texture on top of this.
+The back side shows the image as seen from behind, not mirrored: its `u` runs the other way, so
+the image's left edge is on the left for a viewer behind the face.
 The material's `color` stays the color edges, the Materials list and formats without textures
 show.
 
 The `materials` query adds `"texture": {"width": 0.6, "height": 0.3, "pixel_width": 1024,
 "pixel_height": 512, "format": "png"}` (`format` `png` or `jpeg`) to a material with a texture.
 
+#### Texture positions
+
+| Command | Effect |
+|---|---|
+| `{"type": "position_texture", "ids": ["f8", "f9"], "offset": [0.2, 0], "rotation_deg": 45, "scale": [2, 2]}` | Sets the texture position of the faces in `ids`, as one undo step: `offset` [u, v] moves the texture that many meters along the face's `u` and `v` (within +/-1e6 m), `rotation_deg` turns it counter-clockwise seen from the face's front, about the moved origin (any angle; kept from 0 up to 360), and `scale` [u, v] multiplies the size one copy covers along the image's own width and height (each 0.001 to 1000). Fields left out stay as they are; `"reset": true` first clears the position (then applies any fields given). At least one field or `reset` is required. Ids other than faces are `invalid_input` (naming them), as is an empty list or a value out of range; unknown ids are `not_found`. |
+
+The outcome reports `"positioned": n`, the faces whose position changed (also listed in
+`changes.modified`); a command that changes nothing makes no undo step. A face's `entity` result
+(and its entry in `entities`) has `"texture_position": {"offset": [0.2, 0.0], "rotation_deg": 45.0,
+"scale": [2.0, 2.0]}`, absent when the texture lies as projected (a position back at offset 0,
+rotation 0 and scale 1 is the same as none). The position belongs to the face, whatever material
+its sides have, and applies to both sides.
+
+Pieces of a face split by a new edge keep its position, as do copies, faces moved into a group or
+component or out of one by explode, and the faces of copied definitions. A push/pull cap keeps
+the pushed face's position (a sheet that turns over keeps what each side shows); the new sides
+and other new faces have none. Moving or turning faces keeps the numbers, so a moved face's
+pattern stays where the projection puts it.
+
+In the window, with only faces selected, Entity Info shows the first selected face's texture
+position: Offset (two fields, meters), Rotation (degrees) and Scale (two fields), each applied to
+every selected face when a drag ends or a typed value is entered, and Rotate 90 (turns each
+selected face's texture a further quarter counter-clockwise, as one undo step) and Reset.
+
 #### Lights
 
 A material with an `emission` above 0 glows. The `materials` query always includes `emission`
@@ -461,10 +487,12 @@ lists these controls) turns it on; Esc or F turns it off.
 ### Files
 
 Models are saved as `.skr` files: a zip archive holding `document.json`, which records `format`
-(`"tenon"`), `schema_version` (8 when a material glows or the view is lit by night, else 7 when a material has a
+(`"tenon"`), `schema_version` (9 when a face has a texture position, else 8 when a material glows or the view is lit by night, else 7 when a material has a
 texture, else 6 when a material is see-through, else 5; a material's `opacity` is written only
-when below 1 and its `emission` only when above 0, so a model whose materials are all opaque
-and dark is saved exactly as before opacity existed and older builds still open it; version 7
+when below 1 and its `emission` only when above 0, and a face's `texture_position` only when it
+has one, so a model whose materials are all opaque
+and dark is saved exactly as before opacity existed and older builds still open it; version 8
+and older files open without texture positions; version 7
 and older files open without glow; version 6 and older files open without textures; version 5 files open with
 every material opaque, version 4 files, written before materials, open unpainted, version 3 files, written before camera projections, open in perspective, and
 versions 2 and 1, written before curves and guides, still open), the camera (eye, target, field
@@ -489,8 +517,9 @@ reason (a missing file, not a tenon file, a missing or newer schema versi
 over 256 MiB, an id of 2^53 or more or one number used as the id of two things, a transform that cannot be inverted, a coordinate beyond the
 +/-1e6 m limit, a curve whose edges branch, fall apart or are straight, a soft edge bounding
 fewer than two faces, a face or instance painted with a material the file lacks, a material
-name that is blank, too long or used twice, a material opacity outside 0 to 1, or a texture
-whose image is missing, broken or too large, or whose size is out of range); the model is left as it was. `save` also fails
+name that is blank, too long or used twice, a material opacity outside 0 to 1, a texture
+whose image is missing, broken or too large, or whose size is out of range, or a face texture
+position out of range); the model is left as it was. `save` also fails
 when the model has given out ids past 2^53, which no file can hold. `open` and `new` never drop changes the user has
 not saved (`modified` in the `file` query) unless told to: they fail with `unsaved_changes`, and
 a client that means it, or has asked the user, passes `"discard": true`. The window asks the user
@@ -516,7 +545,7 @@ never overwrites a model; use `save`.
 
 Colors and textures come from each face's front side (its own material, else that of the nearest
 painted instance placing it); back-side materials are not exported. Texture coordinates follow
-the projection described under Textures. An OBJ export whose `.mtl` would be the export path
+the projection described under Textures with each face's texture position on top. An OBJ export whose `.mtl` would be the export path
 itself or the open model's file is refused with `invalid_input`. The image files are written
 first, then the `.mtl`, then the `.obj`, so a failure writing the `.obj` can leave new image
 files and a new `.mtl` beside the old (or no) `.obj`. A glTF material with a texture and an
````

- [ ] **Step 2: Check and commit**

```bash
cargo test --workspace
git status
git add docs/agent-api.md
git commit -m "Agent API docs: position_texture, texture positions, schema 9"
```

---

## After the last task

- Workspace tests: 1012 (992 before).
- Mark M9d done in `docs/superpowers/roadmap.md` with the first and last commit, and record the milestone (done items and anything deferred) in the project memory, as the roadmap rules ask.
- Not verified while planning: the app window was not launched (the Entity Info rows are tested through egui input, the rendering through headless renders on this machine's GPU); exported files were not opened in a glTF validator or another program (the tests check the coordinates written).
- Deferred candidates to record: moving or rotating faces does not carry their pattern along (placements stay in definition coordinates, like the M9b projection; SketchUp-style "texture follows the face" would adjust the offset and rotation on move and rotate); no interactive texture positioning (pins, dragging the texture in the view); Entity Info shows the first selected face's placement only (no "mixed" state for differing faces); a face without a placement turned over keeps none, so on a level face its front pattern turns half round; back-side materials are still not exported, so exported files show only the front's placement; no UV unwrapping or distortion (spec non-goals).
