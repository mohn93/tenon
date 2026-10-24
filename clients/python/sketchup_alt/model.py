"""Higher-level helpers over ``Client``: materials, primitives, solids, instances, labels,
scenes and renders."""
from .transform import Transform


def _id(x):
    return x.id if hasattr(x, "id") else x


def _ids(xs):
    if hasattr(xs, "id") or isinstance(xs, str):
        xs = [xs]
    return [_id(x) for x in xs]


def _tjson(t):
    return t.to_json() if isinstance(t, Transform) else t


def _color(c):
    """``"#rrggbb"`` or three 0-255 numbers."""
    if isinstance(c, str):
        h = c.lstrip("#")
        if len(h) != 6:
            raise ValueError("a color is '#rrggbb' or (r, g, b) with 0-255 each, got %r" % (c,))
        return [int(h[i : i + 2], 16) for i in (0, 2, 4)]
    r, g, b = c
    return [int(round(r)), int(round(g)), int(round(b))]


class Material:
    def __init__(self, id, name):
        self.id = id
        self.name = name

    def __repr__(self):
        return "Material(%r, %r)" % (self.id, self.name)


class Instance:
    """A group or component instance in the model."""

    def __init__(self, model, id, definition=None):
        self.model = model
        self.id = id
        self.definition = definition

    def __repr__(self):
        return "Instance(%r)" % (self.id,)

    def move(self, offset):
        """Move by ``offset`` meters."""
        self.model.client.execute({"type": "move", "ids": [self.id], "offset": list(offset)})
        return self

    def rotate(self, axis, degrees, origin=(0, 0, 0)):
        """Turn by ``degrees`` about the line through ``origin`` along ``axis``."""
        self.model.client.execute(
            {"type": "rotate", "ids": [self.id], "center": list(origin), "axis": list(axis), "angle": degrees}
        )
        return self

    def set_transform(self, transform):
        """Replace the instance's transform (a ``Transform`` or an API transform dict)."""
        self.model.client.execute({"type": "set_transform", "id": self.id, "transform": _tjson(transform)})
        return self

    def paint(self, material):
        """Paint all of it with ``material`` (a ``Material`` or id)."""
        self.model.client.execute({"type": "paint", "ids": [self.id], "material": _id(material)})
        return self

    def hide(self):
        self.model.client.execute({"type": "hide", "ids": [self.id]})
        return self

    def name(self, text):
        """Name its definition (a shared definition renames for every instance)."""
        if self.definition is None:
            for e in self.model.client.query("entities")["instances"]:
                if e["id"] == self.id:
                    self.definition = e["definition"]
        self.model.client.execute({"type": "rename_definition", "definition": self.definition, "name": text})
        return self


class Model:
    """Builds a model through a ``Client``. Lengths are meters."""

    def __init__(self, client):
        self.client = client
        self._materials = {}

    # plumbing

    def _instance(self, r, definition=None):
        placed = r.get("placed")
        if placed:
            iid = placed[0]
        else:
            made = [i for i in r["changes"]["created"] if i.startswith("i")]
            if not made:
                raise RuntimeError("the command made no instance: %r" % (r,))
            iid = made[0]
        d = definition or (r.get("definitions") or [None])[0]
        return Instance(self, iid, d)

    def _instances(self, r):
        return [Instance(self, i) for i in r.get("placed", [])]

    # materials

    def material(self, name, color, metalness=0, roughness=1):
        """A named material, reused if it exists. ``color``: ``"#rrggbb"`` or (r, g, b) 0-255.
        ``metalness`` and ``roughness`` are 0 to 1 (the defaults send nothing)."""
        cmd = {"type": "add_material", "name": name, "color": _color(color), "existing": "reuse"}
        if metalness != 0 or roughness != 1:
            cmd["metalness"] = metalness
            cmd["roughness"] = roughness
            cmd["existing"] = "update"
        r = self.client.execute(cmd)
        m = Material(r["changes"]["materials"][0], name)
        self._materials[name] = m
        return m

    # primitives

    def _prim(self, type, fields, opts):
        cmd = {"type": type}
        cmd.update({k: v for k, v in fields.items() if v is not None})
        if "as_" in opts:
            opts["as"] = opts.pop("as_")
        if "material" in opts and opts["material"] is not None:
            opts["material"] = _id(opts["material"])
        cmd.update({k: v for k, v in opts.items() if v is not None})
        return self._instance(self.client.execute(cmd))

    def box(self, corner, size, **opts):
        """Box with a corner at ``corner`` and sides ``size``. ``opts``: name, material, as_
        (group, component or geometry)."""
        return self._prim("add_box", {"corner": list(corner), "size": list(size)}, opts)

    def cylinder(self, base, radius, height, axis=None, segments=None, **opts):
        return self._prim(
            "add_cylinder",
            {"base": list(base), "radius": radius, "height": height, "axis": axis and list(axis), "segments": segments},
            opts,
        )

    def tube(self, base, radius, inner_radius, height, axis=None, segments=None, **opts):
        return self._prim(
            "add_tube",
            {
                "base": list(base),
                "radius": radius,
                "inner_radius": inner_radius,
                "height": height,
                "axis": axis and list(axis),
                "segments": segments,
            },
            opts,
        )

    def cone(self, base, radius, height, top_radius=None, axis=None, segments=None, **opts):
        return self._prim(
            "add_cone",
            {
                "base": list(base),
                "radius": radius,
                "height": height,
                "top_radius": top_radius,
                "axis": axis and list(axis),
                "segments": segments,
            },
            opts,
        )

    def prism(self, base, radius, height, sides=None, axis=None, direction=None, **opts):
        return self._prim(
            "add_prism",
            {
                "base": list(base),
                "radius": radius,
                "height": height,
                "sides": sides,
                "axis": axis and list(axis),
                "direction": direction and list(direction),
            },
            opts,
        )

    def sphere(self, center, radius, axis=None, segments=None, **opts):
        return self._prim(
            "add_sphere",
            {"center": list(center), "radius": radius, "axis": axis and list(axis), "segments": segments},
            opts,
        )

    def torus(self, center, radius, tube_radius, axis=None, segments=None, tube_segments=None, **opts):
        return self._prim(
            "add_torus",
            {
                "center": list(center),
                "radius": radius,
                "tube_radius": tube_radius,
                "axis": axis and list(axis),
                "segments": segments,
                "tube_segments": tube_segments,
            },
            opts,
        )

    # solid tools (M17c)

    def union(self, instances):
        """One new group that is the union of two or more solids; the operands are consumed."""
        return self._instance(self.client.execute({"type": "solid_union", "ids": _ids(instances)}))

    def subtract(self, target, tools):
        """``target`` minus each of ``tools`` (an instance or a list); the tools are consumed."""
        return self._instance(
            self.client.execute({"type": "solid_subtract", "target": _id(target), "tools": _ids(tools)})
        )

    def intersect(self, instances):
        """The common part of two or more solids."""
        return self._instance(self.client.execute({"type": "solid_intersect", "ids": _ids(instances)}))

    # groups, components, placing

    def group(self, instances, name=None):
        """Wrap instances (or any entity ids) in a new group."""
        cmd = {"type": "make_group", "ids": _ids(instances)}
        if name is not None:
            cmd["name"] = name
        return self._instance(self.client.execute(cmd))

    def component(self, instances, name):
        """Make a component from instances; the first instance placed is returned."""
        r = self.client.execute({"type": "make_component", "ids": _ids(instances), "name": name})
        return self._instance(r)

    def place(self, definition, transform=None):
        """Place another instance of ``definition`` (an id, or an ``Instance`` whose definition is
        used) at ``transform``."""
        d = definition.definition if isinstance(definition, Instance) else definition
        cmd = {"type": "place_instance", "definition": d}
        if transform is not None:
            cmd["transform"] = _tjson(transform)
        return self._instance(self.client.execute(cmd), d)

    def copy_along_path(self, instances, path, count=None, spacing=None, align=None):
        """Copy instances along the edges ``path`` (ids); returns the new instances."""
        cmd = {"type": "copy_along_path", "ids": _ids(instances), "path": _ids(path)}
        for k, v in (("count", count), ("spacing", spacing), ("align", align)):
            if v is not None:
                cmd[k] = v
        return self._instances(self.client.execute(cmd))

    # annotations, scenes, rendering

    def label(self, target, text, position=None):
        """A leader note pointing at ``target`` (face, edge, vertex or instance); returns the note id."""
        cmd = {"type": "add_label", "target": _id(target), "text": text}
        if position is not None:
            cmd["position"] = list(position)
        r = self.client.execute(cmd)
        return next((i for i in r["changes"]["created"] if i.startswith("n")), None)

    def scene(self, name, positions=None, **opts):
        """Add a scene storing the current view. ``positions``: instances whose transforms it
        stores (needs scene positions support). Returns the scene id."""
        cmd = {"type": "add_scene", "name": name}
        if positions is not None:
            cmd["positions"] = _ids(positions) if not isinstance(positions, bool) else positions
        cmd.update(opts)
        r = self.client.execute(cmd)
        return r["changes"]["scenes"][0]

    def render(self, path, width=1280, height=800, camera=None, lighting="studio", **opts):
        """Render to a PNG file at ``path``; returns ``path``. ``camera`` is ``{"eye": [...],
        "target": [...]}`` (more as ``set_camera`` takes); ``opts`` pass to ``render_view``."""
        if camera is not None:
            opts["camera"] = camera
        if lighting is not None:
            opts["lighting"] = lighting
        png = self.client.render_view(width, height, **opts)
        with open(path, "wb") as f:
            f.write(png)
        return path
