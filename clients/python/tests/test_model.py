"""Tests for Transform (pure) and Model (against a launched headless app).

"""
import math
import os
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

from sketchup_alt import ApiError, Joint, Model, Transform, find_app, launch_headless  # noqa: E402

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

    def test_inverse_and_vectors(self):
        t = Transform.translate(1, 2, 3) @ Transform.axis_angle((1, 1, 0), 40) @ Transform.scale(2, 1, 0.5)
        self.assertEqual(t @ t.inverse(), Transform.identity())
        self.assertTrue(close(t.inverse().apply(t.apply((0.3, -2, 5))), (0.3, -2, 5), 1e-9))
        self.assertTrue(close(Transform.translate(5, 5, 5).apply_vector((1, 0, 0)), (1, 0, 0)))
        with self.assertRaises(ValueError):
            Transform.scale(1, 0, 1).inverse()

    def test_bad_input(self):
        with self.assertRaises(ValueError):
            Transform([1, 2, 3])
        with self.assertRaises(ValueError):
            Transform.look((0, 0, 0), (0, 0, 0))


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


    def test_solids(self):
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
        m = self.m.material("Chrome", (200, 200, 210), metalness=1, roughness=0.2)
        again = self.m.material("Chrome", (200, 200, 210), metalness=0.5, roughness=0.4)
        self.assertEqual(m.id, again.id)
        mats = self.client.query("materials")["materials"]
        chrome = next(x for x in mats if x["id"] == m.id)
        self.assertAlmostEqual(chrome["metalness"], 0.5, places=6)
        self.assertAlmostEqual(chrome["roughness"], 0.4, places=6)

    def test_studio_render(self):
        self.m.box((0, 0, 0), (1, 1, 1))
        try:
            self.m.render(os.path.join(self.tmp, "s.png"), 64, 48)
        except ApiError as e:
            self.skipTest("no GPU: %s" % e)

    def test_joint_drive_value_delete(self):
        base = self.m.box((0, 0, 0), (1, 1, 1))
        arm = self.m.box((1, 0, 0), (2, 0.2, 0.2))
        j = self.m.joint("revolute", base, arm, (1, 0, 0), axis=(0, 0, 1), name="Hinge", limits=(-90, 90))
        self.assertIsInstance(j, Joint)
        self.assertEqual(j.value, 0)
        j.drive(45)
        self.assertAlmostEqual(j.value, 45, places=6)
        self.assertTrue(close(self.entity(arm)["bounds"]["max"][:2], (1 + 2 * math.cos(math.radians(45)), 2 * math.sin(math.radians(45)) + 0.2 * math.cos(math.radians(45))), 1e-5))
        with self.assertRaises(ApiError):
            j.drive(120)
        j.delete()
        self.assertEqual(self.client.query("joints")["joints"], [])
        with self.assertRaises(LookupError):
            j.value
        g = self.m.joint("prismatic", None, base, (0, 0, 0), axis=(1, 0, 0))
        g.drive(0.5)
        self.assertAlmostEqual(g.value, 0.5, places=6)

    def test_couple_and_cam_follower(self):
        a = self.m.box((0, 0, 0), (1, 1, 1))
        b = self.m.box((5, 0, 0), (1, 1, 1))
        ja = self.m.joint("revolute", None, a, (0.5, 0.5, 0), axis=(0, 0, 1))
        jb = self.m.joint("revolute", None, b, (5.5, 0.5, 0), axis=(0, 0, 1))
        k = self.m.couple(ja, jb, 0.5)
        self.assertTrue(k.startswith("k"))
        ja.drive(90)
        self.assertAlmostEqual(jb.value, 45, places=5)
        self.assertEqual(self.client.query("couplings")["couplings"][0]["ratio"], 0.5)
        # a cam (a box) with a valve (a prismatic part) above it
        cam = self.m.box((10, 0, 0), (2, 2, 1))
        valve = self.m.box((10.8, 0.8, 1.5), (0.4, 0.4, 1))
        jv = self.m.joint("prismatic", None, valve, (11, 1, 1.5), axis=(0, 0, 1))
        w = self.m.cam_follower(cam, jv, roller_radius=0, preload_direction=(0, 0, -1))
        self.assertTrue(w.startswith("w"))
        self.assertEqual(self.client.query("cam_followers")["cam_followers"][0]["id"], w)

    def test_solid_checks_and_mass(self):
        steel = self.m.material("Steel", (150, 150, 160), density=7850)
        self.assertEqual(steel.density, 7850)
        a = self.m.box((0, 0, 0), (1, 1, 1), material=steel)
        b = self.m.box((0.5, 0, 0), (1, 1, 1))
        c = self.m.box((3, 0, 0), (1, 1, 1))
        i = self.m.interference([a, b])
        self.assertEqual([(p["a"], p["b"]) for p in i["pairs"]], [(a.id, b.id)])
        self.assertAlmostEqual(i["pairs"][0]["volume"], 0.5, places=6)
        self.assertEqual(self.m.interference([a, c])["pairs"], [])
        d = self.m.clearance(a, c)["pairs"][0]
        self.assertAlmostEqual(d["distance"], 2.0, places=6)
        self.assertAlmostEqual(self.m.clearance(ids=[a, c])["pairs"][0]["distance"], 2.0, places=6)
        mp = self.m.mass_properties([a])
        self.assertAlmostEqual(mp["total"]["mass"], 7850, places=3)
        self.assertTrue(close(mp["total"]["centroid"], (0.5, 0.5, 0.5), 1e-6))
        self.assertAlmostEqual(self.m.mass_properties()["total"]["volume"], 3.0, places=6)

    def test_cut_list(self):
        wood = self.m.material("Oak", (200, 160, 100))
        a = self.m.box((0, 0, 0), (1, 0.1, 0.02), material=wood)
        self.m.box((0, 1, 0), (0.5, 0.3, 0.02))
        rows = self.m.cut_list()
        self.assertEqual([r["name"] for r in rows], ["Group", "Group"])
        self.assertEqual(rows[0]["material"], "Oak")
        self.assertTrue(close((rows[0]["length"], rows[0]["width"], rows[0]["thickness"]), (1, 0.1, 0.02), 1e-9))
        self.assertEqual(rows[0]["ids"], [a.id])
        self.assertNotIn("material", rows[1])
        self.assertEqual(len(self.m.cut_list([a], group_by="none")), 1)

    def test_sweep_check_and_measure(self):
        base = self.m.box((-1, -1, -1), (2, 2, 0.5))
        arm = self.m.box((0, -0.1, 0), (1.5, 0.2, 0.2))
        post = self.m.box((0, 1, 0), (0.3, 0.3, 0.3))
        j = self.m.joint("revolute", base, arm, (0, 0, 0), axis=(0, 0, 1))
        r = self.m.sweep_check(j, 0, 360, 37, ids=[arm, post])
        self.assertEqual(r["steps"], 37)
        pair = next(p for p in r["pairs"] if {p["a"], p["b"]} == {arm.id, post.id})
        self.assertGreater(pair["volume"], 0)
        self.assertEqual(self.client.query("joints")["joints"][0]["value"], 0)
        dist = self.m.measure(distance=[arm, post])
        self.assertGreater(dist["distance"], 0)
        with self.assertRaises(ValueError):
            self.m.measure()
        with self.assertRaises(ValueError):
            self.m.measure(distance=[arm, post], angle=[arm, post])

    def test_render_joints_and_ground(self):
        base = self.m.box((0, 0, 0), (1, 1, 1))
        arm = self.m.box((1, 0, 0), (2, 0.2, 0.2))
        j = self.m.joint("revolute", base, arm, (1, 0, 0), axis=(0, 0, 1))
        cam = {"eye": [0.1, -0.1, 30], "target": [0, 0, 0], "fov_y_deg": 60}
        a = os.path.join(self.tmp, "g1.png")
        b = os.path.join(self.tmp, "g2.png")
        try:
            self.m.render(a, 64, 48, camera=cam, lighting="day", joints={j: 90})
            self.m.render(b, 64, 48, camera=cam, lighting="day", joints={j.id: 90}, ground=False)
        except ApiError as e:
            if e.kind != "rendering_unavailable":
                raise
            self.skipTest("no GPU: %s" % e)
        with open(a, "rb") as f, open(b, "rb") as g:
            self.assertNotEqual(f.read(), g.read())
        self.assertEqual(j.value, 0)

    def test_label(self):
        box = self.m.box((0, 0, 0), (1, 1, 1))
        note = self.m.label(box, "Block")
        self.assertTrue(note)

    def test_scene_positions(self):
        box = self.m.box((0, 0, 0), (1, 1, 1))
        self.assertTrue(self.m.scene("Start", positions=[box]))

    def solid(self, inst):
        return self.client.query("solid", id=inst.id)["solid"]

    def test_sweeps_and_profiles(self):
        pipe = self.m.sweep([(0, 0, 0), (0, 0, 1), (0.5, 0, 1.5)], 0.1, inner_radius=0.08, name="Pipe")
        self.assertTrue(self.solid(pipe))
        self.assertEqual(self.entity(pipe)["name"], "Pipe")
        loop = [(3, 0, 0), (4, 0, 0), (4, 1, 0), (3, 1, 0)]
        section = [(-0.05, -0.02), (0.05, -0.02), (0.05, 0.02), (-0.05, 0.02)]
        belt = self.m.sweep(loop, closed=True, profile=section, profile_x=(0, 0, 1), smooth=False)
        self.assertTrue(self.solid(belt))
        spring = self.m.spring((6, 0, 0), (0, 0, 1), 0.2, 0.03, 0.15, 3, segments=12, wire_segments=6)
        self.assertTrue(self.solid(spring))
        self.assertTrue(close(self.entity(spring)["bounds"]["min"], (5.77, -0.23, -0.03), 0.02))
        prism = self.m.extrude([(8, 0, 0), (9, 0, 0), (8.5, 1, 0)], 0.5)
        self.assertAlmostEqual(self.client.query("solid", id=prism.id)["volume"], 0.25, places=9)
        lathe = self.m.revolve([(10.5, 0, 0), (11, 0, 0), (11, 0, 1), (10.5, 0, 1)], (10, 0, 0), (0, 0, 1), segments=48)
        self.assertTrue(self.solid(lathe))
        # nothing loose is left behind at the root
        self.assertEqual(self.client.query("entities", kinds=["edge", "face"]).get("edges", []), [])

    def test_edges_fillet_chamfer(self):
        box = self.m.box((1, 1, 0), (2, 2, 1))
        tops = box.edges(lambda a, b: a[2] > 0.99 and b[2] > 0.99)  # model coordinates
        self.assertEqual(len(tops), 4)
        box.fillet(tops, 0.1, segments=2)
        self.assertTrue(self.solid(box))
        self.assertLess(self.client.query("solid", id=box.id)["volume"], 4.0)
        other = self.m.box((5, 0, 0), (1, 1, 1))
        upright = other.edges(lambda a, b: abs(a[2] - b[2]) > 0.5)
        self.assertEqual(len(upright), 4)
        other.chamfer(upright, 0.05)
        self.assertAlmostEqual(self.client.query("solid", id=other.id)["volume"], 1 - 4 * 0.00125, places=9)

    def test_section_clone_polar_and_cut(self):
        box = self.m.box((0, 0, 0), (1, 1, 1))
        box.apply(Transform.translate(5, 0, 0) @ Transform.axis_angle((0, 0, 1), 90))
        twin = box.clone()
        self.assertNotEqual(self.entity(twin)["definition"], self.entity(box)["definition"])
        self.assertEqual(twin.transform(), box.transform())
        plane = box.section((4.5, 0.5, 0.5), (0, 1, 0))  # model coordinates
        self.client.execute({"type": "open_instance", "id": box.id})
        try:
            planes = self.client.query("section_planes")["planes"]
        finally:
            self.client.execute({"type": "close_instance"})
        self.assertEqual([p["id"] for p in planes], [plane])
        # back in model coordinates the plane is where it was given
        local = planes[0]
        t = box.transform()
        self.assertTrue(close(t.apply(local["point"]), (4.5, 0.5, 0.5), 1e-9))
        self.assertTrue(close(t.apply_vector(local["normal"]), (0, 1, 0), 1e-9))
        ring = box.polar((0, 0, 1), 90, 3)
        self.assertEqual(len(ring), 3)
        self.m.section_cut(ring, cut=False)
        self.assertTrue(all(self.entity(i).get("section_cut") is False for i in ring))
        copies = box.copy((0, 3, 0), count=2)
        self.assertEqual(len(copies), 2)

    def test_view_settings(self):
        self.m.lighting("studio")
        self.assertEqual(self.client.query("camera")["lighting"], "studio")
        sid = self.m.style("Look", ambient_occlusion={"on": True})
        self.assertEqual(self.client.query("camera")["style"], sid)
        self.m.section_display(fill_color="material")
        self.assertEqual(self.client.query("section_display")["fill_color"], "material")
        self.m.camera((5, -5, 4), (0, 0, 0), fov_y_deg=40)
        self.assertEqual(self.client.query("camera")["fov_y_deg"], 40)
        box = self.m.box((0, 0, 0), (1, 1, 1))
        s0 = self.m.scene("A", positions=[box])
        box.move((2, 0, 0))
        s1 = self.m.scene("B", positions=[box])
        self.m.transition(True, 0.5)
        self.m.go_to_scene(s0)
        self.assertTrue(close(self.entity(box)["bounds"]["min"], (0, 0, 0)))
        self.m.explode_view([box, self.m.box((3, 0, 0), (1, 1, 1))], 1.0)
        try:
            self.m.render(os.path.join(self.tmp, "f.png"), 64, 48, lighting=None, scene=s1, from_=s0, t=0.5, supersample=2)
        except ApiError as e:
            if e.kind != "rendering_unavailable":
                raise
            self.skipTest("no GPU: %s" % e)

    def test_scene_plain(self):
        self.assertTrue(self.m.scene("Front"))


if __name__ == "__main__":
    unittest.main()
