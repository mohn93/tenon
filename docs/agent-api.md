# Agent API

Every edit the window can make is also a command agents can send. The API is JSON-RPC 2.0 with one
JSON object per line (UTF-8, `\n`-terminated).

## Starting

| Command | Where the API is |
|---|---|
| `cargo run -p app` | Window, plus a Unix socket, or a named pipe on Windows (printed on stderr: `agent API listening on ...`) |
| `cargo run -p app -- --socket PATH` | Window, socket at `PATH` (on Windows, a pipe name; see below) |
| `cargo run -p app -- --headless --stdio` | No window; requests on stdin, replies on stdout; exits when stdin closes |
| `cargo run -p app -- --headless` | No window; socket at the default path (printed on stderr) |
| `cargo run -p app -- --headless --socket PATH` | No window; socket at `PATH` |
| `cargo run -p app -- --api-root DIR [--api-root DIR2]` | Any mode: the API reads and writes files only inside these folders (see Access and safety) |

Any of these can end with a `.skr` file to open at start, e.g.
`cargo run -p app -- --headless --stdio house.skr`. In headless mode a file that cannot be opened
stops the app with exit code 1 and a message on stderr.

The default socket is `sketchup-alt-<pid>.sock`. On macOS it is in the temp directory, or in
`~/.sketchup-alt/` (created with mode 0700) when the temp directory path is too long for a Unix
socket or is the shared `/tmp`. On Linux it is in `$XDG_RUNTIME_DIR` when that is an absolute path
to a folder you own that others cannot write to, else in `$TMPDIR` when that is set to a short
folder other than `/tmp`, else in `~/.sketchup-alt/`. The socket is readable only by the current user. Try it by hand
with `nc -U PATH`. The window removes the socket file on a normal quit. The default name includes the process id, so a socket left behind
by a killed app is never reused; agents that start the app should pass `--socket PATH` (a fixed
path is replaced on the next start).

On Windows the API listens on a named pipe instead: `\\.\pipe\sketchup-alt-<pid>` by default.
`--socket NAME` and `--live NAME` take a pipe name; a bare name such as `main` becomes
`\\.\pipe\main`. Only the current user can open the pipe (its access list names that user
alone), clients on other machines are refused, and a name another program is already listening
on is refused (`cannot listen on ...`) rather than shared. The pipe goes away when the app exits.
Pipe names are shared by every user on the machine, so `app --mcp --live` checks that the
process serving the pipe runs as the current user, and refuses to connect otherwise (`the pipe
... belongs to another user; not connecting`): another user who created the name first does not
receive what the agent sends.

## MCP

`app --mcp` serves this API as an MCP server on stdin/stdout, headless or forwarding to a running
app with `--live SOCKET` (a pipe name on Windows). Setup for Claude Code and Claude Desktop is in [mcp.md](mcp.md).

## Methods

Call `describe` first: it returns every method, notification, error code and the JSON Schemas of
all commands and results.

- `schema_version` is 2.
- Each method has `params_schema` and `result_schema`: JSON Schemas for its params and result,
  each self-contained (its `$ref`s point into its own `$defs`). `execute` params are any of
  Command, ScreenCommand and EditorCommand; `query` params any of Query and EditorQuery.
  `subscribe` and `describe` take no params (`params_schema` null).
  In `batch` params an `EntityId`, `MaterialId` or `DefinitionId` may also be a `BatchRef` (see References inside
  a batch).
- The `schemas` entries for Command, Query, EditorQuery, EditorCommand and ScreenCommand have
  `"type": "object"` at the root beside their `oneOf`.
- Model points (positions, not directions, normals or move offsets) are arrays of three numbers
  each within +/-1e6 m, the limit commands check, and the schemas say so.

| Method | Params | Result |
|---|---|---|
| `execute` | a Command, e.g. `{"type": "add_edge", "start": [0,0,0], "end": [1,0,0]}` | `{changes, revision}`, plus `import` for an `import` (see Import below), `soften` for `soften_edges` and `soften_by_angle`, `changed` (the ids changed) for `hide`, `unhide`, `unhide_last`, `unhide_all`, `reverse_faces` and `flip` (see Hide and unhide and Reverse and orient faces below), `orient` for `orient_faces`, `intersect` for `intersect_faces` (see Intersect Faces below), `positioned` for `position_texture`, `purged` for `purge_definitions`, `definitions` for `make_unique` (see Make Unique and Flip Along below), `text_3d` for `add_3d_text` (see 3D text below), and `units` (the display units now) for `set_units` and an `undo` or `redo` of a `set_units` step |
| `batch` | `{"commands": [...], "atomic": true}` | `{outcomes, changes, revision}`; `changes` is the net effect of the whole batch (made then erased is absent); atomic = one undo step, all or nothing. Steps made through the API are `agent` steps |
| `query` | a Query, see the table below | the QueryResult |
| `render_view` | `{"width": 640, "height": 400, "camera": {"eye": [...], "target": [...]}}` (camera optional; it also takes `fov_y_deg`, `up` and `projection` as `set_camera` does; `lighting`, `day` or `night`, is optional and defaults to the document's) | `{width, height, png_base64}` |
| `subscribe` | none | `{revision}`, then `changed` notifications after every model change; a `resync` notification (`{revision}`) means changes were missed, so re-read with `{"type": "entities"}` (its `revision` tells you where you are). Also `selection`, `camera` and `editor` notifications; see Notifications below |
| `tool_input` | `{"events": [{"type": "click", "point": [0,0,0]}, {"type": "type", "text": "3"}], "width": 640, "height": 400}` (size optional; as written this draws a line, so the Line tool must be active: the editor starts in Select) | `{editor, inference, preview, commands, revision}`: the active tool fed the events as the window feeds a user's; see Driving the tools below |
| `describe` | none | the API description |
| `quit` | `{"discard": true}` (optional; default false) | `{}`, then the app ends once its replies are sent (window or headless; a headless socket's file is removed). With unsaved changes and no `discard` it fails with `unsaved_changes` and goes on. In the window it ends without asking about unsaved changes. Once it is accepted, every later request, including those already waiting with it, is refused (`-32000`, `invalid_input`, "the program is quitting"), so no edit is made that would not be saved. Camera changes do not count as unsaved (see Camera), so `quit` without `discard` does not keep a view moved since the last save; `save` first to keep it. Params go by name: `[true]` is `-32602` |

Queries:

| Query | Result |
|---|---|
| `{"type": "entities"}` | every vertex, edge, face, instance and guide of the open context, plus `revision` |
| `{"type": "entities", "kinds": ["face"]}` | only the listed kinds (`vertex`, `edge`, `face`, `instance`, `guide`) |
| `{"type": "entities", "ids": ["v0", "e2"]}` | only the listed ids; unknown ids are absent, not an error |
| `{"type": "entities", "within": {"min": [0,0,0], "max": [2,2,1]}}` | only entities whose vertices (an instance: its bounding box; a guide point: its position) all lie in the box, bounds included; an instance with nothing in it and a guide line are in no box. `ids`, `kinds` and `within` combine (all must match); empty lists mean no filter |
| `{"type": "entity", "id": "f7"}` | one entity, or a not-found error; a dimension (`r5`) or note (`n6`) gives its info with its label (see Dimensions and notes), a section plane (`s8`) its point, normal and whether it is active (see Section planes) |
| `{"type": "annotations"}` | every dimension and note of the open context, each as `entity` gives it, plus `revision`; see Dimensions and notes below |
| `{"type": "section_planes"}` | every section plane of the open context with whether it is active, plus `revision`; see Section planes below |
| `{"type": "section_display"}` | `planes`, `cuts` and `fill`: how section planes are shown; see Section planes below |
| `{"type": "units"}` | the model's display units: `length` and `precision`; see Dimensions and notes below |
| `{"type": "curve", "id": "e3"}` | the curve (circle or arc) edge `e3` is part of; see Curves below |
| `{"type": "stats"}` | counts for the open context (plus the number of definitions) and revision |
| `{"type": "history"}` | `undo_depth` and `redo_depth`, and the steps as labels, next first: `undo` (what `undo` would undo, then the step before it) and `redo` (what `redo` would redo first). A step's label is the `type` of the command that made it (`add_edge`, `push_pull`); an atomic batch is labeled by its edit commands' type when they share one, else `batch`. Window edits are labeled the same way. Plus `undo_origins` and `redo_origins`, who made each step in the same order: `user` (in the window) or `agent` (through the API); and `revision` |
| `{"type": "camera"}` | the view: `eye`, `target`, `up`, `fov_y_deg`, `projection`, `view_height`, `lighting`, `color_by_tag`, `x_ray`, `hidden_geometry`, `shadows`, `axes`; see Camera below |
| `{"type": "location"}` | where the model stands: `latitude`, `longitude`, `north_angle`, `utc_offset`; see Location and sun shadows below |
| `{"type": "sun"}` | the sun at the shadow settings' date and time: `direction`, `azimuth`, `elevation`, `up`; see Location and sun shadows below |
| `{"type": "selection"}` | the selected edge, face, instance, guide, dimension, note and section plane ids, plus `revision` |
| `{"type": "definitions"}` | every group and component definition: `id`, `name`, `kind`, how many `instances` place it, `edges`, `faces` |
| `{"type": "materials"}` | every material: `id`, `name`, `color`, `opacity`, `emission`, its `texture` if it has one, and how many `faces` and `instances` in the whole model are painted with it; see Materials below |
| `{"type": "tags"}` | every tag: `id`, `name`, `color`, `visible`, and `count`, how many edges, faces, instances, guides, dimensions, notes and section planes in the whole model are on it; see Tags below |
| `{"type": "scenes"}` | every scene, in order: `id`, `name`, whether it stores a view (`camera`) and tag visibility (`tags`), and whether it is `selected`; plus `revision`; see Scenes below |
| `{"type": "scene", "scene": "p40"}` | one scene: `id`, `name`, the `view` it stores (null when none) and the tags it hides (`hidden_tags`, null when it does not store tag visibility); see Scenes below |
| `{"type": "context"}` | the open context: `path` (open instances from the root, empty at the root) and the `definition` being edited |
| `{"type": "outliner"}` | the tree of placed groups and components in the whole model, plus `revision`; optional `depth` limits it; see Groups and components below |
| `{"type": "file"}` | `path` of the file last opened or saved (`null` if never), `modified` since then, `revision`, and `api_roots`: the folders `--api-root` limits files to, resolved (`null` without it) |
| `{"type": "library", "folder": "/abs/library"}` | the library items in a folder: `{"type": "library", "items": [{"path", "name", "kind", "modified", "size", "error"?}], "more"?}`; see Library items below |
| `{"type": "events", "since": 3}` | model changes after revision 3, oldest first |
| `{"type": "pick", "pixel": [320, 200], "width": 640, "height": 400}` | what lies under a pixel of a view, or along a `ray`: `hits`, front to back; see Picking and inference below |
| `{"type": "infer", "pixel": [320, 200]}` | what a tool's click there would snap to: `kind`, `point` and the `entities` involved; see Picking and inference below |
| `{"type": "editor"}` | the editor around the model: tool, its phase and points, paint material, import units, fly mode, view size, status bar, measurements box, open window; see Editor below |

Unknown fields in commands, queries and params are rejected with `-32602` (a typo fails instead
of silently doing nothing), and so are params given by position (an array).

Commands: `add_edge`, `add_rectangle`, `add_circle`, `add_polygon`, `add_arc`, `add_curve`, `add_rotated_rectangle`, `push_pull`, `erase`, `move`, `copy`, `rotate`, `scale`, `offset`, `follow_me`, `make_group`,
`make_component`, `explode`, `make_unique`, `flip`, `set_axes`, `reset_axes`, `open_instance`, `close_instance`, `open_path`, `rename_definition`,
`purge_definitions`, `add_guide_line`, `add_guide_point`, `delete_guides`, `add_dimension`,
`add_edge_dimension`, `add_radial_dimension`, `add_note`, `add_screen_note`, `change_note`,
`add_3d_text`, `set_units`, `add_section_plane`, `flip_section_plane`, `activate_section_plane`,
`deactivate_section`, `set_section_display`, `add_material`,
`change_material`, `delete_material`, `paint`, `add_tag`, `change_tag`, `delete_tag`, `set_tag`, `add_scene`, `update_scene`, `change_scene`, `move_scene`, `delete_scene`, `go_to_scene`, `position_texture`, `soften_edges`, `soften_by_angle`, `hide`, `unhide`, `unhide_last`, `unhide_all`, `reverse_faces`, `orient_faces`, `intersect_faces`, `paste`, `save_to_library`, `insert_library_item`, `select`, `undo`,
`redo`, `set_camera`, `orbit`, `pan`, `zoom`, `zoom_extents`, `standard_view`, `set_projection`, `look_around`, `position_camera`,
`set_lighting`, `set_color_by_tag`, `set_x_ray`, `set_hidden_geometry`, `set_shadows`, `set_location`, `new`, `open`, `save`, `export`, `import`.
Units are meters, Z is up, angles in degrees, coordinates within +/-1,000,000 m.
Entity ids are strings: `v12` (vertex), `e3` (edge), `f7` (face), `i4` (instance of a group or
component), `g9` (guide line or guide point), `r5` (dimension), `n6` (note), `s8` (section plane). Definitions are `d5`, materials `m6`, tags `t7`, scenes `p8`. Ids are unique across the whole model, including inside
groups and components.

Modeling commands, each one undo step:

| Command | Effect |
|---|---|
| `{"type": "add_edge", "start": [x,y,z], "end": [x,y,z]}` | A straight edge. It merges with nearby vertices, splits edges it crosses, and closed coplanar loops become faces. |
| `{"type": "add_rectangle", "corner": [0,0,0], "opposite": [2,3,0]}` | A rectangle with sides along the drawing axes (the world axes unless moved; see Axes). Without `normal` the corners must differ in exactly two coordinates along them. With `"normal": [nx,ny,nz]` it lies in the plane through `corner` with that normal; `opposite` must be in that plane. Sides then run along u = `Z x normal` (horizontal in the drawing axes, with Z their blue axis; their red axis on planes normal to it) and v = `normal x u`, so directions are fixed only up to sign: on a plane with normal +Y, u = -X and v = +Z; with normal -Y, u = +X. |
| `{"type": "add_circle", "center": [0,0,0], "radius": 1.5}` | A circle as a regular polygon with `segments` sides (default 24, 3 to 360), in the plane through `center` with `normal` (default `[0,0,1]`). The first corner lies along the plane's u axis (as for `add_rectangle`) and the corners run counter-clockwise around the normal. The radius must be positive, and each side (2r sin(pi/segments)) must be at least 0.01 mm, ten times the merge tolerance. Its edges form one curve (see Curves). |
| `{"type": "add_polygon", "center": [0,0,0], "radius": 1.5}` | A regular polygon with `sides` sides (default 6, 3 to 360), in the plane through `center` with `normal` (default `[0,0,1]`). Inscribed (the default), `radius` is the distance to the corners and the first corner lies along `direction`; with `"circumscribed": true`, `radius` is the distance to the middles of the sides and the middle of the side ending at the first corner lies along `direction`. `direction` (default the plane's u axis, as for `add_rectangle`) is projected into the plane and must not lie along the normal. Corners run counter-clockwise around the normal. The radius must be positive, and each side must be at least 0.01 mm. Its edges form one curve of kind `polygon`; pulling it keeps the sides hard (no soft edges), unlike a circle. |
| `{"type": "add_arc", "center": [0,0,0], "start": [2,0,0], "angle": 90}` | An arc around `center` from `start`, turning `angle` degrees counter-clockwise around `normal` (default `[0,0,1]`; a negative angle turns clockwise), as `segments` equal straight edges (default 12 whatever the angle, 2 to 360) that form one curve. `start` must lie in the plane through `center` with that normal (within 0.001 mm); the radius is its distance from `center`. The angle must be more than 0 and less than 360 degrees in size (a whole circle is `add_circle`), and each segment at least 0.01 mm long. Its points snap and split like `add_edge`'s, so an arc ending on existing edges can close a face. |
| `{"type": "add_arc", "start": [0,0,0], "end": [2,0,0], "through": [1,1,0]}` | The arc from `start` to `end` passing through `through`, the way three clicks pick it: the three points set its plane, so `normal` is not allowed (the curve query reports the normal about which start, through and end run counter-clockwise). The points must be apart and not on one line. `segments` as above. |
| `{"type": "add_arc", "start": [0,0,0], "end": [2,0,0], "bulge": 1}` | The arc from `start` to `end` whose middle lies `bulge` meters from the chord, to the left of start-to-end looking down `normal` (default `[0,0,1]`; negative: to the right), as the Arc tool pulls it: a bulge of half the chord is a half circle. `start` and `end` must lie in one plane with the normal and apart; the bulge must not be 0. Give `center` with `angle`, or `end` with `through` or `bulge`; any other mix is `invalid_input`. |
| `{"type": "add_curve", "points": [[0,0,0],[1,0,0],[1.5,1,0]]}` | The polyline through `points` (2 to 10,000; consecutive duplicates, and a last point repeating the first of a closed one, are dropped), with the rules of `add_edge` (faces form where it closes a loop). `closed` (default false) joins the last point to the first and needs 3 distinct points that do not lie on one line. With `curve` (default true) its edges form one curve, as an arc's do (a straight polyline gets none); `"curve": false` draws plain edges. One undo step. |
| `{"type": "add_rotated_rectangle", "corner": [0,0,0], "edge_end": [2,2,0], "width_point": [-0.5,1.5,0]}` | A rectangle at any angle: the first side runs from `corner` to `edge_end`, the width is the distance from `width_point` to the line of that side, and the rectangle lies in the plane through the side and `width_point`, toward it. A zero side or a `width_point` on the side's line is `invalid_input`. Its face forms as for `add_rectangle`; one undo step. |
| `{"type": "move", "ids": ["f8"], "offset": [0, 0, 1]}` | Moves vertices, edges and faces (their vertices), instances, guides, dimensions and notes by `offset` (an instance must keep its bounding box, or its origin when it holds nothing, a guide its point and an annotation its points, within the coordinate limit; so must copies). An annotation's fixed points, offset and text position move; its vertex anchors move with their vertices (only when those are moved too), and a dimension whose two ends the move brings together is removed (undo brings it back). Faces that share only some moved vertices stretch and may tilt but must stay flat: a move that would bend one is `invalid_input` naming that face. A move that would collapse an edge, turn a face over, make a face cross itself, push a hole out of its face or land a vertex on another is `invalid_input` naming the ids. Moved geometry does not merge with what it ends up touching. Unknown ids are `not_found`. |
| `{"type": "copy", "ids": ["f8"], "offset": [3, 0, 0], "count": 4}` | Copies edges, faces, instances, guides, dimensions and notes `count` times (default 1, at most 1,000); copy k is shifted by `k * offset`. Copies are drawn like new edges, so they merge with what they touch: a copy has the faces its original had, made even where their sides already exist as bare edges, plus faces it closes together with existing edges. A copy drawn on a face splits it and every piece stays a face (also under an opening of the copy); elsewhere an opening in a copied face stays open and a loop the original had no face in gets none. Copied faces keep the orientation of their originals. A copied component instance places the same definition; a copied group gets a definition of its own. A copied dimension or note anchored to a vertex copied with it anchors to that vertex's copy; its other vertex anchors become fixed points where the copy puts them, and a radial dimension whose circle is not copied keeps it as a fixed circle. Vertex ids are `invalid_input`. |
| `{"type": "paste", "clip": "sketchup-alt clip 1\n...", "at": [3, 0, 0]}` | Pastes clip text (what Copy puts on the clipboard; see Clipboard) into the open context, as one undo step labelled `paste`, and selects what it pasted. With `at` (a point of the open context), the clip's base point lands there, moved only (not turned); without it (Paste In Place), the clip lands where it was copied from in world coordinates, wherever it is pasted. Edges and faces merge with what they touch, as `copy`'s do, and painted faces keep their texture positions. Materials and tags are matched by name: an existing one of the same name is used, a missing one is added (textures included). A component instance copied from this same document (since it was created, opened or imported as a new model) places its definition when that still exists; otherwise it places the definition an earlier paste of a clip from the same source made for it, while that still exists (the document remembers these until a new model is created or opened, and does not save them; an undone paste's definitions are gone, so pasting again makes new ones); otherwise, and for every group, a new definition is made (nested ones included). Text that is not a clip, is damaged, holds a model a `.skr` file could not hold, decodes to over 64 MiB, nests instances more than 32 deep, places over 100,000 instances or shows over 20,000,000 faces (a nested instance or face counted once for every placement of what holds it), or lands beyond the coordinate limit is `invalid_input` and changes nothing. |
| `{"type": "save_to_library", "path": "/abs/lib/chair.skr", "kind": "component", "item": "i4", "name": "Chair", "overwrite": false}` | Saves a library item (see Library items). `kind` `component`: `item` is a component instance of the open context (a group is `invalid_input`, "Make it a component first"); `material`: `item` is a material id; `model`: no `item`, the whole model. `name` defaults to the component's or material's name, or the file's stem. The path must end in `.skr`, must not be the open model's own file, and an existing file is replaced only with `overwrite` (else `invalid_input`). Not an undo step; the model and its revision do not change. `item` takes a batch reference. Not allowed in an atomic batch. |
| `{"type": "insert_library_item", "path": "/abs/lib/chair.skr", "at": [3, 0, 0]}` | Inserts a library item into the open context as one undo step labelled `insert`. A component or model item: the file is read with every check `open` makes and the clip limits `paste` has, its definitions are always new (inserting twice makes two), its materials and tags are matched by name as `paste` does, its world orientation and size are kept (inside a turned or scaled group it is not turned or scaled again) and the minimum corner of its world bounding box lands at the world point of `at` (a point of the open context) or, without it, of the open context's origin, and what it made becomes the selection. A material item: added to the model; a material of the same name and the same color, opacity, emission and texture is reused, otherwise it is added as "<name> 2", "<name> 3"... (the lowest free name); the outcome's `material` is its id. A name too long for "<name> N" is shortened to fit the 256-character limit before the number. Reusing an identical material changes nothing, so it makes no undo step and no change is reported, so it cannot be referenced by `$N` in a batch; the outcome's `material` names it. Inserting a model item inserts its geometry as `paste` does: its scenes and camera are not brought in. Component and model items must pass the clip limits (nesting depth, expanded instances, placed faces) before anything walks them: every read of an item (insert, `place_library_item`, the window's thumbnails and Open from the Library tab) is refused with `invalid_input` otherwise, though `open` of the same file is unchanged. Any `.skr` file is a model item. |
| `{"type": "rotate", "ids": ["f8"], "center": [0, 0, 0], "angle": 90}` | Turns vertices, edges and faces (their vertices), instances, guides and annotations (as `move` moves them) by `angle` degrees about the line through `center` along `axis` (default `[0,0,1]`), counter-clockwise looking down the axis (right-hand rule), under the rules of `move`: faces that share only some turned vertices must stay flat, and so on; geometry turned whole keeps its shape. Multiples of 90 degrees are exact. An instance turns with its transform. The `axis` is a direction of any length; a zero or non-finite one and whole turns (0, 360, ..., or within 1e-9 degrees of one) are `invalid_input`. With `"copies": n` (1 to 1,000) the originals stay and n copies are made under the rules of `copy`, copy k turned by `k * angle` (a radial array; vertex ids are then `invalid_input`, and so is a count where a copy would complete a whole turn and land on the original, e.g. angle 90 with 4 copies). Vertices on the axis do not move, and a rotation that moves nothing changes nothing. |
| `{"type": "scale", "ids": ["f8"], "origin": [0, 0, 0], "factors": [2, 1, 1]}` | Scales vertices, edges and faces (their vertices), instances, guides and annotations (as `move` moves them; a fixed circle keeps its area, and a radial dimension scaled with a circle that the scale makes an ellipse keeps that circle, scaled) by `factors` along model X, Y and Z about `origin`, under the rules of `move`: faces that share only some scaled vertices must stay flat, and an edge scaled shorter than the merge tolerance is `invalid_input`. Each factor must be finite and between 1e-4 and 1e4 in size, else `invalid_input`; all three exactly 1 is `invalid_input` too. A negative factor mirrors: faces scaled whole keep facing out (their loops are reversed), also a face lying on the mirror plane, which is turned over though none of its vertices move (a ground rectangle scaled `[1, 1, -1]` about z = 0 then faces down); a face that would fold over is `invalid_input`. Scaling that would bring two vertices, or a face's sides or holes, within the merge tolerance of each other is `invalid_input`, and so is squeezing an instance so flat that its transform's determinant falls below 1e-12 (naming it). An instance's transform is scaled (also non-uniformly, and along model axes when it is turned), and a guide line turns with the scaled direction. No copies. Empty `ids` is `invalid_input`; unknown ids are `not_found`. |
| `{"type": "offset", "ids": ["f8"], "distance": 0.1}` | Draws a copy of a face's loops, or of a chain of edges, moved `distance` meters sideways in its plane, as one undo step; what it draws merges with what it touches like any drawn edge (an inward offset splits the face into the inner face and a ring around it). `ids` is one face, or edges forming one unbranched chain in one plane (open, or closed into a loop). For a face, a positive `distance` offsets inward (the outer loop shrinks, the holes grow) and a negative one outward (only the outer loop: the holes stay as they are); `toward` is not allowed. For edges, `toward` (a point in the edges' plane, not on them or straight ahead of an end; for a straight chain it picks the plane too) is required and picks the side, and `distance` must be positive; the ends of an open chain move square to their own edge (an arc's ends move along its radii, so its offset is an arc around the same center). An edge offset may touch or cross the original edges (a chain that bends back on itself), which are then split as a hand drawing splits them. Corners are mitred; a corner on the outside of a turn whose miter would reach farther than 4 times the distance is bevelled (two corners). A loop or chain whose edges are all on one curve gives a new curve of the same kind (a circle gives a circle of as many segments, an arc an arc, a polygon a polygon). `distance` must be finite and at least 0.01 mm in size, else `invalid_input`; so is an offset the shape is too small for ("the offset distance is too large for this shape": a side would reverse or vanish, the offset would cross itself, or, for a face, a new loop would leave the face or meet another loop; outward, the new outer loop must enclose the face), edges that fall apart or branch, edges not in one plane, and anything but one face or edges. Unknown ids are `not_found`. |
| `{"type": "follow_me", "face": "f8", "path": ["e2", "e3"]}` | Sweeps a face (the profile) along `path`, edges forming one connected chain (open, or closed into a loop; branching is `invalid_input`), as one undo step. The sweep starts at the path end nearest the face's plane (for a closed path the path vertex nearest it, passing over one where the path turns within the face's plane; on a tie the one nearer the middle of the face's outer loop, then the first in chain order, and a closed path is walked toward the neighbour that leaves the face's plane more steeply), and the face is carried as it is, not moved onto the path: a face drawn beside a circle sweeps around it (a lathe). The face keeps its place relative to the path as the path turns (each segment turns it by the turn between the two directions); a vertex in the middle of a straight run of the path is no corner (the run is swept as one segment); corners are mitred, and a closed path that would come back twisted has the twist spread over its segments (the sides are then split into two triangles with a soft diagonal). Every profile side and path segment gives a side face; an open path gets a cap at each end (square to the first and last segment, with the face's holes and its front and back materials), a closed one none. The profile face is removed; its edges stay where they bound new faces, except where one lies across a side face (a face not square to the path), which is erased so the side stays one face. What is drawn merges with what it touches like any drawn edge, and faces point out of the swept solid. An edge between the faces of two profile sides on one curve that is not a polygon is soft (a swept circle is smooth, a polygon is not), and so is an edge between the faces of two path segments on one such curve (a ring around a circle has soft joints). `path` may not include the face's own edges unless it is exactly the outer loop of another face. Sweeping into an existing solid merges with it by sticky geometry but does not subtract from it (an end cap landing on an existing face in its plane leaves that face: the two merge, the cap is not cut out of it), and only folds between neighbouring segments are caught: a path that comes back near itself can make a sweep that crosses itself. `invalid_input`: `face` not a face, `path` empty or not all edges, a path edge shorter than 0.01 mm, a profile too big for a turn of the path ("the profile is too big for a turn of the path": a point of it would not move forward between two corners), a path that turns straight back or runs in (or within a hair of) the face's plane where it starts ("the path turns straight back, or the profile lies along the path": a section would be under 0.01 mm across); unknown ids are `not_found`. A refusal changes nothing. |
| `{"type": "erase", "ids": ["e2", "f8"]}` | Erases edges, faces, instances, guides, dimensions and notes (a group's definition goes with its last instance). An erased face leaves its edges. An erased edge takes the faces it bounds with it, except that two faces in one plane meeting at it merge into one. Vertices go with their last edge; a vertex left between two collinear edges is healed away. Faces are erased before edges, so erasing a face together with the seam it shares with a coplanar neighbour removes the neighbour too: the seam then bounds only one face. Vertex ids are `invalid_input` and unknown ids are `not_found`, both with `ids`; empty `ids` is `invalid_input`. |
| `{"type": "make_group", "ids": ["e2", "f8", "i4"]}` | Moves edges, faces (with their edges), instances, guides, dimensions and notes (keeping their ids) into a new group definition and places one instance of it where they were; the instance becomes the selection. Edges that still bound a face left behind stay outside too. An annotation anchored to a vertex that goes inside stays anchored to it; one anchored to a vertex left outside anchors to where that vertex is, and a radial dimension whose circle stays outside keeps it as a fixed circle. Screen notes stay in the root. Vertex ids are `invalid_input`, and so are ids naming only guides and annotations. |
| `{"type": "make_component", "ids": ["f8"], "name": "Chair"}` | Like `make_group`, but makes a named component: its copies share the definition, so editing one edits them all. An empty name is `invalid_input`. |
| `{"type": "explode", "ids": ["i4"]}` | Replaces instances by their geometry, which merges with what it touches; instances, guides, dimensions and notes inside come out as such (with new ids; annotations stay anchored to their vertices, placed by the instance's transform). What comes out becomes the selection. Ids that are not instances are `invalid_input`, and so is an instance whose geometry, guides or annotations would land beyond the coordinate limit (naming it). |
| `{"type": "make_unique", "ids": ["i4"]}` | Gives shared component instances a definition of their own (`Chair#2`); see Make Unique and Flip Along below. `ids` defaults to the selection (when absent). The outcome's `definitions` lists the new definitions |
| `{"type": "flip", "ids": ["i4"], "axis": "red", "frame": "axes"}` | Mirrors the selection about the center of its box along the red, green or blue axis (`frame` `own`: one instance, along its own axis); see Make Unique and Flip Along below |
| `{"type": "push_pull", "face": "f8", "distance": 1.5}` | Moves a face along its normal (negative: against it). A face on a flat sheet extrudes into a solid and stays as its base. A face on a solid extrudes a bump, or cuts a recess when pushed in; pushed in exactly as far as a parallel face that holds it (the back of a wall), it cuts an opening through. A face whose neighbors all run along the normal, like a box top, moves and stretches them; moved back flush with the surface around it (a recess or bump pushed back), it closes up and the seams heal. When the face is extruded, its sides along a curve get a moved copy on the cap that is a curve of its own, and the swept edges between two sides of one curve are soft (`"soft": true`), so a pulled circle is a cylinder with a smooth side. |

`{"type": "select", "ids": ["f8"], "mode": "replace"}` changes the selection (edges, faces,
instances, guides, dimensions and notes; `mode` is `replace` (default), `add`, `remove` or `toggle`). It is not an undo step and
does not change the revision. The window shows the selection and agents see what the user
selected with the `selection` query. Vertex ids are `invalid_input` and unknown ids are
`not_found`, both with `ids`. Empty or absent `ids` with mode `replace` clears the selection
(Select None).

`{"type": "select", "all": true, "mode": "replace"}` selects every edge, face, instance, guide,
dimension, note and section plane (while section planes are shown) of the open context that no hidden tag hides (see Tags), combined with the current selection by `mode` as `ids` are. `ids` must then
be absent or empty: giving both is `invalid_input` ("select takes ids or all, not both").

`{"type": "select", "box": {"from": [100, 80], "to": [300, 240]}, "mode": "add"}` selects by a box
on screen instead of `ids`, as the Select tool's box does: of the open context, the edges, faces,
instances (by their bounding boxes), guide points, and dimensions and leader notes (their lines
and label points) lying entirely inside it, or with `"crossing": true` everything it touches (the
window's box dragged left), guide lines too. Screen notes go by their labels, which only the
window lays out, so a box given here never selects them.
Corners are pixels of a view seen by the document camera (`width` and `height`, both or neither,
default the 3D view's size); a corner outside the view is `invalid_input`. Only `execute` takes a
box (not `batch`).

The selection drops any entity that stops existing, through erase, merges, undo or redo. Undo does
not restore the selection. The `selection` query's `revision` is the model revision: it does not
change when only the selection changes. Subscribers get a `selection` notification when it
changes (see Notifications).

`{"type": "undo"}` undoes the last step and `{"type": "redo"}` redoes the last step undone;
`"count": 3` takes three steps (default 1, at least 1), each its own revision, and the outcome's
`changes` are their net effect. With fewer steps than `count` it fails (`nothing_to_undo`,
`nothing_to_redo`: "only 1 step to undo") and changes nothing. A new edit clears what could be
redone. Undo and redo are not allowed in an atomic batch.

`undo` takes an optional `expect` to guard against undoing someone else's step:
`{"type": "undo", "expect": {"label": "push_pull", "origin": "agent"}}` undoes only if the next
step to undo has that label (and, when given, that origin, see `history`); otherwise it fails
(`unexpected_step`, naming the step found) and changes nothing. With nothing to undo it still
fails `nothing_to_undo`.

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

### Clipboard

Copy turns a selection into clip text: the line `sketchup-alt clip 1`, then base64 (standard
alphabet; line breaks are ignored) of a zip archive holding `model.json`, the selected edges,
faces, instances, guides, dimensions and notes with every definition, material and tag they use, written as a
`.skr` file writes its document (textures as entries beside it), and `clip.json`
`{"base": [x, y, z], "source": "<token>"}`. The clip is stored in world coordinates (what was
copied inside an open group is placed by the group's transform); `base` is the minimum corner of
its bounding box, and `source` names the document it came from. `paste` reads it with every
check opening a `.skr` file makes. The window's Edit menu and the editor commands
`copy_selection`, `cut_selection`, `paste_clipboard` and `paste_in_place` (see Editor) put clip
text on the app's clipboard and paste from it; in the window that is the system clipboard, so
clips pass between app processes.

### Library items

A library item is a `.skr` file with one more archive entry, `library.json`
`{"kind": "component" | "material" | "model", "name": "..."}`; readers that do not look for it
ignore it, and a `.skr` file without it is a model item named by its file name. A component item's
root holds one instance of the component at the origin, with the rotation and scale it had
(the translation is removed), and the materials and tags it uses; a material item holds only
that material (texture included); a model item is the whole model. `save_to_library` writes them
and `insert_library_item` places them (see Commands). A model opened from a library item (`open`)
remembers its `library.json`, and every save of it (`save`, to its own path or another) writes the
entry back, so the saved file is still the same kind and name; `new` and opening a file without
the entry forget it. A recovered copy (autosave) keeps the entry too.

The `library` query lists a folder: the `.skr` files directly in it (no subfolders), sorted by
name ignoring case, at most 1000 (`more` is true when the folder holds more; then the first
1000 by file name are listed). Links (symbolic links, and on Windows any reparse point),
folders and files over 256 MiB are left out. Only the archive's directory and `library.json`
(at most 16 KiB) are read to list a file, never the model. `kind` and `name` come from
`library.json` when it is valid (a name of 1 to 256 characters without control characters),
otherwise the item is a `model` named by its file stem; a file that is not a model archive is
listed with an `error`. `modified` is the file's time in seconds since 1970 and `size` its size in bytes. A path that is not a
folder is `file`, except under `--api-root`, where it is `invalid_input` (as every other
roots refusal is). A file with more than 10,000 archive entries is listed with an `error`, and a
`library.json` over 16 KiB is ignored (the file is a model named by its stem). The `.skr`
extension is matched ignoring case. With `--api-root` the folder, `save_to_library`'s path and
`insert_library_item`'s path must lie within the roots (the root itself is a folder to list); so
must `place_library_item`'s.

The window keeps the library folder in its preferences (Settings > Files > Library folder:
Choose..., Clear) and lists it in the Library dock tab, after Shadows. With no folder the tab says
"Choose a library folder". Otherwise it has a search field (a case-insensitive part of the name), a
kind filter (All, Components, Materials, Models) and a grid of tiles, a thumbnail and a name each.
The list is made again when the tab is shown, after a save or a delete, and when the window gains
focus (no folder watching). Thumbnails are made lazily for the tiles in view, at most two a frame,
128 x 128 pixels, in an iso view fitted to the item on white paper, kept in memory by file and
modification time; a material's is its color or texture, and an item that cannot be drawn shows an
icon; one that is over the clip limits shows the icon with the reason as its hover text and cannot
be placed, inserted or opened from the tab. The thumbnail is made again when the file's time or size
changes. A click on a component or model tile is `place_library_item`; a click on a material tile is
`insert_library_item` and the material becomes `set_paint_material`'s. The tile's context menu has
Insert (`insert_library_item` at the open context's origin), Open (`open` with the file, asking
about unsaved changes first) and Delete from Library (after a confirmation the file is removed, only
when it is a regular `.skr` file directly in the folder and not a link). Saving asks for a name in a
sheet and writes `<folder>/<name>.skr` as `save_to_library` does (characters a file name may not
hold become `_`; an existing file asks before it is replaced): Entity Info's Save to Library for one
selected component instance, the Materials panel's Save to Library for the chosen material, the
context bar's Save Component to Library button, and File > Save Model to Library.... They need a
folder.

### References inside a batch

In a `batch`, a command may refer to ids an earlier command of the same batch made, instead of
predicting them:

```json
{"commands": [
  {"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [4, 3, 0]},
  {"type": "push_pull", "face": "$0:f0", "distance": 2.5}
], "atomic": true}
```

A reference is a whole string matching `^\$(0|[1-9][0-9]*):(([vefigrnsmd])(0|[1-9][0-9]*|\*)|\*)$`:

- `"$N:KI"` is the I-th id (from 0) of kind K that command N made: `v` vertex, `e` edge, `f`
  face, `i` instance, `g` guide, `r` dimension, `n` note, `s` section plane from its `changes.created` (lowest id
  first, which is the order they were made), `m` material from its `changes.materials`, or `d` definition from its `definitions` (a `make_unique` outcome).
- `"$N:K*"` is all ids of that kind command N made, `"$N:*"` every edge, face, instance, guide,
  dimension, note and section plane it created (not vertices, which most commands refuse; `"$N:v*"` gives those; not materials).
  `*` references are allowed only in id lists (`ids`, and `path` of `open_path` and `follow_me`), where they
  expand in place, keeping order; one that expands to nothing leaves nothing in its place.
- N counts commands of the batch from 0 and must be lower than the referring command's own
  position.

References are read only in a command's top-level `ids`, `face`, `id` and `material`, in
`path` of `open_path` and `follow_me`, in `a` and `b` of `add_dimension`, `edge` of
`add_edge_dimension` and `add_radial_dimension`, `anchor` of `add_note`, `note` of
`change_note`, `plane` of `flip_section_plane` and `activate_section_plane`, and `definition` of `rename_definition`. Any other string, such as a name or a file path, stays as written even
when it looks like a reference. `material` takes only `m` references, `definition` only `d` references, the other fields only
entity references; whether the entity suits the command (a face for `push_pull`) is checked by
the command, as for a written id. Tag ids (`tag`, `move_to`) and scene ids (`scene`) take no
references: a `$` string there is not a tag or scene id and fails the batch's parsing.

Errors:

- A malformed reference (a string in one of those fields that starts with `$` but does not match
  the pattern), a kind that does not fit the field, a `*` in `face`, `id`, `material` or `definition`, or a
  reference to the command itself or a later one rejects the whole batch before anything runs:
  `-32602` with the message `invalid params: commands[<index>]: <reason>`. A command that is not a
  valid command at all is also `-32602`, its message prefixed `commands[<index>]: `.
- A reference to a command at or before a `new` or `open` of the same batch is refused the same
  way before anything runs (`-32602`), since ids start over in the new document.
- A reference past what the command made (`"$0:f1"` when command 0 made one face) fails the
  referring command at its turn with `invalid_input`: `$0:f1: command 0 made 1 face`. The batch
  stops there as for any failed command (`-32001`): an atomic batch is rolled back, a non-atomic
  one keeps the commands before it.
- A reference to an id that an earlier command of the batch erased stands for that id all the
  same; the command fails as it would with the id written out (`not_found`).

`execute` does not take references: there one is not a valid id (`-32602`).

### Curves

The edges of a circle are tagged as one curve: each edge's `entity` result (and its entry in
`entities`) has `"curve": "c12"`, the same for every edge of the curve and absent for plain
edges. A curve is not an entity: `c` ids cannot be selected or erased. `{"type": "curve", "id":
"e3"}` takes any edge of a curve and returns:

```
{"type": "curve", "id": "c12", "kind": "circle", "edges": ["e3", ...], "vertices": ["v1", ...],
 "segments": 24, "length": 6.27, "center": [0,0,0], "normal": [0,0,1], "radius": 1.0, "angle": 360.0}
```

- `kind` is `circle` (a closed chain), `arc` (an open one), `polygon` (a closed chain of polygon
  sides whose corners lie on one circle, which does not make it regular: a polygon scaled along
  one axis is a `curve`, but one whose corners stay on a circle stays a `polygon`; `radius` is the
  corner radius and `segments` the number of sides) or `curve` (its corners
  no longer lie on one circle, after some of its vertices were moved, or a polygon opened up by
  erasing a side; `center`, `normal`, `radius` and `angle` are absent when the corners are off a
  circle).
- The shape is worked out from where the edges are now, so it follows moves, rotations and copies.
  `normal` points up (+z; on a vertical plane +y, else +x); `edges` and `vertices` run
  counter-clockwise around it, a circle starting at its oldest vertex (the first corner drawn).
  `angle` is in degrees; `segments` counts straight segments, so an edge another line split in two
  still counts once; `length` is the total edge length.
- Edges split by other geometry stay on the curve, and heal back into one edge when the
  splitting line is erased. Erasing edges of a curve leaves the rest a curve (a circle missing an
  edge is an arc); if what is left falls apart, the oldest piece keeps the id and the others become
  curves of their own; a single edge left over is a plain edge again.
- Copies, groups and exploded geometry get new curve ids, and stay polygons when they were. A
  curve id is unique in the whole model.
- A pulled polygon keeps hard sides (no soft edges), and its moved copy on top is a polygon too.
- A command's `changes.modified` lists edges whose curve or soft flag changed.
- In the window, clicking an edge of a curve with the Select tool selects the whole curve, and
  Entity Info then shows it as a circle, arc, polygon or curve with its radius, angle, segments
  (sides for a polygon) and length. A box selects only the edges it covers.

Soft edges (`"soft": true`, absent when false) are hidden edges between faces that form one
smooth surface, such as the side of a pulled circle (see `push_pull`). The window does not
draw them, and clicking or snapping ignores them (a click there picks the face behind; their ends
still snap as endpoints); box selection and commands treat them like other edges. A soft edge
always bounds at least two faces: one left bounding fewer (its neighbour face erased) becomes a
plain edge again. Pieces of a split soft edge stay soft, and copies, groups and files keep the flag.

| Command | Effect |
|---|---|
| `{"type": "soften_edges", "ids": ["e12", "f8"], "soft": true}` | Makes the edges in `ids`, and the edges around the faces in `ids`, soft (`"soft": false`: hard again), as one undo step. Only an edge between exactly two faces can be soft; others are left as they are. Vertex, instance and guide ids are `invalid_input` (naming them), as is an empty list; unknown ids are `not_found`. `soft` is required. |
| `{"type": "soften_by_angle", "ids": [], "max_angle_deg": 20}` | Softens each edge between two faces whose normals differ by at most `max_angle_deg` (0 to 180; faces oriented against each other are compared as if turned the same way), as one undo step: among the edges in `ids` and around the faces in `ids`, or every edge of the open context when `ids` is empty or absent. Other edges, soft or hard, stay as they are. |

Both report `"soften": {"changed": n, "skipped": m}` in their outcome: `changed` edges whose flag
changed (also listed in `changes.modified`), `skipped` edges left hard (not between exactly two
faces, or for `soften_by_angle` meeting at more than the angle). Edges already as asked are in
neither count. A command that changes nothing makes no undo step.

Faces are shaded smoothly across soft edges: at each corner a face takes the area-weighted
average of the normals of the faces around that corner it reaches through soft edges (a face
oriented against its neighbour counted turned over), so a softened lathe or dome reads as one
surface. glTF export writes these corner normals; OBJ and STL keep one normal per face.

In the window, Edit > Soften Edges... opens a small window with an angle (0 to 180 degrees,
20 to start with): Soften runs `soften_by_angle` with it and Harden runs `soften_edges` with
`"soft": false`, both on the selected edges and faces, or on the whole open context when nothing
is selected; the status line says how many edges changed. With only edges selected, Entity Info
shows a Soft checkbox that softens or hardens them.

### Hide and unhide

A hidden edge, face, instance or guide has `"hidden": true` in `entities` and `entity` (absent
when false). The flag is the entity's own, apart from its tag (see Tags): hiding a face hides
only the face (its edges stay shown), hiding an edge never changes the faces it bounds, and
hiding an instance hides everything placed through it, at any depth. Component instances each
have their own flag; geometry inside a definition is shared by every instance of it. Copies,
clipboard and library items, groups made from or exploded into hidden entities (an exploded
hidden instance leaves its contents hidden), split edges and `.skr` files keep the flag.

A hidden entity is treated as one on a hidden tag (see Tags): not drawn (nor in `render_view`),
not met by `pick`, not selected by a `select` box, not snapped to by `infer` or the tools, not
hiding what is behind it, left out of `zoom_extents`, the hidden-line drawing, SVG, PDF and print,
`intersect_faces` (as source or target), OBJ, STL and glTF exports, and not a part in the phone
viewer; a hidden edge still bounds its faces. Hidden Geometry (`set_hidden_geometry`) draws them
ghosted and lets picking and box select reach them, but never snapping, and hidden tags stay
hidden. A hidden face inside a definition is hidden in
every instance of it, and a hidden instance hides its whole subtree at any depth.

| Command | Effect |
|---|---|
| `{"type": "hide", "ids": ["f8", "e3"]}` | Hides the edges, faces, instances and guides in `ids` (of the open context), as one undo step labelled `hide`; ones already hidden are ignored. Hidden entities leave the selection. Vertex ids are `invalid_input`, unknown ids `not_found`; dimensions, notes and section planes are left alone. The outcome's `changed` lists the ids hidden (`[]`, with no undo step, when there was nothing to hide). |
| `{"type": "unhide", "ids": ["f8"]}` | Shows the listed entities, as one undo step labelled `unhide`; `changed` lists the ids shown. |
| `{"type": "unhide_last"}` | Shows what the most recent `hide` of this session hid that is in the open context and still hidden (so nothing after an undo of that hide, or from another context), as one undo step labelled `unhide_last`. The memory of the last hide is not saved and `new` and `open` clear it. |
| `{"type": "unhide_all"}` | Shows every hidden entity of the open context (not those inside its instances), as one undo step labelled `unhide_all`. |

Doing nothing is a success with an empty `changed` and no undo step. Unless Hidden Geometry is
on (`set_hidden_geometry`), which draws hidden entities ghosted and lets them be picked and
selected, a hidden entity is not
selected: `select` skips hidden ids, and a `hide` or an undo that hides a selected entity drops
it from the selection. In the `outliner`, a node's `hidden` is true for a hidden instance as well as one on a hidden
tag (or under one), and `own_hidden` (absent when false) is true when the instance's own flag
is set.

A SketchUp file written by `export` keeps the flag on edges, faces, instances and guides; an
imported SketchUp file's hidden edges, faces, groups, components and guides come in hidden
(a hidden edge is no longer made soft; soft and smooth edges still are).

### Reverse and orient faces

| Command | Effect |
|---|---|
| `{"type": "reverse_faces", "ids": ["f8", "f9"]}` | Turns each face in `ids` over, as one undo step labelled `reverse_faces`. Faces keep their ids and each side keeps its material and texture look (front and back swap with the normal). Ids that are not faces are ignored; a face the open context does not hold is `not_found`. The outcome's `changed` lists the faces turned (`[]`, with no undo step, when there were none). |
| `{"type": "orient_faces", "id": "f8", "ids": []}` | Makes the faces connected to face `id` agree with it, as one undo step labelled `orient_faces`. The walk is breadth first from `id` (neighbours in the order of each face's loops) across edges used by exactly two faces: a neighbour that runs the shared edge the same way round is turned over, so the two run it in opposite directions. An edge used by three or more faces is not crossed. With `ids` not empty only those faces (and `id`) are walked; empty or absent walks every connected face. A surface that cannot be oriented (a Moebius strip) keeps the first orientation reached. `id` and `ids` must be faces (else `invalid_input`) of the open context (else `not_found`). |

`orient_faces` reports `"orient": {"flipped": ["f9"], "conflicts": 0}`: the faces turned over,
in id order, and the shared edges whose faces were left in disagreement. Nothing to turn is a
success with no undo step. Imported OBJ and STL meshes may keep T junctions (one face's corner on another's side, so they
share no edge), where the walk stops. Both commands take batch references.

### Intersect Faces

| Command | Effect |
|---|---|
| `{"type": "intersect_faces", "ids": ["f7", "i4"], "with": "model"}` | Draws the edges where the faces in `ids` cross other faces, as one undo step labelled `intersect_faces`. `ids` are faces and instances of the open context (faces of a group or component count, through its placement); when empty or absent the selection is used. `with` is `model` (default: every shown face of the model, in groups and components at any depth, hidden tags and hidden instances left out, section cuts ignored), `selection` (the given faces and instances against each other only) or `context` (the faces of the open context's own mesh only, not those in groups). Nothing to intersect (no faces or instances) is `invalid_input` ("Select the faces or groups to intersect"); vertex ids are `invalid_input` and unknown ids `not_found`. |

New edges always go into the open context, in its coordinates, and split the faces of the open
context they cross as drawing an edge does. Groups and components are never modified, even when
their faces are the ones crossed: open one to cut it. Coplanar faces give the boundary of each
that lies inside the other; faces that only touch, and edges that already exist, add nothing.
At most 200,000 face pairs (after a bounding box test) and 100,000 segments are handled at once,
beyond that the command is `invalid_input` ("Too many faces cross to intersect at once") and
changes nothing. A selected group or component is crossed with the rest of the model, never
with its own faces or the parts nested inside it. More than 100,000 placed groups and components
(`model` scope), or nesting past the import depth, is `invalid_input` ("Too many groups and
components to intersect at once", "Groups and components are nested too deeply to intersect"); so
is an open group whose placement flattens space ("The open group is flat and cannot be
intersected inside").

The outcome reports `"intersect": {"added_edges": [ids], "split_faces": [ids], "created_faces": [ids]}`:
the edges made (the halves of an edge it cut included), the faces that existed before and were
split (they no longer exist), and the faces the splits made. All can be batch references
(`$0:e*`, `$0:f*` name the made edges and faces). No crossings is a success with empty lists and
no undo step.

### Guides

Guides are construction geometry, as drawn by the Tape Measure tool: guide lines (infinite) and
guide points. Tools snap to them; they never merge with edges, bound no faces and are only
exported to SketchUp files. They belong to the open context they are added in (the root, or a group or component
definition) and are saved with the model.

| Command | Effect |
|---|---|
| `{"type": "add_guide_line", "point": [0, -1, 0], "direction": [1, 0, 0]}` | A guide line through `point` along `direction` (any finite, non-zero length, however long or short; stored as a unit vector). One undo step. |
| `{"type": "add_guide_point", "position": [1, 1, 0]}` | A guide point. One undo step. |
| `{"type": "delete_guides"}` | Erases every guide of the open context as one undo step; with none there it changes nothing (no error, no undo step). |

`erase`, `select`, `move`, `rotate` and `copy` take guide ids like any other (a moved or turned
guide keeps its id and is reported as `modified`). A guide's `entity` result, and each entry of
the `guides` list in `entities`, is `{"id": "g9", "kind": "line", "point": [...], "direction":
[...]}` (`kind` `point` has no `direction`). `stats` counts no guides.

### Dimensions and notes

Dimensions and notes are annotations, drawn over the model: they bound no faces and no export
writes them (see Export). Each lives in a mesh like a guide (the root
or a definition) and is selected, erased, moved, turned, scaled, copied, tagged, grouped,
exploded and copied to the clipboard like a guide.

| Command | Effect |
|---|---|
| `{"type": "add_dimension", "a": "v3", "b": [2, 0, 0], "offset": [0, -1, 0]}` | A linear dimension measuring the distance from `a` to `b`. Each end is a vertex id of the open context, which the end follows, or a point. The dimension line is drawn from `a` to `b` moved by `offset`, of which only the part perpendicular to a-b is kept. Ends that coincide are `invalid_input` ("the two ends coincide"); an id that is not a vertex is `invalid_input` and a missing vertex `not_found`, naming it. The ends and the line moved by the offset must be within the coordinate limit. |
| `{"type": "add_edge_dimension", "edge": "e4", "offset": [0, -1, 0]}` | A linear dimension between the edge's two vertices. |
| `{"type": "add_radial_dimension", "edge": "e7", "diameter": true, "leader": [1, 1, 0]}` | A radial dimension on the circle or arc `edge` is part of (any of its edges): its radius, or with `"diameter": true` its diameter, labelled at `leader` from the center. An edge on no curve, or on a curve that is not a circle or an arc (a polygon, a bent curve), is `invalid_input`. |
| `{"type": "add_note", "anchor": "v3", "position": [3, 2, 1], "text": "Door"}` | A leader note: `text` at `position` with a line to `anchor` (a vertex id or a point, as for dimensions). Without `text` it shows the anchored point's coordinates in root coordinates and the display units, "(2 m, 1 m, 0 m)". Text has 1 to 1,024 characters, is not blank and has at most 64 lines (`invalid_input`). |
| `{"type": "add_screen_note", "at": [0.1, 0.9], "text": "Plan A"}` | A note fixed to the view: `at` is fractions (0 to 1) of the view's width and height from its top left corner. Only the root holds screen notes: one added while an instance is open is `invalid_input`, and grouping leaves them in the root. |
| `{"type": "change_note", "note": "n6", "text": "Exit"}` | Changes a note's text; absent or null `text` gives a leader note its automatic text again, and is `invalid_input` for a screen note. The same text changes nothing (no undo step). |
| `{"type": "set_units", "length": "feet_inches", "precision": 4}` | Sets the model's display units, which dimension and note labels are written in; absent fields stay (one of them is needed). `length` is `meters` (the default), `centimeters`, `millimeters`, `inches` or `feet_inches`; `precision` (default 3) is the decimals kept, 0 to 6, or for `feet_inches` the finest fraction of an inch, 1/2^precision (0: whole inches, 4: 1/16, 6: 1/64). Labels round to it and drop trailing zeros: "1.5 m", "12.35 cm", `12.5"`, `5' 3 1/2"` (a value that rounds up to 12 inches carries to a foot: `1'`). Saved with the model; the same units again change nothing. The outcome, and the `changed` notification and `events` entry of the step (and of its undo or redo), carry `units`, the display units now, since `changes` lists no entity for them. The Measurements box and Entity Info's lengths keep meters (Entity Info shows a dimension's or note's label text as drawn). |

Each is one undo step. A dimension's `entity` result (and its entry in `annotations`) is
`{"type": "dimension", "id": "r5", "kind": "linear", "a": {"point": [0,0,0], "vertex": "v3"},
"b": {"point": [2,0,0]}, "offset": [0,-1,0], "line": [[0,-1,0],[2,-1,0]], "value": 2.0, "label":
"2 m", "tag": null}`: each end where it is now, with `vertex` while it follows one; `value` is
the length in meters. A radial one has `kind` `radial`, `curve` ("c12", while it follows the
curve), `center`, `normal`, `radius`, `diameter` (when true), `leader`, and a label "R 1.5 m" or
"Ø 3 m". A note's is `{"type": "note", "id": "n6", "kind": "leader", "anchor": {...},
"position": [...], "text": "Door", "label": "Door", "tag": null}` (`text` absent for automatic
text) or, for a screen note, `kind` `screen` with `at` and `text`.

Annotations follow the geometry they are anchored to. When an edit removes an anchor vertex
(erase, a weld, a heal, a redraw), that end becomes a fixed point where the vertex last was, and
a radial dimension whose curve is gone or no longer a circle or an arc keeps the circle it last
had, in the same undo step, so undo attaches them again. A dimension whose two ends an edit
brings together is removed in that step. Annotations inside a group anchor to the group's
vertices and are in its coordinates, like everything else there: queries give anchor points,
positions and circles in the open context's coordinates, and a dimension's `value` and `label`
measure it there (in a group scaled by 2, half what is drawn), while a leader note's automatic
text writes its point in root coordinates. The window's labels and Entity Info measure a
dimension as drawn, placed in root coordinates (the scaled length).

How they are drawn: a linear dimension has extension lines from its ends to the dimension line,
running 4 points past it, and arrows 10 points long at the line's ends; a radial dimension a
line from the center (for a diameter, from the far side of the circle) through the circle to its
label, with an arrow where it meets the circle; a leader note a line from its anchor to its
text, with an arrow at the anchor. Their labels (the dimension's length at the middle of its
line, a note's text) are written by the window over the view, and left out when their point is
out of view or behind the eye; a screen note is only a label. `render_view` draws the lines but no labels. In
the window, clicking a dimension's or note's lines or label selects it (or the group holding
it), and `editor.hovered` names it under the cursor.

In the window the Dimension and Text tools (Tools menu and rail; no keys) make them; through
`tool_input` they are `set_tool` `dimension` and `text` (see Driving the tools). Double-clicking
a note with Select opens the window's note sheet to change its text (`change_note` here). The
display units are set in the window in Settings, Model tab (`set_units` here).

Files keep them (schema 12, see Files). A SketchUp file's dimensions and text are not imported.
For a 2017-2020 file the import report's `warnings` counts them ("N dimensions and text labels
were not imported"); a 2021 or newer file's are not read, so not counted.

### Section planes

A section plane cuts the model open to show its inside. It is an entity of a mesh (the root or
a definition) like a guide, with the id `s8`: the plane through `point` with the unit vector
`normal`, keeping the side the normal points to (a point `p` is kept when
`(p - point) . normal >= -1e-9`, so faces lying on the plane are kept). A mesh has at most one
active plane. The active plane of the root cuts the whole model; one in a group or component cuts
only that definition's geometry and the instances inside it, never its parent or siblings, and the
cut follows the group when it is moved, turned or scaled (also non-uniformly). A hidden plane
still cuts (as in SketchUp), and a hidden tag hides the plane entity like any other.

| Command | Effect |
|---|---|
| `{"type": "add_section_plane", "point": [0,0,1], "normal": [0,0,1]}` | Adds a plane to the open context; with `"activate": false` it is not made active (default true, replacing the active plane). The normal is any non-zero length and is normalized; a zero or non-finite one, or a point beyond the coordinate limit, is `invalid_input`. The new id is in the outcome's `changes.created`. |
| `{"type": "flip_section_plane", "plane": "s8"}` | Negates the normal, keeping the point and whether the plane is active. |
| `{"type": "activate_section_plane", "plane": "s8"}` | Makes it the context's active plane (the previous one is no longer). Already active changes nothing and makes no undo step. |
| `{"type": "deactivate_section"}` | Makes no plane of the open context active. Nothing active changes nothing and makes no undo step. |
| `{"type": "set_section_display", "cuts": false}` | Sets how planes are shown; absent fields stay. `planes` draws the plane entities, `cuts` applies the active planes' cuts (off: geometry is drawn whole), `fill` fills closed cuts. All default to true. A view setting like `set_lighting`: not an undo step, no change to the revision, restored by a failed atomic batch, saved with the view. |

Each of the first four is one undo step; undo restores exactly (erasing the active plane and
undoing brings it back active). A plane that is not in the open context (another mesh's, an
unknown id or an id of another kind) is `invalid_input` naming it. `erase`, `move`, `rotate`,
`scale`, `copy`, `select` (and `select` with `all`), `make_group`, `make_component`, `explode` and
the clipboard accept `s` ids: `move`, `rotate` and `scale` carry the point and bend the normal as a
plane's normal bends (the kept side stays the kept side of the same geometry, also under a mirror),
and must keep the point within the coordinate limit (so must copies and pasted planes); `copy`
(and `rotate` with `copies`) and `paste` make inactive planes; `make_group` moves planes with the
geometry (planes alone cannot be grouped), an active one staying active in the new definition;
`explode` brings a definition's planes out into the context as inactive planes with new ids, placed
by the instance's transform. Erasing the active plane clears the context's active plane in the
same step.

`entity` of a plane (and each entry of `section_planes`) is `{"type": "section_plane", "id":
"s8", "point": [0,0,1], "normal": [0,0,1], "active": true, "tag": null}`. `section_display`
answers `{"type": "section_display", "planes": true, "cuts": true, "fill": true}`.

Drawing (the window and `render_view`): a plane is a square outline centred on its point in the
plane, the larger of 1 m and 1.2 times the diagonal of its context's shown bounds, with four
arrows, one from the middle of each side, pointing along the normal toward the kept side (about 30
points long on screen). The active plane's outline is orange (`#f08c00`, whatever the theme; a
faded orange when its mesh is outside the open group), inactive ones are in the guide colour;
selecting or hovering a plane then colours it as it does a guide (orange first, then the
selection, then the hover). Planes are drawn only with `planes` on, and not when their tag is hidden (a
hidden plane still cuts). With `cuts` on, a placement is cut by the active plane of its own mesh
and of every mesh it is placed in, each in world space: faces and edges are clipped to the kept
side (triangles that straddle a plane are split), and a plane in a group cuts only that group.
Clipped faces cast shadows only where they remain; a glowing face still lights as a whole. Cut
edges are the section loops, drawn four points wide (twice the width of selected edges) in the
edge colour. Closed cuts are filled dark grey (`#3a3a3a`) at the plane, showing the kept material
(a cut through a box with a hole fills with a hole); cuts through open meshes get cut edges but no
fill. The fill and cut edges are drawn nearer than faces lying in the plane. With `cuts` off
geometry is drawn whole and nothing is filled; with `fill` off only the cut edges are drawn.

Picking and inference ignore what an active plane cuts away: in each placement, a face is not
picked, hovered, or snapped to (nor does it hide anything behind it) where the ray meets it on the
cut-away side of any plane cutting that placement (its own mesh's active plane and its ancestors'),
a vertex on that side is not an endpoint, and an edge is not picked or snapped to
where its nearest point lies there (an edge wholly cut away is not found at all; `pick` leaves such
faces, edges and points out too). Faces on the plane are kept. With `cuts` off nothing is left out, and with `planes` off section planes are not picked, hovered, box-selected or selected by `select` with `all`.
The fill of a cut hides what lies behind it from picking too: a ray that meets a cut plane inside the fill of a placement's cut (the region its other planes keep) meets nothing past that point, so no face, edge or snap point behind the fill is found. A box select looks only at the kept part of an edge or face: a box over only the cut-away half of one that straddles a plane does not select it. At most 1,000 plane outlines per scene are drawn and picked; planes past that still cut.
A section plane is picked by its outline (like a dimension), and a box selects it by its outline
(crossing) or by all four corners (inside); a double-click selects it alone.

In the window, the Section Plane tool (Tools menu and the rail; no key) previews a square under the
cursor: on the face under it (the face's normal, turned to face the camera) or, over no face, on
the ground; an arrow key locks the normal to the red (Right), green (Left) or blue (Up) axis (the
same key again, or Escape, unlocks), and a click places the plane through the clicked point with
`add_section_plane` (active, one undo step). Right-clicking in the view with a section plane
selected opens its context menu: Flip (`flip_section_plane`) and Active Cut, checked while the
plane is active (`activate_section_plane` when checked, `deactivate_section` when cleared). The
View menu has Section Planes, Section Cuts and Section Fill checkboxes (`set_section_display`).
Entity Info for a single plane shows "Section Plane", "Active" or "Not active", its point and its
normal.

Files keep planes, each mesh's active plane and the display (schema 13, see Files). Every export
format leaves planes out (see Export), and a SketchUp file's section planes are not imported.

### 3D text

3D text is geometry, not an annotation: the letters' outlines in Inter Regular (built in), as
faces with holes in a new group.

| Command | Effect |
|---|---|
| `{"type": "add_3d_text", "text": "Hello o8%", "position": [1, 2, 0], "height": 0.5, "depth": 0.1, "filled": true}` | Draws `text` as a new group named "3D Text: " and the text's first 32 characters, placed at `position` (its instance's translation; points are those of the open context) and selected. The letters lie in the open context's XY plane, reading along +X, the first line's baseline through `position` and each further line `1.2 * height` lower (-Y); capital letters are `height` meters tall (0.001 to the coordinate limit). Each letter's contours become faces with holes (contours nested by containment, even-odd: an "o" is one face with one hole, an "8" one with two); curves become at most 12 straight edges each, within 1% of the height; letters advance by the font's widths, without kerning. `depth` (default 0: flat faces facing up) above 0 makes each letter the solid `push_pull` by `depth` makes of it: the face turned over as the base, a top `depth` higher and a side for each edge, all facing out. `filled` false (default true) draws only the letters' outline edges, no faces, and needs `depth` 0. Text has 1 to 1,024 characters, is not blank and has at most 64 lines; with a height or depth out of range, or the text reaching beyond the coordinate limit, it is `invalid_input` and changes nothing. |

It is one undo step. The outcome's `text_3d` is `{"instance": "i5", "definition": "d4", "faces": 10,
"skipped": [{"character": "中", "index": 2, "reason": "the font has no glyph for it"}]}`: the new
group's instance and definition, the faces in it, and the characters left out, in order (`index`
counts characters from 0, line breaks included): characters the font has no glyph for, and
letters whose outlines cross ("its outlines cross or touch", within one letter) or whose faces could not be built ("its faces could not be built (a crossing or degenerate outline)"). Letters that touch each other are kept as nested "Letter" groups. Spaces only move the next
letter on and are not listed. Text with no letter to draw (only spaces, or only characters the
font lacks) is `invalid_input`.

In the window, Tools > 3D Text... opens a sheet (the text, its height, depth and whether it is
filled; it opens with the values placed last, at first 0.3 m tall and 0.05 m deep), then the
letters' outline follows the cursor, the first line's baseline start on the snapped point under
it, until a click draws them there as one `add_3d_text` step (Escape cancels and goes back to the
tool used before, as Paste does; meanwhile `editor.status` names the tool "3D Text").

### Materials

A material is a named color with an opacity, and possibly a texture (an image). Faces have two sides, each painted with a material or with none
(the default material: light on the front, blue-gray on the back, as in the window); instances
can be painted too. Materials belong to the whole model, not to the open context, and are saved
with it.

| Command | Effect |
|---|---|
| `{"type": "add_material", "name": "Brick", "color": [180, 60, 40]}` | Adds a material. `color` is sRGB red, green and blue, whole numbers 0 to 255. `opacity` (optional, default 1) is a number from 0 (invisible) to 1 (opaque); anything else is `invalid_input`. Without `name` it is called `Material` (`Material 2`, ... when taken). A name must not be blank, must have at most 256 characters and must differ from every other material's (`invalid_input`). The new id is the outcome's `changes.materials`, e.g. `["m31"]`. |
| `{"type": "change_material", "material": "m31", "name": "Old brick", "color": [150, 50, 30], "opacity": 0.4}` | Renames, recolors and/or changes the opacity of a material; an absent `name`, `color` or `opacity` stays. `opacity` must be from 0 to 1 (`invalid_input`). |
| `{"type": "add_material", "name": "Tiles", "texture": {"path": "/abs/tiles.png", "width": 0.6}}` | Adds a material with a texture; see Textures below. `color` may then be left out: it is the image's average color. |
| `{"type": "change_material", "material": "m31", "texture": {"path": "/abs/oak.jpg", "width": 1.2, "height": 0.2}}` | Gives a material a texture or replaces its image; the color becomes the image's average unless `color` is given too. Without `path` it keeps the image and changes its size (`invalid_input` when the material has none). `"texture": null` removes the texture and keeps the color. |
| `{"type": "add_material", "name": "Lamp", "color": [255, 220, 160], "emission": 5}` | Adds a glowing material; see Lights below. `emission` (optional on `add_material` and `change_material`, default 0) is a number from 0 to 10 (`invalid_input` otherwise). |
| `{"type": "delete_material", "material": "m31"}` | Removes a material. Every face side and instance painted with it, anywhere in the model, goes back to the default material (and comes back with undo). |
| `{"type": "paint", "ids": ["f8", "i4"], "material": "m31", "side": "front"}` | Paints faces and instances of the open context. `side` is `front` (default: the side the face's `normal` points to), `back` or `both`; it does not matter for instances. An absent or null `material` paints with the default material. Edge and guide ids are `invalid_input` (naming them), as are vertex ids and an empty list; unknown ids and an unknown material are `not_found`. Painting what is already painted so changes nothing. |

Each is one undo step. A command that adds, removes, renames, recolors or changes the opacity,
texture or emission of a material lists it in
its `changes.materials` (also in `changed` notifications and `events`); painting reports the
faces and instances as `modified`. Unknown material ids are `not_found`.

The `materials` query always includes `opacity` (1 for an opaque material). A face side whose
material (its own, or the nearest painted instance's) has an opacity below 1 is see-through in
the window and in `render_view`: such sides are drawn after everything opaque, blended over it
back to front, and do not hide edges, guides or faces behind them. Each side of a face uses its
own material's opacity, so a face can be glass from one side and opaque from the other. Edges
of see-through faces (window frames, say) still draw, and a selected see-through face is tinted
like any other.

#### Textures

A texture is a PNG or JPEG file of at most 16 MiB and 4096 pixels on each side, read when the
command runs; the model keeps its own copy (the file is saved inside `.skr`), so the file may
change or go away later. A model holds at most 256 textures, and their images together at most
2^28 pixels (268435456, 1 GiB decoded; an image shared by materials counts once). `texture` is `{"path", "width",
"height"}`: `width` and `height` are the meters one copy of the image covers on the model, each
from 0.001 to 1000 (`invalid_input` otherwise). A new image without `width` is 1 m wide (or as
wide as the texture it replaces), and without `height` keeps the image's proportions (limited to the same 0.001 to 1000 range).
A file
that cannot be read, is not a PNG or JPEG, is too large, or would take the textures over the
total pixel limit gives the error kind `file`, naming the path; nothing changes. Opening a `.skr`
whose textures hold more than that total is a `file` error too, checked from the images' headers
before any is decoded.

A face shows its side's material's texture in place of its color, shaded like any face (the
material's opacity still applies; the image's own transparency is ignored). The image lies flat
in the face's plane, in the coordinates of the group or component holding the face: its width
runs along the face's horizontal direction (`u` = z x normal, normalized; along x on a level
face) and its height across it (`v` = normal x u, up on a wall), repeating every `width` by
`height` meters with a copy's bottom left corner at the definition's origin. Faces in one plane
continue each other's pattern, and a component carries its pattern with it when moved, turned or
scaled. A face's texture position (below) moves, turns and scales its texture on top of this.
The back side shows the image as seen from behind, not mirrored: its `u` runs the other way, so
the image's left edge is on the left for a viewer behind the face.
The material's `color` stays the color edges, the Materials tab and formats without textures
show.

The `materials` query adds `"texture": {"width": 0.6, "height": 0.3, "pixel_width": 1024,
"pixel_height": 512, "format": "png"}` (`format` `png` or `jpeg`) to a material with a texture.

#### Texture positions

| Command | Effect |
|---|---|
| `{"type": "position_texture", "ids": ["f8", "f9"], "offset": [0.2, 0], "rotation_deg": 45, "scale": [2, 2]}` | Sets the texture position of the faces in `ids`, as one undo step: `offset` [u, v] moves the texture that many meters along the face's `u` and `v` (within +/-1e6 m), `rotation_deg` turns it counter-clockwise seen from the face's front, about the moved origin (any angle; kept from 0 up to 360), or `rotate_by` turns each face's texture that many degrees further than it was (not both), and `scale` [u, v] multiplies the size one copy covers along the image's own width and height (each 0.001 to 1000). Fields left out stay as they are; `"reset": true` first clears the position (then applies any fields given). At least one field or `reset` is required. Ids other than faces are `invalid_input` (naming them), as is an empty list or a value out of range; unknown ids are `not_found`. |

The outcome reports `"positioned": n`, the faces whose position changed (also listed in
`changes.modified`); a command that changes nothing makes no undo step. A face's `entity` result
(and its entry in `entities`) has `"texture_position": {"offset": [0.2, 0.0], "rotation_deg": 45.0,
"scale": [2.0, 2.0]}`, absent when the texture lies as projected (a position back at offset 0,
rotation 0 and scale 1 is the same as none). The position belongs to the face, whatever material
its sides have, and applies to both sides.

Pieces of a face split by a new edge keep its position, as do copies, faces moved into a group or
component, and the faces of copied definitions. Explode gives the painted faces of a moved,
turned or evenly scaled instance the position that keeps their texture where the instance showed
it (a mirrored or unevenly scaled instance's is the nearest such position). A push/pull cap keeps
the pushed face's position (a sheet that turns over keeps what each side shows); the new sides
and other new faces have none. Moving or turning faces keeps the numbers, so a moved face's
pattern stays where the projection puts it.

In the window, with only faces selected, Entity Info shows the first selected face's texture
position: Offset (two fields, meters), Rotation (degrees) and Scale (two fields), each applied to
every selected face when a drag ends or a typed value is entered (the faces selected when the
edit began, even if the selection changed before the value was entered; if the fields go away
first, a dragged or arrow-stepped value still applies and text still being typed is dropped),
and Rotate 90 (turns each
selected face's texture a further quarter counter-clockwise: `rotate_by: 90`, one undo step) and Reset.

#### Lights

A material with an `emission` above 0 glows. The `materials` query always includes `emission`
(0 for a material that does not glow).

The view is lit by `day` (the default) or by `night`, a view setting `set_lighting` changes and
the `camera` query reports as `lighting`.

A glowing face side is drawn unshaded, in its material's color (or texture). Each placement of
a group or component whose own faces glow lights its surroundings as one point light, wherever
it is placed, at any depth (a lamp component inside a room inside a house gives its own light in
each placed house; the groups around it give none of their own). The root's loose glowing faces
give one light per connected group of them (glowing faces sharing an edge or a vertex), so lamps
drawn loose in two rooms light each room. A face glows with its stronger side's material (its
own, or the nearest painted instance's, as for color). A light sits at the area-weighted center
of its glowing faces, has
their color weighted by emission times area (a textured glowing material lights with its
material color, its image's average unless a color was given), and its strength is their
emission times their area in square meters (scaled with a scaled placement). It reaches 6 x sqrt(strength) meters
and falls off smoothly to nothing there; surfaces take it by the angle they face it. Lights do
not depend on the open instance. The window and `render_view` use the 128 strongest lights as
seen from the eye (strength over 1 + distance squared), and up to the strongest 64 of those
cast shadows: every face stops their light except glowing ones (a lamp's glowing shade does not hide
its own light), from both sides; shadow edges are slightly soft. Faces nearer a light than 1 mm
cast no shadow from it, and the other lights light without shadows. Shadow maps are kept while
the camera moves and drawn again only when the model's faces move or the shadowed lights
change, so the first frame after an edit is slower than the others. Where the GPU has no cube-map
arrays nothing casts shadows. By day lights add to
the headlight and ambient light; by night those are dimmed, the background is dark, and
unselected edges, the grid and the axes fade toward it; selection, guides and tool previews keep
their colors.

#### Painting

The `entity` result of a face (and its entry in `entities`) has `front_material` and
`back_material`, an instance's has `material`; each is absent for the default material.

An instance's material shows on every face side inside it, at any depth, that has no material
of its own; when instances inside each other are both painted, the one nearer the face wins.
Painting an instance does not change the faces of its definition, so other instances of a
component keep their own look. Exploding an instance paints its material onto the face sides
and inner instances that come out without one.

Paint follows the geometry: pieces of a face split by a new edge keep its materials, two faces
merged by erasing the edge between them keep the materials of one of them (the one a point
inside the merged face lies in), and copies, groups and components keep them. A
face filling an opening is unpainted. Push/pull gives the extruded faces (the cap and the
sides, also the walls and floor of a recess) the pushed face's front and back materials; a base
that turns over (a pulled flat sheet) keeps each material on the side it was on, so its old
front is now its back. A sheet pushed against its normal gives the cap and sides its back
material as their front, so the material on the side facing the pull ends up outside.

In the window, the Paint tool (`b`) paints the side of a face facing the viewer with the
current material (a click on something selected paints the whole selection's faces and
groups, a click on a group paints the group), and Cmd-click on macOS (Ctrl-click on Windows
and Linux) picks up the clicked side's material instead. The Materials tab of the dock (a grid
of swatches) chooses the current material with a click, keeping the tool, and a double-click
chooses it and switches to Paint (`set_paint_material`, then `set_tool` with `paint`); the tab
also adds, renames, recolors and deletes materials and sets their opacity (a 0-100% slider; one
change per drag) and their emission (the Glow slider, 0 to 10 in tenths; one change per drag),
all through these commands. A material with a texture
shows a small copy of its image as its swatch; for the chosen material, Texture... loads a PNG
or JPEG image from a file dialog (as wide as the texture it replaces, or 1 m, and as high as the
new image's proportions make it), Remove takes the
texture off, and the Size fields set its width and height in meters (one change per drag or
typed value).

### Tags

A tag is a named color that edges, faces, instances (groups and components), guides, dimensions
and notes can be put on, to show or hide them together; an entity is on at most one tag, and on none it is
Untagged (which is not a tag). Vertices cannot be tagged. Tags are shared by the whole model.

| Command | Effect |
|---|---|
| `{"type": "add_tag", "name": "Walls", "color": [200, 80, 60]}` | Adds a shown tag. Without `name` it is called `Tag` (`Tag 2`, ... when taken). A name must not be blank or `Untagged`, must have at most 256 characters and must differ from every other tag's (case counts, as for materials; `invalid_input`). Without `color` (sRGB, whole numbers 0 to 255) it takes the first of a fixed palette of twelve colors no tag has (once all are used, the next in turn). The new id is the outcome's `changes.tags`, e.g. `["t31"]`. |
| `{"type": "change_tag", "tag": "t31", "name": "Old walls", "color": [9, 9, 9], "visible": false}` | Renames, recolors, hides (`visible`: false) or shows a tag; absent fields stay. Hiding and showing are undo steps like any other change and mark the model modified. |
| `{"type": "delete_tag", "tag": "t31", "move_to": "t32"}` | Removes a tag. Everything on it, anywhere in the model, moves to `move_to` (absent or null: Untagged) in the same undo step, so undo restores the tag and every entity's tag. `move_to` naming the deleted tag is `invalid_input`. |
| `{"type": "set_tag", "ids": ["e4", "f8", "i12", "g20"], "tag": "t31"}` | Puts edges, faces, instances, guides, dimensions, notes and section planes of the open context on a tag (absent or null `tag`: Untagged), as one undo step. Vertex ids are `invalid_input` (naming them), as is an empty list; unknown ids are `not_found`. Entities already on it are left as they are. |

A command that adds, removes, renames, recolors, shows or hides a tag lists it in its
`changes.tags` (also in `changed` notifications and `events`); `set_tag` and `delete_tag`
report the entities they moved as `modified`. Unknown tag ids (`tag`, `move_to`) are
`not_found`.

The `entity` result of an edge, face, instance or guide (and its entry in `entities`) has
`tag`: the tag's id, or `null` when Untagged.

Tags follow the geometry: the pieces of a split edge or face keep its tag, two edges joined or
two faces merged by an erase keep one's, copies (of geometry, guides and instances) keep their
tags, and `make_group` and `make_component` keep the tags of what goes inside (the new instance is
Untagged). Exploding an instance keeps each piece's own tag; what came out Untagged (edges,
faces, instances, guides) takes the instance's tag. Push/pull's cap (the pushed face moved)
keeps the pushed face's tag, and follow_me's end caps take the profile face's, as they take its
paint; the sides and new edges they make are Untagged whatever the face is on. A copy or explode drawn over coincident existing edges puts
them on the incoming edges' tag (when those have one), as drawn geometry takes the copy's paint.

What a hidden tag is on is hidden: an edge, face or guide on a hidden tag, and a group or
component on a hidden tag with everything inside it, whatever tags its contents are on (content
inside a shown instance is hidden by its own tag). A vertex is hidden when no shown edge or face
uses it. Hidden entities are not drawn (nor in `render_view`), not met by `pick`, not selected
by a `select` box or by `select` with `all` (as SketchUp's Select All), not snapped to by `infer`
or the tools, and do not hide what is behind them; `zoom_extents` leaves them out (with
everything hidden it does nothing, as for an empty model), and so do OBJ, STL and glTF exports
(a SketchUp file export writes everything, each entity on its tag; see Export).
They are still there for everything else: `entities`, `entity`, `stats` and the `outliner`
(whose nodes say `"hidden": true`) list them, and commands given their ids act on them. The
instances the open context is in stay shown while open, whatever their tags (drawn, seen by the
tools and counted by `zoom_extents`).
Hiding a tag does not change the selection: selected entities it hides stay selected, just not
drawn (as in SketchUp).

`{"type": "set_color_by_tag", "on": true}` draws faces in their tags' colors (both sides,
opaque, without textures): a face on a tag takes that tag's color, and a face on none placed by
a tagged group or component takes the nearest such instance's tag's; other faces keep their
material's color (without its texture). Edges keep their colors, and glowing materials still
light the scene. `"on": false` goes back to materials. Like `set_lighting` it is a view
setting: not an undo step, no change to the revision, saved with the camera, put back by a
failed atomic batch, and reported as the `camera` query's `color_by_tag`. `new` and opening a
SketchUp file start with it off.

### Scenes

A scene saves a view and which tags are hidden, to go back to in one step (SketchUp's scenes).
A scene may store the view, the tag visibility, both or neither. The scenes are in an order
(their tabs' order in the window) and shared by the whole model. Importing or opening a
SketchUp file (2021 or newer) adds its scenes (see Import).

| Command | Effect |
|---|---|
| `{"type": "add_scene", "name": "Front", "camera": true, "tags": true}` | Adds a scene after the last one, as one undo step. It stores the current view (camera, projection, `lighting`, `color_by_tag`, `x_ray`, `hidden_geometry` and `shadows`) unless `camera` is false, and which tags are hidden unless `tags` is false (both default true). Without `name` it is called `Scene 1` (the lowest number no scene's name has). A name must not be blank, must have at most 256 characters and must differ from every other scene's (case counts; `invalid_input`). The new id is the outcome's `changes.scenes`, e.g. `["p40"]`, and it becomes the selected scene. A camera whose eye or target is beyond +/-1e6 m (window navigation can go that far) is not stored: `invalid_input` ("the camera is beyond +/-1000000 m: a scene cannot store it"); move the view closer first. |
| `{"type": "update_scene", "scene": "p40", "camera": true, "tags": false}` | Stores the current view and hidden tags in the scene again, as one undo step. An absent `camera` or `tags` stores that part again if the scene stores it; true starts storing it, false stops. Storing what it already stores is no step. Storing the camera beyond +/-1e6 m is refused as for `add_scene`. In the window, Update on a scene that stores neither part says so in the status and changes nothing. |
| `{"type": "change_scene", "scene": "p40", "name": "Back", "camera": false, "tags": true}` | Renames a scene (name rules as for `add_scene`) and chooses what it stores, as one undo step (none when nothing changes). Each of `name`, `camera` and `tags` is optional, but at least one is needed (`invalid_input`). `camera` or `tags` false stops storing that part; true stores it from the current view or tag visibility only if the scene does not store it yet. A part that stays stored keeps exactly what was stored (unlike `update_scene`, which stores it again). Starting to store the camera beyond +/-1e6 m is refused as for `add_scene`. |
| `{"type": "move_scene", "scene": "p40", "index": 0}` | Moves a scene to `index` in the order (0: first), as one undo step. An index past the last scene is `invalid_input`. |
| `{"type": "delete_scene", "scene": "p40"}` | Removes a scene, as one undo step; undo puts it back at its place with what it stored. |
| `{"type": "go_to_scene", "scene": "p40"}` | Goes to a scene, straight (no transition). A stored view sets the camera, projection, lighting, color by tag, X-ray, Hidden Geometry and the shadow settings (going to a scene with Hidden Geometry off drops hidden entities from the selection), a view change like `set_camera` (not an undo step, no change to the revision, put back by a failed atomic batch); the camera comes back exactly as stored, so an `update_scene` right after stores no change. Stored tag visibility shows and hides tags to match (tags the scene does not hide are shown), as one undo step labeled `go_to_scene`, and only when that changes a tag; the outcome's `changes.tags` lists them. A part the scene does not store is left as it is. It becomes the selected scene. |

Every scene command but `go_to_scene` is an undo step (when it changes something), changes the
revision, marks the model modified and lists the scene in its `changes.scenes` (also in
`changed` notifications and `events`). Unknown scene ids are `not_found`. Deleting a tag drops
it from every scene that hides it, in the same undo step (those scenes are listed in
`changes.scenes`), so undo puts it back in each.

The selected scene is the one last added or gone to (the `scenes` query's `selected`); it is
not saved, and `new` and `open` clear it. A selected scene that stops existing (its `add_scene`
undone, or it deleted) is not selected while it does not exist, and is selected again if it
comes back (by redo or undo).

A view a scene stores is the camera in model (root) coordinates, as `set_camera` takes it,
whatever instance is open; `go_to_scene` leaves the open instance open.

`{"type": "scenes"}` gives, for example,
`{"scenes": [{"id": "p40", "name": "Front", "camera": true, "tags": true, "selected": true}], "revision": 12}`.
`{"type": "scene", "scene": "p40"}` gives the stored view in the `camera` query's terms (without
`view_height`) and the hidden tag ids:
`{"id": "p40", "name": "Front", "view": {"eye": [-8, -12, 7], "target": [0, 0, 0], "up": [0, 0, 1], "fov_y_deg": 35, "projection": "perspective", "lighting": "day", "color_by_tag": false, "x_ray": false, "hidden_geometry": false, "shadows": {"on": false, "date": "2026-11-08", "time": "13:30", "light": 80, "dark": 45, "on_ground": true}}, "hidden_tags": ["t31"], "revision": 12}`.

### Groups and components

Commands and queries work on the open context: the root at first. `{"type": "open_instance",
"id": "i4"}` opens an instance of the open context for editing, and `{"type": "close_instance"}`
goes back out one level. Neither is an undo step or changes the revision; both clear the
selection. `open_instance` with an id that is not an instance, and `close_instance` at the root,
are `invalid_input`. `{"type": "open_path", "path": ["i4", "i9"]}` opens a nested instance
directly, from wherever the open context is: `path` lists instances from the root, each placed in
the definition of the one before (`i9` inside `i4`'s definition), and becomes the open context;
`[]` goes back to the root. It clears the selection, except when `path` is already the open
context (then nothing changes). An id in `path` that is not an instance is `invalid_input`, and
one that is not placed where the path puts it is `not_found`, both naming it. While an instance
is open:

- `entities`, `entity`, `stats` and `selection` see only the definition's entities: `entity` and
  commands give ids from elsewhere `not_found`, and `entities` leaves them out;
- points and offsets in commands, and positions in results, are in the definition's own
  coordinates (`instance.transform` maps them into the coordinates around it);
- edits change the definition, so every instance of a component changes with it.

An instance's `entity` result has its `definition`, `name`, `kind` (`group` or `component`),
`transform` (column-major 3x4: x axis, y axis, z axis, translation; the axes turn when the
instance is rotated, and stretch, or mirror with a negative determinant, when it is scaled) and `bounds` (the box around its turned geometry) in the open context's
coordinates. Undo and redo apply to the whole model wherever the change was made; an
open instance that stops existing is closed. Groups keep the coordinates of the context they
were made in (the instance's transform starts as the identity).

A command's `changes.created` can list ids inside a new definition (`make_group`, copying a
group): they are `not_found` from the open context until that instance is opened.

`{"type": "outliner"}` returns the tree of every placed group and component, whatever instance is
open:

```
{"type": "outliner", "revision": 7, "instances": [
  {"id": "i55", "definition": "d54", "kind": "component", "name": "Box", "open": true,
   "children": [{"id": "i27", "definition": "d26", "kind": "group", "name": "", "selected": true,
                 "children": []}]},
  {"id": "i56", "definition": "d54", "kind": "component", "name": "Box", "material": "m57",
   "children": [{"id": "i27", "definition": "d26", "kind": "group", "name": "", "children": []}]}]}
```

- `instances` are the root's instances in id order; each node's `children` are the instances its
  definition places, in id order, at any depth. A component's contents appear under each of its
  instances with the same ids, so a node is named by its path: the ids from the root down to it
  (`["i55", "i27"]` above, with `i55` open and `i27` selected in it), which is what `open_path` takes.
- `name` is the definition's name (empty for an unnamed group; the window shows it as `Group`);
  `material` is the instance's own (absent for none).
- `hidden` (absent when false) marks the nodes that are hidden: their own hide flag or tag, or
  that of an instance above them (see Tags and Hide and unhide). Hidden nodes are still listed.
  `own_hidden` (absent when false) marks a node whose own hide flag is set.
- `open` (absent when false) marks the nodes on the open context's path: the open instance and
  the ones it is inside of. `selected` (absent when false) marks selected instances of the open
  context; the same ids under another placement are not marked.
- `revision` covers the tree's structure (names, materials, children). The `open` and
  `selected` marks follow the open context and the selection, which change no revision and send
  no `changed` notification: query again after `open_path`, `open_instance`, `close_instance` or
  `select`.
- To select a node elsewhere in the tree, open its parent's path, then select it:
  `open_path` with the path minus its last id, then `select` (a non-atomic `batch` of the two
  works).
- The output grows with nesting, since each placement repeats its definition's whole subtree,
  and each nesting level adds about two levels of JSON depth (a node and its `children` array).
  `{"type": "outliner", "depth": n}` stops n levels below the root's instances (0: the root's
  instances only); a node whose children were left out has `"truncated": true` (absent
  otherwise) and empty `children`. For large or deeply nested models use a small `depth` or
  `definitions` to see what exists, then `open_path` and `entities` to look inside one
  placement at a time.

In the window, the Outliner, in the dock under the Entity Info and Materials tabs, shows this
tree (components as `<Name>`, groups by name or as `Group`, selected rows highlighted, open ones
bold, hidden ones greyed, nodes on the open path expanded whenever the open context changes) and sends these same
commands: a click selects the node, with `open_path` first when it is in another context;
Shift-click, or Cmd-click on macOS (Ctrl-click on Windows and Linux), adds it to or removes it
from the selection when it is in the open context (`select` with mode `toggle`); a double-click
opens it (`open_path` with its whole path); the Model row at the top goes back to the root
(`open_path` with `[]`; nothing when already there).

`{"type": "rename_definition", "definition": "d5", "name": "Chair"}` renames a definition (one undo
step; a component's name must not be empty; an unknown definition is `not_found`; `definition` may be a `$N:dI` reference to a definition an earlier `make_unique` of the batch made).
`{"type": "purge_definitions"}` removes every definition no instance places, such as a component
whose last instance was erased, and what only it placed (one undo step). The outcome's `purged`
counts the definitions removed, nested ones included (0 when nothing was unused).

#### Make Unique and Flip Along

`{"type": "make_unique", "ids": ["i4", "i9"]}` gives the component instances in `ids` (absent:
the selection; other entities are ignored; present and empty: nothing to do, so a batch `*`
reference that expands to nothing never reaches the selection) a definition of their own when
their definition is placed elsewhere too, as one undo step. Only definitions the root reaches
count, so an unplaced one does not make a definition shared. All the listed instances of one
definition share one new copy, named like the old one with `#N` in place of any trailing
`#<digits>` (`Chair`, `Chair#2`, `Chair#3`; N is the smallest of 2 or more no definition's name
is). The copy is deep for groups inside it and shares the components inside it. Groups, sole
instances and definitions all of whose instances are listed stay as they are. The outcome's
`definitions` lists the new definitions in the id order of the old ones (empty, with no undo
step, when there was nothing to do); a batch refers to them as `$N:d0`. An unknown id is
`not_found`.

`{"type": "flip", "ids": ["f8", "i4"], "axis": "red"}` mirrors `ids` (absent: the selection; present and empty: nothing to do, with an empty `changed` and no undo step, so a batch `*` reference that expands to nothing is harmless) as
one undo step, `axis` being `red`, `green` or `blue` (x, y, z). `frame` `axes` (the default)
mirrors about the plane through the center of the box around the selection, measured in the
current drawing axes (the open context's own axes), normal to that axis; faces keep facing out,
and instances become mirrored (negative determinant). `frame` `own` takes exactly one instance
and mirrors it about its own center along its own axis (the axis of its definition, also when
the instance is rotated, scaled or sheared), leaving it where it was. Errors: absent `ids`
with nothing selected and `own` with anything but one instance are
`invalid_input`, unknown ids `not_found`. The outcome's `changed` lists the flipped ids.

### Axes

`{"type": "set_axes", "origin": [1, 2, 0], "x_axis": [1, 1, 0], "y_axis": [-1, 1, 0]}` moves the
axes of the open context as one undo step. `x_axis` is normalized, `y_axis` is made
perpendicular to it (Gram-Schmidt) and normalized, `z` is `x` cross `y`; a zero or non-finite
vector, or parallel ones, is `invalid_input`, and so is an origin beyond the coordinate limit.

At the root these are the model axes: the `camera` query's `axes` reports them, `flip` follows
them, and the file keeps them (schema 15; a model with the world axes is written as before).
Scenes do not store axes, and `.skp` import and export ignore them.

Inside an open group or component they become that definition's own origin and axes, given in
the open context's coordinates: its geometry, guides, dimensions, notes, section planes and
nested instances move by the new frame's inverse, and every instance of it, in every context,
is placed by its transform times the frame, so nothing moves in the model and the open context
stays open; a frame that would put anything of it, or an instance placing it, beyond the coordinate limit is `invalid_input` and changes nothing. `{"type": "reset_axes"}` puts the model axes back to the world axes (one undo step;
nothing to change makes none); inside a context it does nothing.

The current drawing axes (the model axes at the root; inside an open group or component its own
axes, the identity in the context's coordinates) drive the tools and the view: inference offers
the red, green and blue directions and the origin snap at the axes' origin, the arrow-key and
Shift locks and the Rotate and Section Plane tools' axis lock follow them, a rectangle's sides
run along them on axis planes (`add_rectangle` too, and its `normal`-less form needs the
corners to differ in two coordinates along them), the tools' horizontal plane is their red and
green plane, `standard_view` and the orientation gizmo look along them (`top` from the blue
axis with the green axis up; inside a context the context's axes as placed in the world), and
the drawn axis lines run along them from their origin (the ground grid stays on world z = 0,
and so does the ground inference: a point inferred on the ground lies on world z = 0 while the
horizontal plane tools draw in follows the axes). Inside a placement mirrored in x or y the
context's frame is left-handed: blue stays the definition's z direction. With a tilted blue axis
the non-vertical standard views give the camera an `up` that is not +Z; `set_camera` and a
scene restore use +Z up, and the first orbit levels the roll.
With the world axes everything is as before. `add_circle`, `add_polygon` and `add_arc` keep
their documented default normal `[0,0,1]` and u axis in the world.

In the window, the Axes tool (Tools menu and the rail; key Y) takes three clicks, each with
inference and the arrow-key locks: the new origin, a point on the red axis, and a point on the
green side of the red-green plane. A preview shows the axes as they will be. The third click
runs one `set_axes` (in the open context's coordinates, so inside a group it re-origins that
definition) and says "Axes moved". A second click on the origin, or a third in line with red,
gives no direction and is ignored with a message; Escape starts again. Group > Make Unique
(enabled with a selected component instance whose definition is placed more often than it is
selected) runs `make_unique` and says "Made 2 components unique". Group > Flip Along
has Red, Green and Blue (the selection, `flip` with `frame` `axes`; "Flipped along red") and
Component's Red, Green and Blue (exactly one instance selected, `frame` `own`). The viewport's
right-click menu has Make Unique, the same Flip Along entries and, at the root with moved axes
(also with nothing selected), Reset Axes (`reset_axes`, "Axes reset"). Entity Info of a
component instance shows its definition's instance count after "1 component" ("3 instances";
`definitions` has it as `instances`). The viewport menu opens for any selection (the entries it
shows depend on what is selected; a lone section plane keeps its own) and, with moved axes at the
root, with nothing selected. Make Unique says "Made N components unique", N the selected
instances that got a definition of their own. The phone viewer has none of these.

### Camera

The camera belongs to the document (it is saved with the model) and is always in root
coordinates, whatever instance is open. Moving it does not mark the model modified (the `file`
query's `modified` stays as it was), so a view moved since the last save is lost on `new`,
`open` or `quit` without `discard`; `save` keeps it. Camera commands are not undo steps, return empty
changes, do not change the revision and send no `changed` notification; inside an atomic batch a
failure puts the camera back. Each leaves the eye and target within the coordinate limit, or
fails with `invalid_input` and leaves the camera as it was.

`{"type": "camera"}` returns:

```
{"type": "camera", "eye": [-8,-12,7], "target": [0,0,0], "up": [0,0,1], "fov_y_deg": 35.0,
 "projection": "perspective", "view_height": 10.11, "lighting": "day", "color_by_tag": false, "x_ray": false, "hidden_geometry": false,
 "shadows": {"on": false, "date": "2026-11-08", "time": "13:30", "light": 80, "dark": 45, "on_ground": true},
 "axes": {"origin": [0,0,0], "x": [1,0,0], "y": [0,1,0], "z": [0,0,1]}}
```

- `up` is the direction pointing up the screen: always `[0,0,1]`, except in a view straight down
  or up, where it is horizontal (`[0,1,0]` in the Top view).
- `projection` is `perspective` or `parallel` (orthographic). In a parallel view sizes do not
  shrink with distance, and the view is `view_height` meters tall (its width follows the
  window's shape). `view_height` is 2 x distance x tan(fov / 2): the height of the view at the
  target, which switching projection keeps, so the zoom of a parallel view is set by how far
  the eye is from the target, and zooming moves the eye. A parallel view sees 100 eye-to-target
  distances in front of and behind the target, including geometry behind the eye.
- `lighting` is `day` or `night` (see Lights).
- `color_by_tag` is whether faces are drawn in their tags' colors (`set_color_by_tag`; see Tags).
- `x_ray` is whether every shown face is drawn as glass (`set_x_ray`).
- `hidden_geometry` is whether entities hidden by their own flag are drawn ghosted and can be
  picked (`set_hidden_geometry`; see Hide and unhide).
- `shadows` is the sun's shadow settings (`set_shadows`; see Location and sun shadows below).
- `axes` is the model axes (see Axes below): `origin` and the unit vectors `x` (red), `y` (green)
  and `z` (blue), the world axes until `set_axes` moves them.

| Command | Effect |
|---|---|
| `{"type": "set_camera", "eye": [10,-10,8], "target": [0,0,0]}` | Places the camera. Optional `fov_y_deg` (1 to 170) and `projection` keep their values when absent. A view straight down or up takes the screen's up from the horizontal part of `up` (default `[0,1,0]`; any length, however large or small; a vertical `up` is `invalid_input`); any other view has `[0,0,1]` up and ignores `up`. `eye` equal to `target` is `invalid_input`. To restore a view, pass back the `eye`, `target`, `fov_y_deg`, `up` and `projection` the `camera` query returned. |
| `{"type": "orbit", "yaw": 30, "pitch": 10}` | Turns the eye about the target, or about `about` (a root point, as a middle-button drag turns the view about the point under the cursor): `yaw` degrees about the vertical (counter-clockwise seen from above), then `pitch` degrees raising the eye (negative lowers it); both default to 0. The view stops 0.01 rad (0.57 degrees) short of straight down or up. From a Top or Bottom view, `yaw` spins the view about the vertical and only a pitch that moves the eye toward the horizon moves it (from Top a negative pitch, from Bottom a positive one). |
| `{"type": "pan", "right": 2, "up": 0.5}` | Moves the eye and target together, meters along the screen's right and up directions (each defaults to 0). |
| `{"type": "zoom", "factor": 2}` | Moves the eye toward the target to `1/factor` of its distance (above 1 zooms in, below 1 out), never nearer than 0.01 m. `factor` must be positive. In a parallel view `view_height` shrinks the same way. With `"about": [x, y, z]` (a root point) it zooms toward that point as the scroll wheel zooms toward the point under the cursor: the point stays on the same pixel and, in perspective with the point ahead of the eye and above the 0.01 m floor, the eye comes to `1/factor` of its distance from it (in a parallel view `view_height` shrinks by the factor instead; at the floor or with the point behind the eye the eye moves on past the point, as the wheel does); in perspective the target moves to the point's depth, so later orbits and zooms work there. |
| `{"type": "zoom_extents"}` | Keeps the view direction and fits the whole model in the view: every vertex and placed instance of the root, in root coordinates, whatever instance is open (guides do not count, nor does what hidden tags hide). Perspective fits the sphere around the model's bounding box; parallel fits the box itself with 5% to spare. With nothing to fit, the camera stays (no error). |
| `{"type": "standard_view", "view": "top"}` | Looks at the target from a standard direction, from the same distance (directions as given for the world axes; with moved drawing axes they turn with them, see Axes): `top` (from +Z, `up` +Y), `bottom` (from -Z, `up` +Y), `front` (from -Y, looking along +Y), `back` (from +Y), `left` (from -X), `right` (from +X), `iso` (from the front left above, along (1,1,-1)). The projection does not change. |
| `{"type": "set_projection", "projection": "parallel"}` | Switches between `perspective` and `parallel`, keeping the eye, target and `view_height`. |
| `{"type": "look_around", "yaw": 30, "pitch": -10}` | Turns the view about the eye (not the target): `yaw` degrees about the vertical, counter-clockwise seen from above, then `pitch` degrees raising the view (negative lowers it); each defaults to 0 and must be finite. The view's pitch stays within -89 and 89 degrees, so the screen's up is always the world's up, even when the view was straight down or up. The target keeps its distance from the eye. Positive `pitch` looks up, as in Fly Mode (unlike `orbit`, whose positive pitch raises the eye); a view straight down or up turns to the 89 degree limit even with only `yaw` given. |
| `{"type": "position_camera", "point": [2,3,0], "eye_height": 1.676, "look_at": [2,10,0]}` | Stands at `point` (world coordinates, as `look_at`) with the eye `eye_height` meters above it (0.01 to 10,000, default 1.676, 5'6"), looking level toward `look_at` (its height is ignored; it must lie away from the point horizontally). Without `look_at` the view keeps its current horizontal heading (from a view straight down or up, the screen's up direction). Switches to perspective, keeps the field of view. |
| `{"type": "set_lighting", "mode": "night"}` | Lights the view by `day` or by `night` (see Lights). Like the camera commands it is not an undo step and does not change the revision; it is saved with the camera, and a failed atomic batch puts it back. |
| `{"type": "set_color_by_tag", "on": true}` | Draws faces in their tags' colors, or (`false`) by their materials (see Tags). A view setting like `set_lighting`. |
| `{"type": "set_x_ray", "on": true}` | X-ray: every shown face is drawn as glass, opacity 0.5 on both sides (a side more transparent already keeps its own), so edges behind faces show. Picking, snapping and shadows are unchanged, and hidden entities stay hidden. A view setting like `set_lighting` (no undo step, not a change to the revision, put back by a failed atomic batch, saved with the camera and stored by scenes); print, PDF, SVG, the line drawing and library thumbnails ignore it. |
| `{"type": "set_hidden_geometry", "on": true}` | Hidden Geometry: entities hidden by their own flag (see Hide and unhide) are drawn ghosted (faces as glass at opacity 0.25 in their material's color, edges and guides in light grey; they cast no shadow and give no light) and can be picked, box selected and selected, so they can be unhidden. Inference and the tools' snaps still ignore them, and entities on hidden tags stay hidden. Turning it off (or going to a scene that has it off) drops hidden entities from the selection. Annotations and section planes placed by a hidden instance are not drawn while it is on. A view setting like `set_x_ray`; print, PDF, SVG, the line drawing and thumbnails ignore it. |
| `{"type": "set_shadows", "on": true, "date": "2026-06-21", "time": "16:45", "light": 80, "dark": 45, "on_ground": true}` | Sets the sun's shadows; every field is optional and absent ones stay as they are. See Location and sun shadows below. A view setting like `set_lighting` (no undo step, not a change to the revision, put back by a failed atomic batch, saved with the camera and stored by scenes). |

In the window, the Camera menu has Zoom Extents (Shift+Z; holding the keys zooms once), the
standard views, the two projections and Night Lighting (checked at night; `set_lighting`); it
sends these same commands.

Navigating in the window moves the same document camera, about the point under the cursor:
the model face there, else the ground (no farther along the cursor's ray than three times the
larger of the eye-to-target distance and the eye's height), else the point that far along it.

- Mouse: middle drag orbits, Shift+middle drag pans, Option/Alt+left drag (Shift+Option+left
  to pan) does the same without a middle button, and the wheel zooms toward the cursor.
- Trackpad: by default two-finger scroll orbits as if dragging the scene, Shift+scroll pans,
  Cmd/Ctrl+scroll and pinch zoom toward the cursor. Settings > Input can swap scroll and
  Cmd+scroll, so scroll zooms and Cmd+scroll orbits. That setting is window chrome, kept in the
  preferences; `orbit`, `zoom` and `pan` are unaffected.
- Orbit speed is per logical pixel, the same on any display. One scroll or pinch event zooms
  at most three wheel notches. Zooming in on a point stops 0.01 m short of it, then moves on
  through it toward what lies beyond; zooming where nothing is under the cursor moves the eye
  and target together. A new two-finger scroll, or a camera change made another way, picks a
  fresh orbit pivot.

Fly mode walks or flies through the model like a game. F (or Camera > Fly mode, whose tooltip
lists these controls) turns it on; Esc or F turns it off.

- Entering keeps the eye and its heading, levels the view to within about 1 degree of straight
  up or down, and puts the target 1 m ahead of the eye; a parallel view switches to
  perspective. Moving carries the eye and target together, so the `camera` query (and a saved
  file) shows where the flyer is. The target stays 1 m ahead when fly mode ends. An agent
  switching the view to parallel ends fly mode.
- The pointer is hidden and held in place; moving the mouse looks around (yaw about the
  vertical, pitch stopping short of straight up or down, at the same speed per logical pixel
  on any display). Where the pointer cannot be held it stays visible and the view turns while
  the left button is held in the view. Two-finger trackpad scroll also looks around, as if
  dragging the scene.
- W/S move forward and back along the view direction (look up to fly up), A/D sideways, E or Space up,
  Q or Ctrl down, by key position (any layout). Speed builds up and dies down over about
  0.15 s.
- The speed starts at 3 m/s and goes from 0.25 to 60 m/s in steps of x1.25: the mouse wheel,
  `[` `]`, `-` `=`, or Cmd+trackpad scroll. Shift flies 3 times faster, Option/Alt at 0.3
  times. The speed is kept for the session; the status bar shows it.
- While flying, tool keys, clicks and the hover do nothing and the marker and tool previews
  are hidden; the tool's operation waits until fly mode ends. Cmd shortcuts (save, undo and
  the rest) still work. Switching to another window lets go of every held key and frees the
  pointer. There is no collision with the model.

### Picking and inference

`pick` and `infer` answer what an agent looking at a `render_view` image (or at the window)
needs to act on a pixel. A pixel is `[x, y]` in physical pixels from the view's top left corner
(x right, y down; the middle of the image's pixel (i, j) is `[i + 0.5, j + 0.5]`), in a view of
`width` x `height` pixels (both or neither, 1 to 4096; default the window's 3D view in physical
pixels, which on a Retina display is twice its size in points; 1280 x 800 headless)
seen by `camera` (`eye`, `target` and the optional `fov_y_deg`, `up` and `projection`, as
`render_view` takes them; default the document camera, which never moves). So a pixel of a
`render_view` image is picked by passing the same `width`, `height` and `camera`. A pixel outside
the view is `invalid_input`. Points, rays, normals and distances are in the open context's
coordinates, like commands; only the camera is in root coordinates.

`{"type": "pick", "pixel": [x, y], ...}` or `{"type": "pick", "ray": {"origin": [...],
"direction": [...]}}` returns every face the pixel's ray (or the given ray, any non-zero
direction) meets in front of its origin, whatever lies in front of it, and every edge (soft
edges are hidden and left out), guide, and dimension or leader note (by its lines, once each)
passing within `radius` of it, nearest first; what hidden tags hide is left out (see Tags).
Labels are not picked here (they are the window's):

```
{"type": "pick", "hits": [
  {"id": "f25", "path": [], "point": [1,1,1], "normal": [0,0,1], "side": "front", "distance": 9.0},
  {"id": "f12", "path": ["i40"], "point": [1,1,0], "normal": [0,0,-1], "side": "back", "distance": 10.0}]}
```

- `path` lists the instances from the root down to the one whose group or component holds the
  entity (`[]` for the root's own), whatever instance is open; `open_path` with it opens that
  group, after which the id is the open context's.
- `point` is where the ray meets a face, or the point of an edge, guide or annotation line
  nearest the ray;
  `distance` is how far along the ray it is, from its origin (the eye in perspective, the near
  end of the view in parallel). Faces have `normal` (the direction their front faces) and `side`
  (`front` or `back`: which side the ray meets). Vertices are not picked (`infer` finds them).
- `radius` is in pixels on screen for a pixel (default 4, at most 1000) and in meters from the
  ray for a ray (default 0: faces only). `camera`, `width` and `height` go with a pixel only.

`{"type": "infer", "pixel": [x, y], ...}` or `{"type": "infer", "point": [x, y, z], ...}` (a
model point, at the pixel where the view shows it and looked for where it is, even behind
other geometry, as a `tool_input` point is; `invalid_input` when it is not in view) returns
what a tool's click there would snap to, with the window's inference rules (points hidden behind
faces are skipped; the order of preference is endpoint, midpoint, origin and guide point, then
intersection, on an edge, on a guide line, along an axis from `from`, on a face, the ground):

```
{"type": "infer", "kind": "endpoint", "point": [2,2,1], "entities": [{"id": "v5", "path": []}]}
```

- `kind` is `endpoint`, `midpoint`, `origin`, `guide_point`, `intersection` (two edges or
  guide lines crossing on screen; the point lies on the nearer), `on_edge`, `on_guide`, `axis`
  (with `axis`: `x`, `y` or `z`, the axis through `from` the point is held to, which Shift would
  lock), `parallel` and `perpendicular` (to the edge the cursor last rested on, from the point
  a tool draws from; `tool_input` only), `on_face`, `ground` (z = 0 of the open context) or `nothing` (a point on the pixel's ray as
  far from the eye as the camera's target is; in a parallel view, at the target's depth). For
  a `point`, `ground` and `nothing` give the point itself (`ground` when it lies at z = 0).
- `entities` are what it snapped to, each with its `path` as in `pick`: the vertex of an
  endpoint, the edge of a midpoint or of a point on an edge, the guide of a guide point or of a
  point on a guide line, the two edges or guide lines of an intersection (the one the point lies
  on first), the face of a point on a face; empty for the others.
- `from` is the point a tool is drawing from, as after its first click: points along the red,
  green and blue axes through it are offered. Without it there are no axis snaps. The editor's
  own state (its tool, its last point, a Shift or arrow-key lock, the edge the cursor last
  rested on for parallel snaps) is not used.
- `radius` is how far from the pixel to snap, in pixels (default 8, the window's on a display of
  scale 1; at most 100: inference pairs every line within twice the radius).

### Editor

The editor is what surrounds the model in the window (and in headless mode, which runs the same
editor without one): the active tool and what it is doing, the material the Paint tool paints
with, the units imports are read in, and the status bar. `{"type": "editor"}` returns:

```
{"type": "editor", "tool": "select", "phase": "idle", "points": [],
 "paint_material": null, "import_units": "meters", "flying": false, "fly_speed": 3.0,
 "viewport": [1280.0, 800.0], "status": "Select  |  Click an edge, face or group, or drag a box; Shift adds or removes; double-click a group to edit it",
 "prompt": "Click an edge, face or group, or drag a box; Shift adds or removes; double-click a group to edit it", "message": null, "lock": null,
 "measurement_label": "", "measurement": null, "typing": false, "soften_angle": 20,
 "message_kind": null, "dialog": null, "modifiers": {"shift": false, "alt": false, "cmd": false},
 "hovered": "f3", "snap_label": null}
```

- `tool` is `select`, `line`, `rectangle`, `circle`, `polygon`, `arc`, `push_pull`, `move`,
  `rotate`, `scale`, `offset`, `follow_me`, `tape_measure`, `dimension`, `text`, `erase`, `paint`,
  `section_plane` or `axes`. The editor starts in the Select tool.
- `phase` is `idle` (the next click starts an operation) or `active`, and `points` are the
  points the operation was given so far, in the open context's coordinates: Line, the chain's
  first point and the point the next edge starts from; Rectangle, the first corner; Circle and
  Polygon, the center; Arc, the start, then the end; Push/Pull, where the face was clicked; Move,
  the base point; Rotate, the center, then where the angle starts; Scale, the point that stays
  put, then the grip clicked; Offset, where the face (or chain) was pressed; Follow Me is always `idle`; Tape Measure, where the measurement starts; Axes, the origin, then the red point; Dimension, the ends clicked (or the clicked edge's ends), or a circle's center; Text, the point the note points at, then where its text goes (none for a note fixed to the view). The Select tool is `active` with no points while its button is held (a
  click or a box, both on screen); Erase, Paint and Follow Me are always `idle`.
- `paint_material` is the material the Paint tool paints with (`null`: the default material);
  `import_units` is what one unit of an imported OBJ or STL file is in the window's import options
  sheet (`meters`, `centimeters`, `millimeters`, `inches` or `feet`; the `import` command takes its own
  `scale`).
- `flying` and `fly_speed` (meters a second) are fly mode's (see Camera). Headless nothing moves
  the eye while flying; turning fly mode on still levels the view as in the window.
- `viewport` is the window's 3D view in physical pixels (on a Retina display twice its size in
  points; 1280 x 800 headless): the view `pick` and
  `infer` use when not given a size.
- `status` is the status bar's line as the window shows it; `prompt`, `message` (feedback from
  the last tool action or failed command), `lock` (e.g. `Locked: red axis`, also Rotate's plane
  lock before its first click) are its parts.
  `message_kind` is `"error"` or `"info"`, or `null` when there is no message; only an error left by a
  typed value flashes in the window (see Window actions below). A tool's dead-end messages (Push/Pull's
  double-click with no pull to repeat, say) are `info`.
  `measurement_label` and `measurement` are the measurements box: the typed text while
  `typing`, else the live measurement (`null` when there is none).
- `soften_angle` is the Soften Edges window's angle in degrees, and `dialog` the window open
  over the view: `soften_edges` or `null`. The window's file and unsaved-changes dialogs are
  modal: while one is open the window handles no requests, which wait until it closes. Its
  sheets (import options, component name, rename, field of view, a note's text) and the
  Settings window are chrome and do not hold requests. The note sheet of a note the Text tool
  is placing closes when the tool no longer waits for its text (a `cancel`, a `set_tool`, or
  the text typed through `tool_input`). A note sheet never replaces another sheet the user has
  open: it opens once that one closes, if the note is still being placed (or still there).
- `modifiers` are the modifier keys held (`cmd` is Cmd or Ctrl): the user's in the window and
  those a `tool_input` `modifiers` event holds (see Driving the tools).
- `hovered` is what a click would act on now with Select, Push/Pull, Paint or Erase (an id of
  the open context, `null` otherwise), and `snap_label` is the snap the window names by the
  cursor ("Endpoint", "On Red Axis", "Parallel to Edge", "Midpoint in Group"; `null` when none,
  and in Select unless the Paste tool is out). Both are as the window last drew them, and change as the cursor moves.
  Both are also `null` while flying, while the cursor is outside the view, and while the pointer
  is over a window, a menu or the gizmo above the view, where the window shows neither.
  `tool_input` puts the cursor in the view under no window, so its result reports both.

Editor commands go through `execute` (not `batch`). They are not undo steps of their own and,
except for `cut_selection`, `paste_clipboard` with `at` and `paste_in_place`, which each run one
model step (`cut` or `paste`), do not change the revision (`place_library_item` changes nothing
itself; the click that places the item is the `insert` step); each returns
`{"editor": {...}, "revision": n}`, the state after it:

| Command | Effect |
|---|---|
| `{"type": "set_tool", "tool": "rectangle"}` | Switches to the tool (a fresh one, even if it is the active tool), dropping the operation in progress and anything typed, as the tool rail does. |
| `{"type": "set_paint_material", "material": "m6"}` | The material the Paint tool paints with; `null` or absent for the default material. The tool does not change (a click on a swatch in the window's Materials tab does the same; a double-click then sets the Paint tool). An unknown material is `not_found`. |
| `{"type": "set_import_units", "units": "millimeters"}` | What one unit of an imported file is: sets the units of the window's import options sheet for the rest of the session. The sheet's other values (up axis, import as, merge coplanar faces) are the `import` command's own parameters. |
| `{"type": "cancel"}` | Aborts the tool's operation, like pressing Escape until the tool is idle: leaves fly mode, drops typed text, the direction lock and the operation (and a copy array or a retyped angle the tool waits for), and lets go of the modifiers `tool_input` holds. It never closes the open group (`close_instance` does). |
| `{"type": "set_fly_mode", "on": true, "speed": 6}` | Turns fly mode on or off, as F does (see Camera), and sets its speed in meters a second (0.25 to 60, else `invalid_input`; absent: unchanged). In the window the pointer is held while flying. |
| `{"type": "duplicate"}` | Starts Move copying the selection, as Edit > Duplicate (Cmd+D) does: with the cursor in the view the base point is the point under it, so the copy follows the cursor; otherwise Move waits for a base point. With nothing selected it is `invalid_input`. The copy is made by the next click or typed distance (one `copy` step). |
| `{"type": "copy_selection"}` | Edit > Copy (Cmd+C): puts the selection on the app's clipboard as clip text (see Clipboard); the window's is the system clipboard, so another app process can paste it. With nothing selected it is `invalid_input`; a clipboard that cannot be written is `file`. Not an undo step. (It is named apart from `copy`, which copies by an offset.) |
| `{"type": "cut_selection"}` | Edit > Cut (Cmd+X): `copy_selection`, then erases the selection as one undo step labelled `cut`. Nothing is erased when the copy fails. |
| `{"type": "paste_clipboard", "at": [3, 0, 0]}` | Edit > Paste (Cmd+V). With `at` (a point of the open context), runs `paste` with the clipboard's clip there. Without it the Paste tool comes out: the clip's edges follow the cursor, its base point on the snapped point under it, and the next click (`tool_input`) pastes there as one `paste` step (the pasted entities become the selection; see `selection`); then (even when the paste changed nothing, landing exactly on what is there), or on Escape or `cancel`, the tool used before comes back (`editor.tool` stays that tool meanwhile, with `phase` `active` and the prompt `Click where to paste (Esc cancels)`; the snap marker and `snap_label` show even when that tool is Select). Typed distances are not taken. With `at` too the pasted entities become the selection. Text that is not a clip is `invalid_input` with the message `The clipboard does not hold sketchup-alt geometry`, and changes nothing; a clip `paste` refuses (damaged, over 64 MiB, nested more than 32 deep, over 100,000 instances or 20,000,000 faces with nesting counted, or landing beyond the coordinate limit) is `invalid_input` with why, before the Paste tool comes out, and changes nothing; a clipboard that cannot be read is `file` (`Could not read the clipboard: ...`). |
| `{"type": "paste_in_place"}` | Edit > Paste In Place: runs `paste` with the clipboard's clip and no `at`, so it lands where it was copied from in world coordinates, and the pasted entities become the selection (see `selection`). Refused as `paste_clipboard` is. |
| `{"type": "place_library_item", "path": "/abs/lib/chair.skr"}` | The Library tab's tile click: the Paste tool comes out with the library item at `path` (a component or model item; see Library items) instead of the clipboard's clip, as `paste_clipboard` without `at` does. The item's edges follow the cursor by the minimum corner of its box (inside a turned or scaled group they are drawn in world axes and size too, so the preview is what the insert makes), and the next click (`tool_input`) runs `insert_library_item` there as one undo step labelled `insert`, with new definitions each time, then the tool used before comes back; Escape or `cancel` puts it away with nothing inserted. `editor.tool` stays the tool used before, with `phase` `active` and the prompt `Click where to paste (Esc cancels)`. `path` follows the read rules (`--api-root`). `invalid_input` for a material item (use `insert_library_item`), and for a file that is not a valid item or is too large, before the tool comes out. |
| `{"type": "export_pdf", "path": "/abs/plan.pdf", "paper": "a3", "landscape": true, "scale": 100, "dpi": 150, "overwrite": false}` | File > Export PDF...: renders the current view (camera, lighting, display settings and section cuts; nothing selected or highlighted) with the GPU on white paper, without the ground grid and axes, and with no group open (the model as it looks closed) and writes a PDF 1.4, one page per sheet. `paper`: `a4` (default), `a3`, `letter`, `legal`, `tabloid`; `landscape` defaults to true; `dpi` 72 to 300 (default 150). Without `scale` the view, its aspect kept, is fitted on one sheet inside 10 mm margins. With `scale` (1:N, a parallel view; a perspective view is `invalid_input`, "Switch to parallel projection to print at scale") the extent of the print as drawn (no group open, only the geometry kept by section cuts, and the shown dimensions and notes, so one placed outside the model widens the print) is printed along the view at exactly 1:N, tiled across ceil(w/W) x ceil(h/H) sheets (more than 100 is `invalid_input` naming the count), with trim marks at each tile's corners, a scale bar (1, 2 or 5 times a power of ten of the display unit: feet for feet and inches, inches for inches, metres otherwise, labelled in the model's length format) and a "Sheet r,c of R x C" footer. Dimension and note text is placed as PDF text over the image. The bytes are the same for the same model and view on the same GPU. Path rules are the export's: `--api-root`, `.skr` and the open model's own file refused, an existing file that is not a PDF refused unless `overwrite`; they are checked before rendering. Result: `{"editor": ..., "pdf": {"sheets": n, "warnings": [...]}, "revision": r}`. Without a GPU adapter (headless on a machine without one) it is `invalid_input`, "printing needs a GPU adapter". Not an undo step. |
| `{"type": "set_dialog", "dialog": "soften_edges"}` | Opens the Soften Edges window over the view (`null` or absent closes it), as Edit > Soften Edges... and its close button do. |
| `{"type": "set_soften_angle", "angle": 30}` | The Soften Edges window's angle, in whole degrees (0 to 180; above 180 is `invalid_input`, a negative or fractional angle `-32602`), which its Soften button uses. |

### Driving the tools

`tool_input` feeds the active tool the events a user makes in the 3D view, through the same
editor code the window runs: the same inference and snapping, Shift and arrow-key locks, typed
values ("3", "2,3", "24s", "5x", "/5", angles), Ctrl copying, Cmd sampling and double-click
opening. Every tool behaviour is reachable this way, in the window and headless alike. The editor
starts in the Select tool, so the examples that draw first switch tool with `set_tool`.

With the Line tool active (`{"type": "set_tool", "tool": "line"}` through `execute`):

```
{"jsonrpc": "2.0", "id": 9, "method": "tool_input", "params": {"events": [
  {"type": "click", "point": [0, 0, 0]},
  {"type": "move", "point": [1, 0, 0]},
  {"type": "type", "text": "3"}]}}
```

Events, run in order:

| Event | What a user does |
|---|---|
| `{"type": "move", "pixel": [320, 200]}` | Moves the cursor there: the inference and the tool's preview follow. |
| `{"type": "down", "point": [1, 2, 0], "button": "left"}`, `{"type": "up", ...}` | Presses or releases a button there (`left` default, `middle`, `right`). A middle drag orbits and Shift+middle pans, as in the window; Alt+left orbits. A button left down stays down for a later call. |
| `{"type": "click", "point": [1, 2, 0]}` | Moves there, presses and releases. Separate `click` events are never a double-click, however soon they are sent: each press counts as coming after the double-click time. |
| `{"type": "double_click", "point": [1, 2, 0]}` | Two left clicks there, then the double-click: the Select tool selects a face with its edges, or an edge with its faces, opens the group under it, or closes the open one when there is nothing of it there. Push/Pull repeats the last pull distance on the face (the one being pulled, else the one under the cursor). In Push/Pull, a click that finishes a pull, followed within the double-click time by another click, repeats the pull (the SketchUp convention). The second click of a `double_click` (and the second and third of a `triple_click`) comes within the double-click time; its first does not, so a `click` that starts a pull followed by a `double_click` finishes the pull with the double-click's first click. In the window, Push/Pull ignores a second press within the double-click time near the first click (a jittery double-click) rather than finishing a pull of a few pixels; a later click there finishes it. |
| `{"type": "triple_click", "point": [1, 2, 0]}` | Three left clicks there: the Select tool selects everything connected to the edge or face under it (a triple-click that follows a double-click that opened or closed a group does nothing more). |
| `{"type": "drag", "pixels": [[100, 100], [200, 150], [300, 300]]}` (or `points`) | Presses at the first place, moves through the others, releases at the last (at least two): a Select box (dragged right: inside; left: crossing), or a navigation drag with `"button": "middle"`. With Line, Rectangle, Circle, Polygon, Arc, Push/Pull or Scale (pressed on a grip), a drag acts as two clicks when the release is at least the drag threshold (4 px) from the press, inside the view. While a navigation drag turns the view, the hover waits, as in the window. |
| `{"type": "key", "key": "r", "shift": false, "alt": false, "ctrl": false, "cmd": false}` | A key pressed with those modifiers held: one character (tool shortcuts, "g" group, "G" or Shift+g component, "Z" zoom extents, "f" fly, or a measurement's first character), or `enter`, `backspace`, `delete`, `escape`, `space`, `right` / `left` / `up` (lock the red / green / blue axis during an operation, or unlock it; Rotate also takes them before its first click, to lock the protractor's plane, and an agent `cancel` drops that lock), `down` (unlock), `ctrl` (Ctrl tapped alone: Move and Rotate switch copying, the Tape Measure guide making, Polygon inscribed and circumscribed, Scale scaling about the center, Dimension a radial dimension's diameter and radius). An uppercase letter holds Shift. With `cmd` or `ctrl` (given or held) only `z` (undo; with `shift` redo), `a` (select all; with `shift` select none), `d` (duplicate, as the `duplicate` command), `x` (cut, as `cut_selection`), `c` (copy, as `copy_selection`) and `v` (paste, as `paste_clipboard` without `at`) are taken, the last four not with `shift`; the window's file shortcuts are commands (`new`, `open`, `save`, `import`, `export`, `quit`) and other keys, Cmd+Shift+D included, are `invalid_input`. With `alt` (given or held) no key is taken (`invalid_input`): in the window Alt with a key does nothing. |
| `{"type": "modifiers", "shift": true}` | Holds exactly these modifiers (`shift`, `alt`, `ctrl`, `cmd`; absent ones are let go, so `{"type": "modifiers"}` lets go of all): Shift over an axis, parallel or perpendicular snap locks the direction, Shift toggles in the Select tool, Shift switches the Scale tool between uniform and per-axis scaling, Cmd or Ctrl makes a Paint click sample the material. They stay held across calls (see Modifiers below). |
| `{"type": "type", "text": "2,3"}` | Types the text into the measurements box, replacing anything typed, and presses Enter. Unlike a key, a first letter is text here, not a shortcut. Refused (`invalid_input`) while Cmd, Ctrl or Alt is held. Right after a pull, a typed distance redoes the pull at that distance (keeping its direction), as Rotate retypes its last angle, Scale its last scale and Offset its last distance; Move only replaces a copy with an array (`5x`, `/5`). A retyped pull is refused ("Click a face first") if another step has come in between. |

- **Where**: a `pixel` is in a view of `width` x `height` (both or neither; default the 3D view,
  `editor.viewport`, 1280 x 800 headless), scaled to the 3D view, which must have the same
  proportions (within 1%); the tool infers from that pixel as for a click there, with the
  window's rules: what is in front wins. A `point` is in the open context's coordinates; the
  cursor goes to the pixel where the document camera shows it at the event's turn (the camera
  may have moved), and the tools look for that model point itself, even when other geometry
  hides it from the camera: what lies in front of it (or behind it) is passed over, so it
  snaps to what is at the point (a corner or edge within the snap radius and about as deep,
  the face it lies on, even the back of a thin wall) or else stands for itself in free space (`nothing`, or `ground` at z = 0), and a click
  there acts on that face (Paint paints it, Push/Pull pulls it). Exact values come from
  typing them.
- **Scale**: with something selected, grips sit on the corners, edge middles and face middles
  of its box along the model axes (none on an axis where the box is flat). `click` a grip by
  its model point (the grip under the cursor is the nearest within 10 px), `move`, then
  `click` again, or `type` instead: a plain number is a factor (`2`, `-1` mirrors), a number
  with a unit a length (`3m`): the new size, whatever point stays put. One value scales every
  moving axis, or give one per moving axis in x, y, z order (`1.5,2`, `1m,2m`). One length
  scaling uniformly is the new size along the grip (from the opposite grip; the diagonal for a
  corner grip); otherwise each length is the new size of the box along its axis. Factors must be
  between 1e-4 and 1e4 in size. The point
  that stays put is the opposite grip, or the box center after a `ctrl` key. A corner grip
  scales uniformly, an edge or face grip along its axes; holding Shift (`modifiers`) switches
  that. With nothing selected, a click selects what the Select tool's would. It makes one
  `scale` command; typed again right after, the scale is redone (`undo`, then `scale`).
- **Offset** (key `o`): `click` a face (the point pressed), `move`, then `click` again, or
  `type` instead. The distance is the cursor's, in the face's plane, from the nearest side:
  inside the face (a hole counts as inside) it offsets inward, outside it outward (the outer
  loop only); a typed length takes the side the cursor is on (negative reverses it; without a
  `move` the side of
  the click). With edges forming one chain in one plane selected, a `click` near them
  starts a chain offset instead, to the side of the cursor (a typed length must be positive);
  a straight chain is offset in its face's plane, else in a plane parallel to the ground
  (a vertical one in a plane facing the view). A distance the shape cannot take (it would
  collapse or cross itself) is refused with a message and nothing runs. It makes one `offset`
  command; typed again right after, it is redone (`undo`, then `offset`, keeping the
  direction), and a `double_click` on a face offsets it by the last distance (kept across
  tools).
- **Follow Me** (key `j`): `click` the profile face. With edges (and nothing else) selected,
  the selected edges are the path; with exactly one other face selected, that face's outer
  loop is the path (a lathe: select the circle's disk, click a face beside it). With anything
  else selected, or nothing, the message is "Select the path edges, then click the profile
  face" and nothing runs. It makes one `follow_me` command; a refusal (a profile too big for a
  turn, a bad path) shows the command's message and changes nothing. It takes no typed value.
- **Axes** (`y`): `click` the new origin, `click` a point on the red axis, `click` a point on the
  green side of the red-green plane; the third click makes one `set_axes` (in the open context's
  coordinates). A click on the origin, or a third in line with red, runs nothing and says why
  (`message`); arrow keys lock the inferred direction as for other tools; Escape starts again. It
  takes no typed value.
- **Dimension** (no key): `click` a point, `click` a second one, `move` to where the dimension
  line goes and `click`, or `type` the line's distance from the ends (on the cursor's side) once
  the cursor has moved off their line; it makes one `add_dimension`, each end the vertex
  clicked (an endpoint snap on the open context's vertex, which the end then follows) or the
  point. `click` an edge away from its ends instead, then where the line goes: one
  `add_edge_dimension` (an edge inside a group, seen from outside it, gives an `add_dimension`
  between its ends as points). `click` an edge of a circle or an arc, then where the label
  goes: one `add_radial_dimension`, a diameter for a circle and a radius for an arc, which a
  `ctrl` key switches before that click. Clicking the first point again, or placing the line
  on the measured line itself (or typing a zero distance), is refused with a message. Typed again right after, a distance moves the line of the dimension just added
  (`undo`, then the command again).
- **Text** (no key): `click` a point on the model (a vertex, which the note follows, or a point
  on an edge, a face or a guide), then where the text goes; or `click` empty space (the ground
  or nothing) for a note fixed to the view there (refused with a message while a group is
  open). The tool then asks for the text (`editor.phase` stays `active`, the prompt says so):
  in the window a sheet asks; here `type` it (a single line), or a `key` `enter` with nothing
  typed for a leader note's automatic text (a note fixed to the view needs text and says so).
  It makes one `add_note` or `add_screen_note`. A `cancel`, an `escape` key or another `click`
  drops the note being placed. A `double_click` on a note with Select selects it and, in the
  window, opens the same sheet to change its text; agents use `change_note`.
- **Modifiers** a `modifiers` event holds stay held after the call, until a later `modifiers`
  event lets them go or `cancel` (so a Shift lock can be set in one call and used in the
  next); arrow-key locks last until the operation ends. A `key` event's own modifiers are
  held only while that key goes down. In the window the user's own keys add to them: letting
  go through `tool_input` never releases a key the user holds. `editor.modifiers` (in the
  result and the `editor` query) shows what is held.
- **Frames**: after each event the hover is worked out as a frame drawn between events would,
  except while a navigation drag turns the view (then, as in the window, it waits for the
  button to come up).
- **Errors**: malformed events are `-32602`; an event that makes no sense (both or neither of
  `pixel` and `point`, a drag of one place, an unknown key, a key with `alt`, empty text, a
  size out of range) or a pixel outside the view is `invalid_input` naming it
  (`event 2: ...`) and nothing runs. An event that cannot run at its turn (a point not in
  sight, a key or typing under a held Cmd or Alt) stops the rest: the error is `-32000` with
  `data` holding `kind` (`invalid_input`), `message`, `event` (its index) and `completed`, a
  result as below for what ran before it (`editor`, `inference`, `preview`, `commands` with
  their outcomes, `revision`); those events stay done. A drag stopped partway lets go of its
  button without the release reaching the tool: a navigation drag stops where it got to, and
  a press the tool saw is cancelled (as Escape), so no orbit or Select box is left going.
  While flying, clicks and typing do nothing, as in the window.

The result is the editor after the events:

```
{"editor": {...EditorState...},
 "inference": {"kind": "axis", "axis": "x", "point": [3, 0, 0]},
 "preview": [[[0, 0, 0], [3, 0, 0]]],
 "commands": [{"command": {"type": "add_edge", "start": [0,0,0], "end": [3,0,0]},
               "outcome": {"changes": {...}}}],
 "revision": 1}
```

`inference` is what the cursor snaps to now (kinds as `infer` names them; `null` while flying),
`preview` the tool's rubber-band segments (open context coordinates), and `commands` every
command the events ran, in order, each with its `outcome` or `error` (a CommandError): the
tool's, a copy's selection of what it made, a shortcut's (`make_group`), an undo. The window
also shows a failed command's message in `editor.message`. Subscribers get the usual `changed`,
`selection`, `camera` and `editor` notifications.

`editor.hovered` and `editor.snap_label` in the result say what the window highlights and names
after the events. `inference` is reported in Select too, though the window draws no marker
there (unless the Paste tool is out). `preview` no longer outlines the hovered target; agents read `editor.hovered` for that.

### Location and sun shadows

The model's location is a model property: `latitude` and `longitude` in degrees,
`north_angle`, the degrees from the green axis clockwise (seen from above) to true north, and
`utc_offset`, the clock's hours east of UTC. The defaults are SketchUp's: 40.0167, -105.2833
(Boulder), 0 and -7. `{"type": "set_location", "latitude": 51.5, "longitude": -0.12,
"north_angle": 12.5, "utc_offset": 0}` changes the fields given and keeps the others, as one
undo step labeled `set_location` (a change to the revision; nothing to change makes no step).
`latitude` is -90 to 90, `longitude` -180 to 180, `north_angle` -360 to 360 and `utc_offset`
-12 to 14 in steps of 0.25 (5.5 is fine, 5.1 is not); anything else is `invalid_input` and
changes nothing. The model's drawing axes (`set_axes`) do not move north: north is measured
from the world green axis. `{"type": "location"}` returns `{"type": "location", "latitude":
40.0167, "longitude": -105.2833, "north_angle": 0.0, "utc_offset": -7.0}`.

The shadow settings are a view setting, in the `camera` query's `shadows`:

```
{"on": false, "date": "2026-11-08", "time": "13:30", "light": 80, "dark": 45, "on_ground": true}
```

`{"type": "set_shadows", ...}` takes any of these fields (the values above are the defaults).
`on` lights the model by the sun and draws shadows (ignored, with no shadows drawn, while the
lighting is `night`). `date` is `YYYY-MM-DD`, a real date in the years 1800 to 2200 (leap years
count); `time` is `HH:MM`, the clock at the model's `utc_offset`, 00:00 to 23:59; `light` and
`dark` are whole percentages, 0 to 100, for how bright a face the sun hits and a face in shade
is; `on_ground` draws the shadows on the ground plane at z = 0. A bad field is `invalid_input`
and none of the fields is applied. The settings are saved with the camera and stored by
scenes (`add_scene`, `update_scene`; `go_to_scene` restores them; a scene's `view` has the same
`shadows`), and `new` starts with the defaults.

`{"type": "sun"}` returns the sun at the settings' date and time whether or not `on` is set:
`{"type": "sun", "direction": [-0.424, -0.773, 0.472], "azimuth": 208.754, "elevation": 28.174,
"up": true}`. `direction` is the unit vector from the model towards the sun, in the model's
x, y, z (z up; true north is the green axis turned clockwise by `north_angle`), `azimuth` is
degrees clockwise from true north (0 to 360), `elevation` degrees above the horizon (negative
at night; geometric, with no refraction) and `up` is whether the sun is above the horizon. The
position is the NOAA solar calculator's algorithm (Julian century of the instant in UTC,
equation of center, apparent longitude, corrected obliquity, equation of time, hour angle),
within 0.05 degrees of the NOAA calculator's geometric elevation and azimuth.

Drawing with `on` set (the window, `render_view`, snapshots and raster print/PDF; the hidden-line
drawing, SVG and library thumbnails ignore it): each face is shaded `dark + light * cos * lit`
times its color, where `cos` is the cosine of the angle between the side facing the viewer and
the sun (0 behind it) and `lit` is 0 to 1 from the shadow map. Without `on` faces are shaded by
the headlight as before. A sun below the horizon gives only the `dark` level and no shadows.
Faces with an opacity of 0.7 or more (the more opaque side counts) cast the sun's shadow, glowing
faces included; see-through faces, ghosted and hidden faces and what a section plane cuts away
do not, and X-ray does not change what casts. With `on_ground` a shadow-only layer on z = 0
darkens the background where the sun does not reach, by the ratio of the dark level to the lit
ground's; it is not drawn when the camera is below z = 0. The shadow map is one 2048 pixel
(1024 on the web) depth map along the sun, fitted to the casters' bounds, so a model many times
larger than the detail loses sharpness, and redrawn only when the sun or the casters change.

### Files

Models are saved as `.skr` files: a zip archive holding `document.json`, which records `format`
(`"sketchup-alt"`), `schema_version` (16 when the location is not the default (written in the model as `location` with `latitude`, `longitude`, `north_angle` and `utc_offset`) or the view or a scene's view has shadow settings other than the defaults (written as `shadows`, with `on`, `year`, `month`, `day`, `minutes`, `light`, `dark` and `on_ground`), else 15 when the model axes are not the world axes (written as `axes` with `origin`, `x`, `y` and `z` in the model), else 14 when something is hidden or the view has `x_ray` or `hidden_geometry` on, else 13 when the model has a section plane or `set_section_display` has turned something off (written in the view as `section_planes`, `section_cuts` and `section_fill`, only when false), else 12 when the model has a dimension or a note, or display units other than meters with 3 decimals, else 11 when the model has a scene, else 10 when the model has a tag or the view colors faces by tag, else 9 when a face has a texture position, else 8 when a material glows or the view is lit by night, else 7 when a material has a
texture, else 6 when a material is see-through, else 5; a material's `opacity` is written only
when below 1 and its `emission` only when above 0, and a face's `texture_position` only when it
has one, so a model whose materials are all opaque
and dark is saved exactly as before opacity existed and older builds still open it; version 11
and older files open with no dimensions or notes and in the default display units; version 10
and older files open with no scenes; version 9
and older files open with no tags and everything Untagged; version 8
and older files open without texture positions; version 7
and older files open without glow; version 6 and older files open without textures; version 5 files open with
every material opaque, version 4 files, written before materials, open unpainted, version 3 files, written before camera projections, open in perspective, and
versions 2 and 1, written before curves and guides, still open), the camera (eye, target, field
of view, up, projection, `lighting` when it is `night`: a file without it opens lit by day,
and `new` starts by day, `"color_by_tag": true` when faces are colored by tag, and `"x_ray"` and `"hidden_geometry": true` when on, which need schema 14 as hidden entities do) and the model with every id, edges with their curve and soft flag, `"hidden": true` on each hidden edge, face, instance and guide (schema 14, written only when something is hidden; a file without hidden entities is written as before),
the materials and what is painted with them, the tags (name, color, and `"visible": false` when
hidden) and each entity's `tag` (absent when Untagged), and the scenes in order (`pages`: each
one's id, name, `view` when it stores one (`eye`, `target`, `up`, `fov_y` in radians, and
`parallel`, `night`, `color_by_tag`, `x_ray` and `hidden_geometry` when true, and `shadows` when not the defaults), and `tags`, the tag ids it hides, when it
stores tag visibility), each mesh's `dimensions` and `notes` (see Dimensions and notes; absent
when none), and the display `units` (absent when the default). Each texture's image is stored in the archive as
the file it was read from, `textures/m31.png` (or `.jpg`), and the material's `texture` in
`document.json` names it with its `width` and `height`.

| Command | Effect |
|---|---|
| `{"type": "save", "path": "/abs/house.skr"}` | Writes the file (the old file is replaced only once the new one is complete: the bytes go to a new hidden `.sketchup-alt-<pid>-<n>.partial` beside it, renamed over it at the end; no other file is touched). Without `path` it saves to the file last opened or saved; a model never saved then gives `invalid_input`. An existing file at `path` that is not a model (any other file but an empty one) is refused with `invalid_input` unless `"overwrite": true`. A model `open` would refuse (a scene's view beyond +/-1e6 m, say) is not written: the error is `file` and names it ("scene p40's view is beyond +/-1000000 m"). |
| `{"type": "open", "path": "/abs/house.skr"}` | Replaces the model and the view with the file's. With unsaved changes it fails with `unsaved_changes` unless it has `"discard": true`. A SketchUp file (a `.skp` path, or a file whose header is SketchUp's) opens as a new model not saved anywhere: what `import` reads from it (see Import) goes straight into the root, with no wrapping group (the file's top-level faces, edges and instances are the root's), lit by day, with the view zoomed to it; the outcome carries the `import` report (without `instance` and `definition`); `file` reports no `path` and `modified` false, so `save` needs a `path` (a `.skr`), and never writes over the `.skp`. |
| `{"type": "new"}` | An empty model with the default view, not saved anywhere. With unsaved changes it fails with `unsaved_changes` unless it has `"discard": true`. |

Paths are relative to the app's working directory, or with `--api-root` to the first
root (see Access and safety). None of these is an undo step. `open` and
`new` clear the undo history, the selection and the open context, and change the revision;
subscribers get a `resync` notification and should read the model again. Ids in an opened file
are those it was saved with, and new ids continue after the largest one ever used. A file that
cannot be read or written gives the error kind `file`, with a message naming the path and the
reason (a missing file, not a sketchup-alt file, a missing or newer schema version, a document
over 256 MiB, an id of 2^53 or more or one number used as the id of two things, a transform that cannot be inverted, a coordinate beyond the
+/-1e6 m limit, a curve whose edges branch, fall apart or are straight, a soft edge bounding
fewer than two faces, a face or instance painted with a material the file lacks, an entity on a
tag the file lacks, a scene hiding a tag the file lacks or whose view is unusable or beyond the
limit, a material, tag or scene name that is blank, too long or used twice (or a tag named
`Untagged`), a material opacity outside 0 to 1, a texture
whose image is missing, broken or too large, or whose size is out of range, or a face texture
position out of range); error messages never quote the file's contents (a JSON fault gives its
line and column); the model is left as it was. `save` also fails
when the model has given out ids past 2^53, which no file can hold. `open` and `new` never drop changes the user has
not saved (`modified` in the `file` query) unless told to: they fail with `unsaved_changes`, and
a client that means it, or has asked the user, passes `"discard": true`. In the window, `new`,
`open` and `quit` with `discard: true` first write the unsaved model to a recovery file,
`<name>-<seconds>.skr` in the recovery folder (macOS `~/.sketchup-alt/recovery`; Linux
`$XDG_STATE_HOME/sketchup-alt/recovery`, by default `~/.local/state/sketchup-alt/recovery`; files
an earlier version left in `~/.sketchup-alt/recovery` stay there); the window's status bar and
its Agent Activity window name it; only the user may read it (0600). A sidecar note
`<file>.json` beside it records that an agent discarded it, the window session, the file it was a
copy of and when; once that window has ended, File > Recover Models... in another window lists
it. If the file cannot be
written the command fails with `file` and nothing is discarded. Headless, nothing is written. The window asks the user
itself before its own New and Open.

The window also autosaves: while the model has unsaved changes, every few minutes (Settings >
Files, 5 by default, Off turns it off) it copies the model to
`<name>-<session>.autosave.skr` in the same folder, with its sidecar, overwriting that one copy in
place; while a drag or a tool operation that has not changed the model yet is in progress it
waits, at most one interval more. The user's own file is never written by
autosave. The copy and its sidecar are removed when the model is saved, when New or Open replaces
it (the window's or an agent's `save`, `new` and `open`), and when the window quits after the unsaved
prompt or an agent's `quit`. A quit the system asks for without the prompt, such as logging
out on macOS, keeps it, writing the newest changes to it first.
Each window holds a lock on `session-<session>.lock` in the folder while it runs, so copies of a
window that is still open are never offered elsewhere. A window that ended without cleaning up
(a crash) leaves its autosave behind: at the next start the Recover Models window opens and
lists it, with Open (after the unsaved-changes prompt, the copy opens as an untitled model with
unsaved changes, titled `<name> (recovered)`, and becomes the new window's autosave) and Discard
(after asking), and Discard All. File > Recover Models... lists the same and the work agents
discarded; opening a discarded copy also makes it this window's autosave, removed like any
other (on save, or on Don't Save at quit). A copy whose note cannot be read, or whose file is
missing or not private, is listed with why and can only be discarded; a copy whose contents
cannot be opened is listed too, and Open says why in the status bar. The newest 200 entries are
listed, with a note saying how many older ones were not; at start only autosave notes are read.
Listing also removes temporary files (`.sketchup-alt-*.partial`, `.session-*.locking`) left a
day or more by a write or a start that never finished. On
macOS and Linux the listing skips links and any note or copy other users can read (each file
must be 0600); on Windows it relies on the folder's owner-only access list and does not check
each file. If the folder cannot be used the window starts with autosave off and says so in the
status bar. Headless and MCP modes do not autosave. `new`, `open`, `save` and `export` are not allowed in an
atomic batch.

### Export

`{"type": "export", "path": "/abs/house.glb"}` writes every face of the whole model as
triangles: the root's faces and every instance's, placed in root coordinates (a component placed
twice is written twice), whatever instance is open. Edges without faces are not written, nor
is what hidden tags hide (see Tags; a model whose faces are all hidden is "nothing to export").
`format` (`obj`, `stl`, `glb`, `gltf`, `skp`, `svg`) defaults to the one the path's extension names, in
any case; any other extension without `format` is `invalid_input`, and so is a model with no
faces (for `skp`: with nothing at all, or only empty groups and components, "nothing to export"). A SketchUp file holds the whole
scene rather than triangles (see the `skp` row below).
A `.skr` path (any case) and the open model's own file are refused with `invalid_input`: export
never overwrites a model; use `save`. After `open` of a SketchUp file the model has no file of
its own, but exporting to the file it was opened from (compared after resolving the paths) is
refused with `invalid_input` unless `"overwrite": true`: the export would replace it with a 2017
copy without what this app cannot keep (scenes, curves, unused materials).
An existing file at `path` (and, for an OBJ with materials, at its `.mtl` and image paths) that is
not of the format written is refused with `invalid_input` unless `"overwrite": true`; all are
checked before anything is written. An empty file is always replaced, and a link is judged by the
file it points to (under `--api-root` any of these files that is a link is refused, and what it
points to is not read). Formats are recognised from their first 64 KiB: OBJ is UTF-8 text whose every
complete line is blank, a `#` comment or starts with an OBJ keyword, with at least one keyword
line; MTL is the same with MTL keywords and at least one `newmtl`; STL is text whose first line
is `solid` (alone or followed by a name, any case) and whose next line starts with `facet` or
`endsolid`, or a binary STL whose size is 84 + 50 x its triangle count; `glb` starts with
`glTF`; `gltf` is a JSON object whose `asset` has a `version` string (read up to the `asset`,
within 16 MiB); `skp` starts with SketchUp's file header; a texture image is a PNG or JPEG. Something that is not a regular file (a folder, a device) is refused.

| Format | Content |
|---|---|
| `obj` | Text OBJ: shared `v` positions and `vn` normals, one `f` triangle per line; Y up (x, z, -y). When faces are painted, it names a material library (`mtllib house.mtl`) written beside it under the same name with `.mtl` (replacing a file there), and each material's triangles follow a `usemtl` line; the library gives each material its diffuse color (`Kd`, linear) and, for a see-through one, its opacity as `d` and its transparency `Tr` (1 - `d`). Material names have spaces and other separators replaced by `_` (and a number added when two become alike); faces without a material use `default`. Without painted faces no `.mtl` is written. A material with a texture adds `map_Kd house_Tiles.png` (the library's name, `_`, the material's name, and the image's own extension), and that image file, as it was read, is written beside the library (replacing a file there); its triangles name `vt` texture coordinates (`f v/vt/vn`), counted from the image's bottom left. A glowing material adds `Ke`, its color (linear) times its emission, and with a texture also `map_Ke` naming the same image. |
| `stl` | Binary STL: one record per triangle with its normal; Z up, as in the model; no colors or opacity |
| `glb` | Binary glTF 2.0: one mesh of flat-shaded triangles (`POSITION`, `NORMAL`), one primitive per material; materials are double-sided, so lone faces show from behind; a painted one has its name and its color as `baseColorFactor` (linear, with the opacity as alpha), and a see-through one `alphaMode` `BLEND`; the default one is named `default`; a material with a texture has a `TEXCOORD_0` attribute and a `baseColorTexture` (its image file, as it was read, in the buffer; a sampler with linear mipmapped filtering that repeats both ways) with a white `baseColorFactor` (and the opacity as alpha); a glowing material has `emissiveFactor`, its color (linear) times its emission up to 1 (white, and an `emissiveTexture` naming its texture, when it has one), and an emission above 1 goes in `KHR_materials_emissive_strength` (`emissiveStrength`, listed in `extensionsUsed`); Y up |
| `gltf` | The same glTF document as JSON, its buffer embedded as a base64 data URI |
| `skp` | A SketchUp 2017 file (SketchUp 2017 and newer open it), written from the scene rather than triangles: the root and every group and component definition the root places (at any depth), each written once in its own coordinates and placed by its instances (transform and material kept, a mirrored one stays mirrored); faces with their holes and front and back materials; every edge, soft edges soft and smooth; guide lines and points; every material (color, opacity, texture image as it was read and its size); each textured face side carries its texture's placement (a face whose corners are too close to a line to fix it keeps the default projection, with a warning); every tag, used or not, as a layer with its name, color and visibility (hidden tags stay hidden), and every edge, face, instance and guide on its tag, hidden or not (Untagged ones on the file's Layer0); hidden edges, faces, instances and guides are written hidden. Lengths are inches, as SketchUp stores them; Z up. Material, tag and definition names are trimmed, blank ones become `Material`, `Tag`, `Group` or `Component`, they are cut to 240 UTF-16 units, and a name already taken gets the first free number (" 2", " 3"), so copied groups sharing a name come back as `Group`, `Group 2` (and a tag named `Layer0`, the file's own layer, is written as `Layer0 2`). Groups and components with nothing in them are left out. Curves (circles, arcs, polygons) are written as their separate edges. The same model gives the same bytes. |

In the triangle formats (`obj`, `stl`, `glb`, `gltf`), colors and textures come from each face's
front side (its own material, else that of the nearest painted instance placing it); back-side
materials are not exported. A SketchUp file keeps both sides' materials and placements. Texture coordinates follow
the projection described under Textures with each face's texture position on top. An OBJ export whose `.mtl` would be the export path
itself or the open model's file is refused with `invalid_input`. The image files are written
first, then the `.mtl`, then the `.obj`, so a failure writing the `.obj` can leave new image
files and a new `.mtl` beside the old (or no) `.obj`. A glTF material with a texture and an
opacity below 1 uses `alphaMode` `BLEND`, so a viewer also applies the image's own alpha,
which the window ignores.

Units are meters. Export writes the file the same safe way as `save`, is not an undo step, and
does not change the revision or the `file` query. Its result has `"export": {"triangles": n,
"skipped_faces": k}`: the triangles written and the faces left out because they could not be
triangulated (counted once per placement). For `skp` both are 0 and the result adds what was
written, each definition counted once: `definitions`, `instances`, `faces`, `edges`, `guides`,
`materials` and `tags` (each absent when 0), and `warnings` (absent when empty) for what the file
cannot hold: "N materials glow; SketchUp files cannot store that", "N curves were written as
separate edges", "N empty groups or components were left out", "N textured faces kept the
default texture placement; their corners are too close to a line", and what it does not write:
"N scenes were not written" (scenes are not exported). Every format but SVG (which draws dimensions and notes and applies section cuts, see below), `skp` included, leaves out
dimensions, notes and section planes; `warnings` then says "N dimensions were not written", "N notes were
not written" and "N section planes were not written" ("1 section plane was not written"; the root and each definition the root places counted once). A binary STL holds at most 2^32 - 1
triangles and a `.glb` at most 4 GiB (about 59 million triangles); a larger model is refused
with `invalid_input` and nothing is written.

#### SVG line drawing

`{"type": "export", "path": "/abs/view.svg"}` (or `"format": "svg"`) draws the current view (the
document's camera at its viewport size) as a line drawing, not triangles. Visible edges only,
found on the CPU: soft edges, edges and faces on hidden tags, and what the active section planes cut
away are left out (with the section display's `cuts` on, as in the window), each edge is cut into pieces of
about 2 px (at most 256 per edge) and a piece shows when nothing nearer lies on the line of sight to
its middle; neighbouring visible pieces join into one `<polyline>`. A section cut adds its fill (grey
`#3a3a3a` polygons, when the display's `fill` is on, which also hides what is behind them) and its outline
(`cuts`, 3 times as thick). Dimensions and notes add their lines and their text (a dimension's
length, a note's text, a screen note at its fraction of the view); guides and section plane
outlines are not drawn. Layers, in this order: `fills`, `lines`, `cuts`, `annotations`, `labels`.
Black lines of 1 px, text of 11 px, y down from the view's top left; numbers have 3
decimals and the same view gives the same bytes. A drawing that would test more than 2,000,000 pieces
is `invalid_input` ("the model is too large for a line drawing at this size"), and so is a view with
nothing to draw ("nothing to export"). With a parallel camera, `"scale": 100` exports at 1:100: the SVG
is in millimetres (`width`, `height` and `viewBox` in mm, lines 0.25 mm, text 3 mm), 1 m of the model
being 10 mm. A scale with a perspective camera is `invalid_input` ("Switch to parallel projection to
export at scale"), and so is one for another format or one that is not a positive number. The result's
`export` has `"lines"`: the polylines drawn (edges and cut outlines), and no triangles.

### Import

`{"type": "import", "path": "/abs/chair.obj"}` reads an OBJ, STL or SketchUp file and adds its
faces to the open context as one undo step. `format` (`obj`, `stl`, `skp`) defaults to the one
the path's extension names, in any case; a file with another extension whose header is
SketchUp's is read as `skp`; any other file without `format` is `invalid_input`, and so is a
`.skr` path (use `open`).

| Field | Meaning |
|---|---|
| `as` | `group` (default): a new group named after the file (`chair`), placed at the open context's origin and selected. `component`: the same as a component. `context`: loose geometry in the open context that merges with what it touches (splitting edges and faces as drawing does), as if the group were made and exploded; what comes out is selected. Drawing with sticky geometry takes seconds for big files, so a file making more than 10,000 faces is `invalid_input` here (import it as a group). |
| `scale` | Meters per file unit, default 1 (a millimeter file: `0.001`). Positive and finite, else `invalid_input`. A SketchUp file has real units: `scale` multiplies them (2 doubles the model). |
| `up` | The file's up axis: `y` or `z`. Defaults to `y` for OBJ and `z` for STL, as `export` writes them, so exporting and importing again gives back the same geometry (a Y-up file's (x, y, z) becomes (x, -z, y)). Ignored for SketchUp (Z-up, as the model). |
| `merge_coplanar` | Default true: neighbouring faces that share an edge no third face uses, lie in one plane, face the same way and have the same material become one face (an STL's triangles come back as the model's faces, holes included). False keeps one face per polygon. Ignored for SketchUp: faces are kept as the file has them. |

Reading: OBJ `v` positions, `f` polygons (`v`, `v/vt`, `v/vt/vn`, `v//vn` corners; negative
indices count back from the last position), `usemtl` and `mtllib`, comments and lines continued
with `\`; normals, texture coordinates, lines, groups and objects are ignored. STL is binary
when the file is exactly 84 bytes plus 50 per triangle its header counts, else ASCII; stored
normals are ignored. Every polygon's front is the side its corners turn counter-clockwise
around. Corners closer than 1e-6 m (after scaling) are welded into one vertex. A polygon that is
not flat, or whose sides cross, becomes triangles. A polygon with fewer than three distinct
corners, no area, or the same corners as an earlier one makes no face and is counted as
`skipped`. Faces are made as the file has them: nothing is split where they cross, and no face
is added the file does not have.

OBJ materials: a material name the model already has is that material (whatever its color);
a name that only the `mtllib` libraries (read beside the OBJ, relative to its folder; missing
or unreadable ones are ignored) give a diffuse color (`Kd`, linear) becomes a new material of
that color, listed in `changes.materials`; `default` (exactly that, lower case; `Default` is an
ordinary name) and unknown names are the default material. Only materials some made face uses
are added, and a file whose libraries give more than 1,024 new materials for the names it uses
is a `file` error. A `mtllib` name with spaces is read as the whole rest of the line, with its
separate words as fallbacks (several libraries on one line). Materials go on the faces' front
sides.

SketchUp files (`skp`) from SketchUp 2017 and newer are read (2017 to 2020 files, which keep
their model in an older format, import as the same model saved by a newer SketchUp would); a
file from before SketchUp 2017 is `invalid_input` ("SketchUp files older than 2017 are not
supported"). What is carried:

- Components and groups: every definition the file's model places, at any depth, becomes one
  definition (a group when every placement of it is a group, else a component; named as in the
  file, or `Group` / `Component`), shared by all its instances, with each instance's transform
  (mirrors and scales included) and material. Names longer than 240 characters are cut.
  Definitions placed nowhere are not imported.
  The file's top level goes into the new group or component (`"as": "context"`: into the open
  context, its instances included).
- Faces with their holes; a face that is not flat to within 5e-7 m or whose loops cross is made
  from its outer loop alone (cut into triangles where needed), losing its holes. Front and back
  materials. A face's front is the side its plane in the file faces.
- Edges: every edge, loose ones included; soft and smooth edges become soft edges (an edge
  not between exactly two faces stays an ordinary edge), and hidden edges become hidden edges.
- Materials used by an imported face or instance: color, opacity, and the texture image (PNG
  or JPEG, read from the file, under the limits of `add_material`: at most 16 MiB, 4096 pixels a
  side, the pixel budget and 256 textures) with the file's tile size (clamped to 0.001 to
  1000 m, times `scale`). A name the model already has gets a number (`Brick 2`); existing
  materials are never changed. Names longer than 240 characters are cut. A texture that cannot
  be used leaves the material its color, with one warning per reason ("2 textures were not
  imported (the image is over 16 MiB or 4096 pixels a side); their materials keep their
  colours"). At most 1,024 materials are added (the first, in the file's order, of those
  the faces and instances made use; of a file using more than 4,096, only the first 4,096 in
  order of use are considered); faces and instances with the others are left unpainted, with a
  warning.
- Guides (construction lines and points) of the top level and of every imported definition,
  at the file's positions (a guide line through its point, infinite). A guide that is not
  finite, has no direction or lies beyond +/-1e6 m is skipped, with a warning; guide points
  count toward the model's size below.
- Tags: every layer of the file but Layer0 (which is Untagged) becomes a tag, used or not,
  with its name, color (a layer the file gives no color takes the next free tag color, as
  `add_tag` does) and visibility (a hidden layer's tag is hidden); every imported edge, face,
  instance and guide goes on its layer's tag. Names are trimmed (a blank one becomes `Tag`),
  cut to 240 characters and made unique against the model's tags and `Untagged` (`Walls 2`);
  existing tags are never changed. At most 1,024 tags are added; what is on the file's further
  layers is imported Untagged, with a warning.
- Scenes (from SketchUp 2021 and newer files): each of the file's scenes becomes a scene, after
  the model's, in the file's order, with its name (trimmed, cut to 240 characters, a blank one
  named as `add_scene` names it, and made unique against the model's scenes: `Front 2`). A
  scene that restores the camera stores its view: the file's camera in meters times `scale`, in
  model coordinates (through the open instance's placement, like the geometry), perspective or
  parallel, its field of view (a parallel camera keeps the height of its view through it), lit
  by day, not colored by tag. A scene that restores tag visibility stores the tags of its
  hidden layers as its hidden tags (Layer0, which is Untagged, and layers not imported are left
  out). What a scene does not restore it does not store. A camera that cannot be used (its eye
  on its target, beyond +/-1e6 m, or missing from a scene that restores one) is left out, with
  a warning ("1 scene's camera could not be used; the scene leaves the camera alone"). At most
  1,024 scenes are added; the file's further scenes are left off, with a warning. No scene is
  selected and the camera is not moved; `open` views the model as described there.

Not carried, each counted in `warnings`: scenes of 2017-2020 files ("12 scenes were not
imported"),
images (skipped),
instances of a definition the file lacks or with a flat or broken transform (skipped),
faces that cannot be made (skipped: an outline that cannot be read, fewer than three distinct
corners, a corner repeated, no area, or a duplicate), holes and edges that cannot be read,
faces made from their outer loop, material references that match no material of the file
(or one whose id is too large to match), guides that cannot be
read, and, in a 2017-2020 file, entities that could not be read after a damaged record
("12 entities could not be read and were skipped": the rest of the model is imported).
Section planes, dimensions, text, texture positions and the file's camera and
styles are not read.

The outcome's `import` reports what was made:

```
{"changes": {...}, "import": {"instance": "i40", "definition": "d39", "polygons": 24,
 "faces": 12, "vertices": 16, "skipped": 0, "materials": 1}, "revision": 3}
```

For a SketchUp file `polygons` counts the faces of the file's top level and of the definitions
it places (each definition once), `faces` the faces made (a face made from its outer loop
may make several), `vertices` the vertices made, `skipped` faces that made none (an outline
that cannot be read, fewer than three distinct corners, no area, or a duplicate), and
`materials` the materials added. It also has `tags` (tags added), `scenes` (scenes added),
`definitions` and `instances` (made from the file's, without the wrapper), `guides` (guides
made), `textures` (materials added with a texture) and `warnings`
(sentences, such as "2 scenes were not imported", "4 dimensions and text labels were not
imported" (2017-2020 files only),
"1 image was skipped", "5 face holes could not be read and were dropped", "1 guide could not be
read and was skipped"); each is absent when 0 or empty, and always absent for
OBJ and STL:

```
{"import": {"instance": "i6341", "definition": "d15", "polygons": 609, "faces": 616,
 "vertices": 2580, "skipped": 0, "materials": 9, "definitions": 6, "instances": 8,
 "warnings": ["1 image was skipped"]}}
```

`instance` and `definition` are absent for `"as": "context"`. Errors leave the model as it was:
a file that cannot be read, is over 128 MiB, has more than 200,000 polygons or 1,000,000 OBJ
positions, has a broken line (the message names it), a number that is not finite, a position
beyond +/-1e6 m after scaling, or no polygon that makes a face, gives the error kind `file`.
A SketchUp file that is damaged, places more than 200,000 faces or 1,000,000 vertices, has
more than 10,000 definitions or 100,000 instances, nests instances more than 32 deep, places a
definition inside itself, resolves to more than 1,000,000 placements through its nesting (a
component of 10 placed in one placed 10 times is 110) or more than 20,000,000 faces with every
placement counted, or makes nothing, is a `file` error too. One whose model unpacks to more
than 512 MiB (for a 2017-2020 file: whose model is larger than that) or holds more than
10,000,000 vertices, edges, faces, loops and instances (placed
or not), or whose placed model (every instance through its transforms, times `scale`) reaches
beyond +/-1e6 m, is `invalid_input`.
A number or index that cannot be read is reported by its line or facet, without quoting it.
`import` is allowed in an atomic batch.

In the window, File > Import... (Cmd+I) picks an OBJ, STL or SketchUp file and opens an
options sheet (units: meters, centimeters, millimeters, inches or feet per file unit, which
becomes `scale`; up axis; import as; merge coplanar faces), whose values are the parameters
above and are kept in the preferences. For a `.skp` the sheet asks only "Import as": the file
keeps its own units, axes and faces (`scale`, `up` and `merge_coplanar` are not sent).
Importing from the sheet runs `import` with them; the status line reports the faces made (and
the units when not meters) and polygons skipped, or the error. For a SketchUp file it reads
`Imported desk.skp: 147 faces, 1 definition, 1 instance` (and faces skipped), followed by
`with N warnings (see the import report)` when the report has warnings, which also go to
stderr. File > Open... lists `.skr` and SketchUp (`.skp`) files; a `.skp` runs `open` as above,
the status reads `Opened <name>.skp: ...` with the same counts, and the window title is
`<name>.skp (imported)` until the model is saved (Save then asks for a `.skr` path, suggesting
`<name>.skr`).

Details: for `"as": "context"` the report's `faces` counts the faces built from the file, before
they merge with what is already in the context. A `.mtl` over 16 MiB is ignored, as is a library
that is not a regular file or is named by an absolute path (relative paths, `..` included, are
read); a model path that is not a regular file (a directory, a pipe) is a `file` error. A
polygon of more than 256 corners skips the crossing check and is triangulated, then merged
back; with `merge_coplanar` false it stays triangles. Collinear points closer than the merge
tolerance along its outline may drop. Bent or self-crossing polygons that together make more
than 200,000 faces are a `file` error. The regular-file check happens before the read, so a path
swapped for a pipe at that very instant could still block (local only, low risk). A file may
have at most 1,000,000 polygon corners in all (OBJ `f` corners, STL vertices) and ASCII STL keywords are read in any case;
vertices left without an `endloop` are a `file` error.

### Access and safety

Anyone who can connect to the socket (programs running as the same user) can drive the app. To
keep an agent's file access to some folders, start the app with `--api-root DIR` (repeatable):
then every path a request names (`open`, `save`, `save_to_library`, `insert_library_item`, the `library` query's `folder`, `export` and the `.mtl` and images an OBJ
writes beside it, `import` and the material libraries an OBJ names, `texture.path`) must lie
within one of them, after symlinks are resolved. A relative path is taken from the first root.
A file to read must exist; a file to write (the `.mtl` and images beside an OBJ too) must be in
an existing folder and must not be a symbolic link; a root folder itself is refused as a folder,
not a file. A hard link inside a root to a file outside is followed (the API cannot create
links). A path outside the roots is refused with `invalid_input` ("outside the folders the API
may use"), the same whether it exists, is missing or cannot be read. Paths are resolved one part
at a time and may pass only through folders that hold a root and folders within one, so
`outside/x/../../root/a.skr` and a dangling link are refused the same way; a part missing within
a root (a missing file, say) gives `file`. A path with a NUL byte
is `invalid_input`. `save` without a path checks the open model's file; inside a `batch` it
needs a path. A batch with a path outside the roots runs nothing (`-32001`, its `index` names
the command). Commands run on the resolved path, so `query file` shows it; `query file` also
lists the roots as `api_roots` (null without `--api-root`). The window's own dialogs and the
FILE argument are not limited. A root that is not an existing folder stops the app (exit code
2).

On Windows, each part of a path is resolved to the name the folder or file has on disk before
it is compared, so `c:\models\A.SKR` lies within a root given as `C:\Models` when those
folders exist; names are then compared exactly, since a folder can be case-sensitive (a sibling
`C:\p\root` of a root `C:\p\Root` is outside it). A file to write keeps its name as given. A
verbatim path (`\\?\C:\...`, `\\?\UNC\server\share\...`) is
judged as its plain form, and roots and resolved paths are reported in plain form. Junctions and
other reparse points count as symbolic links. Forms Windows reads specially are refused with
`invalid_input`: a drive-relative path (`C:foo`), a path rooted without a drive (`\foo`), a
device path (`\\.\...`), a name with `:` (an alternate data stream), a reserved device name
(`CON`, `NUL`, `COM1`, ..., with any extension) and a name ending in a dot or a space. A root
given in one of these forms stops the app. A share reached by its UNC name and the same folder
reached by a drive letter are different paths: give the root in the form requests use. For a UNC root, give the server and share names in
the case requests use too: requests are compared with the root before the server and share are
resolved. Likewise for a root on a mapped or `subst` drive: requests must use relative paths or
the real (UNC) path, since a request's drive is not resolved before the comparison (a request
that does not match is refused).

The window shows `Agents: N` in its status bar while N connections are open; View > Agent
Activity lists the connections' requests (the last 100, newest first, with their results and how
long ago they came; control and format characters, such as bidi overrides, show as spaces)
and any recovery files agents' discards wrote this session. File > Recover Models... lists the
autosaves and discarded work of windows that have ended.

### Notifications

After `subscribe`, a connection is told about changes without asking, whoever made them (an
agent or the person at the window):

| Notification | Params | When |
|---|---|---|
| `changed` | `{revision, changes}`, plus `units` when the change set the display units | after every model change, in order (see Methods) |
| `resync` | `{revision}` | changes were missed: read the model again |
| `selection` | `{ids, revision}`, as the `selection` query returns | the selection changed |
| `camera` | the `camera` query's result (without `type`) | the camera, the lighting or the shadow settings changed: navigating in the window, fly mode, camera commands |
| `editor` | the `editor` query's result (without `type`) | anything in it changed: the tool, its phase or points, what is under the cursor, the status bar or measurements box (which follow the cursor in the window), the paint material, import units, fly mode, the view's size, the Soften Edges window |

`selection`, `camera` and `editor` are coalesced: at most one of each per frame the window draws
(so a drag sends one per frame, not one per mouse event), or per round of requests in headless
mode, each carrying the state at that moment; a change undone within the same frame sends
nothing (a camera turned and turned back may still send one: its eye can differ in the last
digits). In the window they follow the frame after the change, so they come after the replies
and `changed` notifications of the requests that caused them; a window that draws no frames
(minimized, or hidden on another desktop) sends them when it draws again.

Behaviors worth knowing:

- Replies come before the `changed` notifications they cause, and your own edits are notified too.
- A request without an `id` runs but gets no reply. JSON-RPC batch arrays are rejected; use the
  `batch` method.
- Camera commands return empty changes and do not change the revision. `render_view`'s camera
  takes the fields of `set_camera` (absent ones keep the document camera's values) and never
  moves the document camera; its `lighting` never changes the document's.
- A failed atomic batch is rolled back but still consumes entity ids, so later ids may skip numbers.
  Use references (`$0:f0`) rather than predicting ids.
- An `id` that is not a string, number or null is an invalid request (`-32600`, reply id null).

## Window actions and the API

Everything a person can do in the window is either a command or query here, or window chrome. The
tools turn clicks and typing into the same commands with explicit values (the status bar's
measurements become `distance`, `offset`, `angle` and so on), and `pick` and `infer` say what lies
under a pixel and where a click would snap.

| In the window | Through the API |
|---|---|
| File > New, Open... (`.skr` or `.skp`), Save, Save As... (Cmd+N, Cmd+O, Cmd+S, Shift+Cmd+S), the unsaved-changes prompt | `new`, `open` (`discard`), `save` (`path`); the prompt reads `Do you want to save changes to "<name>"?` (Save, Don't Save, Cancel); it is still `unsaved_changes` / `discard` |
| File > Open Recent (the last 10 files) and Clear Menu | `open` (`path`); the list is chrome |
| File > Recover Models... and the window that opens at start after a crash: Open, Discard, Discard All | none: window chrome over the recovery folder (see the autosave paragraph above); agents' `save`, `new` and `open` remove the window's autosave |
| The empty-model card's New, Open... and recent files | `new`, `open` (`path`) |
| File > Import... (Cmd+I) and its options sheet (units, up axis, import as, merge coplanar faces; only import as for a `.skp`) | `import` (`scale`, `up`, `as`, `merge_coplanar`); `set_import_units`, `editor.import_units` |
| File > Export... (Shift+Cmd+E); the dialog also lists SVG drawing (pixels) | `export` (`format`; `svg` with `scale`) |
| File > Export PDF... and File > Print... (a Print sheet: paper, orientation, Fit or 1:N, resolution; remembered in the prefs; Print writes `sketchup-alt-print-<pid>-<n>.pdf` in the temp folder and opens it with the system's viewer) | `export_pdf` (`path`, `paper`, `landscape`, `scale`, `dpi`, `overwrite`) |
| Quit (app menu on macOS, File menu elsewhere; Cmd+Q), closing the window | `quit` (`discard`) |
| View > Agent Activity; the context bar's Agent Activity indicator; the status bar's `Agents: N` | none: they only show what agents do |
| Settings (Cmd+,): Appearance (theme, UI scale), Input (trackpad scroll) and Files (autosave interval, library folder); View > Theme | none: window chrome, kept in the prefs file (see below) |
| Library tab: a tile click, a tile's Insert, Open and Delete from Library | `place_library_item` (component or model), `insert_library_item` (a material, or Insert), `open`; the `library` query lists the folder; deleting a file has no command |
| Entity Info's Save to Library, the Materials panel's Save to Library, File > Save Model to Library... | `save_to_library` (`kind` `component`, `material`, `model`) |
| Settings, Model tab: the model's display units and precision (saved in the model) | `set_units`; the `units` query |
| Edit > Erase (Delete), Edit > Delete Guides | `erase` with the `selection` query's ids; `delete_guides` |
| Edit > Soften Edges...: angle, Soften, Harden; Entity Info's Soft box | `soften_by_angle` (`max_angle_deg`), `soften_edges` (`soft`); the window's angle and whether it is open: `set_soften_angle`, `set_dialog`; `editor.soften_angle`, `editor.dialog` |
| Edit > Intersect Faces > With Model, With Selection, With Context (enabled with a selection), and the same three in the viewport's right-click menu over a selection with faces or groups (the section plane keeps its own right-click menu); the status says "Added N edges, split M faces" or "No faces crossed"; not in the read-only phone viewer | `intersect_faces` (`with`: `model`, `selection`, `context`) with the `selection` query's ids |
| Edit > Hide (enabled with something shown selected) and Edit > Unhide > Selected (a hidden entity selected, possible with Hidden Geometry on), Last, All; the Entity Info Hidden checkbox (unchecked for a mix); right-click an Outliner row: Hide or Unhide that instance; the viewport's right-click menu over a selection: Hide (something still shown), Unhide (hidden selection); Unhide > Last and All are enabled only when they would show something; hiding an Outliner row keeps the open context and selection; the status says "Hid 3 entities", "Unhid 1 entity" or "Nothing to hide" / "Nothing to unhide"; not in the read-only phone viewer, which still honours hidden entities | `hide`, `unhide`, `unhide_last`, `unhide_all` with the `selection` query's ids; `outliner` rows' `own_hidden` |
| View > X-ray and View > Hidden Geometry (checked toggles, saved with the camera and scenes; not in the read-only phone viewer) | `set_x_ray`, `set_hidden_geometry`; `camera` query's `x_ray` and `hidden_geometry` |
| The Shadows dock tab (after Scenes): On, Time (slider in 15-minute steps and an HH:MM field), Date (slider over the day of the year, a "Nov 8" label) and Year, Light and Dark (0 to 100), On ground, and the Location fields (latitude, longitude, north angle, UTC offset); View > Shadows (a checked toggle). The shadow settings apply as the controls move (no undo step); a location field applies when Enter is pressed or the field is left, one undo step; a value out of range is refused with the command's message in the status line. Not in the read-only phone viewer, which draws the saved shadows | `set_shadows`, `set_location`; `camera` query's `shadows`; the `location` and `sun` queries |
| Edit > Reverse Faces and Edit > Orient Faces (enabled with a face selected), and the same two in the viewport's right-click menu over a selection with faces; Orient works from the face that was right-clicked when it is a selected face, else from the selected face with the lowest id (Edit > Orient Faces has no face under the cursor), among the selected faces when several are selected, else over the whole connected surface; the status says "Flipped 2 faces", "Faces already agree" or counts the conflicts | `reverse_faces` with the selected faces; `orient_faces` (`id`, `ids`) |
| Edit > Select All, Select None (Cmd+A, Shift+Cmd+A) | `select` with `all: true`; `select` with `ids: []` |
| Edit > Duplicate (Cmd+D) | `duplicate`, or `copy` with an explicit offset |
| Edit > Cut, Copy, Paste (Cmd+X, Cmd+C, Cmd+V), Paste In Place; Cut and Copy need a selection, Paste and Paste In Place a clip on the clipboard (looked at when the window gains focus); with a text field focused the keys and items are the field's | `cut_selection`, `copy_selection`, `paste_clipboard` (`at`), `paste_in_place`; or `paste` with clip text |
| Undo, Redo (Cmd+Z, Shift+Cmd+Z; disabled when there is nothing to undo or redo, the tooltip naming the step) | `undo`, `redo`; `history` lists the steps |
| Group menu and context bar: Make Group (G), Make Component... (Shift+G, a sheet that suggests `Component N`, the first free N), Explode; Close Group (Group menu, Escape, the breadcrumb) | `make_group`, `make_component` (the agent passes `name` itself; an empty name is `invalid_input`), `explode`, `close_instance` |
| Group > Rename Component... (F2), Entity Info's Name field | `rename_definition` (the instance's `definition` from `entity`) |
| Group > Purge Unused | `purge_definitions` (`purged`) |
| Camera menu: Zoom Extents (Shift+Z), standard views, Perspective / Parallel, Night Lighting, the context bar's day/night toggle | `zoom_extents`, `standard_view`, `set_projection`, `set_lighting` |
| Camera > Field of View... (a sheet; perspective only) | `set_camera` (`fov_y_deg`, with the current `eye`, `target` and `up` kept); `set_camera` takes `fov_y_deg` in either projection |
| Orbit, pan and zoom with the mouse or trackpad (about the point under the cursor), fly mode (F, Escape) and its speed | `orbit` and `zoom` (`about` a point), `pan`, `set_camera`; a middle `drag` in `tool_input`; `set_fly_mode` (`on`, `speed`), `cancel`; `editor.flying`, `editor.fly_speed`. Flying itself (moving keys, mouse look) moves the camera, which `set_camera` places directly |
| Drawing by dragging (Line, Rectangle, Circle, Polygon, Arc, Push/Pull); Push/Pull retype and double-click repeat | `tool_input` (`drag`, `type`, `double_click`) |
| The tool rail and the tools' keys | `set_tool` or a `tool_input` key; `editor.tool`, `editor.phase`, `editor.points` |
| Tools > Axes (rail, Y): click the origin, a point on red, a point on the green side; Group > Make Unique, Flip Along > Red, Green, Blue and Component's Red, Green, Blue; the viewport menu's Make Unique, Flip Along and Reset Axes; Entity Info's instance count | `tool_input` (`click`s) or `set_axes`; `make_unique`; `flip` (`frame` `axes` or `own`); `reset_axes`; `definitions` (`instances`), `camera` (`axes`) |
| Tools > Section Plane (rail; no key): click a face or the ground, arrow keys lock the normal to an axis; right-click a selected plane: Flip, Active Cut; View > Section Planes, Section Cuts, Section Fill; Entity Info of a plane | `tool_input` (`click`, arrow `key`s) or `add_section_plane`; `flip_section_plane`, `activate_section_plane`, `deactivate_section`; `set_section_display`; `section_planes`, `section_display`, `entity` |
| Select: click, Shift-click, box (inside or crossing), double-click (bounds, or open a group), triple-click (connected) | `tool_input` (`click`, `modifiers`, `drag`, `double_click`, `triple_click`); `select` (`mode`; `box` with `crossing`), `pick` for what is under a pixel, `open_instance` / `open_path` |
| Line, Rectangle, Circle, Polygon (`8s`, Ctrl switches circumscribed), Arc, Push/Pull, Move (Ctrl copies, `5x`, `/5`), Rotate, Scale (grips, Shift uniform, Ctrl about the center), Offset (drag or type a distance; double-click repeats), Follow Me (select the path, click the face), Tape Measure, Dimension (Ctrl: diameter or radius), Text (and its note sheet), Erase, Paint (Cmd samples) | `tool_input`, or the commands they make: `add_edge`, `add_rectangle`, `add_circle` (`segments`), `add_polygon` (`sides`, `circumscribed`, `direction`), `add_arc`, `push_pull`, `move`, `copy` (`count`), `rotate` (`copies`), `scale` (`origin`, `factors`), `offset` (`distance`, `toward`), `follow_me` (`face`, `path`), `add_guide_line` / `add_guide_point`, `add_dimension`, `add_edge_dimension`, `add_radial_dimension`, `add_note`, `add_screen_note`, `erase`, `paint` (`side`); a sampled material is the `entity` query's |
| Select: double-click a note, then its text in the note sheet | `change_note` |
| Tools > 3D Text...: its sheet (text, height, depth, filled), then a click places the text | `add_3d_text` (`position`, `height`, `depth`, `filled`) |
| Inference snapping, Shift and arrow-key locks, the marker and the snap label by the cursor | `tool_input` (`modifiers`, arrow `key`s; `inference` in its result); `infer` (with `from`; `axis` is what Shift would lock); `editor.snap_label` |
| Hover highlight (Select, Push/Pull, Paint, Erase) | `editor.hovered`; `pick` for any pixel |
| The orientation gizmo: an axis end, the centre | `standard_view`, `set_projection` |
| Typing into the measurements box, Escape | `tool_input` (`type`, `key`); the commands' explicit values; `cancel`; `editor.measurement`, `editor.typing` |
| The status bar | `editor.status` (and `prompt`, `message`, `lock`) |
| Materials grid: click to choose (for editing and painting; the tool is unchanged), double-click to paint, Add, Delete, name, color, opacity, Glow, Texture..., Remove, texture size | `set_paint_material` (a double-click is `set_paint_material`, then `set_tool` `paint`), `add_material`, `delete_material`, `change_material` (`name`, `color`, `opacity`, `emission`, `texture`); `materials` |
| Entity Info: what is selected, texture Offset / Rotation / Scale, Rotate 90, Reset | `entity`, `entities`, `curve`; `position_texture` (`offset`, `rotation_deg`, `rotate_by: 90`, `scale`, `reset`) |
| Outliner: the tree, click to select, Shift/Cmd-click to toggle, double-click to open, the Model row | `outliner`; `open_path` then `select` (`toggle`); `open_path` (`[]` for the root) |
| Context bar breadcrumb (`Model > House > Kitchen`), Esc | `open_path` with the crumb's path; `close_instance` |
| The window's title (the file name; the edited dot on macOS, `*` elsewhere) | `file` |
| The 3D view's size | `editor.viewport` (the default view of `pick` and `infer`); `render_view` takes any size |

Window chrome, which changes nothing in the model, its view or the editor, has no command: the
window's size and position (at least 960 x 640 points), the panels' sizes and scroll positions,
which menus are open, tooltips, which Outliner rows are expanded, the Outliner's filter text (an
agent filters the `outliner` query's rows itself), the Help > Keyboard Shortcuts window, the export
dialog's suggested format, keyboard focus, and the window's preferences: the theme (Settings or
View > Theme), the UI scale, the trackpad scroll setting, the import options, the dock's width,
divider position and tab, the library folder and the recent files list, kept in `~/.sketchup-alt/prefs.json` on
macOS and in `$XDG_CONFIG_HOME/sketchup-alt/prefs.json` (by default
`~/.config/sketchup-alt/prefs.json`) on Linux, which the API never reads or writes (files an agent opens or saves are not added to the list). The card
the window shows over an untouched empty model (New, Open..., recent files, a drawing hint) is
chrome too, and so is dismissing it. Whether the menus are the native macOS menu bar or the
window's own is chrome too; on macOS, Cmd shortcuts act through the native menu even while a text
field has focus, except Undo, Redo, Select All and Select None, which go to the field. The window's
3D view follows the theme (its day background, the selection's color and width, and the axes, drawn
red, green and blue in the theme's axis colors); `render_view` keeps one fixed look, so its images
do not depend on the window. The view's feedback is chrome too: the pointer's shape per tool (a
grabbing hand while orbiting or panning), the snap chip, which appears once the cursor has rested
150 ms, the 2.5 s flash by the cursor (the error a typed value leaves: the tool refusing it or its
command failing. Clicks and informational messages do not flash. The text is `editor.message`), the
Push/Pull solid preview (its triangles are not reported; `tool_input`'s `preview` has its outline:
the face's loops, the pulled loops and the edges joining them), and the orientation gizmo in the
view's corner. `render_view` never shows the hover highlight, markers, preview lines, solid or
gizmo. The file dialogs are chrome too (commands take paths); while one, or the unsaved-changes
prompt, is open, the window answers no requests. The sheets (import options, component name,
rename, field of view, a note's text) and the Settings window are chrome as well, and hold no requests. The window
is exposed to screen readers such as VoiceOver through AccessKit; that is chrome too.

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
`not_found`, `nothing_to_undo`, `nothing_to_redo`, `unexpected_step`, `file` or `unsaved_changes`), `-32001` batch failed
(`data` is `{index, error, completed, rolled_back}`), `-32002` rendering unavailable and `-32003`
connection lost (MCP `--live` only: the app's socket or pipe closed).
