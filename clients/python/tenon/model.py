"""Higher-level helpers over ``Client``: materials, primitives, solids, sweeps, instances,
section cuts, labels, scenes, view settings, renders, joints, couplings, cam followers and solid
checks (interference, clearance, sweeps, mass, measure)."""
import math

from .transform import Transform, _cross, _norm, _sub, _vec3


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
    def __init__(self, id, name, density=None):
        self.id = id
        self.name = name
        self.density = density

    def __repr__(self):
        return "Material(%r, %r)" % (self.id, self.name)


class Joint:
    """A joint between two parts (or a part and the ground)."""

    def __init__(self, model, id, kind=None, name=None):
        self.model = model
        self.id = id
        self.kind = kind
        self.name = name

    def __repr__(self):
        return "Joint(%r)" % (self.id,)

    def drive(self, value, slide=None):
        """Set the joint's value (degrees for revolute and cylindrical joints, meters for
        prismatic ones; ``slide`` in meters for a cylindrical one) and move the connected parts."""
        cmd = {"type": "drive", "joint": self.id, "value": value}
        if slide is not None:
            cmd["slide"] = slide
        self.model.client.execute(cmd)
        return self

    @property
    def value(self):
        """The joint's current value, in the units ``drive`` takes."""
        for j in self.model.client.query("joints")["joints"]:
            if j["id"] == self.id:
                return j["value"]
        raise LookupError("no joint %s" % self.id)

    def delete(self):
        self.model.client.execute({"type": "delete_joint", "joint": self.id})


def _entity(e):
    """An entity for ``measure``: an id, an object with an id, or a path (a list of them)."""
    if isinstance(e, (list, tuple)):
        return [_id(x) for x in e]
    return _id(e)


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

    def transform(self):
        """The instance's current ``Transform`` (local to model coordinates)."""
        e = self.model.client.query("entities", ids=[self.id])["instances"]
        if not e:
            raise LookupError("no instance %s in the open context" % self.id)
        return Transform(e[0]["transform"])

    def apply(self, transform):
        """Apply ``transform`` (in model coordinates) on top of the current transform."""
        return self.set_transform(transform @ self.transform())

    def copy(self, offset, count=1):
        """``count`` copies, copy k moved by ``k * offset``; returns the new instances."""
        r = self.model.client.execute({"type": "copy", "ids": [self.id], "offset": list(offset), "count": count})
        return self.model._instances(r)

    def clone(self):
        """A copy in the same place (a group's copy gets a definition of its own, so a section
        plane or edit in it leaves this one alone)."""
        t = self.transform()
        (other,) = self.copy((0, 0, 1))
        return other.set_transform(t)

    def polar(self, axis, angle, copies, origin=(0, 0, 0)):
        """A radial array: ``copies`` copies, copy k turned by ``k * angle`` degrees about the line
        through ``origin`` along ``axis``; returns the new instances."""
        r = self.model.client.execute(
            {"type": "rotate", "ids": [self.id], "center": list(origin), "axis": list(axis), "angle": angle, "copies": copies}
        )
        return self.model._instances(r)

    def edges(self, where=None):
        """Ids of the edges of the instance's own geometry, optionally only those for which
        ``where(start, end)`` is true; points are in model coordinates."""
        c = self.model.client
        t = self.transform()
        c.execute({"type": "open_instance", "id": self.id})
        try:
            es = c.query("entities", kinds=["edge"])["edges"]
        finally:
            c.execute({"type": "close_instance"})
        if where is None:
            return [e["id"] for e in es]
        return [e["id"] for e in es if where(t.apply(e["start"]), t.apply(e["end"]))]

    def fillet(self, edges, radius, segments=None, radii=None):
        """Round ``edges`` (ids from ``edges()``, or ``"$N:e*"``-style references in a batch).
        ``radii`` optionally gives one radius per edge in place of ``radius``."""
        cmd = {"type": "fillet_edges", "instance": self.id, "edges": list(edges), "radius": radius}
        if segments is not None:
            cmd["segments"] = segments
        if radii is not None:
            cmd["radii"] = list(radii)
        self.model.client.execute(cmd)
        return self

    def chamfer(self, edges, distance, distances=None):
        """Bevel ``edges`` by ``distance`` (or ``distances``, one per edge)."""
        cmd = {"type": "chamfer_edges", "instance": self.id, "edges": list(edges), "distance": distance}
        if distances is not None:
            cmd["distances"] = list(distances)
        self.model.client.execute(cmd)
        return self

    def bevel(self, edges, sizes, kinds, segments=None):
        """Round some ``edges`` and chamfer others in one call. ``sizes`` (meters) and ``kinds``
        (``"round"`` or ``"chamfer"``) give one value per edge; ``segments`` is the faces per
        quarter turn of the rounds."""
        cmd = {
            "type": "bevel_edges",
            "instance": self.id,
            "edges": list(edges),
            "sizes": list(sizes),
            "kinds": list(kinds),
        }
        if segments is not None:
            cmd["segments"] = segments
        self.model.client.execute(cmd)
        return self

    def section(self, point, normal):
        """Add an active section plane inside this group or component that cuts only it. ``point``
        and ``normal`` are in model coordinates (converted to the instance's own); the side the
        normal points to is kept. Returns the plane id."""
        c = self.model.client
        t = self.transform()
        lp = t.inverse().apply(_vec3(point, "point"))
        # n . (L q + o - p) = 0 is (L^T n) . (q - q0) = 0: the local normal is L^T n
        m = t.m
        n = _vec3(normal, "normal")
        ln = tuple(m[3 * i] * n[0] + m[3 * i + 1] * n[1] + m[3 * i + 2] * n[2] for i in range(3))
        c.execute({"type": "open_instance", "id": self.id})
        try:
            r = c.execute({"type": "add_section_plane", "point": list(lp), "normal": list(ln)})
        finally:
            c.execute({"type": "close_instance"})
        return next(i for i in r["changes"]["created"] if i.startswith("s"))

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

    def _finish(self, cmds, group_refs, name=None, material=None):
        """Run ``cmds`` plus a ``make_group`` of ``group_refs`` (and a paint) as one batch;
        returns the group."""
        g = {"type": "make_group", "ids": group_refs}
        if name is not None:
            g["name"] = name
        cmds = list(cmds) + [g]
        if material is not None:
            cmds.append({"type": "paint", "ids": ["$%d:i0" % (len(cmds) - 1)], "material": _id(material)})
        r = self.client.batch(cmds)
        out = r["outcomes"][len(cmds) - (2 if material is not None else 1)]
        return Instance(self, out["placed"][0], (out.get("definitions") or [None])[0])

    def batch(self, atomic=True):
        """A batch builder (``with model.batch() as b:``); see ``Client.batch_builder``."""
        return self.client.batch_builder(atomic=atomic)

    # materials

    def material(self, name, color, metalness=0, roughness=1, density=None):
        """A named material, reused if it exists. ``color``: ``"#rrggbb"`` or (r, g, b) 0-255.
        ``metalness`` and ``roughness`` are 0 to 1 (the defaults send nothing). ``density`` in
        kg/m3 (1 to 30000) weighs solids painted with it for ``mass_properties``."""
        cmd = {"type": "add_material", "name": name, "color": _color(color), "existing": "reuse"}
        if metalness != 0 or roughness != 1:
            cmd["metalness"] = metalness
            cmd["roughness"] = roughness
            cmd["existing"] = "update"
        if density is not None:
            cmd["density"] = density
            cmd["existing"] = "update"
        r = self.client.execute(cmd)
        m = Material(r["changes"]["materials"][0], name, density)
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

    # sweeps and profiles

    def sweep(self, points, radius=None, inner_radius=None, segments=12, smooth=True, closed=False,
              profile=None, profile_x=(1, 0, 0), name=None, material=None):
        """Sweep a section along the polyline ``points`` with ``follow_me``; returns the new group.

        The section is a circle of ``radius`` (a pipe with ``inner_radius``), or ``profile``: a
        closed polygon of (a, b) offsets in the plane square to the first segment at the first
        point, ``a`` along ``profile_x`` (made square to the path) and ``b`` across. ``closed``
        joins the last point to the first (a loop such as a belt); ``smooth`` makes the bends soft.
        """
        pts = [list(_vec3(p, "a point")) for p in points]
        if len(pts) < 2:
            raise ValueError("a sweep needs at least 2 points")
        if closed:
            # start a loop halfway along its first edge: the section then lies across that run's
            # side faces and none of its edges is left over where the sweep closes
            mid = [(a + b) / 2 for a, b in zip(pts[0], pts[1])]
            pts = [mid] + pts[1:] + [pts[0]]
        t = _norm(_sub(pts[1], pts[0]))
        cmds = [{"type": "add_curve", "points": pts, "curve": smooth, "closed": closed}]
        keep = ["$1:*"]
        if profile is not None:
            px = _vec3(profile_x, "profile_x")
            d = sum(a * b for a, b in zip(px, t))
            u = _norm(tuple(px[i] - d * t[i] for i in range(3)))
            v = _cross(t, u)
            ring = [[pts[0][i] + a * u[i] + b * v[i] for i in range(3)] for a, b in profile]
            cmds.append({"type": "add_curve", "points": ring, "closed": True, "curve": False})
        else:
            if radius is None:
                raise ValueError("give radius or profile")
            cmds.append({"type": "add_circle", "center": pts[0], "radius": radius, "normal": list(t), "segments": segments})
            if inner_radius is not None:
                cmds.append({"type": "add_circle", "center": pts[0], "radius": inner_radius, "normal": list(t), "segments": segments})
                cmds.append({"type": "erase", "ids": ["$2:f0"]})
                keep.append("$2:e*")
        cmds.append({"type": "follow_me", "face": "$1:f0", "path": ["$0:e*"]})
        keep.append("$%d:*" % (len(cmds) - 1))
        cmds.append({"type": "erase", "ids": ["$0:e*"]})
        return self._finish(cmds, keep, name, material)

    def spring(self, base, axis, radius, wire_radius, pitch, turns, segments=24, wire_segments=8,
               start_direction=None, name=None, material=None):
        """A coil spring: a circle of ``wire_radius`` swept with ``follow_me`` along an
        ``add_helix`` of ``radius`` rising ``pitch`` per turn along ``axis`` from ``base``."""
        b = _vec3(base, "base")
        a = _norm(_vec3(axis, "axis"))
        if start_direction is None:
            start_direction = (0.0, 0.0, 1.0) if abs(a[2]) < 0.9 else (1.0, 0.0, 0.0)
        sd = _vec3(start_direction, "start_direction")
        u = _norm(_sub(sd, tuple(x * sum(p * q for p, q in zip(sd, a)) for x in a)))
        v = _cross(a, u)

        def at(theta):
            k = pitch * theta / (2 * math.pi)
            return tuple(b[i] + radius * (math.cos(theta) * u[i] + math.sin(theta) * v[i]) + k * a[i] for i in range(3))

        p0, p1 = at(0.0), at(2 * math.pi / segments)
        cmds = [
            {"type": "add_helix", "center": list(b), "axis": list(a), "radius": radius, "pitch": pitch, "turns": turns,
             "segments": segments, "start_direction": list(u)},
            {"type": "add_circle", "center": list(p0), "radius": wire_radius, "normal": list(_sub(p1, p0)), "segments": wire_segments},
            {"type": "follow_me", "face": "$1:f0", "path": ["$0:e*"]},
            {"type": "erase", "ids": ["$0:e*"]},
        ]
        return self._finish(cmds, ["$1:*", "$2:*"], name, material)

    def extrude(self, points, distance, name=None, material=None):
        """The flat closed polygon ``points`` pushed ``distance`` along its face normal (the
        canonical one: +Z, else +Y, else +X; negative goes the other way); returns the group."""
        pts = [list(_vec3(p, "a point")) for p in points]
        cmds = [
            {"type": "add_curve", "points": pts, "closed": True, "curve": False},
            {"type": "push_pull", "face": "$0:f0", "distance": distance},
        ]
        return self._finish(cmds, ["$0:*", "$1:*"], name, material)

    def revolve(self, points, center, axis, angle=360, segments=24, name=None, material=None):
        """The flat closed polygon ``points`` turned about the line through ``center`` along
        ``axis`` (a lathe); returns the group."""
        pts = [list(_vec3(p, "a point")) for p in points]
        cmds = [
            {"type": "add_curve", "points": pts, "closed": True, "curve": False},
            {"type": "revolve", "face": "$0:f0", "center": list(center), "axis": list(axis), "angle": angle, "segments": segments},
        ]
        return self._finish(cmds, ["$0:*", "$1:*"], name, material)

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

    def section_cut(self, instances, cut=False):
        """Whether section planes outside ``instances`` cut them (``False`` keeps moving parts
        whole in a cutaway)."""
        self.client.execute({"type": "set_section_cut", "ids": _ids(instances), "cut": cut})

    def explode_view(self, instances, factor, center=None):
        """Spread ``instances`` apart by ``factor`` of their offsets from ``center``."""
        cmd = {"type": "explode_view", "ids": _ids(instances), "factor": factor}
        if center is not None:
            cmd["center"] = list(center)
        self.client.execute(cmd)

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

    def go_to_scene(self, scene):
        self.client.execute({"type": "go_to_scene", "scene": scene})

    def transition(self, on=True, seconds=None):
        """How the window animates going to a scene."""
        cmd = {"type": "set_scene_transition", "on": on}
        if seconds is not None:
            cmd["seconds"] = seconds
        self.client.execute(cmd)

    # view settings

    def camera(self, eye, target, fov_y_deg=None, projection=None):
        cmd = {"type": "set_camera", "eye": list(eye), "target": list(target)}
        if fov_y_deg is not None:
            cmd["fov_y_deg"] = fov_y_deg
        if projection is not None:
            cmd["projection"] = projection
        self.client.execute(cmd)

    def lighting(self, mode):
        """``day``, ``night`` or ``studio``."""
        self.client.execute({"type": "set_lighting", "mode": mode})

    def section_display(self, **fields):
        """``planes``, ``cuts``, ``fill`` (booleans) and ``fill_color`` (``dark`` or ``material``)."""
        cmd = {"type": "set_section_display"}
        cmd.update(fields)
        self.client.execute(cmd)

    def style(self, name, use=True, **settings):
        """Add a style with ``settings`` (``edges``, ``background``, ``sky``, ``ground``, ``fog``,
        ``ambient_occlusion``) and, with ``use``, draw with it; returns the style id."""
        r = self.client.batch(
            [{"type": "add_style", "name": name}, {"type": "change_style", "style": "$0:y0", "settings": settings}]
        )
        sid = r["outcomes"][0]["changes"]["styles"][0]
        if use:
            self.client.execute({"type": "set_style", "style": sid})
        return sid

    def render(self, path, width=1280, height=800, camera=None, lighting="studio", joints=None,
               ground=True, **opts):
        """Render to a PNG file at ``path``; returns ``path``. ``camera`` is ``{"eye": [...],
        "target": [...]}`` (more as ``set_camera`` takes); ``joints`` is a dict of joint (or
        ``Joint``) to value to render at, solved on a copy; ``ground=False`` leaves out the
        ground grid, axes and ground shadows; ``opts`` pass to ``render_view`` (``scene``, ``t``,
        ``from_`` for ``from``, ``supersample``, ``section``, ``style``...)."""
        if joints:
            opts["joints"] = {_id(k): v for k, v in joints.items()}
        if not ground:
            opts["ground"] = False
        if "from_" in opts:
            opts["from"] = opts.pop("from_")
        if camera is not None:
            opts["camera"] = camera
        if lighting is not None:
            opts["lighting"] = lighting
        png = self.client.render_view(width, height, **opts)
        with open(path, "wb") as f:
            f.write(png)
        return path

    # joints, couplings, cam followers

    def joint(self, kind, a, b, point, axis=None, reference=None, name=None, limits=None):
        """Join parts ``a`` and ``b`` (instances of the root; ``a`` None is the ground) with a
        ``kind`` joint (``revolute``, ``prismatic``, ``cylindrical``, ``fixed`` or ``ball``) at
        ``point`` along ``axis``. Returns the ``Joint``."""
        cmd = {"type": "add_joint", "kind": kind, "a": None if a is None else _id(a), "b": _id(b),
               "point": list(point)}
        for k, v in (("axis", axis), ("reference", reference)):
            if v is not None:
                cmd[k] = list(v)
        if name is not None:
            cmd["name"] = name
        if limits is not None:
            cmd["limits"] = list(limits)
        r = self.client.execute(cmd)
        return Joint(self, r["changes"]["joints"][0], kind, name)

    def couple(self, driver, driven, ratio, offset=None):
        """Hold ``driven = ratio * driver + offset`` (joint values in their own units); returns
        the coupling id."""
        cmd = {"type": "add_coupling", "driver": _id(driver), "driven": _id(driven), "ratio": ratio}
        if offset is not None:
            cmd["offset"] = offset
        return self.client.execute(cmd)["changes"]["couplings"][0]

    def cam_follower(self, cam, follower_joint, roller_radius=0, preload_direction=None):
        """Make the prismatic ``follower_joint`` follow ``cam`` (an instance); returns the id."""
        cmd = {"type": "add_cam_follower", "cam": _id(cam), "follower": _id(follower_joint),
               "roller_radius": roller_radius}
        if preload_direction is not None:
            cmd["preload_direction"] = list(preload_direction)
        return self.client.execute(cmd)["changes"]["cam_followers"][0]

    # solid checks

    def interference(self, ids=None, touching=False):
        """Which solid parts overlap (``pairs``), with ``touching`` also those that meet."""
        q = {}
        if ids is not None:
            q["ids"] = _ids(ids)
        if touching:
            q["touching"] = True
        return self.client.query("interference", **q)

    def clearance(self, a=None, b=None, ids=None):
        """The minimum distance between parts ``a`` and ``b``, or between every pair among
        ``ids``; returns the reply (``pairs`` with ``distance`` and the nearest points)."""
        q = {}
        if a is not None:
            q["a"] = _id(a)
        if b is not None:
            q["b"] = _id(b)
        if ids is not None:
            q["ids"] = _ids(ids)
        return self.client.query("clearance", **q)

    def sweep_check(self, joint, from_, to, steps, ids=None, slide=None, report_within=None):
        """Turn ``joint`` from ``from_`` to ``to`` in ``steps`` on a copy of the model and report
        the pairs that hit or come within ``report_within`` meters."""
        q = {"joint": _id(joint), "from": from_, "to": to, "steps": steps}
        if ids is not None:
            q["ids"] = _ids(ids)
        if slide is not None:
            q["slide"] = slide
        if report_within is not None:
            q["report_within"] = report_within
        return self.client.query("sweep_check", **q)

    def model_health(self, ids=None):
        """Which parts (``ids``, every group and component when None) are not solids and why:
        the reply's ``parts``, each with ``fixable`` and ``problems``."""
        q = {}
        if ids is not None:
            q["ids"] = _ids(ids)
        return self.client.query("model_health", **q)["parts"]

    def make_solid(self, ids=None):
        """Close the parts ``ids`` (every group and component when None) where that is safe, as
        one undo step; returns the report (``entries`` per definition that was not a solid,
        ``solid`` the count that were)."""
        cmd = {"type": "make_solid"}
        if ids is not None:
            cmd["ids"] = _ids(ids)
        return self.client.execute(cmd)["make_solid"]

    def mass_properties(self, ids=None):
        """Volume, mass, centre of gravity and inertia of ``ids`` (every instance when None)."""
        q = {}
        if ids is not None:
            q["ids"] = _ids(ids)
        return self.client.query("mass_properties", **q)

    def cut_list(self, ids=None, group_by=None):
        """The parts of the open context as a cut list: ``rows`` of name, material, length, width,
        thickness (meters, the part's own box, longest first), quantity, solid and ids.
        ``group_by`` is ``"definition"`` (default) or ``"none"``."""
        q = {}
        if ids is not None:
            q["ids"] = _ids(ids)
        if group_by is not None:
            q["group_by"] = group_by
        return self.client.query("cut_list", **q)["rows"]

    def measure(self, distance=None, angle=None, round=None):
        """Exactly one of ``distance`` or ``angle`` (two entities each) or ``round`` (one):
        returns the measurement dict (``distance``, ``angle`` or ``round`` of the reply). An
        entity is an id (or an object with one) or a path of instance ids ending in the entity."""
        given = [(k, v) for k, v in (("distance", distance), ("angle", angle), ("round", round)) if v is not None]
        if len(given) != 1:
            raise ValueError("give exactly one of distance, angle or round")
        k, v = given[0]
        v = _entity(v) if k == "round" else [_entity(x) for x in v]
        return self.client.query("measure", **{k: v})[k]
