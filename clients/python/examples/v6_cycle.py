#!/usr/bin/env python3
"""The rigged V6 through a whole cycle (720 crank degrees): frames, a contact sheet, a movie
and a report measured from the model.

    python3 v6_cycle.py [--socket PATH] [--out DIR] [--frames N] [--quick]

It builds the engine of ``v6_engine.py`` (every moving part a root instance on joints, the
cams coupled to the crank, the valves following their lobes), then:

- drives the crank through the cycle, rendering ``frames/v6_000.png``... from the cutaway
  camera with ``render_view``'s ``joints`` (N frames, default 72: one every 10 degrees) and
  reading every joint's value as it goes (piston travel, valve lift);
- composes ``v6-cycle.png``, a contact sheet of 12 small renders a sixth of a turn apart
  (PNG read and written here with zlib), and ``v6-cycle.mp4`` when ``ffmpeg`` is on PATH;
- checks the whole cycle with ``sweep_check`` (every part but the timing chains, which do not
  move in this model; then the pistons against the valves, finer), and measures the bore
  (``pick`` and ``measure``), the stroke (the piston joints), the masses
  (``mass_properties``) and the combustion chamber, and writes ``v6-report.txt``.
"""
import argparse
import math
import os
import shutil
import struct
import subprocess
import sys
import time
import zlib

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, ".."))
sys.path.insert(0, HERE)

from sketchup_alt import Model, session  # noqa: E402

import v6_engine  # noqa: E402
from v6_engine import DECK, GASKET, X, add, cylinder_number, mul, sub  # noqa: E402

# ---------------------------------------------------------------- PNG (stdlib only)


def png_read(data):
    """(width, height, rows) of an 8-bit RGB or RGBA PNG (bytes); rows are RGB bytearrays."""
    if data[:8] != b"\x89PNG\r\n\x1a\n":
        raise ValueError("not a PNG")
    pos, idat = 8, []
    while pos < len(data):
        n, kind = struct.unpack(">I4s", data[pos:pos + 8])
        body = data[pos + 8:pos + 8 + n]
        if kind == b"IHDR":
            w, h, depth, ctype, _, _, interlace = struct.unpack(">IIBBBBB", body)
            if depth != 8 or ctype not in (2, 6) or interlace:
                raise ValueError("unsupported PNG (depth %d, colour type %d)" % (depth, ctype))
            bpp = 4 if ctype == 6 else 3
        elif kind == b"IDAT":
            idat.append(body)
        pos += 12 + n
    raw = zlib.decompress(b"".join(idat))
    stride = w * bpp
    prev = bytearray(stride)
    rows = []
    for y in range(h):
        at = y * (stride + 1)
        f = raw[at]
        line = bytearray(raw[at + 1:at + 1 + stride])
        if f == 1:
            for x in range(bpp, stride):
                line[x] = (line[x] + line[x - bpp]) & 255
        elif f == 2:
            line = bytearray((a + b) & 255 for a, b in zip(line, prev))
        elif f == 3:
            for x in range(stride):
                left = line[x - bpp] if x >= bpp else 0
                line[x] = (line[x] + ((left + prev[x]) >> 1)) & 255
        elif f == 4:
            for x in range(stride):
                a = line[x - bpp] if x >= bpp else 0
                b = prev[x]
                c = prev[x - bpp] if x >= bpp else 0
                pa, pb, pc = abs(b - c), abs(a - c), abs(a + b - 2 * c)
                line[x] = (line[x] + (a if pa <= pb and pa <= pc else (b if pb <= pc else c))) & 255
        rows.append(line)
        prev = line
    if bpp == 4:  # drop alpha
        rows = [bytearray(b for i, b in enumerate(r) if i % 4 != 3) for r in rows]
    return w, h, rows


def png_write(path, w, h, rows):
    """An 8-bit RGB PNG from ``h`` rows of ``3 * w`` bytes each."""
    def chunk(kind, body):
        return struct.pack(">I", len(body)) + kind + body + struct.pack(">I", zlib.crc32(kind + body) & 0xFFFFFFFF)

    raw = b"".join(b"\x00" + bytes(r) for r in rows)
    with open(path, "wb") as f:
        f.write(b"\x89PNG\r\n\x1a\n")
        f.write(chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0)))
        f.write(chunk(b"IDAT", zlib.compress(raw, 6)))
        f.write(chunk(b"IEND", b""))


def contact_sheet(path, tiles, columns, gap=4, background=(255, 255, 255)):
    """Lay out PNG ``tiles`` (bytes, all one size) in rows of ``columns``, ``gap`` pixels apart."""
    decoded = [png_read(t) for t in tiles]
    tw, th = decoded[0][0], decoded[0][1]
    rows_n = (len(decoded) + columns - 1) // columns
    w = columns * tw + (columns + 1) * gap
    h = rows_n * th + (rows_n + 1) * gap
    blank = bytearray(background) * w
    out = [bytearray(blank) for _ in range(h)]
    for k, (_, _, rows) in enumerate(decoded):
        x0 = gap + (k % columns) * (tw + gap)
        y0 = gap + (k // columns) * (th + gap)
        for y, r in enumerate(rows):
            out[y0 + y][3 * x0:3 * (x0 + tw)] = r
    png_write(path, w, h, out)
    return w, h


# ---------------------------------------------------------------- the cycle

# looking into the cutaway bank A from the front and below the cover: pistons, rods, crank,
# valves and the exhaust cam
TARGET = (0.06, -0.02, 0.13)
CAMERA = {"eye": [TARGET[0] - 0.55, TARGET[1] - 1.0, TARGET[2] + 0.25], "target": list(TARGET), "fov_y_deg": 30}


def norm(v):
    return mul(v, 1 / math.sqrt(sum(x * x for x in v)))


def cross(a, b):
    return (a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0])


def angle_range(step, end=720.0):
    n = int(round(end / step))
    return [round(step * k, 9) for k in range(n)]


class Cycle:
    def __init__(self, m, v, out, log):
        self.m, self.v, self.out, self.log = m, v, out, log
        self.c = m.client
        self.samples = {}  # crank angle -> {joint id: value}
        self.timings = []

    def timed(self, name, fn, *args):
        t = time.time()
        r = fn(*args)
        self.timings.append((name, time.time() - t))
        self.log("  %-26s %6.1f s" % (name, time.time() - t))
        return r

    def run_cycle(self, frames, sample_step, size, tile_size, tiles, supersample):
        """Drive the crank through 0..720, read every joint at each sample angle and render the
        frames and contact-sheet tiles at theirs (``render_view`` with the crank's value)."""
        v, m = self.v, self.m
        fdir = os.path.join(self.out, "frames")
        os.makedirs(fdir, exist_ok=True)
        frame_at = {round(720.0 * k / frames, 9): k for k in range(frames)}
        tile_at = {round(720.0 * k / tiles, 9): k for k in range(tiles)}
        angles = sorted(set(angle_range(sample_step)) | set(frame_at) | set(tile_at))
        self.tiles = [None] * tiles
        self.frame_paths = []
        crank = v.crank_joint
        for a in angles:
            crank.drive(a)
            self.samples[a] = {j["id"]: j["value"] for j in self.c.query("joints")["joints"]}
            if a in frame_at:
                path = os.path.join(fdir, "v6_%03d.png" % frame_at[a])
                m.render(path, size[0], size[1], camera=CAMERA, lighting=None, ground=False, joints={crank: a},
                         supersample=supersample)
                self.frame_paths.append(path)
            if a in tile_at:
                self.tiles[tile_at[a]] = self.c.render_view(tile_size[0], tile_size[1], camera=CAMERA, ground=False,
                                                            joints={crank.id: a})
        crank.drive(0)

    def sheet(self, columns):
        path = os.path.join(self.out, "v6-cycle.png")
        contact_sheet(path, self.tiles, columns)
        return path

    def movie(self, fps=24):
        ffmpeg = shutil.which("ffmpeg")
        if ffmpeg is None:
            return None
        path = os.path.join(self.out, "v6-cycle.mp4")
        cmd = [ffmpeg, "-y", "-loglevel", "error", "-framerate", str(fps),
               "-i", os.path.join(self.out, "frames", "v6_%03d.png"),
               "-c:v", "libx264", "-pix_fmt", "yuv420p", "-vf", "scale=trunc(iw/2)*2:trunc(ih/2)*2", path]
        r = subprocess.run(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, timeout=120)
        if r.returncode != 0:
            self.log("  ffmpeg failed: %s" % r.stderr.decode(errors="replace").strip()[:300])
            return None
        return path

    # ---- checks

    def sweeps(self, steps, fine_steps):
        v, m = self.v, self.m
        roots = self.c.query("outliner", depth=0)["instances"]
        self.names = {e["id"]: e.get("name") or "group" for e in roots}
        chains = {c.id for c in v.chains}
        self.checked = [e["id"] for e in roots if e["id"] not in chains]
        self.moving = {i.id for i in v.moving()}
        self.sweep_all = m.sweep_check(v.crank_joint, 0, 720, steps, ids=self.checked)
        pistons = [p for _, _, p, _ in v.cyl_parts]
        valves = [x[4] for x in v.valves]
        self.sweep_pv = m.sweep_check(v.crank_joint, 0, 720, fine_steps, ids=pistons + valves, report_within=0.03)

    # ---- measurements

    def joint(self, name):
        return next(j for j in self.c.query("joints")["joints"] if j["name"] == name)

    def measure_bore(self):
        """The bore of cylinder 1: a ray from its axis (the piston joint's) across to the wall,
        the first face of the block it meets, and the cylinder ``measure`` fits to it."""
        j = self.joint("Piston 1")
        axis = norm(j["axis"])
        across = norm(cross(axis, X))
        block = self.v.block_inst.id
        hits = self.c.query("pick", ray={"origin": list(j["point"]), "direction": list(across)})["hits"]
        face = next(h for h in hits if h["path"] and h["path"][0] == block)
        r = self.m.measure(round=face["path"] + [face["id"]])
        return r, axis, j["point"]

    def chamber(self, bore, axis, wrist):
        """Cylinder 1's combustion chamber at top dead centre (crank 0): the crown and the
        chamber roof found by a ray up the bore beside the axis, then a solid built to the
        chamber's shape (the clearance above the crown in the bore, the gasket's bore, the
        head's chamber recess, the crown's four valve reliefs) weighed by ``mass_properties``."""
        v, m = self.v, self.m
        origin = add(wrist, mul(X, 0.01))
        hits = self.c.query("pick", ray={"origin": list(origin), "direction": list(axis)})["hits"]
        piston = v.cyl_parts[0][2].id
        head = v.heads["A"].id
        crown_t = max(h["distance"] for h in hits if h["path"] and h["path"][0] == piston)
        roof_t = min(h["distance"] for h in hits if h["path"] and h["path"][0] == head and h["distance"] > crown_t)
        crown = add(wrist, mul(axis, crown_t))
        s_crown = crown_t  # along the axis from the wrist pin
        s_deck = sum((p - q) * a for p, q, a in zip(v6_engine.FRAME["A"].apply((0, 0, DECK)), wrist, axis))
        s0 = s_deck + GASKET
        parts = [
            m.cylinder(crown, bore, s_deck - s_crown, axis=axis, segments=96),
            m.cylinder(add(wrist, mul(axis, s_deck)), v6_engine.BORE + 0.0015, GASKET, axis=axis, segments=96),
            m.cylinder(add(wrist, mul(axis, s0)), v6_engine.BORE - 0.0015, roof_t - s0, axis=axis, segments=96),
        ]
        # the reliefs: 1.5 mm deep, 16.5 mm radius, over each valve (piston coordinates)
        t = v.cyl_parts[0][2].transform()
        for dx in v6_engine.RELIEF_DX:
            for dy in v6_engine.RELIEF_DY:
                base = t.apply((dx, dy, v6_engine.CROWN_Z - v6_engine.RELIEF_DEPTH))
                parts.append(m.cylinder(base, v6_engine.RELIEF_R, v6_engine.RELIEF_DEPTH, axis=axis, segments=96))
        vol = m.mass_properties(parts)["total"]["volume"]
        self.c.execute({"type": "erase", "ids": [p.id for p in parts]})
        return vol, roof_t - crown_t

    def masses(self):
        v, m = self.v, self.m
        everything = m.mass_properties()
        crank = m.mass_properties([v.crank])["total"]
        piston = m.mass_properties([v.cyl_parts[0][2]])["total"]
        rod = m.mass_properties([v.cyl_parts[0][3]])["total"]
        return everything, crank, piston, rod

    # ---- the report

    def report(self, bore_fit, stroke_by, chamber_vol, chamber_h, masses, runtime):
        v = self.v
        js = {j["id"]: j for j in self.c.query("joints")["joints"]}
        lines = []
        add_ = lines.append
        n_cyl = len(v.cyl_parts)
        r = bore_fit["radius"]
        stroke = max(stroke_by.values())
        swept = math.pi * r * r * stroke
        add_("V6 cycle report (measured from the model)")
        add_("")
        add_("Geometry")
        add_("  bore            %.2f mm  (measure round on cylinder 1's wall: a %s through the faceted bore's "
             "corners, fit deviation %.3f mm)" % (2000 * r, bore_fit["kind"], 1000 * bore_fit["max_deviation"]))
        add_("  stroke          %.2f mm  (piston joint travel, top to bottom dead centre; %s)"
             % (1000 * stroke, ", ".join("%d: %.2f" % (k, 1000 * s) for k, s in sorted(stroke_by.items()))))
        add_("  displacement    %.0f cc per cylinder, %.3f L for %d cylinders" % (swept * 1e6, swept * n_cyl * 1e3, n_cyl))
        if chamber_vol:
            add_("  chamber         %.1f cc at top dead centre (crown to roof %.2f mm; a solid built to the chamber: "
                 "deck clearance, gasket bore, head recess, valve reliefs; throats and plug hole left out)"
                 % (chamber_vol * 1e6, chamber_h * 1000))
            add_("  compression     %.1f : 1" % ((swept + chamber_vol) / chamber_vol))
        everything, crank, piston, rod = masses
        tot = everything.get("total", {})
        add_("")
        add_("Mass (densities: steel 7850, cast iron 7200, aluminium 2700, copper 8960 kg/m3)")
        if "mass" in tot:
            add_("  engine          %.1f kg, centre of gravity (%.3f, %.3f, %.3f) m" % ((tot["mass"],) + tuple(tot["centroid"])))
        else:
            add_("  engine          volume %.4f m3 (a part has no density)" % tot.get("volume", 0))
        if everything.get("skipped"):
            add_("  (%d open pieces left out: %s)" % (len(everything["skipped"]), ", ".join(s["path"] for s in everything["skipped"][:4])))
        add_("  crankshaft      %.2f kg (with sprockets, damper and flywheel)" % crank.get("mass", float("nan")))
        add_("  piston          %.3f kg (with pin and rings), rod %.3f kg" % (piston.get("mass", float("nan")), rod.get("mass", float("nan"))))

        # firing order and timing from the samples
        angles = sorted(self.samples)
        step = angles[1] - angles[0]
        tdc = {}
        for b, i, piston_i, _ in v.cyl_parts:
            pj = v.piston_joints[(b, i)].id
            vals = [(self.samples[a][pj], a) for a in angles]
            top = max(x for x, _ in vals)
            tops = [a for x, a in vals if abs(x - top) < 1e-9]
            # compression: the top dead centre with both valves shut 30 degrees either side
            vj = [j for (bb, ii, _, _, _, j) in v.valve_joints if (bb, ii) == (b, i)]

            def lift_near(a):
                near = [round((a + d) % 720.0, 9) for d in (-30.0, 30.0)]
                return max(self.samples[n][j.id] for n in near if n in self.samples for j in vj) if near else 0

            tdc[cylinder_number(b, i)] = min(tops, key=lift_near)
        order = sorted(tdc, key=lambda c: tdc[c])
        add_("")
        add_("Cycle")
        add_("  firing order    %s (top dead centre of compression at %s degrees)"
             % ("-".join(str(c) for c in order), ", ".join("%d: %.0f" % (c, tdc[c]) for c in order)))
        add_("  valve lift      (max lift and crank angle; the cams turn at half crank speed)")
        for b, i, side, dx, inst, j in sorted(v.valve_joints, key=lambda t: (cylinder_number(t[0], t[1]), -t[2], t[3])):
            vals = [(self.samples[a][j.id], a) for a in angles]
            top, at = max(vals)
            add_("    %-28s %5.2f mm at %3.0f" % (js[j.id]["name"], top * 1000, at))
        add_("  every drive kept every valve on its cam (a follower losing contact refuses the drive)")

        # interference
        sw = self.sweep_all
        hits = [p for p in sw["pairs"] if "first" in p and (p["a"] in self.moving or p["b"] in self.moving)]
        static_hits = [p for p in sw["pairs"] if "first" in p and not (p["a"] in self.moving or p["b"] in self.moving)]
        failed = [p for p in sw["pairs"] if p.get("failed") and (p["a"] in self.moving or p["b"] in self.moving)]
        static_failed = [p for p in sw["pairs"] if p.get("failed") and p not in failed]
        add_("")
        add_("Interference (sweep_check over 720 degrees in %d steps of %.0f degrees, %d parts: every part but the "
             "two timing chains, which stand still in this model while their sprockets turn)"
             % (sw["steps"], 720.0 / max(1, sw["steps"] - 1), len(self.checked)))
        if sw.get("stopped"):
            add_("  STOPPED at %s: %s" % (sw["stopped"]["value"], sw["stopped"]["reason"]))
        skipped = sw.get("skipped") or []
        add_("  parts skipped by the sweep (not solid): %d%s" % (len(skipped), "" if not skipped else ": " + "; ".join(
            "%s: %s" % (self.names.get(s["id"], s["id"]), s["reason"]) for s in skipped[:4])))
        add_("  moving parts interfering (among the pairs measured): %d%s" % (len(hits), "" if not hits else ": " + "; ".join(
            "%s / %s %g..%g" % (self.names[p["a"]], self.names[p["b"]], p["first"], p["last"]) for p in hits)))
        if failed:
            add_("  moving pairs whose overlap could not be measured: %d (%s)" % (len(failed), "; ".join(
                "%s / %s: %s" % (self.names[p["a"]], self.names[p["b"]], p["failed"]["reason"]) for p in failed[:3])))
        add_("  static parts set into each other (bolts, pipes and fittings no joint moves): %d pairs%s"
             % (len(static_hits), "" if not static_failed else ", %d more not measurable (%s)" % (
                 len(static_failed), static_failed[0]["failed"]["reason"])))

        def nearest(pairs, a_names, b_names):
            best = None
            for p in pairs:
                na, nb = self.names.get(p["a"]), self.names.get(p["b"])
                if (na in a_names and nb in b_names) or (nb in a_names and na in b_names):
                    if best is None or p["clearance"] < best["clearance"]:
                        best = p
            return best

        add_("  smallest clearances:")
        for label, pairs, a_names, b_names in (
                ("piston to valve", self.sweep_pv["pairs"], {"Piston"}, {"Intake valve", "Exhaust valve"}),
                ("piston to head", sw["pairs"], {"Piston"}, {"Cylinder head"}),
                ("piston to head gasket", sw["pairs"], {"Piston"}, {"Head gasket"}),
                ("piston to block", sw["pairs"], {"Piston"}, {"Block"}),
                ("rod to block", sw["pairs"], {"Connecting rod"}, {"Block"}),
                ("crankshaft to block", sw["pairs"], {"Crankshaft"}, {"Block"}),
                ("crankshaft to main bearing", sw["pairs"], {"Crankshaft"}, {"Main bearing"}),
                ("valve to head", sw["pairs"], {"Intake valve", "Exhaust valve"}, {"Cylinder head"}),
                ("valve to cam (lash)", sw["pairs"], {"Intake valve", "Exhaust valve"}, {"Camshaft"})):
            p = nearest(pairs, a_names, b_names)
            if p is None:
                add_("    %-28s none within the sweep's report distance" % label)
            else:
                add_("    %-28s %6.2f mm at %3.0f degrees" % (label, p["clearance"] * 1000, p["clearance_at"]))
        add_("")
        add_("Runtime")
        for name, t in self.timings:
            add_("  %-26s %6.1f s" % (name, t))
        add_("  %-26s %6.1f s" % ("total", runtime))
        return "\n".join(lines) + "\n"


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--socket", help="socket of a running app (default: start a headless one)")
    ap.add_argument("--app", help="app binary to start (default: SKETCHUP_ALT_APP or the repo's target/)")
    ap.add_argument("--out", default=".", help="directory for the frames, sheet, movie and report (default: here)")
    ap.add_argument("--frames", type=int, help="frames over the 720 degrees (default 72, quick 6)")
    ap.add_argument("--quick", action="store_true", help="one cylinder per bank, few frames, coarse checks (for tests)")
    ap.add_argument("--width", type=int, help="frame width (default 1280, quick 320)")
    ap.add_argument("--height", type=int, help="frame height (default 800, quick 200)")
    args = ap.parse_args(argv)
    q = args.quick
    frames = args.frames or (6 if q else 72)
    size = (args.width or (320 if q else 1280), args.height or (200 if q else 800))
    tiles = min(12, frames) if q else 12
    tile_size = (160, 100) if q else (480, 300)
    sample_step = 30.0 if q else 10.0
    steps, fine_steps = (13, 13) if q else (73, 145)
    os.makedirs(args.out, exist_ok=True)
    t0 = time.time()
    with session(args.socket, args.app) as s:
        m = Model(s.client)
        s.client.execute({"type": "new", "discard": s.launched})
        cyc = None

        def build():
            return v6_engine.build(m, q, log=lambda *a: None)

        log = print
        log("V6 cycle")
        cyc = Cycle(m, None, args.out, log)
        v = cyc.timed("build and rig", build)
        cyc.v = v
        v.look()
        cyc.timed("frames and samples", cyc.run_cycle, frames, sample_step, size, tile_size, tiles, 1 if q else 2)
        sheet = cyc.timed("contact sheet", cyc.sheet, 4 if tiles > 6 else 3)
        mp4 = cyc.timed("movie", cyc.movie)
        cyc.timed("sweep checks", cyc.sweeps, steps, fine_steps)
        bore_fit, axis, wrist = cyc.timed("bore", cyc.measure_bore)
        stroke_by = {}
        angles = sorted(cyc.samples)
        for b, i, _, _ in v.cyl_parts:
            vals = [cyc.samples[a][v.piston_joints[(b, i)].id] for a in angles]
            stroke_by[cylinder_number(b, i)] = max(vals) - min(vals)
        chamber_vol, chamber_h = cyc.timed("chamber", cyc.chamber, bore_fit["radius"], axis, wrist)
        masses = cyc.timed("mass", cyc.masses)
        text = cyc.report(bore_fit, stroke_by, chamber_vol, chamber_h, masses, time.time() - t0)
    path = os.path.join(args.out, "v6-report.txt")
    with open(path, "w") as f:
        f.write(text)
    print(text)
    print("frames: %s (%d)\ncontact sheet: %s\nmovie: %s\nreport: %s"
          % (os.path.join(args.out, "frames"), len(cyc.frame_paths), sheet, mp4 or "(no ffmpeg)", path))
    return cyc


if __name__ == "__main__":
    main()
