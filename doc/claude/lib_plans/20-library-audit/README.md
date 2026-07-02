<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 20 — Library health audit (`lib_audit`)

> `20` is the [`loft-lang/plans`](https://github.com/loft-lang/plans/issues)
> issue id `@PLN20` (a tracker number, **not** a local sequence — legacy local
> dirs `21`/`31` predate the tracker).  The issue is unfiled; see Open question 1.

## Status (REQUIRED)

Open — design ready, no implementation.  Motivated by a concrete miss: `loft-libs-net`
sat **unvalidated against current loft for ~2 weeks** — its `main` CI last ran
2026-05-31 (before June's #354/#357/#361 stability work), its one in-flight branch
had no PR so `on: pull_request` never fired, and the only live cross-check
(`scripts/verify_external_libs.sh`) is both narrow (8 of 15 packages) and manual, so
nobody ran it.  `loft-libs-net#4` fixes that one symptom; this plan is the systemic
answer — a verification that **runs by itself and cannot silently report green when a
library is broken**.

## Goal (REQUIRED)

One command (and one nightly job) that reports the true health of every
`loft-lang/loft-libs-*` package against current loft — unmerged branches, PRs
needed, interpreter+native correctness, registry currency — auto-fixing what is safe
to auto-fix and reporting the rest, and that fails loudly rather than under-reporting.

## Effort + design (OPTIONAL — recommended)

- **Effort:** S–M (one bash audit + a scheduled workflow; reuses existing scripts).
- **Design:** ✓ (this doc).
- **Deliverable:** `scripts/lib_audit.sh` (read-only by default) + a nightly CI job.

## Composition matrix — Stage A

N/A — pure tooling; adds no language value, type, or operation, so there is no
language-composition surface to enumerate.  **In its place, the audit's own
correctness is the matrix**: its cells are every `(repo × package × backend ×
verdict)` it must classify, and the verdict taxonomy + self-failure-mode table below
*are* the spec for "the tool is right."

## The invariant — why this needs a design, not a checklist

> **The report tells the truth about every `(repo, package, backend)` target: it
> never reports OK for something broken (false-clean), never FAIL for something fine
> (false-alarm), and never silently omits a target (blind spot).**

A verification tool that lies is *worse than no tool* — false confidence is exactly
what let net rot.  Every mitigation below serves this one invariant; the failure
modes are the ways it breaks.

## R1 — Ecosystem states the audit must detect, and fix-or-report

The five questions, each a check with an explicit verdict and a **FIX** (the audit
remediates) or **REPORT** (surface for a human / a later step) disposition.

### Q1–Q3 · Branch & PR hygiene (per repo)

"Merged?" is **not** `ahead_by==0`: squash and rebase merges (GitHub's defaults)
leave the branch's commits non-ancestors of main, so ancestry / `ahead_by` reports a
genuinely-merged branch as "unmerged" (verified — `loft-libs-core/register-regex`
shows `ahead_by=1` yet was squash-merged via PR #88).  A branch is merged iff **any**
of three independent signals fire, checked most-authoritative first:

1. **PR merged** — `gh pr list --head $BR --state merged` non-empty (authoritative for
   any merge strategy).
2. **Ancestor** — `git merge-base --is-ancestor origin/$BR origin/main` (merge / fast-forward).
3. **Content in main** — `git merge-tree --write-tree origin/main origin/$BR` yields a
   tree identical to `origin/main^{tree}` (squash/rebase with no discoverable PR;
   verified — `loft-libs-core/arguments-warning-sweep`: no merged PR, content in main).

**The safety asymmetry sets the default:** a false "merged" deletes unmerged work; a
false "not-merged" only leaves clutter.  So the verdict is **KEEP unless a signal
fires** — never the reverse — and a `merge-tree` conflict (nonzero exit) is KEEP.

| Signals | State | Disposition |
|---|---|---|
| any of 1 / 2 / 3 fire | merged | **FIX** — delete (guarded; see C4) |
| none + open PR | in review (tracked) | **REPORT** — merge the PR (no new PR) |
| none + no PR | **untracked — unknown work** | **FAIL the gate** + REPORT — open a PR / triage |
| merged-state indeterminate (`SKIP-NET`) | unknown | **FAIL the gate** (cannot claim "no unknown work") |
| default branch | — | **never touched** |

**Unknown work fails — it is not a footnote.**  A branch that is neither merged
(deletable) nor tracked by an open PR is *untracked work no process owns* — the exact
shape that let net rot.  The audit must **not fully succeed while one exists**; an
open PR makes the work *known* (acceptable), but silence does not.

### Q4 · Correctness vs current loft (per package)

Ground truth is a fresh local run, not a CI badge — and it must mirror **every step
`library-ci.yml` gates on**, so the audit *predicts* CI (B4) instead of diverging:

```
( cd $PKG && loft --interpret --tests tests )                  # OK = exit 0 AND >0 tests ran
( cd $PKG && loft --native    --tests tests )                  # OK = exit 0 AND native truly compiled
[ -d $PKG/native/tests ] && ( cd $PKG/native && cargo test --release )   # Rust integration tier
( cd $PKG && loft test --deps )                                # Tier-4 transitive-dep tests
```

Plus the **advisory `[auto]` lints** library-ci runs (REPORT-only, never gating):
`api_lint.py` (API surface), `doc_review.py` (doc staleness), `loft doc .` (docs
build).  Verdicts per check: `OK` / `FAIL` (**REPORT**) / `DEGRADED` / `EMPTY` /
`SKIP-ENV` (see R2).  Stamp every result with the loft SHA + the library
SHA/branch/dirty flag — a result without provenance is not a result.

### Q5 · Registry currency (per package)

`check_registry_coverage.sh` / `registry_maintain.sh --dry-run`:

| Finding | Disposition |
|---|---|
| `missing` / `stale` (repo version > registry) | **REPORT** — publish (laptop-signed; the audit can never sign) |
| `orphan` (registry pkg no repo has) | **REPORT** — informational |
| current | OK |

Caveat carried in the report: detection is **version-based, not content-based** — a
code change without a `loft.toml` bump reads "current" (a false-clean for the
registry; see A8).

## R2 — Failure modes of the audit *itself* (how it could lie) + mitigation

This is the design's load-bearing half: the modes that break the invariant.  Each is
either prevented by construction, auto-fixed, or downgraded to an honest non-OK
verdict — **never collapsed into a silent pass.**

### A · False-clean (OK when it isn't) — the most dangerous class

- **A1 silent native→interpret fallback.** `loft --native test` can fall back to the
  interpreter when the cdylib fails to compile, so "native ok" actually ran interp
  (the literal #354 root: "a native compile failure degrading silently to interp is
  the root reason these went undetected"). → **Mitigation:** require proof native ran
  (cdylib artifact present + newer than the run start, or a strict flag that turns
  fallback into a hard error); emit the **`DEGRADED`** verdict, never `OK`.
- **A2 stale native cache.** A `.so` from a previous loft compiler is reused → tests
  pass against old codegen ("always `rm -rf <lib>/native-auto ~/.loft/build-cache`
  before an A/B"). → **FIX:** clear `native-auto/` + `~/.loft/build-cache` before the
  native run; report the loft SHA the artifact was built against.
- **A3 coverage blind spot.** A package exists but isn't in the test matrix (the
  `verify_external_libs.sh` hardcoded list — 8 of 15). A missing target reads as all
  green. → **Mitigation by construction:** discover packages by globbing `*/loft.toml`;
  assert `discovered > 0` per repo (zero ⇒ clone/path error ⇒ **FAIL the audit**, not
  "nothing to test"); the **`BLIND`** verdict is reserved for "found but unrun" and is
  designed to be impossible.
- **A4 empty suite.** No `tests/` or zero test functions → exit 0, nothing verified.
  → **Mitigation:** count tests executed; **`EMPTY`** verdict, not `OK`.
- **A5 wrong loft.** Local `target/release/loft` not rebuilt → validates stale loft.
  → **FIX:** rebuild; stamp report with loft SHA+dirty and warn if behind
  `loft-lang/loft` main tip.
- **A6 wrong library ref.** Tests a dirty/feature checkout but reports as `main`. →
  **Mitigation:** stamp the exact repo SHA+branch+dirty per package; the "ship ref"
  is explicit, never implied.
- **A8 registry content-vs-version blind spot** (see Q5 caveat) → **REPORT** the
  limitation inline; optional future signal: source-changed-since-last-tag without a
  bump.

### B · False-alarm (FAIL when it's fine) — erodes trust → alert fatigue

- **B1 environmental native failure.** No `rustc` / no `libloft.rlib` / linker gap
  (Windows LNK1181) — not a library bug. (`native_library_suite` already separates
  "environmental — LNK1181" skips.) → **Mitigation:** precondition-check the
  toolchain — reuse `scripts/doctor.sh`'s rustc/cargo/mold/rlib probes rather than
  reinvent them; emit **`SKIP-ENV`**, never `FAIL`.
- **B2 network / API transient.** `gh` rate-limit, registry index or clone
  unreachable. → **Mitigation:** "network trouble is a skip, not a red"
  (`check_registry_coverage.sh` precedent) — retry with backoff, then **`SKIP-NET`**
  (indeterminate), never a false `FAIL` or false "needs PR".
- **B4 warnings-policy drift.** library-ci uses `LOFT_DENY_WARNINGS=1` unless a
  package carries `.allow_warnings`; if the audit's policy differs it green/reds
  differently from CI. → **Mitigation:** mirror the `.allow_warnings` rule exactly so
  the audit *predicts* CI rather than diverging from it.

### C · Didn't run / partial — the meta-failure (this is the net rot)

- **C1 never invoked.** A manual tool nobody runs. → **FIX (structural):** a nightly
  scheduled job (precedent: `miri.yml`) + a pre-release gate; the audit is a gate, not
  a suggestion.
- **C2 partial run looks complete.** Dies mid-sweep (one repo's clone fails) and the
  summary reads "done" for what it reached. → **Mitigation:** enumerate *intended*
  targets up front, diff against *reached*; any unreached ⇒ explicit row + **nonzero
  exit** ("no silent caps").
- **C3 state drift between query and action.** A branch gains a commit, or a PR goes
  red, between the read and the delete/merge. → **Mitigation:** re-verify the merged
  criterion *at action time*, immediately before any mutation.
- **C4 destructive action on the wrong target.** Deleting unmerged work or the
  default branch. → **Mitigation:** never delete the default branch; delete only when
  `(ahead_by==0 OR PR merged)` re-checked at delete-time; **`--dry-run` is the
  default, real deletion only behind `--prune`**; deletions are recoverable from the
  merged-PR ref but treated as deliberate.

### D · Reporting integrity

- **D1 verdicts collapsed.** Folding `DEGRADED`/`SKIP-*`/`EMPTY`/`BLIND` into
  pass/fail destroys the signal that prevents A1/A3/A4. → **Mitigation:** a per-target
  verdict **enum** (below); the summary tallies each; the exit code keys off `FAIL`
  **and** `DEGRADED`/`BLIND` (treated as not-green), with `SKIP-*`/`EMPTY` surfaced
  but configurable.
- **D2 no provenance.** → **Mitigation:** every report stamped with loft SHA, each
  repo SHA, and a run timestamp.

### Verdict taxonomy (the honest-reporting linchpin)

`OK` · `FAIL` (real) · `DEGRADED` (native silently ran interp) · `EMPTY` (0 tests) ·
`SKIP-ENV` (toolchain absent) · `SKIP-NET` (unreachable, indeterminate) · `BLIND`
(target found but unrun — must be unreachable; if ever emitted, the audit fails).
**Gate condition — the audit fully succeeds (exit 0) iff ALL hold:** every intended
target was reached (no partial run, C2); every correctness verdict is `OK` (no
`FAIL`/`DEGRADED`/`BLIND`); the registry ship-set is clean; **and every branch is
*resolved* — merged (deletable) or tracked by an open PR.**  Any branch with
untracked/unknown work, or any state the audit could not determine (`SKIP-NET`,
indeterminate), makes it fail — unknown work is a defect, never a silent pass.

## Current tooling — reuse / supersede / adjacent

The audit is **consolidation, not green-field**: every check reuses an existing tool
where one exists.  This table is the explicit accounting (what the audit absorbs,
replaces, or leaves alone) so the plan neither reinvents nor silently drops a tool.

| Tool | Role in the audit |
|---|---|
| `verify_external_libs.sh` | **SUPERSEDE** — the audit is its honest superset (auto-discovery + verdict taxonomy + branch/registry); keep as the thin local subset or retire (Open q. 4). |
| `check_registry_coverage.sh`, `registry_maintain.sh` | **REUSE** — Q5 wraps them (detect; maintenance publishes). |
| `sync-fixtures.sh --check` | **REUSE** — drift signal between the per-PR fixture snapshots and upstream. |
| `doctor.sh` | **REUSE** — its rustc/cargo/mold/rlib probe is the `SKIP-ENV` precondition (B1). |
| `api_lint.py`, `doc_review.py`, `loft doc`, `loft api` | **REUSE** — the `[auto]` lints + docs build library-ci gates on; folded into Q4 (REPORT-only). |
| `loft --tests` (interp/native), `loft test --deps`, `<pkg>/native` `cargo test` | **REUSE** — the correctness tiers Q4 runs (mirror library-ci, B4). |
| In-repo lib harness — `make test-packages` / `test-package-native-tests` / `rebuild-native-cdylibs`, `tests/native.rs::native_library_suite`, `n2_cdylib`/`n3_use_native`/`native_ext`/`native_loader`/`multilib`/`d2b_stdlib_cache` | **ADJACENT — leave alone.** These already gate the in-repo `lib/*` against loft on every loft PR; the audit is their **external-repo counterpart**, not a replacement. |
| `gen-library-catalogue.py`, `tests/api_discovery.rs`, `lint_comments.sh` | **ADJACENT** — catalogue / API discovery / `lib/**/src/` comment quality; the audit reports against the same package set but does not own them. |
| `.github` — `ci.yml`'s `registry-coverage` + fixture-drift jobs, `miri.yml` | **REUSE pattern** — the advisory-job and nightly-schedule precedents for P6. |
| `loft package`/`publish`/`new`/`generate`, `loft-keygen` | **OUT OF SCOPE** — author/publish/sign flow; the audit detects "needs publishing" but never signs. |

## Sub-arcs (phases)

| Item | Disposition | Status |
|---|---|---|
| **P1** — inventory: enumerate repos + packages (`*/loft.toml`), pin current-loft SHA, build loft once | foundation | Open |
| **P2** — branch & PR hygiene: decision table, `--dry-run` default, `--prune` deletes (re-verified) | Q1–Q3 | Open |
| **P3** — correctness: per-package interp+native with cache-clear + native-ran proof + verdict enum | Q4 | Open |
| **P4** — registry currency: wrap `check_registry_coverage.sh` | Q5 | Open |
| **P5** — report + exit status: intended-vs-reached, verdict tally, provenance stamp | invariant gate | Open |
| **P6** — automation: nightly scheduled job (interp every PR is cheap; native nightly per #363 wall-clock) | C1 fix | Open |

## Phase ordering

1. P1 → P3 first: a trustworthy correctness sweep is the core value and exercises the
   verdict taxonomy (the A-class mitigations).  2. P2 + P4 (branch hygiene, registry)
   reuse existing query scripts and are independent.  3. P5 ties verdicts to the exit
   code.  4. P6 wires automation last, once the local command is trusted.

## Open design questions

1. **File `@PLN20`** on `loft-lang/plans` to claim the id (outward-facing — not done
   yet); label `plan` + `subject:libs` + `status:future`.
2. **Exit-code policy** (untracked-work-fails is now firm — see Gate condition).
   Remaining: do `DEGRADED`/`BLIND` fail the gate? (Recommend yes — A-class lies.)
   Do `EMPTY`/`SKIP-ENV` fail, or surface only? (Recommend surface, don't fail.)
3. **Native on every PR vs nightly-only** — native compiles are the slow/fragile part;
   #363 just halved PR wall-clock, so the recommended split is interp-all per PR,
   native-all nightly.
4. **Supersede or fold in `verify_external_libs.sh`** — this audit is its honest
   superset (auto-discovery + verdict taxonomy + branch/registry); decide whether to
   retire it or keep it as the thin local subset.

## Cross-arc dependencies

- Builds on **lib_plan 12 — library-extraction** (the `library-ci.yml` + fixture
  infrastructure this audits).
- Cooperates with the registry maintenance flow (`registry_maintain.sh`) — Q5 detects,
  maintenance publishes.

## See also (REQUIRED)

- [`API_SURFACE.md`](../../API_SURFACE.md), [`LIBRARY_CHECKLIST.md`](../../LIBRARY_CHECKLIST.md) — what a *correct* library is (the audit enforces the runnable subset).
- [`PKG_REGISTRY.md`](../../PKG_REGISTRY.md), [`PACKAGES.md`](../../PACKAGES.md) — registry + package model behind Q5.
- `scripts/verify_external_libs.sh` (narrow predecessor), `scripts/check_registry_coverage.sh`, `scripts/registry_maintain.sh`, `scripts/sync-fixtures.sh`.
- [`lib_plans/12-library-extraction/`](../12-library-extraction/README.md) — CI/fixture infrastructure.
- `@PLN20` on [`loft-lang/plans`](https://github.com/loft-lang/plans/issues) (to file — Open question 1).
