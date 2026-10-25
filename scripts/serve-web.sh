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


def takes(part):
    """Whether one Accept-Encoding item takes gzip (and does not refuse it with q=0)."""
    coding, *params = [p.strip() for p in part.split(";")]
    for p in params:
        if p.startswith("q="):
            try:
                if float(p[2:]) <= 0:
                    return False
            except ValueError:
                pass
    return coding.lower() in ("gzip", "*")


class Handler(http.server.SimpleHTTPRequestHandler):
    extensions_map = {
        **http.server.SimpleHTTPRequestHandler.extensions_map,
        ".wasm": "application/wasm",
        ".js": "text/javascript",
    }

    def send_head(self):
        # The build's gzip copy, for a browser that takes gzip.
        path = self.translate_path(self.path)
        gz = path + ".gz"
        accept = self.headers.get("Accept-Encoding", "")
        takes_gzip = any(takes(part) for part in accept.split(","))
        if takes_gzip and os.path.isfile(path) and os.path.isfile(gz):
            f = open(gz, "rb")
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
