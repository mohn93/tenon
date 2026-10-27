# M20d Import Image Limits Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Pictures up to 8192 pixels a side that still show on small GPUs, an Image tool that refuses early and with the placement's own message, base64 that tolerates what real encoders emit, and pictures that lie flush with the face below without flicker.

**Architecture:** `session::texture` raises the side limit to 8192 (all textures share it), keeps the total pixel budget, and gains one base64 reader used by `import_image` and `place_image`. `Document::texture_budget_allows` is the accessor the Image tool path calls. The render crate caps the texture array layer by the device's `max_texture_dimension_2d` (the `.skr` keeps the original bytes; the GPU gets a resampled copy), and `vs_face` nudges textured sides toward the camera by a small relative depth factor.

**Tech Stack:** Rust 2024. No new dependencies.

**Spec:** `docs/superpowers/roadmap.md` row M20d.

## Global Constraints

- No new dependencies; golden images unchanged; `docs/agent-api.md` updated. No emoji, no trailers, never `git add -A`.

## Spec notes (rulings)

1. **Side limit 8192** for every texture (materials, imports, `.skr`, `.skp`, 3MF), not only pictures. The file limit stays 16 MiB.
2. **Pixel budget stays 2^28 (1 GiB decoded)**: four full 8192 x 8192 pictures or sixteen 4096 x 4096. One picture at the limit decodes to 256 MiB; the budget keeps what a document holds bounded, whatever the side limit.
3. **GPU copy.** The array layer side is the largest image side rounded up to a power of two, at most `MAX_LAYER_SIDE` (raised from 1024 to 2048 so a big picture keeps detail) and at most the device's `max_texture_dimension_2d` (2048 under WebGL2 downlevel limits), and still within the 256 MiB array budget. Bigger images are box-filtered down on upload; the document keeps the original bytes.
4. **Texture budget accessor.** `Document::texture_budget_allows(pixels)` returns the same refusal text `import_image` gives; `place_image` calls it before the tool comes out.
5. **base64.** Whitespace anywhere is ignored, padding is optional, and the size limit counts decoded bytes (an upper bound from the compact length is checked before decoding).
6. **Flush pictures.** Lift defaults to 0. Render rule (revised in review): only a picture's face is pulled nearer, a picture being a group or component of exactly one face whose textured sides the scene marks with `PICTURE_LAYER` in the vertex's layer. `vs_face` pulls it through `biased_clip`, as edges are pulled, by 2^-12: that fraction of the distance in perspective (2.4 cm at 100 m) and of the eye-to-target distance in a parallel view (a bare scale of clip z there would move it by 2^-11 of the depth range, 200 distances, so about 5% of the distance: pictures showed through a face 1 m in front). 2^-12 is some four times the measured coplanar depth error (6e-5) and half the edges' pull (1.0005), so edges still draw over pictures; the first version's 2^-11 for every textured face left edges only 1.2e-5 ahead of textured faces and showed textured walls through untextured trim 2.9 cm in front at 60 m. Every face pipeline (colour, glass, shadow-free, AO normals) uses `vs_face`, so depth stays consistent. Picking raycasts the model: with lift 0 a ray hits the picture's group face and the face below at the same distance, so a tie (within 1e-6 m) goes to the later part (a group over the context's own face), in `nearest_face_part` and `probe::pick`; this also applies to coplanar faces of ordinary groups, which z-fight on screen anyway.
7. **No merge or split.** The picture is its own group definition, so its face is in a different mesh from the face below; a test shows the face below keeps its edges and area.

## Review Focus

1. The depth factor never shows textured faces through nearer geometry at ordinary gaps; all face pipelines agree.
2. Limits: no path decodes more than the budget; the 8192 case works end to end.
3. Picking prefers the picture on ties and nothing else.

---

### Task 1: Limits, base64, budget accessor

**Files:** `crates/session/src/texture.rs`, `crates/session/src/document/image.rs`, `crates/session/src/document/materials.rs`, `crates/app/src/editor/image.rs`, tests in `crates/session/tests/integration/` and `crates/app/src/host/tool_input_tests.rs`.

- [ ] Tests: 8192 accepted and 8193 refused; base64 with line breaks and unpadded accepted; limit counts decoded bytes; `place_image` refuses over budget with the import's message.
- [ ] Implement, gate, commit.

### Task 2: Render

**Files:** `crates/render/src/texture.rs`, `crates/render/src/shaders/scene.wgsl`, `crates/render/tests/integration/`.

- [ ] Tests: layer side capped by device limit; headless render of a picture over a coplanar face at several distances and in parallel projection shows the picture.
- [ ] Implement, gate, commit.

### Task 3: Lift 0, picking, docs

**Files:** `crates/session/src/document/image.rs`, `crates/session/src/command.rs`, picking code, `docs/agent-api.md`, roadmap.

- [ ] Tests: default lift 0; picking returns the picture; the face below keeps its geometry.
- [ ] Implement, docs, gate, commit.
