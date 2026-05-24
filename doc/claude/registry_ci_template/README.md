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
| `pr-validate.yml` | `.github/workflows/pr-validate.yml` | Wires `validate.py` into every PR that touches `index.json`. |
| `registry_README.md` | `README.md` (the registry's own) | Visible-on-GitHub landing page for ecosystem contributors. |
| `../registry_sample.json` | `index.json` (initial seed) | Empty starter index — strip the `_comment` field; set `"packages": {}` if no real package is ready yet. |

Optionally add a JSON Schema file at `schema/index-v1.json` for
editor tooling — useful but not required (`validate.py`'s lint
is sufficient for the gate).

## Signing is NOT in CI

Earlier drafts of this template shipped a `sign-and-commit.yml`
workflow + `sign-index.py` helper that signed `index.json` in
GitHub Actions using a `REGISTRY_SIGNING_KEY_BASE64` secret.

We removed it.  Signing happens locally on the maintainer's
laptop via `loft-keygen sign` — see [REGISTRY_BOOTSTRAP.md § Step 4](../REGISTRY_BOOTSTRAP.md)
maintainer side, and the rationale in [PKG_REGISTRY.md § Index signing](../PKG_REGISTRY.md#index-signing--indexjsonsig).

Short version: an early-stage ecosystem publishes weekly at most;
a human maintainer is always in the merge loop anyway; the
private key never needs to leave hardware the maintainer
physically controls.  When the ecosystem scales beyond what
laptop signing handles, the architecture migrates to Path 1
(real server) and signing follows.

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
