#!/usr/bin/env python3
"""A model of Dubai from the Gulf to the desert: the Burj Khalifa and Downtown, Sheikh Zayed Road,
Dubai Marina, the Burj Al Arab, the Palm Jumeirah, Ain Dubai, a mosque and the villa suburbs,
drawn at golden hour and at night.

    python3 dubai.py [--socket PATH] [--out DIR] [--quick]

Without ``--socket`` it starts a headless app. It writes ``dubai.png`` (the hero, an aerial view from
the Gulf at golden hour), ``dubai-night.png`` (the same view lit up), ``dubai-khalifa.png`` (up the
tower from the fountain), ``dubai-palm.png`` (the Palm Jumeirah from above), ``dubai-szr.png`` (down
Sheikh Zayed Road), ``dubai-flyover.gif`` (a camera path over the city, when ``ffmpeg`` is on PATH)
and ``dubai.tnn`` (the model, saved with the hero view) to ``--out``.

What it shows of the API:

- positions are compressed (about 3.5 km from the Marina to Downtown) but keep the real order along
  the coast; the Gulf lies to -Y, the desert to +Y, X runs along the coast to the north-east, Z is up;
- the Burj Khalifa as three wings round a hexagonal core, each wing a stack of tier components whose
  length shortens in a spiral, with vertical fins and floor bands copied in arrays, then a round
  upper shaft and a stepped steel spire; Cayan Tower as 73 floor-plate components, each turned a
  little more; the Emirates Towers as triangular prisms (a three-sided ``add_cone`` makes the sloped
  tops); the Burj Al Arab as curved white sail strips with a lattice of cylinders along arbitrary axes;
- towers as components (glass shaft, floor bands made by ``copy``, lit floors, crowns) placed with
  rotation by the hundred, so the model has tens of thousands of instances and still builds in
  seconds; cars, street lights, palms, villas and yachts are components placed along roads and canals;
- a Palm Jumeirah of 16 fronds (a frond is a component with a road and rows of villas, placed 16
  times with a turn), a trunk, a crescent breakwater and an Atlantis with an arch;
- see-through water, reflective glass and steel (``metalness``, ``roughness``), emissive lights that
  are switched on for the night picture with ``change_material``, ``set_location`` and
  ``set_shadows`` for the sun, styles with sky, fog and ambient occlusion.

Lengths are meters. Ground level is z = 0 and the sea surface z = -0.8.
"""
import argparse
import math
import os
import random
import shutil
import subprocess
import sys
import time
import types

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))

from tenon import session  # noqa: E402

SEA = -0.8  # the Gulf's surface
HALO = -0.77  # shallows (light water) just above it
HALO2 = -0.74
KHALIFA = (720.0, 640.0)  # the tower's centre
SZR_Y = 1000.0  # centre line of Sheikh Zayed Road
SZR_X = (-1900.0, 1700.0)
BAA = (-20.0, -330.0)  # Burj Al Arab's island
PALM = (-760.0, -900.0)  # the crown of the Palm Jumeirah
CANAL = [(-1790, 0), (-1760, 70), (-1690, 140), (-1560, 185), (-1400, 200), (-1230, 190), (-1090, 160),
         (-1000, 118), (-950, 60)]  # centre line of Dubai Marina's canal
CAYAN = (-1335.0, 290.0)


def coast(x):
    """The shore's y at x: a gentle wander."""
    return 14 * math.sin(x / 260.0) + 9 * math.sin(x / 83.0 + 1.0) + 5 * math.sin(x / 640.0)


# ---------------------------------------------------------------- the command builder


class B:
    """Collects commands and sends them in batches. ``box`` and friends return the index of the
    command, which ``$index:i0`` refers to in later commands of the same batch."""

    def __init__(self, c):
        self.c = c
        self.cmds = []
        self.total = 0  # commands sent
        self.instances = 0  # instances placed by them (copies counted)
        self.serial = 0
        self.results = {}
        self.tris = 0  # triangles drawn, estimated from the commands (the renderer has a 256 MB limit on faces)
        self.cost = {}  # index in the batch collected -> triangles of that command's instance
        self.defcost = {}  # definition id -> triangles of one instance
        self.cat = None  # a name for the instances the next commands place (see ``groups``)
        self.catlist = []
        self.groups = {}  # category -> root instance ids, so a flyover can hide the finest layers
        self.leaves = 0  # solids drawn, nested ones counted (what the renderer flattens to)
        self.lcost = {}
        self.defleaf = {}

    def add(self, **cmd):
        self.cmds.append(cmd)
        self.catlist.append(self.cat)
        i = len(self.cmds) - 1
        c = self.estimate(cmd)
        if c:
            self.cost[i] = c
            self.tris += c
        n = self.leaf_estimate(cmd)
        if n:
            self.lcost[i] = n
            self.leaves += n
        return i

    def leaf_estimate(self, cmd):
        t = cmd["type"]
        if t.startswith("add_") and t != "add_curve":
            return 1
        if t == "make_group":
            return 1
        if t == "copy":
            return cmd["count"] * sum(self.lcost.get(int(q[1:].split(":")[0]), 0) for q in cmd["ids"])
        if t == "place_instance":
            return self.defleaf.get(cmd["definition"], 0)
        return 0

    def estimate(self, cmd):
        """Triangles one command adds (copies and placements count what they copy)."""
        t = cmd["type"]
        seg = cmd.get("segments", 24)
        if t == "add_box":
            return 12
        if t in ("add_cylinder", "add_tube"):
            return seg * 4
        if t == "add_cone":
            return seg * 4
        if t == "add_prism":
            return cmd["sides"] * 4
        if t == "add_sphere":
            return seg * max(2, seg // 2) * 2
        if t == "add_torus":
            return seg * cmd.get("tube_segments", 12) * 2
        if t == "push_pull":
            return 0
        if t == "add_curve":
            n = len(cmd["points"])
            return n * 2 + (n - 2) * 2  # the walls and both caps
        if t == "copy":
            return cmd["count"] * sum(self.cost.get(int(q[1:].split(":")[0]), 0) for q in cmd["ids"])
        if t == "place_instance":
            return self.defcost.get(cmd["definition"], 0)
        return 0

    def flush(self):
        if not self.cmds:
            return []
        cmds, self.cmds = self.cmds, []
        cats, self.catlist = self.catlist, []
        try:
            r = self.c.batch(cmds)
        except Exception as e:
            idx = getattr(e, "index", None)
            if idx is not None:
                for j in range(max(0, idx - 3), idx + 1):
                    print("  %d: %s" % (j, cmds[j]))
            raise
        self.total += len(cmds)
        for o, cat in zip(r["outcomes"], cats):
            self.instances += len(o.get("placed", []))
            if cat and o.get("placed"):
                self.groups.setdefault(cat, []).extend(o["placed"])
        self.results[self.serial] = r["outcomes"]
        self.serial += 1
        return r["outcomes"]

    def tick(self, n=400):
        if len(self.cmds) > n:
            self.flush()

    # primitives -----------------------------------------------------------

    def box(self, p0, p1, mat, name="Box"):
        lo = [min(p0[i], p1[i]) for i in range(3)]
        sz = [abs(p1[i] - p0[i]) for i in range(3)]
        assert min(sz) > 1e-5, (name, p0, p1)
        return self.add(type="add_box", corner=[round(v, 4) for v in lo], size=[round(v, 4) for v in sz],
                        name=name, material=mat.id)

    def cyl(self, base, r, h, mat, name="Cylinder", axis=None, seg=16):
        c = dict(type="add_cylinder", base=[round(v, 4) for v in base], radius=r, height=h, segments=seg, name=name,
                 material=mat.id)
        if axis is not None:
            c["axis"] = list(axis)
        return self.add(**c)

    def cone(self, base, r, h, mat, name="Cone", top=0.0, axis=None, seg=16, phi=0.0):
        """A cone or frustum; ``phi`` (radians) turns it about the vertical through ``base``."""
        c = dict(type="add_cone", base=[round(v, 4) for v in base], radius=r, height=h, top_radius=top, segments=seg,
                 name=name, material=mat.id)
        if axis is not None:
            c["axis"] = list(axis)
        i = self.add(**c)
        if phi:
            self.rot(i, [0, 0, 1], math.degrees(phi), [base[0], base[1], base[2]])
        return i

    def sph(self, center, r, mat, name="Sphere", seg=12):
        return self.add(type="add_sphere", center=list(center), radius=r, segments=seg, name=name, material=mat.id)

    def prism(self, base, r, h, sides, mat, name="Prism", phi=0.0, axis=None):
        c = dict(type="add_prism", base=[round(v, 4) for v in base], radius=r, height=h, sides=sides,
                 direction=[math.cos(phi), math.sin(phi), 0], name=name, material=mat.id)
        if axis is not None:
            c["axis"] = list(axis)
            c.pop("direction")
        return self.add(**c)

    def torus(self, center, r, tube, axis, mat, name="Torus", seg=40, tseg=8):
        return self.add(type="add_torus", center=list(center), radius=r, tube_radius=tube, axis=list(axis),
                        segments=seg, tube_segments=tseg, name=name, material=mat.id)

    def poly(self, pts, dist, mat, name="Slab"):
        """A closed profile pushed ``dist`` along its normal (+Z for a flat one), as a painted group."""
        a = self.add(type="add_curve", points=[[round(v, 4) for v in p] for p in pts], closed=True, curve=False)
        p = self.add(type="push_pull", face="$%d:f0" % a, distance=dist)
        g = self.add(type="make_group", ids=["$%d:*" % a, "$%d:*" % p], name=name)
        self.add(type="paint", ids=["$%d:i0" % g], material=mat.id)
        return g

    # operations on an earlier command's instance --------------------------

    def copy(self, i, offset, count, also=None):
        """Copy the instance made by command ``i`` (and, with ``also``, every instance command ``also``
        made, say an earlier copy) ``count`` times, each ``offset`` further on."""
        if count < 1:
            return None
        ids = ["$%d:i0" % i] + (["$%d:i*" % also] if also is not None else [])
        return self.add(type="copy", ids=ids, offset=[round(v, 4) for v in offset], count=count)

    def row(self, i, offset, count, up, n_up):
        """A row of ``count`` copies along ``offset`` repeated ``n_up`` times along ``up``; returns the commands."""
        out = [i]
        k = self.copy(i, offset, count - 1)
        out.append(k)
        if n_up > 1:
            out.append(self.copy(i, up, n_up - 1, also=k))
        return out

    def move(self, i, offset):
        return self.add(type="move", ids=["$%d:i*" % i], offset=list(offset))

    def rot(self, i, axis, angle, center):
        self.add(type="rotate", ids=["$%d:i*" % i], center=list(center), axis=list(axis), angle=angle)

    def scale(self, i, factors, origin):
        self.add(type="scale", ids=["$%d:i*" % i], origin=list(origin), factors=list(factors))

    # components and placing -----------------------------------------------

    def comp(self, name, parts):
        """A hidden component made at the origin by ``parts()`` (which adds commands and returns
        their indices, and must not flush); returns its definition id for ``place``."""
        self.flush()
        before, lbefore = self.tris, self.leaves
        cat, self.cat = self.cat, None
        refs = parts()
        self.cat = cat
        used, lused = self.tris - before, self.leaves - lbefore
        self.tris, self.leaves = before, lbefore  # the component's parts are hidden; each placement adds them
        i = self.add(type="make_component", ids=["$%d:i*" % r for r in refs if r is not None], name=name)
        self.add(type="hide", ids=["$%d:i0" % i])
        d = self.flush()[i]["definitions"][0]
        self.defcost[d] = used
        self.defleaf[d] = lused
        return d

    def place(self, d, x, y, z=0.0, yaw=0.0, s=1.0, sz=None, sy=None, lean=(0.0, 0.0)):
        """Place a component turned ``yaw`` degrees, scaled ``s`` (``sy`` across, ``sz`` up; negative mirrors),
        its up axis leaning ``lean`` (x, y per unit of height)."""
        sz = s if sz is None else sz
        sy = s if sy is None else sy
        a = math.radians(yaw)
        co, si = math.cos(a), math.sin(a)
        return self.add(type="place_instance", definition=d,
                        transform={"origin": [round(x, 3), round(y, 3), round(z, 3)],
                                   "x_axis": [s * co, s * si, 0], "y_axis": [-sy * si, sy * co, 0],
                                   "z_axis": [lean[0] * sz, lean[1] * sz, sz]})


# ---------------------------------------------------------------- materials

# glowing materials: key -> (name, day, night); day and night are (color, emission, opacity). They are
# switched with ``change_material`` between the pictures.
NIGHT_TONES = ((255, 208, 140), (226, 236, 255), (255, 176, 92), (255, 214, 156), (190, 220, 255), (255, 190, 120), (255, 224, 170), (240, 240, 255), (255, 214, 156))
GLASS_COLORS = {"gl_blue": (70, 118, 168), "gl_blue2": (104, 150, 196), "gl_teal": (56, 126, 140), "gl_dark": (42, 62, 90),
                "gl_silver": (150, 170, 190), "gl_bronze": (136, 110, 82), "gl_green": (72, 132, 118), "gl_gold": (182, 152, 92), "gl_khal": (98, 138, 182)}
GLOWS = {
    "lamp": ("Glow, street lamp", ((120, 120, 120), 0, 1), ((255, 214, 150), 9, 1)),
    "pool": ("Glow, lamp pool", ((255, 214, 150), 0, 0.0), ((255, 190, 110), 2.6, 0.5)),
    "w_warm": ("Lit window, warm", ((92, 118, 146), 0, 1), ((255, 208, 140), 3.2, 1)),
    "w_white": ("Lit window, white", ((92, 118, 146), 0, 1), ((226, 236, 255), 2.8, 1)),
    "w_amber": ("Lit window, amber", ((92, 118, 146), 0, 1), ((255, 170, 84), 3.4, 1)),
    "crown": ("Glow, crown", ((200, 204, 210), 0, 1), ((255, 232, 190), 7, 1)),
    "crown2": ("Glow, crown blue", ((170, 190, 215), 0, 1), ((130, 190, 255), 7, 1)),
    "spire": ("Glow, spire", ((222, 226, 232), 0, 1), ((255, 252, 240), 10, 1)),
    "led": ("Khalifa light fins", ((206, 214, 226), 0, 1), ((170, 212, 255), 6, 1)),
    "jet2": ("Fountain mist", ((236, 244, 250), 0, 0.3), ((190, 224, 255), 4, 0.38)),
    "star": ("Star", ((200, 220, 255), 0, 0.0), ((235, 240, 255), 9, 1)),
    "jet": ("Fountain jet", ((236, 244, 250), 0, 0.55), ((190, 224, 255), 6, 0.7)),
    "tail": ("Glow, tail light", ((120, 20, 20), 0, 1), ((255, 40, 30), 5, 1)),
    "head": ("Glow, headlight", ((222, 222, 214), 0, 1), ((255, 248, 220), 8, 1)),
    "sail": ("Glow, sail", ((246, 244, 238), 0, 0.96), ((120, 150, 255), 3.6, 0.96)),
    "sail2": ("Glow, sail, warm", ((246, 244, 238), 0, 0.96), ((255, 150, 110), 3.6, 0.96)),
    "wheel": ("Glow, Ain Dubai", ((214, 220, 228), 0, 1), ((120, 200, 255), 6, 1)),
    "cabin": ("Glow, cabin", ((226, 226, 230), 0, 1), ((255, 200, 140), 4, 1)),
    "deck": ("Glow, deck light", ((190, 190, 190), 0, 1), ((255, 236, 200), 6, 1)),
    "train": ("Glow, train window", ((60, 74, 96), 0, 1), ((255, 240, 200), 4, 1)),
    "vil": ("Lit villa window", ((60, 70, 82), 0, 1), ((255, 196, 120), 3, 1)),
    "street": ("Lit suburb streets", ((72, 72, 76), 0, 1), ((120, 94, 60), 1.0, 1)),
    "szrpave": ("Lit district paving", ((140, 126, 108), 0, 1), ((120, 100, 78), 0.9, 1)),
}
# a lit-window material for each glass: it looks like the glass by day
for _k, (_g, _col) in enumerate(GLASS_COLORS.items()):
    GLOWS["lit_" + _g] = ("Lit windows, " + _g[3:], (_col, 0, 1), (NIGHT_TONES[_k], 3.0, 1))


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

    # ground and water
    mat("sand", "Sand", (218, 190, 144), rough=1)
    mat("roofc", "Roof tile", (170, 92, 62), rough=0.9)
    mat("sand2", "Sand, pale", (230, 206, 164), rough=1)
    mat("beach", "Beach", (232, 214, 176), rough=1)
    mat("dune", "Dune", (224, 186, 134), rough=1)
    mat("dune2", "Dune, shade", (212, 172, 122), rough=1)
    mat("dune3", "Dune, pale", (230, 194, 144), rough=1)
    mat("sea", "Gulf", (28, 122, 164), metal=0.2, rough=0.05)
    mat("sh1", "Shallows", (36, 150, 172), metal=0.2, rough=0.05)
    mat("sh2", "Shallows, pale", (92, 194, 192), metal=0.15, rough=0.06)
    mat("swell", "Swell, light", (70, 160, 200), opacity=0.14, metal=0.2, rough=0.05)
    mat("swell2", "Swell, dark", (8, 62, 110), opacity=0.14, metal=0.2, rough=0.05)
    mat("canal", "Canal water", (26, 112, 146), metal=0.35, rough=0.04)
    mat("lake", "Lake water", (20, 100, 138), metal=0.4, rough=0.03)
    mat("pooln", "Pool", (60, 190, 210), metal=0.1, rough=0.1)
    mat("quay", "Quay stone", (206, 196, 176), rough=1)
    mat("asphalt", "Asphalt", (58, 58, 62), rough=1)
    mat("asphalt2", "Asphalt, worn", (72, 72, 76), rough=1)
    mat("white", "Road marking", (240, 238, 228), rough=1)
    mat("yellow", "Road marking, yellow", (232, 190, 44), rough=1)
    mat("walk", "Sidewalk", (150, 146, 138), rough=1)
    mat("plaza", "Plaza", (172, 164, 150), rough=1)
    mat("plaza2", "Paving", (178, 160, 138), rough=1)
    mat("verge", "Verge", (96, 128, 70), rough=1)
    mat("verge2", "Verge, dry", (108, 122, 70), rough=1)
    mat("conc", "Concrete", (172, 170, 164), rough=1)
    mat("conc2", "Concrete, dark", (124, 124, 126), rough=1)
    mat("render", "White render", (240, 237, 230), rough=0.9)
    mat("foam", "Foam", (250, 252, 252), opacity=0.5, rough=0.6)
    mat("lawn2", "Lawn, dry", (112, 132, 72), rough=1)
    mat("cream", "Cream render", (228, 214, 188), rough=0.9)
    mat("sandstone", "Sandstone", (212, 184, 140), rough=1)
    mat("roof", "Flat roof", (196, 192, 184), rough=1)
    mat("roofd", "Roof plant", (130, 132, 134), rough=1)
    # glass and metal for the towers
    mat("gl_blue", "Glass, blue", (70, 118, 168), metal=0.7, rough=0.08)
    mat("gl_blue2", "Glass, light blue", (104, 150, 196), metal=0.7, rough=0.08)
    mat("gl_teal", "Glass, teal", (56, 126, 140), metal=0.7, rough=0.08)
    mat("gl_dark", "Glass, dark", (42, 62, 90), metal=0.75, rough=0.07)
    mat("gl_silver", "Glass, silver", (150, 170, 190), metal=0.8, rough=0.08)
    mat("gl_bronze", "Glass, bronze", (136, 110, 82), metal=0.7, rough=0.1)
    mat("gl_green", "Glass, green", (72, 132, 118), metal=0.7, rough=0.08)
    mat("gl_gold", "Glass, gold", (182, 152, 92), metal=0.75, rough=0.1)
    mat("gl_khal", "Glass, Burj Khalifa", (98, 138, 182), metal=0.85, rough=0.05)
    mat("steel", "Steel", (190, 196, 204), metal=0.95, rough=0.28)
    mat("steelw", "Steel, white", (226, 228, 232), metal=0.9, rough=0.3)
    mat("steeld", "Steel, dark", (64, 68, 76), metal=0.85, rough=0.35)
    mat("khal_fin", "Khalifa cladding", (206, 214, 226), metal=0.9, rough=0.22)
    mat("spire_m", "Spire steel", (226, 230, 236), metal=1.0, rough=0.16)
    mat("gold", "Gold", (216, 176, 82), metal=0.95, rough=0.25)
    mat("bandw", "Band, white", (238, 238, 234), rough=0.6)
    mat("bandg", "Band, grey", (150, 152, 156), rough=0.6)
    mat("bandd", "Band, dark", (62, 64, 70), metal=0.6, rough=0.4)
    mat("sailw", "Sail fabric", (246, 244, 238), opacity=0.96, rough=0.5)
    for key, (name, day, night) in GLOWS.items():
        col, em, op = day
        if key in ("sail", "sail2"):
            spec[key] = dict(type="add_material", name=name, color=list(col), existing="update", opacity=op,
                             metalness=0.1, roughness=0.45)
        else:
            shiny = key.startswith("lit_") or key in ("w_warm", "w_white", "w_amber", "train", "wheel", "spire", "crown", "crown2")
            spec[key] = dict(type="add_material", name=name, color=list(col), existing="update", opacity=op,
                             metalness=0.7 if shiny else 0.0, roughness=0.08 if shiny else 1.0)
    # vehicles, furniture of the street
    mat("sign", "Sign green", (16, 104, 70), rough=0.5)
    mat("door", "Door recess", (34, 38, 46), rough=0.9)
    mat("steelg", "Galvanised", (150, 156, 162), metal=0.8, rough=0.5)
    mat("tyre", "Tyre", (24, 24, 26), rough=0.9)
    mat("carglass", "Car glass", (28, 34, 44), metal=0.5, rough=0.1)
    for k, col in enumerate(((236, 236, 238), (24, 24, 28), (196, 198, 204), (150, 30, 30), (36, 56, 110),
                             (214, 196, 160))):
        mat("car%d" % k, "Car paint %d" % (k + 1), col, metal=0.8, rough=0.25)
    mat("taxi", "Taxi", (232, 176, 40), metal=0.7, rough=0.3)
    mat("metrow", "Metro body", (214, 218, 224), metal=0.7, rough=0.3)
    mat("metror", "Metro livery", (170, 40, 40), metal=0.6, rough=0.4)
    mat("pole", "Pole", (96, 100, 106), metal=0.8, rough=0.4)
    # planting
    mat("palmtrunk", "Palm trunk", (156, 130, 98), rough=1)
    mat("frond", "Palm frond", (66, 122, 58), rough=1)
    mat("frond2", "Palm frond, dry", (96, 134, 62), rough=1)
    mat("leaf", "Leaves", (78, 122, 58), rough=1)
    mat("leaf2", "Leaves, light", (112, 148, 70), rough=1)
    mat("leaf3", "Leaves, deep", (52, 98, 54), rough=1)
    mat("bark", "Bark", (100, 80, 62), rough=1)
    mat("hedge", "Hedge", (60, 100, 56), rough=1)
    mat("lawn", "Lawn", (94, 132, 66), rough=1)
    # boats and other
    mat("hull", "Hull", (244, 244, 242), rough=0.3, metal=0.2)
    mat("hullb", "Hull, navy", (30, 44, 80), rough=0.3, metal=0.2)
    mat("teak", "Teak", (156, 106, 62), rough=0.6)
    mat("dhow", "Dhow timber", (122, 84, 52), rough=0.8)
    mat("sailc", "Canvas", (236, 226, 204), rough=1)
    mat("dome", "Dome", (206, 214, 200), metal=0.5, rough=0.4)
    mat("domeg", "Dome, gold", (200, 162, 84), metal=0.9, rough=0.3)
    mat("helipad", "Helipad", (200, 70, 60), rough=0.8)

    keys = list(spec)
    r = c.batch([spec[k] for k in keys])
    return {k: types.SimpleNamespace(id=r["outcomes"][i]["changes"]["materials"][0]) for i, k in enumerate(keys)}


def set_night(c, M, night):
    """Switch the glowing materials on (night) or off (day)."""
    cmds = []
    for key, (_, day, nt) in GLOWS.items():
        col, em, op = nt if night else day
        cmds.append({"type": "change_material", "material": M[key].id, "color": list(col), "emission": em,
                     "opacity": op})
    c.batch(cmds)


# ---------------------------------------------------------------- towers

GLASSES = ("gl_blue", "gl_blue2", "gl_teal", "gl_dark", "gl_silver", "gl_bronze", "gl_green", "gl_gold")
BANDS = ("bandw", "steel", "bandg", "bandd", "steelw")


def solid(b, M, plan, z, h, mat, grow=0.0, name="Tower"):
    """A prism standing on ``plan`` from z up ``h``; ``grow`` pushes the outline out by that much."""
    k = plan[0]
    if k == "box":
        _, cx, cy, hx, hy = plan
        return b.box((cx - hx - grow, cy - hy - grow, z), (cx + hx + grow, cy + hy + grow, z + h), mat, name)
    if k == "cyl":
        _, cx, cy, r = plan
        return b.cyl((cx, cy, z), r + grow, h, mat, name, seg=28)
    _, cx, cy, R, n, phi = plan
    return b.prism((cx, cy, z), R + grow / math.cos(math.pi / n), h, n, mat, name, phi=phi)


def scaled(plan, f, dx=0.0, dy=0.0):
    k = plan[0]
    if k == "box":
        return ("box", plan[1] + dx, plan[2] + dy, plan[3] * f, plan[4] * f)
    if k == "cyl":
        return ("cyl", plan[1] + dx, plan[2] + dy, plan[3] * f)
    return ("poly", plan[1] + dx, plan[2] + dy, plan[3] * f, plan[4], plan[5])


def plan_extent(plan):
    k = plan[0]
    if k == "box":
        return plan[3], plan[4]
    if k == "cyl":
        return plan[3], plan[3]
    return plan[3], plan[3]


DETAIL = 1.0  # 1 for the full model; quick builds lower it (coarser fins, no balconies)


def face_box(plan, face, u0, u1, d0, d1, z0, z1):
    """A box on one side of a rectangular plan: along the face from u0 to u1 (from its centre), out from
    the glass from d0 to d1. ``face`` is 0..3 for +y, -y, +x, -x."""
    _, cx, cy, hx, hy = plan
    if face < 2:
        sy = 1 if face == 0 else -1
        ya, yb = sorted((cy + sy * (hy + d0), cy + sy * (hy + d1)))
        return (cx + u0, ya, z0), (cx + u1, yb, z1)
    sx = 1 if face == 2 else -1
    xa, xb = sorted((cx + sx * (hx + d0), cx + sx * (hx + d1)))
    return (xa, cy + u0, z0), (xb, cy + u1, z1)


def comb_pts(centers, w, a0, a1, s0, s1, tip):
    """The outline of a comb: a spine from ``a0`` to ``a1`` along one axis at ``s0``..``s1`` across it, and a tooth of
    width ``w`` centred on each of ``centers`` reaching out to ``tip``. A whole row of mullions, floor slabs or
    balcony slabs is then one solid (a polygon pushed along its normal) instead of one instance each."""
    pts = [(a0, s0), (a1, s0), (a1, s1)]
    for c in sorted(centers, reverse=True):
        pts += [(c + w / 2, s1), (c + w / 2, tip), (c - w / 2, tip), (c - w / 2, s1)]
    pts.append((a0, s1))
    return pts


def face_origin(plan, face, grow):
    """Where a vertical polygon lying along a face starts (its fixed coordinate) and which way it is pushed."""
    _, cx, cy, hx, hy = plan
    if face == 0:
        return cy + hy - 0.05
    if face == 1:
        return cy - hy - grow
    if face == 2:
        return cx + hx - 0.05
    return cx - hx - grow


def mullion_comb(b, plan, face, z, th, pitch, mat, depth=0.4, w=0.24):
    """All the vertical mullions of one face of a rectangular plan, as one solid."""
    _, cx, cy, hx, hy = plan
    ln = hx if face < 2 else hy
    cnt = max(2, int(2 * ln / pitch))
    step = 2 * ln / cnt
    cs = [-ln + step * (k + 0.5) for k in range(cnt)]
    sy = 1 if face == 0 else -1
    sx = 1 if face == 2 else -1
    pts = []
    for (u, d) in comb_pts(cs, w, -ln, ln, -0.3, -0.1, depth):
        pts.append((cx + u, cy + sy * (hy + d), z) if face < 2 else (cx + sx * (hx + d), cy + u, z))
    return b.poly(pts, th, mat, "Mullions")


def slab_comb(b, plan, face, z, th, f, bh, grow, mat, every=1):
    """The floor slab edges up one face, as one solid: a comb in the plane of the face."""
    _, cx, cy, hx, hy = plan
    ln = (hx if face < 2 else hy) + grow
    n = int(round(th / f))
    cs = [z + (k + 1) * f - bh / 2 for k in range(every - 1, n, every)]
    base = face_origin(plan, face, grow)
    pts = []
    for (zz, u) in comb_pts(cs, bh, z, z + th, -ln, -ln + 0.15, ln):
        pts.append((cx + u, base, zz) if face < 2 else (base, cy + u, zz))
    return b.poly(pts, grow + 0.05, mat, "Slab edges")


def balcony_column(b, plan, face, u, z, th, f, bh, mat):
    """The balcony slabs of one bay, all floors, as one solid (a comb in the plane square to the face)."""
    _, cx, cy, hx, hy = plan
    sy = 1 if face == 0 else -1
    sx = 1 if face == 2 else -1
    n = int(round(th / f))
    cs = [z + (k + 1) * f - bh - 0.125 for k in range(n)]
    pts = []
    for (zz, d) in comb_pts(cs, 0.25, z, z + th, -0.3, -0.1, 1.6):
        if face < 2:
            pts.append((cx + u - 1.5, cy + sy * (hy + d), zz))
        else:
            pts.append((cx + sx * (hx + d), cy + u - 1.5, zz))
    return b.poly(pts, 3.0, mat, "Balcony slabs")


def tier(b, M, rng, s, plan, z, th, out, lit_frac):
    """One tier of a shaft: a glass body, a slab edge at every floor, mullions, a plinth, lit floors and, on
    residential towers, a balcony with a glass rail and a recessed dark door at every bay and floor."""
    fh = s.get("fh", 3.9)
    n = max(2, int(round(th / fh)))
    f = th / n
    bh = 0.55
    glass, band = M[s["glass"]], M[s["band"]]
    out.append(solid(b, M, plan, z, th, glass))
    out.append(solid(b, M, plan, z, 1.1, band, grow=0.4, name="Plinth"))
    every = 1 if DETAIL > 0.5 else 2
    fin = M[s.get("fin", s["band"])]
    k = plan[0]
    pitch = s.get("pitch", 3.0) / DETAIL ** 0.5
    if k == "box":
        for face in range(4):
            out.append(slab_comb(b, plan, face, z, th, f, bh, 0.3, band, every))
            if s.get("fins", True):
                out.append(mullion_comb(b, plan, face, z, th, pitch, fin))
            if not s.get("resid") and DETAIL > 0.5:  # a fine curtain-wall grid between the main fins
                out.append(mullion_comb(b, plan, face, z, th, 1.25, M["door"], depth=0.14, w=0.1))
        if s.get("resid") and DETAIL > 0.5:
            _, cx, cy, hx, hy = plan
            for face in range(4):
                ln = hx if face < 2 else hy
                cnt = max(2, int(2 * ln / 4.2))
                step = 2 * ln / cnt
                u = -ln + step / 2
                p0, p1 = face_box(plan, face, u - 1.15, u + 1.15, -0.03, 0.04, z + 0.4, z + th - 0.4)
                door = b.box(p0, p1, M["door"], "Door strips")
                col = balcony_column(b, plan, face, u, z, th, f, bh, M["bandw"])
                off = (step, 0, 0) if face < 2 else (0, step, 0)
                out += [door, col, b.copy(door, off, cnt - 1), b.copy(col, off, cnt - 1)]
    else:
        bi = solid(b, M, plan, z + every * f - bh, bh, band, grow=0.3, name="Slab edge")
        out.append(bi)
        out.append(b.copy(bi, (0, 0, every * f), max(1, n // every) - 1))
        if k == "poly" and s.get("fins", True):
            m = plan[4]
            for j in range(m):
                a = plan[5] + 2 * math.pi * j / m
                out.append(b.cyl((plan[1] + plan[3] * math.cos(a), plan[2] + plan[3] * math.sin(a), z), 0.6, th, fin, "Rib", seg=6))
                a2 = plan[5] + 2 * math.pi * (j + 0.5) / m
                r2 = plan[3] * math.cos(math.pi / m)
                out.append(b.cyl((plan[1] + r2 * math.cos(a2), plan[2] + r2 * math.sin(a2), z), 0.25, th, fin, "Rib", seg=4))
        elif k == "cyl" and s.get("fins", True):
            for j in range(int(18 * DETAIL ** 0.5) + 4):
                a = 2 * math.pi * j / (int(18 * DETAIL ** 0.5) + 4)
                out.append(b.cyl((plan[1] + plan[3] * math.cos(a), plan[2] + plan[3] * math.sin(a), z), 0.3, th, fin, "Rib", seg=4))
    # lit floors: a few runs of floors that glow at night
    for _ in range(max(1, int(n * lit_frac / 2))):
        kk = rng.randrange(n)
        run = min(n - kk, rng.choice((1, 2, 3)))
        out.append(solid(b, M, plan, z + kk * f + 0.15, run * f - bh - 0.3, M["lit_" + s["glass"]], grow=0.08, name="Lit floors"))


def rooftop(b, M, rng, plan, z, out):
    """Plant on a roof: a lift motor room, air handlers, cooling fans, a water tank and a railing."""
    hx, hy = plan_extent(plan)
    cx, cy = plan[1], plan[2]
    w = min(hx, hy) * 0.7
    out.append(b.box((cx - w, cy - w * 0.6, z), (cx - w * 0.2, cy + w * 0.2, z + rng.uniform(3, 5)), M["roofd"], "Motor room"))
    for q in range(rng.randint(3, 6)):
        x, y = cx + rng.uniform(-w, w), cy + rng.uniform(-w, w)
        out.append(b.box((x, y, z), (x + rng.uniform(1.5, 3.2), y + rng.uniform(1.5, 3.2), z + rng.uniform(1.0, 2.2)),
                         M[rng.choice(("conc", "steelw", "roofd"))], "Air handler"))
    for q in range(rng.randint(1, 3)):
        out.append(b.cyl((cx + rng.uniform(-w, w), cy + rng.uniform(-w, w), z), rng.uniform(0.9, 1.6), rng.uniform(1.0, 1.6), M["steeld"], "Cooling fan", seg=8))
    if rng.random() < 0.5:
        out.append(b.cyl((cx + w * 0.5, cy - w * 0.5, z), 1.8, 3.2, M["steelw"], "Water tank", seg=10))


def crown(b, M, rng, s, plan, z, h, out):
    kind = s.get("crown", "flat")
    hx, hy = plan_extent(plan)
    cx, cy = plan[1], plan[2]
    band, steel = M[s["band"]], M["steel"]
    if kind in ("flat", "ring", "lantern", "twin"):
        out.append(solid(b, M, plan, z, 1.2, band, grow=0.5, name="Parapet"))
    if kind == "flat":
        rooftop(b, M, rng, scaled(plan, 0.8), z + 1.2, out)
        out.append(b.cyl((cx + hx * 0.2, cy, z + 1.2), 0.25, rng.uniform(8, 22), steel, "Mast", seg=6))
    elif kind == "spire":
        out.append(solid(b, M, scaled(plan, 0.55), z, 6, band))
        sh = min(0.26 * h, 90)
        out.append(b.cone((cx, cy, z + 6), min(hx, hy) * 0.12 + 0.6, sh, steel, "Spire", top=0.12, seg=8))
        for q in range(1, 5):
            out.append(b.cyl((cx, cy, z + 6 + q * sh / 5), min(hx, hy) * 0.12 * (1 - q / 5) + 0.8, 0.5, M["steelw"], "Spire ring", seg=8))
    elif kind == "ring":
        out.append(solid(b, M, plan, z + 1.2, 4, band, grow=0.7, name="Crown ring"))
        out.append(solid(b, M, plan, z + 5.2, 1.0, M["crown"], grow=0.5, name="Crown light"))
        # fins rising above the ring
        if plan[0] == "box":
            for face in range(4):
                ln = hx if face < 2 else hy
                cnt = max(3, int(2 * ln / 2.4))
                step = 2 * ln / cnt
                p0, p1 = face_box(plan, face, -ln + step / 2 - 0.15, -ln + step / 2 + 0.15, 0.3, 0.7, z + 5.2, z + 12)
                i = b.box(p0, p1, band, "Crown fin")
                out += [i, b.copy(i, (step, 0, 0) if face < 2 else (0, step, 0), cnt - 1)]
        rooftop(b, M, rng, scaled(plan, 0.7), z + 5.2, out)
        out.append(b.cyl((cx, cy, z + 5.2), 0.3, rng.uniform(14, 36), steel, "Mast", seg=6))
    elif kind == "lantern":
        out.append(solid(b, M, scaled(plan, 0.8), z + 1.2, h * 0.07 + 6, M[s.get("lantern", "crown")], name="Lantern"))
        if plan[0] == "box":  # a lattice of blades round the lantern
            for face in range(4):
                ln = (hx if face < 2 else hy) * 0.8
                cnt = max(3, int(2 * ln / 2.0))
                step = 2 * ln / cnt
                p0, p1 = face_box(scaled(plan, 0.8), face, -ln + step / 2 - 0.1, -ln + step / 2 + 0.1, 0.0, 0.5, z + 1.2, z + 7.2 + h * 0.07)
                i = b.box(p0, p1, band, "Lantern blade")
                out += [i, b.copy(i, (step, 0, 0) if face < 2 else (0, step, 0), cnt - 1)]
        out.append(solid(b, M, scaled(plan, 0.55), z + 7.2 + h * 0.07, 3, band))
        out.append(b.cyl((cx, cy, z + 10.2 + h * 0.07), 0.3, 30, steel, "Mast", seg=6))
    elif kind == "slope":
        top = 0.28
        ph = min(0.12 * h, 34)
        k = plan[0]
        if k == "poly":
            R = plan[3]
            out.append(b.cone((cx, cy, z), R, ph, M[s["glass"]], "Sloped top", top=R * top, seg=plan[4], phi=plan[5]))
        elif k == "cyl":
            out.append(b.cone((cx, cy, z), plan[3], ph, M[s["glass"]], "Sloped top", top=plan[3] * top, seg=28))
        else:
            R = math.hypot(hx, hy)
            out.append(b.cone((cx, cy, z), R, ph, M[s["glass"]], "Sloped top", top=R * top, seg=4, phi=math.pi / 4))
        for q in range(1, 4):  # glazing bars across the slope
            fr = q / 4.0
            rr = (math.hypot(hx, hy) if k == "box" else plan[3]) * (1 - fr * (1 - top))
            if k != "cyl":
                out.append(b.cone((cx, cy, z + ph * fr), rr + 0.2, 0.3, band, "Slope band", top=rr + 0.2,
                                  seg=4 if k == "box" else plan[4], phi=math.pi / 4 if k == "box" else plan[5]))
        out.append(b.cyl((cx, cy, z + ph), 0.35, min(0.3 * h, 70), steel, "Mast", seg=6))
    elif kind == "steps":
        zz = z
        for q in (0.8, 0.6, 0.4):
            out.append(solid(b, M, scaled(plan, q), zz, 6, M[s["glass"]]))
            out.append(solid(b, M, scaled(plan, q), zz + 5.5, 0.5, band, grow=0.3))
            if q == 0.8:
                out.append(solid(b, M, scaled(plan, q), zz + 2.8, 0.4, band, grow=0.2))
            zz += 6
        rooftop(b, M, rng, scaled(plan, 0.35), zz, out)
        out.append(b.cyl((cx, cy, zz), 0.35, min(0.25 * h, 50), steel, "Mast", seg=6))
    elif kind == "twin":
        for sx in (-1, 1):
            pl = scaled(plan, 0.42, dx=sx * hx * 0.5)
            out.append(solid(b, M, pl, z + 1.2, 14, M[s["glass"]]))
            out.append(solid(b, M, pl, z + 15, 1, M["crown"], grow=0.4))
            out.append(b.cyl((pl[1], pl[2], z + 16), 0.3, 24, steel, "Mast", seg=6))
    elif kind == "sail":
        out.append(solid(b, M, scaled(plan, 0.5), z, 3, band))
        pts = [(cx - hx * 0.6, cy, z), (cx + hx * 0.6, cy, z), (cx + hx * 0.6, cy, z + 26), (cx + hx * 0.3, cy, z + 36),
               (cx - hx * 0.6, cy, z + 44)]
        out.append(b.poly(pts, 1.2, M["steelw"], "Crown sail"))
        for q in range(1, 6):
            out.append(b.cyl((cx - hx * 0.6 + q * hx * 0.2, cy + 0.6, z), 0.2, 20 + q * 3, steel, "Sail rib", seg=4))


def podium(b, M, rng, s, plan, out):
    """A podium wider than the tower: glass storeys with mullions and slab edges, a base of dark stone, a
    lobby with a canopy on columns, and a parapet."""
    ph = s["podium"]
    k = plan[0]
    big = scaled(plan, 1.5) if k != "box" else ("box", plan[1], plan[2], plan[3] * 1.55 + 6, plan[4] * 1.35 + 6)
    glass = M[s.get("pglass", "gl_dark")]
    out.append(solid(b, M, big, 0, ph, glass, name="Podium"))
    out.append(solid(b, M, big, 0, 6, M["bandd"], grow=0.4, name="Podium base"))
    nf = max(2, int(ph / 4.5))
    out.append(solid(b, M, big, ph, 1.2, M["bandw"], grow=0.5, name="Podium parapet"))
    if k == "box":
        for face in range(4):
            out.append(slab_comb(b, big, face, 0.0, ph, ph / nf, 0.55, 0.4, M["bandw"]))
            out.append(mullion_comb(b, big, face, 6, ph - 6, 2.6 / DETAIL ** 0.5, M["steelw"], depth=0.3, w=0.2))
    else:
        bi = solid(b, M, big, 4.0, 0.55, M["bandw"], grow=0.4, name="Podium band")
        out += [bi, b.copy(bi, (0, 0, ph / nf), nf - 1)]
    if k == "box":
        # the lobby: a canopy on two columns over a dark glass entrance, on the front
        p0, p1 = face_box(big, 1, -8, 8, -0.1, 0.12, 0.2, 5.8)
        out.append(b.box(p0, p1, M["carglass"], "Entrance glass"))
        p0, p1 = face_box(big, 1, -10, 10, 0.0, 6.5, 5.6, 6.1)
        out.append(b.box(p0, p1, M["steelw"], "Canopy"))
        for u in (-9.5, 9.5):
            p0, p1 = face_box(big, 1, u - 0.3, u + 0.3, 5.7, 6.3, 0.2, 5.6)
            out.append(b.box(p0, p1, M["steelw"], "Canopy column"))
    rooftop(b, M, rng, scaled(big, 0.5), ph + 1.2, out)
    return ph


def build_tower(b, M, rng, s):
    """A tower at the origin from a spec (kind box/poly/cyl, w, d, h, tiers, crown, glass, band ...);
    returns the command indices of its parts."""
    out = []
    kind = s.get("kind", "box")
    w, d, h = s["w"], s.get("d", s["w"]), s["h"]
    if kind == "box":
        plan = ("box", 0.0, 0.0, w / 2, d / 2)
    elif kind == "cyl":
        plan = ("cyl", 0.0, 0.0, w / 2)
    else:
        plan = ("poly", 0.0, 0.0, w / 2, s.get("n", 3), s.get("phi", 0.0))
    z = 0.0
    if s.get("podium"):
        z = podium(b, M, rng, s, plan, out)
    tiers = s.get("tiers", [(1.0, 1.0, 0.0, 0.0)])
    H = h - z
    pl = plan
    lit = s.get("lit", 0.32)
    for q, (fr, sc, dx, dy) in enumerate(tiers):
        th = H * fr
        pl = scaled(plan, sc, dx * w, dy * d)
        tier(b, M, rng, s, pl, z, th, out, lit)
        z += th
        if q < len(tiers) - 1 and pl[0] == "box":  # a terrace on the setback, with a glass rail
            nxt = scaled(plan, tiers[q + 1][1], tiers[q + 1][2] * w, tiers[q + 1][3] * d)
            out.append(solid(b, M, pl, z, 0.5, M["bandw"], grow=0.5, name="Terrace slab"))
            out.append(solid(b, M, nxt, z + 0.5, 0.3, M["roof"], grow=0.8, name="Terrace"))
    crown(b, M, rng, s, pl, z, h, out)
    return out


def random_spec(rng, h, kind=None):
    """A tower spec with its own proportions, glass, tiers and crown; half are residential with balconies."""
    kind = kind or rng.choices(("box", "box", "box", "poly", "cyl"), (6, 5, 3, 2, 1))[0]
    w = rng.uniform(28, 44)
    d = w * rng.uniform(0.6, 1.0)
    glass = rng.choice(GLASSES)
    resid = kind == "box" and rng.random() < 0.5
    s = dict(kind=kind, w=w, d=d, h=h, glass=glass, band=rng.choice(BANDS), fin=rng.choice(BANDS),
             crown=rng.choice(("flat", "spire", "ring", "lantern", "slope", "steps", "twin", "sail")),
             fh=rng.choice((3.4, 3.6, 3.9)), pitch=rng.choice((2.4, 3.0, 3.6)), lit=rng.uniform(0.35, 0.7),
             fins=True, resid=resid)
    if resid:
        s["glass"] = rng.choice(("gl_blue2", "gl_silver", "gl_green", "gl_dark"))
        s["band"] = rng.choice(("bandw", "bandg", "bandw"))
        s["fin"] = "bandw"
        s["pitch"] = 6.8
    if kind == "poly":
        s["n"] = rng.choice((3, 5, 6, 8))
        s["phi"] = rng.choice((0.0, math.pi / 6))
        s["d"] = w
    if kind == "cyl":
        s["d"] = w
    nt = rng.choice((1, 1, 2, 3)) if h > 90 else 1
    if nt == 1:
        s["tiers"] = [(1.0, 1.0, 0.0, 0.0)]
    elif nt == 2:
        s["tiers"] = [(0.68, 1.0, 0, 0), (0.32, rng.uniform(0.72, 0.86), rng.choice((-0.05, 0, 0.05)), 0)]
    else:
        s["tiers"] = [(0.5, 1.0, 0, 0), (0.28, rng.uniform(0.8, 0.9), 0.04, 0), (0.22, rng.uniform(0.58, 0.72), 0.06, 0.03)]
    if h > 80 and rng.random() < 0.7:
        s["podium"] = rng.choice((14, 18, 23))
    if s["crown"] == "slope" and kind == "box":
        s["d"] = s["w"]
    return s


def mid_spec(rng, h):
    """A mid-rise block with a flat or stepped top, banding and a podium."""
    s = random_spec(rng, h)
    s["crown"] = rng.choice(("flat", "flat", "steps", "ring", "lantern"))
    s["kind"] = rng.choice(("box", "box", "box", "poly"))
    if s["kind"] == "poly":
        s["n"] = rng.choice((5, 6, 8))
        s["d"] = s["w"]
        s["resid"] = False
    s["w"] *= 0.8
    s["d"] *= 0.8
    return s


# ---------------------------------------------------------------- the Burj Khalifa

KH_TIERS = [44, 40, 38, 36, 34, 32, 30, 30, 28, 28, 26, 26, 24, 24, 22, 22, 20, 20]  # metres, to 524
KH_WIDTH = 8.5  # half width of a wing


def khalifa(b, M, rng, quick=False):
    """Three wings round a hexagonal core. A wing tier is a component with a glass body, a rounded
    nose, floor bands and vertical fins; the wings shorten tier by tier, each starting its steps
    a little later than the last, which makes the spiral."""
    cx, cy = KHALIFA
    gl, fin = M["gl_khal"], M["khal_fin"]
    cache = {}
    z = 0.0
    zs = []
    for th in KH_TIERS:
        zs.append(z)
        z += th

    def wing_comp(k, L):
        key = (k, round(L, 1))
        if key in cache:
            return cache[key]
        th = KH_TIERS[k]
        hw = KH_WIDTH

        def parts():
            out = []
            body = L - hw
            n = max(2, int(round(th / 3.9)))
            f = th / n
            out.append(b.box((0, -hw, 0), (body, hw, th), gl, "Wing"))
            out.append(b.cyl((body, 0, 0), hw, th, gl, "Wing nose", seg=20))
            bi = b.box((0, -hw - 0.3, f - 0.5), (body, hw + 0.3, f), M["bandw"], "Floor band")
            out.append(bi)
            out.append(b.copy(bi, (0, 0, f), n - 1))
            ni = b.cyl((body, 0, f - 0.5), hw + 0.3, 0.5, M["bandw"], "Nose band", seg=20)
            out.append(ni)
            out.append(b.copy(ni, (0, 0, f), n - 1))
            out.append(b.box((0, -hw - 0.3, 0), (body, hw + 0.3, 0.7), fin, "Plinth"))
            cnt = max(2, int(body / (1.1 if DETAIL > 0.5 else 3.0)))
            step = body / cnt
            for sy in (-1, 1):  # all the fins down one side of the wing, one solid
                pts = [(x, sy * y, 0.0) for (x, y) in comb_pts([step * (q + 0.5) for q in range(cnt)], 0.4, 0, body, hw - 0.3, hw - 0.1, hw + 0.6)]
                out.append(b.poly(pts, th, fin, "Fins"))
            for a in range(-84, 85, 6 if DETAIL > 0.5 else 20):
                ar = math.radians(a)
                out.append(b.cyl((body + hw * math.cos(ar), hw * math.sin(ar), 0), 0.42, th, M["led"], "Nose fin", seg=6))
            # a few lit floors on the nose for the night
            for q in rng.sample(range(n), max(1, n // 3)):
                out.append(b.cyl((body, 0, q * f + 0.2), hw + 0.1, f - 0.9, M["lit_gl_khal"], "Lit floors", seg=20))
            return out

        d = b.comp("Khalifa wing %d/%d" % (k, round(L)), parts)
        cache[key] = d
        return d

    # the podium: a hexagon with the three arms of the hotel round it
    out_nodes = []
    b.flush()
    b.add(type="add_cylinder", base=[cx, cy, 0], radius=96, height=0.35, segments=48, name="Plaza", material=M["plaza"].id)
    b.prism((cx, cy, 0.0), 58, 14, 6, M["gl_bronze"], "Podium", phi=math.radians(30))
    b.prism((cx, cy, 14.0), 59, 1.2, 6, M["bandw"], "Podium roof", phi=math.radians(30))
    for i in range(3):
        a = math.radians(90 + 120 * i)
        ca, sa = math.cos(a), math.sin(a)
        # the low arms: a box turned to point away from the core
        arm = b.box((20, -22, 0), (88, 22, 9), M["gl_dark"], "Hotel arm")
        b.box((20, -22, 9), (88, 22, 10.2), M["bandw"], "Arm roof")
        for ref in (arm, arm + 1):
            b.rot(ref, [0, 0, 1], 90 + 120 * i, [0, 0, 0])
            b.move(ref, [cx, cy, 0])
    b.flush()
    # the shaft
    for k, th in enumerate(KH_TIERS):
        for i in range(3):
            L = 66.0 - 2.9 * (k + 0.9 * i)
            if L < 17:
                continue
            d = wing_comp(k, L)
            b.place(d, cx, cy, zs[k], yaw=90 + 120 * i)
        # the core
        R = 30.0 - 0.75 * k
        spec = dict(glass="gl_khal", band="khal_fin", fin="khal_fin", fh=3.9, pitch=3.0, fins=True)
        d = b.comp("Khalifa core %d" % k, lambda R=R, k=k, th=th: _core(b, M, rng, spec, R, th))
        b.place(d, cx, cy, zs[k])
    ztop = zs[-1] + KH_TIERS[-1]
    # the upper shaft: three round tiers, then the pinnacle and the spire
    z = ztop
    for r, th in ((14.5, 24), (12, 20), (9.5, 16)):
        spec = dict(glass="gl_khal", band="khal_fin", fin="khal_fin", fh=3.9, pitch=3.0, fins=True)
        d = b.comp("Khalifa upper %d" % z, lambda r=r, th=th: _upper(b, M, rng, spec, r, th))
        b.place(d, cx, cy, z)
        z += th
    b.flush()
    # pinnacle: a stack of steel cones with collars, the top section lit at night
    secs = [(584, 640, 6.2, 4.2), (640, 700, 3.9, 3.0), (700, 760, 2.8, 1.9), (760, 800, 1.6, 1.0), (800, 828, 0.9, 0.35)]
    for (z0, z1, r0, r1) in secs:
        mat = M["spire"] if z0 >= 760 else M["spire_m"]
        b.cone((cx, cy, z0), r0, z1 - z0, mat, "Spire", top=r1, seg=14)
        b.cyl((cx, cy, z1 - 1.6), r1 + 0.9, 1.6, M["steelw"], "Collar", seg=14)
    for (z0, z1, r0, r1) in secs:  # cross plates and rings along the spire
        for zr in range(int(z0) + 6, int(z1) - 2, 7):
            rr = r0 + (r1 - r0) * (zr - z0) / (z1 - z0)
            b.cyl((cx, cy, zr), rr + 0.55, 0.45, M["steelw"], "Spire ring", seg=14)
        r = (r0 + r1) / 2 + 0.9
        b.box((cx - r, cy - 0.1, z0), (cx + r, cy + 0.1, z1 - 3), M["steelw"], "Spire plate")
        b.box((cx - 0.1, cy - r, z0), (cx + 0.1, cy + r, z1 - 3), M["steelw"], "Spire plate")
    for zc in (640, 700, 760):  # crown lights on the collars
        b.cyl((cx, cy, zc), 4.4 - (zc - 640) / 60.0, 0.8, M["crown2"], "Crown light", seg=14)
    b.flush()


def _core(b, M, rng, spec, R, th):
    out = []
    plan = ("poly", 0.0, 0.0, R, 6, 0.0)
    tier(b, M, rng, spec, plan, 0.0, th, out, 0.28)
    return out


def _upper(b, M, rng, spec, r, th):
    out = []
    tier(b, M, rng, spec, ("cyl", 0.0, 0.0, r), 0.0, th, out, 0.3)
    return out


# ---------------------------------------------------------------- small repeated parts


class Parts:
    """Components placed many times: planting, cars, street lights, villas, boats, the metro train."""

    def __init__(self, b, M, rng):
        self.b, self.M, self.rng = b, M, rng
        self.d = {}
        self.towers = {}

    def make(self):
        self.palms()
        self.cars()
        self.lamps()
        self.boats()
        self.misc()
        self.villas()

    # planting ---------------------------------------------------------------

    def palms(self):
        b, M, rng = self.b, self.M, self.rng
        for name, h, nf, fl in (("palm_s", 6.0, 6, 3.2), ("palm_m", 9.0, 7, 4.2), ("palm_l", 13.0, 8, 5.0)):
            def parts(h=h, nf=nf, fl=fl):
                out = [b.cone((0, 0, 0), 0.36, h, M["palmtrunk"], "Trunk", top=0.22, seg=5)]
                for j in range(nf):
                    a = 2 * math.pi * j / nf + (0.3 if j % 2 else 0)
                    tilt = 0.5 if j % 2 else 0.1
                    ax = (math.cos(a) * (1 - tilt), math.sin(a) * (1 - tilt), tilt - 0.35)
                    out.append(b.cone((0, 0, h), 0.5, fl, M["frond" if j % 3 else "frond2"], "Frond", top=0.03, axis=ax, seg=3))
                return out
            self.d[name] = b.comp(name, parts)

        def acacia():
            out = [b.cyl((0, 0, 0), 0.22, 3.4, M["bark"], "Trunk", seg=4)]
            cr = b.sph((0, 0, 4.4), 3.0, M["leaf"], "Crown", seg=8)
            b.scale(cr, [1, 1, 0.5], (0, 0, 4.4))
            out.append(cr)
            return out
        self.d["acacia"] = b.comp("acacia", acacia)

        def shrub():
            s = b.sph((0, 0, 0.5), 1.1, M["hedge"], "Shrub", seg=8)
            b.scale(s, [1, 1, 0.7], (0, 0, 0))
            s2 = b.sph((0.8, 0.3, 0.4), 0.7, M["leaf2"], "Shrub", seg=8)
            return [s, s2]
        self.d["shrub"] = b.comp("shrub", shrub)

        def dune(mat):
            def f():
                s = b.sph((0, 0, 0), 1.0, M[mat], "Dune", seg=8)
                return [s]
            return f
        for k in ("dune", "dune2", "dune3"):
            self.d[k] = b.comp(k, dune(k))


    # vehicles -----------------------------------------------------------------

    def cars(self):
        b, M = self.b, self.M

        def car(mat, name, suv=False):
            def parts():
                bh = 0.78 if suv else 0.64
                out = [b.box((-2.3, -0.92, 0.3), (2.3, 0.92, 0.3 + bh), M[mat], "Body")]
                for wx in (-1.45, 1.45):
                    for wy in (-0.82, 0.82):
                        out.append(b.cyl((wx, wy - 0.12, 0.33), 0.33, 0.24, M["tyre"], "Wheel", axis=(0, 1, 0), seg=8))
                top = 1.0 + (0.12 if suv else 0)
                out.append(b.box((-1.9 if suv else -1.5, -0.8, 0.3 + bh), (1.2 if not suv else 1.8, 0.8, top + 0.45), M[mat], "Cabin"))
                out.append(b.box((-1.5 if not suv else -1.9, -0.82, 0.3 + bh + 0.05), (1.25 if not suv else 1.85, 0.82, top + 0.32), M["carglass"], "Glass"))
                out.append(b.box((2.28, -0.75, 0.62), (2.34, 0.75, 0.8), M["head"], "Headlights"))
                out.append(b.box((-2.34, -0.75, 0.66), (-2.28, 0.75, 0.8), M["tail"], "Tail lights"))
                return out
            self.d[name] = b.comp(name, parts)
        for k in range(6):
            car("car%d" % k, "car%d" % k, suv=k in (1, 2))
        car("taxi", "taxi")
        self.car_names = ["car0", "car1", "car2", "car3", "car4", "car5", "taxi", "car0", "car2", "car1"]

        def bus():
            out = [b.box((-6, -1.3, 0.4), (6, 1.3, 3.3), M["metrow"], "Bus"),
                   b.box((-5.8, -1.33, 1.5), (5.8, 1.33, 2.7), M["carglass"], "Bus windows"),
                   b.box((-6, -1.31, 0.4), (6, 1.31, 1.0), M["metror"], "Bus livery")]
            out.append(b.box((-5, -1.2, 0.0), (5, 1.2, 0.6), M["tyre"], "Wheels"))
            return out
        self.d["bus"] = b.comp("bus", bus)

    def lamps(self):
        b, M = self.b, self.M

        def lamp(double):
            def parts():
                out = [b.cyl((0, 0, 0), 0.14, 11.0, M["pole"], "Pole", seg=4)]
                for sy in ((1, -1) if double else (1,)):
                    out.append(b.box((-0.1, 0 if sy > 0 else -3.2, 10.8), (0.1, 3.2 if sy > 0 else 0, 11.0), M["pole"], "Arm"))
                    out.append(b.box((-0.5, 2.95 * sy - 0.3, 10.65), (0.5, 2.95 * sy + 0.3, 10.8), M["lamp"], "Lamp"))
                    out.append(b.cyl((0, 3.0 * sy, 0.14), 10.5, 0.02, M["pool"], "Light pool", seg=8))
                return out
            return parts
        self.d["lamp"] = b.comp("lamp", lamp(False))
        self.d["lamp2"] = b.comp("lamp2", lamp(True))


    # villas -------------------------------------------------------------------

    def villas(self):
        """Houses (with and without a plot, for the suburbs and the Palm), low-rise courtyard blocks and mosques."""
        b, M, rng = self.b, self.M, self.rng
        self.villa_names = []  # with a plot, for the suburbs
        self.villa_names_p = []  # without, for the Palm
        self.lowrise_names = []
        self.mosque_names = []
        for v in range(6):
            wall = rng.choice(("render", "cream", "render", "sandstone"))
            trim = rng.choice(("bandw", "bandg", "roof"))
            W, D = rng.uniform(13, 19), rng.uniform(11, 15)
            H1 = rng.uniform(3.4, 3.8)
            upper = rng.random() < 0.75
            pitched = v % 3 == 2
            for plot in (True, False):
                def parts(wall=wall, trim=trim, W=W, D=D, H1=H1, upper=upper, v=v, plot=plot, pitched=pitched):
                    out = []
                    if plot:
                        out.append(b.box((-15, -17, 0), (15, 17, 0.2), M["sand2"], "Plot"))
                        for (ax, ay, bx, by) in ((-14.6, -16.6, 14.6, -16.2), (-14.6, 16.2, 14.6, 16.6), (-14.6, -16.6, -14.2, 16.6), (14.2, -16.6, 14.6, 16.6)):
                            out.append(b.box((ax, ay, 0.2), (bx, by, 2.2), M["cream"], "Boundary wall"))
                        out.append(b.box((-2.2, -16.7, 0.2), (2.2, -16.1, 2.1), M["bandd"], "Gate"))
                    x0, y0 = -W / 2, -D / 2 - 2
                    z0 = 0.2
                    out.append(b.box((x0, y0, z0), (x0 + W, y0 + D, z0 + H1), M[wall], "Ground floor"))
                    out.append(b.box((x0 + 1.2, y0 - 0.06, 0.9), (x0 + W - 1.2, y0 + 0.04, z0 + H1 - 0.6), M["carglass"], "Glazing"))
                    out.append(b.box((x0 - 0.5, y0 - 0.5, z0 + H1), (x0 + W + 0.5, y0 + D + 0.5, z0 + H1 + 0.35), M[trim], "Slab"))
                    top = z0 + H1 + 0.35
                    if upper:
                        uw, ud = W * 0.62, D * 0.62
                        ux = x0 + (0 if v % 2 else W - uw)
                        out.append(b.box((ux, y0 + 1, top), (ux + uw, y0 + 1 + ud, top + 3.2), M[wall], "Upper floor"))
                        out.append(b.box((ux + 1, y0 + 0.94, top + 0.6), (ux + uw - 1, y0 + 1.04, top + 2.7), M["vil"], "Upper glazing"))
                        out.append(b.box((ux - 0.4, y0 + 0.6, top + 3.2), (ux + uw + 0.4, y0 + 1.4 + ud, top + 3.55), M[trim], "Upper slab"))
                        top += 3.55
                    if pitched:  # a gable roof over the ground floor wing: a three-sided prism lying along x
                        r = D / math.sqrt(3) + 0.5
                        out.append(b.add(type="add_prism", base=[x0 - 0.4, y0 + D / 2, z0 + H1 + 0.35 + r / 2 - 0.05], radius=r, height=W + 0.8,
                                         sides=3, axis=[1, 0, 0], direction=[0, 0, 1], name="Roof", material=M["roofc"].id))
                    else:
                        for (ax, ay, bx, by) in ((x0 - 0.5, y0 - 0.5, x0 + W + 0.5, y0 - 0.3), (x0 - 0.5, y0 + D + 0.3, x0 + W + 0.5, y0 + D + 0.5)):
                            out.append(b.box((ax, ay, z0 + H1 + 0.35), (bx, by, z0 + H1 + 0.95), M[trim], "Parapet"))
                    # roof plant: a water tank on a stand, two air-conditioner condensers, a solar heater
                    out.append(b.cyl((x0 + 1.4, y0 + D - 1.6, top), 0.8, 1.2, M["steelw"], "Water tank", seg=8))
                    out.append(b.box((x0 + W - 3, y0 + D - 2.4, top), (x0 + W - 1.8, y0 + D - 1.2, top + 0.8), M["steeld"], "AC unit"))
                    out.append(b.box((x0 + W - 4.6, y0 + D - 2.4, top), (x0 + W - 3.4, y0 + D - 1.2, top + 0.8), M["steeld"], "AC unit"))
                    # pool with coping and two loungers, lawn, shrubs and a tree
                    out.append(b.box((-4.4, 5.1, 0.2), (4.4, 11.9, 0.34), M["bandw"], "Coping"))
                    out.append(b.box((-4, 5.5, 0.34), (4, 11.5, 0.4), M["pooln"], "Pool"))
                    out.append(b.box((5.2, 6.0, 0.2), (6.4, 8.0, 0.55), M["white"], "Lounger"))
                    out.append(b.box((5.2, 8.6, 0.2), (6.4, 10.6, 0.55), M["white"], "Lounger"))
                    out.append(b.box((-12.5, 4.5, 0.2), (-6, 14, 0.26), M["lawn"], "Lawn"))
                    for (hx_, hy_) in ((-11, 6), (-8, 12.5), (9.5, 13)):
                        out.append(b.sph((hx_, hy_, 0.8), 0.9, M["hedge"], "Shrub", seg=6))
                    out.append(b.place(self.d["palm_s"], 10.5, 3.5, 0.2, yaw=v * 70))
                    # a car port with a car
                    out.append(b.box((x0 + W + 0.5, y0 + 0.5, 2.5), (x0 + W + 5.5, y0 + 6.5, 2.75), M["steelg"], "Car port roof"))
                    for (qx, qy) in ((0.6, 0.7), (5.1, 0.7), (0.6, 6.0), (5.1, 6.0)):
                        out.append(b.box((x0 + W + qx, y0 + qy - 0.1, 0.2), (x0 + W + qx + 0.2, y0 + qy + 0.1, 2.5), M["steelg"], "Post"))
                    out.append(b.place(self.d["car%d" % (v % 6)], x0 + W + 3, y0 + 3.5, 0.2, yaw=90, s=0.9))
                    if not plot:  # on the Palm: a jetty into the water behind the house, with a boat
                        out.append(b.box((-1.4, 17, 0.0), (1.4, 28, 0.25), M["teak"], "Jetty"))
                        out.append(b.place(self.d["yacht_c"], 0.0, 25.5, 0.3, yaw=90, s=0.5))
                    return out
                nm = "villa%d%s" % (v, "" if plot else "p")
                self.d[nm] = b.comp(nm, parts)
                (self.villa_names if plot else self.villa_names_p).append(nm)

        # low-rise courtyard blocks: four wings of five floors round a courtyard with a pool, plant on the roof
        for v in range(4):
            glass = rng.choice(("gl_blue2", "gl_silver", "gl_green", "gl_teal"))

            def block(glass=glass, v=v):
                out = [b.box((-26, -26, 0), (26, 26, 0.25), M["plaza"], "Plaza")]
                spec = dict(glass=glass, band="bandw", fin="bandw", fh=3.2, pitch=6.0, resid=True, lit=0.4)
                for plan in (("box", 0.0, 19.0, 21.0, 5.5), ("box", 0.0, -19.0, 21.0, 5.5), ("box", 19.0, 0.0, 5.5, 13.5), ("box", -19.0, 0.0, 5.5, 13.5)):
                    tier(b, M, rng, spec, plan, 0.25, 16.0, out, 0.3)
                    out.append(solid(b, M, plan, 16.25, 0.7, M["bandw"], grow=0.5, name="Parapet"))
                    for q in range(3):
                        out.append(b.cyl((plan[1] + rng.uniform(-plan[3] * 0.7, plan[3] * 0.7), plan[2] + rng.uniform(-plan[4] * 0.5, plan[4] * 0.5), 16.95),
                                         rng.choice((0.9, 1.3)), rng.choice((1.4, 2.0)), M["steelw"], "Water tank", seg=8))
                    acs = b.box((plan[1] - plan[3] * 0.8, plan[2] - 0.8, 16.95), (plan[1] - plan[3] * 0.8 + 1.4, plan[2] + 0.8, 17.9), M["steeld"], "AC row")
                    out += [acs, b.copy(acs, (3.2, 0, 0) if plan[3] > plan[4] else (0, 0, 0), 4 if plan[3] > plan[4] else 0)]
                out.append(b.box((-5, -3, 0.25), (5, 3, 0.4), M["bandw"], "Pool coping"))
                out.append(b.box((-4.6, -2.6, 0.4), (4.6, 2.6, 0.46), M["pooln"], "Pool"))
                for (qx, qy) in ((-9, -4), (9, 4), (-9, 4), (9, -4)):
                    out.append(b.place(self.d["palm_s"], qx, qy, 0.25, yaw=qx * 13 + qy))
                return out
            nm = "lowrise%d" % v
            self.d[nm] = b.comp(nm, block)
            self.lowrise_names.append(nm)

        # a mosque: prayer hall, a central dome on a drum, four small domes, an arcaded forecourt, two minarets
        def mosque(scale_h=1.0):
            def parts():
                out = [b.box((-34, -30, 0), (34, 30, 0.3), M["plaza"], "Mosque court")]
                out.append(b.box((-14, -10, 0.3), (14, 12, 10.3), M["sandstone"], "Prayer hall"))
                out.append(b.box((-14.4, -10.4, 9.0), (14.4, 12.4, 10.3), M["bandw"], "Hall cornice"))
                wi = b.box((-12, -10.1, 2.0), (-10.4, -9.9, 7.4), M["gl_dark"], "Hall window")
                out += [wi, b.copy(wi, (3.4, 0, 0), 7)]
                out.append(b.box((-3, -10.6, 0.3), (3, -9.8, 8.6), M["domeg"], "Portal"))
                out.append(b.cyl((0, 1, 10.3), 8.4, 3.0, M["sandstone"], "Drum", seg=20))
                out.append(b.sph((0, 1, 13.3), 9.0, M["dome"], "Main dome", seg=20))
                out.append(b.cyl((0, 1, 22.0), 0.3, 6.5, M["domeg"], "Finial", seg=6))
                out.append(b.sph((0, 1, 28.2), 0.6, M["domeg"], "Finial ball", seg=6))
                for (qx, qy) in ((-10.5, -6.5), (10.5, -6.5), (-10.5, 8.5), (10.5, 8.5)):
                    out.append(b.sph((qx, qy, 10.3), 3.6, M["dome"], "Small dome", seg=12))
                    out.append(b.cyl((qx, qy, 13.4), 0.12, 2.0, M["domeg"], "Finial", seg=4))
                # arcade round the forecourt: a row of columns with a flat roof
                for (y0_, y1_) in ((-24, -22), (24, 26)):
                    col = b.cyl((-24, (y0_ + y1_) / 2, 0.3), 0.45, 4.6, M["cream"], "Column", seg=6)
                    out += [col, b.copy(col, (3.2, 0, 0), 15)]
                    out.append(b.box((-25, y0_ - 0.6, 4.9), (25, y1_ + 0.6, 5.3), M["sandstone"], "Arcade roof"))
                for sx in (-1, 1):  # minarets: square base, octagonal shaft with two balconies, a cap and a crescent
                    mx, my = sx * 22, -12
                    out.append(b.box((mx - 2.2, my - 2.2, 0.3), (mx + 2.2, my + 2.2, 7.3), M["sandstone"], "Minaret base"))
                    out.append(b.cyl((mx, my, 7.3), 1.7, 26, M["cream"], "Minaret shaft", seg=8))
                    for zb, rb in ((22.0, 2.7), (31.5, 2.3)):
                        out.append(b.cyl((mx, my, zb), rb, 0.9, M["sandstone"], "Balcony", seg=12))
                        out.append(b.cyl((mx, my, zb + 0.9), rb - 0.25, 0.7, M["steelw"], "Balcony rail", seg=12))
                    out.append(b.cyl((mx, my, 33.3), 1.5, 5, M["cream"], "Upper shaft", seg=8))
                    out.append(b.cone((mx, my, 38.3), 1.9, 5.5, M["dome"], "Minaret cap", top=0.25, seg=12))
                    out.append(b.cyl((mx, my, 43.8), 0.1, 2.5, M["domeg"], "Finial", seg=4))
                    out.append(b.torus((mx, my, 46.2), 0.6, 0.1, (0, 1, 0), M["domeg"], "Crescent", seg=10, tseg=4))
                return out
            return parts
        self.d["mosque"] = b.comp("mosque", mosque())
        self.mosque_names.append("mosque")

    # boats --------------------------------------------------------------------

    def boats(self):
        b, M, rng = self.b, self.M, self.rng

        def yacht(hull, length, name):
            def parts():
                s = length / 18.0
                pts = [(-8 * s, -2.3 * s, 0.1), (5 * s, -2.3 * s, 0.1), (10 * s, 0, 0.1), (5 * s, 2.3 * s, 0.1), (-8 * s, 2.3 * s, 0.1)]
                out = [b.poly(pts, 1.3 * s, M[hull], "Hull")]
                out.append(b.box((-8 * s, -2.1 * s, 1.4 * s), (4 * s, 2.1 * s, 1.55 * s), M["teak"], "Deck"))
                out.append(b.box((-6 * s, -1.7 * s, 1.55 * s), (2 * s, 1.7 * s, 3.1 * s), M["hull"], "Cabin"))
                out.append(b.box((-5.8 * s, -1.74 * s, 2.0 * s), (1.8 * s, 1.74 * s, 2.7 * s), M["carglass"], "Windows"))
                out.append(b.box((-5 * s, -1.2 * s, 3.1 * s), (-0.5 * s, 1.2 * s, 4.1 * s), M["hull"], "Flybridge"))
                out.append(b.cyl((-2 * s, 0, 4.1 * s), 0.08 * s, 2.5 * s, M["steelw"], "Mast", seg=5))
                return out
            self.d[name] = b.comp(name, parts)
        yacht("hull", 18, "yacht_a")
        yacht("hullb", 24, "yacht_b")
        yacht("hull", 12, "yacht_c")
        self.yachts = ["yacht_a", "yacht_b", "yacht_c"]

        def dhow():
            pts = [(-9, -2.4, 0.1), (6, -2.6, 0.1), (11, -1.4, 0.1), (12.5, 0, 0.1), (11, 1.4, 0.1), (6, 2.6, 0.1), (-9, 2.4, 0.1)]
            out = [b.poly(pts, 1.9, M["dhow"], "Hull")]
            out.append(b.box((-9, -2.3, 2.0), (-3, 2.3, 4.2), M["dhow"], "Stern castle"))
            out.append(b.box((-8.6, -2.34, 2.9), (-3.4, 2.34, 3.6), M["carglass"], "Stern windows"))
            out.append(b.cyl((2, 0, 2.0), 0.18, 12.0, M["dhow"], "Mast", seg=6))
            out.append(b.poly([(2, 0.1, 5), (-6, 0.1, 6.5), (2, 0.1, 13.8)][::-1], 0.12, M["sailc"], "Lateen sail"))
            return out
        self.d["dhow"] = b.comp("dhow", dhow)

    def misc(self):
        b, M = self.b, self.M

        def train():
            out = []
            for k in range(4):
                x0 = k * 21.0
                out.append(b.box((x0, -1.5, 0.5), (x0 + 20, 1.5, 4.0), M["metrow"], "Car"))
                out.append(b.box((x0, -1.52, 1.5), (x0 + 20, 1.52, 2.9), M["train"], "Windows"))
                out.append(b.box((x0, -1.53, 0.5), (x0 + 20, 1.53, 1.1), M["metror"], "Livery"))
                out.append(b.box((x0 + 1, -1.2, 4.0), (x0 + 19, 1.2, 4.35), M["steel"], "Roof gear"))
            out.append(b.cone((82.0, 0, 0.5), 1.5, 3.0, M["metror"], "Nose", top=1.2, axis=(1, 0, 0), seg=8))
            return out
        self.d["train"] = b.comp("train", train)

        def ship():
            # a container ship: a dark hull, a white deckhouse aft, stacks of boxes
            pts = [(-110, -16, 0), (95, -16, 0), (125, 0, 0), (95, 16, 0), (-110, 16, 0)]
            out = [b.poly(pts, 12, M["hullb"], "Hull")]
            out.append(b.box((-105, -15, 12), (-80, 15, 40), M["hull"], "Deckhouse"))
            out.append(b.box((-104.5, -15.1, 30), (-80.5, 15.1, 34), M["carglass"], "Bridge windows"))
            for k, mat in enumerate(("car3", "car4", "taxi", "car5", "car0", "car3")):
                x0 = -70 + k * 30
                out.append(b.box((x0, -13, 12), (x0 + 26, 13, 24), M[mat], "Containers"))
                out.append(b.box((x0 + 2, -11, 24), (x0 + 24, 11, 33), M[("car4", "car3", "car5", "taxi", "car4", "car0")[k]], "Containers"))
            return out
        self.d["ship"] = b.comp("ship", ship)

        def gantry():
            # half a gantry over one carriageway, along +y from its post at the origin: two posts, a truss beam,
            # three green direction signs with white lettering strips and a lamp on each
            out = [b.cyl((0, 0, 0), 0.45, 7.2, M["steelg"], "Gantry post", seg=8),
                   b.cyl((0, 21.0, 0), 0.45, 7.2, M["steelg"], "Gantry post", seg=8),
                   b.box((-0.35, -0.5, 6.5), (0.35, 21.5, 7.2), M["steelg"], "Gantry beam"),
                   b.box((-0.2, -0.5, 7.2), (0.2, 21.5, 7.35), M["steelg"], "Gantry rail")]
            for k in range(3):
                y0 = 1.6 + k * 6.6
                out.append(b.box((-0.45, y0, 3.3), (-0.3, y0 + 5.6, 6.3), M["sign"], "Sign"))
                out.append(b.box((-0.5, y0 + 0.4, 5.0), (-0.45, y0 + 5.2, 5.25), M["white"], "Sign text"))
                out.append(b.box((-0.5, y0 + 0.4, 4.3), (-0.45, y0 + 3.6, 4.55), M["white"], "Sign text"))
                out.append(b.box((-0.5, y0 + 0.4, 3.6), (-0.45, y0 + 4.4, 3.85), M["white"], "Sign text"))
                out.append(b.box((-0.8, y0 + 2.4, 6.35), (-0.4, y0 + 3.2, 6.5), M["lamp"], "Sign lamp"))
            return out
        self.d["gantry"] = b.comp("gantry", gantry)

        self.d["crest"] = b.comp("crest", lambda: [b.box((-3.5, -0.25, 0), (3.5, 0.25, 0.12), M["foam"], "Wave crest")])
        self.d["foam"] = b.comp("foam", lambda: [b.box((-1.6, -0.35, 0), (1.6, 0.35, 0.1), M["foam"], "Surf")])
        self.d["star"] = b.comp("star", lambda: [b.box((-0.5, -0.5, -0.5), (0.5, 0.5, 0.5), M["star"], "Star")])

        def plume():
            # one unit high, scaled up: a tapered translucent column over a brighter core, widening to a soft head
            # of mist, with a low skirt of spray round the foot
            return [b.cone((0, 0, 0), 1.0, 0.8, M["jet2"], "Plume column", top=0.55, seg=10),
                    b.cone((0, 0, 0), 0.55, 1.0, M["jet"], "Plume core", top=0.15, seg=8),
                    b.sph((0, 0, 0.92), 0.8, M["jet2"], "Plume head", seg=8),
                    b.sph((0, 0, 0.1), 1.5, M["jet2"], "Plume skirt", seg=8)]
        self.d["plume"] = b.comp("plume", plume)

        def cabin():
            return [b.box((-1.3, -1.0, -1.4), (1.3, 1.0, 1.4), M["cabin"], "Cabin"),
                    b.box((-1.35, -1.05, -0.4), (1.35, 1.05, 0.6), M["carglass"], "Cabin glass")]
        self.d["cabin"] = b.comp("cabin", cabin)

        self.d["pod"] = b.comp("pod", lambda: [b.sph((0, 0, 0), 1.0, M["metrow"], "Station roof", seg=24)])

        def roundshape():
            return [b.cyl((0, 0, 0), 1, 1, M["metrow"], "Roof", axis=(1, 0, 0), seg=24)]
        self.d["tube"] = b.comp("tube", roundshape)

    # tower archetypes ---------------------------------------------------------

    def tower_pool(self, classes, variants, kind=None, name="T"):
        """Tower components, ``variants`` per height class; returns {class: [(def, spec)]}."""
        pool = {}
        for h in classes:
            pool[h] = []
            for v in range(variants):
                s = random_spec(self.rng, h, kind)
                d = self.b.comp("%s %d-%d" % (name, h, v), lambda s=s: build_tower(self.b, self.M, self.rng, s))
                pool[h].append((d, s))
        return pool

    def pick(self, pool, h):
        cls = min(pool, key=lambda c: abs(c - h))
        d, s = self.rng.choice(pool[cls])
        return d, h / cls, s


# ---------------------------------------------------------------- the city


def offset_line(pts, d):
    """The polyline ``pts`` moved sideways by ``d`` (left of the direction of travel), with mitred joints."""
    out = []
    n = len(pts)
    for i in range(n):
        a = pts[max(i - 1, 0)]
        c = pts[min(i + 1, n - 1)]
        tx, ty = c[0] - a[0], c[1] - a[1]
        L = math.hypot(tx, ty) or 1
        out.append((pts[i][0] - ty / L * d, pts[i][1] + tx / L * d))
    return out


def smooth(pts, per=8):
    """A Catmull-Rom curve through the points."""
    out = []
    P = [pts[0]] + list(pts) + [pts[-1]]
    for i in range(1, len(P) - 2):
        p0, p1, p2, p3 = P[i - 1], P[i], P[i + 1], P[i + 2]
        for k in range(per):
            t = k / per
            t2, t3 = t * t, t * t * t
            out.append(tuple(0.5 * ((2 * p1[j]) + (-p0[j] + p2[j]) * t + (2 * p0[j] - 5 * p1[j] + 4 * p2[j] - p3[j]) * t2
                                    + (-p0[j] + 3 * p1[j] - 3 * p2[j] + p3[j]) * t3) for j in (0, 1)))
    out.append(tuple(pts[-1]))
    return out


def band(pts, w):
    """The outline of a strip of width ``w`` along the polyline (z = 0)."""
    l = offset_line(pts, w / 2)
    r = offset_line(pts, -w / 2)
    return [(x, y, 0.0) for x, y in l + r[::-1]]


def sector(cx, cy, r0, r1, a0, a1, step=4.0, z=0.0):
    """An annular sector between angles a0 and a1 (degrees)."""
    n = max(2, int(abs(a1 - a0) / step))
    outer = [(cx + r1 * math.cos(math.radians(a0 + (a1 - a0) * k / n)), cy + r1 * math.sin(math.radians(a0 + (a1 - a0) * k / n)), z) for k in range(n + 1)]
    inner = [(cx + r0 * math.cos(math.radians(a0 + (a1 - a0) * k / n)), cy + r0 * math.sin(math.radians(a0 + (a1 - a0) * k / n)), z) for k in range(n + 1)]
    return outer + inner[::-1]


def ellipse(cx, cy, rx, ry, n=48, z=0.0, wob=0.0, rng=None, phase=0.0):
    pts = []
    for k in range(n):
        a = 2 * math.pi * k / n
        f = 1 + (wob * math.sin(3 * a + phase) + wob * 0.6 * math.sin(5 * a + 2 * phase) if wob else 0)
        pts.append((cx + rx * f * math.cos(a), cy + ry * f * math.sin(a), z))
    return pts


CANAL_DENSE = smooth(CANAL, 10)
CREEK = [(1560, 0), (1520, 260), (1580, 560), (1700, 840), (1780, 1150), (1760, 1500)]


class Dubai:
    def __init__(self, c, quick=False, seed=2026):
        global DETAIL
        DETAIL = 0.25 if quick else 1.0
        self.c = c
        self.q = quick
        self.rng = random.Random(seed)
        self.M = make_materials(c)
        self.b = B(c)
        self.P = Parts(self.b, self.M, self.rng)
        self.creek = smooth(CREEK, 10)
        self.log = print
        self.t = time.time()

    def lap(self, label):
        self.b.flush()
        self.b.cat = None
        self.log("  %-30s %5.1f s  (%d commands, %.2f M triangles, %d solids)" % (label, time.time() - self.t, self.b.total, self.b.tris / 1e6, self.b.leaves))
        self.t = time.time()

    def blocked(self, x, y, pad=0.0):
        """True where something other than villas stands: used by the suburb filler."""
        for (x0, y0, x1, y1) in self.zones:
            if x0 - pad < x < x1 + pad and y0 - pad < y < y1 + pad:
                return True
        # the Marina canal and the Creek
        for (px, py) in CANAL_DENSE + self.creek:
            if abs(x - px) < 90 and abs(y - py) < 90:
                return True
        return False

    # -- sea, land, beaches ------------------------------------------------------

    def ground(self):
        b, M = self.b, self.M
        b.box((-9000, -9000, -40), (9000, 400, SEA), M["sea"], "Gulf")
        xs = list(range(-9000, 9001, 150))
        cp = [(x, coast(x), -3.0) for x in xs]
        b.poly(cp + [(9000, 9000, -3.0), (-9000, 9000, -3.0)], 3.0, M["sand"], "Land")
        # shallows and beach
        for (d0, mat, z) in ((330, "sh1", HALO), (110, "sh2", HALO2)):
            pts = [(x, coast(x) - d0 * (1 + 0.25 * math.sin(x / 400.0)), 0.0) for x in xs] + [(x, coast(x) + 4, 0.0) for x in xs[::-1]]
            pts = [(p[0], p[1], z) for p in pts]
            b.poly(pts, 0.015, M[mat], "Shallows")
        pts = [(x, coast(x) + 0, 0.0) for x in xs] + [(x, coast(x) + 26 + 8 * math.sin(x / 120.0), 0.0) for x in xs[::-1]]
        b.poly(pts, 0.05, M["beach"], "Beach")
        # surf line
        pts = [(x, coast(x) - 1.5, 0.0) for x in xs] + [(x, coast(x) + 1.0, 0.0) for x in xs[::-1]]
        pts = [(p[0], p[1], HALO2 + 0.02) for p in pts]
        b.poly(pts, 0.02, M["white"], "Surf")
        # patches of lighter and darker water break up the flat sea
        for k in range(20 if self.q else 80):
            x, y = self.rng.uniform(-4500, 4500), self.rng.uniform(-4500, -250)
            rx = self.rng.uniform(150, 600)
            b.poly(ellipse(x, y, rx, rx * self.rng.uniform(0.15, 0.4), n=36, z=SEA + 0.02 + k * 0.0004, wob=0.08, phase=k),
                   0.01, M["swell" if k % 2 else "swell2"], "Swell")
        self.lap("sea, land, beaches")

    # -- roads ------------------------------------------------------------------------

    def hroad(self, x0, x1, yc, lanes, lane_w=3.7, median=0.0, walk=6.0, z=0.08, mark=True, name="Road"):
        """An east-west road with ``lanes`` each way; returns its half width (edge of the pavement)."""
        b, M = self.b, self.M
        hw = median / 2 + lanes * lane_w + 2.0
        b.box((x0, yc - hw, 0), (x1, yc + hw, z), M["asphalt"], name)
        if walk:
            b.box((x0, yc + hw, 0), (x1, yc + hw + walk, 0.22), M["walk"], "Sidewalk")
            b.box((x0, yc - hw - walk, 0), (x1, yc - hw, 0.22), M["walk"], "Sidewalk")
        if mark:
            L = x1 - x0
            dash, pitch = 5.0, 24.0
            n = int(L / pitch)
            for sgn in (-1, 1):
                for k in range(1, lanes):
                    y = yc + sgn * (median / 2 + k * lane_w)
                    i = b.box((x0 + 2, y - 0.1, z), (x0 + 2 + dash, y + 0.1, z + 0.02), M["white"], "Lane marking")
                    b.copy(i, (pitch, 0, 0), n - 1)
                edge = yc + sgn * (median / 2 + lanes * lane_w + 0.6)
                b.box((x0, edge - 0.1, z), (x1, edge + 0.1, z + 0.02), M["white"], "Edge line")
                if median:
                    b.box((x0, yc + sgn * (median / 2 + 0.3) - 0.1, z), (x1, yc + sgn * (median / 2 + 0.3) + 0.1, z + 0.02), M["yellow"], "Median line")
        else:
            pass
        return hw + walk

    def vroad(self, y0, y1, xc, lanes, lane_w=3.7, walk=5.0, z=0.07, name="Street", mark=True):
        b, M = self.b, self.M
        hw = lanes * lane_w + 1.5
        b.box((xc - hw, y0, 0), (xc + hw, y1, z), M["asphalt"], name)
        b.box((xc + hw, y0, 0), (xc + hw + walk, y1, 0.22), M["walk"], "Sidewalk")
        b.box((xc - hw - walk, y0, 0), (xc - hw, y1, 0.22), M["walk"], "Sidewalk")
        if mark:
            n = int((y1 - y0) / 12)
            i = b.box((xc - 0.1, y0 + 2, z), (xc + 0.1, y0 + 5, z + 0.02), M["white"], "Lane marking")
            b.copy(i, (0, 12, 0), n - 1)
        return hw + walk

    def szr(self):
        b, M, rng, P = self.b, self.M, self.rng, self.P
        x0, x1 = SZR_X
        yc = SZR_Y
        L = x1 - x0
        # cross streets and district paving
        b.box((x0, yc - 150, 0), (x1, yc + 330, 0.04), M["szrpave"], "Paving")
        self.cross = [x for x in range(int(x0) + 120, int(x1), 186)]
        kx = KHALIFA[0]
        for xs in self.cross:
            if not (kx - 600 < xs < kx + 740):
                self.vroad(yc - 150, yc + 330, xs, 2, mark=True)
        hw = self.hroad(x0, x1, yc, 5, median=16.0, walk=8.0, name="Sheikh Zayed Road")
        self.szr_hw = hw
        # the median: raised, planted
        b.box((x0, yc - 8.0, 0), (x1, yc + 8.0, 0.4), M["conc"], "Median")
        b.box((x0, yc - 7.0, 0.4), (x1, yc + 7.0, 0.5), M["verge2"], "Median planting")
        # traffic: five lanes each way, bus-heavy in the outer lane
        b.cat = "street"
        names = P.car_names
        step = 3 if self.q else 1
        for sgn in (-1, 1):
            for lane in range(5):
                y = yc + sgn * (8 + 3.7 * lane + 1.85)
                x = x0 + rng.uniform(0, 30)
                while x < x1:
                    nm = names[rng.randrange(len(names))] if rng.random() > 0.05 else "bus"
                    b.place(P.d[nm], x, y + rng.uniform(-0.3, 0.3), 0.1, yaw=0 if sgn < 0 else 180)
                    x += rng.uniform(7.5, 24) * step * (1.3 if lane == 0 else 1.0)
                    b.tick()
        # street lights, palms and hedges down both sides and in the median
        for sgn in (-1, 1):
            y = yc + sgn * (hw - 3.5)
            x = x0 + 20
            while x < x1:
                b.place(P.d["lamp"], x, y, 0.22, yaw=0 if sgn < 0 else 180)
                x += 24 if not self.q else 90
                b.tick()
            y2 = yc + sgn * (hw - 1.2)
            x = x0 + 30
            while x < x1:
                b.place(P.d[rng.choice(("palm_m", "palm_l", "palm_m"))], x, y2, 0.2, yaw=rng.uniform(0, 360), s=rng.uniform(0.9, 1.2))
                x += 20 if not self.q else 70
                b.tick()
            x = x0 + 10
            while x < x1:  # the median: a palm between the piers and a trimmed hedge line
                b.place(P.d["palm_s"], x, yc + sgn * 6.9, 0.5, yaw=rng.uniform(0, 360), s=rng.uniform(0.8, 1.1))
                x += 9 if not self.q else 40
                b.tick()
            h = b.box((x0, yc + sgn * 7.6 - 0.35, 0.5), (x1, yc + sgn * 7.6 + 0.35, 1.2), M["hedge"], "Median hedge")
        # overhead gantry signs every 260 m, one over each carriageway, from the median edge to the shoulder; the
        # signs face the oncoming traffic (a mirror placement turns them round)
        x = x0 + 140
        while x < x1:
            b.place(P.d["gantry"], x, yc + 8.4, 0.1, s=-1, sy=1, sz=1)
            b.place(P.d["gantry"], x + 90, yc - 8.4, 0.1, s=1, sy=-1, sz=1)
            x += 260 if not self.q else 600
        self.lap("Sheikh Zayed Road")

    def metro(self):
        b, M, rng, P = self.b, self.M, self.rng, self.P
        x0, x1 = SZR_X
        yc = SZR_Y
        # the viaduct on piers down the median, with a train each way and two stations
        b.box((x0, yc - 5.4, 8.6), (x1, yc + 5.4, 10.2), M["conc"], "Viaduct deck")
        b.box((x0, yc - 5.4, 10.2), (x1, yc - 4.8, 11.1), M["conc"], "Parapet")
        b.box((x0, yc + 4.8, 10.2), (x1, yc + 5.4, 11.1), M["conc"], "Parapet")
        for sy in (-2.4, 2.4):
            b.box((x0, yc + sy - 0.8, 10.2), (x1, yc + sy + 0.8, 10.3), M["conc2"], "Track bed")
            for sr in (-0.7, 0.7):
                b.box((x0, yc + sy + sr - 0.05, 10.3), (x1, yc + sy + sr + 0.05, 10.4), M["steeld"], "Rail")
        n = int((x1 - x0) / 30)
        col = b.box((x0 + 5, yc - 1.0, 0.4), (x0 + 7.2, yc + 1.0, 8.6), M["conc"], "Pier")
        cap = b.box((x0 + 4.2, yc - 4.2, 7.4), (x0 + 8.0, yc + 4.2, 8.6), M["conc"], "Pier cap")
        b.copy(col, (30, 0, 0), n)
        b.copy(cap, (30, 0, 0), n)
        for xs in (-120.0, 340.0):
            # a pod-shaped station: a wide platform, a squashed ellipsoid roof with ribs, glass end walls, and a
            # covered footbridge over the highway with a lift tower at each end
            b.box((xs - 60, yc - 8.6, 8.6), (xs + 60, yc + 8.6, 10.2), M["conc"], "Station platform")
            b.place(P.d["pod"], xs, yc, 10.6, s=61, sy=10.2, sz=5.6)
            for q in range(-5, 6):
                b.box((xs + q * 10 - 0.25, yc - 9.6, 10.2), (xs + q * 10 + 0.25, yc + 9.6, 10.9), M["steelw"], "Roof rib")
            for q in (-1, 1):
                b.box((xs + q * 60 - 0.3, yc - 8, 10.2), (xs + q * 60 + 0.3, yc + 8, 13.2), M["deck"], "Station end wall")
                b.box((xs - 55, yc + q * 8.4 - 0.3, 10.2), (xs + 55, yc + q * 8.4 + 0.3, 12.6), M["deck"], "Station glazing")
            bx = xs + 40
            b.box((bx - 2, yc - 40, 6.0), (bx + 2, yc + 40, 6.5), M["conc"], "Footbridge")
            b.place(P.d["tube"], bx, yc, 7.6, yaw=90, s=80, sy=2.4, sz=1.8)
            for q in (-1, 1):
                b.box((bx - 3, yc + q * 40 - 3, 0.2), (bx + 3, yc + q * 40 + 3, 8.0), M["gl_blue2"], "Lift tower")
                b.box((bx - 3.3, yc + q * 40 - 3.3, 7.8), (bx + 3.3, yc + q * 40 + 3.3, 8.4), M["steelw"], "Lift tower roof")
        # trains: one at each station, one running
        for (xs, sy, yaw) in ((-150, 2.4, 0), (310, -2.4, 180), (-900, 2.4, 0), (700, -2.4, 180), (1300, 2.4, 0)):
            d = P.d["train"]
            if yaw == 0:
                b.place(d, xs, yc + sy, 10.4, yaw=0)
            else:
                b.place(d, xs + 82, yc + sy, 10.4, yaw=180)
        self.lap("Dubai Metro")

    def interchange(self):
        b, M, P, rng = self.b, self.M, self.P, self.rng
        yc = SZR_Y
        xc = -460.0
        w = 14.0
        # the cross road: a bridge over the highway with a ramp at either end. Each ramp is a profile
        # in the plane x = xc - w, pushed 2 w along +x
        for sgn in (-1, 1):
            ya, yb = yc + sgn * 70, yc + sgn * 190
            b.poly([(xc - w, ya, 8.0), (xc - w, yb, 0.0), (xc - w, yb, 1.2), (xc - w, ya, 9.2)], 2 * w, M["asphalt"], "Ramp")
            for sx in (-1, 1):
                b.box((xc + sx * w - (0.6 if sx < 0 else 0), min(ya, yb), 0), (xc + sx * w + (0 if sx < 0 else 0.6), max(ya, yb), 0.5), M["conc"], "Ramp kerb")
        b.box((xc - w, yc - 70, 8.0), (xc + w, yc + 70, 9.2), M["asphalt"], "Overpass")
        b.box((xc - w - 0.6, yc - 70, 9.2), (xc - w, yc + 70, 10.1), M["conc"], "Parapet")
        b.box((xc + w, yc - 70, 9.2), (xc + w + 0.6, yc + 70, 10.1), M["conc"], "Parapet")
        for py in (-62, -34, 34, 62):
            b.box((xc - 3, yc + py - 1.5, 0.3), (xc + 3, yc + py + 1.5, 8.0), M["conc"], "Pier")
        # loops: four ring decks one storey up, each a 270-degree arc, on piers
        R = 46.0
        for sx in (-1, 1):
            for sy in (-1, 1):
                cx = xc + sx * (w + 4 + R)
                cy = yc + sy * (self.szr_hw + 4 + R)
                # the arc leaves out the quarter that faces the crossing
                a0 = {(1, 1): 180, (-1, 1): 270, (1, -1): 90, (-1, -1): 0}[(sx, sy)] + 90
                b.poly(sector(cx, cy, R - 6, R + 6, a0, a0 + 270, z=3.4), 1.1, M["asphalt"], "Loop ramp")
                for q in range(0, 271, 15):
                    a = math.radians(a0 + q)
                    b.cyl((cx + R * math.cos(a), cy + R * math.sin(a), 0.2), 1.1, 3.2, M["conc"], "Loop pier", seg=8)
                    if q % 30 == 0:  # kerb posts and lane lines on the deck
                        b.cyl((cx + (R - 5.6) * math.cos(a), cy + (R - 5.6) * math.sin(a), 4.5), 0.12, 0.9, M["steelw"], "Barrier post", seg=4)
                        b.cyl((cx + (R + 5.6) * math.cos(a), cy + (R + 5.6) * math.sin(a), 4.5), 0.12, 0.9, M["steelw"], "Barrier post", seg=4)
                b.poly(sector(cx, cy, R - 0.15, R + 0.15, a0, a0 + 270, z=4.5), 0.03, M["yellow"], "Loop centre line")
                b.cyl((cx, cy, 0.0), R - 7.5, 0.3, M["lawn2"], "Infield", seg=32)
                for q in range(14):
                    aa = 2 * math.pi * q / 14 + 0.3
                    rr = rng.uniform(4, R - 12)
                    b.place(P.d["palm_m" if q % 3 else "acacia"], cx + rr * math.cos(aa), cy + rr * math.sin(aa), 0.3, yaw=q * 50,
                            s=rng.uniform(0.9, 1.3))
                # lead-in along the highway and exit along the cross road, both descending to the ground
                ex = cx + sx * 230
                b.poly([(cx, cy - sy * R - 6, 3.4), (ex, cy - sy * R - 6, 0.0), (ex, cy - sy * R - 6, 1.1), (cx, cy - sy * R - 6, 4.5)],
                       12, M["asphalt"], "Slip ramp")
                ey = cy + sy * 170
                b.poly([(cx - sx * R - 6, cy, 3.4), (cx - sx * R - 6, ey, 0.0), (cx - sx * R - 6, ey, 1.1), (cx - sx * R - 6, cy, 4.5)], 12, M["asphalt"], "Exit ramp")
                for q in range(1, 9):
                    fr = q / 9.0
                    zh = 3.4 * (1 - fr)
                    if zh > 0.6:
                        b.box((cx + sx * 230 * fr - 1, cy - sy * R - 1, 0.1), (cx + sx * 230 * fr + 1, cy - sy * R + 1, zh), M["conc"], "Ramp pier")
                        b.box((cx - sx * R - 1, cy + sy * 170 * fr - 1, 0.1), (cx - sx * R + 1, cy + sy * 170 * fr + 1, zh), M["conc"], "Ramp pier")
        self.lap("interchange")

    # -- towers along Sheikh Zayed Road -----------------------------------------------

    def envelope(self, x):
        """The typical tower height along the coast: low between the clusters, peaks at the Emirates
        Towers and Downtown, and in the Marina."""
        return (60 + 190 * math.exp(-((x - 200) / 520.0) ** 2) + 210 * math.exp(-((x - 760) / 330.0) ** 2)
                + 160 * math.exp(-((x + 1330) / 300.0) ** 2))

    def canyon(self):
        b, M, rng, P = self.b, self.M, self.rng, self.P
        classes = (70, 110, 160, 220, 290) if not self.q else (110, 220)
        pool = P.tower_pool(classes, 2 if self.q else 4, name="SZR")
        self.lap("tower archetypes")
        x0, x1 = SZR_X
        yc = SZR_Y
        hw = self.szr_hw
        rows = (hw + 34, hw + 108, hw + 190)
        for row, off in enumerate(rows):
            for sgn in (-1, 1):
                if sgn < 0 and row == 2:
                    continue
                x = x0 + 40 + rng.uniform(0, 30)
                while x < x1 - 40:
                    pitch = 62 + 12 * row
                    if any(abs(x - cx) < 22 for cx in self.cross) or self.in_zone(x, yc + sgn * off, 30):
                        x += 24
                        continue
                    if rng.random() < (0.9 if row == 0 else 0.8 if row == 1 else 0.6) * (0.4 if self.q else 1.0):
                        h = self.envelope(x) * rng.uniform(0.55, 1.15) * (1.0 - 0.18 * row)
                        h = max(55, min(330, h))
                        d, sz, s = P.pick(pool, h)
                        yaw = 0 if sgn > 0 else 180
                        sx = rng.uniform(0.9, 1.15)
                        b.place(d, x, yc + sgn * off, 0.0, yaw=yaw + rng.choice((0, 0, 0, 180)), s=sx, sz=sz, sy=sx)
                    x += pitch * rng.uniform(0.95, 1.2)
                    b.tick()
        self.lap("towers along the road")

    def in_zone(self, x, y, pad=0.0):
        for (x0, y0, x1, y1) in self.zones:
            if x0 - pad < x < x1 + pad and y0 - pad < y < y1 + pad:
                return True
        return False

    # -- Emirates Towers --------------------------------------------------------------

    def emirates(self):
        b, M, rng, P = self.b, self.M, self.rng, self.P
        yc = SZR_Y
        offices = dict(kind="poly", n=3, phi=math.radians(90), w=74, h=355, glass="gl_silver", band="steel", fin="steelw",
                       crown="slope", fh=4.2, lit=0.3, fins=True, podium=0)
        hotel = dict(offices, w=64, h=305, glass="gl_blue2")
        # the pair stand on a shared podium; the triangles point at each other
        x0, y0 = 120.0, yc + 110.0
        b.box((x0 - 70, y0 - 55, 0), (x0 + 150, y0 + 55, 0.25), M["plaza"], "Plaza")
        b.box((x0 - 10, y0 - 40, 0.2), (x0 + 130, y0 + 40, 17), M["gl_dark"], "Podium")
        b.box((x0 - 10, y0 - 40, 17), (x0 + 130, y0 + 40, 18.2), M["bandw"], "Podium roof")
        n = 4
        bi = b.box((x0 - 10.4, y0 - 40.4, 3.4), (x0 + 130.4, y0 + 40.4, 3.9), M["bandw"], "Podium band")
        b.copy(bi, (0, 0, 4.4), 3)
        for (s, px, phi) in ((offices, x0 + 5, math.radians(0)), (hotel, x0 + 110, math.radians(180))):
            s = dict(s, phi=phi + math.pi / 2 * 0)
            d = b.comp("Emirates %d" % s["h"], lambda s=s: build_tower(b, M, rng, dict(s, phi=s["phi"])))
            b.place(d, px, y0, 0.0)
        self.lap("Emirates Towers")

    # -- Cayan Tower -------------------------------------------------------------------

    def cayan(self):
        b, M, rng, P = self.b, self.M, self.rng, self.P
        cx, cy = CAYAN
        floors = 73 if not self.q else 30
        fh = 4.2

        def plate():
            out = [b.box((-19, -13, 0), (19, 13, 0.7), M["bandw"], "Slab"),
                   b.box((-18.2, -12.2, 0.7), (18.2, 12.2, fh), M["gl_teal"], "Glass"),
                   b.cyl((0, 0, 0.7), 5, fh - 0.7, M["conc"], "Core", seg=12)]
            for sx in (-1, 1):
                for sy in (-1, 1):
                    out.append(b.box((sx * 18.2 - 0.3, sy * 12.2 - 0.3, 0.7), (sx * 18.2 + 0.3, sy * 12.2 + 0.3, fh), M["steelw"], "Corner post"))
            if floors:
                out.append(b.box((-17, -12.5, 2.9), (17, 12.5, 3.3), M["lit_gl_teal"], "Lit strip"))
            return out
        d = b.comp("Cayan floor", plate)
        b.box((cx - 55, cy - 40, 0), (cx + 55, cy + 40, 0.25), M["plaza"], "Plaza")
        b.box((cx - 30, cy - 22, 0.2), (cx + 30, cy + 22, 14), M["gl_dark"], "Podium")
        b.box((cx - 30.5, cy - 22.5, 14), (cx + 30.5, cy + 22.5, 15), M["bandw"], "Podium roof")
        for k in range(floors):
            b.place(d, cx, cy, 14 + k * fh, yaw=k * 90.0 / (floors - 1) + 10)
            b.tick()
        top = 14 + floors * fh
        b.box((cx - 12, cy - 9, top), (cx + 12, cy + 9, top + 4), M["roofd"], "Plant")
        b.cyl((cx, cy, top + 4), 0.4, 22, M["steel"], "Mast", seg=6)
        self.lap("Cayan Tower")

    # -- Marina ----------------------------------------------------------------------

    def marina(self):
        b, M, rng, P = self.b, self.M, self.rng, self.P
        pts = smooth(CANAL, 10)
        # quays and water
        b.poly(band(pts, 74), 0.3, M["quay"], "Quay")
        b.poly(band(pts, 60), 0.42, M["canal"], "Canal")
        # promenade along both sides
        for side in (1, -1):
            edge = offset_line(pts, side * 52)
            b.poly(band(edge, 26), 0.1, M["plaza"], "Promenade")
        # towers on both banks
        classes = (120, 170, 230, 290) if not self.q else (170, 290)
        pool = P.tower_pool(classes, 2 if self.q else 3, name="Marina")
        jbr = P.tower_pool((90, 130), 2 if self.q else 4, name="JBR")
        self.lap("Marina archetypes")
        # arclength walk
        s = 0.0
        acc = [0.0]
        for i in range(1, len(pts)):
            acc.append(acc[-1] + math.hypot(pts[i][0] - pts[i - 1][0], pts[i][1] - pts[i - 1][1]))
        total = acc[-1]

        def at(sv):
            for i in range(1, len(pts)):
                if acc[i] >= sv:
                    t = (sv - acc[i - 1]) / max(acc[i] - acc[i - 1], 1e-6)
                    x = pts[i - 1][0] + (pts[i][0] - pts[i - 1][0]) * t
                    y = pts[i - 1][1] + (pts[i][1] - pts[i - 1][1]) * t
                    ang = math.atan2(pts[i][1] - pts[i - 1][1], pts[i][0] - pts[i - 1][0])
                    return x, y, ang
            return pts[-1][0], pts[-1][1], 0.0
        self.marina_pos = []
        sv = 40.0
        while sv < total - 30:
            x, y, ang = at(sv)
            for side in (1, -1):
                off = 62 + 18
                px = x - math.sin(ang) * off * side
                py = y + math.cos(ang) * off * side
                if side < 0 and py < coast(px) + 70:
                    continue
                if math.hypot(px - CAYAN[0], py - CAYAN[1]) < 62:
                    continue
                if rng.random() < 0.9:
                    centre = abs(sv - total / 2) / (total / 2)
                    h = 330 * (1 - 0.55 * centre) * rng.uniform(0.45, 1.05)
                    h = max(110, h)
                    d, sz, spec = P.pick(pool, h)
                    b.place(d, px, py, 0.0, yaw=math.degrees(ang) + (90 if side > 0 else -90) + rng.choice((0, 180)) - 90 * 0, sz=sz,
                            s=1.0)
                    self.marina_pos.append((px, py))
            sv += 62 + rng.uniform(-6, 10)
            b.tick()
        # Princess Tower: the tallest, on the bank
        spec = dict(kind="box", w=44, d=30, h=414, glass="gl_blue", band="steelw", fin="steel", crown="spire", fh=4.2, pitch=2.4,
                    lit=0.3, podium=24, tiers=[(0.6, 1.0, 0, 0), (0.4, 0.88, 0, 0)])
        d = b.comp("Princess Tower", lambda: build_tower(b, M, rng, spec))
        b.place(d, -1450, 262, 0.0, yaw=0)
        # JBR: slabs between the canal and the beach
        x = -1760.0
        while x < -1020:
            y = coast(x) + 70
            if y < 135 and not any(abs(x - px) < 50 and abs(y - py) < 50 for px, py in self.marina_pos):
                h = rng.uniform(85, 150)
                d, sz, s = P.pick(jbr, h)
                b.place(d, x, y + rng.uniform(-3, 3), 0.0, yaw=0, sz=sz, s=rng.uniform(1.0, 1.25))
            x += 64
            b.tick()
        # yachts and docks
        for k in range(14 if self.q else 70):
            sv = rng.uniform(70, total - 70)
            x, y, ang = at(sv)
            side = rng.choice((-1, 1))
            off = side * rng.uniform(10, 22)
            b.place(P.d[rng.choice(P.yachts)], x - math.sin(ang) * off, y + math.cos(ang) * off, 0.42,
                    yaw=math.degrees(ang) + rng.choice((0, 180)), s=rng.uniform(0.85, 1.2))
            b.tick()
        # jetties reach into the water from both quays
        for sv in range(60, int(total) - 40, 34):
            x, y, ang = at(sv)
            for side in (1, -1):
                px = x - math.sin(ang) * 33 * side
                py = y + math.cos(ang) * 33 * side
                j = b.box((0, -0.6, 0.42), (11, 0.6, 0.62), M["teak"], "Jetty")
                b.rot(j, [0, 0, 1], math.degrees(ang) + (-90 if side > 0 else 90), (0, 0, 0))
                b.move(j, [px, py, 0])
            b.tick()
        self.lap("Dubai Marina")

    # -- Downtown -----------------------------------------------------------------------

    def downtown(self):
        b, M, rng, P = self.b, self.M, self.rng, self.P
        kx, ky = KHALIFA
        b.box((kx - 570, ky - 480, 0), (kx + 720, ky + 260, 0.06), M["plaza2"], "Downtown paving")
        khalifa(b, M, rng, self.q)
        self.lap("Burj Khalifa")
        # Burj Lake and its fountain, in front (towards the Gulf)
        lcx, lcy = kx - 10, ky - 235
        lake = ellipse(lcx, lcy, 270, 118, n=72, wob=0.045, phase=0.5)
        b.poly(ellipse(lcx, lcy, 290, 136, n=72, wob=0.045, phase=0.5), 0.2, M["plaza"], "Promenade")
        b.poly(ellipse(lcx, lcy, 276, 124, n=72, wob=0.045, phase=0.5), 0.3, M["quay"], "Lake wall")
        b.poly(lake, 0.42, M["lake"], "Burj Lake")
        # the fountain: a line of plumes along the lake, two flanks leaning outwards, and the big jets
        nx = 30 if not self.q else 8
        for k in range(nx):
            x = lcx - 210 + k * (420.0 / (nx - 1))
            h = 14 + 22 * abs(math.sin(k * 0.5)) + (8 if k % 5 == 0 else 0)
            b.place(P.d["plume"], x, lcy + 12 * math.sin(k * 0.3), 0.42, s=0.17 * h, sz=h)
            for sgn in (-1, 1):
                if k % 2 == (0 if sgn > 0 else 1):
                    h2 = 8 + 3 * (k % 3)
                    b.place(P.d["plume"], x, lcy + sgn * 34, 0.42, s=0.15 * h2, sz=h2, lean=(0, -sgn * 0.55))
                    h3 = 5 + 2 * (k % 4)
                    b.place(P.d["plume"], x + 3, lcy + sgn * 58, 0.42, s=0.15 * h3, sz=h3, lean=(0, -sgn * 0.7))
        for k, hh in enumerate((55, 78, 66, 84, 60, 50)):
            b.place(P.d["plume"], lcx - 100 + k * 40, lcy + 6, 0.42, s=0.16 * hh, sz=hh)
        # the Address Downtown and the towers round the lake
        pool = P.tower_pool((110, 170, 230), 2 if self.q else 3, name="Downtown")
        address = dict(kind="box", w=34, d=34, h=302, glass="gl_silver", band="bandw", fin="steelw", crown="lantern", fh=4.0,
                       pitch=2.4, lit=0.32, podium=26, tiers=[(0.52, 1.0, 0, 0), (0.26, 0.87, 0, 0), (0.14, 0.74, 0, 0), (0.08, 0.6, 0, 0)])
        d = b.comp("Address Downtown", lambda: build_tower(b, M, rng, address))
        b.place(d, kx - 175, ky - 10, 0.0)
        vista = dict(kind="box", w=38, d=28, h=185, glass="gl_blue", band="steel", fin="bandw", crown="ring", fh=3.8, lit=0.3,
                     podium=14, tiers=[(0.7, 1.0, 0, 0), (0.3, 0.8, 0, 0)])
        for (px, py) in ((kx + 215, ky - 100), (kx + 265, ky - 100)):
            d = b.comp("Burj Vista %d" % px, lambda: build_tower(b, M, rng, vista))
            b.place(d, px, py, 0.0)
        # a ring of towers on a street grid
        step = 78.0 if not self.q else 156.0
        gx = kx - 520
        while gx < kx + 560:
            gy = ky - 390
            while gy < ky + 250:
                dx, dy = gx - kx, gy - ky
                near = math.hypot(dx, dy)
                inlake = ((gx - lcx) / 345) ** 2 + ((gy - lcy) / 200) ** 2 < 1
                mall = 20 < dx < 360 and -195 < dy < 75
                addr = abs(gx - (kx - 175)) < 60 and abs(gy - (ky - 10)) < 60
                vst = abs(gy - (ky - 100)) < 45 and 200 < dx < 300
                if near > 130 and not (abs(gx - kx) < 150 and gy < ky - 210) and not inlake and not mall and not addr and not vst and rng.random() < 0.78 and abs(gy - SZR_Y) > 190:
                    h = rng.uniform(95, 260) * (1.0 - 0.0006 * max(0, near - 200)) * (0.55 if gy < lcy else 1.0)
                    d, sz, s = P.pick(pool, h)
                    b.place(d, gx + rng.uniform(-6, 6), gy + rng.uniform(-6, 6), 0.0, yaw=rng.choice((0, 90, 180, 270)), sz=sz,
                            s=rng.uniform(0.9, 1.1))
                gy += step
                b.tick()
            gx += step
        # Business Bay: the next blocks east, taller
        gx = kx + 430
        while gx < kx + 720 and not self.q:
            gy = ky - 300
            while gy < ky + 250:
                if not (abs(gy - (ky - 100)) < 45 and gx < kx + 300) and rng.random() < 0.8 and gy > ky - 330:
                    h = rng.uniform(110, 280)
                    d, sz, s_ = P.pick(pool, h)
                    b.place(d, gx + rng.uniform(-6, 6), gy + rng.uniform(-6, 6), 0.0, yaw=rng.choice((0, 90, 180, 270)), sz=sz,
                            s=rng.uniform(0.9, 1.1))
                gy += step
                b.tick()
            gx += step
        self.lap("Downtown towers")
        # the Dubai Mall: a long low block with a glass vault, car parks and the Souk
        mx0, mx1, my0, my1 = kx + 40, kx + 340, ky - 170, ky - 30
        b.box((mx0 - 20, my0 - 20, 0), (mx1 + 20, my1 + 20, 0.3), M["plaza"], "Mall forecourt")
        b.box((mx0, my0, 0.3), (mx1, my1, 30), M["sandstone"], "Dubai Mall")
        nb = b.box((mx0 - 0.4, my0 - 0.4, 6), (mx1 + 0.4, my1 + 0.4, 6.6), M["bandw"], "Mall band")
        b.copy(nb, (0, 0, 6), 3)
        b.box((mx0 + 20, my0 - 0.5, 1), (mx1 - 20, my0 + 0.1, 22), M["gl_blue2"], "Mall glass front")
        b.cyl((mx0 + 20, (my0 + my1) / 2, 30), 22, mx1 - mx0 - 40, M["gl_silver"], "Mall vault", axis=(1, 0, 0), seg=24)
        b.box((mx0, my0, 30), (mx1, my1, 31.2), M["bandw"], "Mall roof")
        for q in range(8):
            b.box((mx0 + 15 + q * 36, my1 + 0.1, 8), (mx0 + 15 + q * 36 + 6, my1 + 2, 26), M["gold"], "Entrance")
        # car parks behind
        for q in range(3):
            b.box((mx0 + 8 + q * 100, my1 + 30, 0.3), (mx0 + 98 + q * 100, my1 + 90, 22), M["conc"], "Car park")
            pb = b.box((mx0 + 7.6 + q * 100, my1 + 29.6, 3), (mx0 + 98.4 + q * 100, my1 + 90.4, 3.6), M["bandg"], "Deck edge")
            b.copy(pb, (0, 0, 3.6), 4)
        # the Souk
        b.box((kx - 110, ky - 150, 0.3), (kx - 40, ky - 90, 18), M["cream"], "Souk Al Bahar")
        sb = b.box((kx - 110.4, ky - 150.4, 5), (kx - 39.6, ky - 89.6, 5.5), M["bandw"], "Souk band")
        b.copy(sb, (0, 0, 4), 2)
        # streets through Downtown
        self.hroad(kx - 560, kx + 700, ky + 150, 3, walk=7.0, name="Boulevard")
        self.vroad(ky - 380, ky + 240, kx + 430, 3, walk=7.0)
        self.vroad(ky - 380, ky + 240, kx - 345, 3, walk=7.0)
        # lamps and palms along the boulevard
        for x in range(int(kx - 540), int(kx + 680), 36):
            b.place(P.d["lamp"], x, ky + 150 - 15, 0.22, yaw=0)
            b.place(P.d["palm_m"], x + 12, ky + 150 - 22, 0.2, yaw=rng.uniform(0, 360))
            b.place(P.d["lamp"], x, ky + 150 + 15, 0.22, yaw=180)
            b.tick()
        self.lap("Dubai Mall, fountain, streets")

    # -- Burj Al Arab ---------------------------------------------------------------------

    def burj_al_arab(self):
        b, M, rng, P = self.b, self.M, self.rng, self.P
        cx, cy = BAA
        H = 321.0
        xl = -48.0
        # the island and its shallows, and the bridge to the beach
        b.poly(ellipse(cx, cy, 215, 175, n=64, z=HALO), 0.015, M["sh1"], "Island shallows")
        b.poly(ellipse(cx, cy, 150, 125, n=64, z=HALO2), 0.015, M["sh2"], "Island shallows")
        b.poly(ellipse(cx, cy, 108, 92, n=64, z=-1.0, wob=0.03), 1.05, M["beach"], "Island")
        b.poly(ellipse(cx, cy, 98, 82, n=64, z=0.0, wob=0.03), 0.1, M["plaza"], "Island deck")
        yb0 = cy + 92
        yb1 = coast(cx) + 4
        b.poly([(cx - 8, yb0, 0.0), (cx + 8, yb0, 0.0), (cx + 8, yb1, 0.0), (cx - 8, yb1, 0.0)], 0.5, M["asphalt"], "Bridge")
        n = int((yb1 - yb0) / 28)
        pr = b.box((cx - 9, yb0 + 14, -2.0), (cx + 9, yb0 + 16, 0.0), M["conc"], "Bridge pier")
        b.copy(pr, (0, 28, 0), n - 1)
        for sx in (-8.4, 8.4):
            b.box((cx + sx - 0.2, yb0, 0.5), (cx + sx + 0.2, yb1, 1.3), M["steelw"], "Bridge rail")
        for k in range(10):
            for sx in (-9, 9):
                b.place(P.d["lamp"], cx + sx, yb0 + 14 + k * (yb1 - yb0 - 20) / 9.0, 0.5, yaw=180 if sx > 0 else 0, s=0.7)
        # the hotel: strips of dark glass, a curved sail strip in front of each
        zs = 8.0
        k = 0
        cyls = []
        while zs < H - 8:
            th = 12.0
            z1 = min(zs + th, H - 2)
            zm = (zs + z1) / 2
            W = 108 * (1 - (zm / H) ** 1.5) + 5
            xr = xl + W
            sag = 0.24 * W
            xm = (xl + xr) / 2
            outer, inner = [], []
            for j in range(13):
                x = xl + W * j / 12.0
                u = (x - xm) / (W / 2)
                ys = -sag * (1 - u * u)
                outer.append((cx + x, cy + ys - 0.8, zs))
                inner.append((cx + x, cy + ys + 0.8, zs))
            b.poly(outer + inner[::-1], z1 - zs, M["sail" if k % 2 == 0 else "sail2"], "Sail")
            body = [(cx + x, cy + ys + 1.2, zs) for (x, ys, _) in [(p[0] - cx, p[1] - cy, 0) for p in inner]]
            body = body + [(cx + xr - 3, cy + 24, zs), (cx + xl, cy + 24, zs)]
            b.poly(body, z1 - zs, M["gl_blue" if k % 3 else "gl_dark"], "Atrium")
            # lattice on the sail: ribs and a brace from rib to rib
            ys_at = lambda x: -sag * (1 - ((x - xm) / (W / 2)) ** 2) - 1.0  # noqa: E731
            for j in range(1, 6):
                x = xl + W * j / 6.0
                cyls.append(b.cyl((cx + x, cy + ys_at(x), zs), 0.45, z1 - zs, M["steelw"], "Sail rib", seg=5))
            for j in range(0, 5):
                xa = xl + W * j / 6.0
                xb = xl + W * (j + 1) / 6.0
                if (j + k) % 2:
                    xa, xb = xb, xa
                pa = (cx + xa, cy + ys_at(xa), zs)
                pb = (cx + xb, cy + ys_at(xb), z1)
                v = [pb[i] - pa[i] for i in range(3)]
                cyls.append(b.cyl(pa, 0.32, math.sqrt(sum(q * q for q in v)), M["steelw"], "Sail brace", axis=v, seg=5))
            zs = z1
            k += 1
            b.tick()
        # the mast and the two legs of the exoskeleton
        b.box((cx + xl - 7, cy - 4, 0), (cx + xl + 1, cy + 26, H), M["steelw"], "Mast spine")
        b.cyl((cx + xl - 3, cy + 11, H), 1.8, 40, M["steelw"], "Mast", seg=8)
        b.cone((cx + xl - 3, cy + 11, H + 40), 1.8, 30, M["steel"], "Mast tip", top=0.3, seg=8)
        for dy in (-1, 1):
            base = (cx + xl + 30, cy + dy * 26 + 12, 0.0)
            top = (cx + xl - 3, cy + 11 + dy * 4, 200.0)
            v = [top[i] - base[i] for i in range(3)]
            b.cyl(base, 2.6, math.sqrt(sum(q * q for q in v)), M["steelw"], "Leg", axis=v, seg=8)
        # the podium, atrium and the helipad
        b.poly(ellipse(cx + 10, cy - 6, 58, 40, n=40, z=0.1), 14, M["render"], "Podium")
        b.poly(ellipse(cx + 10, cy - 6, 59, 41, n=40, z=14.1), 1.0, M["bandw"], "Podium roof")
        hx, hy = cx + xl - 5, cy - 38
        b.box((hx - 3, hy + 6, 207), (hx + 3, cy + 6, 209.5), M["steelw"], "Helipad arm")
        b.cyl((hx, hy, 209.4), 13, 0.7, M["helipad"], "Helipad ring", seg=28)
        b.cyl((hx, hy, 209.9), 11.4, 0.5, M["bandw"], "Helipad", seg=28)
        for (a, bb, c_, d_) in ((-3.5, -4.5, -2.5, 4.5), (2.5, -4.5, 3.5, 4.5), (-3.5, -0.4, 3.5, 0.4)):
            b.box((hx + a, hy + bb, 210.3), (hx + c_, hy + d_, 210.45), M["helipad"], "H")
        b.cyl((hx, hy, 209.0), 13.3, 0.4, M["deck"], "Helipad lights", seg=28)
        # palms on the island
        for j in range(26):
            a = 2 * math.pi * j / 26
            b.place(P.d["palm_m"], cx + 88 * math.cos(a), cy + 72 * math.sin(a), 0.1, yaw=j * 40, s=0.8)
        self.lap("Burj Al Arab")

    # -- Ain Dubai ---------------------------------------------------------------------

    def ain_dubai(self):
        b, M, rng, P = self.b, self.M, self.rng, self.P
        cx, cy = -1120.0, -380.0
        R = 125.0
        hz = 140.0
        b.poly(ellipse(cx, cy, 190, 150, n=48, z=HALO), 0.015, M["sh1"], "Bluewaters shallows")
        b.poly(ellipse(cx, cy, 140, 105, n=48, z=-1.0, wob=0.04), 1.05, M["beach"], "Bluewaters")
        b.poly(ellipse(cx, cy, 128, 92, n=48, z=0.0, wob=0.04), 0.12, M["plaza"], "Bluewaters deck")
        for dy in (-7.0, 7.0):
            b.torus((cx, cy + dy, hz), R, 1.7, (0, 1, 0), M["wheel"], "Rim", seg=72, tseg=6)
            b.torus((cx, cy + dy, hz), R - 12, 0.7, (0, 1, 0), M["steelw"], "Inner rim", seg=48, tseg=5)
        for k in range(48):
            a = 2 * math.pi * k / 48
            for dy in (-7.0, 7.0):
                px, pz = cx + (R - 1) * math.cos(a), hz + (R - 1) * math.sin(a)
                v = (px - cx, 0.7 * dy, pz - hz)
                b.cyl((cx, cy + dy * 0.3, hz), 0.28, math.sqrt(sum(q * q for q in v)), M["steelw"], "Spoke", axis=v, seg=4)
            # struts across between the two rims
            b.cyl((cx + R * math.cos(a), cy - 7, hz + R * math.sin(a)), 0.35, 14, M["steelw"], "Strut", axis=(0, 1, 0), seg=4)
            rr = R + 3.2
            b.place(P.d["cabin"], cx + rr * math.cos(a), cy, hz + rr * math.sin(a), yaw=0)
        b.cyl((cx, cy - 12, hz), 8, 24, M["steelw"], "Hub", axis=(0, 1, 0), seg=16)
        for sgn in (-1, 1):
            for dx in (-1, 1):
                base = (cx + dx * 62, cy + sgn * 40, 0.0)
                top = (cx + dx * 3, cy + sgn * 9, hz)
                v = [top[i] - base[i] for i in range(3)]
                b.cyl(base, 4.0, math.sqrt(sum(q * q for q in v)), M["steelw"], "Support leg", axis=v, seg=10)
        self.lap("Ain Dubai")

    # -- Palm Jumeirah ---------------------------------------------------------------------

    def frond(self, L):
        """A frond of the palm along +y from the origin: beach, land, a road, rows of villas and palms."""
        b, M, rng, P = self.b, self.M, self.rng, self.P
        W = 19.0

        def parts():
            out = [b.box((-W - 44, -26, -2.0), (W + 44, L + 4, HALO + 0.015), M["sh1"], "Frond shallows"),
                   b.cyl((0, L + 4, -2.0), W + 44, 2.0 + HALO + 0.015, M["sh1"], "Frond shallows", seg=24),
                   b.box((-W - 16, -26, -2.0), (W + 16, L + 4, HALO2 + 0.015), M["sh2"], "Frond shallows"),
                   b.cyl((0, L + 4, -2.0), W + 16, 2.0 + HALO2 + 0.015, M["sh2"], "Frond shallows", seg=24),
                   b.box((-W - 3, -4, -1.2), (W + 3, L, 0.02), M["beach"], "Beach"),
                   b.cyl((0, L, -1.2), W + 3, 1.22, M["beach"], "Beach cap", seg=24),
                   b.box((-W, -4, -1.2), (W, L, 0.06), M["sand2"], "Frond land"),
                   b.cyl((0, L, -1.2), W, 1.26, M["sand2"], "Frond cap", seg=24),
                   b.box((-4.8, -4, 0.0), (4.8, L, 0.12), M["asphalt"], "Frond road"),
                   b.cyl((0, L, 0.0), 10, 0.12, M["asphalt"], "Frond turning circle", seg=20),
                   b.box((-0.1, 0, 0.12), (0.1, L, 0.14), M["yellow"], "Centre line")]
            y = 14.0
            side = 0
            while y < L - 14:
                for sx in (-1, 1):
                    nm = P.villa_names_p[rng.randrange(len(P.villa_names_p))]
                    out.append(b.place(P.d[nm], sx * 13.2, y, 0.06, yaw=90 * sx, s=0.56))
                if int(y / 20) % 2 == 0:
                    out.append(b.place(P.d["palm_s"], sx * 6.4, y + 10, 0.1, yaw=y * 17))
                y += 19.0
            # the turning circle's palms
            for q in range(5):
                a = 2 * math.pi * q / 5
                out.append(b.place(P.d["palm_m"], 13 * math.cos(a), L + 13 * math.sin(a), 0.1, yaw=q * 70))
            return out
        return b.comp("Palm frond %d" % L, parts)

    def palm(self):
        b, M, rng, P = self.b, self.M, self.rng, self.P
        px, py = PALM
        # the trunk reaches from the shore to the crown
        yt1 = coast(px) + 6
        b.box((px - 90, py, -2.0), (px + 90, yt1, HALO + 0.015), M["sh1"], "Trunk shallows")
        b.box((px - 62, py, -2.0), (px + 62, yt1, HALO2 + 0.015), M["sh2"], "Trunk shallows")
        b.box((px - 54, py, -1.2), (px + 54, yt1, 0.02), M["beach"], "Trunk beach")
        b.box((px - 51, py, -1.2), (px + 51, yt1, 0.06), M["sand2"], "Trunk land")
        b.box((px - 11, py, 0.0), (px + 11, yt1, 0.12), M["asphalt"], "Trunk road")
        mk = b.box((px - 0.15, py + 4, 0.12), (px + 0.15, py + 8, 0.14), M["white"], "Trunk marking")
        b.copy(mk, (0, 12, 0), int((yt1 - py) / 12) - 1)
        for sx in (-5.0, 5.0):
            mk = b.box((px + sx - 0.1, py + 4, 0.12), (px + sx + 0.1, py + 8, 0.14), M["white"], "Trunk marking")
            b.copy(mk, (0, 12, 0), int((yt1 - py) / 12) - 1)
        # the crown: a disc with a ring road
        b.cyl((px, py, -1.2), 232, 1.22, M["beach"], "Crown beach", seg=64)
        b.cyl((px, py, -1.2), 222, 1.26, M["sand2"], "Crown", seg=64)
        b.add(type="add_tube", base=[px, py, 0.06], radius=118, inner_radius=100, height=0.06, segments=64, name="Ring road", material=M["asphalt"].id)
        b.cyl((px, py, -2.0), 330, 2.0 + HALO + 0.015, M["sh1"], "Crown shallows", seg=64)
        b.cyl((px, py, -2.0), 268, 2.0 + HALO2 + 0.015, M["sh2"], "Crown shallows", seg=64)
        # resorts on the trunk and round the crown
        pool = P.tower_pool((45, 80, 120), 2 if self.q else 4, name="Palm")
        y = py + 230
        while y < yt1 - 70:
            for sx in (-1, 1):
                if rng.random() < 0.85:
                    h = rng.uniform(40, 120)
                    d, sz, s = P.pick(pool, h)
                    b.place(d, px + sx * 36, y + rng.uniform(-5, 5), 0.06, yaw=90 * sx, sz=sz, s=0.72)
            y += 58
            b.tick()
        for q in range(12 if not self.q else 4):
            a = 2 * math.pi * (q + 0.5) / (12 if not self.q else 4)
            if abs(math.degrees(a) - 270) < 25 or abs(math.degrees(a) - 90) < 22:
                continue
            h = rng.uniform(35, 90)
            d, sz, s = P.pick(pool, h)
            b.place(d, px + 150 * math.cos(a), py + 150 * math.sin(a), 0.06, yaw=math.degrees(a) + 90, sz=sz, s=0.8)
            b.place(d, px + 188 * math.cos(a + 0.1), py + 188 * math.sin(a + 0.1), 0.06, yaw=math.degrees(a) + 90, sz=sz * 0.7, s=0.7)
        for q in range(26 if not self.q else 6):
            a = 2 * math.pi * q / (26 if not self.q else 6)
            b.place(P.d["palm_l"], px + 108 * math.cos(a), py + 108 * math.sin(a), 0.06, yaw=q * 33)
        b.flush()
        # sixteen fronds fanning from the crown, away from the shore
        comps = {}
        for j in range(8 if not self.q else 3):
            comps[j] = self.frond(int(430 - 15 * j))
        self.lap("Palm fronds")
        for j in range(8 if not self.q else 3):
            for sgn in (-1, 1):
                alpha = sgn * (7.0 + 13.2 * j)
                th = math.radians(alpha)
                # away from the shore is -y; alpha turns it round
                dx, dy = math.sin(th), -math.cos(th)
                b.place(comps[j], px + 200 * dx, py + 200 * dy, 0.0, yaw=180 + alpha)
        # the crescent breakwater with its halo, and Atlantis on top
        R0, R1 = 725.0, 765.0
        a0, a1 = -90 - 120, -90 + 120
        b.poly(sector(px, py, R0 - 55, R1 + 55, a0, a1, step=3, z=HALO), 0.015, M["sh1"], "Crescent shallows")
        b.poly(sector(px, py, R0 - 20, R1 + 20, a0, a1, step=3, z=HALO2), 0.015, M["sh2"], "Crescent shallows")
        b.poly(sector(px, py, R0, R1, a0, a1, step=3, z=-1.2), 1.26, M["sand2"], "Crescent")
        b.poly(sector(px, py, (R0 + R1) / 2 - 9, (R0 + R1) / 2 + 9, a0 + 3, a1 - 3, step=3, z=0.06), 0.06, M["asphalt"], "Crescent road")
        for q in range(0, 60, 1 if not self.q else 4):
            a = math.radians(a0 + 6 + q * (a1 - a0 - 12) / 59.0)
            rr = (R0 + R1) / 2 + (13 if q % 2 else -13)
            b.place(P.d["palm_m"], px + rr * math.cos(a), py + rr * math.sin(a), 0.06, yaw=q * 37)
            b.tick()
        self.atlantis(px, py - (R0 + R1) / 2)
        self.lap("Palm Jumeirah")

    def atlantis(self, ax, ay):
        b, M, rng, P = self.b, self.M, self.rng, self.P
        b.poly(ellipse(ax, ay, 180, 120, n=48, z=HALO), 0.015, M["sh1"], "Atlantis shallows")
        b.poly(ellipse(ax, ay, 135, 90, n=48, z=-1.2, wob=0.03), 1.26, M["sand2"], "Atlantis island")
        spec = dict(kind="box", w=58, d=40, h=128, glass="gl_gold", band="bandw", fin="sandstone", crown="steps", fh=4.0, pitch=2.6,
                    lit=0.3, podium=0, tiers=[(0.7, 1.0, 0, 0), (0.3, 0.86, 0, 0)])
        d = b.comp("Atlantis tower", lambda: build_tower(b, M, rng, spec))
        for sx in (-1, 1):
            b.place(d, ax + sx * 74, ay, 0.06)
        # the arch between them: a profile in the x-z plane, pushed along y
        zc = 78.0
        Ro, Ri = 62.0, 44.0
        yy = ay - 12
        pts = [(ax - Ro, yy, 0.06), (ax - Ri, yy, 0.06), (ax - Ri, yy, zc)]
        pts += [(ax + Ri * math.cos(math.radians(180 - t)), yy, zc + Ri * math.sin(math.radians(180 - t))) for t in range(10, 171, 10)]
        pts += [(ax + Ri, yy, zc), (ax + Ri, yy, 0.06), (ax + Ro, yy, 0.06), (ax + Ro, yy, zc)]
        pts += [(ax + Ro * math.cos(math.radians(t)), yy, zc + Ro * math.sin(math.radians(t))) for t in range(10, 171, 10)]
        pts += [(ax - Ro, yy, zc)]
        b.poly(pts, 24, M["gl_gold"], "Atlantis arch")
        b.box((ax - 105, ay + 20, 0.06), (ax + 105, ay + 70, 16), M["cream"], "Atlantis pavilion")
        b.box((ax - 105.4, ay + 19.6, 15), (ax + 105.4, ay + 70.4, 16.4), M["bandw"], "Pavilion roof")
        for q in range(14):
            a = 2 * math.pi * q / 14
            b.place(P.d["palm_m"], ax + 125 * math.cos(a), ay + 80 * math.sin(a), 0.06, yaw=q * 41)

    # -- Museum of the Future, the Frame, a mosque, the Creek -----------------------------

    def extras(self):
        b, M, rng, P = self.b, self.M, self.rng, self.P
        # the Museum of the Future: an oval ring on a green mound, with calligraphy slits
        mx, my = 330.0, SZR_Y + 330.0
        b.box((mx - 80, my - 60, 0), (mx + 80, my + 60, 0.2), M["plaza"], "Museum plaza")
        mound = b.cyl((mx, my, 0.1), 34, 14, M["verge"], "Mound", seg=32)
        b.scale(mound, [1.15, 1.0, 1.0], (mx, my, 0))
        ring = b.torus((mx, my, 52), 25, 12, (0, 1, 0), M["steelw"], "Museum", seg=56, tseg=20)
        b.scale(ring, [1.12, 1.0, 1.42], (mx, my, 52))
        for q in range(36):
            a = 2 * math.pi * q / 36
            ex, ez = 25 * 1.12 * math.cos(a), 25 * 1.42 * math.sin(a)
            if ez + 52 < 18:
                continue
            b.box((mx + ex - 0.7, my - 13.4, 52 + ez - 3.5), (mx + ex + 0.7, my - 12.9, 52 + ez + 3.5), M["w_white"], "Calligraphy")
        # the Dubai Frame
        fx, fy = 120.0, SZR_Y + 480.0
        b.box((fx - 110, fy - 50, 0), (fx + 110, fy + 50, 0.2), M["verge"], "Zabeel Park")
        for sx in (-1, 1):
            b.box((fx + sx * 46 - 7, fy - 7, 0.2), (fx + sx * 46 + 7, fy + 7, 150), M["gold"], "Frame column")
            b.box((fx + sx * 46 - 7.3, fy - 7.3, 20), (fx + sx * 46 + 7.3, fy + 7.3, 21), M["bandd"], "Frame band")
        b.box((fx - 53, fy - 7, 143), (fx + 53, fy + 7, 150), M["gold"], "Frame bridge")
        b.box((fx - 46, fy - 7.4, 144), (fx + 46, fy + 7.4, 146), M["gl_dark"], "Frame glass")
        # a mosque with a main dome, four small ones, two minarets and a courtyard
        jx, jy = 250.0, 118.0
        b.box((jx - 50, jy - 40, 0), (jx + 50, jy + 50, 0.3), M["plaza"], "Mosque court")
        b.box((jx - 20, jy - 14, 0.3), (jx + 20, jy + 18, 13), M["sandstone"], "Prayer hall")
        mb = b.box((jx - 20.3, jy - 14.3, 6), (jx + 20.3, jy + 18.3, 6.4), M["bandw"], "Hall band")
        for sx in (-1, 1):
            b.box((jx + sx * 11 - 3, jy - 14.1, 1), (jx + sx * 11 + 3, jy - 13.9, 9), M["gl_dark"], "Hall window")
        b.sph((jx, jy + 2, 13), 11, M["dome"], "Main dome", seg=20)
        b.cyl((jx, jy + 2, 24), 0.4, 6, M["domeg"], "Finial", seg=6)
        for (qx, qy) in ((-14, -8), (14, -8), (-14, 12), (14, 12)):
            b.sph((jx + qx, jy + qy, 13), 4.8, M["dome"], "Dome", seg=12)
        for sx in (-1, 1):
            mxx = jx + sx * 26
            b.cone((mxx, jy - 12, 0.3), 2.6, 38, M["sandstone"], "Minaret", top=1.9, seg=12)
            b.cyl((mxx, jy - 12, 28), 3.0, 1.2, M["sandstone"], "Balcony", seg=12)
            b.cone((mxx, jy - 12, 38.3), 2.0, 6, M["dome"], "Minaret cap", top=0.6, seg=12)
            b.cyl((mxx, jy - 12, 44.2), 0.15, 3.5, M["domeg"], "Finial", seg=5)
        # the Creek: a winding channel with dhows moored along it, with low buildings round it
        creek = self.creek
        b.poly(band(creek, 96), 0.3, M["quay"], "Creek quay")
        b.poly(band(creek, 80), 0.42, M["canal"], "Dubai Creek")
        for k in range(26):
            i = rng.randrange(3, len(creek) - 3)
            (x0_, y0_), (x1_, y1_) = creek[i - 1], creek[i + 1]
            ang = math.atan2(y1_ - y0_, x1_ - x0_)
            off = rng.choice((-1, 1)) * rng.uniform(26, 34)
            b.place(P.d["dhow"], creek[i][0] - math.sin(ang) * off, creek[i][1] + math.cos(ang) * off, 0.42,
                    yaw=math.degrees(ang) + rng.choice((0, 180)), s=rng.uniform(0.9, 1.3))
        self.lap("Museum, Frame, mosque, Creek")

    # -- waves, surf and the night backdrop ---------------------------------------------------

    def waves(self):
        """Wave crests (thin translucent bars, denser near the shore and the islands) and rows of surf along the beaches."""
        b, M, rng, P = self.b, self.M, self.rng, self.P
        b.cat = "waves"
        n = 600 if self.q else 16000
        for k in range(n):
            x = rng.uniform(-3600, 3600)
            y = -(rng.random() ** 1.8) * 3400 - 60
            if y > coast(x) - 8:
                continue
            if math.hypot(x - PALM[0], y - PALM[1]) < 800 and math.hypot(x - PALM[0], y - PALM[1]) > 0:
                if math.hypot(x - PALM[0], y - PALM[1]) < 790:
                    continue
            b.place(P.d["crest"], x, y, SEA + 0.03, yaw=rng.gauss(8, 14), s=rng.uniform(0.7, 1.9), sy=rng.uniform(0.8, 1.6), sz=1)
            b.tick()
        x = -3600.0
        while x < 3600:
            for row, off in enumerate((-3.2, -7.5, -13.0)):
                b.place(P.d["foam"], x + row * 1.3, coast(x) + off + rng.uniform(-0.8, 0.8), SEA + 0.03, yaw=rng.gauss(0, 6),
                        s=rng.uniform(0.8, 1.5), sy=1.0 + 0.3 * row, sz=1)
            x += 3.4 if not self.q else 40
            b.tick()
        self.lap("waves and surf")

    def night_backdrop(self):
        """The night ignores the style's sky, so a dome of stars is built (small glowing cubes); by day they are hidden."""
        b, M, rng, P = self.b, self.M, self.rng, self.P
        b.cat = "stars"
        for k in range(300 if self.q else 2200):
            az = rng.uniform(0, 2 * math.pi)
            el = math.radians(rng.uniform(4, 80) ** 1.0)
            r = 6500.0
            b.place(P.d["star"], PALM[0] + r * math.cos(el) * math.cos(az), 1200 + r * math.cos(el) * math.sin(az), r * math.sin(el),
                    s=rng.uniform(9, 22))
            b.tick()
        self.lap("night backdrop")

    # -- the Gulf ------------------------------------------------------------------------

    def sea_life(self):
        b, M, rng, P = self.b, self.M, self.rng, self.P
        for k in range(10 if self.q else 70):
            x = rng.uniform(-3200, 3200)
            y = rng.uniform(-3300, -140)
            if math.hypot(x - PALM[0], y - PALM[1]) < 1000 or math.hypot(x - BAA[0], y - BAA[1]) < 260 or y > coast(x) - 90 and abs(x) < 3000 and False:
                continue
            if abs(x + 1120) < 260 and abs(y + 380) < 220:
                continue
            nm = rng.choice(P.yachts + ["dhow", "yacht_b"])
            b.place(P.d[nm], x, y, SEA + 0.3, yaw=rng.uniform(0, 360), s=rng.uniform(1.0, 1.6))
            b.tick()
        for k in range(0 if self.q else 18):
            x = rng.uniform(-700, 1600)
            y = rng.uniform(-1900, -420)
            b.place(P.d[rng.choice(P.yachts + ["dhow"])], x, y, SEA + 0.3, yaw=rng.uniform(0, 360), s=rng.uniform(1.2, 2.0))
        for k in range(8 if not self.q else 2):
            x = rng.uniform(-4500, 4500)
            y = rng.uniform(-6500, -3600)
            b.place(P.d["ship"], x, y, SEA + 0.2, yaw=rng.choice((0, 180)) + rng.uniform(-12, 12), s=rng.uniform(1.0, 1.4))
        self.lap("boats in the Gulf")

    # -- suburbs, desert -----------------------------------------------------------------

    def suburbs(self):
        b, M, rng, P = self.b, self.M, self.rng, self.P
        regions = [(-1000, 95, 640, 760), (640, 100, 1500, 300), (-1900, 380, -1000, 780), (-1900, 1340, 1700, 1620), (1560, 100, 2500, 1300)]
        mids = []
        if not self.q:
            for k in range(8):
                s = mid_spec(rng, rng.choice((24, 32, 40, 52)))
                s["fh"] = 3.4
                s["podium"] = 0
                d = b.comp("Mid-rise %d" % k, lambda s=s: build_tower(b, M, rng, s))
                mids.append((d, s))
        step_x, step_y = 31.0, 35.0
        b.cat = "suburbs"
        for (x0, y0, x1, y1) in regions:
            if self.q:
                x1 = x0 + (x1 - x0) * 0.25
                y1 = y0 + (y1 - y0) * 0.25
            b.box((x0, y0, 0), (x1, y1, 0.03), M["street"], "Suburb streets")
            gx = x0 + 16
            ci = 0
            taken = set()
            while gx < x1 - 16:
                gy = y0 + 20
                ri = 0
                while gy < y1 - 20:
                    street = (ci % 6 == 5) or (ri % 7 == 6)
                    if (ci, ri) in taken:
                        pass
                    elif not street and not self.blocked(gx, gy) and rng.random() < 0.7:
                        roll = rng.random()
                        big = ci % 6 < 4 and ri % 7 < 5 and not self.blocked(gx + 31, gy + 35)
                        if big and roll < 0.1:  # a courtyard block over a two by two block of plots
                            taken |= {(ci + 1, ri), (ci, ri + 1), (ci + 1, ri + 1)}
                            b.place(P.d[rng.choice(P.lowrise_names)], gx + step_x / 2, gy + step_y / 2, 0.03, yaw=rng.choice((0, 90, 180, 270)), s=1.1)
                        elif big and roll < 0.125:  # a mosque
                            taken |= {(ci + 1, ri), (ci, ri + 1), (ci + 1, ri + 1)}
                            b.place(P.d["mosque"], gx + step_x / 2, gy + step_y / 2, 0.03, yaw=rng.choice((0, 90, 180, 270)), s=0.95)
                        elif mids and roll < 0.17:
                            d, s = rng.choice(mids)
                            b.place(d, gx, gy, 0.0, yaw=rng.choice((0, 90, 180, 270)), s=1.0)
                        else:
                            b.place(P.d[P.villa_names[rng.randrange(len(P.villa_names))]], gx, gy, 0.03,
                                    yaw=rng.choice((0, 180)) if (ri % 7) < 4 else rng.choice((0, 180)), s=rng.uniform(0.95, 1.05))
                        if rng.random() < 0.18:
                            b.place(P.d["acacia"], gx + rng.uniform(-12, 12), gy + 18, 0.03, s=rng.uniform(0.8, 1.2))
                    elif street and ci % 6 != 5 and rng.random() < 0.3:
                        b.place(P.d["palm_s"], gx, gy, 0.03, yaw=rng.uniform(0, 360))
                    gy += step_y
                    ri += 1
                    b.tick()
                gx += step_x
                ci += 1
        self.lap("suburbs")

    def desert(self):
        b, M, rng, P = self.b, self.M, self.rng, self.P
        n = 60 if self.q else 420
        for k in range(n):
            x = rng.uniform(-4800, 4800)
            y = rng.uniform(2400, 6000)
            nm = rng.choice(("dune", "dune2", "dune3"))
            sx = rng.uniform(180, 560)
            b.place(P.d[nm], x, y, -2.0, yaw=rng.uniform(0, 180), s=sx, sy=sx * rng.uniform(0.35, 0.7), sz=rng.uniform(4, 13))
            b.tick()
        self.lap("desert")

    # -- lights on the sea side ----------------------------------------------------------

    def coast_road(self):
        b, M, rng, P = self.b, self.M, self.rng, self.P
        yc = 72.0
        hw = self.hroad(-1000, 1500, yc, 3, median=4.0, walk=7.0, name="Jumeirah Road")
        for x in range(-980, 1480, 40):
            b.place(P.d["lamp"], x, yc - hw + 3, 0.22, yaw=0)
            b.place(P.d["lamp"], x + 20, yc + hw - 3, 0.22, yaw=180)
            if x % 80 == 0:
                b.place(P.d["palm_l"], x + 8, yc - hw - 3, 0.22, yaw=rng.uniform(0, 360))
            b.tick()
        b.cat = "street"
        for k in range(0 if self.q else 160):
            x = rng.uniform(-1000, 1500)
            lane = rng.randrange(3)
            sgn = rng.choice((-1, 1))
            b.place(P.d[P.car_names[rng.randrange(len(P.car_names))]], x, yc + sgn * (2 + 3.7 * lane + 1.85), 0.1, yaw=0 if sgn < 0 else 180)
            b.tick()
        self.lap("Jumeirah Road")

    # -- everything -----------------------------------------------------------------------

    def build(self, only=None):
        kx, ky = KHALIFA
        self.zones = [
            (kx - 570, ky - 480, kx + 720, ky + 260),  # Downtown
            (20, SZR_Y + 40, 440, SZR_Y + 600),  # Emirates Towers, the Museum
            (CAYAN[0] - 80, CAYAN[1] - 70, CAYAN[0] + 80, CAYAN[1] + 70),
            (-640, SZR_Y - 230, -280, SZR_Y + 230),  # the interchange
            (-180, SZR_Y + 400, 340, SZR_Y + 620),  # the Frame and Zabeel Park
            (190, 55, 320, 190),  # the mosque
                    ]
        P = self.P
        run = lambda name: only is None or name in only  # noqa: E731
        self.lap("materials")
        P.make()
        self.lap("components")
        self.ground()
        self.szr()
        self.metro()
        self.interchange()
        if run("coast"):
            self.coast_road()
        self.emirates()
        self.cayan()
        self.canyon()
        self.marina()
        self.downtown()
        self.burj_al_arab()
        self.ain_dubai()
        self.palm()
        self.sea_life()
        self.waves()
        self.night_backdrop()
        self.extras()
        self.suburbs()
        self.desert()
        self.b.flush()


# ---------------------------------------------------------------- views

HERO = {"eye": [-260.0, -2250.0, 720.0], "target": [-260.0, 600.0, 30.0], "fov_y_deg": 42}
SHOTS = {
    "dubai.png": HERO,
    "dubai-night.png": HERO,
    "dubai-khalifa.png": {"eye": [KHALIFA[0], KHALIFA[1] - 440, 4.0], "target": [KHALIFA[0], KHALIFA[1], 330.0], "fov_y_deg": 80},
    "dubai-palm.png": {"eye": [-380.0, -2750.0, 1350.0], "target": [-760.0, -1000.0, 0.0], "fov_y_deg": 40},
    "dubai-szr.png": {"eye": [-980.0, SZR_Y - 14, 42.0], "target": [700.0, SZR_Y + 30, 150.0], "fov_y_deg": 44},
}
# the flyover: eye and target at each waypoint
FLY = [
    ([-1500, -2600, 520], [-900, -900, 40]),
    ([-700, -2300, 420], [-300, -600, 120]),
    ([-60, -1500, 330], [100, 200, 160]),
    ([380, -700, 260], [650, 650, 260]),
    ([620, -120, 150], [720, 640, 330]),
    ([1100, 300, 360], [720, 640, 280]),
]


def day_look(c, m, M, sid=None):
    set_night(c, M, False)
    # the axes lines are drawn with the ground (needed for the sun's shadows on it): park them far away
    c.execute({"type": "set_axes", "origin": [40000, -40000, 0], "x_axis": [1, 0, 0], "y_axis": [0, 1, 0]})
    c.execute({"type": "set_location", "latitude": 25.2, "longitude": 55.27, "north_angle": 8, "utc_offset": 4})
    c.execute({"type": "set_shadows", "on": True, "date": "2026-11-12", "time": "16:30", "on_ground": True, "light": 76, "dark": 44})
    if sid:
        c.execute({"type": "set_style", "style": sid})
        return sid
    return m.style("Golden hour", sky=[12, 78, 222], background=[246, 218, 180], edges={"show_edges": False},
                   ambient_occlusion={"on": True, "radius": 4.0, "strength": 1.0},
                   fog={"start": 1800, "end": 11000, "color": "background"})


def night_look(c, m, M):
    set_night(c, M, True)
    return m.style("Night", sky=[10, 20, 56], background=[18, 30, 66], edges={"show_edges": False},
                   ambient_occlusion={"on": True, "radius": 4.0, "strength": 1.0},
                   fog={"start": 3000, "end": 16000, "color": {"rgb": [52, 44, 82]}})


def fly_camera(t):
    """The camera at t in 0..1 along the waypoints (smooth)."""
    n = len(FLY) - 1
    u = min(max(t, 0.0), 1.0) * n
    i = min(int(u), n - 1)
    f = u - i
    f = f * f * (3 - 2 * f) * 0.5 + f * 0.5
    a, b = FLY[i], FLY[i + 1]
    lerp = lambda p, q: [p[k] + (q[k] - p[k]) * f for k in range(3)]  # noqa: E731
    return {"eye": lerp(a[0], b[0]), "target": lerp(a[1], b[1]), "fov_y_deg": 40}


def gif(out, paths, width, fps=12, name="dubai-flyover.gif", colors=128):
    ffmpeg = shutil.which("ffmpeg")
    if ffmpeg is None or not paths:
        return None
    fdir = os.path.dirname(paths[0])
    path = os.path.join(out, name)
    vf = ("scale=%d:-1:flags=lanczos,split[a][b];[a]palettegen=max_colors=%d:stats_mode=diff[p];"
          "[b][p]paletteuse=dither=bayer:bayer_scale=5:diff_mode=rectangle" % (width, colors))
    cmd = [ffmpeg, "-y", "-loglevel", "error", "-framerate", str(fps), "-i", os.path.join(fdir, "fly_%03d.png"), "-vf", vf,
           "-loop", "0", path]
    r = subprocess.run(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, timeout=240)
    if r.returncode != 0:
        print("  ffmpeg failed: %s" % r.stderr.decode(errors="replace").strip()[:300])
        return None
    return path


def set_hidden(c, ids, hide):
    """Hide or show instances in chunks (a flyover renders without the finest layers: every render pays for
    every solid, so the frames are much cheaper without the wave crests, suburbs and traffic)."""
    for k in range(0, len(ids), 4000):
        c.execute({"type": "hide" if hide else "unhide", "ids": ids[k:k + 4000]})


def render_all(c, m, M, out, width, height, ss, quick, only=None, frames=0, groups=None):
    groups = groups or {}
    pic = lambda name: os.path.join(out, name)  # noqa: E731
    want = lambda name: only is None or name in only  # noqa: E731
    clear = lambda: c.execute({"type": "select", "ids": [], "mode": "replace"})  # noqa: E731
    clear()
    stars = groups.get("stars", [])
    set_hidden(c, stars, True)
    day = day_look(c, m, M)
    for name in ("dubai.png", "dubai-khalifa.png", "dubai-palm.png", "dubai-szr.png"):
        if want(name):
            t = time.time()
            m.render(pic(name), width, height, camera=SHOTS[name], lighting="day", supersample=ss)
            print("  %-26s %5.1f s" % (name, time.time() - t))
    if want("dubai-night.png"):
        t = time.time()
        set_hidden(c, stars, False)
        night_look(c, m, M)
        m.render(pic("dubai-night.png"), width, height, camera=SHOTS["dubai-night.png"], lighting="night", supersample=ss)
        print("  %-26s %5.1f s" % ("dubai-night.png", time.time() - t))
        set_hidden(c, stars, True)
        day_look(c, m, M, day)
    if frames and want("dubai-flyover.gif"):
        t = time.time()
        fine = [i for k in ("waves", "suburbs", "street") for i in groups.get(k, [])]
        set_hidden(c, fine, True)
        fdir = os.path.join(out, "frames")
        os.makedirs(fdir, exist_ok=True)
        paths = []
        for k in range(frames):
            p = os.path.join(fdir, "fly_%03d.png" % k)
            m.render(p, 480, 300, camera=fly_camera(k / (frames - 1.0)), lighting="day", supersample=1)
            paths.append(p)
        set_hidden(c, fine, False)
        gif(out, paths, 480)
        for p in paths:
            os.remove(p)
        try:
            os.rmdir(fdir)
        except OSError:
            pass
        print("  %-26s %5.1f s  (%d frames)" % ("dubai-flyover.gif", time.time() - t, frames))
    return day


def main(argv=None):
    from tenon import Model

    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--socket", help="socket of a running app (default: start a headless one)")
    ap.add_argument("--app", help="app binary to start (default: TENON_APP or the repo's target/)")
    ap.add_argument("--out", default=".", help="directory for the images (default: here)")
    ap.add_argument("--quick", action="store_true", help="a much smaller city and small images (for tests)")
    ap.add_argument("--frames", type=int, help="frames of the flyover (default 12; 0 leaves it out; quick: none)")
    ap.add_argument("--only", help="comma-separated names of the pictures to render")
    ap.add_argument("--skip-gif", action="store_true", help="leave the flyover out")
    args = ap.parse_args(argv)
    width, height, ss = (320, 200, 1) if args.quick else (1600, 1000, 2)
    frames = 0 if (args.quick or args.skip_gif) else (12 if args.frames is None else args.frames)
    out = os.path.abspath(args.out)
    os.makedirs(out, exist_ok=True)
    only = set(args.only.split(",")) if args.only else None
    t_all = time.time()
    with session(args.socket, args.app) as s:
        c = s.client
        m = Model(c)
        c.execute({"type": "new", "discard": s.launched})
        city = Dubai(c, args.quick)
        city.build()
        print("built in %.1f s: %d commands, %d instances placed, %d solids once nested ones are counted, about %.2f M triangles"
              % (time.time() - t_all, city.b.total, city.b.instances, city.b.leaves, city.b.tris / 1e6))
        print("model:", c.query("stats"))
        day = render_all(c, m, city.M, out, width, height, ss, args.quick, only, frames, city.b.groups)
        c.execute({"type": "set_style", "style": day})
        c.execute({"type": "set_lighting", "mode": "day"})
        c.execute({"type": "set_shadows", "on": True})
        c.execute({"type": "set_camera", "projection": "perspective", **HERO})
        c.execute({"type": "select", "ids": [], "mode": "replace"})
        c.execute({"type": "save", "path": os.path.join(out, "dubai.tnn"), "overwrite": True})
        print("total %.1f s" % (time.time() - t_all))


if __name__ == "__main__":
    main()
