#!/usr/bin/env python3
"""@PLN146 F2 — a dumb static file server that honours Range and LOGS it.

`python3 -m http.server` ignores `Range` and answers `200` with the whole body.
loft still reads correctly through that (it discards the prefix to reach the
offset), so a run against it proves the URL path WORKS and proves nothing about
what crossed the wire — which is the half F2 exists to gate.  This server
answers `206 Partial Content` and writes one line per request to a log, so the
gate can assert that only the requested keys' pages were fetched.

  python3 probes/f2_server.py <dir> <port> <logfile>
"""
import os
import re
import sys
from http.server import BaseHTTPRequestHandler, HTTPServer

ROOT, PORT, LOG = sys.argv[1], int(sys.argv[2]), sys.argv[3]
RANGE = re.compile(r"bytes=(\d*)-(\d*)")
ROOT_REAL = os.path.realpath(ROOT)


def _under_root(name):
    """The file `name` names inside ROOT, or None if it names anything else.

    `basename` already drops every directory component, so a traversal cannot be
    spelled — but a served path is still built from a request, and the check that
    proves it stayed inside belongs where the file is opened rather than in a
    reader's head one function away.
    """
    full = os.path.realpath(os.path.join(ROOT_REAL, name))
    if full != ROOT_REAL and not full.startswith(ROOT_REAL + os.sep):
        return None
    return full


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *_args):
        pass                                   # the range log is the only output

    def _note(self, line):
        with open(LOG, "a", encoding="utf-8") as fh:
            fh.write(line + "\n")

    def _serve(self, body):
        name = os.path.basename(self.path.lstrip("/"))
        full = _under_root(name)
        if full is None or not os.path.isfile(full):
            self._note(f"404 {name}")
            self.send_error(404)
            return
        size = os.path.getsize(full)
        header = self.headers.get("Range")
        m = RANGE.match(header) if header else None
        if not m:
            # Whole-file: legal, and exactly what the gate must be able to SEE.
            self._note(f"FULL {name} {size}")
            self.send_response(200)
            self.send_header("Content-Length", str(size))
            self.send_header("Accept-Ranges", "bytes")
            self.end_headers()
            if body:
                with open(full, "rb") as fh:
                    self.wfile.write(fh.read())
            return
        start = int(m.group(1) or 0)
        last = int(m.group(2)) if m.group(2) else size - 1
        last = min(last, size - 1)
        n = max(0, last - start + 1)
        self._note(f"RANGE {name} {start}-{last} {n}")
        with open(full, "rb") as fh:
            fh.seek(start)
            chunk = fh.read(n)
        self.send_response(206)
        self.send_header("Content-Range", f"bytes {start}-{last}/{size}")
        self.send_header("Content-Length", str(len(chunk)))
        self.send_header("Accept-Ranges", "bytes")
        self.end_headers()
        if body:
            self.wfile.write(chunk)

    def do_GET(self):
        self._serve(True)

    def do_HEAD(self):
        self._serve(False)


if __name__ == "__main__":
    open(LOG, "w", encoding="utf-8").close()
    HTTPServer(("127.0.0.1", PORT), Handler).serve_forever()
