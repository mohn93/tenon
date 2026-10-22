#!/usr/bin/env bash
# Serves web/dist on http://localhost:8080 (PORT overrides it) with the right media type for
# .wasm. Build first with scripts/build-web.sh. Needs python3.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
dist="$root/web/dist"
[ -f "$dist/index.html" ] || { echo "web/dist is empty: run scripts/build-web.sh first" >&2; exit 1; }

exec python3 -I - "$dist" "${PORT:-8080}" <<'PY'
import functools
import http.server
import sys

directory, port = sys.argv[1], int(sys.argv[2])


class Handler(http.server.SimpleHTTPRequestHandler):
    extensions_map = {
        **http.server.SimpleHTTPRequestHandler.extensions_map,
        ".wasm": "application/wasm",
        ".js": "text/javascript",
    }

    def end_headers(self):
        # A rebuilt module is picked up by a plain reload.
        self.send_header("Cache-Control", "no-cache")
        super().end_headers()


handler = functools.partial(Handler, directory=directory)
print(f"serving {directory} on http://localhost:{port}")
http.server.ThreadingHTTPServer(("127.0.0.1", port), handler).serve_forever()
PY
