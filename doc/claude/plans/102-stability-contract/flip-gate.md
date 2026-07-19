<!-- Copyright (c) 2026 Jurjen Stellingwerff -->
<!-- SPDX-License-Identifier: LGPL-3.0-or-later -->

# @PLN102 arc E — the `0 → 1` contract flip: gate + drift-gates + procedure

> **Status: DESIGN (2026-07-19).** The flip itself is a **one-line** change
> (`manifest::CONTRACT_VERSION = 0 → 1`); the risk is entirely in *earning* it.
> This doc is the earn-it checklist (what must be true), the one concretely-unbuilt
> mechanism it still needs — the **CI drift gates** that make an omitted contract
> bump LOUD (versioning-decision.md item 4, ⬜) — as a small-safe-step ladder, and
> the flip procedure. Reference: [versioning-decision.md](versioning-decision.md)
> (the pivot), [../../COMPATIBILITY.md § Before the flip](../../COMPATIBILITY.md).

## What the flip is (and why it is a one-way door)

`contract 1` is the 1.0 compatibility baseline. Today the mechanism ships **inert**
at `CONTRACT_VERSION = 0` (nothing declares `contract`, nothing gates). Flipping to
`1` starts the **absolute never-break promise** (arc A): from that commit on, no
functioning program may break — language, errors, libs, and their persisted data.
A wart shipped at the flip is **permanent** (live-with-it or engineer-around); an
error we *failed* to add is unaddable (the error surface is one-directional). So the
flip is gated on a surface-by-surface audit that fixes every regret while contract 0
still allows it.

## The invariant (design-protocol step 1)

> **Nothing declaring `contract 1` may observe a behaviour change after the flip
> commit.** Corollary for the drift gates below: any edit that *would* silently
> change a frozen surface must either (a) be caught pre-flip by the audit, or (b)
> after the flip, force a *contract bump* — never merge silently. The gate's job is
> to convert "silent" into "loud" for every surface that can drift mechanically.

## The gate — preconditions, by surface

The flip may land only when every row is ✅. Most are already there; the table is the
single reconciled view (supersedes the scattered "open" markers).

| Surface | Precondition | State (2026-07-19) |
|---|---|---|
| **Semantics** | INCONSISTENCIES.md High (silent-wrong) all fixed | ✅ *"All fixed — see CHANGELOG.md"* |
| **Semantics** | INCONSISTENCIES.md Medium/Low resolved-as-design-point (documented + regression-guarded) | ✅ all in the resolved table |
| **Errors** | Pre-freeze error surface maximally strict (add every error we might want — one-way) | ✅ Tier 0/1 fixed; E2-A + E2-B shipped (`tuxedo-e1-diag-codes`); sentinel collisions accepted (C85) |
| **Errors** | Diagnostic machine-identity exists (codes, so prose stays improvable) | ✅ E1 shipped (codes are additive post-flip, so full back-fill is NOT a gate) |
| **Syntax** | The last in-flight syntax plans settled (they *define* what contract 1 is) | 🔶 **THE gate** — see "Syntax-settled" below |
| **Stdlib / libs** | lib-audit worklist resolved (H1–H9) | ✅ done or consciously accepted (H8 = C99) |
| **Stdlib / libs** | The dedicated unhurried lib pass (owner phase 2 — stdlib + core libs, equally permanent) | 🔶 the second half of the E gate |
| **Formats** | Layout/persistence identity distinguishes every semantically-distinct layout | 🔶 F9 nullability half open → [layout-nullability-identity.md](layout-nullability-identity.md) |
| **Formats** | Format sub-language (interpolation) warts fixed | ✅ E2-A (unescaped `}`) shipped; format-brace is the last known one |
| **Mechanism** | Drift gates make an omitted bump loud (layout-hash ⇒ bump; golden-corpus ⇒ classify) | ⬜ **unbuilt** — the ladder below |
| **Test hygiene** | The `code!` harness asserts the diagnostics loft *actually* emits (no tolerated-warnings filter) | 🔶 → [test-hygiene-warnings.md](test-hygiene-warnings.md) |

**"Syntax-settled" is not a vibe — make it a query.** The gate is "no open plan with
`subject:syntax`/`subject:language` that changes a *frozen* surface". Operationalise:
`./scripts/idx` (or `gh issue list -R loft-lang/plans --label status:active`) filtered
to language/syntax subjects; each must be closed **or** its remaining work explicitly
classified *additive* (safe to land post-flip). This list — reviewed with the owner —
is the literal flip trigger, not a judgement call.

## The one unbuilt mechanism — CI drift gates (versioning-decision.md item 4)

After the flip, the promise is only as strong as our ability to *notice* a silent
break. Two surfaces drift mechanically and need an automatic "you changed a frozen
thing → bump `contract` or revert" gate. Both are **inert pre-flip** (at
`CONTRACT_VERSION = 0` a bump is a no-op), so they can be **built and proven now** and
simply become load-bearing at the flip — the ideal safe-step shape.

### Gate 1 — layout-hash-changed ⇒ contract must bump

The store/file layout hash (@PLN97) *is* the persistence contract. A change to it is a
silent data break (open Q1, answered: yes, it bumps `contract`).

**Steps 1–2 PRE-EXISTED** (@PLN97 Phase B): `tests/layout_golden.rs` already pins the
layout of a representative corpus against a committed golden (`tests/golden/layout/corpus.txt`)
+ a `LAYOUT_ALGO_HASH` constant, re-blessed with `LOFT_BLESS_LAYOUT=1`, AND a
`layout_coverage_audit` ratchet that fails if the corpus misses any user-writable
storage kind (the "no blind spots" guarantee this design's falsification asks for).
**Step 3 BUILT 2026-07-19** (inert). **Step 4** is procedural (the flip runbook).

| # | Step | Proof | State |
|---|---|---|---|
| 1 | Golden layout baseline over a coverage-audited corpus | a field reorder / stride change flips the golden + `LAYOUT_ALGO_HASH` (the #477 class) | ✅ pre-existing (`layout_golden.rs`) |
| 2 | Drift is loud at commit time (re-bless forced on an intentional change) | `layout_golden` fails red on any layout move | ✅ pre-existing |
| 3 | **Couple to the contract** — a layout change may land only WITH a `CONTRACT_VERSION` bump (else a silent persistence break). `LAYOUT_CONTRACT` pins the contract-at-bless (`layout_golden.rs`); the git-diff rule (`scripts/check_layout_contract.sh`) fails a golden change with no bump. **Inert while `CONTRACT_VERSION == 0`.** | self-test proves the decision table (`--self-test`); both git detectors verified; `layout_contract_pin_is_consistent` guards `LAYOUT_CONTRACT <= CONTRACT_VERSION`; non-blocking CI job in `api-compat.yml` | ✅ **built 2026-07-19** |
| 4 | **Flip-day arm.** No code change — at the flip the gate arms itself from `CONTRACT_VERSION = 1`. Documented in the flip runbook below. | n/a (procedure) | ⬜ at flip |

**Refinement noted (flip-day sharpening):** step 3's git rule flags ANY `corpus.txt`
change (a real layout move *or* an additive corpus entry) — conservative/fail-closed,
which is correct pre-flip. Post-flip, distinguish additive corpus growth from a genuine
existing-type reshape (reuse api-surface commit-5's layout axis, which already separates
`superset` from `changed`) so a coverage addition doesn't demand a spurious bump.

### Gate 2 — golden-corpus-output-changed ⇒ classify + bump

A behavioural break need not touch layout — a changed *observable output* on the
regression corpus is the general silent-semantics detector (shared with arcs A/C).

| # | Step | Proof | Effort |
|---|---|---|---|
| 1 | **Freeze a behavioural corpus baseline.** The both-backend script suite already produces deterministic output; snapshot the observable output (stdout + typed-error codes, NOT prose) of a curated stable subset to `tests/golden/corpus-output/`. Prose excluded on purpose (E1: prose is improvable; the **code** is the frozen handle). | re-run = identical; a deliberate semantic tweak in a probe → diff | S–M |
| 2 | **Diff job, advisory.** CI re-runs the subset and diffs; mismatch prints the diverging cases. Non-blocking first. | injected behaviour change → red naming the case | S |
| 3 | **Classify on mismatch.** A mismatch is one of: *additive* (new output where there was none — allowed), *bugfix* (was wrong per a High-inconsistency — allowed, re-bless), or *silent-break* (a valid program's valid output changed — **requires a contract bump or revert**). The job asks for the classification label; unlabelled → fail. | each class exercised by a fixture; unlabelled silent-break → fail | M |
| 4 | **Flip-day arm.** As gate 1 — live at the flip, documented in the runbook. | n/a | S |

> **Why advisory-first for both:** a required gate that can false-positive on a
> cosmetic change would block every PR the moment it lands. Ship red-but-non-blocking,
> watch it be correct on real merges for a while, *then* make it required — the same
> inert-first ladder arc C used. Each is entirely in-repo, both backends via the
> existing suite; neither touches `main` behaviour until step 3 couples it.

## The flip procedure (the actual small steps, at the freeze)

When the gate table is all ✅ and the syntax-settled query is empty (owner-reviewed):

1. **Pre-flip dry run.** Run both drift gates in *would-this-bump* mode against
   `main`; confirm clean (no un-blessed layout/behaviour drift sitting on `main`).
2. **The one-line change.** `manifest::CONTRACT_VERSION: 0 → 1`. Nothing else in this
   commit. (`ContractCheck`'s `Drifted`/reject/accept arms were built inert in
   B-semantic and become live purely from this constant.)
3. **Bless the baselines as "contract 1".** Tag `tests/golden/layout-hash.txt` +
   `corpus-output/` as the frozen-at-1 references; the drift gates are now required
   (not advisory) from this commit.
4. **Flip the docs of record together:** COMPATIBILITY.md § Before-the-flip → After;
   RELEASE.md gains the flip runbook + the "how to ship a contract-2 change" epoch
   path; versioning-decision.md item 4 → ✅.
5. **Regression: a `contract`-declaring library round-trips.** A fixture lib with
   `contract = 1` loads clean; a synthetic `contract = 2` lib is *rejected below* by a
   `CONTRACT_VERSION = 1` runtime (the reject-below arm proven live, not inert).
6. **Announce** (arc A promise begins) — CHANGELOG + the never-break statement at the
   public intake (arc D, already landed).

Each step is independently revertible up to step 2; steps 3–4 are the point of no
return and land together in the flip PR.

## Falsification — how the flip could still be wrong (design-protocol steps 3–4)

- **A frozen surface with no drift gate.** Layout + behavioural-output are covered;
  the residual is any surface that is neither hashed nor output-observable (e.g. a
  timing/API-shape change). Mitigation: api-surface (C1, done) covers the API shape;
  name any *third* mechanical surface here before the flip or accept it as manual-audit.
- **The corpus subset is too small** → a silent break slips through a gap in coverage.
  Mitigation: gate 2's subset must be curated for *surface coverage*, not size; log
  what's excluded (no silent truncation — an excluded case reads as "covered" if
  unlogged).
- **"Syntax-settled" judged, not queried** → a moving surface frozen by accident.
  Mitigation: the flip trigger is the label query above, reviewed with the owner, not
  a feeling that "things seem quiet".
- **A bugfix mis-classified as additive** post-flip → a real break waved through.
  Mitigation: gate 2 step 3's three classes are exhaustive and each requires a label;
  the *silent-break* class is the default when unsure (fail-closed).

## See also
- [versioning-decision.md](versioning-decision.md) — the pivot; item 4 = these gates.
- [../../COMPATIBILITY.md](../../COMPATIBILITY.md) — the never-break policy (arc A) + § Before the flip.
- [formal-audit.md](formal-audit.md) / [lib-audit.md](lib-audit.md) — the surface-by-surface audit worklists (the gate's evidence).
- [layout-nullability-identity.md](layout-nullability-identity.md) — F9, a Formats-row precondition.
- [test-hygiene-warnings.md](test-hygiene-warnings.md) — the test-hygiene precondition.
- `src/manifest.rs` (`CONTRACT_VERSION`, `ContractCheck`) — the one-line flip site + the inert arms it arms.
