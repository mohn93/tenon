# sketchup-alt Python client

A small client for the agent API (see `docs/agent-api.md`). Python 3.9+, standard library only.

## Install

By path, from the repository root:

```
pip install ./clients/python
```

or put `clients/python` on `PYTHONPATH` (the examples do this for themselves). The tests and
examples need an app binary: `cargo build -p app` (or `--release`, much faster for big models).

## Connect

```python
from sketchup_alt import Client, launch_headless, session

c = Client("/path/to.sock")              # a running app (--socket PATH), or $SKETCHUP_ALT_SOCKET

with launch_headless() as app:           # starts app --headless --socket <private path>
    c = app.client()                      # uses $SKETCHUP_ALT_APP or target/{debug,release}/app
    c.execute({"type": "add_edge", "start": [0, 0, 0], "end": [1, 0, 0]})
    print(c.query("stats"))

with session(socket_path) as s:          # either: connect when a path is given, else launch
    s.client.query("stats")
```

`Client(socket_path=None, timeout=60)` has `execute(cmd)`, `batch(cmds, atomic=True)`,
`query(type, **params)`, `describe()` and `render_view(width, height, **opts)` (PNG bytes). A
failed command raises `ApiError` with `kind` and `message`; a failed batch also has `index`,
`completed` and `rolled_back`. On Windows the socket is a pipe name and `timeout` is not enforced.

## Batches and references

```python
with c.batch_builder() as b:
    rect = b.add_rectangle(corner=[0, 0, 0], opposite=[4, 3, 0])
    b.push_pull(face=rect.f(0), distance=2.5)   # "$0:f0"
    box = b.add_box(corner=[6, 0, 0], size=[1, 1, 1])
    b.copy(ids=[box.i(0)], offset=[2, 0, 0], count=5)
print(b.result["revision"])
```

A batch is one round trip and, by default, one undo step that is rolled back whole if a command
fails. Each method returns a `Ref` to that command's results: `ref.f(0)`, `ref.e("*")`,
`ref.i(0)`, `ref.d(0)`, `ref.m(0)`, `ref.all()` render as `"$0:f0"`, `"$0:e*"`, `"$0:i0"`,
`"$0:d0"`, `"$0:m0"`, `"$0:*"` and can stand wherever a later command takes an id. Every
command in `describe` is a method named like its `type`, generated at connect time; an unknown
or missing field raises `TypeError` before anything is sent. Leaving the block sends the batch
(not if the block raised).

## Models

`Model(client)` wraps the common steps. Lengths are meters, colors `"#rrggbb"` or `(r, g, b)`.

```python
from sketchup_alt import Model, Transform

m = Model(c)
steel = m.material("Steel", (200, 205, 215), metalness=1, roughness=0.3)   # reused by name
block = m.box((0, 0, 0), (0.4, 0.3, 0.2), material=steel, name="Block")
block.fillet(block.edges(lambda a, b: a[2] > 0.19 and b[2] > 0.19), 0.01)  # model coordinates
bores = [m.cylinder((0.1 + 0.1 * i, 0.15, -0.01), 0.04, 0.22) for i in range(3)]
block = m.subtract(block, bores)                    # also union, intersect
bolt = m.component([m.cylinder((0, 0, 0), 0.005, 0.05)], "Bolt")
for k in range(4):
    m.place(bolt, Transform.translate(0.05 + 0.1 * k, 0.02, 0.2))
spring = m.spring((0.6, 0, 0), (0, 0, 1), 0.02, 0.003, 0.012, 6)      # add_helix + follow_me
pipe = m.sweep([(0, -0.2, 0), (0, -0.2, 0.2), (0.2, -0.2, 0.3)], 0.02, inner_radius=0.017)
block.section((0.2, 0.15, 0.1), (0, 1, 0))          # a plane that cuts only this group
m.lighting("studio")
m.style("Look", ambient_occlusion={"on": True, "radius": 0.02})
m.label(block, "Block")
m.render("out.png", 1600, 1000, camera={"eye": [1, -1, 0.8], "target": [0.2, 0.15, 0.1]}, supersample=2)
```

- Primitives: `box`, `cylinder`, `tube`, `cone`, `prism`, `sphere`, `torus` (options `name`,
  `material`, `as_`); profiles: `extrude(points, distance)`, `revolve(points, center, axis)`,
  `sweep(points, radius | profile, inner_radius, closed)`, `spring(...)`.
  The profile helpers draw in the open context and group the result, so they merge with loose
  geometry they touch there: keep parts in groups (every helper here returns one).
- Solids: `union`, `subtract(target, tools)`, `intersect`; `Instance.fillet` and `chamfer` on
  edge ids from `Instance.edges(where)`.
- Instances: `move`, `rotate`, `set_transform`, `apply(t)` (on top of the current transform),
  `transform()`, `copy(offset, count)`, `polar(axis, angle, copies, origin)` (a radial array),
  `clone()` (a group copy with a definition of its own), `paint`, `hide`, `name`, `section`.
- Assembly: `group`, `component`, `place(definition, transform)`, `copy_along_path`,
  `section_cut(instances, cut=False)`, `explode_view`.
- Views: `camera`, `lighting`, `style`, `section_display`, `scene(name, positions=[...])`,
  `go_to_scene`, `transition`, `render(path, ..., scene=, from_=, t=)`.

`Transform` is the API's `{"matrix": [...]}`: `identity`, `translate`, `scale`,
`axis_angle(axis, degrees, origin)`, `look(origin, z_axis, x_hint)`, `a @ b` (b first),
`inverse`, `apply`, `apply_vector`, `to_json`.

## Examples

Each runs against a running app (`--socket PATH`) or starts a headless one, and writes its
images to `--out DIR` (default: the current directory). `--quick` makes a small model and small
images (the tests use it).

- `examples/v6_engine.py`: a half-open cutaway 60-degree V6 (bores, water jacket, ports, I-beam
  rods, counterweighted crank, helical valve springs, chains along a path, metal materials, a
  section with whole moving parts, studio lighting, ambient occlusion, labels and two crank
  scenes). Writes `v6.png`, `v6-front.png` and `v6-motion.png` (a frame between the scenes).
  About 5 s of building against a release app.
- `examples/city.py`: a city grid from the air, built in a few large batches (`--size N`
  blocks a side). Writes `city.png`.

```
cargo build --release -p app
SKETCHUP_ALT_APP=target/release/app python3 clients/python/examples/v6_engine.py --out /tmp/v6
```

## Tests

`python3 -m unittest discover clients/python/tests` (skips without an app binary; renders skip
without a GPU adapter).
