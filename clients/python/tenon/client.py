"""Connection to a running tenon app: newline-delimited JSON-RPC 2.0."""
import base64
import json
import os
import socket
import sys
import time

_CODE_KINDS = {
    -32700: "parse_error",
    -32600: "invalid_request",
    -32601: "method_not_found",
    -32602: "invalid_params",
    -32001: "batch_failed",
    -32002: "rendering_unavailable",
    -32003: "connection_lost",
}


class ApiError(Exception):
    """An error reply. ``kind`` is the API's error kind (``invalid_input``,
    ``not_found``...) for a failed command, else a name for the JSON-RPC code.
    For a failed batch ``index`` is the failing command, ``rolled_back`` says
    whether the batch was undone, and ``completed`` the outcomes of the commands that ran.
    """

    def __init__(self, kind, message, data=None, code=None, index=None, rolled_back=None, completed=None):
        super().__init__(kind, message, data, code, index, rolled_back, completed)
        self.kind = kind
        self.message = message
        self.data = data
        self.code = code
        self.index = index
        self.rolled_back = rolled_back
        self.completed = completed
        ids = None
        if isinstance(data, dict):
            ids = data.get("ids")
            if ids is None and isinstance(data.get("error"), dict):
                ids = data["error"].get("ids")
        self.ids = ids

    def __str__(self):
        text = self.message if self.index is None else "command %d: %s" % (self.index, self.message)
        return "%s: %s" % (self.kind, text)

    @classmethod
    def from_reply(cls, err):
        code = err.get("code")
        data = err.get("data")
        message = err.get("message", "")
        if code == -32001 and isinstance(data, dict):
            inner = data.get("error")
            kind, msg = _CODE_KINDS[code], message
            if isinstance(inner, dict):
                kind = inner.get("kind", kind)
                msg = inner.get("message", msg)
            return cls(kind, msg, data, code, data.get("index"), data.get("rolled_back"), data.get("completed"))
        if code == -32000 and isinstance(data, dict):
            return cls(data.get("kind", "command_failed"), data.get("message", message), data, code)
        return cls(_CODE_KINDS.get(code, "error"), message, data, code)


class _WinPipe:
    """Windows named pipe with the socket methods the client uses.

    A pipe read cannot time out here, so ``timeout`` is not enforced on Windows.
    """

    def __init__(self, name):
        if not name.startswith("\\\\"):
            name = "\\\\.\\pipe\\" + name
        self._f = open(name, "r+b", buffering=0)

    def settimeout(self, _t):
        pass

    def sendall(self, data):
        self._f.write(data)

    def recv(self, n):
        return self._f.read(n) or b""

    def close(self):
        self._f.close()


class Client:
    """``Client(socket_path=None, timeout=60)``.

    The path defaults to ``$TENON_SOCKET``. On Windows it is a pipe name
    (a bare name becomes ``\\\\.\\pipe\\name``). ``timeout`` (seconds) bounds each
    wait for a reply; it raises ``TimeoutError``. Not thread safe.
    """

    def __init__(self, socket_path=None, timeout=60):
        path = socket_path or os.environ.get("TENON_SOCKET")
        if not path:
            raise ValueError("give socket_path or set TENON_SOCKET")
        self.socket_path = path
        self.timeout = timeout
        self._buf = bytearray()
        self._next_id = 0
        self._describe = None
        self._schema = None
        self._sock = None
        self._connect()

    # transport

    def _connect(self):
        if sys.platform == "win32":
            self._sock = _WinPipe(self.socket_path)
        else:
            s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
            s.settimeout(self.timeout)
            try:
                s.connect(self.socket_path)
            except BaseException:
                s.close()
                raise
            self._sock = s

    def close(self):
        if self._sock is not None:
            try:
                self._sock.close()
            finally:
                self._sock = None

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        self.close()
        return False

    def _send_raw(self, data):
        self._sock.sendall(data)

    def _read_line(self, deadline):
        while True:
            i = self._buf.find(b"\n")
            if i >= 0:
                line = bytes(self._buf[:i])
                del self._buf[: i + 1]
                return line
            left = deadline - time.monotonic()
            if left <= 0:
                raise TimeoutError("no reply within %s s" % self.timeout)
            self._sock.settimeout(left)
            try:
                chunk = self._sock.recv(1 << 16)
            except socket.timeout:
                raise TimeoutError("no reply within %s s" % self.timeout) from None
            if not chunk:
                raise ConnectionError("the app closed the connection")
            self._buf += chunk

    def _read_reply(self, req_id):
        deadline = time.monotonic() + self.timeout
        while True:
            line = self._read_line(deadline)
            if not line.strip():
                continue
            try:
                msg = json.loads(line)
            except ValueError as e:
                raise ConnectionError("protocol error: reply is not JSON: %s" % e) from None
            if not isinstance(msg, dict):
                raise ConnectionError("protocol error: reply is not an object: %.80r" % (line,))
            mid = msg.get("id")
            if mid is None and isinstance(msg.get("error"), dict):
                # the app answers an unparseable request with id null
                raise ApiError.from_reply(msg["error"])
            if mid == req_id:  # anything else is a notification
                return msg

    def request(self, method, params=None):
        """Send one JSON-RPC request and return its result, or raise ``ApiError``."""
        self._next_id += 1
        req = {"jsonrpc": "2.0", "id": self._next_id, "method": method}
        if params is not None:
            req["params"] = params
        data = json.dumps(req, separators=(",", ":"), allow_nan=False).encode() + b"\n"
        self._send_raw(data)
        msg = self._read_reply(self._next_id)
        if isinstance(msg.get("error"), dict):
            raise ApiError.from_reply(msg["error"])
        if "result" not in msg:
            raise ConnectionError("protocol error: reply has neither result nor error")
        return msg["result"]

    # API

    def execute(self, cmd):
        """Run one command (a dict with ``type``); returns ``{changes, revision, ...}``."""
        return self.request("execute", cmd)

    def batch(self, cmds, atomic=True):
        """Run commands as one batch (they may use references like ``"$0:f0"``)."""
        return self.request("batch", {"commands": [dict(c) for c in cmds], "atomic": atomic})

    def query(self, type, **params):
        """Run a query such as ``query("stats")``; returns the result dict."""
        q = {"type": type}
        q.update(params)
        return self.request("query", q)

    def describe(self):
        """The API description (cached after the first call)."""
        if self._describe is None:
            self._describe = self.request("describe")
        return self._describe

    def render_view(self, width, height, **opts):
        """Render the model; returns PNG bytes. ``opts``: camera, lighting, ..."""
        p = {"width": width, "height": height}
        p.update(opts)
        return base64.b64decode(self.request("render_view", p)["png_base64"])

    def quit(self, discard=True):
        """Ask the app to exit."""
        try:
            self.request("quit", {"discard": discard})
        except (ConnectionError, OSError):
            pass

    # batch builder

    def command_schema(self):
        """``{type: {fields, required, doc}}`` for the commands a batch accepts
        (the Command schema of the ``batch`` params in ``describe``). Screen and
        editor commands are for ``execute`` only."""
        if self._schema is None:
            d = self.describe()
            try:
                cmd = d["methods"]["batch"]["params_schema"]["$defs"]["Command"]
            except KeyError:
                cmd = d["schemas"]["Command"]
            out = {}
            for v in cmd.get("oneOf", []):
                name = v["properties"]["type"]["const"]
                out[name] = {
                    "fields": set(v["properties"]) - {"type"},
                    "required": set(v.get("required", [])) - {"type"},
                    "doc": v.get("description", ""),
                }
            self._schema = out
        return self._schema

    def batch_builder(self, atomic=True):
        """A ``Batch`` to use as ``with client.batch_builder() as b:``."""
        from .batch import Batch

        return Batch(self, atomic=atomic)
