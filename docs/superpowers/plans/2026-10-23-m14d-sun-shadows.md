# M14d Daylight and Sun Shadows Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** SketchUp's Shadows: a model location (latitude, longitude, north angle, UTC offset), a date and time, the sun's direction from a solar position formula, faces lit by the sun with light and dark levels, a directional shadow map over the model and the ground, a Shadows panel with time and date sliders and an on/off toggle; shadow settings saved with the camera and with scenes; night lighting and lamp shadows keep working; golden images unchanged with shadows off.

**Architecture:** The location is a model property on the kernel `Scene` (undoable `set_location`). Shadow settings (`on`, date, time, light, dark, on ground) are a view setting in `PageView` following `x_ray` exactly (camera command, no undo step, saved with the camera and scenes). A pure `session::sun` module computes the sun direction in model coordinates (NOAA simplified solar position, then the north angle). The renderer gains a sun term in `face_color` and one 2D directional shadow map (orthographic, fitted to the shadow casters' bounds, redrawn only when the sun direction or the casters change), plus a shadow-only ground quad on z = 0 when "on ground" is set. With shadows off nothing in the shader path changes (the sun term is gated by a uniform flag), so golden images stay identical.

**Tech Stack:** Rust 2024, wgpu 30, egui 0.36.2. No new dependencies (the solar formula and the date arithmetic are hand-written).

**Spec:** `docs/superpowers/roadmap.md` row M14d.

## Global Constraints

- No new dependencies. With shadows off and the default location, behaviour, `.skr` output and golden images are unchanged.
- `.skr` schema 16: written only when the location is not the default or a camera or scene view has shadows on; older files open unchanged.
- New command variants get arms in `Document::execute`/`edit`, the undoable or camera lists, `session::batch::id_slots` (no `_` arm), api dispatch and `docs/agent-api.md` in the task that adds them.
- Gate per task (project rules in the SDD rules files): native gate + `cargo check --target wasm32-unknown-unknown -p kernel -p session -p tools -p render -p api -p app --lib -p web`; `scripts/build-web.sh` in Tasks 2 and 3.
- No emoji. Short commit messages. Never `git add -A`.

## Spec notes (rulings)

1. **Location** (model): `latitude` (-90..90), `longitude` (-180..180), `north_angle` (degrees, clockwise from the green axis to true north, -360..360), `utc_offset` (hours, -12..14, step 0.25). Default: 40.0167, -105.2833, 0, -7 (SketchUp's default, Boulder). `set_location { latitude?, longitude?, north_angle?, utc_offset? }` (only given fields change), one undo step labelled `set_location`; `location` query. Saved in `.skr` only when not default.
2. **Shadow settings** (view): `on` (default false), `date` ("YYYY-MM-DD", default 2026-11-08), `time` ("HH:MM", local time at the model's UTC offset, default 13:30), `light` (0-100, default 80), `dark` (0-100, default 45), `on_ground` (default true). `set_shadows { on?, date?, time?, light?, dark?, on_ground? }` is a camera command (no undo step, does not mark modified), saved in `PageView` (camera and scenes), restored by `go_to_scene`, in the `camera` query. Invalid dates/times/levels are `invalid_input`.
3. **Sun:** NOAA simplified algorithm (fractional year, equation of time, declination, true solar time, hour angle, zenith, azimuth from north clockwise) from date, time, UTC offset, latitude and longitude; then azimuth is turned by the north angle into the model's x/y (green axis = north when north_angle is 0). Query `sun` returns `{ direction: [x,y,z] toward the sun, azimuth, elevation, up: bool }`. Accuracy within 0.5 degree of the NOAA spreadsheet for test cases (document the reference values used). Model drawing axes (M14c) do not affect the sun (north is relative to world green).
4. **Shading with shadows on (day lighting):** each face colour = base * (dark + light * max(dot(n_front_facing, sun), 0) * shadow) with `light`, `dark` as fractions, where `shadow` is 0..1 from the shadow map (PCF). With the sun below the horizon: no direct light (dark only), no shadows drawn. Shadows off: today's headlight shading exactly. Night lighting (M9c): shadows settings are ignored while night is on (lamps and lamp shadows as today); documented.
5. **Shadow map:** one 2D depth map (2048 native, 1024 on the web), orthographic along the sun, fitted to the shown casters' bounds (and the ground receiver area when on ground), stable (snap to texels) so the shadow does not shimmer while orbiting; redrawn only when the sun direction, the casters or their bounds change. Casters: shown, non-ghost faces with opacity >= 0.7 (faces below 0.7 neither cast nor receive darkening from themselves); section cuts remove cut-away casters (as lamp shadows); hidden and ghost faces never cast. X-ray does not change casting.
6. **Ground:** when `on_ground`, a shadow-only quad on z = 0 under the casters (extent: the casters' footprint plus the shadow's reach, capped) darkens where in shadow (multiplying the background by the dark/light ratio); nothing else is drawn for the ground; not drawn when the camera is below z = 0.
7. **Outputs:** `render_view`, snapshots and raster print (PDF) use the document's shadow settings; the hidden-line drawing, SVG and library thumbnails ignore shadows. The web build and the phone viewer draw sun shadows too (2D depth textures work on WebGL2).
8. **Window:** a Shadows dock tab: On toggle, Time slider (minutes, 15 min steps, with a text field), Date slider (day of year with the month/day label, and a year field), Light and Dark sliders, On ground checkbox, and a Location section (latitude, longitude, north angle, UTC offset fields). View menu "Shadows" checked toggle. Read-only phone viewer: no panel or toggle.

## Review Focus

1. Golden images and existing render tests unchanged with shadows off; uniforms and bind groups stay valid on WebGL2 (downlevel limits) with and without shadows.
2. Sun direction: test values against the NOAA reference (equator, high latitude summer/winter, southern hemisphere, date line longitudes, negative and fractional UTC offsets, north angle).
3. Shadow correctness: no acne or peter-panning on a box on the ground and on a wall; no shimmering when orbiting; caching (no redraw when only the camera moves).
4. Settings: camera/scenes/.skr round trip, schema 16 only when needed, schema 15 and older byte-identical; set_location undo; set_shadows no undo step.
5. Performance: a 20k-face model with shadows on: one map redraw per sun change, frame time measured in an ignored perf test.

---

### Task 1: Location, shadow settings and the sun

**Files:** `crates/kernel/src/{scene.rs (location), page.rs (shadow settings in PageView), data.rs}`, `crates/session/src/{(new) sun.rs, camera.rs, command.rs, document.rs, document/scenes.rs, document/files.rs, batch.rs, file.rs (schema 16), query.rs}`, api dispatch and headless, `docs/agent-api.md`; tests (sun reference values; settings round trips).

- [ ] **Step 1: Write the failing tests** for rulings 1-3 (Review Focus 2, 4).
- [ ] **Step 2-5:** run, implement, gate, commit `git commit -m "Location, shadow settings and the sun"`.

### Task 2: Sun lighting and the shadow map in the renderer

**Files:** `crates/render/src/{renderer.rs, (new) sun_shadow.rs, shaders/scene.wgsl, (new) shaders/sun_shadow.wgsl, grid.rs or (new) ground shadow, scene.rs (caster flags), snapshot.rs, print.rs}`, `crates/api/src/headless.rs`, `crates/app/src/app.rs` (pass settings each frame), docs; render tests, a new golden image (box on the ground with sun shadows), an ignored perf test.

- [ ] **Step 1: Write the failing tests** for rulings 4-7 (Review Focus 1, 3, 5).
- [ ] **Step 2-5:** run, implement, gate (+ build-web), commit `git commit -m "Sun shading and shadows"`.

### Task 3: Shadows panel and menu

**Files:** `crates/app/src/{ui/dock.rs, (new) ui/shadows.rs, ui/mod.rs, menus.rs, native_menu.rs, editor.rs, app.rs}`, `docs/agent-api.md` window table; UI tests (accessible names), editor tests.

- [ ] **Step 1: Write the failing tests** for ruling 8.
- [ ] **Step 2-5:** run, implement, gate (+ build-web), commit `git commit -m "Shadows panel"`.

### After the tasks (controller)

- Browser smoke test of the web build with shadows on; push `dev`; CI green; roadmap row M14d done; spec bullet; memory; merge and push; next milestone (M14e).
