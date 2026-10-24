# M18b Couplings and Cams Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make a whole engine follow its crank: gear and chain ratios between joints (crank to cams 2:1), cam followers whose lift comes from the cam's real surface, playing a joint over a range in the window, frames at joint values for agents, and scenes that store joint values.

**Architecture:** Builds on M18a's `Joint` records and solver. A `Coupling { driver: JointId, driven: JointId, ratio, offset }` adds the linear equation `driven = ratio * driver + offset` to the solver. A `CamFollower { cam: InstanceId, follower: JointId (prismatic), direction }` sets the follower joint's value each solve from where the follower's axis line meets the cam's mesh (the extreme contact along the axis, with a roller radius option), evaluated after the rotational parts move (a second solver pass, or as a constraint with numeric derivative). `play` animates a joint in the window; `render_view` takes joint values; pages store joint values.

**Tech Stack:** Rust 2024. No new dependencies.

**Spec:** `docs/superpowers/roadmap.md` row M18b. Depends on M18a.

## Global Constraints

- No new dependencies; golden images unchanged; every registration site; `.skr` schema bump only when couplings, cams or scene joint values exist; `docs/agent-api.md`.
- Gate per task (project rules). No emoji. No trailers. Never `git add -A`.

## Spec notes (rulings)

1. **`add_coupling { driver, driven, ratio, offset? }`** (joints; ratio finite non-zero, |ratio| <= 1000; offset in the driven joint's units, default: the value that holds now, so adding a coupling never moves anything). Revolute-to-revolute (gears, chains, belts: 2:1 for crank to cam is ratio 0.5), revolute-to-prismatic (rack: meters per degree) and prismatic-to-revolute. A coupling cycle that contradicts itself is refused at drive time with the solver's message. `delete_coupling`; erasing a joint deletes its couplings. Queries list couplings. One undo step each; `$N:c0` refs.
2. **`add_cam_follower { cam (instance), follower (a prismatic joint whose part rides on the cam), roller_radius? (default 0: a flat or knife follower contacting at its axis line), preload_direction? (default: toward the cam) }`**: during a solve, after the other constraints are met, the follower joint's value is set so the follower's contact point (its frame origin offset by the roller radius toward the cam) touches the cam's surface along the joint axis: the nearest intersection of the axis line (shifted by the roller radius as an offset surface: approximate by testing the line against the cam mesh and taking the extreme point plus the radius along the line) with the cam mesh in the cam's current pose. No contact along the line: `invalid_input` at drive time naming the follower. Query lists followers.
3. **Valve lift test case.** A cam built with `revolve` or primitives (a base circle plus a lobe), a follower on a prismatic joint above it, cam on a revolute joint coupled 0.5 to a crank: driving the crank 0..720 gives one lift event per 720 degrees whose peak equals the lobe height within the facet error.
4. **`play { joint, from, to, seconds, loop? }`** (window only, an editor command): animates the joint value, solving each frame, drawing-only like scene transitions (M17f); input stops it, leaving the model at the start (no undo step). Agents use `render_view { joints: { "j3": 120 } }`: renders with those joint values solved on a cloned scene, without changing the document (refused if they cannot be met).
5. **Scenes store joint values** (`add_scene`/`update_scene`/`change_scene` with `joints: true|false|[ids]`): going to such a scene drives those joints (one undo step with any positions); transitions interpolate joint values and solve each frame instead of interpolating transforms of parts moved by joints.

## Review Focus

1. Coupled solve correctness (gear trains, sign conventions, offsets), cam contact on faceted cams (no jitter between facets beyond the facet error), refusal when no contact.
2. play and render_view joints never change the document; scenes with joint values round trip and undo.

---

### Task 1: Couplings

**Files:** `crates/kernel/src/{joint.rs, solver.rs}`, session commands and queries, `docs/agent-api.md`; tests (gear train 2:1 over 720 degrees; rack; offset preserves the pose; contradiction refused).

- [ ] **Step 1-5:** tests, implement, gate, commit `git commit -m "Joint couplings"`.

### Task 2: Cam followers

**Files:** `crates/kernel/src/{(new) cam.rs, solver.rs}`, session commands and queries, `docs/agent-api.md`; tests (ruling 3; roller radius; no contact refused).

- [ ] **Step 1-5:** tests, implement, gate, commit `git commit -m "Cam followers"`.

### Task 3: Play, frames and scene joint values

**Files:** app editor/animation (reuse the M17f transition frame machinery), api `render_view` params, session scenes, `docs/agent-api.md`; tests.

- [ ] **Step 1-5:** tests, implement, gate (+ build-web), commit `git commit -m "Play joints, joint frames and scene joint values"`.

### After the tasks (controller)

- Final review; push; CI; roadmap row M18b done; spec bullet; memory; merge.
