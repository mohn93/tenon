# M18a Joints and Motion Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Parts of an assembly connected by joints, so that setting one joint (the crank angle) moves every connected part correctly: rods, pistons and anything else linked to them, including closed loops.

**Architecture:** A kernel `joint.rs`: `Joint` records stored in the `Scene` (like pages and styles, by `JointId`, written "j5"), each joining two root instances (or one instance and the ground) with a frame (origin, axis, reference direction) held in each part's own coordinates, a kind, a current value and optional limits. A kernel `solver.rs`: damped Gauss-Newton over the rigid transforms of the movable parts, with residuals per joint kind and one driver equation, starting from the current pose so it finds the nearest solution. Session commands add, change, delete and drive joints as undo steps; queries list them; `.skr` saves them. The window gets a Joints panel to drive a joint with a slider.

**Tech Stack:** Rust 2024, glam. No new dependencies.

**Spec:** `docs/superpowers/roadmap.md` row M18a.

## Global Constraints

- No new dependencies. Existing behaviour unchanged; golden images unchanged.
- New commands and fields: every registration site (execute/edit arms, undoable list, `batch::id_slots` with a joint id slot kind and `is_reference_key`, api describe, MCP schemas, `docs/agent-api.md`), `.skr` schema bumped to 25 only when the model has joints (old files load; the "newer file" stand-in tests move to 26).
- Gate per task (project rules): native gate + the wasm32 `cargo check` line; `scripts/build-web.sh` when the window changes.
- No emoji. Short commit messages, no trailers. Never `git add -A`.

## Spec notes (rulings)

1. **Joint model.** `Joint { name: String, kind: JointKind, a: Option<InstanceId> (None: the ground, fixed world), b: InstanceId, frame_a: JointFrame, frame_b: JointFrame, value: f64 (radians or meters), slide: f64 (cylindrical only), limits: Option<(f64, f64)> }`, `JointFrame { origin: DVec3, axis: DVec3 (unit), reference: DVec3 (unit, perpendicular to axis) }` in the part's own (definition) coordinates (for the ground: root coordinates). Joints join root instances only (a joint naming an instance inside a group is `invalid_input`: follow-up). A part may have any number of joints. Ids `JointId(u64)` from the scene's id counter, shown as "j<n>"; batch refs `$N:j0` from a new `ChangeSummary.joints` list (like materials and tags).
2. **Kinds and what they hold** (frames A and B in the world through each part's transform):
   - `revolute`: origins on one line along the axes and the axes parallel (same direction); free rotation about the axis; value = signed angle from A's reference to B's reference about A's axis.
   - `prismatic`: as revolute, and the references equal (no rotation); value = signed distance from A's origin to B's origin along the axis.
   - `cylindrical`: as revolute (rotation and sliding free); value = angle as revolute, `slide` = distance as prismatic.
   - `fixed`: frames equal.
   - `ball`: origins equal; free rotation (value unused). Useful for rod ends in skewed linkages.
3. **`add_joint { kind, a (instance id or null), b, point, axis, reference?, name?, limits? }`**: `point` and `axis` in the root's coordinates (the open context must be the root, else `invalid_input`); `reference` defaults to a direction perpendicular to the axis (the plane's u axis, as `add_circle` uses); both frames are derived from the parts' current transforms, so the joint holds as created and its value starts at 0 (the current pose is zero). `a == b`, an instance not in the root, a zero axis, a reference parallel to the axis: `invalid_input`. Limits in the API's units (degrees for angles, meters for distances), min < max, and 0 must lie within them. One undo step; outcome `changes.joints`. Default name "Joint <n>" (lowest free number), names unique (as scene names).
4. **`change_joint { joint, name?, limits? (null clears) }`**, **`delete_joint { joint }`**: one undo step each. Erasing an instance deletes its joints in the same undo step (also make_group/explode/solid tools that remove root instances: hook where scene positions are dropped, `drop_stale_positions`).
5. **`drive { joint, value, slide? }`** (degrees or meters; `slide` only for cylindrical): sets the joint's value (and slide), then solves every joint of the assembly the joint belongs to (the parts connected to it through joints) with the driven joint's value fixed and every other joint's value free, moving parts to meet all joints. Parts joined to the ground keep the ground's frame fixed; an assembly with no ground holds the driven joint's `a` part in place. Value outside the joint's limits, or no solution (residual above 1e-9 m / 1e-9 rad after the solver), is `invalid_input` ("the joints cannot all be met at 400 degrees") and changes nothing. On success: the moved instances' transforms and every joint's stored value updated, one undo step, outcome `modified` lists moved instances. A drive to the current value changes nothing (no undo step).
6. **Solver.** Unknowns: a small rigid update (rotation vector, translation) per movable part, applied to its current transform. Residuals: revolute/cylindrical 4 (two perpendicular offsets of B's origin from A's axis line, two components of B's axis off A's axis), prismatic 5 (+1 rotation about the axis via references), fixed 6, ball 3; driver 1 (angle or distance of the driven joint minus target). Damped least squares (Levenberg-Marquardt) with analytic or central-difference Jacobian, iterating to 1e-12 squared residual or 50 iterations; large drives are taken in steps of at most 10 degrees (or a tenth of the distance) from the current value so closed loops keep their branch (a piston stays on its side). Performance: 30 parts and 40 joints solve in under 20 ms in release (ignored perf test) and the V6-sized test runs in the normal suite.
7. **Queries.** `joints` lists `{ id, name, kind, a, b, value (degrees or m), slide?, limits?, point, axis }` with point and axis in root coordinates from part A (or the ground); `joint { id }` the same for one. `entities` instance info gains nothing.
8. **Scenes.** A scene storing positions (M17f) keeps working: going to it sets transforms and then sets every joint's stored value from the new pose (computed, not solved). No other scene change in this milestone.
9. **Window.** A Joints dock panel: the root's joints by name with kind and value; selecting one selects its two parts; a slider (degrees or the model's length units) drives it live while dragging (solving each frame) and makes one undo step on release; limits shown; delete button; an "Add joint" button is not required (agents and a later milestone add joints).

## Review Focus

1. Frames are stored in each part's own coordinates and stay right after the parts move, are copied, scaled or mirrored (a mirrored instance: frames derived from its transform still meet; test it).
2. Solver correctness on analytic linkages (slider-crank, four-bar), closed loops keeping their branch over a full turn, refusal leaving the model untouched, determinism.
3. Undo of drive restores every transform and value exactly; erase drops joints in the same step; file round trip.
4. Ids and batch references for joints; MCP and describe text.

---

### Task 1: Joint model, commands and queries

**Files:** `crates/kernel/src/(new) joint.rs`, `crates/kernel/src/{scene.rs, data.rs, lib.rs, model.rs (ChangeSummary.joints)}`, `crates/session/src/{command.rs, document.rs, (new) document/joints.rs, batch.rs, query.rs, file.rs}`, api describe test, `docs/agent-api.md`; tests in kernel and session.

- [ ] **Step 1: Write the failing tests** for rulings 1-4 and 7 (add each kind; frames in part coordinates; refusals; names; change and delete; erase drops joints with undo; `.skr` round trip at schema 25; `$0:j0`).
- [ ] **Step 2-5:** run, implement, gate, commit `git commit -m "Joints between parts"`.

### Task 2: The solver and drive

**Files:** `crates/kernel/src/(new) solver.rs`, `crates/kernel/src/joint.rs`, session `drive` command, scenes value update (ruling 8), `docs/agent-api.md`; tests (kernel unit for the solver; session integration).

- [ ] **Step 1: Write the failing tests** for rulings 5-6 and 8: slider-crank piston position against x = r cos(t) + sqrt(l^2 - r^2 sin^2(t)) at 0..360 in 15 degree steps (1e-9 m); a four-bar linkage against its analytic output angle; a V6-like crank with six rods and pistons on two banks driven a full 720 degrees and back, every piston on its bore axis; limits; an unreachable drive refused unchanged; undo exact; a drive to the same value makes no step; a floating assembly; a mirrored part; going to a scene with positions updates joint values.
- [ ] **Step 2-5:** run, implement, gate, commit `git commit -m "Drive joints with a solver"`.

### Task 3: Joints panel

**Files:** `crates/app/src/{ui/(new) joints.rs, ui/mod.rs, editor.rs or (new) editor/joints.rs, app.rs}`, docs; tests (app lib: panel lists joints, slider drag drives with one undo step on release, delete, selection of the two parts).

- [ ] **Step 1: Write the failing tests** for ruling 9.
- [ ] **Step 2-5:** run, implement, gate (+ build-web), commit `git commit -m "Joints panel"`.

### After the tasks (controller)

- Final review; push `dev`; CI green; roadmap row M18a done; spec bullet; memory; merge to main.
