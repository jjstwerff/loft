<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# LIBRARY_CHECKLIST.md — what a correct loft library looks like, and how "verified" is administered

The libraries are the real end of the stack ([GOALS.md](GOALS.md)) — so they carry
the *same* bar as loft, applied at the library level. A library is **correct**
(and earns the registry `verified` mark) when it clears this checklist. The stdlib
(`default/*.loft`) is the library every program imports, so it clears this same bar —
see [API_SURFACE.md](API_SURFACE.md), which runs the API-quality audit over both surfaces.

Every item is one of two tiers:

- **`[auto]`** — mechanically checkable; enforced by the chunk repo's
  `library-ci.yml` (and a future `loft verify <name>`). Green CI = the auto half.
- **`[review]`** — human judgment (doc quality, friction-free API, dogfood);
  signed off once at the registry PR. This doc *is* the registry-PR review template.

**Verified = `[auto]` green AND `[review]` signed off**, recorded in the registry
(below). The split mirrors the project doctrine: machines enforce the invariants
that fail silently; humans judge the things that need taste (legibility, fun).

---

## How "verified" is administered

**The lints never block a release.** A library releases and installs regardless of
api_lint / doc_review findings — only correctness (the test suite) can fail its CI.
The **`verified` mark is the only place clean lints are required**: it gates the
*badge*, not the library's existence. This keeps a doc-quality finding from ever
holding a bug fix (same rule as `lint_comments.sh` — advisory, never fails CI).

1. **`[auto]` half** — the library's `library-ci.yml` runs the tests as a hard gate
   (both backends, gold, deterministic package, `loft.toml` validity, naming,
   lifecycle stubs), and runs api_lint / doc_review **advisory** (`continue-on-error`)
   so findings are visible but non-blocking. Clean lints are required to *grant*
   `verified`, checked at the registry PR — not to merge or release.
2. **`[review]` half** — judged once during the **registry PR** (the human gate
   that already exists). The reviewer records the result in the package's
   `index.json` entry, next to `yanked`:
   ```json
   "verified": { "checklist": "1", "release": "0.1.1", "date": "2026-06-07", "by": "<gh-login>" }
   ```
   Absent ⇒ not verified (or pending). `yanked` overrides `verified`.
3. **Surfacing** — `loft list-installed` / `loft audit` show the verified status
   from the index; a proposed `loft verify <name>` runs the `[auto]` half locally
   and reports the registry flag.
4. **Re-verification** — a feature/major release re-runs the full checklist; a
   patch release inherits `verified` unless it touched the public API or the docs.

---

## The checklist

### Structure & packaging — `[auto]`
- [ ] `loft.toml` valid: `[package]` name/version/`loft` range; `[library] entry`; deps declared (path or registry).
- [ ] Repo is `loft-lang/loft-libs-<chunk>` (canonical naming; not `loft-<pkg>`, not a loft-monorepo dir).
- [ ] Carries the org lifecycle stubs (`fpm-apply` / `fpm-strip` → `loft-lang/.github` reusables).
- [ ] Deterministic package: two `loft package` runs produce an identical sha256.

### Goal A — Soundness (no silent corruption) — `[auto]`
- [ ] Tests pass on **interpret** and **`--native`** (`loft test` / `loft --native test`).
- [ ] `LOFT_DENY_WARNINGS=1 loft test` is green (or a justified `.allow_warnings` opt-out while not ready).
- [ ] If it manages stores: clean under `LOFT_STORE_GUARD` and the sanitizer surfaces it touches (N/A for pure-data libs).
      *(`LOFT_STORE_GUARD=1` now rides the unified library CI's own test run on both
      backends, so this is checked for every package on every push — no extra test to
      write, and a brand-new package passes it on day one.)*

### Goal D — Cross-backend / cross-platform parity — `[auto]`
- [ ] Interpret and native produce **identical** results (the both-backend test run covers it).
- [ ] Rendering/output libraries have **gold tests** (e.g. `native/tests/gold.rs`); a regen is a deliberate, reviewed change.
- [ ] wasm: builds + runs if the library targets wasm, else a documented N/A.

### Goal E — Predictable memory — `[review]` (+ `[auto]` guard where applicable)
- [ ] No store-lifetime surprises: heap values freed at scope end, no hidden retention. `[auto]` via `LOFT_STORE_GUARD` for store-managing libs; `[review]` that the API doesn't leak ownership the caller can't reason about.

### Goal C — Capability via dogfood — `[review]`
- [ ] At least one **real consumer or example** exercises the public API (a genuine use, not a toy).
- [ ] Tests cover the public surface — each `pub fn` / `pub struct` has a test or example path.
      *(`loft test` lists the functions a suite never entered — see
      [TESTING.md § What a run did NOT check](TESTING.md#what-a-run-did-not-check--scope-admission-coverage).
      It is a report for this review to read, never a gate: a library is written before
      its consumers exist, so a coverage bar would fail exactly the case the package
      system is meant to support.)*

### Goal F — Friction-free surface — `[review]`
- [ ] The public API expresses **intent**, with no compiler-serving ceremony — it passes the fun-on-pickup bar.
- [ ] Errors **bound the language** ("not supported yet — use X"), never hand the user a proof obligation.

### Goal B — Release & legibility — `[auto]` + `[review]`
- [ ] `[auto]` Published: signed registry index entry (`url`/`sha256`/`size`/`loft`/`deps`), a release tag, a CHANGELOG note.
- [ ] `[auto]` `loft install <name>` resolves → verifies signature → installs, end-to-end.
- [ ] `[review]` README is **legible on contact**: what it's for + a runnable first example, in the first screen.

### API surface quality — `[auto]` + `[review]` (see [API_SURFACE.md](API_SURFACE.md); tool: `scripts/api_lint.py --check <lib>`)
- [ ] `[auto]` No accidental duplicates (same name **and** signature), no confusable-name clusters, no asymmetric overload sets.
- [ ] `[auto]` Every `pub fn` / `pub struct` documented; naming consistent (one spelling per concept).
- [ ] `[review]` No redundant "two ways to do one job"; no same-type-param swap footguns; names express intent.
- [ ] `[review]` No brittle setup / hidden state: no usage sequence whose violation is silent — a setup-order contract is encoded in the types, eliminated, or a loud error (never silent-wrong).

### Documentation structure & section review — `[auto]` + `[review]` (tool: `scripts/doc_review.py <lib>`)
The library is reviewed the **same way as the stdlib** — `doc_review` enumerates the
library's own internal sections (`// --- Section ---` dividers; README `##` headings)
and checks each as a unit. This reuses the structure the docs already have, so the
review can't gloss (every section is a line) and a section signed off stays signed
off until *its* text changes (content-hash ledger).
- [ ] `[review]` The public API is **split into clear sections** — each a coherent unit a user can navigate in the rendered docs *and* a reviewer can sign off. No catch-all `(intro)` blob holding the real API. `doc_review` **red-flags any section with ≥20 public items** (`MAX_SECTION`, tunable) as too big to review or scan — split it into sub-sections.
- [ ] `[auto]` `scripts/doc_review.py <lib>` → every section cleared: **auto** (api_lint clean + no temporal/hedge language) and **review** (signed off per section).
- [ ] `[review]` No stale docs (see [API_SURFACE.md § S7](API_SURFACE.md)): examples run, referenced symbols resolve, no `currently`/`planned`/`for now`/`TODO` language.

### Documentation quality — `[review]` (see [DOC_QUALITY.md](DOC_QUALITY.md))
- [ ] README: purpose + a copy-paste quick-start + the "fun-on-pickup" first path.
- [ ] A doc comment on **every** `pub fn` / `pub struct`: present-tense, says **why to use it** (not its history), in plain language readable by entry-level and non-native-English readers.
- [ ] An `examples/` dir with runnable programs.
- [ ] No dead plan-tag / date narration in comments; only live pointers (issues / plans).

---

## Relationship to other docs
- [LIBRARY_AUTHORING.md § 3](LIBRARY_AUTHORING.md) "Pre-release checklist" is the `[auto]` mechanical core of this list (tests, warnings, version, deterministic package); this doc is the superset (adds the Goal-by-Goal + doc-quality `[review]` bar + the verified administration).
- [GOALS.md](GOALS.md) defines the six goals; this applies them at the library level.
- [LIBRARY_AUTHORING.md § 5e](LIBRARY_AUTHORING.md) is the dev workflow that produces a release; this is the bar that release must clear.
