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
| M5c | Files: `.skr` save/open (zip with versioned JSON, schema version), new; export OBJ, STL, glTF; `new`/`open`/`save`/`export` commands; native dialogs (rfd) and menu/shortcuts; purge unused component definitions; rename definition command | todo |
| M6a | Circle tool and command (edge chains tagged as curves, segment count); arcs if cheap | todo |
| M6b | Rotate command and tool (rotate vertices and instances about an axis; rotated placements: normals, picking, tool camera and bounds in rotated instances) | todo |
| M6c | Tape Measure tool: guide lines and points (session-level guides, inference snaps to them, commands to add/remove) | todo |
| M7a | Camera: orbit, zoom extents, standard views and orthographic projection as commands and UI | todo |
| M7b | Materials: front/back face colors, Paint tool, material list in the UI, materials in `.skr` and export | todo |
| M7c | Outliner panel (groups/components tree, select and open from it) | todo |
| M8 | Import OBJ and STL; golden-image render tests; clean-up of deferred minors recorded in memory | todo |

## Rules for the loop

- Work on `dev`; never push, merge to `main`, deploy, or publish. The user pushes.
- One milestone at a time. A milestone too big for one plan is split (like M4 and M5) and the split is recorded in the spec and in this table.
- Decisions the spec does not settle are made as rulings, written into the plan's "Spec notes" and the ledger, and reported to the user at the end of the milestone.
- Each milestone ends with: all tests passing, clippy `-D warnings` clean, fmt clean, the table above updated, the project memory updated, and a short report to the user.
- Stop and ask the user only for: an irreversible or destructive operation, a security-sensitive action, anything outside the repo (push, publish, accounts), a plan so broken that every path is a guess, or a new dependency whose license or weight is questionable.
