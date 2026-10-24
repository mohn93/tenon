"""Start and stop a headless app on a private socket."""
import os
import shutil
import subprocess
import sys
import tempfile
import time


def find_app():
    """Path of the app binary, or None: ``$SKETCHUP_ALT_APP``, then
    ``target/debug/app`` and ``target/release/app`` under the repo root."""
    env = os.environ.get("SKETCHUP_ALT_APP")
    if env and os.path.isfile(env):
        return env
    root = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", "..", ".."))
    exe = "app.exe" if sys.platform == "win32" else "app"
    for profile in ("debug", "release"):
        p = os.path.join(root, "target", profile, exe)
        if os.path.isfile(p):
            return p
    return None


class Launched:
    """A running headless app: ``socket_path``, ``client()``, ``stop()``."""

    def __init__(self, proc, socket_path, tmpdir, errfile):
        self.proc = proc
        self.socket_path = socket_path
        self._tmpdir = tmpdir
        self._errfile = errfile
        self._clients = []

    def client(self, **kwargs):
        from .client import Client

        c = Client(self.socket_path, **kwargs)
        self._clients.append(c)
        return c

    def stderr(self):
        try:
            with open(self._errfile, "r", errors="replace") as f:
                return f.read()
        except OSError:
            return ""

    def stop(self):
        """Quit the app and remove its temp dir; safe to call twice."""
        if self.proc is None:
            return
        for c in self._clients:
            c.close()
        if self.proc.poll() is None:
            try:
                from .client import Client

                c = Client(self.socket_path, timeout=5)
                c.quit(discard=True)
                c.close()
            except Exception:
                pass
            try:
                self.proc.wait(timeout=10)
            except subprocess.TimeoutExpired:
                self.proc.terminate()
                try:
                    self.proc.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    self.proc.kill()
                    self.proc.wait()
        self.proc = None
        shutil.rmtree(self._tmpdir, ignore_errors=True)

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        self.stop()
        return False


def launch_headless(app=None, *args, startup_timeout=30):
    """Start ``app --headless --socket <private path> *args`` and wait until it
    accepts connections. ``app`` defaults to ``find_app()``."""
    app = app or find_app()
    if app is None:
        raise FileNotFoundError("no app binary: set SKETCHUP_ALT_APP or run `cargo build -p app`")
    tmpdir = tempfile.mkdtemp(prefix="skalt")
    if sys.platform != "win32" and len(os.path.join(tmpdir, "api.sock").encode()) > 100:
        shutil.rmtree(tmpdir, ignore_errors=True)  # too long for a Unix socket path
        tmpdir = tempfile.mkdtemp(prefix="skalt", dir="/tmp")
    if sys.platform == "win32":
        sock = "sketchup-alt-py-%d-%s" % (os.getpid(), os.path.basename(tmpdir))
    else:
        sock = os.path.join(tmpdir, "api.sock")
    errfile = os.path.join(tmpdir, "stderr.log")
    err = open(errfile, "wb")
    try:
        proc = subprocess.Popen(
            [app, "--headless", "--socket", sock, *args],
            stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL,
            stderr=err,
        )
    except BaseException:
        err.close()
        shutil.rmtree(tmpdir, ignore_errors=True)
        raise
    finally:
        err.close()
    launched = Launched(proc, sock, tmpdir, errfile)
    try:
        _wait_ready(launched, startup_timeout)
    except BaseException:
        launched.stop()
        raise
    return launched


def _wait_ready(launched, startup_timeout):
    from .client import Client

    deadline = time.monotonic() + startup_timeout
    while True:
        if launched.proc.poll() is not None:
            raise RuntimeError(
                "the app exited with code %s: %s" % (launched.proc.returncode, launched.stderr().strip())
            )
        try:
            Client(launched.socket_path, timeout=5).close()
            return
        except OSError:
            if time.monotonic() > deadline:
                raise TimeoutError(
                    "the app did not listen within %s s: %s" % (startup_timeout, launched.stderr().strip())
                )
            time.sleep(0.05)
