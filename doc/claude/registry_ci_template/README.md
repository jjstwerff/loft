<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Registry CI templates

Drop these files into the **`loft-lang/registry`** repo (per
[REGISTRY_BOOTSTRAP.md](../REGISTRY_BOOTSTRAP.md) Step 3) — they're
the PR-validation + post-merge-signing workflows.

| File | Destination in `loft-lang/registry` | What it does |
|---|---|---|
| `validate.py` | `tools/validate.py` | R9 PR validator: schema lint + tarball sha256 verify + reproducible-build re-check. |
| `pr-validate.yml` | `.github/workflows/pr-validate.yml` | Wires `validate.py` into every PR that touches `index.json`. |
| `sign-and-commit.yml` | `.github/workflows/sign-and-commit.yml` | R3.5 post-merge signing: re-signs `index.json` with the maintainer Ed25519 key, commits `index.json.sig`. |
| `index.json.example` | `index.json` (initial) | Empty starter index — derived from `doc/claude/registry_sample.json`. |

Also write a tiny `tools/sign-index.py`:

```python
#!/usr/bin/env python3
import base64, sys, pathlib
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
key_bytes = base64.b64decode(sys.argv[1])
private_key = Ed25519PrivateKey.from_private_bytes(key_bytes)
content = pathlib.Path("index.json").read_bytes()
sig = private_key.sign(content)
pathlib.Path("index.json.sig").write_bytes(sig)
```

Plus the schema lint can be sharpened with a JSON Schema file at
`schema/index-v1.json` — useful for editor tooling but not strictly
required (validate.py's lint is sufficient for the gate).

## Local dry-run

Run the validator against a candidate index before opening the PR:

```sh
cd loft-lang/registry/
python3 tools/validate.py
```

The reproducible-build gate clones each new homepage repo + runs
`loft package` — slow but the publisher's machine is the right
place to catch mismatches before CI does.

To skip the reproducible-build gate locally (much faster smoke):

```sh
LOFT_VALIDATE_SKIP_REPRO=1 python3 tools/validate.py
```
