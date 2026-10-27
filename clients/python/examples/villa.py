#!/usr/bin/env python3
"""A cantilevered contemporary villa on a terraced hillside, drawn by day, at dusk, in section and in plan.

    python3 villa.py [--socket PATH] [--out DIR] [--quick]

Without ``--socket`` it starts a headless app. It writes ``villa.png`` (a low three-quarter view
at golden hour: sun shadows, ambient occlusion, fog), ``villa-dusk.png`` (the same view at night,
the pendants, coves, pool, bollards and wall and tree uplights glowing), ``villa-section.png`` (a
section through the double-height living room and both floors, cuts filled in each material's
colour, labelled), ``villa-plan.png`` (a parallel top view of the ground floor, cut at 1.6 m) and
``villa.tnn`` (the model, saved with the daytime view, shadows on, nothing selected) to ``--out``.
The garden lights (pool, bollards, uplights) are switched off with ``change_material`` for the
daytime pictures and on for the night one; the saved model has them off, like the daytime picture.

What it shows of the API:

- a two-storey house of interlocking volumes: a double-height living pavilion in a stone
  and glass box, a kitchen and dining wing, and a bedroom volume that cantilevers 5.5 m out over
  the veranda and the end of the pool, under thin flat roofs with deep overhangs and timber soffits;
- natural stone as coursed ashlar: every course a box copied along the wall with ``copy`` (two
  interleaved arrays, so the joints alternate and the tone varies), on the house, the chimney
  stack, the screen walls and the retaining walls that step down the slope;
- arrays for everything repeated: deck boards, vertical timber fins, soffit boards, pergola slats,
  glass balustrade panels, mullions, the floating staircase's treads;
- see-through glass (opacity, metalness, roughness), pool water, glowing materials (``emission``)
  for pendants, cove lights, underwater lights, bollards and uplights;
- components with scale and rotation variation for the planting: cypress, plane trees, olives,
  palms, grass tufts, and for the chairs, loungers and stools; ``fillet_edges`` on the hedges;
- ``set_location`` and ``set_shadows`` for the sun, styles with sky, fog and ambient occlusion,
  ``night`` lighting, a section plane with ``fill_color: material`` and a parallel top view.

Lengths are meters. X runs east along the house (about 33 m), Y north (the front, with the pool,
faces -Y), Z up; the ground floor is z = 0 and the site falls away to the south in three terraces
(z = 0, -1.5, -3).
"""
import argparse
import math
import os
import random
import sys
import time
import types

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))

from tenon import session  # noqa: E402

# ---------------------------------------------------------------- levels (meters)

Z_SLAB = 3.2  # underside of the first-floor slab
Z_UF = 3.5  # first floor
Z_UR = 6.5  # underside of the bedroom roof
Z_UT = 6.8  # its top
Z_WR = 7.0  # underside of the living pavilion's roof
Z_WT = 7.3
POOL = (3.0, 23.0, -8.4, -1.5)  # x0, x1, y0, y1 of the water


# ---------------------------------------------------------------- the command builder


class B:
    """Collects commands and sends them in batches. ``box`` and friends return the index of the
    command, which ``$index:i0`` refers to in later commands of the same batch."""

    def __init__(self, c):
        self.c = c
        self.cmds = []
        self.total = 0
        self.serial = 0  # batches sent so far
        self.results = {}  # serial -> outcomes
        self.sink = None  # a list that collects tokens of the instances made meanwhile

    def add(self, **cmd):
        self.cmds.append(cmd)
        i = len(self.cmds) - 1
        if self.sink is not None and cmd["type"] in ("add_box", "copy", "add_cylinder", "add_cone", "add_sphere", "place_instance"):
            self.sink.append((self.serial, i))
        return i

    def flush(self):
        if not self.cmds:
            return []
        cmds, self.cmds = self.cmds, []
        try:
            r = self.c.batch(cmds)
        except Exception as e:
            idx = getattr(e, "index", None)
            if idx is not None:
                for j in range(max(0, idx - 3), idx + 1):
                    print("  %d: %s" % (j, cmds[j]))
            raise
        self.total += len(cmds)
        self.results[self.serial] = r["outcomes"]
        self.serial += 1
        return r["outcomes"]

    def mark(self, idx):
        """A token for command ``idx`` of the batch being collected, good after any flush."""
        return (self.serial, idx)

    def placed(self, token):
        """The id of the instance that command made."""
        return self.results[token[0]][token[1]]["placed"][0]

    def placed_all(self, tokens):
        """Every instance the commands made (a copy makes several)."""
        return [i for t in tokens for i in self.results[t[0]][t[1]].get("placed", [])]

    def tick(self, n=400):
        """Send what has collected, between units that refer to nothing earlier."""
        if len(self.cmds) > n:
            self.flush()

    # primitives -----------------------------------------------------------

    def box(self, p0, p1, mat, name):
        lo = [min(p0[i], p1[i]) for i in range(3)]
        sz = [abs(p1[i] - p0[i]) for i in range(3)]
        assert min(sz) > 1e-5, (name, p0, p1)
        return self.add(type="add_box", corner=[round(v, 5) for v in lo], size=[round(v, 5) for v in sz],
                        name=name, material=mat.id)

    def cyl(self, base, r, h, mat, name, axis=None, seg=16):
        c = dict(type="add_cylinder", base=list(base), radius=r, height=h, segments=seg, name=name, material=mat.id)
        if axis is not None:
            c["axis"] = list(axis)
        return self.add(**c)

    def cone(self, base, r, h, mat, name, top=0.0, axis=None, seg=16):
        c = dict(type="add_cone", base=list(base), radius=r, height=h, top_radius=top, segments=seg,
                 name=name, material=mat.id)
        if axis is not None:
            c["axis"] = list(axis)
        return self.add(**c)

    def sph(self, center, r, mat, name, seg=14):
        return self.add(type="add_sphere", center=list(center), radius=r, segments=seg, name=name, material=mat.id)

    def poly(self, pts, dist, mat, name):
        """A flat closed profile pushed ``dist`` along its normal, as a painted group."""
        a = self.add(type="add_curve", points=[list(p) for p in pts], closed=True, curve=False)
        p = self.add(type="push_pull", face="$%d:f0" % a, distance=dist)
        g = self.add(type="make_group", ids=["$%d:*" % a, "$%d:*" % p], name=name)
        self.add(type="paint", ids=["$%d:i0" % g], material=mat.id)
        return g

    # operations on an earlier command's instance --------------------------

    def copy(self, i, offset, count):
        if count < 1:
            return None
        return self.add(type="copy", ids=["$%d:i0" % i], offset=[round(v, 5) for v in offset], count=count)

    def rot(self, i, axis, angle, center):
        self.add(type="rotate", ids=["$%d:i0" % i], center=list(center), axis=list(axis), angle=angle)

    def scale(self, i, factors, origin):
        self.add(type="scale", ids=["$%d:i0" % i], origin=list(origin), factors=list(factors))

    def round_edges(self, i, radius, segments=3):
        self.add(type="fillet_edges", instance="$%d:i0" % i, edges=["$%d:e*" % i], radius=radius, segments=segments)

    # components and placing -----------------------------------------------

    def comp(self, name, parts):
        """A hidden component made at the origin by ``parts()`` (which adds commands and returns
        their indices); returns its definition id for ``place``."""
        self.flush()
        refs = parts()
        i = self.add(type="make_component", ids=["$%d:i0" % r for r in refs], name=name)
        self.add(type="hide", ids=["$%d:i0" % i])
        return self.flush()[i]["definitions"][0]

    def place(self, d, x, y, z=0.0, yaw=0.0, s=1.0, sz=None):
        sz = s if sz is None else sz
        a = math.radians(yaw)
        co, si = math.cos(a), math.sin(a)
        return self.add(type="place_instance", definition=d,
                        transform={"origin": [x, y, z], "x_axis": [s * co, s * si, 0], "y_axis": [-s * si, s * co, 0],
                                   "z_axis": [0, 0, sz]})


# ---------------------------------------------------------------- materials

# glowing materials: key -> (name, color, emission). The garden lights are switched off for the
# daytime pictures and on again for the night one (``change_material``).
GLOWS = {
    "g_warm": ("Glow, warm", (255, 200, 130), 4.0),
    "g_cove": ("Glow, cove", (255, 206, 146), 8.0),
    "g_pend": ("Glow, pendant", (255, 214, 156), 7.0),
    "g_pool": ("Glow, pool", (120, 232, 255), 10.0),
    "g_fire": ("Glow, flame", (255, 150, 60), 6.0),
    "g_bol": ("Glow, bollard", (255, 214, 160), 8.0),
    "g_up": ("Glow, uplight", (255, 226, 176), 10.0),
}
GARDEN_LIGHTS = ("g_pool", "g_bol", "g_up")


def make_materials(c):
    spec = {}

    def mat(key, name, color, opacity=1.0, metal=0.0, rough=1.0, emission=0.0):
        d = dict(type="add_material", name=name, color=list(color), existing="update")
        if opacity != 1.0:
            d["opacity"] = opacity
        if metal or rough != 1.0:
            d["metalness"] = metal
            d["roughness"] = rough
        if emission:
            d["emission"] = emission
        spec[key] = d

    # stone: five warm limestone tones, a dark one for cores and retaining walls
    for k, col in enumerate(((190, 176, 152), (176, 161, 138), (164, 149, 126), (198, 186, 164), (154, 140, 118))):
        mat("stone%d" % k, "Limestone %d" % (k + 1), col, rough=0.95)
    mat("core", "Wall core", (118, 110, 100), rough=1)
    mat("render", "White render", (240, 238, 232), rough=0.9)
    mat("render2", "Light grey render", (214, 212, 206), rough=0.9)
    mat("timber", "Oak, warm", (176, 124, 72), rough=0.55)
    mat("timber2", "Oak, light", (192, 142, 88), rough=0.55)
    mat("timber3", "Oak, honey", (170, 116, 64), rough=0.55)
    mat("walnut", "Walnut", (92, 62, 40), rough=0.45)
    mat("bronze", "Dark bronze", (58, 52, 46), metal=0.8, rough=0.38)
    mat("steel", "Anthracite steel", (46, 48, 52), metal=0.85, rough=0.42)
    mat("steelb", "Brushed steel", (176, 180, 186), metal=0.9, rough=0.3)
    mat("glass", "Glass", (138, 190, 206), opacity=0.25, metal=0.3, rough=0.05)
    mat("water", "Pool water", (22, 104, 134), opacity=0.64, metal=0.0, rough=0.04)
    mat("sheet", "Water sheet", (150, 220, 232), opacity=0.45, rough=0.05)
    mat("tile", "Pool tile", (118, 176, 186), rough=0.3)
    mat("basalt", "Basalt", (46, 48, 50), rough=0.7)
    mat("coping", "Pool coping", (226, 220, 206), rough=0.8)
    mat("lawn", "Lawn", (92, 130, 70), rough=1)
    mat("lawn2", "Lawn, sunlit", (106, 142, 76), rough=1)
    mat("dry", "Dry grass", (156, 150, 98), rough=1)
    mat("dry2", "Dry grass, olive", (136, 134, 86), rough=1)
    mat("gravel", "Gravel", (214, 204, 184), rough=1)
    mat("paving", "Paving", (190, 186, 176), rough=0.9)
    mat("floor", "Polished limestone", (226, 220, 208), metal=0.0, rough=0.25)
    mat("cypress", "Cypress", (40, 78, 52), rough=1)
    mat("cypress2", "Cypress, young", (54, 94, 60), rough=1)
    mat("leaf", "Leaves", (84, 128, 62), rough=1)
    mat("leaf2", "Leaves, light", (112, 150, 74), rough=1)
    mat("leaf3", "Leaves, deep", (62, 106, 58), rough=1)
    mat("olive", "Olive leaves", (126, 142, 106), rough=1)
    mat("olive2", "Olive leaves, grey", (148, 160, 130), rough=1)
    mat("bark", "Bark", (96, 76, 60), rough=1)
    mat("palmtrunk", "Palm trunk", (150, 126, 98), rough=1)
    mat("frond", "Palm frond", (66, 122, 58), rough=1)
    mat("hedge", "Hedge", (58, 98, 56), rough=1)
    mat("grass", "Ornamental grass", (176, 176, 112), rough=1)
    mat("grass2", "Ornamental grass, green", (128, 160, 96), rough=1)
    mat("soil", "Planting bed", (84, 66, 52), rough=1)
    mat("sofa", "Cream fabric", (222, 214, 198), rough=1)
    mat("charcoal", "Charcoal fabric", (68, 70, 74), rough=1)
    mat("leather", "Tan leather", (156, 100, 58), rough=0.55)
    mat("rug", "Wool rug", (206, 194, 174), rough=1)
    mat("linen", "White linen", (246, 244, 238), rough=1)
    mat("marble", "Marble", (236, 234, 230), rough=0.2)
    mat("art", "Terracotta canvas", (176, 84, 52), rough=1)
    mat("art2", "Ochre canvas", (206, 158, 72), rough=1)
    mat("asphalt", "Dark paving", (84, 84, 86), rough=0.9)
    mat("carbody", "Car paint, graphite", (62, 66, 74), metal=0.9, rough=0.22)
    mat("tyre", "Tyre", (24, 24, 26), rough=0.9)
    mat("embers", "Embers", (70, 30, 20), rough=1)
    for key, (name, color, em) in GLOWS.items():
        mat(key, name, color, emission=em)

    keys = list(spec)
    r = c.batch([spec[k] for k in keys])
    return {k: types.SimpleNamespace(id=r["outcomes"][i]["changes"]["materials"][0]) for i, k in enumerate(keys)}


# ---------------------------------------------------------------- stone, boards, glass


STONES = ("stone0", "stone1", "stone2", "stone3", "stone4")
COURSES = (0.21, 0.30, 0.18, 0.26, 0.36, 0.22, 0.30, 0.17)
GAP = 0.014


def ashlar(b, M, axis, a0, a1, face, sgn, z0, z1, rng, T=0.07, L0=0.95, joints=True, name="Limestone"):
    """Coursed stone on a wall: blocks ``T`` thick on the plane ``face`` (the wall's outer face)
    facing ``sgn`` (+1 or -1) across ``axis`` ('x': a wall running along x, facing +-y; 'y': along y,
    facing +-x), from ``a0`` to ``a1`` along the wall and ``z0`` to ``z1`` up. Each course is one block
    copied along the wall twice, the second array shifted a block, so the joints alternate and the
    tone varies; odd courses start with a half block."""
    length = a1 - a0
    z, k = z0, 0
    while z < z1 - 1e-6:
        h = COURSES[(k + rng.randrange(3)) % len(COURSES)]
        if z1 - z - h < 0.12:
            h = z1 - z
        h = min(h, z1 - z)
        ta, tb = rng.sample(STONES, 2)

        def blk(a, la, tone):
            lo = a + GAP / 2
            ln = la - GAP
            zz = z + GAP / 2
            hh = h - GAP
            f0 = face if sgn > 0 else face - T
            if axis == "x":
                return b.add(type="add_box", corner=[round(lo, 5), round(f0, 5), round(zz, 5)],
                             size=[round(ln, 5), T, round(hh, 5)], name=name, material=M[tone].id)
            return b.add(type="add_box", corner=[round(f0, 5), round(lo, 5), round(zz, 5)],
                         size=[T, round(ln, 5), round(hh, 5)], name=name, material=M[tone].id)

        def step(d):
            return (d, 0, 0) if axis == "x" else (0, d, 0)

        if joints:
            L = L0 * rng.choice((0.8, 1.0, 1.25))
            npair = max(1, round(length / (2 * L)))
            Lp = length / (2 * npair)
            if k % 2 == 0:
                i = blk(a0, Lp, ta)
                b.copy(i, step(2 * Lp), npair - 1)
                i = blk(a0 + Lp, Lp, tb)
                b.copy(i, step(2 * Lp), npair - 1)
            else:
                blk(a0, Lp / 2, tb)
                i = blk(a0 + Lp / 2, Lp, ta)
                b.copy(i, step(2 * Lp), npair - 1)
                if npair > 1:
                    i = blk(a0 + Lp / 2 + Lp, Lp, tb)
                    b.copy(i, step(2 * Lp), npair - 2)
                blk(a1 - Lp / 2, Lp / 2, tb)
        else:
            blk(a0, length, ta)
        z += h
        k += 1
        b.tick(300)


def boards(b, M, x0, x1, y0, y1, ztop, mats, pitch=0.155, w=0.14, thick=0.05, name="Deck board", along="x"):
    """Boards running along x (or y) across the box x0..x1, y0..y1, their tops at ``ztop``,
    two alternating tones as two interleaved arrays."""
    span = (y1 - y0) if along == "x" else (x1 - x0)
    n = max(1, int(span / pitch))
    for j, key in enumerate(mats[:2]):
        cnt = (n + 1 - j) // 2
        if cnt < 1:
            continue
        o = j * pitch
        if along == "x":
            i = b.box((x0, y0 + o, ztop - thick), (x1, y0 + o + w, ztop), M[key], name)
            b.copy(i, (0, 2 * pitch, 0), cnt - 1)
        else:
            i = b.box((x0 + o, y0, ztop - thick), (x0 + o + w, y1, ztop), M[key], name)
            b.copy(i, (2 * pitch, 0, 0), cnt - 1)


def glass_wall(b, M, axis, a0, a1, fixed, z0, z1, pitch, name, transoms=(), depth=0.12, mw=0.06):
    """Floor-to-ceiling glazing: panes (one box copied) between slim dark mullions (one box copied),
    transoms across. ``axis`` 'x': the wall runs along x at y = ``fixed``."""
    n = max(1, round((a1 - a0) / pitch))
    p = (a1 - a0) / n

    def bx(a_lo, a_hi, f_lo, f_hi, zl, zh, key, nm):
        if axis == "x":
            return b.box((a_lo, f_lo, zl), (a_hi, f_hi, zh), M[key], nm)
        return b.box((f_lo, a_lo, zl), (f_hi, a_hi, zh), M[key], nm)

    def off(d):
        return (d, 0, 0) if axis == "x" else (0, d, 0)

    i = bx(a0 + mw / 2, a0 + p - mw / 2, fixed - 0.015, fixed + 0.015, z0, z1, "glass", name + " glass")
    b.copy(i, off(p), n - 1)
    i = bx(a0 - mw / 2, a0 + mw / 2, fixed - depth / 2, fixed + depth / 2, z0, z1, "steel", name + " mullion")
    b.copy(i, off(p), n)
    for zt in transoms:
        bx(a0, a1, fixed - depth / 2, fixed + depth / 2, zt - 0.03, zt + 0.03, "steel", name + " transom")


def balustrade(b, M, axis, a0, a1, fixed, z0, h=1.05, pitch=1.9, name="Glass balustrade"):
    """Frameless glass panels, 20 mm apart, in a slim steel shoe."""
    n = max(1, round((a1 - a0) / pitch))
    p = (a1 - a0) / n

    def bx(a_lo, a_hi, f_lo, f_hi, zl, zh, key, nm):
        if axis == "x":
            return b.box((a_lo, f_lo, zl), (a_hi, f_hi, zh), M[key], nm)
        return b.box((f_lo, a_lo, zl), (f_hi, a_hi, zh), M[key], nm)

    off = (lambda d: (d, 0, 0)) if axis == "x" else (lambda d: (0, d, 0))
    i = bx(a0 + 0.01, a0 + p - 0.01, fixed - 0.012, fixed + 0.012, z0 + 0.08, z0 + h, "glass", name)
    b.copy(i, off(p), n - 1)
    bx(a0, a1, fixed - 0.04, fixed + 0.04, z0, z0 + 0.08, "steel", name + " shoe")
    bx(a0, a1, fixed - 0.02, fixed + 0.02, z0 + h, z0 + h + 0.025, "steel", name + " cap")


# ---------------------------------------------------------------- the villa


class Villa:
    def __init__(self, c, quick=False):
        self.c = c
        self.quick = quick
        self.b = B(c)
        self.M = make_materials(c)
        self.rng = random.Random(11)
        self.keys = {}  # name -> token of an instance the labels point at

    # ---- site ---------------------------------------------------------------

    def terrain(self):
        b, M, rng = self.b, self.M, self.rng
        # the hill: a lawn at z = -3, then terraces stepping up toward the house and up behind it
        b.box((-60, -70, -4), (90, 100, -3), M["lawn"], "Hillside lawn")
        b.box((-14, -19, -3), (58, 40, -1.5), M["lawn2"], "Lower terrace")
        self.hill = []
        b.sink = self.hill
        b.box((-60, 18, -3), (90, 60, 1.5), M["dry2"], "Upper terrace")
        b.box((-60, 26, -3), (90, 70, 3.0), M["dry"], "Upper terrace 2")
        b.box((-60, 34, -3), (90, 80, 4.5), M["dry2"], "Upper terrace 3")
        b.box((-60, 60, -3), (90, 100, 9.0), M["dry"], "Ridge")
        b.sink = None
        # mown stripes on the lower lawns
        boards(b, M, -13.8, 57.8, -18.8, -9.4, -1.49, ("lawn", "lawn2"), pitch=1.6, w=0.8, thick=0.012, name="Lawn stripe", along="y")
        boards(b, M, -55, 85, -65, -19.4, -2.99, ("lawn2", "lawn"), pitch=3.0, w=1.5, thick=0.012, name="Lawn stripe", along="y")
        b.flush()
        j = not self.quick
        # retaining walls in stone: a dark core is the platform itself, the courses sit on its faces
        ashlar(b, M, "x", -6, 44, -9, -1, -1.5, 0.0, rng, T=0.09, L0=1.1, joints=j, name="Retaining wall")
        ashlar(b, M, "y", -9, 18, -6, -1, -1.5, 0.0, rng, T=0.09, L0=1.1, joints=j, name="Retaining wall")
        ashlar(b, M, "x", -14, 46, -19, -1, -3.0, -1.5, rng, T=0.09, L0=1.4, joints=j, name="Retaining wall")
        ashlar(b, M, "y", -19, 24, -14, -1, -3.0, -1.5, rng, T=0.09, L0=1.4, joints=j, name="Retaining wall")
        b.sink = self.hill
        ashlar(b, M, "x", -14, 46, 18, -1, 0.0, 1.5, rng, T=0.09, L0=1.6, joints=False, name="Retaining wall")
        ashlar(b, M, "x", -14, 46, 26, -1, 1.5, 3.0, rng, T=0.09, L0=1.6, joints=False, name="Retaining wall")
        ashlar(b, M, "x", -14, 46, 34, -1, 3.0, 4.5, rng, T=0.09, L0=1.6, joints=False, name="Retaining wall")
        b.sink = None
        # wall copings
        b.box((-6.1, -9.12, -0.07), (44, -8.95, 0.0), M["coping"], "Coping")
        b.flush()

    def pool_terrace(self):
        b, M, rng = self.b, self.M, self.rng
        x0, x1, y0, y1 = POOL
        # the platform, the deck round the pool and the pool itself
        conc = M["core"]
        b.box((-6, -9, -1.5), (x0, 18, -0.05), conc, "Plinth")
        b.box((x1, -9, -1.5), (50, 18, -0.05), conc, "Plinth")
        b.box((x0, y1, -1.5), (x1, 18, -0.05), conc, "Plinth")
        b.box((x0, -9, -1.5), (x1, y0, -0.7), conc, "Overflow trough")
        b.box((x0, y0, -1.5), (x1, y1, -1.42), M["tile"], "Pool floor")
        # tile liners and the overflow edge
        b.box((x0, y0, -1.5), (x0 + 0.06, y1, -0.12), M["tile"], "Pool wall")
        b.box((x1 - 0.06, y0, -1.5), (x1, y1, -0.12), M["tile"], "Pool wall")
        b.box((x0, y1 - 0.06, -1.5), (x1, y1, -0.12), M["tile"], "Pool wall")
        b.box((x0, y0 - 0.06, -1.5), (x1, y0, -0.16), M["basalt"], "Overflow edge")
        b.box((x0 + 0.06, y0 - 0.1, -0.85), (x1 - 0.06, y0 - 0.06, -0.14), M["sheet"], "Overflow sheet")
        self.keys["pool"] = b.mark(b.box((x0 + 0.06, y0, -1.42), (x1 - 0.06, y1 - 0.06, -0.12), M["water"], "Pool water"))
        # sun shelf
        b.box((x0 + 0.06, -7.0, -1.42), (x0 + 3.4, -2.4, -0.5), M["tile"], "Sun shelf")
        # copings
        b.box((x0 - 0.4, y0 - 0.06, -0.05), (x0, y1 + 0.4, 0.0), M["coping"], "Pool coping")
        b.box((x1, y0 - 0.06, -0.05), (x1 + 0.4, y1 + 0.4, 0.0), M["coping"], "Pool coping")
        b.box((x0, y1, -0.05), (x1, y1 + 0.4, 0.0), M["coping"], "Pool coping")
        # decks: board arrays round the pool
        tm = ("timber", "timber2")
        boards(b, M, -6, x0 - 0.4, -9, 0, 0.0, tm, along="y")
        boards(b, M, x0 - 0.4, x1 + 0.4, y1 + 0.4, 0, 0.0, tm)
        boards(b, M, x1 + 0.4, 38, -9, 0, 0.0, tm, along="y")
        b.box((x0 - 0.4, 0, -0.05), (x1 + 0.4, 0.1, 0.0), M["floor"], "Threshold")
        b.flush()
        # pool lights: bars on the long wall and the shelf edge
        for k in range(9):
            xx = x0 + 1.6 + 2.15 * k
            b.box((xx, y1 - 0.12, -0.95), (xx + 0.9, y1 - 0.06, -0.75), M["g_pool"], "Pool light")
        for yy in (-7.6, -5.4, -3.4):
            b.box((x1 - 0.12, yy, -0.95), (x1 - 0.06, yy + 0.7, -0.75), M["g_pool"], "Pool light")
        for xx in (7.0, 10.0, 13.0, 16.0, 19.0):
            for yy in (-6.6, -3.6):
                b.box((xx, yy, -1.42), (xx + 0.7, yy + 0.5, -1.38), M["g_pool"], "Pool light")
        # steps and glass balustrade on the platform edge
        for k in range(3):
            top = -0.375 * (k + 1)
            b.box((28.6, -9.0 - 0.42 * (k + 1), -1.5), (31.4, -9.0 - 0.42 * k, top), M["stone3"], "Stone step")
            b.box((28.6, -19.0 - 0.42 * (k + 1), -3.0), (31.4, -19.0 - 0.42 * k, -1.5 - 0.375 * (k + 1)), M["stone3"], "Stone step")
        balustrade(b, M, "x", -5.8, x0 - 0.4, -8.95, 0.0)
        balustrade(b, M, "x", x1 + 0.4, 28.4, -8.95, 0.0)
        balustrade(b, M, "x", 31.6, 37.6, -8.95, 0.0)
        balustrade(b, M, "y", -8.9, 0.0, -5.9, 0.0)
        b.flush()

    # ---- the house -------------------------------------------------------------

    def shell(self):
        b, M, rng = self.b, self.M, self.rng
        j = not self.quick
        R, S = M["render"], M["steel"]
        # ground slab (limestone floor) and the plinth line
        b.box((0, 0, -0.3), (26, 10, 0), M["floor"], "Ground floor")
        b.box((26, 2.5, -0.3), (34, 10, 0), M["paving"], "Carport floor")
        # living pavilion W: stone west wall, glass south, white north, roof
        self.keys["stone"] = b.mark(b.box((0, 0, 0), (0.35, 10, Z_WR), M["core"], "Stone wall core"))
        ashlar(b, M, "y", 0, 10, 0, -1, 0, Z_WR, rng, T=0.08, L0=1.0, joints=j)
        b.box((0.35, 9.7, 0), (11.7, 10, Z_WR), R, "North wall")
        b.box((11.7, 0, 0), (12.0, 2.5, Z_WR), R, "Veranda wall")
        b.box((11.7, 2.5, Z_SLAB), (12.0, 8.0, Z_WR), M["walnut"], "Gallery wall")
        b.box((11.7, 8.0, 6.5), (12.0, 9.7, Z_WR), R, "Lintel")
        b.box((11.7, 9.7, Z_SLAB), (12.0, 10, Z_WR), R, "Wall")
        # roof of the pavilion
        b.box((-1.3, -1.6, Z_WR), (12.0, 10.3, Z_WT), R, "Pavilion roof")
        b.box((-1.3, -1.6, Z_WR - 0.045), (11.9, 10.2, Z_WR), M["timber2"], "Pavilion soffit")
        b.box((-1.1, -1.4, Z_WT), (11.8, 10.1, Z_WT + 0.05), M["gravel"], "Roof gravel")
        for x in (1.6, 5.0):
            b.box((x, 3.0, Z_WT + 0.05), (x + 2.4, 5.0, Z_WT + 0.3), M["steel"], "Rooflight frame")
            b.box((x + 0.1, 3.1, Z_WT + 0.3), (x + 2.3, 4.9, Z_WT + 0.33), M["glass"], "Rooflight")
        glass_wall(b, M, "x", 0.35, 11.7, 0.0, 0.0, Z_WR, 2.33, "Living glazing", transoms=(Z_UF, 5.2))
        # screen wall of stone in front of the west wall
        b.box((0, -5.6, 0), (0.35, 0, 3.0), M["core"], "Screen wall core")
        ashlar(b, M, "y", -5.6, 0, 0, -1, 0, 3.0, rng, T=0.08, L0=0.95, joints=j, name="Screen wall")
        ashlar(b, M, "y", -5.6, 0, 0.35, 1, 0, 3.0, rng, T=0.08, L0=0.95, joints=j, name="Screen wall")
        b.box((-0.1, -5.7, 3.0), (0.45, 0.0, 3.08), M["coping"], "Screen coping")
        b.flush()

        # kitchen and dining wing E (under the cantilever)
        b.box((12, 9.7, 0), (26, 10, Z_SLAB), R, "North wall E")
        b.box((26, 2.5, 0), (26.3, 10, Z_SLAB), M["core"], "Stone wall core")
        ashlar(b, M, "y", 2.5, 10, 26.3, 1, 0, Z_SLAB, rng, T=0.08, L0=1.0, joints=j)
        b.box((22, 2.5, 0), (26, 2.8, Z_SLAB), M["core"], "Stone wall core")
        ashlar(b, M, "x", 22, 26, 2.5, -1, 0, Z_SLAB, rng, T=0.08, L0=0.9, joints=j)
        glass_wall(b, M, "x", 12.0, 22.0, 2.5, 0.0, Z_SLAB, 2.5, "Dining glazing")
        # carport: stone back wall, bronze column
        b.box((26.3, 9.7, 0), (32, 10, Z_SLAB), M["core"], "Carport wall")
        ashlar(b, M, "x", 26.3, 32, 9.7, -1, 0, Z_SLAB, rng, T=0.08, L0=1.0, joints=j, name="Carport stone")
        b.box((31.78, 2.55, 0), (31.9, 2.67, Z_SLAB), S, "Carport column")
        b.flush()

        # bedroom volume U: slab, walls, roof (the cantilever)
        self.keys["cantilever"] = b.mark(b.box((12, -3, Z_SLAB), (32, 10, Z_UF), R, "Cantilever slab"))
        b.box((12, -3, Z_SLAB - 0.04), (32, 2.5, Z_SLAB), M["timber"], "Cantilever soffit")
        # soffit boards: 600 mm panels under the cantilever
        i = b.box((12.0, -2.98, Z_SLAB - 0.07), (12.6 - 0.02, 2.5, Z_SLAB - 0.04), M["timber2"], "Soffit board")
        b.copy(i, (0.6, 0, 0), 32)
        b.box((12, 8.0, Z_UF), (12.25, 9.7, Z_UF + 0.01), R, "Landing")
        b.box((12, -3, Z_UF), (12.25, 8.0, Z_UR), R, "West wall U")
        b.box((12, 9.7, Z_UF), (12.25, 10, Z_UR), R, "West wall U")
        b.box((31.75, -3, Z_UF), (32, 10, Z_UR), R, "East wall U")
        b.box((12.25, 9.7, Z_UF), (31.75, 10, Z_UR), R, "North wall U")
        b.box((24, -3, Z_UF), (24.25, -1.2, Z_UR), R, "Balcony return")
        b.box((24.25, -3, Z_UF), (31.75, -2.75, Z_UR), R, "South wall U")
        b.box((12.25, 8.0, 6.0), (12.4, 9.7, Z_UR), R, "Lintel U")
        # balcony: glazing set back, timber floor, glass balustrade on the slab edge
        glass_wall(b, M, "x", 12.25, 24.0, -1.2, Z_UF, Z_UR, 2.35, "Bedroom glazing")
        boards(b, M, 12.25, 24.0, -2.9, -1.26, Z_UF + 0.05, ("timber", "timber3"), pitch=0.15, w=0.13, name="Balcony board")
        balustrade(b, M, "x", 12.3, 24.0, -2.95, Z_UF + 0.05)
        balustrade(b, M, "y", -2.95, -1.3, 12.3, Z_UF + 0.05, pitch=1.65)
        # vertical timber fins over the closed end
        i = b.box((24.35, -3.2, Z_UF), (24.41, -3.0, Z_UR), M["timber"], "Timber fin")
        b.copy(i, (0.3, 0, 0), 24)
        i = b.box((24.5, -3.0, Z_UF), (24.53, -2.75, Z_UR), M["bronze"], "Fin rail")
        b.flush()
        # the roof of U: thin slab, deep overhang, timber soffit, gravel and terrace
        b.box((12.0, -4.2, Z_UR), (33.2, 11.2, Z_UT), R, "Bedroom roof")
        i = b.box((12.0, -4.18, Z_UR - 0.045), (12.6 - 0.02, -1.2, Z_UR), M["timber2"], "Roof soffit board")
        b.copy(i, (0.6, 0, 0), 35)
        b.box((31.7, -1.2, Z_UR - 0.045), (33.18, 11.18, Z_UR), M["timber2"], "Roof soffit")
        b.box((12.2, 3.2, Z_UT), (33.0, 11.0, Z_UT + 0.05), M["gravel"], "Roof gravel")
        b.box((12.2, -4.0, Z_UT), (22.4, 3.2, Z_UT + 0.05), M["gravel"], "Roof gravel")
        boards(b, M, 22.4, 33.0, -4.0, 3.2, Z_UT + 0.06, ("timber", "timber2"), pitch=0.17, w=0.15, name="Terrace board", thick=0.06)
        balustrade(b, M, "x", 12.4, 33.05, -4.05, Z_UT, h=1.05, pitch=2.1, name="Roof balustrade")
        balustrade(b, M, "y", -4.0, 11.0, 33.05, Z_UT, h=1.05, pitch=2.1, name="Roof balustrade")
        b.flush()
        # chimney stack of stone, standing in the living room and rising through the roof
        self.keys["chimney"] = b.mark(b.box((9.6, 3.8, 0), (11.4, 6.0, 9.9), M["core"], "Chimney core"))
        ashlar(b, M, "x", 9.6, 11.4, 3.8, -1, 0, 9.9, rng, T=0.07, L0=0.9, joints=j, name="Chimney stone")
        ashlar(b, M, "x", 9.6, 11.4, 6.0, 1, 0, 9.9, rng, T=0.07, L0=0.9, joints=j, name="Chimney stone")
        ashlar(b, M, "y", 3.8, 6.0, 9.6, -1, 0, 9.9, rng, T=0.07, L0=0.9, joints=j, name="Chimney stone")
        ashlar(b, M, "y", 3.8, 6.0, 11.4, 1, 0, 9.9, rng, T=0.07, L0=0.9, joints=j, name="Chimney stone")
        b.box((9.45, 3.65, 9.9), (11.55, 6.15, 10.05), S, "Chimney cap")
        b.box((9.9, 4.0, 10.05), (11.1, 5.8, 10.35), M["core"], "Flue")
        b.flush()

    def facade_details(self):
        b, M = self.b, self.M
        # cove lights: warm strips tucked against the walls above the glass
        for x in (0.6, 3.5, 6.4, 9.3):
            b.box((x, -0.36, Z_WR - 0.07), (x + 2.8, -0.26, Z_WR - 0.045), M["g_cove"], "Cove light")
        for x in (12.4, 17.4, 22.4, 27.4):
            b.box((x, -2.62, Z_SLAB - 0.07), (x + 4.8, -2.52, Z_SLAB - 0.045), M["g_cove"], "Cove light")
        for y in (-1.2, 2.2):
            b.box((12.12, y, Z_SLAB - 0.07), (12.22, y + 3.4, Z_SLAB - 0.045), M["g_cove"], "Cove light")
        for x in (12.5, 16.7, 20.9):
            b.box((x, -3.4, Z_UR - 0.07), (x + 4.0, -3.3, Z_UR - 0.045), M["g_cove"], "Cove light")
        b.box((24.4, -3.9, Z_UR - 0.07), (32, -3.8, Z_UR - 0.045), M["g_cove"], "Cove light")
        b.flush()

    # ---- interior -------------------------------------------------------------

    def interior(self):
        b, M = self.b, self.M

        def sofa(x0, y0, w, d, facing, key="sofa"):
            """A sofa with its back on the side opposite ``facing`` (+y or -y)."""
            seat, back = 0.44, 0.9
            if facing > 0:
                by0, by1 = y0, y0 + 0.28
            else:
                by0, by1 = y0 + d - 0.28, y0 + d
            b.box((x0, y0, 0.08), (x0 + w, y0 + d, seat), M[key], "Sofa seat")
            b.box((x0, by0, 0.08), (x0 + w, by1, back), M[key], "Sofa back")
            for ax in (x0, x0 + w - 0.22):
                b.box((ax, y0, 0.08), (ax + 0.22, y0 + d, 0.62), M[key], "Sofa arm")
            b.box((x0 + 0.1, y0 + 0.05, 0.0), (x0 + 0.2, y0 + 0.15, 0.08), M["bronze"], "Sofa foot")
            b.box((x0 + w - 0.2, y0 + d - 0.15, 0.0), (x0 + w - 0.1, y0 + d - 0.05, 0.08), M["bronze"], "Sofa foot")
            n = max(2, int((w - 0.5) / 0.7))
            cw = (w - 0.44) / n
            cy = (y0 + 0.3) if facing > 0 else (y0 + 0.02)
            i = b.box((x0 + 0.22 + 0.01, cy, seat), (x0 + 0.22 + cw - 0.01, cy + d - 0.32, seat + 0.1), M[key], "Cushion")
            b.copy(i, (cw, 0, 0), n - 1)

        # living room: rug, two sofas, table, armchairs, lamp, art, fig
        b.box((2.8, 1.0, 0.0), (9.2, 6.8, 0.014), M["rug"], "Rug")
        sofa(3.9, 5.0, 4.4, 1.0, -1)
        sofa(3.9, 1.5, 4.4, 1.0, 1)
        b.box((5.2, 3.2, 0.32), (7.0, 4.4, 0.37), M["marble"], "Coffee table top")
        b.box((5.3, 3.3, 0.0), (5.36, 4.3, 0.32), M["bronze"], "Coffee table leg")
        b.box((6.84, 3.3, 0.0), (6.9, 4.3, 0.32), M["bronze"], "Coffee table leg")
        b.box((8.5, 3.1, 0.0), (9.3, 3.9, 0.46), M["leather"], "Armchair")
        b.box((9.1, 3.1, 0.46), (9.3, 3.9, 0.95), M["leather"], "Armchair back")
        b.box((8.5, 4.4, 0.0), (9.3, 5.2, 0.46), M["leather"], "Armchair")
        b.box((9.1, 4.4, 0.46), (9.3, 5.2, 0.95), M["leather"], "Armchair back")
        b.cyl((3.3, 6.2, 0.0), 0.012, 1.5, M["bronze"], "Floor lamp")
        b.sph((3.3, 6.2, 1.62), 0.22, M["g_warm"], "Floor lamp shade", seg=12)
        b.box((3.0, 9.62, 1.9), (6.2, 9.7, 4.7), M["art"], "Canvas")
        b.box((6.6, 9.62, 2.4), (8.8, 9.7, 4.4), M["art2"], "Canvas")
        b.box((0.9, 0.5, 0.0), (1.5, 1.1, 0.45), M["charcoal"], "Planter")
        b.cyl((1.2, 0.8, 0.45), 0.03, 1.2, M["bark"], "Fig trunk", seg=8)
        for dz, rr in ((1.9, 0.5), (1.55, 0.45), (2.2, 0.4)):
            b.sph((1.2 + 0.1 * (dz - 1.9), 0.8, dz), rr, M["leaf"], "Fig leaves", seg=10)
        # fire in the chimney: bronze surround, glowing flames
        b.box((9.4, 4.3, 0.35), (9.6, 5.5, 1.35), M["steel"], "Fire surround")
        b.box((9.38, 4.45, 0.45), (9.4, 5.35, 1.0), M["g_fire"], "Flames")
        b.box((9.3, 3.8, 0.0), (9.6, 4.3, 0.35), M["bronze"], "Log store")
        b.flush()

        # staircase: treads copied up the slope on a steel stringer, glass on the open side
        xa, rise, run = 6.87, 0.175, 0.27
        ang = math.degrees(math.atan2(rise, run))
        i = b.box((xa, 8.35, rise - 0.06), (xa + 0.3, 9.65, rise), M["walnut"], "Stair tread")
        b.copy(i, (run, 0, rise), 18)
        ls = (12.0 - xa) / math.cos(math.radians(ang))
        i = b.box((xa, 8.96, rise - 0.06 - 0.22), (xa + ls, 9.04, rise - 0.06), M["steel"], "Stair stringer")
        self.keys["stair"] = b.mark(i)
        b.rot(i, (0, 1, 0), -ang, (xa, 9.0, rise - 0.06))
        i = b.box((xa, 8.33, rise), (xa + ls - 0.15, 8.354, rise + 0.95), M["glass"], "Stair glass")
        b.rot(i, (0, 1, 0), -ang, (xa, 8.34, rise))
        i = b.box((xa, 8.325, rise + 0.95), (xa + ls - 0.15, 8.36, rise + 0.975), M["steel"], "Stair handrail")
        b.rot(i, (0, 1, 0), -ang, (xa, 8.34, rise))
        b.flush()

        # dining: walnut table, eight chairs, three pendants
        chair, stool = self.chair, self.stool
        b.box((13.6, 3.5, 0.72), (18.4, 4.7, 0.78), M["walnut"], "Dining table")
        for xx in (13.8, 18.0):
            b.box((xx, 3.55, 0.0), (xx + 0.14, 4.65, 0.72), M["steel"], "Table leg")
        for k in range(4):
            xx = 14.2 + 1.35 * k
            b.place(chair, xx, 3.0, 0.0, yaw=90)
            b.place(chair, xx, 5.2, 0.0, yaw=-90)
        b.place(chair, 13.0, 4.1, 0.0, yaw=0)
        b.place(chair, 19.0, 4.1, 0.0, yaw=180)
        for xx, h in ((14.6, 1.95), (16.0, 1.75), (17.4, 1.95)):
            self._pendant(xx, 4.1, h, 0.17)
        for k, (dx, dy, h, r) in enumerate(((0, 0, 3.6, 0.2), (0.7, 0.4, 4.4, 0.16), (-0.6, 0.5, 5.1, 0.18), (0.4, -0.7, 4.0, 0.15),
                                            (-0.8, -0.4, 3.2, 0.17), (1.1, -0.2, 5.4, 0.14), (-0.2, 0.9, 2.9, 0.15))):
            self._pendant(6.2 + dx, 3.9 + dy, h, r)
        # ceiling coves in the kitchen and dining wing
        for x0, x1 in ((12.5, 18.6), (18.9, 25.5)):
            b.box((x0, 3.0, Z_SLAB - 0.05), (x1, 3.16, Z_SLAB - 0.01), M["g_cove"], "Ceiling cove")
            b.box((x0, 8.3, Z_SLAB - 0.05), (x1, 8.46, Z_SLAB - 0.01), M["g_cove"], "Ceiling cove")
        # kitchen: island, run of cabinets, tall wall, stools
        self.keys["island"] = b.mark(b.box((14.6, 6.5, 0.0), (19.4, 7.7, 0.9), M["walnut"], "Island"))
        b.box((14.4, 6.35, 0.9), (19.6, 7.85, 0.96), M["marble"], "Island top")
        for k in range(4):
            b.place(stool, 15.2 + 1.1 * k, 5.95, 0.0)
        b.box((13.0, 8.7, 0.0), (22.0, 9.35, 0.9), M["timber3"], "Kitchen run")
        b.box((13.0, 8.65, 0.9), (22.0, 9.35, 0.95), M["marble"], "Kitchen counter")
        b.box((12.3, 9.35, 0.0), (25.7, 9.7, 2.9), M["walnut"], "Tall cabinets")
        i = b.box((12.9, 9.33, 0.0), (12.93, 9.35, 2.9), M["steel"], "Cabinet joint")
        b.copy(i, (0.65, 0, 0), 19)
        for xx, h in ((15.2, 1.95), (16.9, 1.8), (18.6, 1.95)):
            self._pendant(xx, 7.1, h, 0.15)
        # east nook: console, art, two chairs
        b.box((22.6, 4.2, 0.0), (23.4, 6.4, 0.5), M["charcoal"], "Lounge sofa")
        b.box((22.6, 4.2, 0.5), (22.9, 6.4, 0.95), M["charcoal"], "Lounge sofa back")
        b.box((24.0, 5.0, 0.0), (24.5, 5.5, 0.4), M["marble"], "Side table")
        b.flush()

        # first floor: bedroom
        b.box((13.0, -1.0, Z_UF), (23.6, 9.4, Z_UF + 0.012), M["floor"], "Bedroom floor")
        b.box((16.0, 0.4, Z_UF + 0.012), (22.0, 5.2, Z_UF + 0.026), M["rug"], "Bedroom rug")
        b.box((16.9, 4.95, Z_UF), (21.1, 5.1, Z_UF + 2.35), M["walnut"], "Headboard wall")
        self.keys["bed"] = b.mark(b.box((18.2, 2.7, Z_UF + 0.012), (19.8, 4.95, Z_UF + 0.45), M["walnut"], "Bed base"))
        b.box((18.15, 2.65, Z_UF + 0.45), (19.85, 4.9, Z_UF + 0.72), M["linen"], "Mattress")
        b.box((18.15, 2.75, Z_UF + 0.72), (19.85, 3.55, Z_UF + 0.76), M["sofa"], "Duvet")
        for xx in (18.35, 19.15):
            b.box((xx, 4.3, Z_UF + 0.72), (xx + 0.55, 4.8, Z_UF + 0.84), M["linen"], "Pillow")
        for xx in (17.4, 20.1):
            b.box((xx, 4.4, Z_UF + 0.012), (xx + 0.5, 4.9, Z_UF + 0.42), M["walnut"], "Bedside table")
            b.sph((xx + 0.25, 4.65, Z_UF + 0.62), 0.12, M["g_warm"], "Bedside lamp", seg=10)
        b.box((14.0, 0.2, Z_UF + 0.012), (14.8, 1.0, Z_UF + 0.46), M["leather"], "Lounge chair")
        b.box((14.0, 0.2, Z_UF + 0.46), (14.2, 1.0, Z_UF + 0.95), M["leather"], "Lounge chair back")
        b.box((15.0, 0.4, Z_UF + 0.012), (15.4, 0.8, Z_UF + 0.5), M["marble"], "Side table")
        b.box((12.5, 7.0, Z_UF), (14.5, 9.4, Z_UF + 2.4), M["walnut"], "Wardrobe")
        self._pendant(19.0, 1.2, Z_UR - 0.35, 0.14)
        b.flush()

    def _chair(self):
        b, M = self.b, self.M
        p = [b.box((-0.2, -0.2, 0.43), (0.2, 0.2, 0.47), M["leather"], "Seat"),
             b.box((-0.2, -0.2, 0.47), (-0.17, 0.2, 0.9), M["leather"], "Back")]
        for sx in (-0.19, 0.17):
            for sy in (-0.19, 0.17):
                p.append(b.box((sx, sy, 0.0), (sx + 0.025, sy + 0.025, 0.43), M["bronze"], "Leg"))
        return p

    def _stool(self):
        b, M = self.b, self.M
        return [b.cyl((0, 0, 0.62), 0.17, 0.05, M["leather"], "Stool seat", seg=14),
                b.cyl((0, 0, 0.0), 0.02, 0.62, M["bronze"], "Stool stem", seg=8),
                b.cyl((0, 0, 0.0), 0.15, 0.02, M["bronze"], "Stool foot", seg=12)]

    def _pendant(self, x, y, z, r):
        """A glowing globe hung on a cable from the ceiling above it."""
        b, M = self.b, self.M
        top = Z_SLAB if z < 3.2 else (Z_UR if z < 6.6 else Z_WR)
        b.cyl((x, y, z + r), 0.004, top - z - r, M["bronze"], "Pendant cable", seg=6)
        b.sph((x, y, z), r, M["g_pend"], "Pendant", seg=12)

    # ---- outdoors ---------------------------------------------------------------

    def veranda(self):
        b, M, rng = self.b, self.M, self.rng
        chair, stool = self.chair, self.stool
        # outdoor kitchen against the stone wall under the cantilever
        self.keys["kitchen"] = b.mark(b.box((22.2, 1.55, 0.0), (25.9, 2.42, 0.88), M["basalt"], "Outdoor kitchen"))
        b.box((22.1, 1.5, 0.88), (26.0, 2.42, 0.94), M["marble"], "Outdoor kitchen top")
        b.box((22.2, 1.5, 0.05), (25.9, 1.54, 0.12), M["g_cove"], "Toe light")
        b.box((22.9, 1.75, 0.94), (23.9, 2.25, 1.12), M["steelb"], "Grill")
        b.box((22.85, 1.7, 1.12), (23.95, 1.78, 1.2), M["steel"], "Grill lid")
        b.box((24.5, 1.8, 0.94), (25.2, 2.2, 0.99), M["steelb"], "Sink")
        for k in range(3):
            b.place(stool, 22.8 + 1.1 * k, 1.0, 0.0)
        # outdoor dining on the west deck
        b.box((-4.4, -4.5, 0.72), (-2.0, -3.4, 0.76), M["timber"], "Outdoor table")
        for xx in (-4.2, -2.3):
            b.box((xx, -4.45, 0.0), (xx + 0.08, -3.45, 0.72), M["steel"], "Table leg")
        for k in range(3):
            b.place(chair, -4.0 + 0.85 * k, -5.0, 0.0, yaw=90)
            b.place(chair, -4.0 + 0.85 * k, -2.9, 0.0, yaw=-90)
        # a low outdoor sofa group on the veranda
        b.box((13.0, 0.3, 0.0), (17.4, 1.2, 0.42), M["charcoal"], "Veranda sofa")
        b.box((13.0, 0.3, 0.42), (17.4, 0.55, 0.8), M["charcoal"], "Veranda sofa back")
        i = b.box((13.05, 0.55, 0.42), (14.25, 1.15, 0.5), M["sofa"], "Veranda cushion")
        b.copy(i, (1.45, 0, 0), 2)
        b.box((14.2, 1.55, 0.0), (16.2, 2.35, 0.34), M["marble"], "Veranda table")
        b.flush()
        # loungers and parasols on the east deck
        lounger = b.comp("Lounger", lambda: self._lounger())
        for k in range(4):
            b.place(lounger, 25.4 + 1.55 * k, -7.2, 0.0, yaw=0)
        for x in (26.2, 29.3):
            b.cyl((x, -5.7, 0.0), 0.025, 2.5, M["bronze"], "Parasol pole", seg=8)
            b.cone((x, -5.7, 2.15), 1.7, 0.4, M["linen"], "Parasol", top=0.04, seg=20)
        # a stone fire pit in the lower garden, ringed with a timber-topped bench
        fx, fy, fz = 12.0, -14.3, -1.5
        b.cyl((fx, fy, fz), 3.7, 0.04, M["gravel"], "Fire pit gravel", seg=40)
        b.add(type="add_tube", base=[fx, fy, fz], radius=3.0, inner_radius=2.45, height=0.42, segments=40,
              name="Fire pit bench", material=M["stone2"].id)
        b.add(type="add_tube", base=[fx, fy, fz + 0.42], radius=3.06, inner_radius=2.4, height=0.05, segments=40,
              name="Bench seat", material=M["timber"].id)
        self.keys["firepit"] = b.mark(b.cyl((fx, fy, fz), 0.7, 0.4, M["steel"], "Fire bowl", seg=24))
        b.cone((fx, fy, fz + 0.4), 0.5, 0.65, M["g_fire"], "Flame", top=0.04, seg=10)
        b.cyl((fx, fy, fz + 0.4), 0.55, 0.04, M["embers"], "Embers", seg=20)
        # stepping stones from the lower steps across the lawn
        pts = [(29.9, -25.0), (28.0, -26.6), (25.0, -27.4), (22.0, -26.5), (19.6, -24.4), (17.5, -22.2)]
        for k, (x, y) in enumerate(pts):
            i = b.box((x - 0.7, y - 0.4, -3.0), (x + 0.7, y + 0.4, -2.94), M["stone3"], "Stepping stone")
            b.rot(i, (0, 0, 1), rng.uniform(-25, 25), (x, y, -3.0))
        b.flush()

    def _lounger(self):
        b, M = self.b, self.M
        p = [b.box((0, 0, 0.2), (0.75, 1.3, 0.3), M["linen"], "Lounger seat")]
        back = b.box((0, 1.3, 0.2), (0.75, 2.05, 0.3), M["linen"], "Lounger back")
        b.rot(back, (1, 0, 0), 38, (0, 1.3, 0.3))
        p.append(back)
        p.append(b.box((0.02, 0.0, 0.12), (0.73, 2.0, 0.2), M["timber"], "Lounger frame"))
        for sx in (0.02, 0.68):
            for sy in (0.05, 1.9):
                p.append(b.box((sx, sy, 0.0), (sx + 0.05, sy + 0.05, 0.12), M["bronze"], "Lounger foot"))
        return p

    def pergola(self):
        b, M = self.b, self.M
        zb = Z_UT + 0.06
        for x in (22.6, 27.0, 31.2):
            for y in (-3.4, 3.0):
                b.box((x - 0.06, y - 0.06, zb), (x + 0.06, y + 0.06, zb + 2.55), M["bronze"], "Pergola post")
        for y in (-3.4, 3.0):
            self.keys["pergola"] = b.mark(b.box((22.4, y - 0.06, zb + 2.55), (31.4, y + 0.06, zb + 2.78), M["bronze"], "Pergola beam"))
        for y in (-3.4, 3.0):
            b.box((22.8, y - 0.03, zb + 2.5), (31.0, y + 0.03, zb + 2.55), M["g_cove"], "Pergola light")
        i = b.box((22.5, -3.7, zb + 2.78), (22.57, 3.3, zb + 2.94), M["timber"], "Pergola slat")
        b.copy(i, (0.27, 0, 0), 32)
        # roof terrace: two loungers, a table, planters of grasses
        lounger = b.comp("Roof lounger", lambda: self._lounger())
        b.place(lounger, 24.0, -2.6, zb, yaw=90)
        b.place(lounger, 24.0, -1.4, zb, yaw=90)
        b.box((26.2, -2.5, zb), (27.4, -1.3, zb + 0.4), M["charcoal"], "Roof table base")
        b.cyl((26.8, -1.9, zb + 0.4), 0.7, 0.05, M["marble"], "Roof table top", seg=24)
        for x in (29.0, 30.3):
            b.box((x, -3.5, zb), (x + 1.0, -2.8, zb + 0.55), M["render"], "Roof planter")
        b.flush()
        tuft = self.tuft
        for k in range(10):
            b.place(tuft, 29.2 + 0.1 * (k % 5) + (k // 5) * 1.3, -3.4 + 0.1 * (k // 5 + k % 3), zb + 0.55,
                    yaw=self.rng.uniform(0, 360), s=self.rng.uniform(0.8, 1.2))
        b.flush()

    def parts_libraries(self):
        """The planting components, made once at the origin and hidden."""
        b, M = self.b, self.M

        def cypress(r, h, key):
            def f():
                return [b.cyl((0, 0, 0), 0.1, 0.8, M["bark"], "Trunk", seg=8),
                        b.cone((0, 0, 0.4), r, h, M[key], "Cypress", top=0.04, seg=14)]
            return f

        self.cypress = b.comp("Cypress", cypress(0.78, 7.0, "cypress"))
        self.cypress2 = b.comp("Cypress, slim", cypress(0.55, 8.6, "cypress2"))

        def plane():
            p = [b.cyl((0, 0, 0), 0.16, 3.4, M["bark"], "Trunk", seg=10),
                 b.cyl((0, 0, 2.6), 0.09, 1.9, M["bark"], "Limb", axis=(0.6, 0.2, 1), seg=8),
                 b.cyl((0, 0, 2.6), 0.08, 1.8, M["bark"], "Limb", axis=(-0.5, -0.4, 1), seg=8)]
            for k in range(12):
                a = math.radians(k * 137.5)
                rad = 0.6 + 1.5 * ((k * 7) % 5) / 4.0
                z = 4.3 + 1.6 * (k % 4) / 3.0 + 0.35 * math.sin(k)
                r = 0.95 + 0.4 * ((k * 3) % 4) / 3.0
                sp = b.sph((rad * math.cos(a), rad * math.sin(a), z), r, M[("leaf", "leaf2", "leaf3")[k % 3]], "Canopy", seg=12)
                b.scale(sp, (1.1, 1.1, 0.8), (rad * math.cos(a), rad * math.sin(a), z))
                p.append(sp)
            return p
        self.plane = b.comp("Plane tree", plane)

        def olive():
            p = [b.cyl((0, 0, 0), 0.19, 1.2, M["bark"], "Trunk", seg=8),
                 b.cyl((0, 0, 1.0), 0.12, 1.5, M["bark"], "Limb", axis=(0.5, 0.1, 1), seg=8),
                 b.cyl((0, 0, 1.0), 0.11, 1.4, M["bark"], "Limb", axis=(-0.5, -0.2, 1), seg=8),
                 b.cyl((0, 0, 1.0), 0.09, 1.2, M["bark"], "Limb", axis=(0.0, 0.6, 1), seg=8)]
            for k in range(8):
                a = math.radians(k * 137.5 + 20)
                rad = 0.4 + 0.9 * ((k * 5) % 4) / 3.0
                z = 2.5 + 0.6 * (k % 3) / 2.0
                x, y = rad * math.cos(a), rad * math.sin(a)
                sp = b.sph((x, y, z), 0.8 + 0.2 * (k % 2), M[("olive", "olive2")[k % 2]], "Olive canopy", seg=12)
                b.scale(sp, (1.15, 1.15, 0.6), (x, y, z))
                p.append(sp)
            return p
        self.olive = b.comp("Olive", olive)

        def palm():
            p = []
            ox = oy = 0.0
            z = 0.0
            for k in range(4):
                dx, dy = 0.10 * (k + 1) * 0.5, 0.05 * (k + 1)
                r0, r1 = 0.27 - 0.03 * k, 0.24 - 0.03 * k
                p.append(b.cone((ox, oy, z), r0, 1.7, M["palmtrunk"], "Palm trunk", top=r1, axis=(dx, dy, 1.7), seg=10))
                ox, oy, z = ox + dx, oy + dy, z + 1.7
            top = (ox, oy, z)
            widths = (0.0, 0.34, 0.3, 0.14, 0.0)
            L = 0.78
            for k in range(15):
                yaw = k * 24.0 + (k % 3) * 5
                xj, zj, th = 0.0, 0.0, (-16, 4, 20)[k % 3]
                for sg in range(4):
                    w0, w1 = widths[sg], widths[sg + 1]
                    if w0 == 0:
                        pts = [(xj, 0), (xj + L, w1), (xj + L, -w1)]
                    elif w1 == 0:
                        pts = [(xj, w0), (xj + L, 0), (xj, -w0)]
                    else:
                        pts = [(xj, w0), (xj + L, w1), (xj + L, -w1), (xj, -w0)]
                    g = b.poly([(top[0] + q[0], top[1] + q[1], top[2] + zj) for q in pts], 0.02, M["frond"], "Frond")
                    b.rot(g, (0, 1, 0), th, (top[0] + xj, top[1], top[2] + zj))
                    b.rot(g, (0, 0, 1), yaw, top)
                    p.append(g)
                    xj += L * math.cos(math.radians(th))
                    zj -= L * math.sin(math.radians(th))
                    th += 15 + 8 * sg
            return p
        self.palm = b.comp("Palm", palm)

        def tuft(key):
            def f():
                p = []
                for k in range(9):
                    a = k * 40.0
                    ra = math.radians(a)
                    p.append(b.cone((0.06 * math.cos(ra), 0.06 * math.sin(ra), 0), 0.03, 0.55 + 0.05 * (k % 4), M[key], "Grass",
                                    axis=(0.32 * math.cos(ra), 0.32 * math.sin(ra), 1), seg=5))
                return p
            return f

        self.tuft = b.comp("Grass tuft", tuft("grass"))
        self.tuft2 = b.comp("Grass tuft, green", tuft("grass2"))
        self.chair = b.comp("Dining chair", lambda: self._chair())
        self.stool = b.comp("Bar stool", lambda: self._stool())

    def landscape(self):
        b, M, rng = self.b, self.M, self.rng
        tree_idx = []

        def put(d, x, y, z, s0, s1, yawr=True):
            s = rng.uniform(s0, s1)
            tree_idx.append(b.mark(b.place(d, x, y, z, yaw=rng.uniform(0, 360) if yawr else 0, s=s,
                                           sz=s * rng.uniform(0.92, 1.1))))
            b.tick(500)

        # cypresses: a column row at the back of the platform, clusters on the terraces, one framing the view
        for x, y in ((-4.5, 14.2), (-2.0, 14.5), (0.5, 14.3), (3.0, 14.6), (6.0, 14.2), (9.0, 14.5), (28.0, 13.8),
                     (31.0, 14.2), (34.0, 13.9), (37.0, 14.4), (40.5, 14.0)):
            put(self.cypress2 if rng.random() < 0.5 else self.cypress, x, y, -0.05, 0.75, 1.2)
        for x, y, z in ((-12.0, 8.0, -1.55), (-11.0, -9.0, -1.55), (-12.0, -1.0, -1.55), (47.0, -17.0, -1.55),
                        (50.0, -12.0, -1.55), (-27.0, -30.0, -3.0), (-30.5, -27.0, -3.0)):
            put(self.cypress, x, y, z, 0.55, 0.8)
        for x, y, z in ((-8, 21, 1.5), (4, 22.5, 1.5), (17, 21.5, 1.5), (36, 22, 1.5), (44, 23, 1.5),
                        (-2, 29, 3.0), (14, 30, 3.0), (26, 29.5, 3.0), (42, 30, 3.0)):
            put(self.cypress if rng.random() < 0.6 else self.cypress2, x, y, z, 0.9, 1.4)
        # broad canopy trees on the terraces behind and at the sides
        for x, y, z in ((-11, 3.0, -1.5), (-11, 14, -1.5), (1.0, 23.5, 1.5), (23, 23, 1.5), (10, 29, 3.0), (33, 28.5, 3.0),
                        (-5, 37, 4.5), (18, 39, 4.5), (38, 38, 4.5), (47, 4.0, -0.05), (47, -3, -0.05), (-3, 22, 1.5),
                        (41, -18.5, -1.5), (52, -24, -3.0)):
            put(self.plane, x, y, z, 0.7, 1.05)
        for k in range(22):
            put(self.plane if rng.random() < 0.55 else self.cypress, -50 + 6.5 * k + rng.uniform(-2, 2), rng.uniform(62, 80), 9.0, 1.2, 2.0)
        for x, y in ((56, 28), (64, 27), (72, 29), (60, 40), (70, 38), (80, 40)):
            put(self.plane if rng.random() < 0.5 else self.cypress, x, y, 4.5 if y > 34 else 3.0, 1.0, 1.5)
        # olives on the lawns
        for x, y, z in ((-30, -34, -3.0), (28, -36, -3.0), (40, -32, -3.0), (31, -14, -1.5), (-24, -14, -3.0),
                        (48, -9, -1.5), (14, 11.0, -0.05)):
            put(self.olive, x, y, z, 0.8, 1.1)
        # palms round the pool deck
        for x, y in ((24.5, -8.3), (36.0, -8.0), (36.4, -1.5)):
            put(self.palm, x, y, 0.0, 0.85, 1.1)
        b.flush()
        self.tree_ids = [b.placed(t) for t in tree_idx]
        # hedges: boxes with rounded edges
        hedges = (((-5.6, 12.0, -0.05), (41, 13.2, 1.4)), 
                  ((-5.6, -8.7, -0.05), (-4.7, 0, 0.9)), ((34.5, -0.9, -0.05), (38, -0.2, 0.7)))
        for p0, p1 in hedges:
            i = b.box(p0, p1, M["hedge"], "Hedge")
            b.round_edges(i, min(0.14, (min(p1[0] - p0[0], p1[1] - p0[1]) - 0.02) / 2.2))
        # planting beds with ornamental grasses
        beds = (((0.5, -5.5), (2.0, -0.5), -0.05, "grass"), ((34.0, -7.5), (38.0, -2.0), 0.0, "grass2"),
                ((26.5, -9.0 + 0.2), (31.0, -8.4 + 0.2), 0.0, "grass"), ((-5.0, 4.0), (-1.0, 11.0), -0.05, "grass2"),
                ((26.5, 10.6), (40, 12.0), -0.05, "grass"))
        for (x0, y0), (x1, y1), z, key in beds:
            b.box((x0, y0, z), (x1, y1, z + 0.12), M["soil"], "Planting bed")
            d = self.tuft if key == "grass" else self.tuft2
            n = max(6, int((x1 - x0) * (y1 - y0) * 1.6))
            for k in range(n):
                b.place(d, rng.uniform(x0 + 0.2, x1 - 0.2), rng.uniform(y0 + 0.2, y1 - 0.2), z + 0.1,
                        yaw=rng.uniform(0, 360), s=rng.uniform(0.7, 1.4))
            b.tick(500)
        b.flush()
        # low planting along the foot of the retaining walls: grasses and clipped boxwood
        for (x0, x1), y, z, key in (((-3.0, 25.0), -11.6, -1.5, "grass"), ((33.0, 45.0), -11.6, -1.5, "grass2"),
                                    ((-11.0, 26.0), -21.4, -3.0, "grass2"), ((33.0, 44.0), -21.4, -3.0, "grass")):
            b.box((x0, y - 0.9, z), (x1, y + 0.9, z + 0.1), M["soil"], "Planting bed")
            d = self.tuft if key == "grass" else self.tuft2
            n = int((x1 - x0) * 1.1)
            for k in range(n):
                x = x0 + 0.5 + (x1 - x0 - 1.0) * k / max(1, n - 1)
                if k % 4 == 3:
                    bx = b.sph((x, y + rng.uniform(-0.3, 0.3), z + 0.42), 0.46, M["hedge"], "Boxwood", seg=12)
                    b.scale(bx, (1.0, 1.0, 0.85), (x, y, z + 0.42))
                else:
                    b.place(d, x, y + rng.uniform(-0.55, 0.55), z + 0.08, yaw=rng.uniform(0, 360), s=rng.uniform(0.9, 1.5))
            b.tick(500)
        b.flush()
        # gravel court behind the house, a drive to the carport
        b.box((-6, 10.3, -0.05), (50, 12.0, 0.0), M["gravel"], "Gravel")
        b.box((-6, 13.2, -0.05), (50, 18.0, 0.0), M["gravel"], "Gravel")
        b.box((32, 2.5, -0.05), (50, 10.3, 0.0), M["asphalt"], "Drive")
        b.flush()

    def lights(self):
        """Garden bollards along the path and the drive, uplights under the trees."""
        b, M = self.b, self.M
        for x, y, z in ((27.6, -10.1, -0.05), (32.4, -10.1, -0.05), (27.6, -20.1, -1.5), (32.4, -20.1, -1.5),
                        (36, 6.5, 0.0), (42, 6.5, 0.0), (46, 6.5, 0.0), (34, 11.0, 0.0)):
            b.cyl((x, y, z), 0.07, 0.7, M["bronze"], "Bollard", seg=12)
            b.cyl((x, y, z + 0.7), 0.075, 0.1, M["g_bol"], "Bollard light", seg=12)
        for k in range(14):  # wall washers at the foot of the two retaining walls
            x = -7.0 + 3.9 * k
            b.box((x, -9.75, -1.5), (x + 0.9, -9.62, -1.45), M["g_up"], "Wall washer")
        for k in range(10):
            x = -11.0 + 5.2 * k
            b.box((x, -19.75, -3.0), (x + 0.9, -19.62, -2.95), M["g_up"], "Wall washer")
        for y in (1.2, 4.2, 7.2):  # washers on the stone west wall
            b.box((-0.75, y, -0.05), (-0.62, y + 1.0, 0.0), M["g_up"], "Wall washer")
        for k in range(8):  # bollards along the lower lawn
            x = -8.0 + 7.0 * k
            b.cyl((x, -17.0, -1.5), 0.07, 0.7, M["bronze"], "Bollard", seg=12)
            b.cyl((x, -17.0, -0.8), 0.075, 0.1, M["g_bol"], "Bollard light", seg=12)
        for x, y, z in ((24.5, -8.3, 0.0), (36.0, -8.0, 0.0), (-24, -14, -3.0), (31, -14, -1.5),
                        (-10, -15, -1.5), (-10, 3.0, -1.5), (28, -36, -3.0), (40, -32, -3.0),
                        (9, 14.0, 0.0), (0.5, 14.0, 0.0), (38, 13.8, 0.0), (28.0, 13.6, 0.0)):
            b.box((x - 0.25, y + 0.5, z), (x + 0.25, y + 1.0, z + 0.05), M["g_up"], "Uplight")
        b.flush()

    def car(self):
        b, M = self.b, self.M
        x0, y0 = 26.9, 4.6
        z0 = 0.0
        body = [(0.0, 0.42), (0.05, 0.66), (0.8, 0.84), (1.55, 0.95), (3.5, 0.95), (4.25, 0.9), (4.8, 0.78), (4.8, 0.42),
                (4.3, 0.34), (0.45, 0.34)]
        i = b.poly([(x0 + x, y0, z0 + z) for x, z in body], 1.9, M["carbody"], "Car body")
        cab = [(1.5, 0.95), (2.2, 1.38), (3.0, 1.4), (3.85, 0.95)]
        b.poly([(x0 + x, y0 + 0.14, z0 + z) for x, z in cab], 1.62, M["steel"], "Car glass")
        b.box((x0 + 2.2, y0 + 0.16, z0 + 1.36), (x0 + 3.0, y0 + 1.74, z0 + 1.42), M["carbody"], "Car roof")
        for dx in (0.95, 3.75):
            for dy in (0.0, 1.66):
                b.cyl((x0 + dx, y0 + dy, z0 + 0.34), 0.34, 0.24, M["tyre"], "Wheel", axis=(0, 1, 0), seg=18)
        b.box((x0 + 4.78, y0 + 0.2, z0 + 0.62), (x0 + 4.83, y0 + 1.7, z0 + 0.7), M["g_warm"], "Headlight")
        b.box((x0 - 0.03, y0 + 0.2, z0 + 0.6), (x0 + 0.02, y0 + 1.7, z0 + 0.66), M["g_fire"], "Tail light")
        b.flush()


# ---------------------------------------------------------------- main


def build(v, log=print):
    t = time.time()

    def lap(label):
        nonlocal t
        log("  %-26s %5.1f s  (%d commands)" % (label, time.time() - t, v.b.total))
        t = time.time()

    v.b.flush()
    v.parts_libraries()
    lap("materials, planting parts")
    v.terrain()
    v.pool_terrace()
    lap("terraces, pool, retaining walls")
    v.shell()
    v.facade_details()
    lap("house shell, stone, glazing")
    v.interior()
    lap("interior")
    v.veranda()
    v.pergola()
    v.car()
    lap("veranda, roof terrace, car")
    v.landscape()
    v.lights()
    lap("planting, lights")
    v.b.flush()


def main(argv=None):
    from tenon import Model

    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--socket", help="socket of a running app (default: start a headless one)")
    ap.add_argument("--app", help="app binary to start (default: TENON_APP or the repo's target/)")
    ap.add_argument("--out", default=".", help="directory for the images (default: here)")
    ap.add_argument("--quick", action="store_true", help="small images (for tests)")
    args = ap.parse_args(argv)
    width, height, ss = (320, 200, 1) if args.quick else (1600, 1000, 2)
    out = os.path.abspath(args.out)
    os.makedirs(out, exist_ok=True)
    pic = lambda name: os.path.join(out, name)  # noqa: E731
    t_all = time.time()
    with session(args.socket, args.app) as s:
        c = s.client
        m = Model(c)
        c.execute({"type": "new", "discard": s.launched})
        v = Villa(c, args.quick)
        build(v)
        clear = lambda: c.execute({"type": "select", "ids": [], "mode": "replace"})  # noqa: E731
        clear()
        t = time.time()
        # ---- 1: golden hour, low three-quarter view from the south-west
        c.execute({"type": "set_location", "latitude": 34.03, "longitude": -118.7, "north_angle": -16, "utc_offset": -7})
        c.execute({"type": "set_shadows", "on": True, "date": "2026-09-20", "time": "17:05", "on_ground": True, "light": 100, "dark": 40})
        glow = lambda keys, on: [c.execute({"type": "change_material", "material": v.M[k].id,  # noqa: E731
                                            "emission": GLOWS[k][2] if on else 0}) for k in keys]
        glow(GARDEN_LIGHTS, False)
        day = m.style("Golden hour", sky=[62, 116, 198], background=[248, 220, 186], edges={"color": [74, 62, 52]},
                      ambient_occlusion={"on": True, "radius": 0.9, "strength": 1.1},
                      fog={"start": 60, "end": 260, "color": "background"})
        hero = {"eye": [-19.0, -32.0, 2.6], "target": [14.5, 0.0, 3.6], "fov_y_deg": 35}
        m.render(pic("villa.png"), width, height, camera=hero, lighting="day", supersample=ss, ground=False)
        print("  %-26s %5.1f s" % ("villa.png", time.time() - t))
        t = time.time()
        # ---- 2: the same view at night, garden lights on
        glow(GARDEN_LIGHTS, True)
        m.style("Dusk", sky=[14, 30, 78], background=[16, 28, 66], edges={"color": [30, 30, 40]},
                ambient_occlusion={"on": True, "radius": 0.9, "strength": 1.0},
                fog={"start": 70, "end": 280, "color": "background"})
        m.render(pic("villa-dusk.png"), width, height, camera=hero, lighting="night", supersample=ss, ground=False)
        print("  %-26s %5.1f s" % ("villa-dusk.png", time.time() - t))
        t = time.time()
        # ---- 3: section through the living room, the stair, the kitchen and the bedroom
        gone = v.b.placed_all(v.hill) + v.tree_ids
        c.execute({"type": "hide", "ids": gone})
        c.execute({"type": "add_section_plane", "point": [16, 4.3, 3], "normal": [0, 1, 0]})
        m.section_display(planes=False, cuts=True, fill=True, fill_color="material")
        m.style("Section", sky=None, background=[240, 240, 242], edges={"color": [44, 38, 34]}, fog=None,
                ambient_occlusion={"on": True, "radius": 0.5, "strength": 1.0})
        c.execute({"type": "set_shadows", "on": False})
        key = lambda k: v.b.placed(v.keys[k])  # noqa: E731
        notes = [m.label(key("stone"), "Stone wall, double-height living room", (2.0, 4.3, 8.6)),
                 m.label(key("chimney"), "Stone chimney stack", (10.5, 4.3, 11.2)),
                 m.label(key("stair"), "Floating stair, steel stringer", (7.4, 4.3, 5.9)),
                 m.label(key("island"), "Kitchen island", (16.2, 4.3, 2.3)),
                 m.label(key("bed"), "Bedroom", (19.0, 4.3, 5.6)),
                 m.label(key("cantilever"), "Slab cantilevered 5.5 m", (24.0, 4.3, 8.4))]
        sec = {"eye": [15.5, -57.0, 4.4], "target": [15.5, 4.3, 4.4], "fov_y_deg": 22, "projection": "parallel"}
        m.render(pic("villa-section.png"), width, height, camera=sec, lighting="studio", supersample=ss, ground=False)
        c.execute({"type": "deactivate_section"})
        print("  %-26s %5.1f s" % ("villa-section.png", time.time() - t))
        t = time.time()
        # ---- 4: the ground floor plan, cut at 1.6 m, looking straight down
        c.execute({"type": "erase", "ids": [n for n in notes if n]})
        c.execute({"type": "add_section_plane", "point": [16, 4, 1.6], "normal": [0, 0, -1]})
        m.style("Plan", sky=None, background=[250, 249, 246], edges={"color": [46, 40, 36], "profiles": 2}, fog=None,
                ambient_occlusion={"on": True, "radius": 0.4, "strength": 0.8})
        plan_notes = [m.label(key("stone"), "Living, double height", (4.0, 7.0, 1.6)),
                      m.label(key("island"), "Kitchen and dining", (17.0, 5.5, 1.6)),
                      m.label(key("pool"), "Infinity pool", (12.0, -5.0, 0.0)),
                      m.label(key("kitchen"), "Outdoor kitchen", (24.0, 0.5, 1.6))]
        plan = {"eye": [15.5, -0.5, 90.0], "target": [15.5, -0.5, 0], "fov_y_deg": 20, "projection": "parallel", "up": [0, 1, 0]}
        m.render(pic("villa-plan.png"), width, height, camera=plan, lighting="studio", supersample=ss, ground=False)
        c.execute({"type": "erase", "ids": [n for n in plan_notes if n]})
        c.execute({"type": "deactivate_section"})
        c.execute({"type": "unhide", "ids": gone})
        print("  %-26s %5.1f s" % ("villa-plan.png", time.time() - t))
        # ---- 5: save with the daytime view (garden lights off, as in the first picture)
        glow(GARDEN_LIGHTS, False)
        m.section_display(planes=True)
        c.execute({"type": "set_style", "style": day})
        c.execute({"type": "set_lighting", "mode": "day"})
        c.execute({"type": "set_shadows", "on": True})
        c.execute({"type": "set_camera", "projection": "perspective", **hero})
        clear()
        c.execute({"type": "save", "path": pic("villa.tnn"), "overwrite": True})
        print("total %.1f s" % (time.time() - t_all))


if __name__ == "__main__":
    main()
