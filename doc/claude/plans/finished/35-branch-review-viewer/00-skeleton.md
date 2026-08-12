<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 00 — Skeleton + frozen-binary build

**Status:** **Shipped 2026-05-13.**

## Deviation from the original design

The original design called for a `tools/viewer/loft.toml` package
manifest with a `[binary]` section.  PACKAGES.md's spec doesn't
have a `[binary]` section yet (today loft packages are
`[library]`-only — see all 14 `lib/*/loft.toml` examples).  The
viewer is shipped as a SCRIPT + Makefile rather than a
loft-package, until the package format gains binary support.

The Makefile target compiles `tools/viewer/src/main.loft` via
`loft --native`, finds the cached binary at
`tools/viewer/src/.loft/cache/main-<hash>`, and copies it to
`tools/viewer/bin/loft-view`.  Same outcome as the package
approach: deliberate build, stable artifact path, frozen
provenance via `tools/viewer/BUILD_NOTES.md`.

When PACKAGES.md grows `[binary]` support, this can migrate to a
proper package manifest without changing the user-facing
contract.

## Goal

Get a minimal loft binary at `tools/viewer/bin/loft-view` that
prints "loft-view v0.1" and exits 0.  Establishes the
`tools/viewer/` package layout, the `loft.toml` manifest with
its pinned loft version, the `Makefile` targets, and the
frozen-binary contract.  No HTTP, no rendering — just the
build pipeline + repo wiring.

This is the smallest possible commit that demonstrates the
architecture: a viewer source tree, a deliberate build, a
binary that the user can copy to other VMs.

## What ships

### Files

```
tools/viewer/
├── loft.toml               # Package manifest, pins loft version
├── README.md               # Tiny "what is this" pointing at plans/35
├── refresh.sh              # Empty stub (filled in phase 04)
├── src/
│   └── main.loft           # Entry point: prints version, exits
├── state/                  # gitignored; populated by refresh.sh
│   └── .gitkeep
└── bin/
    └── .gitkeep            # Binary committed in phase 07; placeholder for now
```

### `tools/viewer/loft.toml`

```toml
[package]
name = "loft-view"
version = "0.1.0"
loft = "=0.8.4"          # PINNED — see frozen-binary contract

[binary]
entry = "src/main.loft"
output = "bin/loft-view"

[deps]
server = { path = "../../lib/server" }
```

The `loft = "=0.8.4"` pin is the frozen-binary contract: this
viewer was built against loft 0.8.4 and will only rebuild
against that version.  Bumping the pin is a deliberate
viewer-update commit.

### `tools/viewer/src/main.loft`

```loft
// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// loft-view: branch-aware doc + code review viewer.
// See doc/claude/plans/35-branch-review-viewer/README.md.

fn main() {
    print("loft-view v0.1\n");
}
```

### `Makefile` additions

```make
# ── Branch review viewer (plans/35) ─────────────────────────────
view-build:  ## Compile the loft-view binary deliberately
	cargo run --bin loft -- --native --output tools/viewer/bin/loft-view tools/viewer/src/main.loft

view:  ## Run the pre-built loft-view binary
	@if [ ! -x tools/viewer/bin/loft-view ]; then \
	   echo "loft-view not built; run: make view-build"; \
	   exit 1; \
	fi
	./tools/viewer/bin/loft-view
```

(Exact loft-build invocation is what `--native` produces today;
adjust per current loft CLI.)

### `tools/viewer/README.md` (one paragraph)

```markdown
# loft-view — branch review viewer (plan-35)

A frozen loft binary that serves a branch-aware doc + code
review dashboard from this repo to a browser via SSH
port-forward.  See
[doc/claude/plans/finished/35-branch-review-viewer/README.md](README.md)
for the full spec.

Build: `make view-build`.  Run: `make view`.
```

### `.gitignore` additions

```
# loft-view runtime state (refreshed by tools/viewer/refresh.sh)
/tools/viewer/state/*
!/tools/viewer/state/.gitkeep
```

The `bin/loft-view` binary is committed (or, in phase 07,
attached to releases).  The `state/` directory is per-checkout
and gitignored.

## Critical files

| Path | Action |
|---|---|
| `tools/viewer/loft.toml` | NEW |
| `tools/viewer/src/main.loft` | NEW |
| `tools/viewer/refresh.sh` | NEW (empty stub: `#!/bin/bash` + `# filled by plan-35 phase 04`) |
| `tools/viewer/state/.gitkeep` | NEW |
| `tools/viewer/bin/.gitkeep` | NEW |
| `tools/viewer/README.md` | NEW |
| `Makefile` | ADD `view-build:` and `view:` targets |
| `.gitignore` | ADD `tools/viewer/state/*` |

## Existing functions / tooling to reuse

- **`lib/server/loft.toml`** as the manifest template — already
  uses `[package]` + `[library]`; this plan uses `[package]` +
  `[binary]` (verify the `[binary]` section name against
  `doc/claude/PACKAGES.md`'s spec; adjust if the spec uses a
  different name).
- **`make` patterns** from existing targets — `view` mirrors
  `serve`'s "start a local thing" shape.

## Test surface

- `make view-build` succeeds; produces `tools/viewer/bin/loft-view`
  with executable bit set.
- `./tools/viewer/bin/loft-view` prints `loft-view v0.1` and
  exits 0.
- `make view` invokes the binary (same output).
- `make view` without `make view-build` first prints the
  "build me" hint and exits non-zero.
- `git status` on a fresh checkout shows no untracked files
  in `tools/viewer/state/` (the `.gitignore` works).

## Verification

```bash
$ make view-build
# … loft compiles main.loft → tools/viewer/bin/loft-view

$ ls -l tools/viewer/bin/loft-view
-rwxr-xr-x 1 ubuntu ubuntu 1.2M ...

$ make view
loft-view v0.1

$ rm tools/viewer/bin/loft-view
$ make view
loft-view not built; run: make view-build
```

## Risks

| Risk | Mitigation |
|---|---|
| `loft.toml` `[binary]` section not yet supported by current loft | Verify against `doc/claude/PACKAGES.md`; if `[binary]` isn't supported, ship a small Makefile shim that invokes the loft compiler directly |
| Pinning loft to `=0.8.4` blocks other workflow that needs newer | Pin is per-package; only this binary build sees it.  Other code paths use whatever is in the workspace. |
| Committing the binary bloats the repo | 1-2 MB per release is acceptable; if it grows, switch to release-attached artifacts in phase 07 |

## Cross-references

- [README § Architecture — frozen-binary contract](README.md#architecture--frozen-binary-contract)
- [PACKAGES.md](../../../PACKAGES.md) — `loft.toml` manifest spec
- [Makefile](../../../../../Makefile) — target conventions
