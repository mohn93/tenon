# sketchup-alt Python client

A small client for the agent API (see `docs/agent-api.md`). Python 3.9+, standard library only.
Install by path: `pip install ./clients/python`, or put `clients/python` on `PYTHONPATH`.

```python
from sketchup_alt import Client, Ref, launch_headless

# Connect to a running app (--socket PATH, or $SKETCHUP_ALT_SOCKET), or start a headless one.
with launch_headless() as app:           # uses $SKETCHUP_ALT_APP or target/{debug,release}/app
    c = app.client()                      # or Client("/path/to.sock")
    c.execute({"type": "add_edge", "start": [0, 0, 0], "end": [1, 0, 0]})
    print(c.query("stats"))
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
print(b.result["revision"])
```

`ref.f(0)`, `ref.e("*")`, `ref.i(0)`, `ref.all()` render as `"$0:f0"`, `"$0:e*"`, `"$0:i0"`,
`"$0:*"`. Every command in `describe` is a method named like its `type`, generated at connect
time; an unknown or missing field raises `TypeError` before anything is sent. Leaving the block
sends the batch (not if the block raised).

## Tests

`python3 -m unittest discover clients/python/tests` (skips without an app binary).
