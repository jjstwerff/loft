<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Registry CI templates

Drop these files into the **`loft-lang/registry`** repo (per
[REGISTRY_BOOTSTRAP.md](../REGISTRY_BOOTSTRAP.md) Step 3) — they're
the PR-validation + post-merge-signing workflows.

| File in this dir | Destination in `loft-lang/registry` | What it does |
|---|---|---|
| `validate.py` | `tools/validate.py` | R9 PR validator: schema lint + tarball sha256 verify + reproducible-build re-check. |
| `sign-index.py` | `tools/sign-index.py` | Reads `REGISTRY_SIGNING_KEY_BASE64` secret, signs `index.json`, writes `index.json.sig`.  Called by `sign-and-commit.yml`. |
| `pr-validate.yml` | `.github/workflows/pr-validate.yml` | Wires `validate.py` into every PR that touches `index.json`. |
| `sign-and-commit.yml` | `.github/workflows/sign-and-commit.yml` | R3.5 post-merge signing: calls `sign-index.py`, commits `index.json.sig`. |
| `registry_README.md` | `README.md` (the registry's own) | Visible-on-GitHub landing page for ecosystem contributors. |
| `../registry_sample.json` | `index.json` (initial seed) | Empty starter index — strip the `_comment` field; set `"packages": {}` if no real package is ready yet. |

Optionally add a JSON Schema file at `schema/index-v1.json` for
editor tooling — useful but not required (`validate.py`'s lint
is sufficient for the gate).

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
