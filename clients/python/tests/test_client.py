"""Tests for the client, batch builder and launcher, against a real headless app.

They skip cleanly when no app binary is found (SKETCHUP_ALT_APP, then
target/debug/app and target/release/app under the repo root).
"""
import copy
import json
import os
import pickle
import socket
import sys
import threading
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

from sketchup_alt import ApiError, Client, Ref, find_app, launch_headless  # noqa: E402
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


def fake_client(replies, timeout=2):
    """A Client talking to a thread that answers each request with ``replies(req)``,
    a list of raw lines to send back."""
    a, b = socket.socketpair()
    c = Client.__new__(Client)
    c.socket_path, c.timeout = "fake", timeout
    c._buf, c._next_id, c._describe, c._schema, c._sock = bytearray(), 0, None, None, a

    def serve():
        f = b.makefile("rwb")
        for line in f:
            for out in replies(json.loads(line)):
                f.write(out if isinstance(out, bytes) else json.dumps(out).encode() + b"\n")
            f.flush()
        b.close()

    threading.Thread(target=serve, daemon=True).start()
    return c


class ProtocolTests(unittest.TestCase):
    def test_notifications_and_other_ids_skipped(self):
        c = fake_client(lambda r: [
            {"jsonrpc": "2.0", "method": "changed", "params": {"revision": 1}},
            {"jsonrpc": "2.0", "id": 999, "result": {"stale": True}},
            b"\n",
            {"jsonrpc": "2.0", "id": r["id"], "result": {"ok": r["id"]}},
        ])
        self.assertEqual(c.request("describe"), {"ok": 1})
        self.assertEqual(c.request("describe"), {"ok": 2})
        c.close()

    def test_null_id_error_raises(self):
        c = fake_client(lambda r: [{"jsonrpc": "2.0", "id": None, "error": {"code": -32700, "message": "parse error"}}])
        with self.assertRaises(ApiError) as cm:
            c.request("describe")
        self.assertEqual(cm.exception.kind, "parse_error")
        c.close()

    def test_malformed_replies_are_connection_errors(self):
        for bad in (b"not json\n", b"[1,2]\n", b"3\n"):
            c = fake_client(lambda r, bad=bad: [bad])
            with self.assertRaises(ConnectionError):
                c.request("describe")
            c.close()

    def test_nan_refused_before_sending(self):
        sent = []
        c = fake_client(lambda r: sent.append(r) or [])
        with self.assertRaises(ValueError):
            c.execute({"type": "add_edge", "start": [float("nan"), 0, 0], "end": [1, 0, 0]})
        with self.assertRaises(ValueError):
            c.batch([{"type": "add_edge", "start": [0, 0, float("inf")], "end": [1, 0, 0]}])
        self.assertEqual(sent, [])
        c.close()

    def test_api_error_ids_pickle_copy(self):
        e = ApiError("invalid_input", "bad", {"kind": "invalid_input", "ids": ["f1"]}, -32000)
        self.assertEqual(e.ids, ["f1"])
        b = ApiError.from_reply({"code": -32001, "message": "x", "data": {
            "index": 2, "completed": 2, "rolled_back": False,
            "error": {"kind": "not_found", "message": "gone", "ids": ["e3"]}}})
        self.assertEqual((b.kind, b.index, b.ids, b.rolled_back), ("not_found", 2, ["e3"], False))
        for x in (pickle.loads(pickle.dumps(b)), copy.copy(b), copy.deepcopy(b)):
            self.assertEqual((x.kind, x.message, x.index, x.ids, x.completed), ("not_found", "gone", 2, ["e3"], 2))
            self.assertEqual(str(x), str(b))

    def test_nan_in_builder_value(self):
        class Fake(Client):
            def __init__(self):
                pass

            def command_schema(self):
                return {"add_edge": {"fields": {"start", "end"}, "required": {"start", "end"}, "doc": ""}}

        from sketchup_alt.batch import Batch

        b = Batch(Fake())
        with self.assertRaises(ValueError):
            b.add_edge(start=[float("nan"), 0, 0], end=[1, 0, 0])


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

    def test_non_atomic_batch_failure_keeps_earlier_commands(self):
        with self.assertRaises(ApiError) as cm:
            self.client.batch(
                [
                    {"type": "add_rectangle", "corner": [0, 0, 0], "opposite": [2, 2, 0]},
                    {"type": "push_pull", "face": "f99", "distance": 1},
                ],
                atomic=False,
            )
        self.assertEqual(cm.exception.index, 1)
        self.assertEqual(len(cm.exception.completed), 1)
        self.assertFalse(cm.exception.rolled_back)
        self.assertEqual(self.client.query("stats")["faces"], 1)

    def test_batch_schema_excludes_editor_commands(self):
        schema = self.client.command_schema()
        self.assertIn("add_rectangle", schema)
        for name in ("set_tool", "set_fly_mode"):
            self.assertNotIn(name, schema)

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
