#!/usr/bin/env python3
"""A six-axis industrial robot in a palletising work cell, rigged and animated.

    python3 robot_cell.py [--socket PATH] [--out DIR] [--quick]

Without ``--socket`` it starts a headless app (``TENON_APP`` or the repo's
``target/{debug,release}/app``). It writes ``robot.png`` (the cell, labelled), ``robot-detail.png``
(the wrist and gripper holding a box), ``robot-section.png`` (the arm cut open along its middle),
``robot-motion.gif`` (a pick and place, when ``ffmpeg`` is on PATH) and ``robot-report.txt`` to
``--out``.

What it shows of the API:

- cast-like housings made of extruded side profiles (hulls of circles) with filleted edges and
  closed cavities (``solid_subtract``), and lathe-turned hubs, covers and wrist cylinders
  (``revolve``) with rounded shoulders;
- servo motors with finned housings (radial arrays), harmonic-drive covers with bolt circles,
  bolt heads that are one component placed in batches, cable dress packs (``follow_me`` sweeps);
- a rigged mechanism: revolute joints J1 to J6 with limits, a parallel gripper whose two fingers
  are prismatic joints coupled together (``add_coupling``), scenes storing the arm's positions;
- the cell: a roller conveyor (rollers copied along the frame), cardboard boxes (one component),
  a pallet, safety fencing (a mesh panel component of thin bars, copy arrays, placed along the
  fence), a light curtain with glowing strips, a control cabinet with a signal tower, floor markings;
- a pick-and-place motion solved with a closed-form inverse kinematics for the arm, posed frame by
  frame (the carried box follows the gripper), rendered to a GIF with ``ffmpeg``;
- checks: each axis turned through its limits against the rest of the arm, the J1 swing against the
  fence and the rest of the cell, ``interference`` and ``clearance`` at every pose of the motion,
  ``sweep_check`` of the gripper's coupled fingers, ``mass_properties`` of the arm with the
  materials' densities, the reach measured from the rig;
- studio lighting with sun-style shadows, ambient occlusion, leader labels, a section plane with
  filled cuts and ``set_section_cut`` false on the floor.

Lengths are meters. The robot stands at the origin, its zero pose has the upper arm vertical and
the forearm along +X: J1 turns about Z, J2, J3 and J5 about Y (a positive angle leans or pitches
toward +X and down), J4 and J6 about X, the tool axis.

A pose is set in one batch from the chain of joint rotations (``Cell.set_pose``) and not by driving
the joints one after another: a drive leaves every other joint free, and the solver spends the motion
wherever it is cheapest (driving J5 of this arm swings J3 as well), which is no good for a serial arm
that has to hold six values at once.
"""
import argparse
import math
import os
import shutil
import subprocess
import sys
import time

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))

from tenon import ApiError, Instance, Model, Transform, session  # noqa: E402

# ---------------------------------------------------------------- dimensions (meters)

J1_Z = 0.30  # the slewing bearing plane
SHOULDER = (0.14, 0.0, 0.58)  # J2 axis (along Y)
L2 = 0.66  # upper arm
ELBOW = (SHOULDER[0], 0.0, SHOULDER[2] + L2)  # J3 axis
L3 = 0.74  # forearm, elbow to wrist
WRIST = (ELBOW[0] + L3, 0.0, ELBOW[2])  # J5 axis
J4_X = 0.72  # where the forearm ends and the wrist roll begins
FLANGE_X = WRIST[0] + 0.115  # tool flange face
PAD_MID = 0.2425  # the pads' middle, from the flange: the tool centre point
TOOL = 0.115 + PAD_MID  # wrist axis to the tool centre point
FINGER_HOME = 0.058  # a pad's inner face, from the gripper's middle, with the fingers at 0

BOX = (0.16, 0.30, 0.24)  # cardboard box: across the gripper, along the conveyor, height
CONVEYOR_X, CONVEYOR_TOP = 1.0, 0.55
PICK = (CONVEYOR_X, -0.70)  # where the boxes stop
PALLET_C = (0.30, 1.05)
PALLET_H = 0.144
SLOTS = [(0.04, 0.85), (0.30, 0.85), (0.04, 1.25), (0.30, 1.25)]  # boxes already on the pallet
PLACE = (0.56, 0.85)  # where the carried box goes

STEEL, CAST_IRON, ALUMINIUM, COPPER, RUBBER, CARDBOARD, WOOD = 7850, 7200, 2700, 8960, 1200, 150, 550
LIMITS = {"J1": (-135, 135), "J2": (-50, 95), "J3": (-95, 25), "J4": (-180, 180), "J5": (-65, 115), "J6": (-360, 360)}
FINGER_MAX = 0.06
GRASP = 0.0222  # finger value that holds a box: the pads 0.2 mm off its sides

# ---------------------------------------------------------------- small vector helpers


def add(a, b):
    return tuple(x + y for x, y in zip(a, b))


def sub(a, b):
    return tuple(x - y for x, y in zip(a, b))


def mul(a, k):
    return tuple(x * k for x in a)


def dot(a, b):
    return sum(x * y for x, y in zip(a, b))


def cross(a, b):
    return (a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0])


def unit(a):
    n = math.sqrt(dot(a, a))
    return mul(a, 1 / n)


def perp(d, hint=None):
    """A unit vector square to ``d`` (as near ``hint`` as possible)."""
    if hint is None:
        hint = (0.0, 0.0, 1.0) if abs(d[2]) < 0.9 else (1.0, 0.0, 0.0)
    return unit(sub(hint, mul(d, dot(hint, d))))


def hull2(points):
    """Convex hull of 2D points, counter-clockwise (monotone chain)."""
    pts = sorted(set(points))

    def crs(o, a, b):
        return (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0])

    lower, upper = [], []
    for p in pts:
        while len(lower) >= 2 and crs(lower[-2], lower[-1], p) <= 0:
            lower.pop()
        lower.append(p)
    for p in reversed(pts):
        while len(upper) >= 2 and crs(upper[-2], upper[-1], p) <= 0:
            upper.pop()
        upper.append(p)
    return lower[:-1] + upper[:-1]


def circle_hull(circles, samples=24, extra=()):
    """The convex outline round 2D circles ``(cx, cz, r)`` and extra points: a tapered link."""
    pts = list(extra)
    for cx, cz, r in circles:
        for k in range(samples):
            a = 2 * math.pi * k / samples
            pts.append((round(cx + r * math.cos(a), 9), round(cz + r * math.sin(a), 9)))
    return hull2(pts)


def round_loop(pts, default=0.0, seg=5):
    """A closed 2D polygon with its corners rounded. A point is ``(u, v)`` or ``(u, v, radius)``."""
    n = len(pts)
    out = []
    for i, p in enumerate(pts):
        a, b = pts[i - 1], pts[(i + 1) % n]
        rad = p[2] if len(p) > 2 else default
        x, y = p[0], p[1]
        if rad <= 1e-9:
            out.append((x, y))
            continue
        u1, u2 = (a[0] - x, a[1] - y), (b[0] - x, b[1] - y)
        l1, l2 = math.hypot(*u1), math.hypot(*u2)
        u1, u2 = (u1[0] / l1, u1[1] / l1), (u2[0] / l2, u2[1] / l2)
        ang = math.acos(max(-1.0, min(1.0, u1[0] * u2[0] + u1[1] * u2[1])))
        if ang > math.pi - 1e-3:
            out.append((x, y))
            continue
        t = math.tan(ang / 2)
        d = min(rad / t, 0.5 * l1, 0.5 * l2)
        r = d * t
        t1, t2 = (x + u1[0] * d, y + u1[1] * d), (x + u2[0] * d, y + u2[1] * d)
        bis = (u1[0] + u2[0], u1[1] + u2[1])
        bl = math.hypot(*bis)
        cd = r / math.sin(ang / 2)
        c = (x + bis[0] / bl * cd, y + bis[1] / bl * cd)
        a1 = math.atan2(t1[1] - c[1], t1[0] - c[0])
        a2 = math.atan2(t2[1] - c[1], t2[0] - c[0])
        da = (a2 - a1 + math.pi) % (2 * math.pi) - math.pi
        for k in range(seg + 1):
            ak = a1 + da * k / seg
            out.append((c[0] + r * math.cos(ak), c[1] + r * math.sin(ak)))
    return out


def ring_points(center, axis, radius, count, start=None, phase=0.0):
    """``count`` points on a circle of ``radius`` about ``axis`` through ``center``, and the unit
    outward direction of each."""
    d = unit(axis)
    u = perp(d, start)
    v = cross(d, u)
    out = []
    for k in range(count):
        a = phase + 2 * math.pi * k / count
        o = add(mul(u, math.cos(a)), mul(v, math.sin(a)))
        out.append((add(center, mul(o, radius)), o))
    return out


# ---------------------------------------------------------------- inverse kinematics


def ik(r, theta, z, q6_deg):
    """Joint angles (degrees) that put the tool centre point at radius ``r``, azimuth ``theta``
    (degrees) and height ``z`` with the tool pointing straight down; None when out of reach."""
    wz = z + TOOL
    dx, dz = r - SHOULDER[0], wz - SHOULDER[2]
    d2 = dx * dx + dz * dz
    s3 = (L2 * L2 + L3 * L3 - d2) / (2 * L2 * L3)
    if abs(s3) > 1:
        return None
    q3 = math.asin(s3)
    a = L2 - L3 * s3
    b = L3 * math.cos(q3)
    q2 = math.atan2(dx, dz) - math.atan2(b, a)
    q5 = math.pi / 2 - q2 - q3
    return {"J1": theta, "J2": math.degrees(q2), "J3": math.degrees(q3), "J4": 0.0, "J5": math.degrees(q5), "J6": q6_deg}


def tcp_position(q):
    """Forward kinematics of the arm: the tool centre point for joint angles ``q`` (degrees)."""
    q1, q2, q3, q5 = (math.radians(q[k]) for k in ("J1", "J2", "J3", "J5"))
    ex, ez = SHOULDER[0] + L2 * math.sin(q2), SHOULDER[2] + L2 * math.cos(q2)
    wx, wz = ex + L3 * math.cos(q2 + q3), ez - L3 * math.sin(q2 + q3)
    psi = q2 + q3 + q5
    r, z = wx + TOOL * math.cos(psi), wz - TOOL * math.sin(psi)
    return (r * math.cos(q1), r * math.sin(q1), z)


# ---------------------------------------------------------------- the build


class Cell:
    def __init__(self, m, quick=False, log=print):
        self.m = m
        self.c = m.client
        self.quick = quick
        self.log = log
        self.seg = 20 if quick else 48  # circle segments for the larger round parts
        self.hull_n = 10 if quick else 20  # samples per circle of a link outline
        self.fins = 8 if quick else 16
        self.originals = []  # drawn component originals, erased once placed
        self.parts = {}  # name -> root instance
        self.panels = {}

    # helpers --------------------------------------------------------

    def place_many(self, comp, transforms):
        """One batch of ``place_instance`` for ``comp`` (an Instance); returns the new instances."""
        d = comp.definition
        if not transforms:
            return []
        r = self.c.batch([{"type": "place_instance", "definition": d, "transform": t.to_json()} for t in transforms])
        return [Instance(self.m, o["placed"][0], d) for o in r["outcomes"]]

    def slab(self, outline, y0, y1, mat, fillet=None, name=None, inner=None, wall=0.012):
        """The outline (x, z) pushed from y0 to y1, with the perimeter edges filleted and, given the
        ``inner`` outline, a closed cavity of it, ``wall`` in from both faces (a cast housing is hollow)."""
        g = self.m.extrude([(x, y0, z) for x, z in outline], y1 - y0, name=name, material=mat)
        if fillet:
            es = g.edges(lambda a, b: abs(a[1] - b[1]) < 1e-7)
            try:
                g.fillet(es, fillet)
            except ApiError as e:
                self.log("    (fillet skipped: %s)" % e)
        if inner is not None:
            cav = self.m.extrude([(x, y0 + wall, z) for x, z in inner], y1 - y0 - 2 * wall)
            g = self.m.subtract(g, [cav])
        return g

    def hull_slab(self, circles, y0, y1, mat, fillet, wall, extra=(), extra_in=None, thin=0.012):
        """A link side plate: the hull of ``circles`` (cx, cz, r) and ``extra`` points, hollowed with a
        cavity ``thin`` in from the skin (circles shrunk, the extra points moved to ``extra_in``)."""
        n = self.hull_n
        outline = circle_hull(circles, n, extra)
        inner = circle_hull([(cx, cz, r - thin) for cx, cz, r in circles], n, extra_in if extra_in is not None else extra)
        return self.slab(outline, y0, y1, mat, fillet, inner=inner, wall=wall), outline

    def cavity_cyl(self, solid, origin, axis, r, height):
        """``solid`` with a closed cylindrical cavity."""
        cav = self.m.cylinder(origin, r, height, axis=axis, segments=self.seg)
        return self.m.subtract(solid, [cav])

    def lathe(self, origin, axis, prof, mat, name=None, rounding=0.0, seg=None):
        """A body of revolution: ``prof`` is (distance along ``axis``, radius[, corner radius]) points
        of a closed outline."""
        d = unit(axis)
        u = perp(d)
        loop = round_loop(prof, rounding)
        pts = [add(add(origin, mul(d, a)), mul(u, r)) for a, r in loop]
        return self.m.revolve(pts, origin, d, segments=seg or self.seg, name=name, material=mat)

    def cyl_y(self, y0, y1, x, z, r, mat, **kw):
        """A cylinder along Y from y0 to y1 (either order)."""
        lo, hi = min(y0, y1), max(y0, y1)
        return self.m.cylinder((x, lo, z), r, hi - lo, axis=(0, 1, 0), segments=self.seg, material=mat, **kw)

    def bolt_ring(self, center, axis, radius, count, size=1.0, phase=0.0, start=None):
        pts = ring_points(center, axis, radius, count, start, phase)
        ts = [Transform.look(p, axis, o) @ Transform.scale(size) for p, o in pts]
        return self.place_many(self.bolt, ts)

    def bolt_at(self, points, axis, size=1.0):
        return self.place_many(self.bolt, [Transform.look(p, axis) @ Transform.scale(size) for p in points])

    def keep(self, name, inst):
        self.parts[name] = inst
        return inst

    # materials ------------------------------------------------------

    def materials(self):
        m = self.m
        self.orange = m.material("Robot orange", (236, 98, 14), metalness=0.25, roughness=0.34, density=ALUMINIUM)
        self.casting = m.material("Dark casting", (64, 67, 73), metalness=0.55, roughness=0.55, density=CAST_IRON)
        self.motor_black = m.material("Motor black", (30, 31, 34), metalness=0.35, roughness=0.5, density=3200)
        self.steel = m.material("Brushed steel", (196, 200, 206), metalness=1.0, roughness=0.33, density=STEEL)
        self.zinc = m.material("Zinc bolts", (182, 178, 150), metalness=0.9, roughness=0.35, density=STEEL)
        self.paper = m.material("Label paper", (236, 236, 230), roughness=0.8, density=CARDBOARD)
        self.rubber = m.material("Rubber", (24, 24, 26), roughness=0.9, density=RUBBER)
        self.cable = m.material("Cable sheath", (34, 35, 38), metalness=0.1, roughness=0.55, density=RUBBER)
        self.cable_grey = m.material("Cable grey", (118, 122, 128), metalness=0.2, roughness=0.5, density=RUBBER)
        self.graphite = m.material("Gripper graphite", (48, 51, 58), metalness=0.6, roughness=0.42, density=ALUMINIUM)
        self.yellow = m.material("Safety yellow", (246, 190, 0), metalness=0.1, roughness=0.5, density=STEEL)
        self.mesh = m.material("Fence mesh", (92, 96, 102), metalness=0.8, roughness=0.5, density=STEEL)
        self.conveyor_blue = m.material("Conveyor blue", (36, 86, 150), metalness=0.3, roughness=0.5, density=STEEL)
        self.zinc_plate = m.material("Zinc plated", (205, 209, 214), metalness=0.95, roughness=0.3, density=STEEL)
        self.cardboard = m.material("Cardboard", (186, 140, 88), roughness=0.95, density=CARDBOARD)
        self.tape = m.material("Packing tape", (214, 190, 140), roughness=0.5, density=CARDBOARD)
        self.pine = m.material("Pallet wood", (205, 160, 104), roughness=0.9, density=WOOD)
        self.cabinet_grey = m.material("Cabinet grey", (204, 208, 210), metalness=0.2, roughness=0.45, density=STEEL)
        self.concrete = m.material("Concrete floor", (172, 174, 178), roughness=0.95, density=2400)
        self.black_mark = m.material("Floor black", (30, 30, 32), roughness=0.8, density=1000)
        self.red = m.material("Signal red", (210, 24, 24), metalness=0.1, roughness=0.4, density=1200)
        self.screen = self.glow("HMI screen", (40, 120, 100), 0.5)
        self.led_green = self.glow("LED green", (60, 255, 110), 2.5)
        self.led_red = self.glow("LED red", (255, 50, 40), 2.5)
        self.led_amber = self.glow("LED amber", (255, 170, 20), 2.5)

    def glow(self, name, color, emission):
        r = self.c.execute({"type": "add_material", "name": name, "color": list(color), "emission": emission,
                            "density": 1100, "existing": "update"})
        return r["changes"]["materials"][0]

    def make_bolt(self):
        """The bolt head every bolt circle places: a washer and a hex head, built along +Z."""
        m = self.m
        washer = m.cylinder((0, 0, 0), 0.0105, 0.0015, segments=14, material=self.zinc)
        head = m.prism((0, 0, 0.0015), 0.0078, 0.0055, sides=6, material=self.steel)
        self.bolt = m.component([washer, head], "Bolt head")
        self.originals.append(self.bolt)

    # servo motor ----------------------------------------------------

    def motor(self, origin, axis, r, length, name, hint=(0, 0, 1)):
        """A finned servo motor: mounting flange at ``origin``, body along ``axis``, a radial array of
        fins, an end cap, an encoder and a connector. Built along +Z then moved into place."""
        m = self.m
        L = length
        parts = [m.cylinder((0, 0, 0), r * 1.22, 0.014, segments=self.seg, material=self.casting),
                 m.cylinder((0, 0, 0.014), r, L - 0.054, segments=self.seg, material=self.motor_black)]
        fin = m.box((r - 0.004, -0.0025, 0.036), (0.017, 0.005, L - 0.082), material=self.motor_black)
        parts += [fin] + fin.polar((0, 0, 1), 360.0 / self.fins, self.fins - 1)
        parts.append(m.cylinder((0, 0, L - 0.04), r * 0.86, 0.02, segments=self.seg, material=self.casting))
        parts.append(m.cylinder((0, 0, L - 0.02), r * 0.52, 0.02, segments=self.seg, material=self.motor_black))
        parts.append(m.cylinder((r * 0.5, 0, L - 0.03), 0.0095, 0.03, axis=(1, 0, 0), segments=12, material=self.steel))
        g = m.group(parts, name=name)
        g.set_transform(Transform.look(origin, axis, hint))
        return g

    # the robot -------------------------------------------------------

    def base(self):
        m, Z = self.m, (0, 0, 1)
        plate = m.box((-0.29, -0.29, 0), (0.58, 0.58, 0.022), material=self.casting)
        plate.fillet(plate.edges(lambda a, b: abs(a[2] - b[2]) > 1e-6), 0.04)
        body = self.lathe((0, 0, 0), Z, [(0.022, 0, 0), (0.022, 0.258, 0.004), (0.056, 0.258, 0.006), (0.056, 0.215, 0),
                                          (0.10, 0.2, 0.04), (0.22, 0.185, 0.04), (0.265, 0.2, 0.03), (J1_Z, 0.208, 0.012),
                                          (J1_Z, 0, 0)], self.casting)
        body = self.cavity_cyl(body, (0, 0, 0.06), (0, 0, 1), 0.17, 0.225)
        parts = [plate, body]
        # connector pad on the rear with two round connectors
        parts.append(m.box((-0.228, -0.08, 0.12), (0.05, 0.16, 0.13), material=self.casting))
        for y in (-0.04, 0.04):
            parts.append(m.cylinder((-0.228, y, 0.185), 0.022, 0.03, axis=(-1, 0, 0), segments=20, material=self.steel))
            parts.append(m.cylinder((-0.258, y, 0.185), 0.015, 0.02, axis=(-1, 0, 0), segments=16, material=self.motor_black))
        # a bolt circle round the flange and a bolt at each corner of the plate
        parts += self.bolt_ring((0, 0, 0.056), Z, 0.236, 8 if self.quick else 16, 1.0)
        parts += self.bolt_at([(sx * 0.245, sy * 0.245, 0.022) for sx in (-1, 1) for sy in (-1, 1)], Z, 1.8)
        self.keep("base", m.group(parts, name="Robot base"))

    def turret(self):
        m, Z, Y = self.m, (0, 0, 1), (0, 1, 0)
        sx, sz = SHOULDER[0], SHOULDER[2]
        ring = self.lathe((0, 0, J1_Z), Z, [(0, 0.162), (0.018, 0.162), (0.018, 0.206), (0, 0.206)], self.steel)
        hub = self.lathe((0, 0, 0), Z, [(0.318, 0, 0), (0.318, 0.206, 0), (0.335, 0.209, 0.01), (0.36, 0.2, 0.02),
                                        (0.42, 0.186, 0.04), (0.44, 0.17, 0.0), (0.44, 0, 0)], self.orange)
        hub = self.cavity_cyl(hub, (0, 0, 0.332), Z, 0.155, 0.1)
        block = m.box((-0.17, -0.17, 0.40), (0.39, 0.34, 0.06), material=self.orange)
        block.fillet(block.edges(lambda a, b: abs(a[2] - b[2]) > 1e-6), 0.05)
        block = m.subtract(block, [m.box((-0.15, -0.15, 0.412), (0.35, 0.3, 0.036))])
        parts = [ring, hub, block]
        for y0, y1 in ((0.10, 0.165), (-0.165, -0.10)):
            lug, _ = self.hull_slab([(sx, sz, 0.125)], y0, y1, self.orange, 0.012, 0.012, extra=[(-0.17, 0.40), (0.22, 0.40)],
                                    extra_in=[(-0.15, 0.415), (0.20, 0.415)])
            parts.append(lug)
        # J2 gearbox cover with a bolt circle on +Y, a bearing cap on -Y, and the J2 servo behind
        parts.append(self.lathe((sx, 0.165, sz), Y, [(0, 0), (0, 0.1, 0), (0.026, 0.1, 0.008), (0.026, 0)], self.casting))
        parts.append(self.cyl_y(0.191, 0.2, sx, sz, 0.042, self.steel))
        parts += self.bolt_ring((sx, 0.191, sz), Y, 0.078, 10, 1.0, start=(1, 0, 0))
        parts.append(self.lathe((sx, -0.165, sz), (0, -1, 0), [(0, 0), (0, 0.088, 0), (0.018, 0.088, 0.006), (0.018, 0)], self.casting))
        parts.append(self.cyl_y(-0.195, -0.183, sx, sz, 0.04, self.steel))
        parts += self.bolt_ring((sx, -0.183, sz), (0, -1, 0), 0.066, 8, 0.9, start=(1, 0, 0))
        parts.append(self.motor((-0.04, -0.165, 0.50), (0, -1, 0), 0.058, 0.24, "J2 servo", hint=(1, 0, 0)))
        # a name plate and an eye bolt on the block
        parts.append(m.box((0.221, -0.05, 0.415), (0.004, 0.1, 0.03), material=self.steel))
        parts += self.bolt_at([(-0.1, -0.1, 0.46), (-0.1, 0.1, 0.46)], Z, 1.4)
        self.keep("turret", m.group(parts, name="Turret"))

    def upper_arm(self):
        m, Y = self.m, (0, 1, 0)
        sx, sz = SHOULDER[0], SHOULDER[2]
        ex, ez = ELBOW[0], ELBOW[2]
        main, self.arm_outline = self.hull_slab([(sx, sz, 0.115), (sx, 1.0, 0.098), (0.065, 0.86, 0.12)], -0.098, 0.098,
                                                self.orange, 0.014, 0.014, thin=0.014)
        parts = [main]
        for y0, y1 in ((0.045, 0.098), (-0.098, -0.045)):
            cheek, _ = self.hull_slab([(sx, 1.0, 0.098), (ex, ez, 0.098)], y0, y1, self.orange, 0.009, 0.01, thin=0.01)
            parts.append(cheek)
        # J3 gearbox cover and bolt circle (+Y), bearing cap (-Y), the J3 servo on the -Y face
        parts.append(self.lathe((ex, 0.098, ez), Y, [(0, 0), (0, 0.105, 0), (0.03, 0.105, 0.008), (0.03, 0)], self.casting))
        parts.append(self.cyl_y(0.128, 0.138, ex, ez, 0.045, self.steel))
        parts += self.bolt_ring((ex, 0.128, ez), Y, 0.085, 12, 1.0, start=(1, 0, 0))
        parts.append(self.lathe((ex, -0.098, ez), (0, -1, 0), [(0, 0), (0, 0.092, 0), (0.022, 0.092, 0.006), (0.022, 0)], self.casting))
        parts.append(self.cyl_y(-0.14, -0.12, ex, ez, 0.04, self.steel))
        parts += self.bolt_ring((ex, -0.12, ez), (0, -1, 0), 0.07, 8, 0.9, start=(1, 0, 0))
        parts.append(self.motor((0.02, -0.098, 0.92), (0, -1, 0), 0.052, 0.22, "J3 servo", hint=(1, 0, 0)))
        # the pivot bosses where the arm turns in the turret
        parts.append(self.cyl_y(-0.1, -0.098, sx, sz, 0.07, self.steel))
        parts.append(self.cyl_y(0.098, 0.1, sx, sz, 0.07, self.steel))
        self.arm_parts = parts

    def finish_arm(self):
        self.keep("upper_arm", self.m.group(self.arm_parts + self.arm_extra, name="Upper arm"))
        self.keep("forearm", self.m.group(self.fore_parts + self.fore_extra, name="Forearm"))

    def forearm(self):
        m, Y = self.m, (0, 1, 0)
        ex, ez = ELBOW[0], ELBOW[2]
        neck, _ = self.hull_slab([(ex, ez, 0.095), (0.34, ez, 0.075)], -0.04, 0.04, self.orange, 0.008, 0.012)
        main, _ = self.hull_slab([(0.34, ez, 0.082), (0.60, ez, 0.068)], -0.085, 0.085, self.orange, 0.012, 0.012)
        parts = [neck, main]
        parts.append(self.cyl_y(-0.0405, 0.0405, ex, ez, 0.058, self.steel))
        # J4 gear housing at the wrist end of the forearm, with a bolt circle on its face
        parts.append(self.lathe((0.60, 0, ez), (1, 0, 0), [(0, 0), (0, 0.066, 0), (0.02, 0.075, 0.01), (0.1, 0.075, 0.01),
                                                          (0.12, 0.072, 0.004), (0.12, 0)], self.casting))
        # the J4 servo rides on top of the forearm
        parts.append(self.motor((0.36, 0, ez + 0.135), (1, 0, 0), 0.05, 0.24, "J4 servo", hint=(0, 1, 0)))
        parts.append(m.box((0.40, -0.05, ez + 0.07), (0.16, 0.1, 0.012), material=self.casting))
        self.fore_parts = parts

    def wrist(self):
        """Wrist roll (J4), pitch (J5) and flange (J6): three links."""
        m, Y, X = self.m, (0, 1, 0), (1, 0, 0)
        wx, wz = WRIST[0], WRIST[2]
        # link 4: the roll cylinder with a fork
        tube = self.lathe((J4_X, 0, wz), X, [(0, 0), (0, 0.064, 0), (0.08, 0.062, 0.005), (0.08, 0)], self.orange)
        parts = [tube]
        for y0, y1 in ((0.05, 0.088), (-0.088, -0.05)):
            cheek, _ = self.hull_slab([(wx, wz, 0.078)], y0, y1, self.orange, 0.008, 0.009, thin=0.009,
                                      extra=[(0.76, wz + 0.062), (0.76, wz - 0.062)],
                                      extra_in=[(0.77, wz + 0.053), (0.77, wz - 0.053)])
            parts.append(cheek)
        parts.append(self.lathe((wx, 0.088, wz), Y, [(0, 0), (0, 0.06, 0), (0.016, 0.06, 0.006), (0.016, 0)], self.casting))
        parts.append(self.cyl_y(0.104, 0.108, wx, wz, 0.028, self.steel))
        parts += self.bolt_ring((wx, 0.104, wz), Y, 0.046, 8, 0.8, start=(1, 0, 0))
        parts.append(self.lathe((wx, -0.088, wz), (0, -1, 0), [(0, 0), (0, 0.055, 0), (0.012, 0.055, 0.005), (0.012, 0)], self.casting))
        parts.append(self.motor((0.77, -0.088 - 0.012, wz), (0, -1, 0), 0.036, 0.12, "J5 servo", hint=(1, 0, 0)))
        self.keep("wrist_roll", m.group(parts, name="Wrist roll"))
        # link 5: the wrist head between the fork's cheeks, with its nose and the J6 servo
        head = [self.cyl_y(-0.044, 0.044, wx, wz, 0.065, self.orange),
                self.lathe((wx, 0, wz), X, [(0.02, 0, 0), (0.02, 0.047, 0), (0.10, 0.047, 0.006), (0.10, 0.052, 0),
                                            (0.115, 0.052, 0), (0.115, 0)], self.orange),
                self.motor((wx + 0.065, 0, wz + 0.04), (0, 0, 1), 0.034, 0.1, "J6 servo", hint=(1, 0, 0))]
        head.append(self.cyl_y(-0.0445, 0.0445, wx, wz, 0.03, self.steel))
        self.keep("wrist_head", m.group(head, name="Wrist head"))
        # link 6: the tool plate and the parallel gripper's body
        fx, z0 = FLANGE_X, wz
        plate = self.lathe((fx, 0, z0), X, [(0, 0), (0, 0.056, 0), (0.02, 0.056, 0.003), (0.02, 0)], self.steel)
        adapter = self.lathe((fx + 0.02, 0, z0), X, [(0, 0), (0, 0.05, 0), (0.02, 0.05, 0.004), (0.02, 0)], self.casting)
        body = m.box((fx + 0.04, -0.185, z0 - 0.042), (0.09, 0.37, 0.084), material=self.graphite)
        body.fillet(body.edges(lambda a, b: abs(a[0] - b[0]) > 1e-6), 0.012)
        g = [plate, adapter, body]
        for z in (0.024, -0.036):
            g.append(m.box((fx + 0.13, -0.18, z0 + z), (0.01, 0.36, 0.012), material=self.steel))
        for s in (-1, 1):
            g.append(m.cylinder((fx + 0.085, s * 0.185, z0), 0.012, 0.016, axis=(0, s, 0), segments=16, material=self.steel))
            g.append(m.cylinder((fx + 0.085, s * 0.201, z0), 0.0075, 0.012, axis=(0, s, 0), segments=12, material=self.steel))
            g.append(m.cylinder((fx + 0.07, s * 0.07, z0 + 0.042), 0.007, 0.02, axis=(0, 0, 1), segments=12, material=self.steel))
        g.append(m.box((fx + 0.1, -0.02, z0 + 0.042), (0.016, 0.04, 0.003), material=self.led_green))
        self.keep("tool", m.group(g, name="Gripper"))
        # the two fingers, each its own part so each can slide
        self.fingers = []
        for s in (1, -1):
            f = [m.box((fx + 0.14, s * 0.07 - (0.012 if s < 0 else 0), z0 - 0.035), (0.145, 0.012, 0.07), material=self.steel),
                 m.box((fx + 0.14, s * 0.082 if s > 0 else s * 0.115, z0 - 0.045), (0.06, 0.033, 0.09), material=self.graphite),
                 m.box((fx + 0.2, s * 0.058 - (0.012 if s < 0 else 0), z0 - 0.032), (0.085, 0.012, 0.064), material=self.rubber)]
            f += self.bolt_at([(fx + 0.2, s * 0.1, z0 + z) for z in (-0.028, 0.028)], (1, 0, 0), 0.8)
            self.fingers.append(m.group(f, name="Finger %s" % ("+Y" if s > 0 else "-Y")))

    def rig(self):
        """Joints J1 to J6 and the gripper's two prismatic fingers, coupled. Every part stands in
        the zero pose, so each joint is met as it is made and every value starts at 0."""
        m, P = self.m, self.parts
        wz = WRIST[2]
        j = {}
        j["J1"] = m.joint("revolute", None, P["turret"], (0, 0, J1_Z), axis=(0, 0, 1), name="J1 turret", limits=LIMITS["J1"])
        j["J2"] = m.joint("revolute", P["turret"], P["upper_arm"], SHOULDER, axis=(0, 1, 0), name="J2 shoulder", limits=LIMITS["J2"])
        j["J3"] = m.joint("revolute", P["upper_arm"], P["forearm"], ELBOW, axis=(0, 1, 0), name="J3 elbow", limits=LIMITS["J3"])
        j["J4"] = m.joint("revolute", P["forearm"], P["wrist_roll"], (J4_X, 0, wz), axis=(1, 0, 0), name="J4 forearm roll", limits=LIMITS["J4"])
        j["J5"] = m.joint("revolute", P["wrist_roll"], P["wrist_head"], WRIST, axis=(0, 1, 0), name="J5 wrist pitch", limits=LIMITS["J5"])
        j["J6"] = m.joint("revolute", P["wrist_head"], P["tool"], (FLANGE_X, 0, wz), axis=(1, 0, 0), name="J6 tool roll", limits=LIMITS["J6"])
        for s, f, name in ((1, self.fingers[0], "G1 finger +Y"), (-1, self.fingers[1], "G2 finger -Y")):
            j[name[:2]] = m.joint("prismatic", P["tool"], f, (FLANGE_X + 0.17, s * 0.09, wz), axis=(0, s, 0), name=name,
                                  limits=(0, FINGER_MAX))
        m.couple(j["G1"], j["G2"], 1.0)
        self.joints = j
        self.arm = [P[k] for k in ("turret", "upper_arm", "forearm", "wrist_roll", "wrist_head", "tool")] + self.fingers

    def tidy(self):
        self.c.execute({"type": "erase", "ids": [o.id for o in self.originals]})
        self.originals = []

    # cable dress packs ------------------------------------------------

    def cable_pack(self, paths, radii, mats):
        """Sweeps (``follow_me`` of a circle along each path, bends smoothed) grouped as one dress pack."""
        m = self.m
        sweeps = [m.sweep(p, radius=r, segments=10 if self.quick else 12, smooth=True, material=mat)
                  for p, r, mat in zip(paths, radii, mats)]
        return sweeps

    def arm_cables(self):
        m = self.m
        ex, ez = ELBOW[0], ELBOW[2]
        # upper arm: three cables along the rear spine of the casting, up to the back of the elbow
        rear = sorted((p for p in self.arm_outline if p[1] > 0.66 and p[1] < 1.0 and p[0] < 0.11), key=lambda p: p[1])
        spine = [(x - 0.02, z) for x, z in rear[::2]]
        spine = [(spine[0][0] + 0.035, 0.64)] + spine + [(0.0, 1.06), (0.01, 1.16), (0.04, 1.2)]
        paths = []
        for y in (-0.024, 0.0, 0.024):
            paths.append([(x, y, z) for x, z in spine])
        packs = self.cable_pack(paths, (0.011, 0.0095, 0.011), (self.cable, self.cable_grey, self.cable))
        extra = list(packs)
        for x, z in (spine[len(spine) // 3], spine[2 * len(spine) // 3]):
            extra.append(m.box((x - 0.022, -0.045, z - 0.02), (0.044, 0.09, 0.04), material=self.casting))
        self.arm_extra = extra
        # forearm: two cables along the -Y face into the wrist housing
        zc = ez + 0.03
        paths = [[(0.27, -0.105, zc), (0.33, -0.105, zc), (0.58, -0.105, zc), (0.63, -0.085, zc), (0.66, -0.06, zc)],
                 [(0.27, -0.105, zc - 0.026), (0.33, -0.105, zc - 0.026), (0.58, -0.105, zc - 0.026), (0.63, -0.085, zc - 0.026), (0.66, -0.06, zc - 0.026)]]
        packs = self.cable_pack(paths, (0.011, 0.0095), (self.cable, self.cable_grey))
        packs += [m.box((x, -0.115, zc - 0.045), (0.04, 0.03, 0.09), material=self.casting) for x in (0.34, 0.50)]
        self.fore_extra = packs

    # the cell -------------------------------------------------------

    def floor(self):
        m = self.m
        slab = m.box((-3.4, -3.2, -0.04), (6.8, 6.4, 0.04), material=self.concrete, name="Floor")
        self.keep("floor", slab)
        yel, blk = self.yellow, self.black_mark
        mk = []
        # cell outline 0.22 m inside the fence, a dashed reach circle round the robot
        x0, x1, y0, y1 = -1.23, 1.88, -1.43, 1.68
        w = 0.07
        for corner, size in (((x0, y0, 0), (x1 - x0, w, 0.003)), ((x0, y1 - w, 0), (x1 - x0, w, 0.003)),
                             ((x0, y0, 0), (w, y1 - y0, 0.003)), ((x1 - w, y0, 0), (w, y1 - y0, 0.003))):
            mk.append(m.box(corner, size, material=yel))
        n = 36
        dash = m.box((1.5, -0.04, 0), (0.22, 0.08, 0.003), material=yel)
        mk += [dash] + dash.polar((0, 0, 1), 360.0 / n, n - 1)
        # hazard blocks across the conveyor's entry
        blocks = 8 if self.quick else 14
        a = m.box((0.3, -1.95, 0), (0.1, 0.22, 0.003), material=yel)
        b = m.box((0.4, -1.95, 0), (0.1, 0.22, 0.003), material=blk)
        mk += [a] + a.copy((0.2, 0, 0), blocks // 2 - 1) + [b] + b.copy((0.2, 0, 0), blocks // 2 - 1)
        # the robot's square pad
        mk.append(m.box((-0.36, -0.36, 0), (0.72, 0.72, 0.003), material=blk))
        self.keep("markings", m.group(mk, name="Floor markings"))

    def carton(self):
        """The cardboard box component, built with its bottom centre at the origin."""
        m = self.m
        w, l, h = BOX
        body = m.box((-w / 2, -l / 2, 0), (w, l, h), material=self.cardboard)
        body.fillet(body.edges(lambda a, b: abs(a[2] - b[2]) > 1e-6), 0.004)
        tape = m.box((-0.0125, -l / 2, h), (0.025, l, 0.0015), material=self.tape)
        lab = [m.box((-0.06, s * l / 2 - (0.0 if s > 0 else 0.0015), 0.09), (0.12, 0.0015, 0.08), material=self.paper) for s in (1, -1)]
        self.carton_def = m.component([body, tape] + lab, "Carton")
        self.originals.append(self.carton_def)

    def place_cartons(self, spots, z, name):
        ts = [Transform.translate(x, y, z) for x, y in spots]
        return self.m.group(self.place_many(self.carton_def, ts), name=name)

    def conveyor(self):
        m = self.m
        cx = CONVEYOR_X
        xa, xb = cx - 0.30, cx + 0.30  # between the side rails
        y0, y1 = -2.7, -0.5
        parts = []
        for x in (xa - 0.04, xb):
            rail = m.box((x, y0, 0.36), (0.04, y1 - y0, 0.175), material=self.conveyor_blue)
            rail.fillet(rail.edges(lambda a, b: abs(a[1] - b[1]) > 1e-6), 0.006)
            parts.append(rail)
        # rollers copied along the frame, with an end cap at each rail
        roller = m.cylinder((xa, -0.6, 0.525), 0.025, xb - xa, axis=(1, 0, 0), segments=24, material=self.zinc_plate)
        count = 8 if self.quick else 21
        parts += [roller] + roller.copy((0, -0.1, 0), count - 1)
        cap = m.cylinder((xa - 0.046, -0.6, 0.525), 0.011, 0.006, axis=(1, 0, 0), segments=10, material=self.steel)
        cap2 = m.cylinder((xb + 0.04, -0.6, 0.525), 0.011, 0.006, axis=(1, 0, 0), segments=10, material=self.steel)
        for cp in (cap, cap2):
            parts += [cp] + cp.copy((0, -0.1, 0), count - 1)
        # cross members, legs with feet and braces
        for k in range(5):
            y = y1 - 0.2 - k * 0.5
            parts.append(m.box((xa, y - 0.02, 0.40), (xb - xa, 0.04, 0.03), material=self.conveyor_blue))
        for k in range(4):
            y = y1 - 0.3 - k * 0.65
            for x in (xa - 0.035, xb + 0.005):
                parts.append(m.box((x, y - 0.025, 0.03), (0.03, 0.05, 0.33), material=self.conveyor_blue))
                parts.append(m.box((x - 0.03, y - 0.05, 0.0), (0.09, 0.1, 0.03), material=self.casting))
            parts.append(m.box((xa, y - 0.015, 0.16), (xb - xa, 0.03, 0.03), material=self.conveyor_blue))
        # the stop where the boxes wait to be picked
        parts.append(m.box((xa, -0.55, 0.50), (xb - xa, 0.02, 0.1), material=self.yellow))
        # the drive: a geared servo on the far end
        parts.append(self.motor((xb + 0.04, y0 + 0.2, 0.45), (1, 0, 0), 0.05, 0.2, "Conveyor drive", hint=(0, 0, 1)))
        self.keep("conveyor", m.group(parts, name="Roller conveyor"))

    def pallet(self):
        m = self.m
        px, py = PALLET_C
        x0, y0 = px - 0.6, py - 0.4
        parts = []
        top = m.box((x0, y0, 0.122), (0.12, 0.8, 0.022), material=self.pine)
        parts += [top] + top.copy((0.15, 0, 0), 7)
        bottom = m.box((x0, y0, 0.0), (0.14, 0.8, 0.022), material=self.pine)
        parts += [bottom] + bottom.copy((0.53, 0, 0), 2)
        bearer = m.box((x0, y0, 0.022), (1.2, 0.1, 0.1), material=self.pine)
        parts += [bearer] + bearer.copy((0, 0.35, 0), 2)
        self.keep("pallet", m.group(parts, name="Pallet"))

    def fence_panel(self, w):
        """A mesh panel of width ``w`` as a component: a yellow frame, vertical and horizontal wires copied."""
        m = self.m
        key = round(w, 3)
        if key in self.panels:
            return self.panels[key]
        parts = [m.box((0, -0.02, 1.86), (w, 0.04, 0.04), material=self.yellow),
                 m.box((0, -0.02, 0.10), (w, 0.04, 0.04), material=self.yellow),
                 m.box((0, -0.015, 0.14), (0.03, 0.03, 1.72), material=self.yellow),
                 m.box((w - 0.03, -0.015, 0.14), (0.03, 0.03, 1.72), material=self.yellow)]
        nv = int((w - 0.08) / 0.06) + 1
        v = m.box((0.04, -0.0025, 0.14), (0.005, 0.005, 1.72), material=self.mesh)
        parts += [v] + v.copy(((w - 0.09) / (nv - 1), 0, 0), nv - 1)
        h = m.box((0.03, 0.0025, 0.18), (w - 0.06, 0.005, 0.005), material=self.mesh)
        parts += [h] + h.copy((0, 0, 0.12), 13)
        comp = m.component(parts, "Fence panel %.2f m" % w)
        self.originals.append(comp)
        self.panels[key] = comp
        return comp

    def fence_post(self):
        m = self.m
        parts = [m.box((-0.03, -0.03, 0.012), (0.06, 0.06, 1.89), material=self.yellow),
                 m.box((-0.07, -0.07, 0), (0.14, 0.14, 0.012), material=self.casting),
                 m.box((-0.035, -0.035, 1.9), (0.07, 0.07, 0.006), material=self.casting)]
        parts += self.bolt_at([(sx * 0.05, sy * 0.05, 0.012) for sx in (-1, 1) for sy in (-1, 1)], (0, 0, 1), 1.3)
        comp = m.component(parts, "Fence post")
        self.originals.append(comp)
        return comp

    def fence(self):
        m = self.m
        post = self.fence_post()
        self.panels = {}
        # the front, where the conveyor comes in, is left open (the light curtain guards the conveyor)
        runs = [((-1.45, -1.65), (-1.45, 1.9)), ((-1.45, 1.9), (2.1, 1.9)), ((2.1, 1.9), (2.1, -1.65))]
        panel_ts, post_pts = [], {}
        counts = {}
        for (ax, ay), (bx, by) in runs:
            d = (bx - ax, by - ay)
            length = math.hypot(*d)
            n = max(1, math.ceil(length / 1.25 - 1e-6))
            w = length / n
            u = (d[0] / length, d[1] / length, 0.0)
            comp = self.fence_panel(w)
            for k in range(n):
                p = (ax + u[0] * w * k, ay + u[1] * w * k, 0.0)
                panel_ts.append((comp, Transform.look(p, (0, 0, 1), u)))
            for k in range(n + 1):
                post_pts[(round(ax + u[0] * w * k, 3), round(ay + u[1] * w * k, 3))] = True
        placed = []
        for comp in {id(c): c for c, _ in panel_ts}.values():
            placed += self.place_many(comp, [t for c, t in panel_ts if c is comp])
        placed += self.place_many(post, [Transform.translate(x, y, 0) for x, y in post_pts])
        self.keep("fence", m.group(placed, name="Safety fence"))

    def light_curtain(self):
        m = self.m
        y = -1.65
        parts = []
        for x, s in ((0.45, 1), (1.55, -1)):
            parts.append(m.box((x - 0.025, y - 0.025, 0.0), (0.05, 0.05, 1.12), material=self.motor_black))
            parts.append(m.box((x - 0.03, y - 0.03, 1.12), (0.06, 0.06, 0.04), material=self.yellow))
            parts.append(m.box((x - 0.06, y - 0.06, 0), (0.12, 0.12, 0.01), material=self.casting))
            strip = m.box((x + (0.025 if s > 0 else -0.027), y - 0.01, 0.12), (0.002, 0.02, 0.9), material=self.led_red)
            parts.append(strip)
            parts.append(m.box((x - 0.012, y + (s * 0.0) - 0.012, 1.16), (0.024, 0.024, 0.01), material=self.led_green))
        self.keep("curtain", m.group(parts, name="Light curtain"))
        # the protective field between the two posts, as a see-through red sheet
        self.field_mat = self.c.execute({"type": "add_material", "name": "Protective field", "color": [255, 60, 50],
                                         "opacity": 0.16, "emission": 0.0, "density": 1.2, "existing": "update"})["changes"]["materials"][0]
        fld = m.box((0.48, y - 0.002, 0.12), (1.04, 0.004, 0.9), material=self.field_mat)
        self.keep("field", fld)

    def cabinet(self):
        m = self.m
        x0, y0, w, d, h = -2.5, -0.6, 0.8, 0.4, 1.8
        z0 = 0.1
        parts = [m.box((x0, y0, 0), (w, d, z0), material=self.casting)]
        body = m.box((x0, y0, z0), (w, d, h), material=self.cabinet_grey)
        body.fillet(body.edges(lambda a, b: abs(a[2] - b[2]) < 1e-6 and a[2] > z0 + h - 1e-6), 0.01)
        parts.append(body)
        fy = y0 - 0.006  # the door face
        parts.append(m.box((x0 + 0.02, fy, z0 + 0.02), (w - 0.04, 0.006, h - 0.04), material=self.cabinet_grey))
        # vents low on the door, a handle bar, a screen, push buttons, an emergency stop
        vent = m.box((x0 + 0.1, fy - 0.003, z0 + 0.12), (w - 0.2, 0.003, 0.014), material=self.motor_black)
        parts += [vent] + vent.copy((0, 0, 0.035), 5)
        parts.append(m.cylinder((x0 + w - 0.07, fy - 0.01, z0 + 0.7), 0.009, 0.7, axis=(0, 0, 1), segments=12, material=self.steel))
        parts.append(m.box((x0 + w - 0.085, fy - 0.025, z0 + 0.62), (0.03, 0.025, 0.04), material=self.steel))
        parts.append(m.box((x0 + 0.12, fy - 0.004, z0 + 1.2), (0.3, 0.004, 0.2), material=self.motor_black))
        parts.append(m.box((x0 + 0.13, fy - 0.0055, z0 + 1.21), (0.28, 0.0015, 0.18), material=self.screen))
        for k, mat in enumerate((self.led_green, self.led_amber, self.red)):
            parts.append(m.cylinder((x0 + 0.14 + k * 0.07, fy, z0 + 1.0), 0.015, 0.012, axis=(0, -1, 0), segments=16, material=mat))
        parts.append(m.cylinder((x0 + 0.55, fy, z0 + 1.0), 0.035, 0.006, axis=(0, -1, 0), segments=20, material=self.yellow))
        parts.append(m.cylinder((x0 + 0.55, fy - 0.006, z0 + 1.0), 0.022, 0.025, axis=(0, -1, 0), segments=20, material=self.red))
        # a signal tower on the roof
        for k, mat in enumerate((self.led_green, self.led_amber, self.red)):
            parts.append(m.cylinder((x0 + 0.2, y0 + 0.2, z0 + h + 0.0 + k * 0.05), 0.03, 0.05, segments=20, material=mat))
        self.keep("cabinet", m.group(parts, name="Control cabinet"))
        self.cabinet_box = (x0, y0, w, d, h)

    def power_cables(self):
        m = self.m
        x0, y0, w, d, h = self.cabinet_box
        mats = (self.cable, self.cable_grey)
        paths = []
        for y, yb in ((-0.04, -0.04), (0.04, 0.04)):
            paths.append([(x0 + w, y0 + 0.2 + 0.1 * (1 if y > 0 else -1), 0.45), (x0 + w + 0.12, y0 + 0.2 + 0.1 * (1 if y > 0 else -1), 0.45),
                          (x0 + w + 0.18, y, 0.06), (-0.7, y, 0.045), (-0.38, y, 0.12), (-0.3, y, 0.185),
                          (-0.28, y, 0.185)])
        sw = self.cable_pack(paths, (0.016, 0.014), mats)
        self.keep("cables", m.group(sw, name="Cell cables"))

    def look(self):
        m = self.m
        m.lighting("studio")
        self.c.execute({"type": "set_shadows", "on": True})
        m.style("Cell", background=[214, 218, 224], sky=[246, 248, 251],
                edges={"color": [30, 30, 36], "profiles": 2}, ambient_occlusion={"on": True, "radius": 0.04, "strength": 1.3})
        self.c.execute({"type": "set_camera", "eye": HERO["eye"], "target": HERO["target"], "fov_y_deg": HERO["fov_y_deg"]})

    # motion ---------------------------------------------------------

    def pose_transforms(self, q, grip):
        """Each moving part's transform for joint angles ``q`` (degrees) and gripper opening ``grip``
        (meters): the chain of rotations about the joint axes, drawn where the parts stand at zero."""
        wz = WRIST[2]
        X, Y, Z = (1, 0, 0), (0, 1, 0), (0, 0, 1)
        t1 = Transform.axis_angle(Z, q["J1"], (0, 0, 0))
        t2 = t1 @ Transform.axis_angle(Y, q["J2"], SHOULDER)
        t3 = t2 @ Transform.axis_angle(Y, q["J3"], ELBOW)
        t4 = t3 @ Transform.axis_angle(X, q["J4"], (J4_X, 0, wz))
        t5 = t4 @ Transform.axis_angle(Y, q["J5"], WRIST)
        t6 = t5 @ Transform.axis_angle(X, q["J6"], (FLANGE_X, 0, wz))
        P = self.parts
        return [(P["turret"], t1), (P["upper_arm"], t2), (P["forearm"], t3), (P["wrist_roll"], t4),
                (P["wrist_head"], t5), (P["tool"], t6),
                (self.fingers[0], t6 @ Transform.translate(0, grip, 0)),
                (self.fingers[1], t6 @ Transform.translate(0, -grip, 0))]

    def set_pose(self, q, grip=0.0, extra=()):
        """Pose the arm in one batch of ``set_transform``. (Driving the joints one at a time does not
        hold the others: a drive leaves every other joint free, and the solver spends the motion
        wherever it is cheapest, so a whole pose is set from the joint chain instead.) ``extra``
        is more ``(instance, transform)`` pairs to set in the same step."""
        cmds = [{"type": "set_transform", "id": inst.id, "transform": t.to_json()}
                for inst, t in list(self.pose_transforms(q, grip)) + list(extra)]
        self.c.batch(cmds)

    def tcp(self):
        """The tool centre point where the document's arm stands now (read from the tool's transform)."""
        t = self.parts["tool"].transform()
        return t.apply((FLANGE_X + PAD_MID, 0.0, WRIST[2]))

    def waypoint(self, r, theta, z, grip, q6=None):
        return {"r": r, "th": theta, "z": z, "q6": theta + 90.0 if q6 is None else q6, "grip": grip}

    def plan(self, frames=48):  # the legs below add up to 48 frames; fewer are scaled
        """The pick-and-place as ``frames`` poses: dicts with joint values ``q``, the gripper's ``grip``,
        whether the carton is ``held`` and the ``phase`` name. The arm's path is solved in polar
        coordinates of the tool centre point (the tool always straight down) with ``ik``; the first
        and last legs go between the zero pose and the pose above the box in joint space."""
        pr, pt = math.hypot(*PICK), math.degrees(math.atan2(PICK[1], PICK[0]))
        lr, lt = math.hypot(*PLACE), math.degrees(math.atan2(PLACE[1], PLACE[0]))
        z_pick = CONVEYOR_TOP + BOX[2] - 0.05
        z_place = PALLET_H + BOX[2] - 0.05
        self.grasp_z, self.release_z = z_pick, z_place
        open_, hold = 0.045, GRASP
        home = {"J1": 0.0, "J2": 0.0, "J3": 0.0, "J4": 0.0, "J5": 0.0, "J6": 0.0}
        above_pick = self.waypoint(pr, pt, 0.95, open_)
        at_pick = self.waypoint(pr, pt, z_pick, open_)
        gripped = self.waypoint(pr, pt, z_pick, hold)
        lifted = self.waypoint(pr, pt, 0.95, hold)
        above_place = self.waypoint(lr, lt, 0.78, hold)
        at_place = self.waypoint(lr, lt, z_place, hold)
        released = self.waypoint(lr, lt, z_place, open_)
        away = self.waypoint(lr, lt, 0.78, open_)
        # (name, frames, kind, from, to, carton held while it runs)
        legs = [("approach", 8, "joint", (home, open_), above_pick, False),
                ("descend", 5, "cart", above_pick, at_pick, False),
                ("grip", 3, "cart", at_pick, gripped, False),
                ("lift", 4, "cart", gripped, lifted, True),
                ("transfer", 9, "cart", lifted, above_place, True),
                ("lower", 5, "cart", above_place, at_place, True),
                ("release", 3, "cart", at_place, released, False),
                ("withdraw", 3, "cart", released, away, False),
                ("return", 8, "joint", away, (home, open_), False)]
        counts = [max(1, round(n * frames / 48)) for _, n, *_ in legs]
        counts[0] += frames - sum(counts)
        legs = [(name, c, *rest) for (name, _, *rest), c in zip(legs, counts)]

        def ease(t):
            return t * t * (3 - 2 * t)

        def solve(w):
            q = ik(w["r"], w["th"], w["z"], w["q6"])
            if q is None:
                raise RuntimeError("the tool centre point is out of reach at %r" % (w,))
            return q

        out = []
        for name, n, kind, a, b, held in legs:
            for k in range(n):
                t = ease(k / n)
                if kind == "cart":
                    w = {key: a[key] + (b[key] - a[key]) * t for key in a}
                    q, grip = solve(w), w["grip"]
                else:
                    qa, ga = (a if isinstance(a, tuple) else (solve(a), a["grip"]))
                    qb, gb = (b if isinstance(b, tuple) else (solve(b), b["grip"]))
                    q = {key: qa[key] + (qb[key] - qa[key]) * t for key in qa}
                    grip = ga + (gb - ga) * t
                out.append({"q": q, "grip": grip, "held": held, "phase": name})
        return out

    def carton_at(self, x, y, z):
        return Transform.translate(x, y, z)

    def run_motion(self, plan, on_frame):
        """Pose the arm at each frame of ``plan`` in turn; the carried carton follows the gripper while it
        is ``held`` and stays where it was put once let go. ``on_frame(i, frame)`` is called with the
        document at each pose."""
        carried = self.carried
        pick_t = self.carton_at(PICK[0], PICK[1], CONVEYOR_TOP)
        self.set_pose({"J1": 0, "J2": 0, "J3": 0, "J4": 0, "J5": 0, "J6": 0}, 0.045, [(carried, pick_t)])
        off = None
        done = False
        for i, f in enumerate(plan):
            t6 = self.pose_transforms(f["q"], f["grip"])[5][1]
            extra = []
            if f["held"]:
                if off is None:
                    off = t6.inverse() @ pick_t
                extra.append((carried, t6 @ off))
            elif off is not None:
                extra.append((carried, self.carton_at(PLACE[0], PLACE[1], PALLET_H)))
            self.set_pose(f["q"], f["grip"], extra)
            on_frame(i, f)
        return carried

    def scenes(self):
        """Three scenes storing the arm's positions (and the carton's): home, at the pick, at the place."""
        m = self.m
        pr, pt = math.hypot(*PICK), math.degrees(math.atan2(PICK[1], PICK[0]))
        lr, lt = math.hypot(*PLACE), math.degrees(math.atan2(PLACE[1], PLACE[0]))
        pick_t = self.carton_at(PICK[0], PICK[1], CONVEYOR_TOP)
        place_t = self.carton_at(PLACE[0], PLACE[1], PALLET_H)
        zero = {"J1": 0.0, "J2": 0.0, "J3": 0.0, "J4": 0.0, "J5": 0.0, "J6": 0.0}
        stops = [("Home", zero, 0.045, pick_t), ("Pick", ik(pr, pt, self.grasp_z, pt + 90), GRASP, pick_t),
                 ("Place", ik(lr, lt, self.release_z, lt + 90), 0.045, place_t)]
        out = {}
        for name, q, grip, carton in stops:
            self.set_pose(q, grip, [(self.carried, carton)])
            out[name] = m.scene(name, positions=self.arm + [self.carried])
        m.transition(True, 1.5)
        return out

    def check_scenes(self, scenes):
        """Go to the Pick scene and read where the tool centre stands: the scene's positions put it there."""
        self.m.go_to_scene(scenes["Pick"])
        at = self.tcp()
        self.m.go_to_scene(scenes["Home"])
        return math.dist(at, (PICK[0], PICK[1], self.grasp_z))

    def section_picture(self, path, width, height, ss):
        """The arm cut open on the plane through J1, J2 and J3: the castings are hollow shells, the
        cut filled in the materials' colors. The floor and the markings stay whole."""
        m, c = self.m, self.c
        q = ik(1.0, 0.0, 0.85, 90.0)
        self.set_pose(q, 0.045)  # in the plane of the cut
        r = c.execute({"type": "add_section_plane", "point": [0, 0, 0], "normal": [0, 1, 0]})
        m.section_cut([self.parts["floor"], self.parts["markings"]], cut=False)
        m.section_display(planes=False, fill=True, fill_color="dark")
        c.execute({"type": "select", "ids": [], "mode": "replace"})
        m.render(path, width, height, camera=SECTION, lighting=None, supersample=ss, ground=False)
        c.execute({"type": "deactivate_section"})
        m.section_display(planes=True)
        return path

    def hero_labels(self):
        """(part, text, point it points at, dx, dy) for the callouts of the hero image; the points of
        the moving parts are given where they stand at zero and carried to the pose they have now."""
        def at(part, pt):
            return self.parts[part].transform().apply(pt)

        return [("base", "Base, bolt circle", (0.0, -0.25, 0.12), -0.21, 0.07),
                ("turret", "J1 turret, J2 shoulder", at("turret", (0.14, -0.17, 0.6)), -0.2, 0.0),
                ("upper_arm", "Upper arm, J3 servo", at("upper_arm", (0.02, -0.15, 0.95)), -0.2, -0.09),
                ("wrist_head", "Wrist J4 to J6", at("wrist_head", (WRIST[0], 0.0, WRIST[2] + 0.07)), -0.04, -0.24),
                ("tool", "Parallel gripper", at("tool", (FLANGE_X + 0.14, 0.0, WRIST[2] - 0.05)), 0.13, -0.16),
                ("conveyor", "Roller conveyor", (CONVEYOR_X, -1.5, CONVEYOR_TOP), 0.2, 0.05),
                ("pallet", "Pallet", (PALLET_C[0] + 0.55, PALLET_C[1], PALLET_H), 0.2, -0.07),
                ("curtain", "Light curtain", (1.55, -1.65, 1.15), 0.08, -0.13),
                ("fence", "Safety fence", (2.1, 0.4, 1.7), -0.1, -0.06),
                ("cabinet", "Control cabinet", (-2.1, -0.4, 1.5), 0.02, -0.12)]

    def cartons(self):
        """Three boxes waiting on the conveyor, four already on the pallet, one at the pick point
        that the robot carries (its own part)."""
        waiting = [(CONVEYOR_X, PICK[1] - 0.5), (CONVEYOR_X, PICK[1] - 1.0)]
        self.keep("waiting", self.place_cartons(waiting, CONVEYOR_TOP, "Cartons on the conveyor"))
        self.keep("stacked", self.place_cartons(SLOTS, PALLET_H, "Cartons on the pallet"))
        (self.carried,) = self.place_many(self.carton_def, [Transform.translate(PICK[0], PICK[1], CONVEYOR_TOP)])
        self.parts["carried"] = self.carried


def build(m, quick=False, log=print):
    cell = Cell(m, quick, log)
    t0 = time.time()
    steps = [("materials", cell.materials), ("bolts", cell.make_bolt), ("base", cell.base), ("turret", cell.turret),
             ("upper arm", cell.upper_arm), ("forearm", cell.forearm), ("wrist and gripper", cell.wrist),
             ("cable packs", cell.arm_cables), ("assemble arm", cell.finish_arm), ("floor", cell.floor),
             ("conveyor", lambda: (cell.carton(), cell.conveyor())), ("pallet", cell.pallet), ("fence", cell.fence),
             ("light curtain", cell.light_curtain), ("cabinet", lambda: (cell.cabinet(), cell.power_cables())),
             ("cartons", cell.cartons), ("tidy", cell.tidy), ("rig", cell.rig)]
    for name, fn in steps:
        t = time.time()
        fn()
        log("  %-22s %6.1f s" % (name, time.time() - t))
    log("built in %.1f s" % (time.time() - t0))
    return cell


# ---------------------------------------------------------------- checks


class Checks:
    """Solid checks of the rigged cell: each axis through its limits, the swing against the fence,
    every pose of the motion, the arm's mass and its reach. The poses come from ``Cell.set_pose``
    (the whole chain at once); ``interference`` and ``clearance`` read the document at each."""

    ADJACENT = {"J2": ("turret", "upper_arm"), "J3": ("upper_arm", "forearm"), "J4": ("forearm", "wrist_roll"),
                "J5": ("wrist_roll", "wrist_head"), "J6": ("wrist_head", "tool")}
    ENV = ("fence", "conveyor", "pallet", "curtain", "cabinet", "waiting", "stacked", "base")
    TIP = ("forearm", "wrist_head", "tool", "finger +Y", "finger -Y")

    def __init__(self, cell, log=print):
        self.cell = cell
        self.m = cell.m
        self.log = log
        P = cell.parts
        self.part = dict(P)
        self.part["finger +Y"], self.part["finger -Y"] = cell.fingers
        self.name = {v.id: k for k, v in self.part.items() if v is not None}
        self.arm = [P[k] for k in ("turret", "upper_arm", "forearm", "wrist_roll", "wrist_head", "tool")] + cell.fingers
        self.zero = {"J1": 0.0, "J2": 0.0, "J3": 0.0, "J4": 0.0, "J5": 0.0, "J6": 0.0}

    def dist(self, a, b):
        return self.m.clearance(a=self.part[a], b=self.part[b])["pairs"][0]["distance"]

    def overlaps(self, ids):
        r = self.m.interference(ids=ids)
        return [(self.name[p["a"]], self.name[p["b"]], p["volume"]) for p in r["pairs"]]

    # each axis through its limits ------------------------------------

    def axes(self, steps=25):
        """J2 to J6 turned through their limits from the zero pose with the rest at zero (the
        arm against itself), and J1 swung through its limits against the cell with the arm stretched
        out at carrying height."""
        c = self.cell
        out = {}
        arm_ids = [p.id for p in self.arm]
        for ax in ("J2", "J3", "J4", "J5", "J6"):
            lo, hi = LIMITS[ax]
            n = steps if hi - lo < 400 else 49
            hits, best = {}, (9.0, None)
            a, b = self.ADJACENT[ax]
            for k in range(n):
                v = lo + (hi - lo) * k / (n - 1)
                q = dict(self.zero)
                q[ax] = v
                c.set_pose(q, 0.0)
                for na, nb, vol in self.overlaps(arm_ids):
                    hits.setdefault((na, nb), []).append(v)
                d = self.dist(a, b)
                if d < best[0]:
                    best = (d, v)
            out[ax] = {"limits": (lo, hi), "poses": n, "hits": hits, "pair": (a, b), "closest": best}
        c.set_pose(self.zero, 0.0)
        return out

    def swing(self, step=15):
        """J1 through its limits at the arm's carrying pose, tool at the pick radius and 0.95 m high with
        the gripper's long axis along X (the widest it can be): the clearance to every part of the cell."""
        c = self.cell
        lo, hi = LIMITS["J1"]
        r = math.hypot(*PICK)
        q0 = ik(r, 0.0, 0.95, 90.0)
        env = [k for k in self.ENV if k != "base"]
        ids = [p.id for p in self.arm] + [self.part[k].id for k in env]
        near = {k: (9.0, None, None) for k in env}
        hits = []
        for th in range(lo, hi + 1, step):
            q = dict(q0)
            q["J1"], q["J6"] = float(th), th + 90.0
            c.set_pose(q, 0.045)
            hits += [(th,) + h for h in self.overlaps(ids)]
            for k in env:
                for t in self.TIP:
                    d = self.dist(t, k)
                    if d < near[k][0]:
                        near[k] = (d, th, t)
        c.set_pose(self.zero, 0.0)
        return {"limits": (lo, hi), "step": step, "hits": hits, "near": near, "pose": q0}

    # the pick and place ----------------------------------------------

    def motion(self, plan, clearance_every=4, extra=None):
        """Run ``plan`` and, at every pose, look for overlaps between the arm, the carried carton and
        the cell; at every ``clearance_every``-th pose measure the arm's distance to each part."""
        c, m = self.cell, self.m
        env = [k for k in self.ENV if k != "base"]
        ids = [p.id for p in self.arm] + [self.part[k].id for k in self.ENV] + [c.carried.id]
        self.name[c.carried.id] = "carried carton"
        near = {k: (9.0, None, None) for k in env}
        grip = (9.0, None)
        hits = []
        arm_names = ("forearm", "wrist_roll", "wrist_head", "tool", "finger +Y", "finger -Y", "upper_arm")

        def at(i, f):
            nonlocal grip
            hits.extend((i, f["phase"]) + h for h in self.overlaps(ids))
            if i % clearance_every == 0:
                for k in env:
                    for t in arm_names:
                        d = self.dist(t, k)
                        if d < near[k][0]:
                            near[k] = (d, i, t)
            if f["held"]:
                for t in ("finger +Y", "finger -Y"):
                    d = m.clearance(a=self.part[t], b=c.carried)["pairs"][0]["distance"]
                    if d < grip[0]:
                        grip = (d, t)

        c.run_motion(plan, at if extra is None else (lambda i, f: (at(i, f), extra(i, f))))
        return {"frames": len(plan), "hits": hits, "near": near, "grip": grip}

    def gripper(self):
        """``sweep_check`` of the coupled finger joint through its travel: the fingers against the
        gripper body, and the coupling that makes the second finger follow."""
        c = self.cell
        c.set_pose(self.zero, 0.0)
        r = self.m.sweep_check(c.joints["G1"], 0.0, FINGER_MAX, 25, ids=[self.part["tool"]] + c.fingers, report_within=0.005)
        cpl = [k for k in self.m.client.query("couplings")["couplings"]
               if k["driver"] == c.joints["G1"].id and k["driven"] == c.joints["G2"].id]
        return {"pairs": [(self.name[p["a"]], self.name[p["b"]], p.get("volume", 0.0), p["clearance"]) for p in r["pairs"]],
                "steps": r["steps"], "stopped": r.get("stopped"), "coupling": cpl[0] if cpl else None}

    # mass and reach -----------------------------------------------------

    def mass(self):
        m = self.m
        out = {}
        for k in ("turret", "upper_arm", "forearm", "wrist_roll", "wrist_head", "tool"):
            t = m.mass_properties([self.part[k]])["total"]
            out[k] = t
        out["fingers"] = m.mass_properties(self.cell.fingers)["total"]
        out["arm"] = m.mass_properties(self.arm)["total"]
        out["base"] = m.mass_properties([self.part["base"]])["total"]
        out["carton"] = m.mass_properties([self.cell.carried])["total"]
        return out

    def reach(self):
        """The reach, measured from the document with the arm stretched along X (J2 90, J3 -90, J5 0): the
        wrist centre and the tool centre point, and the lowest and highest the tool centre goes
        with the tool down at the pick radius."""
        c = self.cell
        q = dict(self.zero, J2=90.0, J3=-90.0)
        c.set_pose(q, 0.0)
        tcp = c.tcp()
        fk_error = math.dist(tcp, tcp_position(q))
        wrist = self.part["wrist_head"].transform().apply(WRIST)
        c.set_pose(self.zero, 0.0)
        zero_tcp = c.tcp()
        return {"tcp": tcp, "wrist": wrist, "zero_tcp": zero_tcp, "fk_error": fk_error}


# ---------------------------------------------------------------- pictures and report

HERO = {"eye": [0.9, -5.9, 3.4], "target": [-0.05, 0.15, 0.75], "fov_y_deg": 32}
MOTION = {"eye": [0.9, -4.5, 2.8], "target": [0.5, 0.0, 0.7], "fov_y_deg": 30}
SECTION = {"eye": [1.15, -2.9, 1.25], "target": [0.3, 0.0, 0.72], "fov_y_deg": 34}
DETAIL = {"eye": [0.0, -1.85, 1.3], "target": [1.0, -0.72, 0.82], "fov_y_deg": 26}


def camera_basis(cam):
    fwd = unit(sub(tuple(cam["target"]), tuple(cam["eye"])))
    right = unit(cross(fwd, (0, 0, 1)))
    up = cross(right, fwd)
    return fwd, right, up


def project(cam, aspect, p):
    """Where the world point ``p`` falls on the screen of ``cam``: (u, v) from the top left, 0 to 1, and its depth."""
    fwd, right, up = camera_basis(cam)
    d = sub(p, tuple(cam["eye"]))
    depth = dot(d, fwd)
    t = math.tan(math.radians(cam["fov_y_deg"]) / 2)
    return 0.5 + 0.5 * dot(d, right) / (depth * t * aspect), 0.5 - 0.5 * dot(d, up) / (depth * t), depth


def unproject(cam, aspect, u, v, depth):
    fwd, right, up = camera_basis(cam)
    t = math.tan(math.radians(cam["fov_y_deg"]) / 2)
    x, y = (2 * u - 1) * aspect * t * depth, (1 - 2 * v) * t * depth
    return add(add(tuple(cam["eye"]), mul(fwd, depth)), add(mul(right, x), mul(up, y)))


def callouts(cell, cam, aspect, items):
    """Leader labels: each text sits ``(du, dv)`` of the screen's width and height from where the part
    it names is drawn, a little in front of the cell so the leader runs over it. Returns the note ids."""
    ids = []
    for target, text, anchor, du, dv in items:
        u, v, depth = project(cam, aspect, anchor)
        pos = unproject(cam, aspect, u + du, v + dv, depth - 0.3)
        ids.append(cell.m.label(cell.parts[target], text, position=pos))
    return [i for i in ids if i]


def make_gif(frame_dir, out, width=640, fps=20, log=print):
    ffmpeg = shutil.which("ffmpeg")
    if ffmpeg is None:
        return None
    pattern = os.path.join(frame_dir, "frame_%03d.png")
    pal = os.path.join(frame_dir, "palette.png")
    try:
        subprocess.run([ffmpeg, "-y", "-loglevel", "error", "-framerate", str(fps), "-i", pattern, "-vf",
                        "scale=%d:-1:flags=lanczos,palettegen=max_colors=160:stats_mode=full" % width, pal],
                       check=True, stderr=subprocess.PIPE, timeout=120)
        subprocess.run([ffmpeg, "-y", "-loglevel", "error", "-framerate", str(fps), "-i", pattern, "-i", pal, "-lavfi",
                        "scale=%d:-1:flags=lanczos[x];[x][1:v]paletteuse=dither=bayer:bayer_scale=5:diff_mode=rectangle" % width,
                        "-loop", "0", out], check=True, stderr=subprocess.PIPE, timeout=120)
    except (subprocess.CalledProcessError, subprocess.TimeoutExpired) as e:
        log("  ffmpeg failed: %s" % (getattr(e, "stderr", b"") or b"").decode(errors="replace").strip()[:300])
        return None
    return out


def kg(x):
    return "%.1f kg" % x


def report(path, cell, ck, axes, swing, grip, motion, mass, reach, plan, scene_err, timings, runtime):
    L = []
    add_ = L.append
    add_("Tenon work cell: six-axis robot palletising cardboard boxes")
    add_("=" * 64)
    add_("")
    add_("Joints (J1 to J6 revolute, G1 and G2 prismatic; limits found by turning each axis through the")
    add_("range with the rest at zero and keeping every pose free of overlaps)")
    for k in ("J1", "J2", "J3", "J4", "J5", "J6"):
        lo, hi = LIMITS[k]
        add_("  %-3s  %-18s %5d to %4d deg   range %3d deg" % (k, {"J1": "turret", "J2": "shoulder", "J3": "elbow",
                                                                  "J4": "forearm roll", "J5": "wrist pitch",
                                                                  "J6": "tool roll"}[k], lo, hi, hi - lo))
    add_("  G1/G2 gripper fingers      0 to %d mm each, coupled 1:1 (add_coupling): the jaws open %d to %d mm"
         % (FINGER_MAX * 1000, 2 * FINGER_HOME * 1000, 2 * (FINGER_HOME + FINGER_MAX) * 1000))
    add_("")
    add_("Reach (measured on the rig: J2 90, J3 -90, J5 0, the arm stretched along X)")
    add_("  J1 axis to wrist centre (J5 axis)  %.3f m" % math.hypot(reach["wrist"][0], reach["wrist"][1]))
    add_("  J1 axis to tool centre point       %.3f m" % math.hypot(reach["tcp"][0], reach["tcp"][1]))
    add_("  the closed-form kinematics used for the motion agree with the posed model to %.3f mm" % (reach["fk_error"] * 1000))
    add_("  tool centre at zero pose (%.3f, %.3f, %.3f) m: the zero pose has the forearm along +X" % reach["zero_tcp"])
    add_("  working radius used by the cell: pick %.2f m, place %.2f m, tool straight down"
         % (math.hypot(*PICK), math.hypot(*PLACE)))
    add_("")
    add_("Mass (mass_properties, material densities in kg/m3: cast aluminium %d, cast iron %d, steel %d,"
         % (ALUMINIUM, CAST_IRON, STEEL))
    add_("servo motors 3200 as an average of steel, copper and fins, rubber %d; the castings are hollow shells,"
         " the motors and covers solid)" % RUBBER)
    names = {"turret": "J1 turret", "upper_arm": "upper arm", "forearm": "forearm", "wrist_roll": "wrist roll",
             "wrist_head": "wrist head", "tool": "tool plate and gripper body", "fingers": "two fingers"}
    for k, label in names.items():
        t = mass[k]
        add_("  %-28s %9s   volume %7.2f L" % (label, kg(t["mass"]), t["volume"] * 1000))
    add_("  %-28s %9s   (centre of gravity %.3f, %.3f, %.3f m at zero pose)"
         % ("arm, J1 to the fingers", kg(mass["arm"]["mass"]), *mass["arm"]["centroid"]))
    add_("  %-28s %9s" % ("base with its plate", kg(mass["base"]["mass"])))
    add_("  %-28s %9s" % ("one carton", kg(mass["carton"]["mass"])))
    add_("")
    add_("Each axis through its limits, the rest at zero (arm against itself)")
    for k, a in axes.items():
        lo, hi = a["limits"]
        hits = "; ".join("%s / %s at %s" % (n1, n2, ", ".join("%g" % v for v in vs[:6]))
                         for (n1, n2), vs in a["hits"].items()) or "none"
        d, v = a["closest"]
        add_("  %-3s %4d to %4d deg, %2d poses: overlaps: %s; closest %s / %s %.1f mm at %g deg"
             % (k, lo, hi, a["poses"], hits, a["pair"][0], a["pair"][1], d * 1000, v))
    add_("")
    lo, hi = swing["limits"]
    add_("J1 swing against the cell, %d to %d deg every %d deg, arm stretched at carrying height (tool centre at"
         % (lo, hi, swing["step"]))
    q = swing["pose"]
    add_("%.2f m radius, 0.95 m up, J2 %.0f, J3 %.0f, J5 %.0f; gripper along the radius, the widest it gets)"
         % (math.hypot(*PICK), q["J2"], q["J3"], q["J5"]))
    add_("  overlaps with the cell: %s" % ("none" if not swing["hits"] else "; ".join(
        "%d deg: %s / %s" % (h[0], h[1], h[2]) for h in swing["hits"][:6])))
    add_("  nearest approach of forearm, wrist and gripper to:")
    for k, (d, th, t) in swing["near"].items():
        add_("    %-10s %7.1f mm  (%s at J1 %d deg)" % (k, d * 1000, t, th))
    add_("")
    g = grip
    add_("Gripper: sweep_check of finger joint G1 from 0 to %d mm in %d steps (G2 coupled 1:1 to it, ratio %s)"
         % (FINGER_MAX * 1000, g["steps"], "n/a" if g["coupling"] is None else "%g" % g["coupling"]["ratio"]))
    for a, b, vol, d in g["pairs"]:
        add_("  %-10s / %-10s overlap volume %g m3, closest %.2f mm" % (a, b, vol, d * 1000))
    if g["stopped"]:
        add_("  stopped: %s" % g["stopped"]["reason"])
    add_("")
    add_("Pick and place: %d poses (%s)" % (motion["frames"], ", ".join(sorted({f["phase"] for f in plan}, key=lambda p: [f["phase"] for f in plan].index(p)))))
    add_("  tool centre point: pick (%.2f, %.2f, %.3f) m, place (%.2f, %.2f, %.3f) m; boxes %.0f x %.0f x %.0f mm, %s"
         % (PICK[0], PICK[1], cell.grasp_z, PLACE[0], PLACE[1], cell.release_z, BOX[0] * 1000, BOX[1] * 1000,
            BOX[2] * 1000, kg(mass["carton"]["mass"])))
    qs = [f["q"] for f in plan]
    add_("  joint ranges used: " + ", ".join("%s %.0f to %.0f" % (k, min(q[k] for q in qs), max(q[k] for q in qs))
                                              for k in ("J1", "J2", "J3", "J5", "J6")))
    add_("  overlaps between arm, carton and cell at any pose: %s" % ("none" if not motion["hits"] else "; ".join(
        "pose %d (%s): %s / %s" % (h[0], h[1], h[2], h[3]) for h in motion["hits"][:6])))
    add_("  nearest approach over the motion (every fourth pose), arm part and pose:")
    for k, (d, i, t) in motion["near"].items():
        add_("    %-10s %7.1f mm  (%s at pose %d, %s)" % (k, d * 1000, t, i, plan[i]["phase"]))
    d, t = motion["grip"]
    add_("  fingers to the carried carton while it is held: %.2f mm (%s); the pads close on it with 0.2 mm to spare"
         % (d * 1000, t))
    add_("  scenes Home, Pick and Place store the arm's positions; going to Pick puts the tool centre %.3f mm from the"
         " pick point" % (scene_err * 1000))
    add_("")
    add_("Runtime")
    for name, t in timings:
        add_("  %-26s %6.1f s" % (name, t))
    add_("  %-26s %6.1f s" % ("total", runtime))
    with open(path, "w") as f:
        f.write("\n".join(L) + "\n")
    return path


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--socket", help="socket of a running app (default: start a headless one)")
    ap.add_argument("--app", help="app binary to start (default: TENON_APP or the repo's target/)")
    ap.add_argument("--out", default=".", help="directory for the pictures and report (default: here)")
    ap.add_argument("--quick", action="store_true", help="small model, small images, few frames (for tests)")
    ap.add_argument("--frames", type=int, help="frames of the animation (default 48, quick 8)")
    args = ap.parse_args(argv)
    quick = args.quick
    width, height = (320, 200) if quick else (1600, 1000)
    gw, gh = (320, 200) if quick else (800, 500)
    ss = 1 if quick else 2
    frames = args.frames or (8 if quick else 48)
    os.makedirs(args.out, exist_ok=True)
    fdir = os.path.join(args.out, "frames")
    os.makedirs(fdir, exist_ok=True)
    t_start = time.time()
    timings = []

    def timed(name, fn):
        t = time.time()
        r = fn()
        timings.append((name, time.time() - t))
        print("  %-26s %6.1f s" % (name, time.time() - t))
        return r

    with session(args.socket, args.app) as s:
        m = Model(s.client)
        c = s.client
        c.execute({"type": "new", "discard": s.launched})
        cell = timed("build", lambda: build(m, quick, log=lambda *a: None))
        cell.look()
        ck = Checks(cell)
        plan = cell.plan(frames)
        hero_i = next(i for i, f in enumerate(plan) if f["phase"] == "transfer") + len(
            [f for f in plan if f["phase"] == "transfer"]) // 2
        detail_i = next(i for i, f in enumerate(plan) if f["held"])
        paths = {"hero": os.path.join(args.out, "robot.png"), "detail": os.path.join(args.out, "robot-detail.png"),
                 "section": os.path.join(args.out, "robot-section.png")}
        gif_ok = shutil.which("ffmpeg") is not None

        mass = timed("mass", ck.mass)
        reach = timed("reach", ck.reach)
        axes = timed("axes through their limits", lambda: ck.axes(5 if quick else 25))
        swing = timed("J1 swing against the cell", lambda: ck.swing(45 if quick else 15))
        grip = timed("gripper sweep_check", ck.gripper)

        def picture(i, f):
            c.execute({"type": "select", "ids": [], "mode": "replace"})
            if i == hero_i:
                notes = callouts(cell, HERO, width / height, cell.hero_labels())
                m.render(paths["hero"], width, height, camera=HERO, lighting=None, supersample=ss, ground=False)
                c.execute({"type": "erase", "ids": notes})
            if i == detail_i:
                m.render(paths["detail"], width, height, camera=DETAIL, lighting=None, supersample=ss, ground=False)
            if gif_ok:
                m.render(os.path.join(fdir, "frame_%03d.png" % i), gw, gh, camera=MOTION, lighting=None, supersample=ss, ground=False)

        motion = timed("pick and place (checks, renders)", lambda: ck.motion(plan, 4, picture))
        cell.set_pose(ck.zero, 0.0, [(cell.carried, Transform.translate(PICK[0], PICK[1], CONVEYOR_TOP))])
        scenes = timed("scenes", cell.scenes)
        scene_err = cell.check_scenes(scenes)
        timed("section picture", lambda: cell.section_picture(paths["section"], width, height, ss))
        gif = timed("gif", lambda: make_gif(fdir, os.path.join(args.out, "robot-motion.gif"))) if gif_ok else None
        runtime = time.time() - t_start
        rpt = report(os.path.join(args.out, "robot-report.txt"), cell, ck, axes, swing, grip, motion, mass, reach, plan,
                     scene_err, timings, runtime)
    print("wrote %s" % ", ".join(p for p in (paths["hero"], paths["detail"], paths["section"], gif or "(no ffmpeg: no gif)", rpt)))


if __name__ == "__main__":
    main()
