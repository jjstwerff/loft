<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# loft-view — branch review viewer (plan-35)

A frozen loft binary that serves a branch-aware doc + code review
dashboard from this repo to a browser via SSH port-forward.

See [doc/claude/plans/35-branch-review-viewer/](../../doc/claude/plans/35-branch-review-viewer/README.md)
for the full design and per-phase plans.

## Build

```
make view-build
```

Compiles `src/main.loft` via loft's native backend, copies the
resulting binary to `bin/loft-view`.  The binary is built
deliberately, not on every cargo invocation — this is the
**frozen-binary contract**: a broken `src/parser/foo.rs` change
in loft does not break the viewer.

## Run

```
make view
```

Starts the binary.  Currently (phase 00) just prints the version
and exits.  Subsequent phases (01+) add HTTP serving + dashboard.

## Layout

```
tools/viewer/
├── README.md           This file.
├── refresh.sh          Phase 04 git-state wrapper (stub today).
├── src/
│   └── main.loft       Entry point.
├── state/              Runtime state from refresh.sh; gitignored.
└── bin/
    └── loft-view       Built binary (committed in phase 07 closeout).
```

## BUILD_NOTES

The binary records the loft commit it was built against.  Update
`BUILD_NOTES.md` whenever `make view-build` runs successfully so the
frozen-binary provenance stays explicit.
