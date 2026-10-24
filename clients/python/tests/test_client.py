"""Tests for the client, batch builder and launcher, against a real headless app.

They skip cleanly when no app binary is found (SKETCHUP_ALT_APP, then
target/debug/app and target/release/app under the repo root).
"""
import os
import sys
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

from sketchup_alt import ApiError, Ref, find_app, launch_headless  # noqa: E402
from sketchup_alt.batch import Ref as BatchRef  # noqa: E402

APP = find_app()


class RefTests(unittest.TestCase):
    """Reference rendering needs no app."""

    def test_reference_strings(self):
        self.assertEqual(Ref(0).f(0), "$0:f0")
        self.assertEqual(Ref(3).e("*"), "$3:e*")
        self.assertEqual(Ref(12).i(2), "$12:i2")
        self.assertEqual(Ref(1).all(), "$1:*")
        self.assertEqual(Ref(2).d(0), "$2:d0")
        self.assertEqual(Ref(2).m(0), "$2:m0")
        self.assertIs(Ref, BatchRef)

    def test_bad_reference(self):
        with self.assertRaises(ValueError):
            Ref(-1)
        with self.assertRaises(ValueError):
            Ref(0).f(-1)
        with self.assertRaises(AttributeError):
            Ref(0).q(0)


@unittest.skipIf(APP is None, "no sketchup-alt app binary found")
class AppTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.app = launch_headless(APP)
        cls.client = cls.app.client()

    @classmethod
    def tearDownClass(cls):
        cls.client.close()
        cls.app.stop()

    def setUp(self):
        self.client.execute({"type": "new", "discard": True})

    def test_execute_and_query(self):
        r = self.client.execute({"type": "add_edge", "start": [0, 0, 0], "end": [1, 0, 0]})
        self.assertEqual(len(r["changes"]["created"]), 3)
        stats = self.client.query("stats")
        self.assertEqual(stats["edges"], 1)

    def test_error_raises_api_error(self):
        with self.assertRaises(ApiError) as cm:
            self.client.execute({"type": "push_pull", "face": "f99", "distance": 1})
        self.assertEqual(cm.exception.kind, "not_found")
        self.assertTrue(cm.exception.message)

    def test_describe_and_render(self):
        d = self.client.describe()
        self.assertIn("schemas", d)
        self.client.execute({"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [1, 1, 0]})
        try:
            png = self.client.render_view(64, 48)
        except ApiError as e:
            self.skipTest("no GPU: %s" % e)
        self.assertEqual(png[:8], b"\x89PNG\r\n\x1a\n")

    def test_batch_with_references(self):
        out = self.client.batch(
            [
                {"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [2, 2, 0]},
                {"type": "push_pull", "face": "$0:f0", "distance": 1},
            ]
        )
        self.assertEqual(len(out["outcomes"]), 2)

    def test_batch_failure_has_index(self):
        with self.assertRaises(ApiError) as cm:
            self.client.batch(
                [
                    {"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [2, 2, 0]},
                    {"type": "push_pull", "face": "f99", "distance": 1},
                ]
            )
        self.assertEqual(cm.exception.index, 1)
        self.assertTrue(cm.exception.rolled_back)
        self.assertEqual(self.client.query("stats")["edges"], 0)

    def test_builder_refs_and_result(self):
        with self.client.batch_builder() as b:
            rect = b.add_rectangle(corner=[0, 0, 0], opposite=[3, 2, 0])
            self.assertIsInstance(rect, Ref)
            b.push_pull(face=rect.f(0), distance=1.5)
        self.assertEqual(len(b.result["outcomes"]), 2)
        self.assertEqual(self.client.query("stats")["faces"], 6)

    def test_builder_generated_method(self):
        # add_circle is generated or hand-written; add_polygon-like commands come from the schema
        with self.client.batch_builder() as b:
            b.add_circle(center=[0, 0, 0], radius=1.0, segments=12)
        self.assertEqual(self.client.query("stats")["faces"], 1)

    def test_builder_unknown_field_raises_before_sending(self):
        rev = self.client.query("stats")["revision"]
        with self.assertRaises(TypeError):
            with self.client.batch_builder() as b:
                b.add_circle(center=[0, 0, 0], radius=1.0, bogus=1)
        with self.assertRaises(TypeError):
            with self.client.batch_builder() as b:
                b.add_circle(center=[0, 0, 0])  # radius missing
        with self.assertRaises(AttributeError):
            self.client.batch_builder().no_such_command
        self.assertEqual(self.client.query("stats")["revision"], rev)

    def test_builder_exception_in_block_sends_nothing(self):
        with self.assertRaises(RuntimeError):
            with self.client.batch_builder() as b:
                b.add_circle(center=[0, 0, 0], radius=1.0)
                raise RuntimeError("boom")
        self.assertEqual(self.client.query("stats")["faces"], 0)

    def test_builder_failure_raises_with_index(self):
        with self.assertRaises(ApiError) as cm:
            with self.client.batch_builder() as b:
                b.add_circle(center=[0, 0, 0], radius=1.0)
                b.push_pull(face="f99", distance=1)
        self.assertEqual(cm.exception.index, 1)

    def test_large_reply_and_many_requests(self):
        # a big query reply arrives in several reads
        with self.client.batch_builder() as b:
            for i in range(60):
                b.add_circle(center=[i * 3, 0, 0], radius=1.0, segments=48)
        ents = self.client.query("entities")
        self.assertGreater(len(str(ents)), 65536)

    def test_timeout(self):
        c = self.app.client(timeout=0.2)
        try:
            c._send_raw(b'{"jsonrpc":"2.0","id":1,"method":"describe"')  # no newline: no reply
            with self.assertRaises(TimeoutError):
                c._read_reply(1)
        finally:
            c.close()

    def test_env_socket(self):
        from sketchup_alt import Client

        os.environ["SKETCHUP_ALT_SOCKET"] = self.app.socket_path
        try:
            c = Client()
            self.assertEqual(c.query("stats")["edges"], 0)
            c.close()
        finally:
            del os.environ["SKETCHUP_ALT_SOCKET"]


class LauncherTests(unittest.TestCase):
    @unittest.skipIf(APP is None, "no sketchup-alt app binary found")
    def test_stop_removes_socket_dir(self):
        app = launch_headless(APP)
        path = app.socket_path
        self.assertTrue(os.path.exists(path))
        app.stop()
        self.assertFalse(os.path.exists(os.path.dirname(path)))
        app.stop()  # idempotent

    def test_find_app_env(self):
        old = os.environ.get("SKETCHUP_ALT_APP")
        os.environ["SKETCHUP_ALT_APP"] = sys.executable
        try:
            self.assertEqual(find_app(), sys.executable)
            os.environ["SKETCHUP_ALT_APP"] = "/nonexistent/app"
            self.assertNotEqual(find_app(), "/nonexistent/app")
        finally:
            if old is None:
                del os.environ["SKETCHUP_ALT_APP"]
            else:
                os.environ["SKETCHUP_ALT_APP"] = old


if __name__ == "__main__":
    unittest.main()
