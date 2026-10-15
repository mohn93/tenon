# Roadmap

The milestones left to build the v1 in the spec (`specs/2026-10-10-sketchup-alt-design.md`), in order. The autonomous loop works through them top to bottom. For each one it:
1. scopes it in the spec (amending the milestone list);
2. writes a plan with superpowers:writing-plans, verifying the code in a scratch copy;
3. executes the plan with superpowers:subagent-driven-development (a task review after each task and a final review);
4. marks it done here.

Status values: `todo`, `planned <plan file>`, `done <first>..<last commit>`.

| Milestone | Scope | Status |
|---|---|---|
| M1-M5b | viewport, kernel, agent API, drawing tools, egui shell, selection, move/copy, openings, groups and components (model and tools) | done |
| M5c | Files: `.skr` save/open/new as commands, FILE argument, File menu with native dialogs and shortcuts, unsaved-changes prompts; rename and purge definitions | done 791774f..bc88ce9 |
| M5d | Export OBJ, STL, glTF as commands and from the File menu | done 5d98f77..ff298be |
| M6a | Circle tool and `add_circle` command (segment count; `c` = Circle, Shift+G = component) | done a748c20..0e40c5e |
| M6b | Rotate command and tool (rotate vertices and instances about an axis; rotated placements: normals, picking, tool camera and bounds in rotated instances) | done 3d1c782..9d8e1ee |
| M6c | Tape Measure tool: guide lines and points (session-level guides, inference snaps to them, commands to add/remove) | done 5bd8d1d..82b4e07 |
| M6d | Curves: tag edge chains from circles (and arcs) as curves; select a whole curve, Entity Info shows it, soft side edges on pushed curves; Arc tool and command | done 76ae9e3..8b3b3d4 |
| M7a | Camera: orbit, zoom extents, standard views and orthographic projection as commands and UI | done 83d8f17..afd4001 |
| M7b | Materials: front/back face colors, Paint tool, material list in the UI, materials in `.skr` and export | done d05ff34..19429bf |
| M7c | Outliner panel (groups/components tree, select and open from it) | done b63d32f..44d13dd |
| M8a | Import OBJ and STL as commands and from the File menu (into the open context or as a group) | done ebdf2f6..f4ac453 |
| M8b | Golden-image render tests (headless renders compared with stored images) and toolbar wrapping | done 33e0ae5..3acc26d |
| M8c | Clean-up of deferred correctness and performance items recorded in memory (copy/stencil speed, BVH rebuild on paint, float round trip in files, fixed tmp paths in tests, and the like) | done 8124607..ee7b754 |
| M8d | Clean-up of deferred UX and API polish items recorded in memory | done 3575840..7a060a9 |
| M9a | Soften and smooth: soften_edges and soften_by_angle commands, smooth shading across soft edges, glTF smooth normals, Edit > Soften Edges... and Entity Info soft toggle | done ee07743..e24aed0 |
| M9b | Textures: image textures on materials (embedded in .skr), planar projection, texture-array rendering, glTF/OBJ export, Materials panel | done 8f34616..ffe9b1f |
| M9c | Lights: glowing materials (emission), point lights from glowing instances, day/night lighting mode | done 9b27327..110673a |
| M9d | Texture positioning: per-face offset, rotation and scale (`position_texture`), back sides not mirrored, export UVs, Entity Info fields | done 726a96d..45b0d23 |
| M9e | Light shadows: cube shadow maps for up to 64 lights, cached between frames, glowing faces cast none | done 0561d71..a7037f4 |
| M10a | Agent sight and state: pick and infer queries, editor state query and setters, selection/camera/editor notifications, undo labels, quit | done 29a3982..584fff4 |
| M10b | Tool driving and parity: tool_input events through the real tools and inference; orbit/zoom about a point, crossing box select, 3-point arc, relative texture rotation, undo/redo count | done e2c645f..f69b7f0 |
| M10c | Safe agent access: connected-agent indicator and activity log, opt-in --api-root folders, overwrite guard, no file contents in errors, recovery file on discard | planned plans/2026-10-15-m10c-safe-access.md |

## Rules for the loop

- Work on `dev`; never push, merge to `main`, deploy, or publish. The user pushes.
- One milestone at a time. A milestone too big for one plan is split (like M4 and M5) and the split is recorded in the spec and in this table.
- Decisions the spec does not settle are made as rulings, written into the plan's "Spec notes" and the ledger, and reported to the user at the end of the milestone.
- Each milestone ends with: all tests passing, clippy `-D warnings` clean, fmt clean, the table above updated, the project memory updated, and a short report to the user.
- Stop and ask the user only for: an irreversible or destructive operation, a security-sensitive action, anything outside the repo (push, publish, accounts), a plan so broken that every path is a guess, or a new dependency whose license or weight is questionable.
