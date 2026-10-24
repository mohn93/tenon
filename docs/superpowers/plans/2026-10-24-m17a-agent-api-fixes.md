# M17a Agent API Fixes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Remove the API papercuts an agent hit building a detailed V6 engine over the socket: faces that ignore the requested normal, a refused zero rotation, unnamed groups, `add_material` failing on a known name, batch references that pick nested ids or erased ones, no way to place an instance at a transform, and `not_found` for ids that live in another context.

**Architecture:** Session-level changes to `Command` handling in `crates/session/src/document.rs` (and its `document/` modules), `crates/session/src/batch.rs` for references, two new commands (`place_instance`, `set_transform`) with a shared `Transform` input type, kernel helpers where a change needs mesh access (face orientation, instance transform). The api crate only needs schema/describe coverage, which follows from the session types.

**Tech Stack:** Rust 2024. No new dependencies.

**Spec:** `docs/superpowers/roadmap.md` row M17a.

## Global Constraints

- No new dependencies. Existing behaviour unchanged except where a ruling says so; golden images unchanged.
- New command variants get arms in `Document::execute`/`edit`, the undoable list, `session::batch::id_slots` (no `_` arm), api dispatch where needed and `docs/agent-api.md` in the task that adds them.
- Gate per task (project rules in the SDD rules file): native gate + `cargo check --target wasm32-unknown-unknown -p kernel -p session -p tools -p render -p api -p app --lib -p web`.
- No emoji. Short commit messages. Never `git add -A`.

## Spec notes (rulings)

1. **Faces follow an explicit normal.** `add_circle`, `add_polygon`, `add_rectangle` and `add_rotated_rectangle`: when the command is given an explicit `normal` (for the rotated rectangle: always, the normal is (edge_end - corner) x (width_point - corner)), every face the command creates that lies in the shape's plane is turned (materials kept on their sides, as `orient` in `push_pull.rs` does) so its normal points along that normal. Without an explicit `normal` nothing changes (canonical orientation as today). Faces the command splits from an existing face keep the existing face's orientation (only faces new to the mesh are turned). So `add_circle{normal:[0,0,-1]}` then `push_pull` with a positive distance extrudes downward. Tools are unaffected (they do not pass a normal through these commands, or if they do, check the tools tests still pass; the Circle tool on a face must keep matching that face).
2. **Zero rotation.** `rotate` whose angle is a whole number of turns (|angle mod 360| < 1e-9 degrees, after the existing finiteness check) without `copies` succeeds with an empty outcome, no undo step and no revision change. With `copies` it stays `invalid_input` (the copies would coincide). The same for `scale` with all factors exactly 1 (no copies there): an empty outcome.
3. **Named groups.** `make_group` takes an optional `name` (same limits as component names); the group's definition gets it (shown in the Outliner and the `entities` query `name`). Without it, the existing default name.
4. **`add_material { existing }`**: `"fail"` (default, today's behaviour when the name is taken), `"reuse"` (the existing material of that name is returned in `changes.materials`, nothing changes, no undo step), `"update"` (the existing material gets the given color/opacity/texture/emission as `change_material` would, one undo step, and is returned). `existing` needs a `name`; names compare exactly as the uniqueness check compares them today. Batch `$N:m0` resolves to the returned id in all three cases.
5. **Instance references count only top-level instances.** In batch references, `$N:iK` and `$N:i*` (and the `i` part of `$N:*`) consider only instances that command left in the open context (instances created inside a newly made or copied definition are not addressable from the open context and are skipped). `changes.created` is unchanged. The outcome gains `placed: [ids]`: the instances the command made in the open context in creation order (present for commands that make any: copy, rotate/scale with copies, make_group, make_component, explode, paste, insert_library_item, place_instance; absent otherwise).
6. **Wildcards skip erased ids.** At resolution time `$N:*`, `$N:e*`, `$N:f*`, `$N:i*` drop ids that no longer exist in the open context (erased or merged away by a later command of the batch); an indexed reference (`$N:f0`) to an id that no longer exists is `not_found` naming the reference. A wildcard that resolves to nothing gives an empty list (the command then decides: e.g. `erase {ids: []}` does nothing, `make_group {ids: []}` is `invalid_input` as today).
7. **`Transform` input.** One JSON shape used by both new commands: `{ "origin": [x,y,z] (default 0), "x_axis": [..] (default [1,0,0]), "y_axis": [..] (default [0,1,0]), "z_axis": [..] (default [0,0,1]) }` or `{ "matrix": [12 numbers] }` (column-major 3x4: x axis, y axis, z axis, translation, the same layout the `entities` query reports) - not both. All numbers finite, the origin within the coordinate limit, the axes' determinant at least 1e-12 in size (a mirror, negative determinant, is allowed; skew and scale are allowed, as instances already allow them).
8. **`place_instance { definition, transform, name? }`**: places a new instance of an existing definition (a component or a group's definition; a group definition placed this way becomes shared like a component's, the same as copying a component) in the open context at `transform`, one undo step; `changes.created` and `placed` name it. Unknown definition is `not_found`; placing a definition inside itself (directly or through nesting, the open path contains an instance of it) is `invalid_input`. `name` is reserved for M17b and absent here (do not add it).
9. **`set_transform { id, transform }`**: replaces an instance's transform in the open context, one undo step, outcome `modified`. Not an instance of the open context: as other commands (rule 10 for the message).
10. **Ids in another context.** When a command names an id (directly or through a resolved reference) that does not exist in the open context but exists elsewhere in the model, the `not_found` message says so and gives the instance path to open, e.g. `s129810 is inside i95934 > i85590; open that path first (open_path)`. Applies everywhere `not_found` / "not in the open context" is reported for entity ids in session commands (a shared helper, not per-command code). For an id in a definition with several instances the path is any one of them.

## Review Focus

1. Ruling 1 orients only the command's new faces, never existing ones; holes (a circle inside a face) keep the surrounding face correct; tools tests and golden images unchanged.
2. Rulings 5-6 keep existing batch tests green and handle `$N:*` mixing kinds; no panic on references past the end.
3. Transform validation never panics (NaN, huge, singular), round-trips with the `entities` query format, and undo restores the old transform exactly.
4. `place_instance` cannot create a cycle; `existing: update` makes one undo step and `reuse` none.

---

### Task 1: Faces along the normal, zero rotation, named groups, material reuse

**Files:** `crates/session/src/{command.rs, document.rs, document/*.rs as needed, batch.rs (id slots only if fields change)}`, `crates/kernel/src/{edit.rs or push_pull.rs (expose a pub(crate)/pub orient helper)}`, `docs/agent-api.md`; tests in `crates/session/tests/integration/`.

- [ ] **Step 1: Write the failing tests** for rulings 1-4 (circle/polygon/rectangle with normal -Z and -Y then push_pull positive goes that way; circle inside an existing face keeps the outer face's orientation; rotate 0 and 360 no-op without undo step, with copies refused; scale [1,1,1] no-op; make_group name in the entities query; add_material reuse/update/fail and batch `$0:m0`).
- [ ] **Step 2-5:** run (fail), implement, gate, commit `git commit -m "Faces follow the given normal; zero rotation, group names, material reuse"`.

### Task 2: Batch references and placed instances

**Files:** `crates/session/src/{batch.rs, command.rs (Outcome.placed), document.rs}`, `docs/agent-api.md`; tests in session and api integration suites.

- [ ] **Step 1: Write the failing tests** for rulings 5-6 (copy of a group containing groups: `$0:i0` is the copy, `placed` lists only it; rotate copies `placed` order; `$0:*` after a later erase of one of its faces; indexed ref to an erased id is `not_found`).
- [ ] **Step 2-5:** run, implement, gate, commit `git commit -m "Batch references skip nested and erased ids; outcome placed"`.

### Task 3: place_instance and set_transform

**Files:** `crates/session/src/{command.rs (Transform type, two variants), document.rs or a new document/transform.rs, batch.rs id_slots}`, kernel instance helpers if needed, `crates/api` schema tests if they enumerate commands, `docs/agent-api.md`; tests.

- [ ] **Step 1: Write the failing tests** for rulings 7-9 (axes form and matrix form; query round trip; singular/NaN/huge refused; undo; cycle refused; group definition placed twice then edited shows in both).
- [ ] **Step 2-5:** run, implement, gate, commit `git commit -m "place_instance and set_transform"`.

### Task 4: Ids in another context

**Files:** `crates/session/src/{document.rs, document/*.rs}` (a shared lookup helper that finds an entity's instance path), `docs/agent-api.md`; tests.

- [ ] **Step 1: Write the failing tests** for ruling 10 (erase, move, flip_section_plane and paint with an id inside a nested group: message names the path; an unknown id keeps today's message).
- [ ] **Step 2-5:** run, implement, gate, commit `git commit -m "not_found names the path of ids in other contexts"`.

### After the tasks (controller)

- Final review; push `dev`; CI green; roadmap row M17a done; spec bullet; memory; merge and push; next milestone (M17b).
