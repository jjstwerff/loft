#!/usr/bin/env python3
"""COOP/COEP-aware static file server for testing SharedArrayBuffer."""
import http.server
import socketserver
import sys

PORT = int(sys.argv[1]) if len(sys.argv) > 1 else 8765
ROOT = sys.argv[2] if len(sys.argv) > 2 else "."

class Handler(http.server.SimpleHTTPRequestHandler):
    def __init__(self, *args, **kwargs):
        super().__init__(*args, directory=ROOT, **kwargs)
    def end_headers(self):
        self.send_header("Cross-Origin-Opener-Policy", "same-origin")
        self.send_header("Cross-Origin-Embedder-Policy", "require-corp")
        self.send_header("Cross-Origin-Resource-Policy", "same-origin")
        super().end_headers()
    def log_message(self, format, *args):
        pass

with socketserver.TCPServer(("127.0.0.1", PORT), Handler) as httpd:
    print(f"serving {ROOT} on http://127.0.0.1:{PORT} with COOP/COEP", flush=True)
    httpd.serve_forever()
