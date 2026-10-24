# M17e Rendering for Illustrations Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make renders of mechanical and product models read like cutaway illustrations: metal that looks like metal, depth from ambient occlusion, studio lighting for objects floating in space, section fills in the cut part's color, parts that a section plane leaves whole, and `render_view` options to set all of it per render.

**Architecture:** Material gains `metalness` and `roughness`, sent per face to `scene.wgsl` and shaded with a specular term (headlight, sun, studio lights) and a sky/ground environment term; defaults give exactly today's shading. A `Studio` lighting mode with camera-relative key, fill and rim lights. An SSAO pass (depth and normals, half resolution, bilateral blur) toggled by the style. `sections.rs` fill color from the cut part's material when the style says so; an instance flag excludes it from outer planes' cuts. `render_view` (and the MCP render tool) take overrides for one render without changing the document.

**Tech Stack:** Rust 2024, wgpu 30, WGSL. No new dependencies.

**Spec:** `docs/superpowers/roadmap.md` row M17e.

## Global Constraints

- No new dependencies. Defaults keep every existing golden image unchanged (new settings default off/neutral).
- WebGL2 fallback: anything it cannot do is switched off there with the existing "not available in this browser" style of message; the wasm check and `scripts/build-web.sh` pass.
- New commands and fields: every registration site, `.skr` save/load (old files load with defaults), `docs/agent-api.md`, in the task that adds them.
- Gate per task (project rules): native gate + the wasm32 `cargo check` line; `scripts/build-web.sh` in tasks touching shaders or the window.
- No emoji. Short commit messages. Never `git add -A`.

## Spec notes (rulings)

1. **Material finish.** `Material` gains `metalness` (0..1, default 0) and `roughness` (0..1, default 1). `add_material`/`change_material` take both; Materials panel gets two sliders; `.skr` saves them (absent = default); `.skp` import and export ignore them (not in the format; documented). Shading: with roughness 1 and metalness 0 the result is bit-for-bit today's (diffuse + ambient + emission). Otherwise add a GGX-style (or Blinn-Phong with exponent from roughness; implementer's choice, documented) specular highlight from each light in play (headlight, sun when shadows on, studio lights), strength (1 - roughness)², plus an environment reflection term: the reflected view direction looks up a vertical gradient (sky color above, ground color below, from the style; defaults when the style has none) weighted by (1 - roughness) and Fresnel. Metalness tints specular by the base color and scales diffuse by (1 - metalness). Translucent and textured materials take part (texture color is the base color).
2. **Studio lighting.** `Lighting::Studio` (`set_lighting`, `render_view`, scenes, the View menu next to Day/Night): no headlight; a key light from the camera's upper left front (direction in camera space (-0.5, 0.6, 0.6) normalised, toward the scene), a fill light from the right (0.6, 0.1, 0.5) at 0.35 strength, a rim light from behind above (0, 0.5, -0.8) at 0.5 strength, ambient 0.25; the background is the style's background color, or a vertical gradient from it when the style's sky is on. With shadows on, the key light casts the shadow map instead of the sun.
3. **Ambient occlusion.** Style setting `ambient_occlusion { on (default false), radius (m, default 0.25, 0.001..100), strength (0..2, default 1) }` in `add_style`/`change_style`, saved with styles and `.skr`, Styles panel controls. SSAO from depth and normals at half resolution with a bilateral blur, multiplied into ambient and diffuse (not into emission or edges). Off in WebGL2 if it cannot be done there (say so in the Styles panel).
4. **Section fill color.** `set_section_display` gains `fill_color: "dark" | "material"` (default dark, today's #3a3a3a). With material, each closed cut is filled with the color of the material most of the cut faces show on their front (else the nearest painted instance's material, else the default face color), darkened by 20% so the cut reads as a cut. Saved with the display setting (and with scenes if the section display is stored there).
5. **Instances left whole.** `set_section_cut { ids, cut: bool }` (default `true` for every instance): an instance with `cut: false` is not cut by section planes of the contexts outside it (planes inside its own definition still cut its contents). One undo step; in `.skr`; `entities` query field `section_cut` (absent when true); Entity Info checkbox "Cut by section planes".
6. **`render_view` options** (all optional; none changes the document): `lighting` (existing, adds `studio`), `style` (a StyleId used for this render), `projection` (`perspective`/`parallel`), `section` `{ planes, cuts, fill, fill_color }`, `shadows` (bool), `supersample` (1..4, default 1: renders at k times the size and averages down; width*k and height*k at most `MAX_IMAGE_SIDE` * 2 else `invalid_input`), `background` (`"style"` default or `"transparent"`: pixels with no geometry, guides or edges get alpha 0 and drawn pixels keep alpha 1; the PNG then has alpha). The MCP `render_view` tool takes the same fields.

## Review Focus

1. Golden images unchanged with defaults; WebGL2 path builds and degrades as ruled.
2. Shader math: no NaN at grazing angles or roughness 0; specular energy reasonable; metal looks metallic in a test render (store a small golden of a metal sphere, studio lit, created by the test itself on first run only if the project does that already; otherwise numeric pixel checks).
3. SSAO darkens creases and not flat open faces (pixel-region tests).
4. Section fill colors and `cut: false` both in the window renderer and the headless one, and in print/PDF if they share the path.
5. `render_view` overrides restore nothing because they never change the document (check the revision and the document's style/lighting after).

---

### Task 1: Material metalness and roughness

**Files:** `crates/kernel/src/material.rs`, `crates/kernel/src/data.rs` (save/load), `crates/session/src/{command.rs, document.rs}`, `crates/render/src/{scene.rs, renderer.rs, shaders/scene.wgsl}`, `crates/app/src/ui/` materials panel, `docs/agent-api.md`; tests.

- [ ] **Step 1: Write the failing tests** for ruling 1.
- [ ] **Step 2-5:** run, implement, gate (+ build-web), commit `git commit -m "Material metalness and roughness"`.

### Task 2: Studio lighting

**Files:** `crates/session/src/camera.rs`, `crates/render/src/{lights.rs, renderer.rs, shaders/scene.wgsl, sun_shadow.rs}`, app View menu, `docs/agent-api.md`; tests.

- [ ] **Step 1: Write the failing tests** for ruling 2.
- [ ] **Step 2-5:** run, implement, gate (+ build-web), commit `git commit -m "Studio lighting"`.

### Task 3: Ambient occlusion

**Files:** `crates/render/src/{(new) ssao.rs, renderer.rs, style.rs, shaders/(new) ssao.wgsl}`, kernel style data and save/load, session style commands, Styles panel, `docs/agent-api.md`; tests.

- [ ] **Step 1: Write the failing tests** for ruling 3.
- [ ] **Step 2-5:** run, implement, gate (+ build-web), commit `git commit -m "Ambient occlusion"`.

### Task 4: Section fill color and instances left whole

**Files:** `crates/render/src/sections.rs`, `crates/session/src/{command.rs, section_clip.rs, section_shape.rs, query.rs}`, kernel instance flag and save/load, Entity Info, `docs/agent-api.md`; tests.

- [ ] **Step 1: Write the failing tests** for rulings 4-5.
- [ ] **Step 2-5:** run, implement, gate, commit `git commit -m "Section fill in material color; instances left whole"`.

### Task 5: render_view options

**Files:** `crates/api/src/{dispatch.rs, mcp/*}`, app and headless `render_view` hosts, `docs/agent-api.md`; tests.

- [ ] **Step 1: Write the failing tests** for ruling 6.
- [ ] **Step 2-5:** run, implement, gate, commit `git commit -m "render_view options"`.

### After the tasks (controller)

- Final review; push `dev`; CI green; roadmap row M17e done; spec bullet; memory; merge and push; next milestone (M17f).
