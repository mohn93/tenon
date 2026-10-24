#!/usr/bin/env python3
"""A half-open cutaway of a 60-degree V6, drawn like a technical cutaway illustration.

    python3 v6_engine.py [--socket PATH] [--out DIR] [--quick]

Without ``--socket`` it starts a headless app (``SKETCHUP_ALT_APP`` or the repo's
``target/{debug,release}/app``). It writes ``v6.png``, ``v6-front.png`` and
``v6-motion.png`` (a frame halfway between the two crank scenes) to ``--out``.

What it shows of the API:

- primitives (boxes, cylinders, tubes, cones, prisms, tori) and solid booleans: the block's
  bores, crank cavity and water jacket, the heads' ports, valve throats, guides and bolt holes,
  pistons with ring grooves and valve reliefs, I-beam rods, crank webs with counterweights;
- fillets and chamfers on the block, the main caps, the rods and the valve covers;
- ``add_helix`` + ``follow_me`` valve springs, ``follow_me`` intake runners, exhausts and belt;
- radial arrays (``rotate`` with ``copies``) for sprocket and ring gear teeth and bolt circles;
- ``copy_along_path`` timing chains; components placed with ``place_instance``;
- metal materials, a section plane at the root with ``set_section_cut`` false on the moving
  parts, per-group section planes, ``fill_color`` material, studio lighting, ambient
  occlusion, labels, scenes storing crank positions and ``render_view`` frames between them.

Lengths are meters. The crank axis is the X axis; bank A leans toward -Y, bank B toward +Y.
The parts of one bank are drawn once in a "bank frame" (x along the crank, y toward the
valley, s up the bores) and placed in both banks, bank B mirrored.
"""
import argparse
import math
import os
import sys
import time

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))

from sketchup_alt import Model, Transform, session  # noqa: E402

# ---------------------------------------------------------------- dimensions (meters)

R = 0.043  # crank radius (86 mm stroke)
ROD = 0.150  # rod length, center to center
BORE = 0.0465  # bore radius (93 mm)
PITCH = 0.106  # bore spacing
STAGGER = 0.030  # bank B sits this far behind bank A along the crank
DECK = 0.226  # deck height along the bore axis
GASKET = 0.0015
HEAD_TOP = DECK + GASKET + 0.063
VALVE_TILT = math.radians(15)
SEAT_S = DECK + GASKET + 0.0055  # valve faces, at the combustion chamber roof
CAM_AXIAL = 0.120  # cam axis distance from the valve seat along the valve
HALF_V = math.radians(30)

# ---------------------------------------------------------------- small vector helpers


def add(a, b):
    return tuple(x + y for x, y in zip(a, b))


def sub(a, b):
    return tuple(x - y for x, y in zip(a, b))


def mul(a, k):
    return tuple(x * k for x in a)


def dot(a, b):
    return sum(x * y for x, y in zip(a, b))


def length(a):
    return math.sqrt(dot(a, a))


def unit(a):
    return mul(a, 1 / length(a))


def hull2(points):
    """Convex hull of 2D points, counter-clockwise (monotone chain)."""
    pts = sorted(set(points))

    def cross(o, a, b):
        return (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0])

    lower, upper = [], []
    for p in pts:
        while len(lower) >= 2 and cross(lower[-2], lower[-1], p) <= 0:
            lower.pop()
        lower.append(p)
    for p in reversed(pts):
        while len(upper) >= 2 and cross(upper[-2], upper[-1], p) <= 0:
            upper.pop()
        upper.append(p)
    return lower[:-1] + upper[:-1]


def circle_hull(circles, samples=72):
    """The loop around 2D circles ``(cy, cz, r)``: a belt or a chain path."""
    pts = []
    for cy, cz, r in circles:
        for k in range(samples):
            a = 2 * math.pi * k / samples
            pts.append((round(cy + r * math.cos(a), 9), round(cz + r * math.sin(a), 9)))
    return hull2(pts)


def resample(loop, step):
    """Points every ``step`` along a closed 2D loop (drops near-duplicate vertices)."""
    out = [loop[0]]
    acc = 0.0
    n = len(loop)
    for i in range(1, n + 1):
        p = loop[i % n]
        d = math.dist(out[-1], p)
        acc += d
        if acc >= step or i == n:
            if i != n:
                out.append(p)
            acc = 0.0
    return out


# ---------------------------------------------------------------- frames

D_A = (0.0, -math.sin(HALF_V), math.cos(HALF_V))  # bank A bore axis
W_A = (0.0, math.cos(HALF_V), math.sin(HALF_V))  # toward the valley
D_B = (0.0, math.sin(HALF_V), math.cos(HALF_V))
W_B = (0.0, -math.cos(HALF_V), math.sin(HALF_V))
X = (1.0, 0.0, 0.0)

# The engine is built around the origin, then lifted to LIFT: below the ground plane, so that a
# camera looking down on it never sees the ground grid or the axes.
LIFT = (2.0, 2.0, -1.5)

# bank frame -> model: x, y (valley), s (bore axis). Bank B is the mirror image, moved back.
FRAME = {
    "A": Transform(X + W_A + D_A + (0.0, 0.0, 0.0)),
    "B": Transform(X + W_B + D_B + (STAGGER, 0.0, 0.0)),
}
AXIS = {"A": D_A, "B": D_B}
VALLEY = {"A": W_A, "B": W_B}


def valve_axis(side):
    """Unit valve axis in the bank frame; side +1 intake (valley), -1 exhaust."""
    return (0.0, side * math.sin(VALVE_TILT), math.cos(VALVE_TILT))


def valve_seat(xc, dx, side):
    return (xc + dx, side * 0.018, SEAT_S)


def cam_axis_point(side, x=0.0):
    return add(valve_seat(x, 0.0, side), mul(valve_axis(side), CAM_AXIAL))


# ---------------------------------------------------------------- the build


class V6:
    def __init__(self, m, quick=False):
        self.m = m
        self.quick = quick
        self.n = 1 if quick else 3  # cylinders per bank
        self.xc = [PITCH * i for i in range(self.n)]
        self.seg = 16 if quick else 32  # circle segments for the larger round parts
        self.moving = []  # root instances that move with the crank (scene positions)
        self.whole = []  # instances the root section plane must not cut
        self.labels = {}
        self.offset = (0.0, 0.0, 0.0)
        self.mains = [self.xc[0] - 0.038] + [x + 0.068 for x in self.xc]
        self.x_front = self.xc[0] - 0.075
        self.x_rear = self.xc[-1] + 0.103
        self.pin_angle = {}
        for i in range(self.n):
            self.pin_angle[("A", i)] = 120.0 * i
            self.pin_angle[("B", i)] = 120.0 * i - 60.0

    # helpers --------------------------------------------------------

    def kbox(self, frame, corner, size, **kw):
        """A box given in the bank frame, placed by ``frame`` (None: model coordinates)."""
        b = self.m.box(corner, size, **kw)
        if frame is not None:
            b.apply(frame)
        return b

    def kcyl(self, frame, base, axis, radius, height, **kw):
        if frame is not None:
            base, axis = frame.apply(base), frame.apply_vector(axis)
        return self.m.cylinder(base, radius, height, axis=axis, **kw)

    def between(self, a, b, radius, **kw):
        return self.m.cylinder(a, radius, length(sub(b, a)), axis=sub(b, a), **kw)

    def cyl_x(self, x0, x1, radius, y=0.0, z=0.0, **kw):
        """A cylinder along X from x0 to x1."""
        return self.m.cylinder((x0, y, z), radius, x1 - x0, axis=X, **kw)

    def edges_along(self, inst, axis, where=None):
        """Edges of ``inst`` running along ``axis`` (0 x, 1 y, 2 z), optionally filtered."""
        def ok(a, b):
            d = [abs(a[i] - b[i]) for i in range(3)]
            straight = all(d[i] < 1e-7 for i in range(3) if i != axis) and d[axis] > 1e-6
            return straight and (where is None or where(a, b))

        return inst.edges(ok)

    # materials --------------------------------------------------------

    def materials(self):
        m = self.m
        self.iron = m.material("Cast iron", (86, 88, 92), metalness=0.5, roughness=0.75)
        self.alu = m.material("Cast aluminium", (184, 189, 196), metalness=0.85, roughness=0.42)
        self.forged = m.material("Forged steel", (146, 150, 158), metalness=0.9, roughness=0.32)
        self.polished = m.material("Polished steel", (204, 207, 214), metalness=1.0, roughness=0.18)
        self.copper = m.material("Copper", (200, 112, 62), metalness=1.0, roughness=0.3)
        self.paint = m.material("Valve cover red", (176, 22, 30), metalness=0.25, roughness=0.3)
        self.black = m.material("Black crinkle", (36, 36, 40), metalness=0.1, roughness=0.65)
        self.spring_mat = m.material("Spring steel", (54, 104, 190), metalness=0.7, roughness=0.3)
        self.chain_mat = m.material("Chain steel", (74, 76, 84), metalness=0.95, roughness=0.38)
        self.exhaust_mat = m.material("Exhaust", (136, 98, 76), metalness=0.75, roughness=0.55)
        self.zinc = m.material("Zinc bolts", (186, 180, 150), metalness=0.85, roughness=0.35)
        self.ceramic = m.material("Ceramic", (240, 238, 228), roughness=0.35)
        self.rubber = m.material("Rubber", (30, 30, 33), roughness=0.9)
        self.filter_mat = m.material("Filter blue", (30, 70, 160), metalness=0.3, roughness=0.4)
        self.oil = self.m.client.execute(
            {"type": "add_material", "name": "Oil", "color": [196, 140, 30], "opacity": 0.55, "existing": "reuse"}
        )["changes"]["materials"][0]

    # block ------------------------------------------------------------

    def block(self):
        m, n = self.m, self.n
        x0, x1 = self.x_front, self.x_rear
        case = m.box((x0, -0.145, -0.09), (x1 - x0, 0.29, 0.21), name="Block")
        case.fillet(self.edges_along(case, 2), 0.012)  # round the four upright corners
        banks = []
        for b in "AB":
            bx0 = self.xc[0] - 0.068
            bx1 = self.xc[-1] + 0.068
            bank = m.box((bx0, -0.068, 0.06), (bx1 - bx0, 0.136, DECK - 0.06))
            bank.fillet(self.edges_along(bank, 2), 0.01)
            bank.apply(FRAME[b])
            banks.append(bank)
        blk = m.union([case] + banks)

        tools = []
        for b in "AB":
            f, d = FRAME[b], AXIS[b]
            for xc in self.xc:  # bores, open into the crankcase and the deck
                tools.append(m.cylinder(f.apply((xc, 0, 0.05)), BORE, DECK + 0.01 - 0.05, axis=d, segments=self.seg))
            # water jacket: slots down both sides, at the ends and between the bores
            xa, xb = self.xc[0] - 0.056, self.xc[-1] + 0.056
            for y in (0.054, -0.060):
                tools.append(self.kbox(f, (xa, y, 0.12), (xb - xa, 0.006, 0.095)))
            for xs in [self.xc[0] - 0.058, self.xc[-1] + 0.052] + [x + 0.0505 for x in self.xc[:-1]]:
                tools.append(self.kbox(f, (xs, -0.035, 0.125), (0.005, 0.07, 0.085)))
            # head bolt holes, blind
            for xs in [self.xc[0] - 0.053] + [x + 0.053 for x in self.xc]:
                for y in (-0.045, 0.045):
                    tools.append(self.kcyl(f, (xs, y, DECK - 0.05), (0, 0, 1), 0.0055, 0.051, segments=12))
        # crank cavity between the bulkheads, the open bottom and the main bearing bore
        for a, b in zip(self.mains, self.mains[1:]):
            tools.append(self.cyl_x(a + 0.0135, b - 0.0135, 0.1, segments=self.seg))
        tools.append(m.box((x0 + 0.01, -0.135, -0.1), (x1 - x0 - 0.02, 0.27, 0.099)))
        tools.append(self.cyl_x(x0 - 0.01, x1 + 0.01, 0.032, segments=self.seg))
        self.block_inst = m.subtract(blk, tools)
        self.block_inst.paint(self.iron).name("Block")

        # main bearing caps: chamfered, bored, bolted
        cap = m.box((-0.0115, -0.05, -0.052), (0.023, 0.1, 0.0508))
        cap.chamfer(self.edges_along(cap, 0, lambda a, b: a[2] < -0.05), 0.006)
        cap = m.subtract(cap, [self.cyl_x(-0.02, 0.02, 0.032, segments=self.seg)]
                         + [m.cylinder((0, y, -0.06), 0.0045, 0.07, segments=10) for y in (-0.04, 0.04)])
        cap.paint(self.iron)
        cap = m.component([cap], "Main cap")
        bolt = self.bolt(0.0045, 0.075, "Cap bolt")
        shell = m.tube((-0.01, 0, 0), 0.032, 0.0302, 0.02, axis=X, segments=self.seg, material=self.copper)
        shell = m.component([shell], "Main bearing")
        for k, xm in enumerate(self.mains):
            c = cap if k == 0 else m.place(cap, Transform.identity())
            c.set_transform(Transform.translate(xm, 0, 0))
            s = shell if k == 0 else m.place(shell, Transform.identity())
            s.set_transform(Transform.translate(xm, 0, 0))
            self.whole.append(s)
            for y in (-0.04, 0.04):
                m.place(bolt, Transform.translate(xm, y, -0.0585))
        bolt.hide()  # the original sits at the origin

        # oil pan with a sump, oil and a pickup
        pan = m.box((x0, -0.145, -0.205), (x1 - x0, 0.29, 0.115), name="Oil pan")
        pan.fillet(self.edges_along(pan, 2), 0.02)
        pan.fillet(pan.edges(lambda a, b: a[2] < -0.2 and b[2] < -0.2), 0.015)
        pan = m.subtract(pan, [m.box((x0 + 0.004, -0.141, -0.201), (x1 - x0 - 0.008, 0.282, 0.12))])
        pan.paint(self.black).name("Oil pan")
        m.box((x0 + 0.005, -0.14, -0.2), (x1 - x0 - 0.01, 0.28, 0.035), material=self.oil, name="Oil")
        xp = (self.xc[0] + self.xc[-1]) / 2
        pick = m.sweep([(xp, 0.0, -0.035), (xp, 0.0, -0.12), (xp, 0.04, -0.16), (xp, 0.06, -0.175)],
                       0.008, inner_radius=0.0065, name="Oil pickup", material=self.forged)
        strainer = m.cylinder((xp, 0.06, -0.19), 0.028, 0.012, segments=self.seg, material=self.forged)
        self.whole += [pick, strainer]

        # oil filter on the +Y side
        flt = m.cylinder((xp, 0.145, -0.02), 0.038, 0.09, axis=(0, 1, 0), segments=self.seg, material=self.filter_mat)
        self.whole.append(flt)

    def bolt(self, radius, length_, name):
        """A bolt along +Z from its head at the origin: shank, washer and hex head."""
        m = self.m
        shank = m.cylinder((0, 0, 0), radius, length_, segments=10)
        washer = m.cylinder((0, 0, -0.0015), radius * 2.1, 0.0015, segments=16)
        head = m.prism((0, 0, -0.0015 - radius * 1.3), radius * 1.75, radius * 1.3, sides=6)
        b = m.group([shank, washer, head])
        b.paint(self.zinc)
        return m.component([b], name)

    # rotating assembly ------------------------------------------------

    def crankshaft(self):
        m = self.m
        parts = []
        for xm in self.mains:  # main journals
            parts.append(self.cyl_x(xm - 0.015, xm + 0.015, 0.03, segments=self.seg))
        # one web with its counterweight, placed at every throw
        plate = m.box((0, -0.024, 0), (0.012, 0.048, R))
        pin_end = m.cylinder((0, 0, R), 0.029, 0.012, axis=X, segments=self.seg)
        main_end = m.cylinder((0, 0, 0), 0.036, 0.012, axis=X, segments=self.seg)
        weight = m.intersect([m.cylinder((0, 0, 0), 0.07, 0.012, axis=X, segments=self.seg * 2),
                              m.box((-0.001, -0.075, -0.075), (0.014, 0.15, 0.063))])
        web = m.union([plate, pin_end, main_end, weight])
        web = m.component([web], "Crank web")
        first = True
        for i, xa in enumerate(self.xc):
            aa, ab = self.pin_angle[("A", i)], self.pin_angle[("B", i)]
            for x, ang in ((xa - 0.023, aa), (xa + 0.041, ab)):
                t = Transform.translate(x, 0, 0) @ Transform.axis_angle(X, -ang)
                if first:
                    web.set_transform(t)
                    parts.append(web)
                    first = False
                else:
                    parts.append(m.place(web, t))
            for x, ang in ((xa, aa), (xa + STAGGER, ab)):  # crank pins
                p = Transform.axis_angle(X, -ang).apply((0, 0, R))
                parts.append(self.cyl_x(x - 0.0115, x + 0.0115, 0.026, p[1], p[2], segments=self.seg))
            pa = Transform.axis_angle(X, -aa).apply((0, 0, R))
            pb = Transform.axis_angle(X, -ab).apply((0, 0, R))
            parts.append(m.union([self.cyl_x(xa + 0.011, xa + 0.019, 0.03, pa[1], pa[2], segments=self.seg),
                                  self.cyl_x(xa + 0.011, xa + 0.019, 0.03, pb[1], pb[2], segments=self.seg)]))
        # snout, sprockets, damper; flange, flexplate and ring gear
        parts.append(self.cyl_x(self.x_front - 0.105, self.mains[0] - 0.015, 0.02, segments=self.seg))
        for xs, teeth in ((-0.112, 20), (-0.112 + STAGGER, 20)):
            parts.append(self.sprocket(xs + self.xc[0], teeth, "Crank sprocket"))
        parts.append(self.damper(self.x_front - 0.08))
        parts.append(self.cyl_x(self.mains[-1] + 0.015, self.x_rear + 0.012, 0.045, segments=self.seg))
        parts += self.flywheel(self.x_rear + 0.012)
        crank = m.group(parts, name="Crankshaft")
        crank.paint(self.polished)
        self.crank = crank
        self.crank_t0 = crank.transform()
        self.moving.append(crank)
        self.whole.append(crank)
        self.labels["crank"] = crank

    def sprocket(self, x, teeth, name):
        """A chain sprocket in the plane x: a disc with ``teeth`` teeth (one tooth turned
        around in a radial array), lightening holes and a hub."""
        m = self.m
        pitch_r = 0.0095 / (2 * math.sin(math.pi / teeth))
        half = math.pi / teeth
        disc = m.cylinder((x - 0.004, 0, 0), pitch_r - 0.003, 0.008, axis=X, segments=teeth * 2)

        def p(r, a):
            return (x - 0.004, r * math.sin(a), r * math.cos(a))

        tooth = m.extrude([p(pitch_r - 0.0035, -half * 0.6), p(pitch_r - 0.0035, half * 0.6),
                           p(pitch_r + 0.004, half * 0.22), p(pitch_r + 0.004, -half * 0.22)], 0.008)
        ring = [disc, tooth] + tooth.polar(X, 360.0 / teeth, teeth - 1, (x, 0, 0))
        body = m.union(ring)
        holes = []
        if pitch_r > 0.035:
            hole = self.cyl_x(x - 0.01, x + 0.01, 0.0075, 0, pitch_r * 0.55, segments=12)
            holes = [hole] + hole.polar(X, 72, 4, (x, 0, 0))
        hub = m.cylinder((x - 0.007, 0, 0), 0.016, 0.014, axis=X, segments=self.seg)
        body = m.union([m.subtract(body, holes) if holes else body, hub])
        body.paint(self.chain_mat).name(name)
        return body

    def damper(self, x):
        """Crank pulley and damper: a lathe profile with belt grooves."""
        prof = [(x, 0.0, 0.021), (x, 0.0, 0.07), (x + 0.004, 0.0, 0.074)]
        for k in range(4):  # poly-V grooves
            xa = x + 0.006 + 0.005 * k
            prof += [(xa, 0.0, 0.074), (xa + 0.0025, 0.0, 0.071), (xa + 0.005, 0.0, 0.074)]
        prof += [(x + 0.028, 0.0, 0.074), (x + 0.032, 0.0, 0.07), (x + 0.032, 0.0, 0.021)]
        d = self.m.revolve(prof, (x, 0, 0), X, segments=self.seg * 2)
        d.paint(self.black)
        return d

    def flywheel(self, x):
        m = self.m
        plate = m.cylinder((x, 0, 0), 0.128, 0.004, axis=X, segments=self.seg * 2)
        hole = self.cyl_x(x - 0.01, x + 0.01, 0.014, 0, 0.085, segments=12)
        plate = m.subtract(plate, [hole] + hole.polar(X, 60, 5, (x, 0, 0)))
        teeth = 40 if self.quick else 120
        ring = m.tube((x - 0.005, 0, 0), 0.1405, 0.126, 0.014, axis=X, segments=teeth)
        half = math.pi / teeth

        def p(r, a):
            return (x - 0.005, r * math.sin(a), r * math.cos(a))

        tooth = m.extrude([p(0.1395, -half * 0.55), p(0.1395, half * 0.55), p(0.1475, half * 0.22), p(0.1475, -half * 0.22)], 0.014)
        gear = m.union([ring, tooth] + tooth.polar(X, 360.0 / teeth, teeth - 1, (x, 0, 0)))
        gear.paint(self.forged).name("Ring gear")
        plate.paint(self.forged)
        b = self.bolt(0.005, 0.012, "Flywheel bolt")
        into_flange = Transform.look((x + 0.0055, 0, 0.035), (-1, 0, 0), (0, 1, 0))
        bolts = [m.place(b, Transform.axis_angle(X, 60 * k) @ into_flange) for k in range(6)]
        b.hide()
        return [plate, gear] + bolts

    def piston(self):
        """A piston (component) with its pin at the origin and the crown up +Z."""
        m = self.m
        body = m.cylinder((0, 0, -0.026), BORE - 0.0004, 0.058, segments=self.seg * 2)
        body = m.subtract(body, [m.cylinder((0, 0, -0.03), 0.039, 0.046, segments=self.seg)])
        body = m.union([body, self.cyl_x(-0.04, 0.04, 0.0145, segments=self.seg)])
        tools = [m.tube((0, 0, z), 0.048, 0.0425, 0.0016, segments=self.seg * 2) for z in (0.0255, 0.0215, 0.0165)]
        for dx in (-0.017, 0.017):  # valve reliefs in the crown
            for dy in (-0.021, 0.021):
                tools.append(m.cylinder((dx, dy, 0.0305), 0.0165, 0.003, segments=self.seg))
        for sx in (-1, 1):  # slipper skirt cut-outs under the pin bosses
            tools.append(m.box((0.036 if sx > 0 else -0.05, -0.05, -0.03), (0.014, 0.1, 0.012)))
        tools.append(self.cyl_x(-0.05, 0.05, 0.0105, segments=self.seg))
        body = m.subtract(body, tools)
        body.paint(self.alu)
        pin = m.tube((-0.036, 0, 0), 0.0104, 0.006, 0.072, axis=X, segments=self.seg, material=self.polished)
        rings = [m.tube((0, 0, z + 0.00015), BORE - 0.0003, 0.0436, 0.0013, segments=self.seg * 2, material=self.chain_mat)
                 for z in (0.0255, 0.0215, 0.0165)]
        return m.component([body, pin] + rings, "Piston")

    def rod(self):
        """A connecting rod (component): big end at the origin, small end at +Z ROD."""
        m = self.m
        big = m.cylinder((-0.0105, 0, 0), 0.034, 0.021, axis=X, segments=self.seg)
        small = m.cylinder((-0.0105, 0, ROD), 0.0165, 0.021, axis=X, segments=self.seg)
        shank = m.box((-0.0095, -0.011, 0.025), (0.019, 0.022, ROD - 0.035))
        shank.chamfer(self.edges_along(shank, 2), 0.002)
        rod = m.union([big, small, shank])
        pockets = [m.box((x, -0.0072, 0.045), (0.0075, 0.0144, ROD - 0.075)) for x in (0.0035, -0.011)]
        bores = [self.cyl_x(-0.02, 0.02, 0.0265, segments=self.seg), self.cyl_x(-0.02, 0.02, 0.0105, 0, ROD, segments=self.seg)]
        rod = m.subtract(rod, pockets + bores)
        rod.paint(self.forged)
        shell = m.tube((-0.0095, 0, 0), 0.0265, 0.0248, 0.019, axis=X, segments=self.seg, material=self.copper)
        bolts = []
        for y in (-0.0302, 0.0302):
            bolts.append(m.cylinder((0, y, -0.036), 0.0032, 0.052, segments=10, material=self.zinc))
            bolts.append(m.prism((0, y, -0.041), 0.0058, 0.005, sides=6, material=self.zinc))
        return m.component([rod, shell] + bolts, "Connecting rod")

    def pistons_and_rods(self):
        m = self.m
        self.piston_def = self.piston()
        self.rod_def = self.rod()
        self.cyl_parts = []
        first = True
        for b in "AB":
            for i in range(self.n):
                if first:
                    p, r = self.piston_def, self.rod_def
                    first = False
                else:
                    p, r = m.place(self.piston_def), m.place(self.rod_def)
                self.cyl_parts.append((b, i, p, r))
                self.moving += [p, r]
                self.whole += [p, r]
        self.labels["piston"] = self.cyl_parts[min(1, self.n - 1)][2]
        self.set_crank(0.0)

    def cylinder_x(self, b, i):
        return self.xc[i] + (STAGGER if b == "B" else 0.0)

    def set_crank(self, phi):
        """Put the crank, rods, pistons and cams at crank angle ``phi`` degrees."""
        for b, i, piston, rod in self.cyl_parts:
            x = self.cylinder_x(b, i)
            beta = math.radians(self.pin_angle[(b, i)] + phi)
            pin = (x, R * math.sin(beta), R * math.cos(beta))
            d = AXIS[b]
            s_pin = dot(pin, d)
            q = length(sub(pin, add((x, 0, 0), mul(d, s_pin))))
            wrist = add((x, 0, 0), mul(d, s_pin + math.sqrt(ROD * ROD - q * q)))
            lift = Transform.translate(self.offset)
            rod.set_transform(lift @ Transform.look(pin, sub(wrist, pin), X))
            piston.set_transform(lift @ Transform.look(wrist, d, X))
        if hasattr(self, "crank"):
            self.crank.set_transform(Transform.axis_angle(X, -phi, self.offset) @ self.crank_t0)
        for cam, t0, centre in getattr(self, "cams", []):
            cam.set_transform(Transform.axis_angle(X, -phi / 2, add(centre, self.offset)) @ t0)

    def lift(self, offset=LIFT):
        """Move everything (instances and the root section plane) by ``offset``."""
        c = self.m.client
        ents = c.query("entities", kinds=["instance"])["instances"]
        planes = [p["id"] for p in c.query("section_planes")["planes"]]
        c.execute({"type": "move", "ids": [e["id"] for e in ents] + planes, "offset": list(offset)})
        self.offset = offset
        self.crank_t0 = self.crank.transform()
        self.cams = [(cam, cam.transform(), centre) for cam, _, centre in self.cams]

    def at(self, p):
        """A point of the engine, in model coordinates after the lift."""
        return add(p, self.offset)

    # one bank's top end, drawn in the bank frame -------------------------

    def head(self):
        m = self.m
        xa, xb = self.xc[0] - 0.066, self.xc[-1] + 0.066
        s0 = DECK + GASKET
        head = m.box((xa, -0.086, s0), (xb - xa, 0.172, HEAD_TOP - s0), name="Cylinder head")
        head.chamfer(self.edges_along(head, 0, lambda a, b: a[2] > HEAD_TOP - 1e-6), 0.008)
        tools = []
        for xc in self.xc:
            tools.append(m.cylinder((xc, 0, s0 - 0.001), BORE - 0.0015, 0.0055 + 0.001, segments=self.seg))  # chamber
            tools.append(m.cylinder((xc, 0, s0 + 0.004), 0.006, HEAD_TOP - s0, segments=12))  # spark plug
            for side in (1, -1):
                u = valve_axis(side)
                for dx in (-0.017, 0.017):
                    seat = valve_seat(xc, dx, side)
                    tools.append(m.cylinder(add(seat, mul(u, -0.002)), 0.0135 if side > 0 else 0.0122, 0.016, axis=u, segments=self.seg))
                    tools.append(m.cylinder(add(seat, mul(u, 0.012)), 0.0042, 0.06, axis=u, segments=10))  # guide
                    port_in = add(seat, mul(u, 0.01))
                    port_out = (xc + dx * 0.6, side * 0.1, s0 + 0.036)
                    tools.append(self.between(port_in, port_out, 0.012 if side > 0 else 0.011, segments=self.seg))
        for xs in [self.xc[0] - 0.053] + [x + 0.053 for x in self.xc]:
            for y in (-0.045, 0.045):
                tools.append(m.cylinder((xs, y, s0 - 0.001), 0.0058, HEAD_TOP - s0 + 0.002, segments=12))
        head = m.subtract(head, tools)
        head.paint(self.alu).name("Cylinder head")
        gasket = m.box((xa, -0.07, DECK), (xb - xa, 0.14, GASKET))
        gasket = m.subtract(gasket, [m.cylinder((xc, 0, DECK - 0.001), BORE + 0.0015, 0.004, segments=self.seg) for xc in self.xc])
        gasket.paint(self.copper).name("Head gasket")
        return head, gasket

    def valve(self, side):
        """A valve along +Z from its face at the origin (component)."""
        m = self.m
        r = 0.0155 if side > 0 else 0.0135
        face = m.cylinder((0, 0, 0), r, 0.0015, segments=self.seg)
        tulip = m.cone((0, 0, 0.0015), r, 0.008, top_radius=0.0035, segments=self.seg)
        stem = m.cylinder((0, 0, 0.009), 0.003, 0.088, segments=10)
        keeper = m.cone((0, 0, 0.088), 0.0042, 0.006, top_radius=0.0034, segments=10)
        v = m.group([face, tulip, stem, keeper])
        v.paint(self.polished)
        return m.component([v], "Intake valve" if side > 0 else "Exhaust valve")

    def spring_stack(self):
        """Valve spring, retainer and bucket tappet along +Z from the spring seat (component)."""
        m = self.m
        turns = 4 if self.quick else 6
        spring = m.spring((0, 0, 0.0012), (0, 0, 1), 0.0105, 0.0017, 0.0265 / turns, turns,
                          segments=12 if self.quick else 18, wire_segments=6, material=self.spring_mat)
        seat = m.tube((0, 0, 0), 0.0125, 0.0045, 0.0012, segments=self.seg, material=self.polished)
        retainer = m.cone((0, 0, 0.0285), 0.0125, 0.0035, top_radius=0.0095, segments=self.seg, material=self.polished)
        bucket = m.cylinder((0, 0, 0.031), 0.0142, 0.012, segments=self.seg, material=self.polished)
        return m.component([spring, seat, retainer, bucket], "Valve spring")

    def lobe(self):
        """One cam lobe in the plane x = 0, nose along +Z: the hull of base circle and nose."""
        pts = []
        for k in range(48):
            a = 2 * math.pi * k / 48
            pts.append((0.018 * math.cos(a), 0.018 * math.sin(a)))
            pts.append((0.0072 * math.cos(a), 0.019 + 0.0072 * math.sin(a)))
        loop = hull2([(round(y, 9), round(z, 9)) for y, z in pts])
        lobe = self.m.extrude([(-0.007, y, z) for y, z in loop], 0.014)
        lobe.paint(self.polished)
        return self.m.component([lobe], "Cam lobe")

    def camshaft(self, side, lobe, sprocket):
        """A camshaft along X in the bank frame; returns its parts (lobes placed)."""
        m = self.m
        c = cam_axis_point(side)
        parts = [self.cyl_x(-0.112 + 0.004, self.xc[-1] + 0.06, 0.0115, c[1], c[2], segments=self.seg)]
        for xj in self.journals():
            parts.append(self.cyl_x(xj - 0.007, xj + 0.007, 0.0135, c[1], c[2], segments=self.seg))
        for i, xc in enumerate(self.xc):
            for dx in (-0.017, 0.017):
                ang = -self.pin_angle[("A", i)] / 2 + (90 if side > 0 else -30)
                t = Transform.translate(xc + dx, c[1], c[2]) @ Transform.axis_angle(X, ang)
                parts.append(m.place(lobe, t))
        parts.append(m.place(sprocket, Transform.translate(0, c[1], c[2])))
        return parts

    def journals(self):
        return [self.xc[0] - 0.045] + [x + 0.053 for x in self.xc[:-1]] + [self.xc[-1] + 0.047]

    def bearing_tower(self, side):
        m = self.m
        c = cam_axis_point(side)
        t = m.box((-0.008, c[1] - 0.017, HEAD_TOP), (0.016, 0.034, c[2] + 0.016 - HEAD_TOP))
        t.chamfer(self.edges_along(t, 0, lambda a, b: a[2] > c[2] + 0.0159), 0.004)
        t = m.subtract(t, [self.cyl_x(-0.01, 0.01, 0.0137, c[1], c[2], segments=self.seg)])
        t.paint(self.alu)
        return m.component([t], "Cam bearing")

    def valve_cover(self):
        m = self.m
        xa, xb = self.xc[0] - 0.07, self.xc[-1] + 0.07
        top = HEAD_TOP + 0.115
        cover = m.box((xa, -0.088, HEAD_TOP), (xb - xa, 0.176, top - HEAD_TOP), name="Valve cover")
        cover.fillet(self.edges_along(cover, 2), 0.02)
        cover.fillet(cover.edges(lambda a, b: a[2] > top - 1e-6 and b[2] > top - 1e-6), 0.012, segments=4)
        inner = m.box((xa + 0.006, -0.082, HEAD_TOP - 0.005), (xb - xa - 0.012, 0.164, top - HEAD_TOP - 0.001))
        holes = [m.cylinder((xc, 0, top - 0.02), 0.0135, 0.04, segments=self.seg) for xc in self.xc]
        cover = m.subtract(cover, [inner] + holes)
        cover.paint(self.paint).name("Valve cover")
        return cover

    def plug_and_coil(self):
        m = self.m
        s0 = DECK + GASKET
        thread = m.cylinder((0, 0, s0 + 0.004), 0.006, 0.02, segments=12, material=self.polished)
        hexa = m.prism((0, 0, s0 + 0.024), 0.0095, 0.008, sides=6, material=self.polished)
        insul = m.cone((0, 0, s0 + 0.032), 0.0062, 0.03, top_radius=0.0045, segments=12, material=self.ceramic)
        coil = m.cylinder((0, 0, HEAD_TOP + 0.06), 0.012, 0.075, segments=self.seg, material=self.black)
        cap = m.box((-0.02, -0.014, HEAD_TOP + 0.135), (0.04, 0.028, 0.014), material=self.black)
        return m.component([thread, hexa, insul, coil, cap], "Plug and coil")

    def runners(self):
        """Intake runners from the head's valley side up into the plenum (follow_me pipes),
        given in the bank frame of bank A (bank B gets the mirror image)."""
        m = self.m
        inv = FRAME["A"].inverse()
        out = []
        for xc in self.xc:
            p0 = (xc, 0.086, DECK + GASKET + 0.036)
            pts = [p0]
            # a quadratic bend from the port, out of the head, to the plenum's underside
            a = FRAME["A"].apply(p0)
            c = add(a, mul(W_A, 0.09))
            e = (xc, -0.022, 0.43)
            for k in range(1, 9):
                t = k / 8
                q = add(add(mul(a, (1 - t) ** 2), mul(c, 2 * t * (1 - t))), mul(e, t * t))
                pts.append(inv.apply(q))
            out.append(m.sweep(pts, 0.018, inner_radius=0.0155, segments=self.seg, name="Intake runner", material=self.alu))
        return out

    def exhaust(self):
        m = self.m
        out = []
        ys, ss = -0.13, DECK - 0.07
        for xc in self.xc:
            pts = [(xc, -0.084, DECK + GASKET + 0.036), (xc, -0.11, DECK + GASKET + 0.036)]
            for k in range(1, 6):  # a quarter bend down to the collector
                a = math.pi / 2 * k / 5
                pts.append((xc + 0.012 * k / 5, ys + 0.02 * (1 - math.cos(a)) - 0.0, DECK + GASKET + 0.036 - 0.07 * math.sin(a)))
            pts.append((xc + 0.012, ys + 0.02, ss + 0.02))
            out.append(m.sweep(pts, 0.016, inner_radius=0.0135, segments=self.seg, name="Exhaust primary", material=self.exhaust_mat))
        xa, xb = self.xc[0] - 0.03, self.xc[-1] + 0.07
        col = [(xa, ys + 0.02, ss), (xb, ys + 0.02, ss), (xb + 0.04, ys + 0.02, ss - 0.05), (xb + 0.05, ys + 0.02, ss - 0.12)]
        out.append(m.sweep(col, 0.024, inner_radius=0.021, segments=self.seg, name="Exhaust collector", material=self.exhaust_mat))
        flange = m.box((xa - 0.005, -0.092, DECK + 0.01), (xb - xa - 0.03, 0.006, 0.05), material=self.exhaust_mat)
        out.append(flange)
        return out

    def bank_top(self):
        """Heads, gaskets, valvetrain, cams, covers, plugs, runners and exhausts of both banks."""
        m = self.m
        head, gasket = self.head()
        cover = self.valve_cover()
        runners = self.runners()
        exhaust = self.exhaust()
        hb = self.bolt(0.0055, 0.11, "Head bolt")
        v_in, v_ex = self.valve(1), self.valve(-1)
        stack = self.spring_stack()
        lobe = self.lobe()
        cam_sprocket = self.sprocket(-0.112, 26, "Cam sprocket")
        cam_sprocket = m.component([cam_sprocket], "Cam sprocket")
        towers = {s: self.bearing_tower(s) for s in (1, -1)}
        plug = self.plug_and_coil()
        local_parts = [head, gasket, cover] + runners + exhaust
        # bank B gets copies of the drawn parts, then both are placed by their frames
        copies = {"A": local_parts, "B": [p.clone() for p in local_parts]}
        for b in "AB":
            for p in copies[b]:
                p.apply(FRAME[b])
        self.heads = {b: copies[b][0] for b in "AB"}
        self.covers = {b: copies[b][2] for b in "AB"}
        self.runner_insts = copies["A"][3:3 + len(runners)] + copies["B"][3:3 + len(runners)]
        self.cams = []
        springs = []
        defs = [hb, v_in, v_ex, stack, cam_sprocket, lobe, plug] + list(towers.values())
        for b in "AB":
            f = FRAME[b]
            for xs in [self.xc[0] - 0.053] + [x + 0.053 for x in self.xc]:
                for y in (-0.045, 0.045):
                    m.place(hb, f @ Transform.look((xs, y, HEAD_TOP), (0, 0, -1), X))
            for xc in self.xc:
                self.whole.append(m.place(plug, f @ Transform.translate(xc, 0, 0)))
                for side in (1, -1):
                    u = valve_axis(side)
                    for dx in (-0.017, 0.017):
                        seat = valve_seat(xc, dx, side)
                        self.whole.append(m.place(v_in if side > 0 else v_ex, f @ Transform.look(seat, u, X)))
                        sp = m.place(stack, f @ Transform.look(add(seat, mul(u, 0.0595)), u, X))
                        self.whole.append(sp)
                        springs.append(sp)
            for side in (1, -1):
                for xj in self.journals():
                    m.place(towers[side], f @ Transform.translate(xj, 0, 0))
                parts = self.camshaft(side, lobe, cam_sprocket)
                cam = m.group(parts, name="Camshaft")
                cam.set_transform(f @ cam.transform())
                centre = f.apply(cam_axis_point(side))
                self.cams.append((cam, cam.transform(), centre))
                self.moving.append(cam)
                self.whole.append(cam)
        for d in defs:  # the drawn originals sit at the bank frame's origin; tuck them away
            d.hide()
        self.labels["springs"] = springs[min(6, len(springs) - 1)]  # a bank A exhaust spring
        self.labels["cam"] = self.cams[1][0]  # bank A's exhaust cam

    # the front: timing chains, belt and accessories --------------------

    def timing(self):
        m = self.m
        self.chains = []
        for b in "AB":
            f = FRAME[b]
            xs = -0.112 + (STAGGER if b == "B" else 0.0) + self.xc[0]
            circles = [(0.0, 0.0, 0.0095 / (2 * math.sin(math.pi / 20)))]
            for side in (1, -1):
                c = f.apply(cam_axis_point(side))
                circles.append((c[1], c[2], 0.0095 / (2 * math.sin(math.pi / 26))))
            loop = resample(circle_hull(circles, samples=96), 0.0095 / 3)
            path = [(xs, y, z) for y, z in loop]
            # one link: two side plates and two rollers, laid along the first edge
            t = unit(sub(path[1], path[0]))
            link_pts = []
            for k in range(16):
                a = 2 * math.pi * k / 16
                link_pts.append((0.0045 * math.cos(a) - 0.0, 0.0045 * math.sin(a)))
                link_pts.append((0.0045 * math.cos(a) + 0.0095, 0.0045 * math.sin(a)))
            outline = hull2([(round(u, 9), round(v, 9)) for u, v in link_pts])
            frame = Transform.look(path[0], (1, 0, 0), t)  # local x along the chain, z across it
            plates = []
            for dz in (-0.0055, 0.0035):
                pl = m.extrude([frame.apply((u - 0.00475, v, dz)) for u, v in outline], 0.002)
                plates.append(pl)
            rollers = [m.cylinder(frame.apply((u, 0, -0.0045)), 0.0031, 0.009, axis=(1, 0, 0), segments=10)
                       for u in (-0.00475, 0.00475)]
            link = m.group(plates + rollers, name="Chain link")
            link.paint(self.chain_mat)
            r = m.client.execute({"type": "add_curve", "points": [list(p) for p in path], "closed": True, "curve": False})
            edges = [i for i in r["changes"]["created"] if i.startswith("e")]
            count = int(round(self.loop_length(path) / 0.0095))
            links = m.copy_along_path([link], edges, count=count)
            m.client.execute({"type": "erase", "ids": edges})
            chain = m.group([link] + links, name="Timing chain")
            self.chains.append(chain)
            self.whole.append(chain)
        self.labels["chain"] = self.chains[0]

    @staticmethod
    def loop_length(path):
        return sum(math.dist(path[i], path[(i + 1) % len(path)]) for i in range(len(path)))

    def front(self):
        """Water pump, alternator, tensioner and the serpentine belt in the plane of the damper."""
        m = self.m
        xb = self.x_front - 0.08 + 0.016  # belt plane: the damper grooves
        pulleys = [(0.0, 0.0, 0.074)]
        wp = (0.0, 0.2, 0.035)  # water pump pulley (y, z, r)
        alt = (0.24, 0.12, 0.03)
        idl = (-0.13, 0.13, 0.03)
        pulleys += [wp, alt, idl]
        for y, z, r in pulleys[1:]:
            pul = m.cylinder((xb - 0.014, y, z), r, 0.028, axis=X, segments=self.seg, material=self.forged)
            hub = m.cylinder((xb - 0.022, y, z), r * 0.35, 0.008, axis=X, segments=12, material=self.forged)
            self.whole += [pul, hub]
        # water pump body into the front of the block
        body = m.cylinder((xb + 0.014, wp[0], wp[1]), 0.045, 0.01, axis=X, segments=self.seg, material=self.alu)
        self.whole.append(body)
        # alternator: finned body with cooling slots, fan and bracket
        ax0 = xb + 0.014
        body = m.cylinder((ax0 + 0.012, alt[0], alt[1]), 0.055, 0.09, axis=X, segments=self.seg * 2)
        slot = m.box((ax0 + 0.022, alt[0] - 0.004, alt[1] + 0.045), (0.07, 0.008, 0.02))
        slots = [slot] + slot.polar(X, 30, 11, (0, alt[0], alt[1]))
        body = m.subtract(body, slots)
        body.paint(self.alu).name("Alternator")
        fan = m.cylinder((ax0, alt[0], alt[1]), 0.045, 0.003, axis=X, segments=self.seg)
        blade = m.box((ax0 + 0.002, alt[0] - 0.002, alt[1] + 0.018), (0.009, 0.004, 0.026))
        fan = m.union([fan, blade] + blade.polar(X, 30, 11, (0, alt[0], alt[1])))
        fan.paint(self.forged)
        self.whole += [body, fan]
        # belt: a rectangle swept round the pulleys
        loop = resample(circle_hull([(y, z, r + 0.002) for y, z, r in pulleys], samples=96), 0.012 if self.quick else 0.006)
        path = [(xb, y, z) for y, z in loop]
        belt = m.sweep(path, closed=True, profile=[(-0.011, -0.002), (0.011, -0.002), (0.011, 0.002), (-0.011, 0.002)],
                       smooth=False, name="Belt", material=self.rubber)
        self.whole.append(belt)
        # intake plenum and throttle body over the valley
        xa, xe = self.xc[0] - 0.06, self.xc[-1] + 0.09
        plenum = m.tube((xa, 0, 0.45), 0.058, 0.053, xe - xa, axis=X, segments=self.seg * 2)
        caps = [self.cyl_x(xa - 0.006, xa, 0.058, 0, 0.45, segments=self.seg * 2),
                self.cyl_x(xe, xe + 0.006, 0.058, 0, 0.45, segments=self.seg * 2)]
        plenum = m.union([plenum] + caps)
        plenum = m.subtract(plenum, [self.cyl_x(xa - 0.01, xa + 0.001, 0.035, 0, 0.45, segments=self.seg)])
        plenum.paint(self.alu).name("Plenum")
        tb = m.tube((xa - 0.05, 0, 0.45), 0.04, 0.035, 0.05, axis=X, segments=self.seg, material=self.alu)
        plate = m.cylinder((xa - 0.03, 0, 0.45), 0.0345, 0.002, axis=(1, 0, 0.6), segments=self.seg, material=self.polished)
        lip = m.torus((xa - 0.05, 0, 0.45), 0.0375, 0.0045, axis=X, segments=self.seg, tube_segments=8, material=self.alu)
        xf = (self.xc[0] + self.xc[-1]) / 2  # the oil filter's seal against the block
        seal = m.torus((xf, 0.145, -0.02), 0.03, 0.003, axis=(0, 1, 0), segments=self.seg, tube_segments=6, material=self.rubber)
        self.plenum = plenum
        self.whole += [tb, plate, lip, seal]

    # cutaway, look, labels, scenes ------------------------------------

    def cutaway(self):
        m = self.m
        c = m.client
        # the root plane: through the crank axis along bank A's bores, keeping the valley side
        c.execute({"type": "add_section_plane", "point": [0, 0, 0], "normal": list(W_A)})
        m.section_cut(self.whole + self.runner_insts, cut=False)
        # per-group planes: bank B's cover opens toward the camera, the plenum is halved
        f = FRAME["B"]
        self.covers["B"].section(f.apply((0, 0, 0)), f.apply_vector((0, -1, 0)))
        self.plenum.section((0, 0, 0.45), (0, 1, 0.35))
        m.section_display(planes=False, fill=True, fill_color="material")

    def look(self):
        m = self.m
        m.lighting("studio")
        m.style("Cutaway", background=[226, 228, 232], sky=[250, 251, 253],
                edges={"color": [28, 28, 32], "profiles": 2}, ambient_occlusion={"on": True, "radius": 0.02, "strength": 1.2})
        self.centre = self.at(((self.xc[0] + self.xc[-1]) / 2, -0.03, 0.14))
        self.eye = add(self.centre, (-0.85, -1.05, 0.6))
        self.front_eye = add(self.centre, (-1.75, -0.3, 0.62))
        m.camera(self.eye, self.centre, fov_y_deg=30)

    def callouts(self):
        """Leader labels around the engine: each text sits a little in front of the engine's
        centre, (dx, dy) meters right and up on screen, so the leaders stay in front of it."""
        fwd = unit(sub(self.centre, self.eye))
        right = unit((fwd[1], -fwd[0], 0.0))  # fwd x Z
        up = (right[1] * fwd[2] - right[2] * fwd[1], right[2] * fwd[0] - right[0] * fwd[2], right[0] * fwd[1] - right[1] * fwd[0])
        for key, text, dx, dy in (("piston", "Pistons", 0.36, -0.12), ("crank", "Crankshaft", 0.2, -0.28),
                                  ("cam", "Camshaft", 0.38, 0.1), ("springs", "Valve springs", 0.12, 0.29),
                                  ("chain", "Timing chain", -0.42, 0.18)):
            pos = add(add(self.centre, mul(fwd, -0.2)), add(mul(right, dx), mul(up, dy)))
            self.m.label(self.labels[key], text, position=pos)

    def scenes(self):
        m = self.m
        self.set_crank(0.0)
        s0 = m.scene("Crank 0", positions=self.moving)
        self.set_crank(120.0)
        s1 = m.scene("Crank 120", positions=self.moving)
        m.transition(True, 1.5)
        m.go_to_scene(s0)
        return s0, s1


def build(m, quick=False, log=print):
    v = V6(m, quick)
    t0 = time.time()
    steps = [("materials", v.materials), ("block", v.block), ("crankshaft", v.crankshaft),
             ("pistons and rods", v.pistons_and_rods), ("heads and valvetrain", v.bank_top),
             ("timing chains", v.timing), ("front and intake", v.front), ("cutaway", v.cutaway), ("lift", v.lift)]
    for name, fn in steps:
        t = time.time()
        fn()
        log("  %-22s %6.1f s" % (name, time.time() - t))
    log("built in %.1f s" % (time.time() - t0))
    return v


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--socket", help="socket of a running app (default: start a headless one)")
    ap.add_argument("--app", help="app binary to start (default: SKETCHUP_ALT_APP or the repo's target/)")
    ap.add_argument("--out", default=".", help="directory for the PNG files (default: here)")
    ap.add_argument("--quick", action="store_true", help="one cylinder per bank and small images (for tests)")
    ap.add_argument("--width", type=int, help="image width (default 1600, quick 320)")
    ap.add_argument("--height", type=int, help="image height (default 1000, quick 200)")
    args = ap.parse_args(argv)
    width = args.width or (320 if args.quick else 1600)
    height = args.height or (200 if args.quick else 1000)
    ss = 1 if args.quick else 2
    os.makedirs(args.out, exist_ok=True)
    with session(args.socket, args.app) as s:
        m = Model(s.client)
        s.client.execute({"type": "new", "discard": s.launched})
        v = build(m, args.quick)
        v.look()
        v.callouts()
        s0, s1 = v.scenes()
        t = time.time()
        out = os.path.join(args.out, "v6.png")
        m.render(out, width, height, lighting=None, supersample=ss)
        front = os.path.join(args.out, "v6-front.png")
        m.render(front, width, height, lighting=None, supersample=ss,
                 camera={"eye": list(v.front_eye), "target": list(v.centre), "fov_y_deg": 30})
        motion = os.path.join(args.out, "v6-motion.png")
        m.render(motion, width // 2, height // 2, lighting=None, scene=s1, from_=s0, t=0.5)
        print("rendered in %.1f s: %s" % (time.time() - t, ", ".join((out, front, motion))))


if __name__ == "__main__":
    main()
