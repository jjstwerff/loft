#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# @PLN117 arc D — the counterpart to coi-server.py: a host that does NOT send
# COOP/COEP, so the page is not cross-origin isolated and has no
# SharedArrayBuffer.  A threaded loft bundle must still come up there and run
# `par` sequentially with identical results — that is the "never breaks"
# guarantee, and this server is what proves it instead of assuming it.
#
#   python3 tests/wasm/html-plain-server.py [port] [root] [report-file]
#
# GET /report?r=... appends r to the report file; everything else is static.
import http.server, socketserver, sys, urllib.parse, os
PORT   = int(sys.argv[1]) if len(sys.argv) > 1 else 8792
ROOT   = sys.argv[2] if len(sys.argv) > 2 else "."
REPORT = sys.argv[3] if len(sys.argv) > 3 else os.path.join(ROOT, ".par-report")
class H(http.server.SimpleHTTPRequestHandler):
    def __init__(self, *a, **k): super().__init__(*a, directory=ROOT, **k)
    def end_headers(self):
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
socketserver.ThreadingTCPServer.allow_reuse_address = True
with socketserver.ThreadingTCPServer(("127.0.0.1", PORT), H) as httpd:
    print(f"plain server (no COOP/COEP): {ROOT} on http://127.0.0.1:{PORT}", flush=True)
    httpd.serve_forever()
