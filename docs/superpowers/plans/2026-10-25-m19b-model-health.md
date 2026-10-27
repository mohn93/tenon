# M19b Model Health and Make Solid Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Say which parts are not solids and why, and close the ones that can be closed safely in one undo step, so parts can be printed, exported as 3MF, measured and used in Solid Tools.

**Architecture:** A kernel `health.rs` with two entry points on one mesh (a definition's own geometry): `check` lists the problems (open edges, edges shared by more than two faces, stray edges, faces turned inside out, tiny gaps, internal faces, no volume), and `repair` runs the fixes on a copy (weld gaps, remove stray edges and internal faces, orient, fill planar holes, orient outward) and hands the copy back only when `check` finds it a solid afterwards. Session: a `model_health` query over the open context's groups and components (each with everything it places at any depth, as `interference` treats parts) and a `make_solid` command that repairs each definition once, all edits joined into one undo step. App: Tools > Model Health opens a small window listing the parts that are not solids with their problems; a row selects and zooms to the part; Make Solid repairs the listed parts and refreshes the list.

**Tech Stack:** Rust 2024. No new dependencies.

**Spec:** `docs/superpowers/roadmap.md` row M19b.

## Global Constraints

- No new dependencies; golden images unchanged; `docs/agent-api.md`; Python client helpers next to the other solid checks.
- Gate per task (project rules). No emoji. No trailers. Never `git add -A`.

## Spec notes (rulings)

1. **Parts.** Both `model_health { ids? }` and `make_solid { ids? }` take groups and components of the open context (every one when `ids` is empty); a part is checked with every definition it places at any depth, each definition once. Loose geometry of the open context is not a part and is never changed: removing "stray" edges there would erase drawing lines and guides-in-progress. A definition without faces or edges (an assembly holding only instances) has nothing of its own to check; a part with no faces anywhere is reported as `no_faces`.
2. **Problems** (per definition, ids of that definition's entities, in id order): `open_edges` (an edge bounding one face), `shared_edges` (more than two faces), `stray_edges` (no face), `reversed_faces` (faces turned inside out: in a closed shell those facing in, in an open shell the side that has less area when the shell's faces disagree; a shell is the faces reached across edges with exactly two faces), `gaps` (pairs of vertices on open edges nearer than the weld tolerance, not joined by an edge and not on one face), `internal_faces` (see 5), `no_volume` (closed but enclosing no volume). A definition is a solid exactly when `kernel::solid::open_reason` finds nothing and the volume is more than 1e-12, as `instance_volume` and `interference` decide; a closed shell facing inward everywhere is a solid (as before) and is not reported.
3. **Weld tolerance.** `max(1 µm, min(1e-5 × the definition's bounding-box diagonal, 1 mm))`, and below a quarter of the shortest edge at either vertex, so a short real edge is never collapsed. 1 µm is the kernel's own `LENGTH_TOL`: anything the kernel treats as one point is always welded. Only vertices on open edges weld, into the lower id; faces that used the welded vertex get their normal refitted and must stay flat within `LENGTH_TOL`, else the part is refused.
4. **Holes.** The open sides are traced into loops (a vertex where loops meet is refused: which way round is a guess). A loop is filled only when it is planar: every vertex within `LENGTH_TOL` of the plane through its first vertex with its Newell normal, the flatness every face in the model keeps. Coplanar loops nest: a loop inside one coplanar loop facing the other way becomes its hole (the annulus on a tube); any deeper or same-facing nesting is refused. A non-planar or degenerate loop is reported in `reasons`, not guessed (no triangulated patch).
5. **Internal faces.** A face every one of whose edges bounds three or more faces (a wall between two stacked boxes, a partition inside a box). Removing them is the only fix for `shared_edges`; edges still shared by more than two faces afterwards (two boxes meeting along one edge) are reported, never "fixed".
6. **Orientation.** Before filling, each shell is made to agree (the side with less area turns); after filling, each closed shell is turned to face outward (positive volume). `faces_flipped` counts faces whose final side differs from the original.
7. **All or nothing per definition.** The fixes run on a copy; the definition changes only when the copy is a solid afterwards (re-checked) and every face touched or made is valid (flat, wound right, triangulable). Otherwise it is left exactly as it was and the entry has `fixed: false`, the problems still found after the attempt in `left`, and why in `reasons`. A definition that is already a solid is never touched. Every changed definition fixes every instance of it (components included), and all changes are one undo step `make_solid`; nothing changed makes no step.
8. **Report.** `make_solid` answers `make_solid: {entries: [{definition, parts, fixed, welded, strays_removed, internal_removed, holes_filled, faces_flipped, left, reasons}], solid}`: one entry per definition that was not a solid, `parts` the named parts holding it; `solid` the number of definitions that already were. `model_health` answers `{parts: [{id, name, fixable, problems: [{definition, kind, ids, text}]}], checked, revision}`, listing only parts that are not solids; `fixable` is whether `make_solid` would close every definition of it (the repair is run on copies).
9. **Window.** Tools > Model Health: runs `model_health` on the selected parts (all when none is; loose selection asks for parts; refused inside an open context like Check Interference), lists each part that is not a solid with its problems (`Box (i4): 1 open edge, 1 face inside out`), a row selects the part and zooms to it, and a Make Solid button (when any part is fixable) runs `make_solid` on the listed parts and re-checks; the status line says what was fixed.

## Review Focus

1. Never changing a solid; never committing a non-solid; the all-or-nothing copy and one undo step.
2. Hole tracing and nesting (pinched loops, annuli, non-planar loops), orientation choice, weld guards.
3. Every instance of a component fixed once; nested definitions.

---

### Task 1: Kernel check and repair

**Files:** `crates/kernel/src/(new) health.rs`, `crates/kernel/src/weld.rs` (weld split so the vertex merge runs alone), `crates/kernel/src/lib.rs`, `crates/kernel/tests/integration/(new) health.rs`; tests: a box with one face missing is filled; a box with a face flipped is oriented; two boxes sharing one edge four ways are reported, not fixed; a stray edge on a box is removed; vertices 1e-7 apart are welded; a sphere already solid is untouched; a tube missing its annular top gets an annulus with a hole; a non-planar hole is refused; a partition inside a box is removed; a lone face is refused (no volume).

- [ ] **Step 1-5:** tests, implement, gate, commit `git commit -m "Model health check and repair in the kernel"`.

### Task 2: model_health query and make_solid command

**Files:** `crates/session/src/(new) document/health.rs`, `crates/session/src/query.rs`, `crates/session/src/command.rs`, `crates/session/src/document.rs`, `crates/session/src/lib.rs`, `crates/api/tests/integration/dispatch.rs`, `crates/session/tests/integration/(new) health.rs`, `docs/agent-api.md`, `clients/python/tenon/model.py` and its tests; tests: the query lists a broken box and not a solid one; make_solid fixes a component once for all its instances, reports, and undo restores the original in one step; a part with nothing fixable makes no undo step; loose ids are invalid input.

- [ ] **Step 1-5:** tests, implement, gate, commit `git commit -m "model_health query and make_solid command"`.

### Task 3: Model Health window

**Files:** `crates/app/src/editor/(new) health.rs`, `crates/app/src/editor.rs`, `crates/app/src/app.rs`, `crates/app/src/menus.rs`, `crates/app/src/ui/dialogs.rs`, `crates/app/src/ui/mod.rs`, `crates/app/src/ui/tests/(new) health.rs`, `crates/app/src/ui/tests/{mod,menus}.rs`, `docs/agent-api.md` (window table); tests.

- [ ] **Step 1-5:** tests, implement, gate, commit `git commit -m "Model Health window"`.

### After the tasks (controller)

- Final review; roadmap row M19b done; merge.
