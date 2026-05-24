#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later

"""Sign `index.json` with the maintainer's Ed25519 key.

Drop at `tools/sign-index.py` in the `loft-lang/registry` repo.
Called by `.github/workflows/sign-and-commit.yml` after every merge
to `main` that touches `index.json`.

Usage:

    python3 tools/sign-index.py "$REGISTRY_SIGNING_KEY_BASE64"

Reads `index.json` from cwd, signs its bytes, writes `index.json.sig`
(raw 64-byte Ed25519 signature, no encoding wrapper — the loft
client expects raw bytes, not PEM/DER).

Requires `cryptography` (added via `pip install cryptography` in
the workflow).  ~30 lines; no other deps.
"""

from __future__ import annotations

import base64
import pathlib
import sys

from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey


def main() -> None:
    if len(sys.argv) != 2:
        sys.stderr.write(
            "usage: sign-index.py <base64-private-key>\n"
            "       (REGISTRY_SIGNING_KEY_BASE64 from secrets, NOT logged)\n"
        )
        sys.exit(1)

    key_b64 = sys.argv[1]
    try:
        key_bytes = base64.b64decode(key_b64)
    except Exception as e:  # noqa: BLE001
        sys.stderr.write(f"sign-index.py: base64 decode failed: {e}\n")
        sys.exit(1)

    if len(key_bytes) != 32:
        sys.stderr.write(
            f"sign-index.py: private key must be 32 bytes; got {len(key_bytes)}\n"
        )
        sys.exit(1)

    private_key = Ed25519PrivateKey.from_private_bytes(key_bytes)
    content = pathlib.Path("index.json").read_bytes()
    signature = private_key.sign(content)

    # Atomic write: temp file + rename so a half-written sig file
    # can't briefly serve to a client mid-CI.
    tmp = pathlib.Path("index.json.sig.tmp")
    tmp.write_bytes(signature)
    tmp.replace("index.json.sig")
    print(f"signed index.json ({len(content)} bytes) → index.json.sig ({len(signature)} bytes)")


if __name__ == "__main__":
    main()
