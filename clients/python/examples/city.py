#!/usr/bin/env python3
"""A small city grid seen from the air: roads with lane markings, sidewalks, towers downtown,
mid-rise blocks, houses with gable roofs, a park with a pond, and trees.

    python3 city.py [--socket PATH] [--out DIR] [--quick]

Without ``--socket`` it starts a headless app. It writes ``city.png`` to ``--out``.

Most of the model goes up in a few large batches: each block is one batch of ``add_box``,
``add_prism`` and ``add_cylinder`` commands, lane markings are ``copy`` arrays of one dash, and
the trees are a component placed with ``place_instance``.
"""
import argparse
import math
import os
import random
import sys
import time

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))

from sketchup_alt import Model, Transform, session  # noqa: E402

BLOCK = 40.0  # block side, meters
ROAD = 12.0  # road width
PITCH = BLOCK + ROAD

COLORS = {
    "grass": (96, 150, 78), "asphalt": (58, 60, 64), "sidewalk": (186, 184, 176), "marking": (240, 236, 220),
    "glass": (110, 150, 190), "glass2": (74, 98, 130), "concrete": (178, 176, 170), "brick": (168, 86, 64),
    "beige": (222, 206, 170), "white": (238, 236, 230), "roof": (120, 64, 52), "roof2": (80, 84, 92),
    "trunk": (98, 70, 48), "leaves": (58, 120, 56), "leaves2": (84, 140, 60), "water": (64, 120, 180),
    "plaza": (206, 200, 186),
}
FINISH = {"glass": (0.6, 0.15), "glass2": (0.6, 0.2), "water": (0.2, 0.1)}


class City:
    def __init__(self, m, n, seed=7):
        self.m = m
        self.n = n
        self.rng = random.Random(seed)
        self.mat = {}
        self.size = n * PITCH + ROAD  # roads all round
        self.origin = -self.size / 2

    def materials(self):
        for name, color in COLORS.items():
            metal, rough = FINISH.get(name, (0, 1))
            self.mat[name] = self.m.material(name.capitalize(), color, metalness=metal, roughness=rough).id

    def block_origin(self, i, j):
        return (self.origin + ROAD + i * PITCH, self.origin + ROAD + j * PITCH)

    def ground(self):
        m = self.m
        s = self.size
        m.box((-6 * s, -6 * s, -0.6), (12 * s, 12 * s, 0.5), material=self.mat["grass"], name="Ground")
        m.box((self.origin, self.origin, -0.1), (s, s, 0.12), material=self.mat["asphalt"], name="Roads")
        # dashed centre lines: one dash per road, copied along it
        with m.batch() as b:
            for k in range(self.n + 1):
                c = self.origin + ROAD / 2 + k * PITCH
                count = int(self.size / 6) - 1
                d = b.add_box(corner=[self.origin + 1, c - 0.15, 0.02], size=[3, 0.3, 0.01], material=self.mat["marking"])
                b.copy(ids=[d.i(0)], offset=[6, 0, 0], count=count)
                d = b.add_box(corner=[c - 0.15, self.origin + 1, 0.02], size=[0.3, 3, 0.01], material=self.mat["marking"])
                b.copy(ids=[d.i(0)], offset=[0, 6, 0], count=count)

    def tree(self):
        m = self.m
        trunk = m.cylinder((0, 0, 0), 0.25, 3, segments=8, material=self.mat["trunk"])
        crown = m.sphere((0, 0, 4.2), 2.0, segments=12, material=self.mat["leaves"])
        top = m.cone((0, 0, 4.8), 1.6, 2.6, segments=12, material=self.mat["leaves2"])
        t = m.component([trunk, crown, top], "Tree")
        self.tree_def = t.definition
        t.hide()

    def plant(self, b, x, y, scale):
        b.place_instance(definition=self.tree_def,
                         transform=(Transform.translate(x, y, 0.15) @ Transform.scale(scale)).to_json())

    def block(self, i, j, kind):
        x0, y0 = self.block_origin(i, j)
        rng, mat = self.rng, self.mat
        with self.m.batch() as b:
            b.add_box(corner=[x0, y0, 0], size=[BLOCK, BLOCK, 0.15], material=mat["grass" if kind == "park" else "sidewalk"])
            if kind == "park":
                b.add_cylinder(base=[x0 + 24, y0 + 22, 0.15], radius=8, height=0.05, segments=48, material=mat["water"])
                b.add_box(corner=[x0 + 2, y0 + 19, 0.15], size=[36, 2.5, 0.04], material=mat["plaza"])
                for _ in range(28):
                    x, y = x0 + rng.uniform(3, 37), y0 + rng.uniform(3, 37)
                    if math.hypot(x - x0 - 24, y - y0 - 22) > 9.5 and abs(y - y0 - 20.2) > 2.5:
                        self.plant(b, x, y, rng.uniform(0.8, 1.3))
                return
            # trees along the kerb
            for k in range(5):
                t = 4 + k * 8
                self.plant(b, x0 + t, y0 + 1.5, 0.7)
                self.plant(b, x0 + 1.5, y0 + t, 0.7)
            lots = 2 if kind == "tower" else (3 if kind == "midrise" else 4)
            lot = (BLOCK - 6) / lots
            for a in range(lots):
                for c in range(lots):
                    lx, ly = x0 + 4 + a * lot, y0 + 4 + c * lot
                    w = lot - 2.5
                    if kind == "tower":
                        self.tower(b, lx, ly, w)
                    elif kind == "midrise":
                        h = rng.uniform(12, 36)
                        m = rng.choice(["brick", "beige", "concrete", "white"])
                        b.add_box(corner=[lx, ly, 0.15], size=[w, w, h], material=mat[m])
                        b.add_box(corner=[lx + w * 0.3, ly + w * 0.3, 0.15 + h], size=[w * 0.3, w * 0.25, 2.5], material=mat["concrete"])
                    else:
                        self.house(b, lx, ly, w)

    def tower(self, b, x, y, w):
        rng, mat = self.rng, self.mat
        glass = mat[rng.choice(["glass", "glass2"])]
        if rng.random() < 0.3:  # a round glass tower
            h = rng.uniform(60, 110)
            b.add_cylinder(base=[x + w / 2, y + w / 2, 0.15], radius=w / 2, height=h, segments=32, material=glass)
            b.add_cylinder(base=[x + w / 2, y + w / 2, 0.15 + h], radius=w / 2.6, height=4, segments=32, material=mat["concrete"])
            return
        h = 0.15
        side = w
        for _ in range(rng.randint(2, 4)):  # setbacks
            tier = rng.uniform(18, 40)
            off = (w - side) / 2
            b.add_box(corner=[x + off, y + off, h], size=[side, side, tier], material=glass)
            b.add_box(corner=[x + off - 0.3, y + off - 0.3, h + tier - 0.6], size=[side + 0.6, side + 0.6, 0.6], material=mat["concrete"])
            h += tier
            side *= rng.uniform(0.68, 0.85)
        b.add_cylinder(base=[x + w / 2, y + w / 2, h], radius=0.3, height=rng.uniform(6, 14), segments=8, material=mat["concrete"])

    def house(self, b, x, y, w):
        rng, mat = self.rng, self.mat
        h = rng.uniform(3, 7)
        d = w * rng.uniform(0.6, 0.85)
        b.add_box(corner=[x, y, 0.15], size=[w, d, h], material=mat[rng.choice(["white", "beige", "brick"])])
        # a gable roof: a three-sided prism lying along x, one corner up
        r = d / math.sqrt(3) + 0.3
        b.add_prism(base=[x - 0.3, y + d / 2, 0.15 + h + r / 2 - 0.05], radius=r, height=w + 0.6, sides=3,
                    axis=[1, 0, 0], direction=[0, 0, 1], material=mat[rng.choice(["roof", "roof2"])])

    def build(self, log=print):
        t0 = time.time()
        self.materials()
        self.ground()
        self.tree()
        mid = (self.n - 1) / 2
        park = (self.n - 1, 0) if self.n > 2 else None
        for i in range(self.n):
            for j in range(self.n):
                dist = max(abs(i - mid), abs(j - mid))
                if (i, j) == park:
                    kind = "park"
                elif dist < 1:
                    kind = "tower"
                elif dist < 2:
                    kind = "midrise"
                else:
                    kind = "houses"
                self.block(i, j, kind)
        log("built in %.1f s" % (time.time() - t0))


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--socket", help="socket of a running app (default: start a headless one)")
    ap.add_argument("--app", help="app binary to start (default: SKETCHUP_ALT_APP or the repo's target/)")
    ap.add_argument("--out", default=".", help="directory for city.png (default: here)")
    ap.add_argument("--quick", action="store_true", help="a 2 x 2 grid and a small image (for tests)")
    ap.add_argument("--size", type=int, help="blocks along each side (default 5, quick 2)")
    ap.add_argument("--width", type=int, help="image width (default 1600, quick 320)")
    ap.add_argument("--height", type=int, help="image height (default 1000, quick 200)")
    args = ap.parse_args(argv)
    n = args.size or (2 if args.quick else 5)
    width = args.width or (320 if args.quick else 1600)
    height = args.height or (200 if args.quick else 1000)
    os.makedirs(args.out, exist_ok=True)
    with session(args.socket, args.app) as s:
        m = Model(s.client)
        s.client.execute({"type": "new", "discard": s.launched})
        city = City(m, n)
        city.build()
        m.style("Aerial", sky=[150, 190, 235], background=[222, 232, 242],
                fog={"start": city.size * 1.2, "end": city.size * 4, "color": "background"},
                ambient_occlusion={"on": True, "radius": 3, "strength": 1})
        m.client.execute({"type": "set_shadows", "on": True, "date": "2026-06-21", "time": "16:30", "on_ground": True})
        d = city.size
        t = time.time()
        out = os.path.join(args.out, "city.png")
        m.render(out, width, height, lighting="day", supersample=1 if args.quick else 2,
                 camera={"eye": [-0.75 * d, -1.0 * d, 0.85 * d], "target": [0, 0, 0], "fov_y_deg": 35})
        print("rendered in %.1f s: %s" % (time.time() - t, out))


if __name__ == "__main__":
    main()
