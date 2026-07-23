#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# @PLN117 arc D — cross-origin-isolated static server for the threaded-wasm
# harness.  Sends the COOP/COEP header pair that a host MUST send for
# `crossOriginIsolated === true` (the precondition for SharedArrayBuffer + wasm
# atomics), plus correct .wasm / .js MIME types.  Real deployments (the gallery,
# `loft --html` hosts) must reproduce these two headers — see doc/claude/WASM.md.
#
#   python3 tests/wasm/coi-server.py [port] [root] [report-file]
#
# GET /report?r=... appends r to the report file (the test channel the headless
# driver reads); everything else is served statically.
import http.server, socketserver, sys, urllib.parse, os
PORT   = int(sys.argv[1]) if len(sys.argv) > 1 else 8791
ROOT   = sys.argv[2] if len(sys.argv) > 2 else "."
REPORT = sys.argv[3] if len(sys.argv) > 3 else os.path.join(ROOT, ".par-report")
class H(http.server.SimpleHTTPRequestHandler):
    def __init__(self, *a, **k): super().__init__(*a, directory=ROOT, **k)
    def end_headers(self):
        self.send_header("Cross-Origin-Opener-Policy", "same-origin")
        self.send_header("Cross-Origin-Embedder-Policy", "require-corp")
        self.send_header("Cache-Control", "no-store")
        super().end_headers()
    def guess_type(self, path):
        if path.endswith(".wasm"): return "application/wasm"
        if path.endswith((".js", ".mjs")): return "text/javascript"
        return super().guess_type(path)
    def do_GET(self):
        if self.path.startswith("/report"):
            q = urllib.parse.urlparse(self.path).query
            with open(REPORT, "a") as f:
                f.write(urllib.parse.parse_qs(q).get("r", [""])[0] + "\n")
            self.send_response(204); self.end_headers(); return
        return super().do_GET()
    def log_message(self, *a): pass
# Two runs of a gate on the same port, back to back, otherwise hit the socket's
# TIME_WAIT and the second server silently never binds — which reads as "the page
# produced nothing" rather than "the server never started".
socketserver.ThreadingTCPServer.allow_reuse_address = True
with socketserver.ThreadingTCPServer(("127.0.0.1", PORT), H) as httpd:
    print(f"COI server: {ROOT} on http://127.0.0.1:{PORT}", flush=True)
    httpd.serve_forever()
