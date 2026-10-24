"""Tests for Transform (pure) and Model (against a launched headless app).

Helpers for milestones not merged yet (solid tools, material finish, studio lighting,
labels, scene positions) are skipped until the app's `describe` lists them.
"""
import math
import os
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

from sketchup_alt import ApiError, Model, Transform, find_app, launch_headless  # noqa: E402

APP = find_app()


def close(a, b, tol=1e-9):
    return all(abs(x - y) < tol for x, y in zip(a, b))


class TransformTests(unittest.TestCase):
    def test_identity_and_translate(self):
        self.assertEqual(Transform.identity().to_json(), {"matrix": [1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0]})
        t = Transform.translate(1, 2, 3)
        self.assertEqual(t.m[9:], (1, 2, 3))
        self.assertEqual(Transform.translate([1, 2, 3]), t)
        self.assertTrue(close(t.apply((1, 1, 1)), (2, 3, 4)))

    def test_axis_angle(self):
        r = Transform.axis_angle((0, 0, 1), 90)
        self.assertTrue(close(r.apply((1, 0, 0)), (0, 1, 0)))
        about = Transform.axis_angle((0, 0, 1), 90, origin=(1, 0, 0))
        self.assertTrue(close(about.apply((1, 0, 0)), (1, 0, 0)))
        self.assertTrue(close(about.apply((2, 0, 0)), (1, 1, 0)))
        x = Transform.axis_angle((2, 0, 0), 90)  # axis is normalised
        self.assertTrue(close(x.apply((0, 1, 0)), (0, 0, 1)))
        with self.assertRaises(ValueError):
            Transform.axis_angle((0, 0, 0), 10)

    def test_composition_applies_right_first(self):
        t = Transform.translate(10, 0, 0) @ Transform.axis_angle((0, 0, 1), 90)
        self.assertTrue(close(t.apply((1, 0, 0)), (10, 1, 0)))
        u = Transform.axis_angle((0, 0, 1), 90) @ Transform.translate(10, 0, 0)
        self.assertTrue(close(u.apply((0, 0, 0)), (0, 10, 0)))
        self.assertEqual(t @ Transform.identity(), t)

    def test_look(self):
        t = Transform.look((1, 2, 3), (1, 0, 0), (0, 0, 1))
        self.assertTrue(close(t.apply((0, 0, 1)), (2, 2, 3)))  # local z points along +x
        self.assertTrue(close(t.apply((1, 0, 0)), (1, 2, 4)))  # local x follows the hint
        s = Transform.look((0, 0, 0), (0, 0, 1))  # hint parallel handled
        self.assertEqual(s, Transform.identity())
        p = Transform.look((0, 0, 0), (1, 0, 0), (1, 0, 0))  # hint parallel to z
        self.assertTrue(close(p.apply((0, 0, 1)), (1, 0, 0)))
        # right-handed
        a, b, c = t.m[0:3], t.m[3:6], t.m[6:9]
        cross = (a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0])
        self.assertTrue(close(cross, c))

    def test_bad_input(self):
        with self.assertRaises(ValueError):
            Transform([1, 2, 3])
        with self.assertRaises(ValueError):
            Transform.look((0, 0, 0), (0, 0, 0))


def describe_has(client, command=None, field=None, lighting=None):
    d = client.describe()
    cmds = {v["properties"]["type"]["const"]: v for v in d["methods"]["batch"]["params_schema"]["$defs"]["Command"]["oneOf"]}
    if lighting is not None:
        p = d["methods"]["render_view"]["params_schema"]
        return lighting in repr(p)
    if command not in cmds:
        return False
    return field is None or field in cmds[command]["properties"]


@unittest.skipIf(APP is None, "no sketchup-alt app binary found")
class ModelTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.app = launch_headless(APP)
        cls.client = cls.app.client()
        cls.tmp = tempfile.mkdtemp(prefix="skmodel")

    @classmethod
    def tearDownClass(cls):
        cls.client.close()
        cls.app.stop()
        import shutil

        shutil.rmtree(cls.tmp, ignore_errors=True)

    def setUp(self):
        self.client.execute({"type": "new", "discard": True})
        self.m = Model(self.client)

    def entity(self, inst):
        return next(e for e in self.client.query("entities")["instances"] if e["id"] == inst.id)

    def test_material_reuse_by_name(self):
        a = self.m.material("Steel", "#808080")
        b = self.m.material("Steel", (128, 128, 128))
        self.assertEqual(a.id, b.id)
        self.assertNotEqual(a.id, self.m.material("Red", (255, 0, 0)).id)
        with self.assertRaises(ValueError):
            self.m.material("Bad", "#12")

    def test_primitives(self):
        mat = self.m.material("Blue", (0, 0, 255))
        made = [
            self.m.box((0, 0, 0), (1, 2, 3), name="Slab", material=mat),
            self.m.cylinder((5, 0, 0), 0.5, 2),
            self.m.tube((8, 0, 0), 1, 0.5, 1, segments=16),
            self.m.cone((11, 0, 0), 1, 2, top_radius=0.2),
            self.m.prism((14, 0, 0), 1, 1, sides=6),
            self.m.sphere((18, 0, 0), 1),
            self.m.torus((22, 0, 0), 1, 0.2),
        ]
        self.assertEqual(len({i.id for i in made}), 7)
        ents = {e["id"]: e for e in self.client.query("entities")["instances"]}
        self.assertEqual(ents[made[0].id]["name"], "Slab")
        self.assertEqual(ents[made[0].id]["bounds"]["max"], [1.0, 2.0, 3.0])

    def test_component_primitive_and_place(self):
        box = self.m.box((0, 0, 0), (1, 1, 1), as_="component", name="Peg")
        other = self.m.place(box, Transform.translate(5, 0, 0))
        self.assertNotEqual(other.id, box.id)
        self.assertEqual(other.definition, box.definition)
        self.assertTrue(close(self.entity(other)["bounds"]["min"], (5, 0, 0)))

    def test_instance_ops(self):
        box = self.m.box((0, 0, 0), (1, 1, 1))
        box.move((2, 0, 0))
        self.assertTrue(close(self.entity(box)["bounds"]["min"], (2, 0, 0)))
        box.rotate((0, 0, 1), 90, origin=(0, 0, 0))
        self.assertTrue(close(self.entity(box)["bounds"]["min"], (-1, 2, 0), 1e-6))
        t = Transform.look((0, 0, 0), (1, 0, 0), (0, 0, 1))
        box.set_transform(t)
        self.assertTrue(close(self.entity(box)["transform"], t.m, 1e-9))
        box.name("Renamed")
        self.assertEqual(self.entity(box)["name"], "Renamed")
        box.paint(self.m.material("Green", (0, 255, 0)))
        box.hide()

    def test_group_and_component(self):
        a = self.m.box((0, 0, 0), (1, 1, 1))
        b = self.m.box((3, 0, 0), (1, 1, 1))
        g = self.m.group([a, b], name="Pair")
        self.assertEqual(self.entity(g)["name"], "Pair")
        c = self.m.component(g, "Pairs")
        self.assertEqual(self.entity(c)["kind"], "component")

    def test_copy_along_path(self):
        peg = self.m.box((0, 0, 0), (0.2, 0.2, 0.2))
        r = self.client.execute({"type": "add_edge", "start": [0, 5, 0], "end": [4, 5, 0]})
        edge = next(i for i in r["changes"]["created"] if i.startswith("e"))
        copies = self.m.copy_along_path([peg], [edge], count=3)
        self.assertGreaterEqual(len(copies), 1)

    def test_render_writes_png(self):
        self.m.box((0, 0, 0), (1, 1, 1))
        path = os.path.join(self.tmp, "r.png")
        try:
            out = self.m.render(path, 64, 48, camera={"eye": [4, -4, 3], "target": [0.5, 0.5, 0.5]}, lighting="day")
        except ApiError as e:
            self.skipTest("no GPU: %s" % e)
        self.assertEqual(out, path)
        with open(path, "rb") as f:
            self.assertEqual(f.read(8), b"\x89PNG\r\n\x1a\n")

    # helpers for later milestones

    def test_solids(self):
        if not describe_has(self.client, "solid_union"):
            self.skipTest("solid tools not in this app")
        a = self.m.box((0, 0, 0), (2, 2, 2))
        b = self.m.box((1, 1, 1), (2, 2, 2))
        u = self.m.union([a, b])
        self.assertTrue(close(self.entity(u)["bounds"]["max"], (3, 3, 3), 1e-6))
        blk = self.m.box((10, 0, 0), (2, 2, 2))
        bore = self.m.cylinder((11, 1, -1), 0.5, 4)
        s = self.m.subtract(blk, bore)
        self.assertIn(s.id, [e["id"] for e in self.client.query("entities")["instances"]])
        x = self.m.box((20, 0, 0), (2, 2, 2))
        y = self.m.box((21, 1, 1), (2, 2, 2))
        i = self.m.intersect([x, y])
        self.assertTrue(close(self.entity(i)["bounds"]["min"], (21, 1, 1), 1e-6))

    def test_material_finish(self):
        if not describe_has(self.client, "add_material", "metalness"):
            self.skipTest("material finish not in this app")
        m = self.m.material("Chrome", (200, 200, 210), metalness=1, roughness=0.2)
        again = self.m.material("Chrome", (200, 200, 210), metalness=0.5, roughness=0.4)
        self.assertEqual(m.id, again.id)

    def test_studio_render(self):
        if not describe_has(self.client, lighting="studio"):
            self.skipTest("studio lighting not in this app")
        self.m.box((0, 0, 0), (1, 1, 1))
        try:
            self.m.render(os.path.join(self.tmp, "s.png"), 64, 48)
        except ApiError as e:
            self.skipTest("no GPU: %s" % e)

    def test_label(self):
        if not describe_has(self.client, "add_label"):
            self.skipTest("labels not in this app")
        box = self.m.box((0, 0, 0), (1, 1, 1))
        note = self.m.label(box, "Block")
        self.assertTrue(note)

    def test_scene_positions(self):
        if not describe_has(self.client, "add_scene", "positions"):
            self.skipTest("scene positions not in this app")
        box = self.m.box((0, 0, 0), (1, 1, 1))
        self.assertTrue(self.m.scene("Start", positions=[box]))

    def test_scene_plain(self):
        self.assertTrue(self.m.scene("Front"))


if __name__ == "__main__":
    unittest.main()
