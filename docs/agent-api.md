# Agent API

Every edit the window can make is also a command agents can send. The API is JSON-RPC 2.0 with one
JSON object per line (UTF-8, `\n`-terminated).

## Starting

| Command | Where the API is |
|---|---|
| `cargo run -p app` | Window, plus a Unix socket (path printed on stderr: `agent API listening on ...`) |
| `cargo run -p app -- --socket PATH` | Window, socket at `PATH` |
| `cargo run -p app -- --headless --stdio` | No window; requests on stdin, replies on stdout; exits when stdin closes |
| `cargo run -p app -- --headless` | No window; socket at the default path (printed on stderr) |
| `cargo run -p app -- --headless --socket PATH` | No window; socket at `PATH` |

Any of these can end with a `.skr` file to open at start, e.g.
`cargo run -p app -- --headless --stdio house.skr`. In headless mode a file that cannot be opened
stops the app with exit code 1 and a message on stderr.

The default socket is `sketchup-alt-<pid>.sock` in the temp directory, or in `~/.sketchup-alt/`
(created with mode 0700) when the temp directory path is too long for a Unix socket. The socket is
readable only by the current user. Try it by hand with `nc -U PATH`. The window removes the
socket file on a normal quit. The default name includes the process id, so a socket left behind
by a killed app is never reused; agents that start the app should pass `--socket PATH` (a fixed
path is replaced on the next start).

## Methods

Call `describe` first: it returns every method, notification, error code and the JSON Schemas of
all commands and results.

| Method | Params | Result |
|---|---|---|
| `execute` | a Command, e.g. `{"type": "add_edge", "start": [0,0,0], "end": [1,0,0]}` | `{changes, revision}` |
| `batch` | `{"commands": [...], "atomic": true}` | `{outcomes, revision}`; atomic = one undo step, all or nothing |
| `query` | a Query, see the table below | the QueryResult |
| `render_view` | `{"width": 640, "height": 400, "camera": {"eye": [...], "target": [...]}}` (camera optional) | `{width, height, png_base64}` |
| `subscribe` | none | `{revision}`, then `changed` notifications after every model change; a `resync` notification (`{revision}`) means changes were missed, so re-read with `{"type": "entities"}` (its `revision` tells you where you are) |
| `describe` | none | the API description |

Queries:

| Query | Result |
|---|---|
| `{"type": "entities"}` | every vertex, edge, face, instance and guide of the open context, plus `revision` |
| `{"type": "entities", "kinds": ["face"]}` | only the listed kinds (`vertex`, `edge`, `face`, `instance`, `guide`) |
| `{"type": "entities", "ids": ["v0", "e2"]}` | only the listed ids; unknown ids are absent, not an error |
| `{"type": "entities", "within": {"min": [0,0,0], "max": [2,2,1]}}` | only entities whose vertices (an instance: its bounding box; a guide point: its position) all lie in the box, bounds included; an instance with nothing in it and a guide line are in no box. `ids`, `kinds` and `within` combine (all must match); empty lists mean no filter |
| `{"type": "entity", "id": "f7"}` | one entity, or a not-found error |
| `{"type": "curve", "id": "e3"}` | the curve (circle or arc) edge `e3` is part of; see Curves below |
| `{"type": "stats"}`, `{"type": "camera"}`, `{"type": "history"}` | counts for the open context (plus the number of definitions) and revision, camera, undo/redo depth |
| `{"type": "selection"}` | the selected edge, face, instance and guide ids, plus `revision` |
| `{"type": "definitions"}` | every group and component definition: `id`, `name`, `kind`, how many `instances` place it, `edges`, `faces` |
| `{"type": "context"}` | the open context: `path` (open instances from the root, empty at the root) and the `definition` being edited |
| `{"type": "file"}` | `path` of the file last opened or saved (`null` if never), `modified` since then, and `revision` |
| `{"type": "events", "since": 3}` | model changes after revision 3, oldest first |

Unknown fields in commands, queries and params are rejected with `-32602` (a typo fails instead
of silently doing nothing).

Commands: `add_edge`, `add_rectangle`, `add_circle`, `add_arc`, `push_pull`, `erase`, `move`, `copy`, `rotate`, `make_group`,
`make_component`, `explode`, `open_instance`, `close_instance`, `rename_definition`,
`purge_definitions`, `add_guide_line`, `add_guide_point`, `delete_guides`, `select`, `undo`,
`redo`, `set_camera`, `new`, `open`, `save`, `export`.
Units are meters, Z is up, angles in degrees, coordinates within +/-1,000,000 m.
Entity ids are strings: `v12` (vertex), `e3` (edge), `f7` (face), `i4` (instance of a group or
component), `g9` (guide line or guide point). Definitions are `d5`. Ids are unique across the whole model, including inside
groups and components.

Modeling commands, each one undo step:

| Command | Effect |
|---|---|
| `{"type": "add_edge", "start": [x,y,z], "end": [x,y,z]}` | A straight edge. It merges with nearby vertices, splits edges it crosses, and closed coplanar loops become faces. |
| `{"type": "add_rectangle", "corner": [0,0,0], "opposite": [2,3,0]}` | A rectangle with sides along the axes. Without `normal` the corners must differ in exactly two coordinates. With `"normal": [nx,ny,nz]` it lies in the plane through `corner` with that normal; `opposite` must be in that plane. Sides then run along u = `Z x normal` (horizontal; X on horizontal planes) and v = `normal x u`, so directions are fixed only up to sign: on a plane with normal +Y, u = -X and v = +Z; with normal -Y, u = +X. |
| `{"type": "add_circle", "center": [0,0,0], "radius": 1.5}` | A circle as a regular polygon with `segments` sides (default 24, 3 to 360), in the plane through `center` with `normal` (default `[0,0,1]`). The first corner lies along the plane's u axis (as for `add_rectangle`) and the corners run counter-clockwise around the normal. The radius must be positive, and each side (2r sin(pi/segments)) must be at least 0.01 mm, ten times the merge tolerance. Its edges form one curve (see Curves). |
| `{"type": "add_arc", "center": [0,0,0], "start": [2,0,0], "angle": 90}` | An arc around `center` from `start`, turning `angle` degrees counter-clockwise around `normal` (default `[0,0,1]`; a negative angle turns clockwise), as `segments` equal straight edges (default 12 whatever the angle, 2 to 360) that form one curve. `start` must lie in the plane through `center` with that normal (within 0.001 mm); the radius is its distance from `center`. The angle must be more than 0 and less than 360 degrees in size (a whole circle is `add_circle`), and each segment at least 0.01 mm long. Its points snap and split like `add_edge`'s, so an arc ending on existing edges can close a face. |
| `{"type": "move", "ids": ["f8"], "offset": [0, 0, 1]}` | Moves vertices, edges and faces (their vertices), instances and guides by `offset` (an instance must keep its bounding box, or its origin when it holds nothing, and a guide its point, within the coordinate limit; so must copies). Faces that share only some moved vertices stretch and may tilt but must stay flat: a move that would bend one is `invalid_input` naming that face. A move that would collapse an edge, turn a face over, make a face cross itself, push a hole out of its face or land a vertex on another is `invalid_input` naming the ids. Moved geometry does not merge with what it ends up touching. Unknown ids are `not_found`. |
| `{"type": "copy", "ids": ["f8"], "offset": [3, 0, 0], "count": 4}` | Copies edges, faces, instances and guides `count` times (default 1, at most 100); copy k is shifted by `k * offset`. Copies are drawn like new edges, so they merge with what they touch: a copy has the faces its original had, made even where their sides already exist as bare edges, plus faces it closes together with existing edges. A copy drawn on a face splits it and every piece stays a face (also under an opening of the copy); elsewhere an opening in a copied face stays open and a loop the original had no face in gets none. Copied faces keep the orientation of their originals. A copied component instance places the same definition; a copied group gets a definition of its own. Vertex ids are `invalid_input`. |
| `{"type": "rotate", "ids": ["f8"], "center": [0, 0, 0], "angle": 90}` | Turns vertices, edges and faces (their vertices), instances and guides by `angle` degrees about the line through `center` along `axis` (default `[0,0,1]`), counter-clockwise looking down the axis (right-hand rule), under the rules of `move`: faces that share only some turned vertices must stay flat, and so on; geometry turned whole keeps its shape. Multiples of 90 degrees are exact. An instance turns with its transform. A zero `axis` and whole turns (0, 360, ..., or within 1e-9 degrees of one) are `invalid_input`. With `"copies": n` (1 to 100) the originals stay and n copies are made under the rules of `copy`, copy k turned by `k * angle` (a radial array; vertex ids are then `invalid_input`, and so is a count where a copy would complete a whole turn and land on the original, e.g. angle 90 with 4 copies). Vertices on the axis do not move, and a rotation that moves nothing changes nothing. |
| `{"type": "erase", "ids": ["e2", "f8"]}` | Erases edges, faces, instances and guides (a group's definition goes with its last instance). An erased face leaves its edges. An erased edge takes the faces it bounds with it, except that two faces in one plane meeting at it merge into one. Vertices go with their last edge; a vertex left between two collinear edges is healed away. Faces are erased before edges, so erasing a face together with the seam it shares with a coplanar neighbour removes the neighbour too: the seam then bounds only one face. Vertex ids are `invalid_input` and unknown ids are `not_found`, both with `ids`; empty `ids` is `invalid_input`. |
| `{"type": "make_group", "ids": ["e2", "f8", "i4"]}` | Moves edges, faces (with their edges), instances and guides (keeping their ids) into a new group definition and places one instance of it where they were; the instance becomes the selection. Edges that still bound a face left behind stay outside too. Vertex ids are `invalid_input`, and so are ids naming only guides. |
| `{"type": "make_component", "ids": ["f8"], "name": "Chair"}` | Like `make_group`, but makes a named component: its copies share the definition, so editing one edits them all. An empty name is `invalid_input`. |
| `{"type": "explode", "ids": ["i4"]}` | Replaces instances by their geometry, which merges with what it touches; instances and guides inside come out as instances and guides (with new ids). What comes out becomes the selection. Ids that are not instances are `invalid_input`, and so is an instance whose geometry or guides would land beyond the coordinate limit (naming it). |
| `{"type": "push_pull", "face": "f8", "distance": 1.5}` | Moves a face along its normal (negative: against it). A face on a flat sheet extrudes into a solid and stays as its base. A face on a solid extrudes a bump, or cuts a recess when pushed in; pushed in exactly as far as a parallel face that holds it (the back of a wall), it cuts an opening through. A face whose neighbors all run along the normal, like a box top, moves and stretches them; moved back flush with the surface around it (a recess or bump pushed back), it closes up and the seams heal. When the face is extruded, its sides along a curve get a moved copy on the cap that is a curve of its own, and the swept edges between two sides of one curve are soft (`"soft": true`), so a pulled circle is a cylinder with a smooth side. |

`{"type": "select", "ids": ["f8"], "mode": "replace"}` changes the selection (edges, faces,
instances and guides; `mode` is `replace` (default), `add`, `remove` or `toggle`). It is not an undo step and
does not change the revision. The window shows the selection and agents see what the user
selected with the `selection` query. Vertex ids are `invalid_input` and unknown ids are
`not_found`, both with `ids`. Empty `ids` with mode `replace` clears the selection.

The selection drops any entity that stops existing, through erase, merges, undo or redo. Undo does
not restore the selection. The `selection` query's `revision` is the model revision: it does not
change when only the selection changes, and there is no notification for selection changes, so
compare ids to detect changes.

Find the face to push from the `created` ids of the command that made it, or with
`{"type": "entities", "kinds": ["face"]}`. Faces created by push/pull point out of the solid.
`distance` is measured along the face's own normal, and faces in one plane can point either way
(a pulled-up rectangle's base faces down), so read the face's `normal` with
`{"type": "entity", "id": "f8"}` before choosing the sign. A push/pull that would collapse or
invert the edges around a moved face fails with `invalid_input`. So does one that would carry the
face into or past other geometry, unless it ends exactly on a parallel face that holds the whole
pushed face, with nothing else on it inside that area: that cuts an opening (a window through a
wall, a hole through a slab). The floor of a recess pushed on to the back opens the same way.
When a face moves, its vertices are reported as `modified`; the edges and faces using them changed
shape too.

### Curves

The edges of a circle are tagged as one curve: each edge's `entity` result (and its entry in
`entities`) has `"curve": "c12"`, the same for every edge of the curve and absent for plain
edges. A curve is not an entity: `c` ids cannot be selected or erased. `{"type": "curve", "id":
"e3"}` takes any edge of a curve and returns:

```
{"type": "curve", "id": "c12", "kind": "circle", "edges": ["e3", ...], "vertices": ["v1", ...],
 "segments": 24, "length": 6.27, "center": [0,0,0], "normal": [0,0,1], "radius": 1.0, "angle": 360.0}
```

- `kind` is `circle` (a closed chain), `arc` (an open one) or `curve` (its corners no longer lie on
  one circle, after some of its vertices were moved; then `center`, `normal`, `radius` and `angle`
  are absent).
- The shape is worked out from where the edges are now, so it follows moves, rotations and copies.
  `normal` points up (+z; on a vertical plane +y, else +x); `edges` and `vertices` run
  counter-clockwise around it, a circle starting at its oldest vertex (the first corner drawn).
  `angle` is in degrees; `segments` counts straight segments, so an edge another line split in two
  still counts once; `length` is the total edge length.
- Edges split by other geometry stay on the curve, and heal back into one edge when the
  splitting line is erased. Erasing edges of a curve leaves the rest a curve (a circle missing an
  edge is an arc); if what is left falls apart, the oldest piece keeps the id and the others become
  curves of their own; a single edge left over is a plain edge again.
- Copies, groups and exploded geometry get new curve ids. A curve id is unique in the whole model.
- A command's `changes.modified` lists edges whose curve or soft flag changed.

Soft edges (`"soft": true`, absent when false) are hidden edges between faces that form one
smooth surface, such as the side of a pulled circle (see `push_pull`). Commands treat them like
other edges. A soft edge always bounds
at least two faces: one left bounding fewer (its neighbour face erased) becomes a plain edge
again. Pieces of a split soft edge stay soft, and copies, groups and files keep the flag.

### Guides

Guides are construction geometry, as drawn by the Tape Measure tool: guide lines (infinite) and
guide points. Tools snap to them; they never merge with edges, bound no faces and are not
exported. They belong to the open context they are added in (the root, or a group or component
definition) and are saved with the model.

| Command | Effect |
|---|---|
| `{"type": "add_guide_line", "point": [0, -1, 0], "direction": [1, 0, 0]}` | A guide line through `point` along `direction` (any non-zero length; stored as a unit vector). One undo step. |
| `{"type": "add_guide_point", "position": [1, 1, 0]}` | A guide point. One undo step. |
| `{"type": "delete_guides"}` | Erases every guide of the open context as one undo step; with none there it changes nothing (no error, no undo step). |

`erase`, `select`, `move`, `rotate` and `copy` take guide ids like any other (a moved or turned
guide keeps its id and is reported as `modified`). A guide's `entity` result, and each entry of
the `guides` list in `entities`, is `{"id": "g9", "kind": "line", "point": [...], "direction":
[...]}` (`kind` `point` has no `direction`). `stats` counts no guides.

### Groups and components

Commands and queries work on the open context: the root at first. `{"type": "open_instance",
"id": "i4"}` opens an instance of the open context for editing, and `{"type": "close_instance"}`
goes back out one level. Neither is an undo step or changes the revision; both clear the
selection. `open_instance` with an id that is not an instance, and `close_instance` at the root,
are `invalid_input`. While an instance is open:

- `entities`, `entity`, `stats` and `selection` see only the definition's entities: `entity` and
  commands give ids from elsewhere `not_found`, and `entities` leaves them out;
- points and offsets in commands, and positions in results, are in the definition's own
  coordinates (`instance.transform` maps them into the coordinates around it);
- edits change the definition, so every instance of a component changes with it.

An instance's `entity` result has its `definition`, `name`, `kind` (`group` or `component`),
`transform` (column-major 3x4: x axis, y axis, z axis, translation; the axes turn when the
instance is rotated) and `bounds` (the box around its turned geometry) in the open context's
coordinates. Undo and redo apply to the whole model wherever the change was made; an
open instance that stops existing is closed. Groups keep the coordinates of the context they
were made in (the instance's transform starts as the identity).

A command's `changes.created` can list ids inside a new definition (`make_group`, copying a
group): they are `not_found` from the open context until that instance is opened.

`{"type": "rename_definition", "definition": "d5", "name": "Chair"}` renames a definition (one
undo step; a component's name must not be empty; an unknown definition is `not_found`).
`{"type": "purge_definitions"}` removes every definition no instance places, such as a component
whose last instance was erased, and what only it placed (one undo step).

### Files

Models are saved as `.skr` files: a zip archive holding `document.json`, which records `format`
(`"sketchup-alt"`), `schema_version` (3; version 2 files, written before curves, and version 1
files, written before guides, still open), the camera and the model with every id, edges with
their curve and soft flag.

| Command | Effect |
|---|---|
| `{"type": "save", "path": "/abs/house.skr"}` | Writes the file (the old file is replaced only once the new one is complete). Without `path` it saves to the file last opened or saved; a model never saved then gives `invalid_input`. |
| `{"type": "open", "path": "/abs/house.skr"}` | Replaces the model and the view with the file's. With unsaved changes it fails with `unsaved_changes` unless it has `"discard": true`. |
| `{"type": "new"}` | An empty model with the default view, not saved anywhere. With unsaved changes it fails with `unsaved_changes` unless it has `"discard": true`. |

Paths are relative to the app's working directory. None of these is an undo step. `open` and
`new` clear the undo history, the selection and the open context, and change the revision;
subscribers get a `resync` notification and should read the model again. Ids in an opened file
are those it was saved with, and new ids continue after the largest one ever used. A file that
cannot be read or written gives the error kind `file`, with a message naming the path and the
reason (a missing file, not a sketchup-alt file, a missing or newer schema version, a document
over 256 MiB, ids over 2^53, a transform that cannot be inverted, a coordinate beyond the
+/-1e6 m limit, a curve whose edges branch, fall apart or are straight, or a soft edge bounding
fewer than two faces); the model is left as it was. `open` and `new` never drop changes the user has
not saved (`modified` in the `file` query) unless told to: they fail with `unsaved_changes`, and
a client that means it, or has asked the user, passes `"discard": true`. The window asks the user
itself before its own New and Open. `new`, `open`, `save` and `export` are not allowed in an
atomic batch.

### Export

`{"type": "export", "path": "/abs/house.glb"}` writes every face of the whole model as
triangles: the root's faces and every instance's, placed in root coordinates (a component placed
twice is written twice), whatever instance is open. Edges without faces are not written.
`format` (`obj`, `stl`, `glb`, `gltf`) defaults to the one the path's extension names, in any
case; any other extension without `format` is `invalid_input`, and so is a model with no faces.
A `.skr` path (any case) and the open model's own file are refused with `invalid_input`: export
never overwrites a model; use `save`.

| Format | Content |
|---|---|
| `obj` | Text OBJ: shared `v` positions and `vn` normals, one `f` triangle per line; Y up (x, z, -y) |
| `stl` | Binary STL: one record per triangle with its normal; Z up, as in the model |
| `glb` | Binary glTF 2.0: one mesh of flat-shaded triangles (`POSITION`, `NORMAL`) with a double-sided default material, so lone faces show from behind; Y up |
| `gltf` | The same glTF document as JSON, its buffer embedded as a base64 data URI |

Units are meters. Export writes the file the same safe way as `save`, is not an undo step, and
does not change the revision or the `file` query.

Behaviors worth knowing:

- Replies come before the `changed` notifications they cause, and your own edits are notified too.
- A request without an `id` runs but gets no reply. JSON-RPC batch arrays are rejected; use the
  `batch` method.
- `set_camera` returns empty changes and does not change the revision. `render_view`'s camera
  accepts `fov_y_deg` and never moves the document camera.
- A failed atomic batch is rolled back but still consumes entity ids, so later ids may skip numbers.
- An `id` that is not a string, number or null is an invalid request (`-32600`, reply id null).

## Example

```
-> {"jsonrpc":"2.0","id":1,"method":"execute","params":{"type":"add_edge","start":[0,0,0],"end":[2,0,0]}}
<- {"jsonrpc":"2.0","id":1,"result":{"changes":{"created":["v0","v1","e2"],"deleted":[],"modified":[]},"revision":1}}
-> {"jsonrpc":"2.0","id":2,"method":"query","params":{"type":"stats"}}
<- {"jsonrpc":"2.0","id":2,"result":{"type":"stats","vertices":2,"edges":1,"faces":0,"instances":0,"definitions":0,"revision":1}}
```

Building a 2 x 3 x 1.5 m box on an empty model:

```
-> {"jsonrpc":"2.0","id":3,"method":"execute","params":{"type":"add_rectangle","corner":[0,0,0],"opposite":[2,3,0]}}
<- {"jsonrpc":"2.0","id":3,"result":{"changes":{"created":["v0","v1","v3","v5","e2","e4","e6","e7","f8"],"deleted":[],"modified":[]},"revision":1}}
-> {"jsonrpc":"2.0","id":4,"method":"execute","params":{"type":"push_pull","face":"f8","distance":1.5}}
<- {"jsonrpc":"2.0","id":4,"result":{"changes":{"created":["v9","v11","v13","v15","e10","e12","e14","e16","e17","e19","e21","e23","f18","f20","f22","f24","f25"],"deleted":[],"modified":["f8"]},"revision":2}}
```

`f8` is now the box's bottom, flipped to face down; `f25` is the top.

## Errors

Standard JSON-RPC codes (-32700 parse, -32600 invalid request, -32601 unknown method, -32602 bad
params) plus `-32000` command failed (`data` is `{kind, message, ids?}`; `kind` is `invalid_input`,
`not_found`, `nothing_to_undo`, `nothing_to_redo`, `file` or `unsaved_changes`), `-32001` batch failed
(`data` is `{index, error, completed, rolled_back}`) and `-32002` rendering unavailable.
