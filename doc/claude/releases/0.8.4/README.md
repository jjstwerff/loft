<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 0.8.4 progress

> The record of ONE release cycle — its blockers, the evidence each gate produced, and the
> decisions taken.  The process every cycle follows lives in
> [RELEASE.md](../../RELEASE.md); the index of cycles in [releases/README.md](../README.md).

**2026-04-14:** tag deferred — safety gate caught P54
chained-call leak (`json_*().method()` leaks temporary store).

**2026-04-25 (dep-fix-sprint):** dep-inference fix landed.
Two changes:
1. Parser (`src/parser/definitions.rs`): native methods
   returning same struct-enum as `self` now carry `dep=[0]`
   (borrow from self).  Constructors (no self) keep `dep=[]`.
2. Scope lift (`src/scopes.rs::inline_struct_return`): native
   struct-enum constructors (empty dep) are lifted to
   temporaries and freed at scope exit.

Result: **79 previously-ignored P54/Q4 leak tests un-ignored
and passing**.  Ignored count in `issues.rs` dropped from 89
to 6 (maintenance, B2/B3 match crash, B5 recursive, B7
character-interpolation, P136 harness, step-6 by design).

**Remaining blockers for 0.8.4 tag:**
- WASM-build + WASM-runtime gates — both verified green
  (run via `make wasm-html-test` to avoid the rlib-feature collision)
- Crash bugs: none (B2-runtime, B3, B5, B7, P136, P142, P155 all closed)
- Zero-leak gate — wrap-suite `loft_suite` currently emits no
  `stores not freed` warnings across scripts 42/62/76/95; re-verify
  on the tag candidate
- Zero-ignore baseline approval — only the `regen_fill_rs`
  maintenance entry remains (candidate for permanent exemption)

Severity legend:
- **H** — hard block.  Release cannot ship.
- **M** — block unless the exact scenario is documented and the
  release notes call it out as a known issue.
