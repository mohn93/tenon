#!/usr/bin/env python3
"""A rigged, cut-away two-stage planetary gearbox, drawn like an engineering illustration.

    python3 gearbox.py [--socket PATH] [--app APP] [--out DIR] [--quick] [--frames N]

Without ``--socket`` it starts a headless app (``TENON_APP`` or the repo's
``target/{debug,release}/app``). It writes ``gearbox.png`` (the cutaway), ``gearbox-exploded.png``,
``gearbox-mesh.png`` (a section square to the axis through stage 1), ``gearbox-motion.gif`` (the
input turning, there and back; needs ``ffmpeg``) and ``gearbox-report.txt``.

What it shows of the API:

- involute gear teeth generated in Python (module 1.5 mm, 20 degree pressure angle, flank and
  tip points computed from the involute function) and pushed out with ``extrude``: both suns, six
  planets and the ring's cutter. The ring gear is a bar minus a gear-shaped cutter, so its
  internal teeth are the exact conjugate of the planets'. The tooth counts follow
  Z_ring = Z_sun + 2 Z_planet (54 = 18 + 2 * 18 = 24 + 2 * 15) and the planets are phased so they
  mesh with sun and ring at once (the assembly condition, (Z_sun + Z_ring) / 3 a whole number);
- lathe parts with ``revolve``: shafts, carriers, housing, cover and the races of three ball
  bearings (grooves cut to the balls); balls, needle rollers and bolts as radial arrays
  (``rotate`` with copies), a hex bolt as a component placed eight times;
- solid booleans for bores, keyways, bolt circles, feet and the ring's teeth; materials with
  metalness and densities on every part (steel, painted cast housing, bronze, brass, rubber);
- a rigged mechanism: every moving part a root instance on revolute joints (the input shaft, both
  carriers, the six planets on their pins relative to their carriers, three ball sets) and
  couplings whose ratios come from the tooth counts, so one ``drive`` of the input turns
  everything the right way;
- ``mass_properties``, ``interference``, ``clearance`` and ``sweep_check`` for the report, and a
  check that driving the input gives the output angle the ratio predicts;
- a quarter cutaway: each static part (housing, cover, ring) is two instances with its own section
  planes, one keeping the back half and one the lower front quarter; the moving parts stay whole;
  filled cut faces, studio lighting, ambient occlusion, labels; an exploded view along the axis;
  ``render_view`` with ``joints`` for the frames of the GIF.

Lengths are meters in the API; the code below works in millimeters and converts. The parts are
drawn about the Z axis (up is +Y) and turned onto the X axis (up +Z) before they are rigged, so
a positive joint value turns counter-clockwise looking from the output end back at the input.
"""
import argparse
import math
import os
import shutil
import subprocess
import sys
import time

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))

from tenon import Model, Transform, session  # noqa: E402

K = 1e-3  # millimeters to meters

# ---------------------------------------------------------------- gear data

MODULE = 1.5
ALPHA = math.radians(20.0)
ZS1, ZP1 = 18, 18  # stage 1: sun and planets
ZS2, ZP2 = 24, 15  # stage 2
ZR = 54  # the ring, shared by both stages
NP = 3  # planets per stage
BACKLASH = 0.08  # each external gear's teeth are thinned by this much at the pitch circle
A1 = MODULE * (ZS1 + ZP1) / 2  # centre distances
A2 = MODULE * (ZS2 + ZP2) / 2
THETA0 = math.radians(150.0)  # where planet 0 of each stage stands

STAGE1 = (18.0, 30.0)  # axial extent of the gears (z, mm)
STAGE2 = (40.0, 56.0)
RING_Z = (17.0, 57.0)

STEEL, CAST_IRON, BRONZE, BRASS, RUBBER, GLASS = 7850, 7200, 8800, 8500, 1200, 2500

# bearings: bore radius, outside radius, z of the front face, width, ball radius, balls
BEARING_IN = dict(ri=8.0, ro=20.0, z0=4.7, w=10.0, rb=2.7, n=9)
BEARING_OUT1 = dict(ri=15.0, ro=27.5, z0=67.0, w=13.0, rb=3.1, n=10)
BEARING_OUT2 = dict(ri=15.0, ro=27.5, z0=84.0, w=13.0, rb=3.1, n=10)


def involute(a):
    return math.tan(a) - a


def gear_points(z_teeth, cx, cy, phase, tip, root, thickness, flank):
    """The outline of an external involute gear of module ``MODULE`` with ``z_teeth`` teeth, as
    (x, y) in mm, counter-clockwise, tooth 0 centred on the angle ``phase``, its centre at
    (cx, cy). ``thickness`` is the tooth's thickness on the pitch circle; ``tip`` and ``root`` are the
    radii of the tooth tips and of the space between teeth. Below the base circle the flank runs
    radially to the root."""
    rp = MODULE * z_teeth / 2
    rb = rp * math.cos(ALPHA)
    half_p = thickness / (2 * rp)

    def half(r):  # half the angle a tooth spans at radius r
        return half_p + involute(ALPHA) - involute(math.acos(min(1.0, rb / r)))

    r0 = max(rb, root)
    radii = [r0 + (tip - r0) * i / (flank - 1) for i in range(flank)]
    pts = []
    for i in range(z_teeth):
        c = phase + 2 * math.pi * i / z_teeth
        seq = []
        if root < r0:
            seq.append((root, c - half(r0)))
        seq += [(r, c - half(r)) for r in radii]
        seq.append((tip, c))
        seq += [(r, c + half(r)) for r in reversed(radii)]
        if root < r0:
            seq.append((root, c + half(r0)))
        pts += [(cx + r * math.cos(a), cy + r * math.sin(a)) for r, a in seq]
    return pts


def mesh_phases():
    """Phases (radians) of every gear so that sun, planets and ring mesh together.

    A gear's phase is the angle of the centre of its tooth 0; the ring's is that of its tooth 0
    (its material). At the contact between sun and planet the tooth coordinates add to a half,
    and between planet and ring they differ by a half; (Z_sun + Z_ring) / 3 whole makes the same
    phases serve all three planets, which are copies turned by 120 degrees."""
    th = THETA0
    two_pi = 2 * math.pi
    # stage 1: sun 1 tooth 0 at angle 0
    phi_s1 = 0.0
    u_s = (th - phi_s1) * ZS1 / two_pi
    phi_p1 = th + math.pi - (two_pi / ZP1) * (0.5 - u_s)
    phi_r = th - (two_pi / ZR) * ((th - phi_p1) * ZP1 / two_pi - 0.5)
    # stage 2: the ring is fixed, so the planets follow it and the sun follows them
    phi_p2 = th - (two_pi / ZP2) * (0.5 + (th - phi_r) * ZR / two_pi)
    u_s2 = 0.5 - (th + math.pi - phi_p2) * ZP2 / two_pi
    phi_s2 = th - u_s2 * two_pi / ZS2
    return dict(s1=phi_s1, p1=phi_p1, r=phi_r, p2=phi_p2, s2=phi_s2)


# ---------------------------------------------------------------- the build


class Gearbox:
    def __init__(self, m, quick=False):
        self.m = m
        self.c = m.client
        self.quick = quick
        self.seg = 28 if quick else 72  # round parts
        self.bseg = 20 if quick else 48  # bearing races
        self.flank = 3 if quick else 8  # points per tooth flank
        self.ball_seg = 8 if quick else 12
        self.n_rollers = 6 if quick else 8
        self.ph = mesh_phases()
        self.originals = []
        self.roots = []  # every root instance drawn (before the turn onto the X axis)

    # helpers --------------------------------------------------------

    def v(self, *p):
        return tuple(x * K for x in p)

    def cyl(self, base, r, h, axis=(0, 0, 1), seg=None, **kw):
        return self.m.cylinder(self.v(*base), r * K, h * K, axis=axis, segments=seg or self.seg, **kw)

    def tube(self, base, ro, ri, h, axis=(0, 0, 1), seg=None, **kw):
        return self.m.tube(self.v(*base), ro * K, ri * K, h * K, axis=axis, segments=seg or self.seg, **kw)

    def box(self, corner, size, **kw):
        return self.m.box(self.v(*corner), self.v(*size), **kw)

    def rev(self, profile, name=None, material=None, seg=None):
        """A lathe part: ``profile`` is (r, z) in mm, turned about the Z axis."""
        pts = [(r * K, 0.0, z * K) for r, z in profile]
        return self.m.revolve(pts, (0, 0, 0), (0, 0, 1), segments=seg or self.seg, name=name, material=material)

    def gear(self, z_teeth, cx, cy, phase, z0, width, tip, root, thickness, name, material=None):
        pts = gear_points(z_teeth, cx, cy, phase, tip, root, thickness, self.flank)
        return self.m.extrude([(x * K, y * K, z0 * K) for x, y in pts], width * K, name=name, material=material)

    def polar_z(self, inst, count, origin=(0, 0)):
        """``count`` copies of ``inst`` evenly around the Z axis (through ``origin``), plus itself."""
        return [inst] + inst.polar((0, 0, 1), 360.0 / count, count - 1, origin=self.v(origin[0], origin[1], 0))

    # materials --------------------------------------------------------

    def materials(self):
        m = self.m
        self.gear_steel = m.material("Gear steel", (176, 182, 192), metalness=0.95, roughness=0.26, density=STEEL)
        self.gear_dark = m.material("Nitrided steel", (122, 130, 146), metalness=0.95, roughness=0.3, density=STEEL)
        self.ring_steel = m.material("Ring gear steel", (196, 200, 208), metalness=0.9, roughness=0.34, density=STEEL)
        self.polished = m.material("Polished steel", (212, 215, 222), metalness=1.0, roughness=0.16, density=STEEL)
        self.carrier_iron = m.material("Carrier iron", (84, 90, 100), metalness=0.65, roughness=0.5, density=CAST_IRON)
        self.blue = m.material("Housing blue", (20, 88, 176), metalness=0.2, roughness=0.36, density=CAST_IRON)
        self.bronze = m.material("Bronze", (196, 130, 58), metalness=0.9, roughness=0.36, density=BRONZE)
        self.brass = m.material("Brass", (222, 184, 74), metalness=0.92, roughness=0.28, density=BRASS)
        self.rubber = m.material("Rubber", (28, 28, 32), roughness=0.9, density=RUBBER)
        self.zinc = m.material("Zinc plated", (192, 188, 160), metalness=0.85, roughness=0.34, density=STEEL)
        self.chrome = m.material("Bearing steel", (228, 231, 238), metalness=1.0, roughness=0.1, density=STEEL)
        self.black = m.material("Black oxide", (38, 40, 46), metalness=0.5, roughness=0.5, density=STEEL)
        self.red = m.material("Level mark", (206, 40, 30), roughness=0.4, density=STEEL)
        self.glass = self.c.execute(
            {"type": "add_material", "name": "Sight glass", "color": [150, 205, 230], "opacity": 0.4,
             "density": GLASS, "existing": "reuse"}
        )["changes"]["materials"][0]

    # bearings -----------------------------------------------------------

    def bearing(self, ri, ro, z0, w, rb, n, name):
        """A deep-groove ball bearing: (outer race, inner race, balls). Races are lathe parts with
        a groove 0.06 mm larger than the balls, so nothing touches."""
        rp = (ri + ro) / 2
        zc = z0 + w / 2
        g = rb + 0.06
        arc_n = 5 if self.quick else 9

        r_in = rp - 0.5 * rb  # shoulders of the inner race
        t0 = math.acos((r_in - rp) / g)
        h = g * math.sin(t0)
        inner = [(ri, z0), (r_in, z0)]
        for k in range(arc_n + 1):
            t = (2 * math.pi - t0) + (2 * t0 - 2 * math.pi) * k / arc_n
            inner.append((rp + g * math.cos(t), zc + g * math.sin(t)))
        inner += [(r_in, z0 + w), (ri, z0 + w)]

        r_out = rp + 0.5 * rb
        t1 = math.acos((r_out - rp) / g)
        h1 = g * math.sin(t1)
        outer = [(ro, z0), (ro, z0 + w), (r_out, z0 + w)]
        for k in range(arc_n + 1):
            t = t1 - 2 * t1 * k / arc_n
            outer.append((rp + g * math.cos(t), zc + g * math.sin(t)))
        outer += [(r_out, z0)]

        outer_race = self.rev(outer, name + " outer race", self.chrome, self.bseg)
        inner_race = self.rev(inner, name + " inner race", self.chrome, self.bseg)
        ball = self.m.sphere(self.v(rp, 0, zc), rb * K, segments=self.ball_seg, material=self.chrome)
        balls = self.polar_z(ball, n)
        balls = self.m.group(balls, name=name + " balls")
        return outer_race, inner_race, balls, (rp - rb) / (2 * rp)

    # the ring gear --------------------------------------------------------

    def ring_gear(self):
        m = self.m
        rp = MODULE * ZR / 2
        body = self.cyl((0, 0, RING_Z[0]), 50.0, RING_Z[1] - RING_Z[0], seg=self.seg * 2)
        # the cutter is an external gear whose teeth are the ring's spaces: tips at the ring's
        # root circle (1.25 m outside the pitch circle), roots at the ring's tooth tips
        cutter = self.gear(ZR, 0, 0, self.ph["r"] + math.pi / ZR, RING_Z[0] - 1.0, RING_Z[1] - RING_Z[0] + 2.0,
                           rp + 1.25 * MODULE, rp - MODULE, math.pi * MODULE / 2, "Ring cutter")
        ring = m.subtract(body, [cutter])
        ring.paint(self.ring_steel).name("Ring gear")
        self.ring = ring

    # input shaft and sun gear 1 ---------------------------------------------

    def input_shaft(self):
        m = self.m
        prof = [(0, -34), (7, -34), (8, -33), (8, 30), (4, 30), (4, 43), (3, 44), (0, 44)]
        shaft = self.rev(prof, seg=self.seg)
        sun = self.gear(ZS1, 0, 0, self.ph["s1"], STAGE1[0], STAGE1[1] - STAGE1[0],
                        MODULE * ZS1 / 2 + MODULE, MODULE * ZS1 / 2 - 1.25 * MODULE, math.pi * MODULE / 2 - BACKLASH,
                        "Sun 1")
        body = m.union([shaft, sun])
        slot = self.box((-2.5, 5.0, -32.0), (5.0, 3.4, 20.0))
        body = m.subtract(body, [slot])
        body.paint(self.polished).name("Input shaft")
        key = self.box((-2.45, 5.05, -30.0), (4.9, 4.0, 16.0), material=self.zinc)
        key.name("Input key")
        b = BEARING_IN
        outer, inner, balls, ratio = self.bearing(name="Input bearing", **b)
        self.bearing_in_outer = outer
        self.balls_in, self.balls_in_ratio = balls, ratio
        self.input = m.group([body, key, inner], name="Input shaft")

    # one stage's planets -------------------------------------------------------

    def planet(self, z_teeth, a, phase, z_range, pin_r, roller_r, name, material):
        """Planet 0 at THETA0 (gear, bronze thrust washer and needle rollers), then its two copies."""
        m = self.m
        cx, cy = a * math.cos(THETA0), a * math.sin(THETA0)
        rp = MODULE * z_teeth / 2
        z0, z1 = z_range
        bore = pin_r + 2 * roller_r + 0.1
        gear = self.gear(z_teeth, cx, cy, phase, z0, z1 - z0, rp + MODULE, rp - 1.25 * MODULE,
                         math.pi * MODULE / 2 - BACKLASH, name + " gear")
        hole = self.cyl((cx, cy, z0 - 1), bore, z1 - z0 + 2)
        gear = m.subtract(gear, [hole])
        gear.paint(material)
        orbit = pin_r + roller_r + 0.05
        roller = self.cyl((cx + orbit, cy, z0 + 1.0), roller_r, z1 - z0 - 2.0, seg=8 if not self.quick else 6,
                          material=self.chrome)
        rollers = [roller] + roller.polar((0, 0, 1), 360.0 / self.n_rollers, self.n_rollers - 1,
                                          origin=self.v(cx, cy, 0))
        washer = self.tube((cx, cy, z1 + 0.05), bore + 1.5, pin_r + 0.05, 0.9, material=self.bronze)
        p0 = m.group([gear] + rollers + [washer], name=name)
        return self.polar_z(p0, NP)

    def pin(self, a, z_end, z_in, r, groove_z):
        """A planet pin with a circlip groove at its free end, reaching ``z_in`` into the carrier,
        at the planet-0 position; returns the pin and its circlip."""
        m = self.m
        cx, cy = a * math.cos(THETA0), a * math.sin(THETA0)
        g0, g1 = groove_z
        rg = r - 0.7
        prof = [(0, z_end), (r - 0.3, z_end), (r, z_end + 0.3), (r, g0), (rg, g0), (rg, g1), (r, g1), (r, z_in), (0, z_in)]
        pts = [(x * K, 0.0, z * K) for x, z in prof]
        pin = m.revolve(pts, (0, 0, 0), (0, 0, 1), segments=self.seg, material=self.polished)
        pin.move(self.v(cx, cy, 0))
        clip = self.tube((cx, cy, g0 + 0.05), r + 1.1, rg + 0.05, g1 - g0 - 0.1, material=self.black)
        return pin, clip

    def carrier1(self):
        m = self.m
        r_pin = 3.5
        plate = self.cyl((0, 0, 31.0), 37.0, 5.0, seg=self.seg * 2)
        hub = self.cyl((0, 0, 35.5), 12.0, 4.7)
        sun2 = self.gear(ZS2, 0, 0, self.ph["s2"], STAGE2[0], STAGE2[1] - STAGE2[0],
                         MODULE * ZS2 / 2 + MODULE, MODULE * ZS2 / 2 - 1.25 * MODULE, math.pi * MODULE / 2 - BACKLASH,
                         "Sun 2")
        pin, clip = self.pin(A1, 16.4, 34.0, r_pin, (16.85, 17.95))
        pins = self.polar_z(pin, NP)
        clips = self.polar_z(clip, NP)
        body = m.union([plate, hub, sun2] + pins)
        holes = []
        h0 = self.cyl((A1 * math.cos(THETA0 + math.pi / NP), A1 * math.sin(THETA0 + math.pi / NP), 30.5), 8.0, 6.0)
        holes = self.polar_z(h0, NP)
        bore = self.cyl((0, 0, 30.0), 5.55, 16.0, seg=self.seg)
        body = m.subtract(body, holes + [bore])
        body.paint(self.carrier_iron).name("Carrier 1 and sun 2")
        bush = self.tube((0, 0, 31.0), 5.5, 4.05, 14.0, material=self.bronze)
        bush.name("Carrier bushing")
        self.carrier1_inst = m.group([body, bush] + clips, name="Carrier 1 and sun 2")

    def carrier2(self):
        m = self.m
        r_pin = 4.0
        prof = [(0, 57), (37, 57), (37, 64), (18, 64), (18, 67), (15, 67), (15, 118), (14, 119), (0, 119)]
        shaft = self.rev(prof, seg=self.seg * 2 if not self.quick else self.seg)
        pin, clip = self.pin(A2, 38.4, 62.0, r_pin, (38.85, 39.95))
        pins = self.polar_z(pin, NP)
        clips = self.polar_z(clip, NP)
        body = m.union([shaft] + pins)
        h0 = self.cyl((A2 * math.cos(THETA0 + math.pi / NP), A2 * math.sin(THETA0 + math.pi / NP), 56.0), 8.0, 9.0)
        holes = self.polar_z(h0, NP)
        slot = self.box((-3.0, 11.5, 105.0), (6.0, 4.0, 8.0))
        body = m.subtract(body, holes + [slot])
        body.paint(self.carrier_iron).name("Output carrier and shaft")
        # the flange, keyed to the shaft
        flange = self.cyl((0, 0, 104.0), 40.0, 10.0, seg=self.seg * 2)
        bore = self.cyl((0, 0, 103.0), 15.05, 12.0)
        keyway = self.box((-3.0, 15.0, 103.0), (6.0, 2.5, 12.0))
        bolt_hole = self.cyl((30.0 * math.cos(math.radians(15)), 30.0 * math.sin(math.radians(15)), 103.0), 3.4, 12.0, seg=16)
        bolt_holes = self.polar_z(bolt_hole, 6)
        pilot = self.cyl((0, 0, 112.0), 22.0, 3.0, seg=self.seg)
        flange = m.subtract(flange, [bore, keyway, pilot] + bolt_holes)
        flange.paint(self.carrier_iron).name("Output flange")
        key = self.box((-2.95, 11.55, 105.0), (5.9, 5.9, 8.0), material=self.zinc)
        key.name("Output key")
        # bearing inner races, and the collar between them
        o1, i1, b1, r1 = self.bearing(name="Output bearing 1", **BEARING_OUT1)
        o2, i2, b2, r2 = self.bearing(name="Output bearing 2", **BEARING_OUT2)
        collar = self.tube((0, 0, 80.1), 18.0, 15.0, 3.8, material=self.polished)
        self.bearing_out_outer = [o1, o2]
        self.balls_out = [b1, b2]
        self.balls_out_ratio = [r1, r2]
        self.output = m.group([body, flange, key, i1, i2, collar] + clips, name="Output carrier")

    # the housing -----------------------------------------------------------------

    def bolt_component(self):
        m = self.m
        head = self.m.prism(self.v(0, 0, -4.0), 5.5 * K, 4.0 * K, sides=6, material=self.zinc)
        shank = self.cyl((0, 0, 0), 3.0, 22.0, seg=12, material=self.zinc)
        bolt = m.group([head, shank], name="Bolt")
        self.bolt = m.component([bolt], "Hex bolt M6")
        self.originals.append(self.bolt)

    def seal(self, z0, z1, ro, ri, name):
        case = self.tube((0, 0, z0), ro, ro - 1.0, z1 - z0, seg=self.bseg, material=self.zinc)
        lip = self.tube((0, 0, z0), ro - 1.0, ri, z1 - z0, seg=self.bseg, material=self.rubber)
        return self.m.group([case, lip], name=name)

    def cover(self):
        m = self.m
        prof = [(62, 0), (62, 15), (14, 15), (14, 14.7), (20.05, 14.7), (20.05, 4.65), (21.1, 4.65), (21.1, 3.55),
                (20.05, 3.55), (20.05, 0)]
        cover = self.rev(prof, seg=self.seg * 2)
        hole = self.cyl((56.0 * math.cos(math.radians(22.5)), 56.0 * math.sin(math.radians(22.5)), -1.0), 3.2, 17.0, seg=12)
        holes = self.polar_z(hole, 8)
        cover = m.subtract(cover, holes)
        cover.paint(self.blue).name("Front cover")
        seal = self.seal(0.0, 3.5, 20.0, 8.15, "Input seal")
        clip = self.tube((0, 0, 3.6), 21.0, 18.3, 1.0, seg=self.bseg, material=self.black)
        clip.name("Input circlip")
        bolts = []
        for k in range(8):
            a = math.radians(22.5 + 45 * k)
            bolts.append(m.place(self.bolt, Transform.translate(56.0 * math.cos(a) * K, 56.0 * math.sin(a) * K, 0.0)))
        self.cover_parts = [cover, seal, clip, self.bearing_in_outer] + bolts

    def body(self):
        m = self.m
        fins = [22, 29, 36, 43, 50, 57]
        outer = [(62, 15)]
        for c in fins:
            outer += [(62, c - 1.25), (67, c - 1.25), (67, c + 1.25), (62, c + 1.25)]
        outer += [(62, 62), (42, 72), (42, 102)]
        inner = [(27.55, 102), (27.55, 98.7), (28.6, 98.7), (28.6, 97.5), (27.55, 97.5), (27.55, 83.5), (24, 83.5),
                 (24, 80.5), (27.55, 80.5), (27.55, 66.5), (24, 66.5), (24, 64.5), (50.05, 64.5), (50.05, 15)]
        shell = self.rev(outer + inner, seg=self.seg * 2)
        feet = []
        for z in (20.0, 44.0):
            feet.append(self.box((-76.0, -78.0, z), (152.0, 24.0, 14.0)))
        shell = m.union([shell] + feet)
        tools = []
        for z in (27.0, 51.0):
            for x in (-63.0, 63.0):
                tools.append(self.cyl((x, -52.0, z), 6.5, 30.0, axis=(0, -1, 0), seg=20))
        cover_hole = self.cyl((56.0 * math.cos(math.radians(22.5)), 56.0 * math.sin(math.radians(22.5)), 14.5), 3.05, 8.5, seg=12)
        tools += self.polar_z(cover_hole, 8)
        shell = m.subtract(shell, tools)
        shell.paint(self.blue).name("Housing")
        # breather on top, between two fins
        nut = self.m.prism(self.v(20.0, 57.0, 40.0), 5.0 * K, 4.5 * K, sides=6, axis=(0, 1, 0), material=self.brass)
        stem = self.cyl((20.0, 61.0, 40.0), 3.6, 11.0, axis=(0, 1, 0), seg=16, material=self.brass)
        cap = self.m.cone(self.v(20.0, 72.0, 40.0), 7.0 * K, 5.0 * K, top_radius=3.0 * K, axis=(0, 1, 0),
                          segments=20, material=self.black)
        breather = m.group([nut, stem, cap], name="Breather")
        # oil level sight glass on the lower front, between two fins
        n = (-math.cos(math.radians(12)), -math.sin(math.radians(12)), 0.0)
        at = lambda d: (n[0] * d, n[1] * d, 39.5)  # noqa: E731
        boss = self.tube(at(60.0), 15.0, 10.0, 8.0, axis=n, seg=32, material=self.blue)
        boss.name("Oil sight glass boss")
        glass = self.cyl(at(63.0), 10.0, 2.4, axis=n, seg=32, material=self.glass)
        mark = self.m.torus(self.v(*at(65.8)), 6.0 * K, 0.7 * K, axis=n, segments=24, tube_segments=8, material=self.red)
        sight = m.group([boss, glass, mark], name="Oil sight glass")
        seal = self.seal(98.8, 102.0, 27.5, 15.15, "Output seal")
        clip = self.tube((0, 0, 97.6), 28.5, 25.0, 1.0, seg=self.bseg, material=self.black)
        clip.name("Output circlip")
        self.body_parts = [shell, breather, sight, seal, clip] + self.bearing_out_outer

    def static_groups(self):
        m = self.m
        self.cover_inst = m.group(self.cover_parts, name="Front cover")
        self.body_inst = m.group(self.body_parts, name="Housing")

    # assemble ------------------------------------------------------------------

    def build(self, log=print):
        t0 = time.time()
        steps = [("materials", self.materials), ("ring gear", self.ring_gear), ("input shaft", self.input_shaft),
                 ("stage 1", self.stage1), ("carrier 1", self.carrier1), ("stage 2", self.stage2),
                 ("output", self.carrier2), ("bolts", self.bolt_component), ("cover", self.cover),
                 ("housing", self.body), ("groups", self.static_groups), ("tidy", self.tidy), ("turn", self.turn)]
        for name, fn in steps:
            t = time.time()
            fn()
            log("  %-18s %6.1f s" % (name, time.time() - t))
        log("built in %.1f s" % (time.time() - t0))

    def stage1(self):
        self.planets1 = self.planet(ZP1, A1, self.ph["p1"], STAGE1, 3.5, 0.95, "Stage 1 planet", self.gear_steel)

    def stage2(self):
        self.planets2 = self.planet(ZP2, A2, self.ph["p2"], STAGE2, 4.0, 1.0, "Stage 2 planet", self.gear_dark)

    def tidy(self):
        self.c.execute({"type": "erase", "ids": [o.id for o in self.originals]})
        self.originals = []

    def turn(self):
        """Turn every root instance from the Z axis onto the X axis: x -> y, y -> z, z -> x."""
        self.T = Transform((0, 1, 0, 0, 0, 1, 1, 0, 0, 0, 0, 0))
        ents = self.c.query("entities", kinds=["instance"])["instances"]
        cmds = []
        for e in ents:
            t = self.T @ Transform(e["transform"])
            cmds.append({"type": "set_transform", "id": e["id"], "transform": t.to_json()})
        self.c.batch(cmds)

    def pt(self, x, y, z):
        """A point drawn in the Z-axis frame, in meters on the X axis."""
        return self.T.apply((x * K, y * K, z * K))


    # the rig ------------------------------------------------------------------------

    def rig(self):
        """Joints and couplings from the tooth counts. Every part stands where the input angle 0 puts
        it, so each joint is met as it is made and every value starts at 0.

        With the ring fixed and the sun driven, the carrier turns at 1 / (1 + Z_ring / Z_sun) of the
        sun. A planet turns relative to its carrier, which is how its joint is made, at
        -(Z_sun / Z_planet) of the sun's turn relative to the carrier (the sign: opposite to the sun)."""
        m = self.m
        ax = (1.0, 0.0, 0.0)
        self.r1 = 1.0 / (1.0 + ZR / ZS1)
        self.r2 = 1.0 / (1.0 + ZR / ZS2)
        self.k_p1 = -(ZS1 / ZP1) * (1.0 - self.r1)  # planet joint per input degree
        self.k_p2 = -(ZS2 / ZP2) * (1.0 - self.r2)  # planet joint per carrier 1 degree
        self.j_in = m.joint("revolute", None, self.input, self.pt(0, 0, 0), axis=ax, name="Input shaft")
        self.j_c1 = m.joint("revolute", None, self.carrier1_inst, self.pt(0, 0, 0), axis=ax, name="Carrier 1")
        m.couple(self.j_in, self.j_c1, self.r1)
        self.j_p1, self.j_p2 = [], []
        for k in range(NP):
            a = THETA0 + 2 * math.pi * k / NP
            j = m.joint("revolute", self.carrier1_inst, self.planets1[k], self.pt(A1 * math.cos(a), A1 * math.sin(a), 24.0),
                        axis=ax, name="Planet 1.%d" % (k + 1))
            m.couple(self.j_in, j, self.k_p1)
            self.j_p1.append(j)
        self.j_out = m.joint("revolute", None, self.output, self.pt(0, 0, 0), axis=ax, name="Output carrier")
        m.couple(self.j_c1, self.j_out, self.r2)
        for k in range(NP):
            a = THETA0 + 2 * math.pi * k / NP
            j = m.joint("revolute", self.output, self.planets2[k], self.pt(A2 * math.cos(a), A2 * math.sin(a), 48.0),
                        axis=ax, name="Planet 2.%d" % (k + 1))
            m.couple(self.j_c1, j, self.k_p2)
            self.j_p2.append(j)
        # the balls of each bearing roll round at (r_inner contact) / (r_inner + r_outer contact)
        # of the inner race's speed (the outer race is fixed)
        self.j_balls = []
        for name, balls, ratio, driver in [("Input bearing balls", self.balls_in, self.balls_in_ratio, self.j_in),
                                           ("Output bearing 1 balls", self.balls_out[0], self.balls_out_ratio[0], self.j_out),
                                           ("Output bearing 2 balls", self.balls_out[1], self.balls_out_ratio[1], self.j_out)]:
            j = m.joint("revolute", None, balls, self.pt(0, 0, 0), axis=ax, name=name)
            m.couple(driver, j, ratio)
            self.j_balls.append(j)
        self.j_in.drive(0.001)
        self.j_in.drive(0)

    def moving(self):
        return ([self.input, self.carrier1_inst, self.output, self.balls_in] + self.planets1 + self.planets2
                + self.balls_out)

    def static_parts(self):
        return [self.body_inst, self.cover_inst, self.ring]

    # the cutaway ---------------------------------------------------------------------

    def cutaway(self):
        """A quarter cutaway of the static parts: each is two instances, ``a`` keeping the back half
        (y >= 0) and ``b`` the lower front quarter (y < 0 and z <= 0), so the upper front quarter is
        open and the moving parts show whole."""
        self.pairs = []
        self.ring_full = self.ring.clone()  # for the face-on view, shown only there
        self.ring_full.hide()
        for a in self.static_parts():
            b = a.clone()
            a.section((0, 0, 0), (0, 1, 0))
            b.section((0, 0, 0), (0, -1, 0))
            b.section((0, 0, 0), (0, 0, -1))
            self.pairs.append((a, b))
        self.m.section_display(planes=False, fill=True, fill_color="dark")

    def look(self):
        m = self.m
        m.lighting("studio")
        m.style("Cutaway", background=[232, 235, 240], sky=[252, 253, 255],
                edges={"color": [26, 28, 34], "profiles": 2},
                ambient_occlusion={"on": True, "radius": 0.012, "strength": 1.2})
        self.centre = (0.052, 0.0, -0.002)
        self.eye = (-0.13, -0.34, 0.225)
        m.camera(self.eye, self.centre, fov_y_deg=30)

    def callouts(self, specs, eye, centre, back, spread):
        """Leader labels on a plane ``back`` meters in front of ``centre`` square to the view (so the
        leaders run along the view and are not hidden behind the cut), each text (dx, dy) of
        ``spread`` meters right and up on screen at the centre."""
        fwd = _unit(_sub(centre, eye))
        right = _unit((fwd[1], -fwd[0], 0.0))
        up = _cross(right, fwd)
        far = _add(centre, _mul(fwd, -back))
        d = math.dist(eye, centre)
        spread = spread * (d - back) / d  # the nearer the plane, the less a meter spreads on screen
        notes = []
        for inst, text, dx, dy, hint, *child in specs:
            pos = _add(far, _add(_mul(right, dx * spread), _mul(up, dy * spread)))
            if hint is None:
                notes.append(([], self.m.label(inst, text, position=pos)))
            else:
                notes.append(self.label_vertex(inst, text, hint, pos, *child))
        return notes

    def label_vertex(self, inst, text, near, pos, child=None):
        """A label pointing at the vertex of ``inst`` (or of its part called ``child``) nearest ``near``
        (model coordinates), for a part whose centre is out of sight."""
        c = self.c
        t = inst.transform()
        c.execute({"type": "open_instance", "id": inst.id})
        opened = 1
        path = [inst.id]
        try:
            for name in ([child] if isinstance(child, str) else child or []):
                e = next(e for e in c.query("entities", kinds=["instance"])["instances"] if e["name"] == name)
                t = t @ Transform(e["transform"])
                c.execute({"type": "open_instance", "id": e["id"]})
                path.append(e["id"])
                opened += 1
            vs = c.query("entities", kinds=["vertex"])["vertices"]
            best = min(vs, key=lambda v: math.dist(t.apply(v["position"]), near))
            r = c.execute({"type": "add_label", "target": best["id"], "text": text,
                           "position": list(t.inverse().apply(pos))})
        finally:
            for _ in range(opened):
                c.execute({"type": "close_instance"})
        return (path, next((i for i in r["changes"]["created"] if i.startswith("n")), None))

    def erase_notes(self, notes):
        """Erase labels made by ``callouts``: root ones by id, the others inside their instances."""
        c = self.c
        for path, n in notes:
            if not n:
                continue
            for i in path:
                c.execute({"type": "open_instance", "id": i})
            try:
                c.execute({"type": "erase", "ids": [n]})
            finally:
                for _ in path:
                    c.execute({"type": "close_instance"})

    def hero_labels(self):
        return [(self.input, "Input shaft, key and sun gear 1", -0.150, 0.045, None),
                (self.planets1[0], "Stage 1 planets (3)", -0.065, 0.100, (0.018, -0.0329, 0.019), "Stage 1 planet gear"),
                (self.carrier1_inst, "Carrier 1 with sun gear 2", 0.000, 0.108, None),
                (self.planets2[0], "Stage 2 planets (3)", 0.090, 0.096, (0.040, -0.0349, 0.0201), "Stage 2 planet gear"),
                (self.output, "Output carrier and flange", 0.150, 0.045, None),
                (self.pairs[2][0], "Ring gear, Z 54", -0.170, 0.100, (0.030, 0.015, 0.036)),
                (self.pairs[1][1], "Front cover", -0.150, -0.075, (0.0, -0.050, -0.030), "Front cover"),
                (self.pairs[0][0], "Cast housing", 0.040, 0.075, (0.043, 0.004, 0.067), "Housing"),
                (self.pairs[0][1], "Oil sight glass", 0.060, -0.098, (0.0395, -0.060, -0.032),
                 ["Oil sight glass", "Oil sight glass boss"])]

    def exploded_labels(self):
        return [(self.input, "Input shaft and sun gear 1", -0.20, -0.07, None),
                (self.pairs[1][1], "Front cover", -0.15, 0.115, None),
                (self.planets1[1], "Stage 1 planets", -0.095, -0.115, None),
                (self.pairs[2][1], "Ring gear", -0.045, 0.125, None),
                (self.carrier1_inst, "Carrier 1, sun gear 2", 0.015, -0.13, None),
                (self.pairs[0][1], "Housing", 0.075, 0.125, None),
                (self.planets2[1], "Stage 2 planets", 0.17, 0.10, None),
                (self.output, "Output carrier, flange", 0.20, -0.09, None)]

    # checks and the report ------------------------------------------------------------

    def report(self):
        m, c = self.m, self.c
        lines = []
        add = lines.append
        names = {}
        for label, inst in [("Input shaft and sun 1", self.input), ("Carrier 1 and sun 2", self.carrier1_inst),
                            ("Output carrier and flange", self.output), ("Ring gear", self.ring),
                            ("Housing", self.body_inst), ("Front cover", self.cover_inst),
                            ("Input bearing balls", self.balls_in), ("Output bearing 1 balls", self.balls_out[0]),
                            ("Output bearing 2 balls", self.balls_out[1])]:
            names[inst.id] = label
        for k in range(NP):
            names[self.planets1[k].id] = "Stage 1 planet %d" % (k + 1)
            names[self.planets2[k].id] = "Stage 2 planet %d" % (k + 1)
        add("Two-stage planetary gearbox")
        add("=" * 60)
        add("tooth geometry")
        add("  module                    %.2f mm, pressure angle %.0f deg, backlash %.2f mm per gear (chordal, at the pitch circle)"
            % (MODULE, math.degrees(ALPHA), BACKLASH))
        add("  stage 1                   sun %d, %d planets of %d, ring %d" % (ZS1, NP, ZP1, ZR))
        add("  stage 2                   sun %d, %d planets of %d, ring %d (the same ring)" % (ZS2, NP, ZP2, ZR))
        add("  ring = sun + 2 x planet   stage 1: %d = %d + 2 x %d  %s;  stage 2: %d = %d + 2 x %d  %s"
            % (ZR, ZS1, ZP1, "ok" if ZR == ZS1 + 2 * ZP1 else "WRONG", ZR, ZS2, ZP2,
               "ok" if ZR == ZS2 + 2 * ZP2 else "WRONG"))
        add("  assembly condition        (sun + ring) / %d whole: stage 1 %d / %d = %d, stage 2 %d / %d = %d"
            % (NP, ZS1 + ZR, NP, (ZS1 + ZR) // NP, ZS2 + ZR, NP, (ZS2 + ZR) // NP))
        add("  pitch diameters           sun 1 %.1f, planet 1 %.1f, sun 2 %.1f, planet 2 %.1f, ring %.1f mm"
            % (MODULE * ZS1, MODULE * ZP1, MODULE * ZS2, MODULE * ZP2, MODULE * ZR))
        add("  centre distances          stage 1 %.2f mm, stage 2 %.2f mm" % (A1, A2))
        add("  face widths               stage 1 %.0f mm, stage 2 %.0f mm" % (STAGE1[1] - STAGE1[0], STAGE2[1] - STAGE2[0]))
        add("")
        i1 = 1 + ZR / ZS1
        i2 = 1 + ZR / ZS2
        add("ratios (ring fixed, sun driven, carrier out: i = 1 + Z_ring / Z_sun)")
        add("  stage 1                   %.4f" % i1)
        add("  stage 2                   %.4f" % i2)
        add("  overall                   %.4f  (%.4f x %.4f)" % (i1 * i2, i1, i2))
        # driving the input, reading the joints
        angle = 260.0
        self.j_in.drive(angle)
        vals = {j["id"]: j["value"] for j in c.query("joints")["joints"]}
        out = vals[self.j_out.id]
        add("  driven                    input turned %.1f deg: output reads %.4f deg, ratio %.4f"
            % (angle, out, angle / out))
        add("")
        add("joint values with the input at %.0f deg (measured | expected from the tooth counts)" % angle)
        rows = [(self.j_in, angle), (self.j_c1, angle * self.r1), (self.j_out, angle * self.r1 * self.r2)]
        rows += [(j, angle * self.k_p1) for j in self.j_p1]
        rows += [(j, angle * self.r1 * self.k_p2) for j in self.j_p2]
        for j, want in rows:
            add("  %-24s %10.4f | %10.4f deg" % (j.name, vals[j.id], want))
        # the planets' absolute speeds: the carrier plus the planet joint
        add("  planet turn, absolute     stage 1 %.4f deg, stage 2 %.4f deg"
            % (vals[self.j_c1.id] + vals[self.j_p1[0].id], vals[self.j_out.id] + vals[self.j_p2[0].id]))
        self.j_in.drive(0)
        add("")
        # mass
        every = m.mass_properties()
        rot = m.mass_properties(self.moving())
        add("mass (mass_properties, densities per material)")
        add("  whole gearbox             %.2f kg" % every["total"]["mass"])
        add("  rotating assembly         %.2f kg  (input shaft, both carriers, six planets, ball sets)" % rot["total"]["mass"])
        ine = rot["total"]["inertia"]
        # the model axis is X: inertia[0][0] is about the axis through the centroid, which is on it
        add("  rotating inertia, axis    %.3f kg cm2 (about the input axis, all moving parts)" % (ine[0][0] * 1e4))
        for label, inst in [("input shaft and sun 1", self.input), ("carrier 1 and sun 2", self.carrier1_inst),
                            ("output carrier and flange", self.output), ("one stage 1 planet", self.planets1[0]),
                            ("one stage 2 planet", self.planets2[0]), ("ring gear", self.ring),
                            ("housing", self.body_inst), ("front cover", self.cover_inst)]:
            r = m.mass_properties([inst])
            add("    %-26s %.3f kg" % (label, r["total"]["mass"]))
        add("")
        # interference and clearance
        gears = [self.input, self.carrier1_inst, self.output, self.ring] + self.planets1 + self.planets2
        r = m.interference(ids=gears, touching=True)
        add("interference (every pair of the gears, carriers, shafts and ring, at the build pose)")
        add("  overlapping pairs         %d" % len(r.get("pairs", [])))
        for p in r.get("pairs", []):
            add("    %s / %s  %.3g mm3" % (names.get(p["a"], p["a"]), names.get(p["b"], p["b"]), p["volume"] * 1e9))
        add("  touching pairs            %d" % len(r.get("touching", [])))
        for p in r.get("skipped", []):
            add("  skipped %s: %s" % (names.get(p["id"], p["id"]), p["reason"]))
        add("clearance between meshing parts (minimum distance between surfaces)")
        for label, a, b in [("sun 1 / planet 1.1", self.input, self.planets1[0]),
                            ("planet 1.1 / ring", self.planets1[0], self.ring),
                            ("sun 2 / planet 2.1", self.carrier1_inst, self.planets2[0]),
                            ("planet 2.1 / ring", self.planets2[0], self.ring),
                            ("planet 1.1 / planet 1.2", self.planets1[0], self.planets1[1]),
                            ("planet 2.1 / planet 2.2", self.planets2[0], self.planets2[1])]:
            d = m.clearance(a=a, b=b)["pairs"][0]
            add("  %-24s %6.3f mm%s" % (label, d["distance"] * 1000, "  OVERLAP" if d.get("overlap") else ""))
        steps = 9 if self.quick else 41
        sw = m.sweep_check(self.j_in, 0.0, 120.0, steps, ids=gears, report_within=0.0006)
        hit = [p for p in sw.get("pairs", []) if "first" in p]
        add("sweep: input turned 0 to 120 deg in %d steps (%.1f deg each; a sun-1 tooth pitch is %.0f deg)"
            % (steps, 120.0 / (steps - 1), 360.0 / ZS1))
        add("  pairs that interfere      %d" % len(hit))
        if sw.get("stopped"):
            add("  STOPPED at %s: %s" % (sw["stopped"]["value"], sw["stopped"]["reason"]))
        close = sorted(sw.get("pairs", []), key=lambda p: p["clearance"])[:6]
        for p in close:
            add("    %-40s closest %.3f mm at %.1f deg" % ("%s / %s" % (names.get(p["a"], p["a"]), names.get(p["b"], p["b"])),
                                                           p["clearance"] * 1000, p["clearance_at"]))
        add("")
        add("notes")
        add("  - the needle rollers ride with their planet; a real cage turns at about half the planet's speed")
        add("    about the pin, which is not rigged. The ball sets are: each turns at (r_inner contact) /")
        add("    (r_inner contact + r_outer contact) of the inner race, i.e. %.4f, %.4f, %.4f of its shaft."
            % (self.balls_in_ratio, self.balls_out_ratio[0], self.balls_out_ratio[1]))
        add("  - the planet joints are relative to their carrier (a revolute between carrier and planet); their")
        add("    couplings are -(Z_sun / Z_planet) x (sun turn - carrier turn), the planet's rolling on the sun.")
        add("  - tooth flanks are polylines of %d points each (involute, 20 deg); the ring's cutter is the" % self.flank)
        add("    conjugate gear, so the planets roll in it. Backlash is %.2f mm per external gear." % BACKLASH)
        add("  - masses and checks are of the whole parts; the quarter cutaway is two instances of each")
        add("    static part (housing, cover, ring) with section planes and is made after they are measured.")
        return "\n".join(lines) + "\n"

    # frames and the GIF ----------------------------------------------------------------

    def frames(self, out, count, size, supersample, turn, prefix="gearbox", camera=None, **opts):
        """``count`` frames of the input turning ``turn`` degrees; returns the paths."""
        fdir = os.path.join(out, "frames")
        os.makedirs(fdir, exist_ok=True)
        paths = []
        for k in range(count):
            a = turn * k / max(1, count - 1)
            p = os.path.join(fdir, "%s_%03d.png" % (prefix, k))
            self.m.render(p, size[0], size[1], lighting=None, ground=False, joints={self.j_in: a},
                          supersample=supersample, camera=camera or self.close_camera(), **opts)
            paths.append(p)
        return paths

    def gear_camera(self):
        """Closer on the gears, from the hero's direction."""
        c = (0.040, -0.006, 0.004)
        e = tuple(c[i] + 0.62 * (self.eye[i] - self.centre[i]) for i in range(3))
        return {"eye": list(e), "target": list(c), "fov_y_deg": 30}

    def close_camera(self):
        """A tighter view for the animation: the hero's direction, nearer and a little lower."""
        c = (self.centre[0], self.centre[1], self.centre[2] - 0.014)
        e = tuple(c[i] + 0.95 * (self.eye[i] - self.centre[i]) for i in range(3))
        return {"eye": list(e), "target": list(c), "fov_y_deg": 30}

    MESH_CAMERA = {"eye": [-0.215, 0.0, 0.0], "target": [0.024, 0.0, 0.0], "fov_y_deg": 30}
    MESH_SECTION = {"planes": False, "cuts": True, "fill": True, "fill_color": "dark"}

    def mesh_begin(self):
        """Set up the face-on view: a root section plane square to the axis through the middle of the
        stage-1 gear face cuts every part whole; the cutaway pieces, carriers, bearings and output are
        hidden and an unsplit copy of the ring shown, so sun, planets and ring stand clean."""
        c = self.c
        self.mesh_hidden = [i.id for pair in self.pairs for i in pair] + [self.carrier1_inst.id, self.output.id,
                                                                          self.balls_in.id] + [b.id for b in self.balls_out] + [p.id for p in self.planets2]
        c.execute({"type": "hide", "ids": self.mesh_hidden})
        c.execute({"type": "unhide", "ids": [self.ring_full.id]})
        r = c.execute({"type": "add_section_plane", "point": [0.024, 0, 0], "normal": [1, 0, 0]})
        self.mesh_plane = next(i for i in r["changes"]["created"] if i.startswith("s"))

    def mesh_end(self):
        c = self.c
        c.execute({"type": "erase", "ids": [self.mesh_plane]})
        c.execute({"type": "hide", "ids": [self.ring_full.id]})
        c.execute({"type": "unhide", "ids": self.mesh_hidden})

    def mesh_view(self, path, size, supersample):
        self.m.render(path, size[0], size[1], lighting=None, ground=False, supersample=supersample,
                      camera=self.MESH_CAMERA, section=self.MESH_SECTION)
        return path

    def gif(self, out, paths, width, fps=20, name="gearbox-motion.gif", colors=96):
        ffmpeg = shutil.which("ffmpeg")
        if ffmpeg is None or not paths:
            return None
        # forward, then back: a loop with no jump
        seq = paths + paths[-2:0:-1]
        fdir = os.path.join(out, "frames")
        for n, p in enumerate(seq):
            tmp = os.path.join(fdir, "loop_%s_%03d.png" % (name[:-4], n))
            shutil.copyfile(p, tmp)
        path = os.path.join(out, name)
        vf = "scale=%d:-1:flags=lanczos,split[a][b];[a]palettegen=max_colors=%d:stats_mode=diff[p];[b][p]paletteuse=dither=bayer:bayer_scale=5:diff_mode=rectangle" % (width, colors)
        cmd = [ffmpeg, "-y", "-loglevel", "error", "-framerate", str(fps), "-i", os.path.join(fdir, "loop_%s_%%03d.png" % name[:-4]),
               "-vf", vf, "-loop", "0", path]
        r = subprocess.run(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, timeout=240)
        for n in range(len(seq)):
            os.remove(os.path.join(fdir, "loop_%s_%03d.png" % (name[:-4], n)))
        if r.returncode != 0:
            print("  ffmpeg failed: %s" % r.stderr.decode(errors="replace").strip()[:300])
            return None
        return path

    # the exploded view ------------------------------------------------------------------

    def explode(self):
        """Move the parts apart along the axis (a view only: the joints are not met any more)."""
        spread = [([self.input, self.balls_in], -190.0), (list(self.pairs[1]), -125.0), (self.planets1, -112.0),
                  (list(self.pairs[2]), -88.0), ([self.carrier1_inst], -45.0), (list(self.pairs[0]), 0.0),
                  (self.planets2, 65.0), ([self.output] + self.balls_out, 105.0)]
        for insts, d in spread:
            for i in insts:
                if d:
                    i.move((d * K, 0, 0))


def _add(a, b):
    return tuple(x + y for x, y in zip(a, b))


def _sub(a, b):
    return tuple(x - y for x, y in zip(a, b))


def _mul(a, k):
    return tuple(x * k for x in a)


def _unit(a):
    n = math.sqrt(sum(x * x for x in a))
    return _mul(a, 1.0 / n)


def _cross(a, b):
    return (a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0])


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--socket", help="socket of a running app (default: start a headless one)")
    ap.add_argument("--app", help="app binary to start (default: TENON_APP or the repo's target/)")
    ap.add_argument("--out", default=".", help="directory for the images and the report (default: here)")
    ap.add_argument("--quick", action="store_true", help="coarse teeth, small images, no frames (for tests)")
    ap.add_argument("--frames", type=int, help="frames of the GIF (default 60, quick 0)")
    ap.add_argument("--width", type=int, help="image width (default 1600, quick 320)")
    ap.add_argument("--height", type=int, help="image height (default 1000, quick 200)")
    args = ap.parse_args(argv)
    q = args.quick
    width = args.width or (320 if q else 1600)
    height = args.height or (200 if q else 1000)
    ss = 1 if q else 2
    frames = args.frames if args.frames is not None else (0 if q else 60)
    os.makedirs(args.out, exist_ok=True)
    t0 = time.time()
    timings = []

    def timed(name, fn, *a, **kw):
        t = time.time()
        r = fn(*a, **kw)
        timings.append((name, time.time() - t))
        print("  %-24s %6.1f s" % (name, time.time() - t))
        return r

    with session(args.socket, args.app) as s:
        m = Model(s.client)
        s.client.execute({"type": "new", "discard": s.launched})
        print("two-stage planetary gearbox")
        g = Gearbox(m, q)
        timed("build", g.build, lambda *a: None)
        timed("rig", g.rig)
        text = timed("checks and report", g.report)
        timed("cutaway", g.cutaway)
        g.look()
        gif = gif2 = None
        mesh = os.path.join(args.out, "gearbox-mesh.png")
        g.mesh_begin()
        timed("mesh section", g.mesh_view, mesh, (height, height), ss)
        if frames:
            paths = timed("mesh frames", g.frames, args.out, frames, (640, 640), 2, 360.0, "mesh",
                          g.MESH_CAMERA, section=g.MESH_SECTION)
            gif = timed("mesh gif", g.gif, args.out, paths, 640, 20, "gearbox-motion.gif", 64)
        g.mesh_end()
        if frames:
            paths = timed("cutaway frames", g.frames, args.out, frames, (640, 400), 2, 360.0, "cut", g.gear_camera())
            gif2 = timed("cutaway gif", g.gif, args.out, paths, 640, 20, "gearbox-motion-cutaway.gif", 96)
        g.notes = timed("labels", g.callouts, g.hero_labels(), g.eye, g.centre, 0.31, 0.82)
        hero = os.path.join(args.out, "gearbox.png")
        timed("hero render", m.render, hero, width, height, lighting=None, supersample=ss, ground=False)
        timed("explode", g.explode)
        g.erase_notes(g.notes)
        eye, target = (-0.19, -0.50, 0.27), (-0.008, 0.0, 0.0)
        timed("exploded labels", g.callouts, g.exploded_labels(), eye, target, 0.3, 0.8)
        ex = os.path.join(args.out, "gearbox-exploded.png")
        timed("exploded render", m.render, ex, width, height, lighting=None, supersample=ss, ground=False,
              camera={"eye": list(eye), "target": list(target), "fov_y_deg": 30})
    rpath = os.path.join(args.out, "gearbox-report.txt")
    with open(rpath, "w") as f:
        f.write(text + "\nbuilt, rigged, checked and rendered in %.1f s\n" % (time.time() - t0))
    print("done in %.1f s: %s" % (time.time() - t0, ", ".join(p for p in (hero, ex, mesh, gif, gif2, rpath) if p)))


if __name__ == "__main__":
    main()
