# M19d Import Image Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Place a PNG or JPEG as a picture on the ground or on a face: a textured rectangle in its own group, sized by dragging or typing a width, kept in `.skr` and exported as a textured face.

**Architecture:** A session command `import_image` builds a one-face import tree (the rectangle in the open context's coordinates), imports it as a group named after the file, adds a material carrying the decoded image as its texture, paints both sides, and positions the texture (placement offset and rotation) so the image fills the face exactly. An Image tool in `tools` (placed like the Paste tool, over the current tool) follows the cursor, takes the plane of the face or ground under the first click, sizes the picture keeping its aspect ratio and emits the command. File > Import accepts `.png`, `.jpg`, `.jpeg` and starts that tool; on the web the bytes come from the upload.

**Tech Stack:** Rust 2024. No new dependencies (the `image` and `base64` crates are already used by textures and clips).

**Spec:** `docs/superpowers/roadmap.md` row M19d. Builds on M9b textures, M9d texture positions, M16a/M16c placement with the Paste tool.

## Global Constraints

- No new dependencies; golden images unchanged; `docs/agent-api.md` and the Python client updated; one undo step per image.
- Gate per task (project rules). No emoji. No trailers. Never `git add -A`.

## Spec notes (rulings)

1. **`import_image { path | data, origin, normal?, x_axis?, width, name?, lift? }`.** `path` is a PNG or JPEG file (checked against the API roots like a texture path); `data` is the file's bytes in base64 (for the web build and agents without file access); exactly one is given. `origin` is the picture's lower left corner, `normal` (default Z) the way the front faces, `x_axis` (default the texture's own horizontal direction for that normal, as `add_rectangle` and textures use) the way the image's width runs, projected into the plane. `width` is in meters (0.001 to 1000, the texture limits); the height follows the image's aspect ratio.
2. **Result.** One group named `name` (default the file's stem, or "Image") holding one rectangle face painted front and back with a new material of the same name (made unique) whose texture is the image, `width` by height meters. The placement (offset and rotation) makes the image's corners land on the face's corners: front UVs (0,0), (1,0), (1,1), (0,1). The back is not mirrored (M9d's back rule). The group is selected and listed in `placed`; `definitions` holds the new definition.
3. **Offset ruling.** The picture lies `lift` meters (default 0.001, 0 to 1) along `normal` from the plane through `origin`, so a picture put on a face (origin on its plane) does not fight it for the depth buffer. The tool uses the default on faces and on the ground.
4. **Caps.** The existing texture limits apply and are stricter than the roadmap's 8192: at most 4096 pixels a side and 16 MiB of file, and the shared 2^28 total pixel budget. A refused image says which limit it broke (`file` error for a path, `invalid_input` for `data`).
5. **Tool.** File > Import with a `.png`/`.jpg`/`.jpeg` file starts the Image tool over the current tool (Escape or the finishing click goes back, as with Paste). Before the first click a one-meter wide outline of the picture follows the cursor on the plane under it (inference on). The first click sets the lower-left or any corner; moving sizes the picture keeping its aspect ratio (the corner stays put, the picture grows toward the cursor); a typed length sets the width and finishes; the second click finishes. The sides follow the drawing axes as the Rectangle tool's do. Scale resizes a placed picture later (it is a group).
6. **Web.** The import works from upload bytes (`Source::Bytes`): the bytes are kept for the tool and sent as `data`.

## Review Focus

1. UVs at the corners on tilted and rotated planes; the back side not mirrored; no z-fighting.
2. Caps enforced before decoding; `.skr` round trip keeps the image bytes; glTF carries the texture.
3. Tool state (Escape, undo while active), typed width, and the file filter.

---

### Task 1: The `import_image` command

**Files:** `crates/session/src/command.rs`, `crates/session/src/document.rs`, `crates/session/src/document/image.rs` (new), `crates/session/src/batch.rs`, `crates/api/src/roots.rs`, `docs/agent-api.md`; tests in `crates/session/tests/integration/import_image.rs`.

- [ ] One group, one face of the right aspect and size, front UVs at the corners; tilted face plane and lift; round trip through `.skr`; glTF export carries the texture; oversize and broken images refused; `data` and `path` both given refused.

### Task 2: The Image tool and File > Import

**Files:** `crates/tools/src/image.rs` (new), `crates/tools/src/lib.rs`, `crates/app/src/editor/clip.rs` (or a new `editor/image.rs`), `crates/app/src/editor.rs`, `crates/app/src/flows.rs`; tests: tool unit tests, `tool_input` place, size and finish, typed width, Escape.

- [ ] Tool tests, editor entry point, import filter and flow, headless `tool_input` coverage.

### Task 3: Docs, Python client, roadmap

**Files:** `docs/agent-api.md`, `clients/python`, `docs/superpowers/roadmap.md`.
