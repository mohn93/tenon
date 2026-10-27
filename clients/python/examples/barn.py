#!/usr/bin/env python3
"""A pegged oak timber-frame barn with a sliding door, partly clad and roofed, drawn four ways.

    python3 barn.py [--socket PATH] [--out DIR] [--quick]

Without ``--socket`` it starts a headless app. It writes ``barn.png`` (a three-quarter view in
sunlight, sun shadows and ambient occlusion), ``barn-joint.png`` (a post, plate, tie beam and
knee brace pulled apart to show the mortises, tenons and pegs), ``barn-section.png`` (a section
through a bent with the cuts filled in the oak's own colour), ``barn-sketch.png`` (a pencil
elevation) and ``barn-cutlist.txt`` (the timber schedule: ``cut_list`` rows grouped by member,
and the oak frame's volume and mass from ``mass_properties``) to ``--out``.

What it shows of the API:

- a heavy-timber frame of 8 posts, 2 plates, 4 tie beams, 4 sills, 20 knee braces, girts, queen
  posts, collars, crown posts, a ridge beam, principal rafters and purlins, joined with real
  joinery: every tenon is a box fused on with ``solid_union`` and every mortise and peg hole is
  a tool taken out with ``solid_subtract``, each member's whole pipeline in one batch (about 1,200
  commands in 11 batches, a second or so against a release app); about 100 oak pegs sit in the
  bored holes;
- ``chamfer_edges`` on every exposed timber, on loose extruded profiles (the sloped knee braces,
  queen post seats and principal rafters with birdsmouths) and on boxes;
- batch references (``$7:i0``, ``$9:e*``) so a member's whole pipeline is one round trip;
- ``copy`` for the common rafters, boards, battens and roof planks, ``rotate`` for sloped plates;
- a foundation of stone piers and walls, a plank roof and standing-seam metal over part of one
  slope, a board-and-batten gable and side bay, a sliding door on a steel track, a workbench with
  hand tools; wood materials (one oak tone per member type, with densities) and metal ones;
- ``set_location`` and ``set_shadows`` for the sun, ambient occlusion in a style, ``explode_view``
  and labels for the joint, a section plane with ``fill_color: material`` and a parallel view,
  a sketchy-edges style, ``cut_list`` and ``mass_properties`` queries.

Lengths are meters. X runs along the ridge (12 m), Y across (8 m, the front is y = 0), Z up.
The frame stands on a stone foundation: sill tops at z = 0.75, plates at z = 4.0, tie beams at
z = 4.25 to 4.55, the ridge at z = 7.5. Bents (the cross frames) stand at x = 0.3, 4.1, 7.9, 11.7.
"""
import argparse
import math
import os
import sys
import time

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))

from tenon import Model, session  # noqa: E402
from tenon.model import Instance  # noqa: E402

# ---------------------------------------------------------------- dimensions (meters)

XB = (0.3, 4.1, 7.9, 11.7)  # bent positions
LEN, WID = 12.0, 8.0
PYC = (0.15, 7.85)  # post center lines (front, back)
POST = 0.26  # post section
Z_FLOOR = 0.45
Z_SILL = 0.5  # sill bottom
Z_SILL_TOP = 0.75
Z_PLATE = 4.0  # post top, plate bottom
Z_TIE = 4.25  # tie beam bottom (plate top)
Z_SEAT = 4.55  # tie beam top: where the principal rafters sit
PITCH = 0.62  # tan of the roof pitch (31.8 degrees)
THETA = math.degrees(math.atan(PITCH))
COS = 1.0 / math.sqrt(1.0 + PITCH * PITCH)
SIN = PITCH * COS
Y_SEAT = 0.05  # where the principal's underside meets the seat
DV = 0.24  # vertical depth of the principal rafter
Y_RIDGE0 = 3.92  # principals and common rafters stop at the ridge beam's faces
Y_QUEEN = 2.5
PEG_R = 0.019
HOLE_R = 0.0205
OAK, PINE, STEEL = 700, 520, 7850  # kg/m3


def zb(y):
    """Underside of a principal rafter on the front slope (y from 0 to 4)."""
    return Z_SEAT + PITCH * (y - Y_SEAT)


def zt(y):
    return zb(y) + DV


def zroof(y, n):
    """The roof surface ``n`` meters above the principals' tops, at y on the front slope."""
    return zt(y) + n / COS


def mir(y):
    return WID - y


def zt_any(y):
    return zt(y if y <= WID / 2 else mir(y))


# ---------------------------------------------------------------- joinery helpers


class Spec:
    """A member to build: a box or an extruded profile, plus tenons (boxes fused on) and cuts
    (mortises and peg holes, tools taken out)."""

    def __init__(self, name, mat, base, chamfer=0.012):
        self.name = name
        self.mat = mat
        self.base = base  # ("box", corner, size) or ("poly", points, distance)
        self.chamfer = chamfer
        self.tenons = []  # (corner, size)
        self.cuts = []  # ("box", corner, size) or ("cyl", base, axis, radius, height)
        self.id = None


def _axis(d):
    k = max(range(3), key=lambda i: abs(d[i]))
    return k, (1 if d[k] > 0 else -1)


def _box_along(P, d, a, b, half):
    """A box around P with half sizes ``half`` across ``d`` and from ``a`` to ``b`` along it."""
    k, s = _axis(d)
    lo = [P[i] - half[i] for i in range(3)]
    hi = [P[i] + half[i] for i in range(3)]
    p, q = P[k] + s * a, P[k] + s * b
    lo[k], hi[k] = min(p, q), max(p, q)
    return lo, [hi[i] - lo[i] for i in range(3)]


def join(a, b, P, d, length, half, back=0.02):
    """A tenon on member ``a`` and its mortise in ``b``. ``P`` is the tenon's center on the
    shoulder plane, ``d`` the axis-aligned direction it points in (into ``b``), ``half`` its half
    sizes across ``d``."""
    corner, size = _box_along(P, d, -back, length, half)
    a.tenons.append((corner, size))
    clear = [h + 0.003 for h in half]
    corner, size = _box_along(P, d, -0.04, length + 0.01, clear)
    b.cuts.append(("box", corner, size))


def peg(pegs, center, axis, length, *members):
    """An oak peg through ``members`` (their holes are bored for it) along ``axis``."""
    for m in members:
        base = [center[i] - axis[i] * (length / 2 + 0.01) for i in range(3)]
        m.cuts.append(("cyl", base, axis, HOLE_R, length + 0.02))
    pegs.append((center, axis, length))


class Builder:
    """Collects commands, sends them in batches, and resolves the instance each member made."""

    def __init__(self, client, off=(0.0, 0.0, 0.0)):
        self.c = client
        self.off = tuple(off)
        self.cmds = []
        self.pending = []

    def P(self, p):
        return [round(p[i] + self.off[i], 6) for i in range(3)]

    def add(self, **cmd):
        self.cmds.append(cmd)
        return len(self.cmds) - 1

    def box(self, corner, size, **kw):
        return self.add(type="add_box", corner=self.P(corner), size=[round(s, 6) for s in size], **kw)

    def cyl(self, base, radius, height, axis=None, segments=None, **kw):
        c = dict(type="add_cylinder", base=self.P(base), radius=radius, height=height, **kw)
        if axis is not None:
            c["axis"] = list(axis)
        if segments is not None:
            c["segments"] = segments
        return self.add(**c)

    def flush(self):
        if not self.cmds:
            return
        try:
            r = self.c.batch(self.cmds)
        except Exception as e:
            idx = getattr(e, "index", None)
            if idx is not None:
                for j in range(max(0, idx - 8), idx + 1):
                    print("  %d: %s" % (j, self.cmds[j]))
            raise
        for obj, idx in self.pending:
            obj.id = r["outcomes"][idx]["placed"][0]
        self.cmds, self.pending = [], []

    def poly(self, name, mat, points, distance, chamfer=0.0):
        """An extruded profile as a group; returns the command index of the group."""
        a = self.add(type="add_curve", points=[self.P(p) for p in points], closed=True, curve=False)
        b = self.add(type="push_pull", face="$%d:f0" % a, distance=distance)
        ids = ["$%d:*" % a, "$%d:*" % b]
        if chamfer:
            ch = self.add(type="chamfer_edges", edges=["$%d:e*" % a, "$%d:e*" % b], distance=chamfer)
            ids.append("$%d:*" % ch)
        g = self.add(type="make_group", ids=ids, name=name)
        self.add(type="paint", ids=["$%d:i0" % g], material=mat.id)
        return g

    def emit(self, s):
        """The member's whole pipeline: base, chamfer, tenons fused on, mortises and holes cut."""
        if len(self.cmds) > 260:
            self.flush()
        if s.base[0] == "box":
            i = self.box(s.base[1], s.base[2], name=s.name, material=s.mat.id)
            if s.chamfer:
                self.add(type="chamfer_edges", instance="$%d:i0" % i, edges=["$%d:e*" % i], distance=s.chamfer)
        else:
            i = self.poly(s.name, s.mat, s.base[1], s.base[2], s.chamfer)
        cur = i
        if s.tenons:
            ids = ["$%d:i0" % cur] + ["$%d:i0" % self.box(c, z) for c, z in s.tenons]
            cur = self.add(type="solid_union", ids=ids)
        if s.cuts:
            tools = []
            for cut in s.cuts:
                if cut[0] == "box":
                    tools.append("$%d:i0" % self.box(cut[1], cut[2]))
                else:
                    tools.append("$%d:i0" % self.cyl(cut[1], cut[3], cut[4], axis=cut[2], segments=10))
            cur = self.add(type="solid_subtract", target="$%d:i0" % cur, tools=tools)
        self.pending.append((s, cur))
        return s

    def emit_pegs(self, pegs, mat):
        for center, axis, length in pegs:
            if len(self.cmds) > 260:
                self.flush()
            base = [center[i] - axis[i] * length / 2 for i in range(3)]
            o = type("Peg", (), {})()
            o.cut = None
            i = self.cyl(base, PEG_R, length, axis=axis, segments=10, name="Oak peg", material=mat.id)
            self.pending.append((o, i))
            yield o


# ---------------------------------------------------------------- the frame


class Barn:
    def __init__(self, c, m, quick=False):
        self.c, self.m, self.quick = c, m, quick
        self.b = Builder(c)
        self.frame = []  # the oak frame's members and pegs (objects with an id)
        self.pegs = []
        self.by = {}  # name -> Specs

    # materials ---------------------------------------------------------

    def materials(self):
        m = self.m
        oak = lambda n, c: m.material(n, c, roughness=0.8, density=OAK)  # noqa: E731
        self.mt = {
            "post": oak("Oak post", (178, 128, 72)),
            "sill": oak("Oak sill", (150, 108, 62)),
            "plate": oak("Oak plate", (170, 120, 68)),
            "tie": oak("Oak tie beam", (164, 114, 62)),
            "brace": oak("Oak brace", (192, 142, 84)),
            "girt": oak("Oak girt", (184, 134, 78)),
            "truss": oak("Oak truss", (172, 122, 70)),
            "rafter": oak("Oak rafter", (186, 136, 80)),
            "ridge": oak("Oak ridge", (198, 150, 92)),
            "peg": oak("Oak peg", (104, 66, 36)),
            "bench": oak("Bench oak", (208, 164, 104)),
            "common": m.material("Pine rafter", (214, 176, 118), roughness=0.85, density=PINE),
            "plank": m.material("Pine sheathing", (222, 190, 140), roughness=0.9, density=PINE),
            "red": m.material("Barn red", (140, 48, 38), roughness=0.85),
            "batten": m.material("Barn red dark", (112, 36, 30), roughness=0.85),
            "cream": m.material("Door cream", (226, 218, 192), roughness=0.8),
            "metal": m.material("Standing seam", (176, 182, 190), metalness=0.6, roughness=0.42, density=STEEL),
            "seam": m.material("Seam cap", (150, 156, 164), metalness=0.6, roughness=0.4, density=STEEL),
            "steel": m.material("Black steel", (52, 54, 58), metalness=0.8, roughness=0.45, density=STEEL),
            "blade": m.material("Tool steel", (198, 202, 210), metalness=1.0, roughness=0.25, density=STEEL),
            "stone": m.material("Fieldstone", (132, 126, 116), roughness=0.95),
            "concrete": m.material("Concrete", (178, 176, 170), roughness=0.95),
            "grass": m.material("Lawn", (104, 142, 76), roughness=1),
            "gravel": m.material("Gravel", (190, 178, 152), roughness=1),
            "earth": m.material("Packed earth", (140, 116, 88), roughness=1),
            "trunk": m.material("Bark", (92, 66, 46), roughness=1),
            "leaf": m.material("Leaves", (66, 118, 62), roughness=1),
            "leaf2": m.material("Leaves 2", (92, 140, 66), roughness=1),
            "handle": m.material("Ash handle", (196, 160, 110), roughness=0.7, density=650),
            "hay": m.material("Hay", (214, 182, 90), roughness=1),
        }

    # helpers -----------------------------------------------------------

    def spec(self, key, name, mat, base, chamfer=0.012, frame=True):
        s = Spec(name, self.mt[mat], base, chamfer)
        self.by.setdefault(key, []).append(s)
        if frame:
            self.frame.append(s)
        return s

    def beam(self, key, name, mat, corner, size, chamfer=0.012):
        return self.spec(key, name, mat, ("box", corner, size), chamfer)

    # the members -------------------------------------------------------

    def define(self):
        """Every member of the frame as a Spec, joined by tenons, mortises and pegs."""
        pegs = self.pegs
        sills, plates = [], []
        for y0 in (0.0, WID - 0.3):
            sills.append(self.beam("sill", "Sill", "sill", (0.17, y0, Z_SILL), (LEN - 0.34, 0.30, 0.25), 0.015))
        for yc in PYC:
            plates.append(self.beam("plate", "Wall plate", "plate", (0.17, yc - 0.12, Z_PLATE), (LEN - 0.34, 0.24, 0.25), 0.014))
        # end sills between the long sills, through-tenoned (end grain shows at the outer face)
        for xb in (XB[0], XB[-1]):
            e = self.beam("endsill", "End sill", "sill", (xb - 0.125, 0.30, Z_SILL), (0.25, WID - 0.60, 0.25), 0.015)
            join(e, sills[0], (xb, 0.30, Z_SILL + 0.125), (0, -1, 0), 0.07, (0.05, 0, 0.06))
            join(e, sills[1], (xb, WID - 0.30, Z_SILL + 0.125), (0, 1, 0), 0.07, (0.05, 0, 0.06))
        posts = {}
        self.principals = {}
        crowns = []
        for bi, xb in enumerate(XB):
            tie = self.beam("tie", "Tie beam", "tie", (xb - 0.11, -0.15, Z_TIE), (0.22, WID + 0.30, 0.30), 0.014)
            queens = []
            for si, yc in enumerate(PYC):
                sgn = 1 if si == 0 else -1  # toward the barn's middle
                p = self.beam("post", "Post", "post", (xb - 0.13, yc - 0.13, Z_SILL_TOP), (POST, POST, Z_PLATE - Z_SILL_TOP), 0.014)
                posts[bi, si] = p
                # tenon down into the sill, tenon up into the plate, a peg through each
                join(p, sills[si], (xb, yc, Z_SILL_TOP), (0, 0, -1), 0.15, (0.05, 0.05, 0))
                join(p, plates[si], (xb, yc, Z_PLATE), (0, 0, 1), 0.17, (0.05, 0.05, 0))
                peg(pegs, (xb, yc, Z_SILL_TOP - 0.07), (0, 1, 0), 0.40, p, sills[si])
                peg(pegs, (xb, yc, Z_PLATE + 0.09), (0, 1, 0), 0.34, p, plates[si])
                # knee brace across the bent, post to tie beam
                y_f = yc + sgn * 0.13  # the post's inner face
                pts = [(y_f, 3.05), (y_f, 3.22), (y_f + sgn * 1.03, Z_TIE), (y_f + sgn * 1.20, Z_TIE)]
                br = self.spec("kbrace", "Knee brace (bent)", "brace", ("poly", [(xb - 0.06, y, z) for y, z in pts], 0.12), 0.01)
                join(br, p, (xb, y_f, 3.135), (0, -sgn, 0), 0.12, (0.045, 0, 0.045))
                yt = y_f + sgn * 1.115
                join(br, tie, (xb, yt, Z_TIE), (0, 0, 1), 0.12, (0.045, 0.045, 0))
                peg(pegs, (xb, y_f - sgn * 0.06, 3.135), (1, 0, 0), 0.34, p, br)
                peg(pegs, (xb, yt, Z_TIE + 0.075), (1, 0, 0), 0.30, tie, br)
                # queen post on the tie beam, its top cut to the rafter's underside
                yq = Y_QUEEN if si == 0 else mir(Y_QUEEN)
                qp = [(yq - 0.1, Z_SEAT), (yq + 0.1, Z_SEAT)]
                for yy in (yq + 0.1, yq - 0.1):
                    qp.append((yy, zb(yy if si == 0 else mir(yy))))
                q = self.spec("queen", "Queen post", "truss", ("poly", [(xb - 0.1, y, z) for y, z in qp], 0.2), 0.012)
                join(q, tie, (xb, yq, Z_SEAT), (0, 0, -1), 0.12, (0.045, 0.045, 0))
                peg(pegs, (xb, yq, Z_SEAT - 0.07), (1, 0, 0), 0.30, tie, q)
                queens.append((q, yq))
                self._principal(bi, xb, si, tie)
            # collar between the queen posts, crown post up to the ridge
            (q0, y0), (q1, y1) = queens
            collar = self.beam("collar", "Collar", "truss", (xb - 0.08, y0 + 0.1, 5.45), (0.16, y1 - y0 - 0.2, 0.25), 0.012)
            join(collar, q0, (xb, y0 + 0.1, 5.575), (0, -1, 0), 0.12, (0.045, 0, 0.055))
            join(collar, q1, (xb, y1 - 0.1, 5.575), (0, 1, 0), 0.12, (0.045, 0, 0.055))
            peg(pegs, (xb, y0 + 0.04, 5.575), (1, 0, 0), 0.28, q0, collar)
            peg(pegs, (xb, y1 - 0.04, 5.575), (1, 0, 0), 0.28, q1, collar)
            crown = self.beam("crown", "Crown post", "truss", (xb - 0.08, WID / 2 - 0.08, 5.70),
                              (0.16, 0.16, zb(Y_RIDGE0) - 5.70), 0.012)
            join(crown, collar, (xb, WID / 2, 5.70), (0, 0, -1), 0.12, (0.045, 0.045, 0))
            peg(pegs, (xb, WID / 2, 5.62), (1, 0, 0), 0.24, collar, crown)
            crowns.append(crown)
        # ridge beam: the crown posts are tenoned into it, one peg ties it to both principals
        ridge = self.beam("ridge", "Ridge beam", "ridge", (-0.3, Y_RIDGE0, zb(Y_RIDGE0)),
                          (LEN + 0.6, WID - 2 * Y_RIDGE0, zroof(Y_RIDGE0, 0.25) - zb(Y_RIDGE0)), 0.014)
        for bi, xb in enumerate(XB):
            join(crowns[bi], ridge, (xb, WID / 2, zb(Y_RIDGE0)), (0, 0, 1), 0.15, (0.045, 0.045, 0))
            peg(pegs, (xb, WID / 2, zb(Y_RIDGE0) + 0.09), (0, 1, 0), 0.24, ridge, crowns[bi])
            peg(pegs, (xb, WID / 2, zb(Y_RIDGE0) + 0.185), (0, 1, 0), 0.50, ridge, *self.principals[bi])
        for side in (0, 1):
            for ps in (1.45, 3.20):
                sp = self.spec("purlin", "Purlin", "ridge", ("box", (0, 0, 0), (LEN + 0.6, 0.14, 0.14)), 0.01)
                sp.slope = (side, ps)
        # wall knee braces (post to plate) and girts, bay by bay
        for si, yc in enumerate(PYC):
            for bi in range(len(XB) - 1):
                xa, xc = XB[bi], XB[bi + 1]
                for k, (xb, sgn) in enumerate(((xa, 1), (xc, -1))):
                    p = posts[bi + k, si]
                    xf = xb + sgn * 0.13
                    pts = [(xf, 3.15), (xf, 3.32), (xf + sgn * 0.68, Z_PLATE), (xf + sgn * 0.85, Z_PLATE)]
                    br = self.spec("wbrace", "Knee brace (wall)", "brace", ("poly", [(x, yc - 0.06, z) for x, z in pts], 0.12), 0.01)
                    join(br, p, (xf, yc, 3.235), (-sgn, 0, 0), 0.12, (0, 0.045, 0.045))
                    join(br, plates[si], (xf + sgn * 0.765, yc, Z_PLATE), (0, 0, 1), 0.12, (0.05, 0.045, 0))
                    peg(pegs, (xf - sgn * 0.06, yc, 3.235), (0, 1, 0), 0.34, p, br)
                    peg(pegs, (xf + sgn * 0.765, yc, Z_PLATE + 0.075), (0, 1, 0), 0.34, plates[si], br)
                if si == 0 and bi == 1:
                    continue  # the door bay has no front girt
                g = self.beam("girt", "Girt", "girt", (xa + 0.13, yc - 0.07, 2.2), (xc - xa - 0.26, 0.14, 0.22), 0.012)
                for k, (xb, sgn) in enumerate(((xa, 1), (xc, -1))):
                    p = posts[bi + k, si]
                    xf = xb + sgn * 0.13
                    join(g, p, (xf, yc, 2.31), (-sgn, 0, 0), 0.12, (0, 0.045, 0.055))
                    peg(pegs, (xf - sgn * 0.06, yc, 2.31), (0, 1, 0), 0.34, p, g)

    def _principal(self, bi, xb, si, tie):
        """A principal rafter: tail, birdsmouth on the tie beam, ridge end, all in the YZ plane."""
        f = (lambda y: y) if si == 0 else mir
        y_t, y_h, y_s, y_r = -0.55, -0.15, Y_SEAT, Y_RIDGE0
        pts = [(y_t, zt(y_t)), (y_r, zt(y_r)), (y_r, zb(y_r)), (y_s, Z_SEAT), (y_h, Z_SEAT), (y_h, zb(y_h)), (y_t, zb(y_t))]
        s = self.spec("principal", "Principal rafter", "rafter", ("poly", [(xb - 0.08, f(y), z) for y, z in pts], 0.16), 0.0)
        self.principals.setdefault(bi, []).append(s)
        lo, hi = Z_SEAT - 0.25, zt(-0.05) + 0.03
        peg(self.pegs, (xb, f(-0.05), (lo + hi) / 2), (0, 0, 1), hi - lo, tie, s)

    # building ----------------------------------------------------------

    def slope_box(self, x0, lx, s0, ls, n0, ln, mat, name, side=0, chamfer=0.0):
        """A box on the roof plane: ``s0`` to ``s0 + ls`` up the slope from the principals' seat,
        ``n0`` to ``n0 + ln`` above their tops, ``lx`` long from ``x0``; made flat, then turned."""
        b = self.b
        org_y, z0 = (Y_SEAT if side == 0 else mir(Y_SEAT)), zt(Y_SEAT)
        y = org_y + (s0 if side == 0 else -(s0 + ls))
        i = b.box((x0, y, z0 + n0), (lx, ls, ln), name=name, material=mat.id)
        if chamfer:
            b.add(type="chamfer_edges", instance="$%d:i0" % i, edges=["$%d:e*" % i], distance=chamfer)
        b.add(type="rotate", ids=["$%d:i0" % i], center=b.P((0, org_y, z0)), axis=[1, 0, 0],
              angle=THETA if side == 0 else -THETA)
        return i

    def build_frame(self):
        b = self.b
        self.define()
        for specs in self.by.values():
            for s in specs:
                if not hasattr(s, "slope"):
                    b.emit(s)
        b.flush()
        for p in self.by["purlin"]:
            side, s_along = p.slope
            i = self.slope_box(-0.3, LEN + 0.6, s_along - 0.07, 0.14, 0.0, 0.14, p.mat, "Purlin", side, 0.01)
            b.pending.append((p, i))
        b.flush()
        for o in b.emit_pegs(self.pegs, self.mt["peg"]):
            self.frame.append(o)
        b.flush()

    def roof_and_walls(self):
        """The common rafters, planks, metal, cladding and door: plain batches."""
        b, mt = self.b, self.mt
        # common rafters: one profile per slope, copied along the ridge
        n0, n1 = 0.14, 0.25
        for side in (0, 1):
            f = (lambda y: y) if side == 0 else mir
            y_t = -0.75
            pts = [(y_t, zroof(y_t, n1)), (Y_RIDGE0, zroof(Y_RIDGE0, n1)), (Y_RIDGE0, zroof(Y_RIDGE0, n0)), (y_t, zroof(y_t, n0))]
            g = b.poly("Common rafter", mt["common"], [(-0.295, f(y), z) for y, z in pts], 0.09)
            b.add(type="copy", ids=["$%d:i0" % g], offset=[0.5, 0, 0], count=25)
        # roof planks: every row over bay 3 and the end overhang, the lower rows over bay 2
        s_lo, s_hi, step = -0.80, 4.78, 0.155
        rows = int((s_hi - s_lo) / step)
        i = self.slope_box(8.0, 4.3, s_lo, step - 0.006, 0.25, 0.022, mt["plank"], "Roof plank")
        b.add(type="copy", ids=["$%d:i0" % i], offset=[0, step * COS, step * SIN], count=rows)
        i = self.slope_box(4.0, 4.0, s_lo, step - 0.006, 0.25, 0.022, mt["plank"], "Roof plank")
        b.add(type="copy", ids=["$%d:i0" % i], offset=[0, step * COS, step * SIN], count=int((2.9 - s_lo) / step))
        # standing-seam metal: 12 pans over bay 3 and the three before it, with a seam between each
        pan, npan = 0.46, 12
        xm = 8.1 - 3 * pan
        i = self.slope_box(xm, pan - 0.03, s_lo - 0.02, s_hi - s_lo + 0.02, 0.272, 0.014, mt["metal"], "Metal panel")
        b.add(type="copy", ids=["$%d:i0" % i], offset=[pan, 0, 0], count=npan - 1)
        i = self.slope_box(xm - 0.03, 0.03, s_lo - 0.02, s_hi - s_lo + 0.02, 0.272, 0.04, mt["seam"], "Seam")
        b.add(type="copy", ids=["$%d:i0" % i], offset=[pan, 0, 0], count=npan)
        chain = [(WID / 2 + d, zroof(WID / 2 - abs(d), 0.29)) for d in (-0.38, 0.0, 0.38)]
        cap = chain + [(y, z + 0.04) for y, z in reversed(chain)]
        b.poly("Ridge cap", mt["seam"], [(xm - 0.03, y, z) for y, z in cap], npan * pan + 0.03)
        # gable-end cladding: boards (profiles with the roofline cut in) and battens
        xo = XB[-1] + 0.13
        for k in range(40):
            ya, yb = 0.2 * k + 0.002, 0.2 * (k + 1) - 0.002
            pts = [(ya, Z_SILL_TOP), (yb, Z_SILL_TOP), (yb, zt_any(yb) - 0.02), (ya, zt_any(ya) - 0.02)]
            b.poly("Board", mt["red"], [(xo, y, z) for y, z in pts], 0.022)
        for k in range(41):
            y = 0.2 * k
            ya, yb = max(y - 0.025, 0.0), min(y + 0.025, WID)
            top = zt_any(y) - 0.02
            b.add(type="add_box", corner=b.P((xo + 0.022, ya, Z_SILL_TOP)), size=[0.03, yb - ya, top - Z_SILL_TOP],
                  name="Batten", material=mt["batten"].id)
        # side cladding over bay 3: boards and battens, one copy each
        x0 = XB[2] + 0.13
        i = b.add(type="add_box", corner=b.P((x0 + 0.002, -0.022, Z_SILL_TOP)), size=[0.196, 0.022, Z_PLATE - Z_SILL_TOP],
                  name="Board", material=mt["red"].id)
        b.add(type="copy", ids=["$%d:i0" % i], offset=[0.2, 0, 0], count=18)
        i = b.add(type="add_box", corner=b.P((x0 - 0.025, -0.05, Z_SILL_TOP)), size=[0.05, 0.03, Z_PLATE - Z_SILL_TOP],
                  name="Batten", material=mt["batten"].id)
        b.add(type="copy", ids=["$%d:i0" % i], offset=[0.2, 0, 0], count=19)
        b.flush()

    def door(self):
        b, mt = self.b, self.mt
        x0, w, z0, h = 2.15, 3.7, 0.55, 2.95  # the leaf, slid 2 m toward bay 1
        y0 = -0.115
        n = 24
        i = b.add(type="add_box", corner=b.P((x0, y0, z0)), size=[w / n - 0.004, 0.04, h], name="Door board", material=mt["cream"].id)
        b.add(type="copy", ids=["$%d:i0" % i], offset=[w / n, 0, 0], count=n - 1)
        for z in (z0 + 0.2, z0 + h - 0.35):
            b.add(type="add_box", corner=b.P((x0, y0 - 0.03, z)), size=[w, 0.03, 0.14], name="Door rail", material=mt["post"].id)
        b.poly("Door brace", mt["post"], [(x0 + 0.08, y0 - 0.03, z0 + 0.34), (x0 + 0.25, y0 - 0.03, z0 + 0.34),
                                           (x0 + w - 0.08, y0 - 0.03, z0 + h - 0.35), (x0 + w - 0.25, y0 - 0.03, z0 + h - 0.35)], 0.03)
        steel = mt["steel"].id
        ztr = 3.89
        b.add(type="add_box", corner=b.P((0.3, -0.17, ztr)), size=[8.0, 0.06, 0.06], name="Door track", material=steel)
        for xb in (0.3, 4.1, 7.9):
            b.add(type="add_box", corner=b.P((xb - 0.04, -0.17, ztr + 0.06)), size=[0.08, 0.19, 0.1], name="Track bracket", material=steel)
        for dx in (0.3, w - 0.3):
            xw = x0 + dx
            b.add(type="add_box", corner=b.P((xw - 0.025, -0.134, z0 + h - 0.6)), size=[0.05, 0.014, ztr - 0.05 - (z0 + h - 0.6)],
                  name="Door hanger", material=steel)
            b.cyl((xw, -0.168, ztr - 0.05), 0.05, 0.035, axis=(0, 1, 0), segments=20, name="Door wheel", material=steel)
        b.add(type="add_box", corner=b.P((x0 + w - 0.12, y0 - 0.06, z0 + 1.0)), size=[0.03, 0.03, 0.55], name="Door pull", material=steel)
        for xs in (0.35, 8.25):
            b.add(type="add_box", corner=b.P((xs - 0.03, -0.17, ztr - 0.06)), size=[0.06, 0.06, 0.2], name="Track stop", material=steel)
        b.flush()

    def foundation(self):
        b, mt = self.b, self.mt
        self.ground = []
        for corner, size, name, mat in (([-14, -14, -0.5], [40, 40, 0.5], "Lawn", "grass"),
                                        ([1.2, -4.8, 0], [8.4, 3.75, 0.03], "Gravel", "gravel")):
            o = Spec(name, mt[mat], None)
            self.ground.append(o)
            b.pending.append((o, b.add(type="add_box", corner=corner, size=size, name=name, material=mt[mat].id)))
        st = mt["stone"].id
        b.add(type="add_box", corner=[0.1, -0.05, 0], size=[11.8, 0.4, Z_SILL], name="Foundation wall", material=st)
        b.add(type="add_box", corner=[0.1, WID - 0.35, 0], size=[11.8, 0.4, Z_SILL], name="Foundation wall", material=st)
        for xb in (XB[0], XB[-1]):
            b.add(type="add_box", corner=[xb - 0.2, 0.35, 0], size=[0.4, WID - 0.7, Z_SILL], name="Foundation wall", material=st)
        b.add(type="add_box", corner=[0.5, 0.35, 0], size=[XB[-1] - XB[0] - 0.4, WID - 0.7, Z_FLOOR], name="Earth floor", material=mt["earth"].id)
        for xb in XB:
            for yc in PYC:
                b.add(type="add_box", corner=[xb - 0.3, yc - 0.3, -0.3], size=[0.6, 0.6, Z_SILL + 0.3], name="Pier footing", material=mt["concrete"].id)
        b.flush()

    def workshop(self):
        b, mt = self.b, self.mt
        wood = mt["bench"].id
        x0, y0, z0 = 5.1, 6.85, Z_FLOOR
        top_z = z0 + 0.88
        for dx in (0.05, 2.26):
            for dy in (0.05, 0.56):
                b.add(type="add_box", corner=[x0 + dx, y0 + dy, z0], size=[0.09, 0.09, 0.88], name="Bench leg", material=wood)
        b.add(type="add_box", corner=[x0, y0, top_z], size=[2.4, 0.7, 0.07], name="Bench top", material=wood)
        b.add(type="add_box", corner=[x0 + 0.1, y0 + 0.1, z0 + 0.3], size=[2.2, 0.5, 0.03], name="Bench shelf", material=wood)
        b.add(type="add_box", corner=[x0 + 0.1, y0 + 0.03, top_z - 0.12], size=[2.2, 0.03, 0.12], name="Bench apron", material=wood)
        steel, blade, handle = mt["steel"].id, mt["blade"].id, mt["handle"].id
        b.add(type="add_box", corner=[x0 - 0.06, y0 - 0.08, top_z - 0.06], size=[0.16, 0.1, 0.18], name="Vise", material=steel)
        b.cyl((x0 - 0.02, y0 - 0.3, top_z - 0.02), 0.012, 0.25, axis=(0, 1, 0), segments=8, name="Vise screw", material=steel)
        zt_ = top_z + 0.07  # the bench top's face
        for k in range(3):
            xc = x0 + 0.5 + 0.07 * k
            b.cyl((xc, y0 + 0.30, zt_ + 0.014), 0.014, 0.12, axis=(0, 1, 0), segments=10, name="Chisel handle", material=handle)
            b.add(type="add_box", corner=[xc - 0.009, y0 + 0.42, zt_], size=[0.018, 0.16, 0.006], name="Chisel blade", material=blade)
        b.add(type="add_box", corner=[x0 + 1.0, y0 + 0.30, zt_], size=[0.12, 0.09, 0.07], name="Mallet head", material=handle)
        b.cyl((x0 + 1.06, y0 + 0.14, zt_ + 0.03), 0.014, 0.17, axis=(0, 1, 0), segments=10, name="Mallet handle", material=handle)
        b.add(type="add_box", corner=[x0 + 1.4, y0 + 0.20, zt_], size=[0.07, 0.25, 0.05], name="Plane", material=steel)
        b.add(type="add_box", corner=[x0 + 1.4, y0 + 0.20, zt_ + 0.05], size=[0.07, 0.05, 0.02], name="Plane tote", material=handle)
        b.add(type="add_box", corner=[x0 + 1.75, y0 + 0.15, zt_], size=[0.3, 0.025, 0.004], name="Try square", material=blade)
        b.add(type="add_box", corner=[x0 + 1.75, y0 + 0.15, zt_ + 0.004], size=[0.025, 0.2, 0.012], name="Try square stock", material=handle)
        for k in range(4):
            b.cyl((x0 + 1.95 + 0.04 * k, y0 + 0.45, zt_ + 0.008), 0.008, 0.18, axis=(0, 1, 0), segments=8, name="Drawbore pin", material=steel)
        # a stick of timber on two trestles, and a stack of offcuts
        for xt in (8.6, 10.4):
            for yy in (5.3, 5.8):
                b.add(type="add_box", corner=[xt, yy, Z_FLOOR], size=[0.08, 0.08, 0.7], name="Trestle leg", material=wood)
            b.add(type="add_box", corner=[xt - 0.05, 5.28, Z_FLOOR + 0.7], size=[0.18, 0.62, 0.08], name="Trestle top", material=wood)
        b.add(type="add_box", corner=[8.2, 5.4, Z_FLOOR + 0.78], size=[2.9, 0.26, 0.16], name="Timber on trestles", material=mt["post"].id)
        for k in range(5):
            b.add(type="add_box", corner=[1.0, 5.6, Z_FLOOR + 0.12 * k], size=[1.8, 0.1, 0.1], name="Offcut", material=mt["common"].id)
        b.flush()

    def yard(self):
        """Hay bales by the gable, a firewood stack at the west end, stone steps at the door."""
        b, mt = self.b, self.mt
        hay = mt["hay"].id
        for k, (x, y, z, ry) in enumerate(((12.7, -0.2, 0.0, 0), (13.4, -0.2, 0.0, 0), (12.7, 0.5, 0.0, 0), (13.4, 0.5, 0.0, 0),
                                           (13.05, 0.15, 0.38, 0), (13.0, 2.3, 0.0, 0))):
            b.add(type="add_box", corner=[x, y, z], size=[0.7, 0.55, 0.38], name="Hay bale", material=hay)
        for row in range(4):
            for k in range(8 - row):
                b.cyl((-1.9 + 0.14 * k + 0.07 * row, 1.4, 0.07 + 0.125 * row), 0.065, 2.2,
                      axis=(0, 1, 0), segments=8, name="Firewood", material=mt["trunk"].id)
        for x0, y0, w, d, z0 in ((4.65, -1.05, 3.3, 1.0, 0.0), (4.95, -0.5, 2.7, 0.45, 0.2)):
            b.add(type="add_box", corner=[x0, y0, z0], size=[w, d, 0.2], name="Stone step", material=mt["stone"].id)
        b.flush()

    def scenery(self):
        m, mt = self.m, self.mt
        trunk = m.cylinder((0, 0, 0), 0.2, 2.8, segments=10, material=mt["trunk"])
        crown = m.sphere((0, 0, 4.2), 1.9, segments=16, material=mt["leaf"])
        top = m.sphere((0.5, 0.4, 5.1), 1.2, segments=14, material=mt["leaf2"])
        tree = m.component([trunk, crown, top], "Tree")
        tree.hide()
        from tenon import Transform
        spots = ((-12.0, 9.0, 1.2), (-11.0, -6.0, 1.0), (17.0, 14.0, 1.3), (1.0, 19.0, 1.1), (21.0, 3.0, 0.9), (24.0, -8.0, 1.0))
        self.trees = [m.place(tree.definition, Transform.translate(x, y, 0) @ Transform.scale(sc)) for x, y, sc in spots]


# ---------------------------------------------------------------- the joint, pulled apart


def joint_demo(b, mt):
    """The front corner of bent 1 again, in its own place: post, plate, tie beam, knee brace and
    three pegs. Returns the member Specs and the pegs."""
    xb, yc = XB[0], PYC[0]
    specs, pegs = [], []

    def mk(name, mat, base, chamfer):
        s = Spec(name, mt[mat], base, chamfer)
        specs.append(s)
        return s

    post = mk("Post", "post", ("box", (xb - 0.13, yc - 0.13, 3.0), (POST, POST, Z_PLATE - 3.0)), 0.014)
    plate = mk("Wall plate", "plate", ("box", (xb - 0.7, yc - 0.12, Z_PLATE), (1.4, 0.24, 0.25)), 0.014)
    tie = mk("Tie beam", "tie", ("box", (xb - 0.11, -0.15, Z_TIE), (0.22, 1.75, 0.30)), 0.014)
    y_f = yc + 0.13
    pts = [(y_f, 3.05), (y_f, 3.22), (y_f + 1.03, Z_TIE), (y_f + 1.20, Z_TIE)]
    brace = mk("Knee brace", "brace", ("poly", [(xb - 0.06, y, z) for y, z in pts], 0.12), 0.01)
    join(post, plate, (xb, yc, Z_PLATE), (0, 0, 1), 0.17, (0.05, 0.05, 0))
    peg(pegs, (xb, yc, Z_PLATE + 0.09), (0, 1, 0), 0.34, post, plate)
    join(brace, post, (xb, y_f, 3.135), (0, -1, 0), 0.12, (0.045, 0, 0.045))
    peg(pegs, (xb, y_f - 0.06, 3.135), (1, 0, 0), 0.34, post, brace)
    join(brace, tie, (xb, y_f + 1.115, Z_TIE), (0, 0, 1), 0.12, (0.045, 0.045, 0))
    peg(pegs, (xb, y_f + 1.115, Z_TIE + 0.075), (1, 0, 0), 0.30, tie, brace)
    for s in specs:
        b.emit(s)
    objs = list(b.emit_pegs(pegs, mt["peg"]))
    b.flush()
    return specs, objs, pegs


# ---------------------------------------------------------------- the schedule


def schedule(c, frame_ids, path):
    """The timber schedule from one ``cut_list`` query (a row per part, grouped here by member
    and size) and the volume and mass of the oak frame from ``mass_properties``."""
    rows = c.query("cut_list", group_by="none")["rows"]
    frame = set(frame_ids)
    oak = [r for r in rows if r["ids"] and r["ids"][0] in frame]
    pine = [r for r in rows if r["name"] in ("Common rafter", "Roof plank")]

    def table(rows, title):
        groups = {}
        for r in rows:
            key = (r["name"], round(r["thickness"] * 1000), round(r["width"] * 1000), round(r["length"] * 1000 / 10) * 10)
            groups[key] = groups.get(key, 0) + 1
        out = [title, "-" * 72,
               "%-26s %5s  %-14s %11s %12s" % ("Member", "Count", "Section (mm)", "Length (mm)", "Volume (m3)"), "-" * 72]
        total, count = 0.0, 0
        for key in sorted(groups, key=lambda k: (-groups[k] * k[1] * k[2] * k[3], k[0])):
            name, t, w, l = key
            n = groups[key]
            vol = n * (t / 1000) * (w / 1000) * (l / 1000)
            total += vol
            count += n
            out.append("%-26s %5d  %-14s %11d %12.3f" % (name, n, "%d x %d" % (w, t), l, vol))
        out += ["-" * 72, "%-26s %5d  %-14s %11s %12.3f" % ("Total (bounding boards)", count, "", "", total), ""]
        return out

    lines = ["TIMBER SCHEDULE: PEGGED OAK BARN, 8 m x 12 m, FOUR BENTS", "",
             "Lengths are overall (tenons included); volumes are the boards the pieces are cut from.", ""]
    lines += table(oak, "OAK FRAME AND PEGS")
    lines += table(pine, "SOFTWOOD: COMMON RAFTERS AND ROOF PLANKS")
    tot = c.query("mass_properties", ids=frame_ids)["total"]
    lines += ["OAK FRAME, MASS PROPERTIES (solid volume after the mortises and peg holes are cut)",
              "  Pieces    %8d" % len(frame_ids),
              "  Volume    %8.3f m3" % tot["volume"],
              "  Mass      %8.0f kg  (oak, %d kg/m3)" % (tot.get("mass", 0), OAK),
              "  Centroid  (%.2f, %.2f, %.2f) m" % tuple(tot["centroid"]), ""]
    pids = sorted({i for r in pine for i in r["ids"]})
    if pids:
        t2 = c.query("mass_properties", ids=pids)["total"]
        lines += ["SOFTWOOD, MASS PROPERTIES (pine, %d kg/m3)" % PINE,
                  "  Volume    %8.3f m3" % t2["volume"], "  Mass      %8.0f kg" % t2.get("mass", 0), ""]
    with open(path, "w") as f:
        f.write("\n".join(lines))
    return tot


# ---------------------------------------------------------------- main

DEMO_OFF = (40.0, 0.0, 0.0)  # the joint detail stands apart, east of the barn


def build(c, m, quick=False, log=print):
    """The whole scene; returns the Barn and the joint detail's (specs, pegs)."""
    t = time.time()

    def lap(label):
        nonlocal t
        log("  %-28s %5.1f s" % (label, time.time() - t))
        t = time.time()

    barn = Barn(c, m, quick)
    barn.materials()
    barn.foundation()
    lap("materials, foundation")
    barn.build_frame()
    lap("frame: joinery and pegs")
    barn.roof_and_walls()
    barn.door()
    lap("roof, cladding, door")
    barn.workshop()
    barn.yard()
    barn.scenery()
    lap("workshop, trees")
    specs, pegs, _ = joint_demo(Builder(c, off=DEMO_OFF), barn.mt)
    c.execute({"type": "hide", "ids": [x.id for x in specs + pegs]})
    lap("joint detail")
    return barn, (specs, pegs)


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--socket", help="socket of a running app (default: start a headless one)")
    ap.add_argument("--app", help="app binary to start (default: TENON_APP or the repo's target/)")
    ap.add_argument("--out", default=".", help="directory for the images (default: here)")
    ap.add_argument("--quick", action="store_true", help="small images (for tests)")
    args = ap.parse_args(argv)
    width, height, ss = (320, 200, 1) if args.quick else (1600, 1000, 2)
    out = args.out
    os.makedirs(out, exist_ok=True)
    t_all = time.time()
    with session(args.socket, args.app) as s:
        c = s.client
        m = Model(c)
        c.execute({"type": "new", "discard": s.launched})
        barn, demo = build(c, m, args.quick)
        c.execute({"type": "select", "ids": [], "mode": "replace"})
        mt = barn.mt
        specs, pegs = demo
        pic = lambda name: os.path.join(out, name)  # noqa: E731
        t = time.time()
        # ---- 1: the barn in sunlight, with sun shadows and ambient occlusion
        c.execute({"type": "set_location", "latitude": 44.3, "longitude": -72.6, "north_angle": 0, "utc_offset": -4})
        c.execute({"type": "set_shadows", "on": True, "date": "2026-09-12", "time": "16:00", "on_ground": True})
        m.style("Daylight", sky=[132, 176, 228], background=[218, 230, 244], edges={"color": [46, 34, 26]},
                ambient_occlusion={"on": True, "radius": 0.5, "strength": 1.1})
        hero = {"eye": [-5.4, -13.4, 6.5], "target": [6.3, 3.0, 3.3], "fov_y_deg": 36}
        m.render(pic("barn.png"), width, height, camera=hero, lighting="day", supersample=ss, ground=False)
        print("  %-28s %5.1f s" % ("barn.png", time.time() - t))
        t = time.time()
        # ---- 2: a section through the second bent, cuts filled in the oak's own colour
        c.execute({"type": "add_section_plane", "point": [XB[1], 4, 4], "normal": [1, 0, 0]})
        m.section_display(planes=False, cuts=True, fill=True, fill_color="material")
        m.style("Section", sky=[236, 238, 242], background=[236, 238, 242], edges={"color": [40, 30, 24], "profiles": 2},
                ambient_occlusion={"on": True, "radius": 0.4, "strength": 1.0})
        c.execute({"type": "set_shadows", "on": False})
        trees = [x.id for x in barn.trees]
        c.execute({"type": "hide", "ids": trees})
        sec = {"eye": [-26.0, 4.0, 3.9], "target": [XB[1], 4.0, 3.7], "fov_y_deg": 17, "projection": "parallel"}
        m.render(pic("barn-section.png"), width, height, camera=sec, lighting="studio", supersample=ss, ground=False)
        c.execute({"type": "deactivate_section"})
        m.section_display(cuts=False)
        c.execute({"type": "unhide", "ids": trees})
        print("  %-28s %5.1f s" % ("barn-section.png", time.time() - t))
        t = time.time()
        # ---- 3: a pencil drawing: profiles and sketchy edges, lawn and trees left out
        scenery = [x.id for x in barn.ground + barn.trees]
        c.execute({"type": "hide", "ids": scenery})
        m.style("Pencil", background=[250, 248, 242], sky=None,
                edges={"color": [44, 38, 36], "profiles": 3, "sketchy": {"jitter": 1.6, "extension": 6}},
                ambient_occlusion={"on": False})
        m.render(pic("barn-sketch.png"), width, height, camera=hero, lighting="day", supersample=ss, ground=False)
        c.execute({"type": "unhide", "ids": scenery})
        print("  %-28s %5.1f s" % ("barn-sketch.png", time.time() - t))
        t = time.time()
        # ---- 4: the timber schedule
        frame_ids = [x.id for x in barn.frame]
        tot = schedule(c, frame_ids, pic("barn-cutlist.txt"))
        print("  %-28s %5.1f s  (%d pieces, %.2f m3, %.0f kg)" % ("barn-cutlist.txt", time.time() - t, len(frame_ids), tot["volume"], tot.get("mass", 0)))
        t = time.time()
        # ---- 5: the joint pulled apart: post, plate, tie beam, knee brace and their three pegs
        post, plate, tie, brace = specs[:4]
        peg_plate, peg_post, peg_tie = pegs[:3]
        c.execute({"type": "unhide", "ids": [x.id for x in specs + pegs]})
        x0 = DEMO_OFF[0]
        # each member is drawn out a short way (about 1.5 tenon lengths) straight along its own
        # axis; explode_view takes the post straight down from a centre on its own axis
        xc = x0 + XB[0]
        yc = PYC[0]
        m.explode_view([post], 1.0, center=(xc, yc, 3.45 + 0.27))  # post centre z 3.45: down 0.27
        Instance(m, tie.id).move((0, 0, 0.22))
        Instance(m, brace.id).move((0, 0.22, 0))
        Instance(m, peg_plate.id).move((0, 0.32, 0))
        Instance(m, peg_post.id).move((-0.32, 0, -0.27))
        Instance(m, peg_tie.id).move((-0.32, 0, 0.22))
        guide = m.material("Guide", (90, 90, 96), roughness=0.6)

        def line(a, b):
            d = [b[i] - a[i] for i in range(3)]
            n = math.sqrt(sum(v * v for v in d))
            m.cylinder(a, 0.004, n, axis=d, segments=6, material=guide)

        for a, b in (((xc, yc, 3.7), (xc, yc, 4.5)),  # the post's tenon up through the plate
                     ((xc, yc - 0.45, Z_PLATE + 0.09), (xc, yc + 0.75, Z_PLATE + 0.09)),  # plate peg
                     ((xc - 0.6, yc + 0.07, 3.135 - 0.27 + 0.0), (xc + 0.3, yc + 0.07, 3.135 - 0.27))):  # post peg):
            line(a, b)
        m.style("Studio", background=[232, 234, 238], sky=None, edges={"color": [46, 34, 26]},
                ambient_occlusion={"on": True, "radius": 0.2, "strength": 1.4})
        m.label(post.id, "Post tenon", position=(xc + 0.3, yc - 0.3, 3.35))
        m.label(plate.id, "Plate: mortise below", position=(xc + 0.55, yc, 4.4))
        m.label(tie.id, "Tie beam: mortise for the brace", position=(xc + 0.1, yc + 0.2, 4.95))
        m.label(brace.id, "Knee brace tenons", position=(xc + 0.1, yc + 1.5, 3.3))
        m.label(peg_post.id, "Oak peg", position=(xc - 0.55, yc + 0.3, 2.9))
        c.execute({"type": "select", "ids": [], "mode": "replace"})
        joint_cam = {"eye": [xc - 3.4, 3.6, 2.6], "target": [xc + 0.05, 0.5, 3.9], "fov_y_deg": 32}
        m.render(pic("barn-joint.png"), width, height, camera=joint_cam, lighting="studio", supersample=ss, ground=False)
        print("  %-28s %5.1f s" % ("barn-joint.png", time.time() - t))
        print("total %.1f s" % (time.time() - t_all))


if __name__ == "__main__":
    main()
