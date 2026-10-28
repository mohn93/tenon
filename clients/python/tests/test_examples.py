"""The examples run end to end against a headless app (small ``--quick`` builds) and write
PNG files that are not blank. Skipped without an app binary; the renders skip without a GPU."""
import io
import os
import shutil
import struct
import sys
import tempfile
import unittest
import zlib
from contextlib import redirect_stdout

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, ".."))
sys.path.insert(0, os.path.join(HERE, "..", "examples"))

from tenon import ApiError, find_app  # noqa: E402

APP = find_app()


def read_png(path):
    """(width, height, rows) of an 8-bit RGBA or RGB PNG; rows are lists of pixel tuples."""
    with open(path, "rb") as f:
        data = f.read()
    if data[:8] != b"\x89PNG\r\n\x1a\n":
        raise AssertionError("%s is not a PNG" % path)
    pos, idat = 8, b""
    while pos < len(data):
        n, kind = struct.unpack(">I4s", data[pos : pos + 8])
        body = data[pos + 8 : pos + 8 + n]
        if kind == b"IHDR":
            w, h, depth, ctype = struct.unpack(">IIBB", body[:10])
            if depth != 8 or ctype not in (2, 6):
                raise AssertionError("unexpected PNG format %d/%d" % (depth, ctype))
            bpp = 4 if ctype == 6 else 3
        elif kind == b"IDAT":
            idat += body
        pos += 12 + n
    raw = zlib.decompress(idat)
    stride = w * bpp
    prev = bytearray(stride)
    rows = []
    for y in range(h):
        f = raw[y * (stride + 1)]
        line = bytearray(raw[y * (stride + 1) + 1 : (y + 1) * (stride + 1)])
        for x in range(stride):
            a = line[x - bpp] if x >= bpp else 0
            b = prev[x]
            c = prev[x - bpp] if x >= bpp else 0
            if f == 1:
                line[x] = (line[x] + a) & 255
            elif f == 2:
                line[x] = (line[x] + b) & 255
            elif f == 3:
                line[x] = (line[x] + (a + b) // 2) & 255
            elif f == 4:
                pa, pb, pc = abs(b - c), abs(a - c), abs(a + b - 2 * c)
                pr = a if pa <= pb and pa <= pc else (b if pb <= pc else c)
                line[x] = (line[x] + pr) & 255
        rows.append([tuple(line[i : i + 3]) for i in range(0, stride, bpp)])
        prev = line
    return w, h, rows


def drawn_fraction(rows):
    """The share of pixels that differ clearly from their row's first pixel (the background,
    which only changes from row to row)."""
    n = hit = 0
    for row in rows:
        bg = row[0]
        for p in row:
            n += 1
            if max(abs(p[0] - bg[0]), abs(p[1] - bg[1]), abs(p[2] - bg[2])) > 40:
                hit += 1
    return hit / n


@unittest.skipIf(APP is None, "no tenon app binary found")
class ExampleTests(unittest.TestCase):
    def setUp(self):
        self.out = tempfile.mkdtemp(prefix="skex")

    def tearDown(self):
        shutil.rmtree(self.out, ignore_errors=True)

    def run_example(self, module, names, extra=()):
        mod = __import__(module)
        try:
            with redirect_stdout(io.StringIO()):
                mod.main(["--quick", "--app", APP, "--out", self.out, *extra])
        except ApiError as e:
            if e.kind == "rendering_unavailable":
                self.skipTest("no GPU: %s" % e)
            raise
        for name, (w, h) in names.items():
            path = os.path.join(self.out, name)
            self.assertTrue(os.path.isfile(path), name)
            pw, ph, rows = read_png(path)
            self.assertEqual((pw, ph), (w, h), name)
            self.assertGreater(drawn_fraction(rows), 0.08, "%s looks empty" % name)

    def test_v6_engine(self):
        self.run_example("v6_engine", {"v6.png": (320, 200), "v6-front.png": (320, 200), "v6-motion.png": (160, 100)})

    def test_v6_cycle(self):
        """A quick cycle: frames and a contact sheet that are not blank, a report with its
        measurements, and no moving part running into another."""
        mod = __import__("v6_cycle")
        try:
            with redirect_stdout(io.StringIO()):
                mod.main(["--quick", "--app", APP, "--out", self.out, "--frames", "6"])
        except ApiError as e:
            if e.kind == "rendering_unavailable":
                self.skipTest("no GPU: %s" % e)
            raise
        frames = sorted(os.listdir(os.path.join(self.out, "frames")))
        self.assertEqual(frames, ["v6_%03d.png" % k for k in range(6)])
        for name in (frames[0], frames[3]):
            w, h, rows = read_png(os.path.join(self.out, "frames", name))
            self.assertEqual((w, h), (320, 200))
            self.assertGreater(drawn_fraction(rows), 0.08, "%s looks empty" % name)
        w, h, rows = read_png(os.path.join(self.out, "v6-cycle.png"))
        self.assertEqual((w, h), (3 * 160 + 4 * 4, 2 * 100 + 3 * 4))
        with open(os.path.join(self.out, "v6-report.txt")) as f:
            report = f.read()
        for field in ("bore ", "stroke ", "displacement ", "compression ", "engine ", "crankshaft ",
                      "firing order    1-2 ", "Valve 1 intake front", "Valve 2 exhaust rear",
                      "piston to valve", "piston to head", "valve to cam"):
            self.assertIn(field, report)
        self.assertIn("moving parts interfering (among the pairs measured): 0\n", report)
        self.assertIn("parts skipped by the sweep (not solid): 0\n", report)
        self.assertNotIn("moving pairs whose overlap could not be measured", report)
        self.assertNotIn("STOPPED", report)

    def test_city(self):
        self.run_example("city", {"city.png": (320, 200)})

    def test_barn(self):
        self.run_example("barn", {n: (320, 200) for n in
                                  ("barn.png", "barn-joint.png", "barn-section.png", "barn-sketch.png")})
        self.assertTrue(os.path.isfile(os.path.join(self.out, "barn-cutlist.txt")))

    def test_gearbox(self):
        self.run_example("gearbox", {"gearbox.png": (320, 200), "gearbox-exploded.png": (320, 200),
                                     "gearbox-mesh.png": (200, 200)})
        with open(os.path.join(self.out, "gearbox-report.txt")) as f:
            self.assertIn("ratio 13.0000", f.read())

    def test_villa(self):
        self.run_example("villa", {n: (320, 200) for n in
                                   ("villa.png", "villa-dusk.png", "villa-section.png", "villa-plan.png")})
        self.assertTrue(os.path.isfile(os.path.join(self.out, "villa.tnn")))

    def test_dubai(self):
        self.run_example("dubai", {n: (320, 200) for n in
                                   ("dubai.png", "dubai-night.png", "dubai-khalifa.png", "dubai-palm.png", "dubai-szr.png")})
        self.assertTrue(os.path.isfile(os.path.join(self.out, "dubai.tnn")))

    def test_robot_cell(self):
        self.run_example("robot_cell", {n: (320, 200) for n in
                                        ("robot.png", "robot-detail.png", "robot-section.png")})
        self.assertTrue(os.path.isfile(os.path.join(self.out, "robot-report.txt")))

if __name__ == "__main__":
    unittest.main()
