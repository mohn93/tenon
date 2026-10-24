# M17g Python Client Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A small, dependency-free Python package so agents and people can script models without hand-writing JSON-RPC: connect, run commands and batches with references, build parts, place and transform instances, make primitives and solids, render to PNG; with the V6 engine and the city as examples.

**Architecture:** `clients/python/sketchup_alt/` (standard library only: `socket`, `json`, `base64`, `math`, `dataclasses`, `subprocess`): `client.py` (connection over the Unix socket or Windows named pipe / TCP as the app offers, newline-delimited JSON-RPC 2.0, errors raised as `ApiError(kind, message, data)`), `batch.py` (a builder whose methods return reference objects like `Ref(0).f(0)`, `Ref(0).all()`, rendered as `"$0:f0"`), `model.py` (helpers: primitives, booleans, transforms as `Transform.axis_angle(...)`, `Transform.look(...)`, groups and components, materials with `existing="reuse"`, labels, scenes, `render(path, ...)`), `launch.py` (start a headless app with `--socket` in a private temp dir for tests and scripts, and stop it). Tests run against a real headless app binary (built by cargo) when available and skip otherwise.

**Tech Stack:** Python 3.9+ (stdlib only). The Rust side is unchanged except docs.

**Spec:** `docs/superpowers/roadmap.md` row M17g. Uses M17a-M17f commands.

## Global Constraints

- No third-party Python packages. No changes to the Rust crates except where a bug blocks the client (then fixed with a test in that crate).
- The package works with the socket path given explicitly or from `SKETCHUP_ALT_SOCKET`.
- CI: add a job step (in the existing Linux job) that runs `python3 -m unittest discover clients/python/tests` after the cargo build, using the built app in headless mode; skip cleanly when no binary.
- No emoji. Short commit messages. Never `git add -A`.

## Spec notes (rulings)

1. **Client.** `Client(socket_path=None, timeout=60)`; methods `execute(cmd: dict)`, `batch(cmds, atomic=True)`, `query(type, **params)`, `describe()`, `render_view(width, height, **opts) -> bytes (PNG)`; ids are plain strings. A failed command raises `ApiError` with the API's error kind and message; batch failure includes the failing index.
2. **Batch builder.** `with client.batch_builder() as b:` collects commands; `b.add_circle(...)` returns a `Ref`; `ref.f(0)`, `ref.e("*")`, `ref.i(0)`, `ref.all()` produce reference strings; leaving the block sends it; results are reachable as `b.result`. Every command in `describe`'s schema is available as a snake_case method generated from the schema at connect time (unknown fields raise `TypeError` before sending), plus hand-written helpers for the common ones with docstrings.
3. **Model helpers.** `Model(client)`: `material(name, color, metalness=0, roughness=1)` (reuse by name), `box/cylinder/tube/cone/prism/sphere/torus(...) -> Instance`, `union/subtract/intersect(...)`, `Instance.move/rotate/set_transform/paint/hide/name`, `group(instances, name)`, `component(...)`, `place(definition, transform)`, `copy_along_path`, `label(target, text)`, `scene(name, positions=[...])`, `render(path, width, height, camera=None, lighting="studio", **opts)`. `Transform`: `identity`, `translate`, `axis_angle(axis, degrees, origin)`, `look(origin, z_axis, x_hint)`, composition with `@`, `to_json()` in the API's `{matrix}` form.
4. **Examples.** `clients/python/examples/v6_engine.py` (the cutaway V6 rebuilt with primitives, booleans for bores and per-group section planes, studio lighting, labels; renders `v6.png`) and `city.py` (a small city grid). Each runs against a running app (`--socket`) or launches a headless one.
5. **Docs.** `clients/python/README.md` (install by path, connect, batches, references, examples) and a section in `docs/agent-api.md` pointing to it.

## Review Focus

1. Protocol handling: partial reads, large render payloads, errors, timeouts; no busy loops.
2. Generated methods match the schema; references render correctly; nothing outside stdlib.
3. Examples run end to end against a headless app and produce images.

---

### Task 1: Client, batch builder and launcher

**Files:** `clients/python/sketchup_alt/{__init__.py, client.py, batch.py, launch.py}`, `clients/python/tests/test_client.py`, `clients/python/README.md`.

- [ ] **Step 1: Write the failing tests** for rulings 1-2 (against a launched headless app).
- [ ] **Step 2-5:** run, implement, run the tests and the cargo gate (unchanged crates: fmt/clippy unaffected), commit `git commit -m "Python client: connection, batches, launcher"`.

### Task 2: Model helpers and transforms

**Files:** `clients/python/sketchup_alt/{model.py, transform.py}`, tests.

- [ ] **Step 1: Write the failing tests** for ruling 3.
- [ ] **Step 2-5:** run, implement, test, commit `git commit -m "Python client: model helpers and transforms"`.

### Task 3: Examples, docs and CI

**Files:** `clients/python/examples/{v6_engine.py, city.py}`, `clients/python/README.md`, `docs/agent-api.md`, `.github/workflows/ci.yml`; tests (examples run headless, produce PNG files).

- [ ] **Step 1: Write the failing tests** for rulings 4-5.
- [ ] **Step 2-5:** run, implement, test, commit `git commit -m "Python client examples, docs and CI"`.

### After the tasks (controller)

- Final review; push `dev`; CI green; roadmap row M17g done; spec bullet; memory; merge and push; final report to the user with a rebuilt engine render.
