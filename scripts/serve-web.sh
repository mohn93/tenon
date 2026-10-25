#!/usr/bin/env bash
# Serves web/dist on http://localhost:8080 (PORT overrides it) with the right media type for
# .wasm, sending the build's gzip copies to browsers that take gzip. Build first with
# scripts/build-web.sh. Needs python3.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
dist="$root/web/dist"
[ -f "$dist/index.html" ] || { echo "web/dist is empty: run scripts/build-web.sh first" >&2; exit 1; }

exec python3 -I - "$dist" "${PORT:-8080}" <<'PY'
import functools
import http.server
import os
import sys

directory, port = sys.argv[1], int(sys.argv[2])


def takes_gzip(accept):
    """Whether an Accept-Encoding value takes gzip: as gzip says (refused with q=0), else as *
    says, else not."""
    said = {}
    for part in accept.split(","):
        coding, *params = [p.strip() for p in part.split(";")]
        refused = False
        for p in params:
            if p.startswith("q="):
                try:
                    refused = float(p[2:]) <= 0
                except ValueError:
                    pass
        if coding.lower() in ("gzip", "*"):
            said[coding.lower()] = not refused
    return said.get("gzip", said.get("*", False))


class Handler(http.server.SimpleHTTPRequestHandler):
    extensions_map = {
        **http.server.SimpleHTTPRequestHandler.extensions_map,
        ".wasm": "application/wasm",
        ".js": "text/javascript",
    }

    def send_head(self):
        # The build's gzip copy, for a browser that takes gzip, unless it is older than the file
        # (left from an earlier build) or cannot be opened.
        path = self.translate_path(self.path)
        gz = path + ".gz"
        f = None
        if takes_gzip(self.headers.get("Accept-Encoding", "")) and os.path.isfile(path):
            try:
                # Within a millisecond counts as as new (gzip keeps microseconds).
                if os.path.isfile(gz) and os.stat(gz).st_mtime_ns + 1_000_000 >= os.stat(path).st_mtime_ns:
                    f = open(gz, "rb")
            except OSError:
                f = None
        if f is not None:
            self.send_response(200)
            self.send_header("Content-Type", self.guess_type(path))
            self.send_header("Content-Encoding", "gzip")
            self.send_header("Vary", "Accept-Encoding")
            self.send_header("Content-Length", str(os.fstat(f.fileno()).st_size))
            self.end_headers()
            return f
        return super().send_head()

    def end_headers(self):
        # A rebuilt module is picked up by a plain reload.
        self.send_header("Cache-Control", "no-cache")
        super().end_headers()


handler = functools.partial(Handler, directory=directory)
print(f"serving {directory} on http://localhost:{port}")
http.server.ThreadingHTTPServer(("127.0.0.1", port), handler).serve_forever()
PY
