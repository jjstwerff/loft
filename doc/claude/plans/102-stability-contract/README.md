<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 102 — Stability contract for scale

## Status

**Open — no implementation.  Design partial; arc B's spec is written (the matrix
below), arcs A / C / E need design.**  This is the wide-release bar's **gate 5**
([STABILITY_ROADMAP.md § The wide-release bar](../../STABILITY_ROADMAP.md)).  Its
opening condition — *"open one when gate 1 is in sight"* — fired **2026-07-10**, when
gate 1 came down to a single work item (the Cluster C `copy_claims` fold).  Gates 2
(@PLN25) and 3 (@PLN28 / @PLN36) are already closed.

The plan is opened **because the failure mode it prevents is already live**, not as a
speculative hardening.  See § Why now.

## Goal

Ship a stated compatibility contract for loft, plus the mechanism that enforces it —
expressible version bounds, a deprecation channel, a public bug-intake path, and a
declared 1.0 line — so that a loft release cannot silently break the libraries and
programs built on it.

## Effort + design

- **Effort:** L (multi-arc: policy + mechanism)
- **Design:** ~ (partial — arc B specified by the matrix below; A / C / E need design)
- **Last touched:** 2026-07-10

## Why now — the failure mode is live, not hypothetical

`hex_terrain 0.1.0` fails its own registry-validation test with `0 land cells`.  The
library uses the plain-bind write-through idiom throughout:

```loft
th = t.tr_h;      // intent: alias the terrain's height vector
th[i] = value;    // intent: write through to t.tr_h
```

loft now **copies on plain bind** ([DESIGN_DECISIONS.md § C86](../../DESIGN_DECISIONS.md)
— H-Copy: whole-value heap binds copy; aliasing is a last-use elision), so the writes
land in a throwaway copy, `t.tr_h` stays all-zeros, every cell classifies as sea, and
the island has no interior.  **It does not crash.  It computes a plausible wrong
answer.**

`graphics` hit the identical class and was migrated to `&self.data`.  `hex_terrain` was
never migrated.  Both declare `loft = ">=0.8"`, so nothing guarded either of them.

[GOALS.md](../../GOALS.md) holds the platform to the opposite standard, and names it as
the thing loft is trying to win back from the AS/400:

> backward compatibility was a contract the maker kept: the platform never broke its
> users; the cost of change was paid by the maker, not the customer.

A compatibility promise with a deprecation channel is the mechanism that would have
caught this before publication.

## Composition matrix — Stage A

Arc **B** extends an existing operation (`manifest::check_version`), so it carries a
matrix.  Arcs **A / D / E** are policy and documentation with no composition surface,
and arc **C** gains its matrix once its warning shape is designed — stated here rather
than left silent.

`check_version` (`src/manifest.rs:513`) strips a leading `>=` and parses the remainder
numerically with `unwrap_or(0)`, so **every non-`>=` form degrades to `0.0.0` and always
passes**.  Probed against the current interpreter, `2026.7.1`:

| `loft = ` in `loft.toml` | today | required after arc B |
|---|---|---|
| `">=0.8"` | ACCEPT | ACCEPT |
| `">=9999.0"` | **REJECT** | REJECT |
| `"<=0.1"` | ACCEPT | **REJECT** — upper bound must bind |
| `"<2026.0.0"` | ACCEPT | **REJECT** |
| `"=0.1"` | ACCEPT | **REJECT** — exact pin must bind |
| `"^0.9"` | ACCEPT | **REJECT** (or support caret; decide in arc B) |
| `">=0.8, <2027"` | *unrepresentable* | ACCEPT — ranges must parse |
| `"garbage"` | ACCEPT | **REJECT LOUDLY** — a constraint you don't honour is a category-S silent failure |

The `">=9999.0"` row is the **positive control**: it is the one cell that fires today's
gate (`"Package X requires loft R but interpreter is C"`), which proves the harness can
fail and that the other rows' silence is a real accept, not a dead probe.  Accept cells
proceed past the version gate into downstream symbol resolution; the gate's own failure
is that distinct fatal.

Two consequences, both load-bearing for this plan:

1. A library **cannot** declare that it does not work with loft ≥ X.  There is no
   syntax that binds, so `hex_terrain` could not have protected itself even if its
   author had known.
2. Under **calendar versioning** (`2026.7.1`), a `>=0.8` lower bound is *permanently*
   vacuous — every future loft satisfies it.  Every library in the registry carries one.

Cells graduate to `tests/` as arc B's regression suite (both backends: the check runs in
the parser, so `--interpret` and `--native` must agree on accept/reject — an
accept/reject divergence here is a Goal-D violation).

## Sub-arcs

| Item | Source | Status |
|---|---|---|
| **A** — Compatibility policy: what *is* a breaking change, per surface (language syntax/semantics · stdlib API · store/heap layout · on-disk + wire format · package format) | needs design → `COMPATIBILITY.md` | Open |
| **B** — Expressible version bounds: upper bounds + ranges; loud rejection of unparseable constraints; how the language versions itself; registry validates the declared range | matrix above; `src/manifest.rs:513`, `src/parser/mod.rs:7593` | Open — spec written |
| **C** — Deprecation channel: a warning path for a semantic change a library can trip.  [Goal F](../../GOALS.md) permits exactly one channel — warnings, free to ignore | needs design; worked example = C86 / `hex_terrain` | Open |
| **D** — Public bug-intake path: the fix-not-file discipline is internal and does not reach strangers | [ISSUE_TRACKING.md](../../ISSUE_TRACKING.md) | Open |
| **E** — The 1.0 line: what is frozen vs still moving | [RELEASE.md](../../RELEASE.md) | Open |

## Phase ordering

1. **A first — policy before mechanism.**  B enforces a rule; the rule has to exist.  A
   is doc-only and unblocks everything else.
2. **B next — it has the sharpest evidence and the smallest surface.**  The matrix is
   already the spec.  Landing B alone closes the *silent* half of the gap (a constraint
   that is accepted but not honoured), even before any policy is stated.
3. **C after A** — the deprecation channel needs A's definition of "breaking" to know
   what it must warn about.  C is what would actually have caught `hex_terrain`; B only
   lets a library defend itself after the fact.
4. **D in parallel** — independent of A/B/C; no dependencies.
5. **E last** — the 1.0 line is a statement about A, so it cannot precede it.

## Open design questions

1. **What does semver mean when the schema is data and the heap layout *is* the
   contract?**  @PLN97 shipped a formal memory/file layout contract with a layout-identity
   hash.  Is that hash the compatibility key for the store surface — i.e. does a layout-hash
   change *define* a breaking change there?
2. **Does the language version itself keep calendar versioning?**  `2026.7.1` cannot
   express compatibility (see the matrix).  Options: a separate monotone *language
   edition* number that libraries declare against; or semver for the language surface
   with calver retained for releases.
3. **Whose obligation is a deprecation warning?**  C86 changed the meaning of code that
   libraries had *already shipped*.  A warning at loft-build time reaches the loft team;
   a warning at library-compile time reaches the author; a warning at consumer-compile
   time reaches the wrong person.  Where does it fire?
4. **Is `loft = ">=0.8"` grandfathered?**  Every published library carries a vacuous
   bound.  Does arc B rewrite them, reject them, or treat a bare lower bound as
   "unknown compatibility"?
5. **How does this interact with the registry `verified` mark?**  See
   [LIBRARY_CHECKLIST.md](../../LIBRARY_CHECKLIST.md).  Should `verified` assert
   "tested against loft X" rather than a point-in-time pass?

## Cross-arc dependencies

- **@PLN97** (layout contract, closed) — supplies the layout-identity hash; the natural
  compatibility key for the store/file surface.  Question 1 above.
- **PKG.7 — `loft.lock`** ([PACKAGES.md § Open work](../../PACKAGES.md)) — a lockfile pins a
  working set; complementary to a compat range, not a substitute (a lock protects an
  existing consumer, a range protects a *new* resolve).
- **`registry-validation` CI** — the enforcement point, and currently the detector: it has
  never had a green run, and `hex_terrain` is why.  Arc B gives it something to check.
- **@PLN78** (loft distribution / self-update) — a self-updating binary needs a stated
  compatibility promise before it can be trusted to update under a user's project.
- **[Goal B](../../GOALS.md)** (release & legibility) and **[Goal F](../../GOALS.md)**
  (warnings are the only channel that may bill the programmer).

## See also

- [GOALS.md](../../GOALS.md) — the AS/400 aspiration this plan defends; the two floors.
- [STABILITY_ROADMAP.md](../../STABILITY_ROADMAP.md) — the wide-release bar; gate 5 is this
  plan, gate 1 is the Cluster C fold that precedes it.
- [DESIGN_DECISIONS.md § C86](../../DESIGN_DECISIONS.md) — the plain-bind copy decision that
  `hex_terrain` did not survive.
- [PACKAGES.md](../../PACKAGES.md) / [PKG_REGISTRY.md](../../PKG_REGISTRY.md) — the manifest
  and registry that arc B changes.
- [LIBRARY_CHECKLIST.md](../../LIBRARY_CHECKLIST.md) — the per-library quality bar.
- `src/manifest.rs:513` (`check_version`) · `src/parser/mod.rs:7593` (the call site) — arc B's
  two touch points.
- Tracking issue: [@PLN102](https://github.com/loft-lang/plans/issues/102).
