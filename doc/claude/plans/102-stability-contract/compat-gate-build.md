<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN102 — Building the never-break mechanism (the compat gate + status + folding)

> **Status: design, buildable (2026-07-11).** This is the implementation plan for the compatibility
> doctrine written in [COMPATIBILITY.md](../../COMPATIBILITY.md) — the three legs (**fold** / **host**
> / **key**), the deprecation-as-signal+fold reframe, and the new per-version API-compat status. It
> names every component with concrete steps and code-points. Read COMPATIBILITY.md first for the
> *why*; this is the *how*.

## The one invariant (design-protocol step 1)

> **Once an artifact is accepted at a declared `contract`, it runs with that observable behaviour
> forever, and a later version never silently changes it.** Every component below exists to make that
> invariant enforceable, and — the new part — to make its *state visible* (is version N+1 a drop-in
> over N, or a break?).

## What already exists (do not rebuild)

- **Leg 3 — key (contract-keying), the loader half.** `src/manifest.rs`: `CONTRACT_VERSION`
  (`:672`, ships at 0), `check_contract` (`:711`) → `ContractCheck::{Ok,TooOld,Drifted,Malformed}`,
  and `check_version`/`VersionCheck` (`:576`/`:522`, arc B-mechanical — bounds/ranges parser). The
  `[package] contract` field is parsed but inert (no lib declares it yet). **The escape-valve
  *behaviour split* (edition-style — the compiler serving different behaviour by declared contract)
  is deliberately NOT built** — design it when the first genuine keyed change needs it.
- **The registry client.** `src/registry.rs`: `PackageStatus` (`:50`), `installed_packages`
  (`:202`) — resolve/install. This READS the registry; the gate below WRITES it.
- **A baseline/snapshot substrate.** `src/ss.rs` (`check_against_baseline`, called from
  `main.rs:3610`) + the @PLN97 layout-hash work — reuse this for the gate's "store more" baseline
  rather than inventing a second snapshot format.

## The components to build

Each is scoped to one repo. **loft** (this repo) grows the *tools*; **loft-lang/registry** grows the
*gate* that calls them; the registry *index* grows the *status*.

### C1 — API-surface extraction + diff (the verdict engine) · KEYSTONE · repo: loft

The whole "informed migration" status is a function of this. Build it first; it is independently
useful (our own libs, CI) before any registry work.

- **C1.1 — extract a canonical public surface** for a package. Source of truth: the parser's
  `pub`-marked definitions (functions, structs/enums, typedefs, operators) reachable from
  `[library] entry`. Emit a stable, sorted, normalised descriptor (name · kind · full signature with
  resolved types · field/variant layout for value types). Code-point: a new `loft api-surface <path>`
  subcommand beside the existing `introspect`/`check` dispatch (`src/main.rs:~3851`), reading the same
  `Data` the parser fills (`src/data.rs`; `pub` flag on `Definition`). Determinism is the hard part —
  canonicalise type spellings and ordering so a cosmetic edit is not a diff.
- **C1.2 — diff two surfaces → a verdict.** `Superset` (every old symbol present with a
  compatible signature; only additions) → **drop-in**; else `Break` (a removed/renamed symbol, a
  changed signature, a narrowed type, a value-struct layout change) with the offending symbols named.
  New module `src/api_diff.rs`; unit-tested with a fixture pair per verdict class.
- **C1.3 — verdict output** as machine-readable JSON (for the gate) and human text (for `loft
  upgrade`).
- *Verify:* run C1 on our own libs across their published version pairs (`web` 0.2→0.3,
  `server` 0.2→0.3, …) and hand-check the verdicts.

#### C1 — the commit ladder (small verifiable steps)

Seven commits, each complete + tested before the next (PLANNING.md § goal 5). **Ordering
principle: front-load determinism** — the canonical descriptor and its fixture corpus are
locked *before* the diff consumes them, because a non-canonical surface makes every verdict
noise (§ Falsification names this the make-or-break). C1 is independently shippable at commit
6 (it gates our OWN libs' additivity before any registry exists), so the keystone earns its
keep before C2/C3 depend on it. Dogfood closes.

| # | Commit | Code-points | Verify | E |
|---|---|---|---|---|
| 1 | **`api-surface`: enumerate the public surface** — new `loft api-surface <path>` subcommand + `src/api_surface.rs`; walk `Data` for `pub`-marked, `[library] entry`-reachable defs (fn / struct / enum / typedef / operator); emit `name · kind`, sorted. No signatures yet. | dispatch beside `introspect` (`src/main.rs:~3851`); pub-mark (`src/parser/mod.rs:216`,`:6526`); `Definition.def_type` (`src/data.rs:2513`); `[library] entry` (`src/manifest.rs:102`) | a fixture lib prints exactly its N pub symbols; a sibling fixture with `pub` / non-`pub` / unreachable defs proves the filter excludes private + unreachable | S |
| 2 | **Attach resolved signatures** — each entry gains fn params (name+type) + return (+ `returned_not_null`), struct/enum field & variant layout (value types), operator sigs. Spellings via `Type::show`. | `Definition.{returned,returned_not_null,variables}` (`src/data.rs:2526`+); `Type::show` (`src/data.rs:1751`) | golden descriptor for a fixture covering every kind (nullable-return fn, value struct, enum, typedef, operator) matches byte-for-byte | S |
| 3 | **Canonicalise → determinism (THE make-or-break)** — normalise type spellings + ordering so a cosmetic edit is not a diff: canonical type form (alias resolution, stable rendering), stable sort key, whitespace-normalised. | `src/api_surface.rs` canonicaliser; `Type::show` normalisation | **determinism corpus**: cosmetically-different-but-identical-surface pairs (reordered defs, renamed private local, reformatted, alias-vs-expanded type) → byte-identical descriptors; a genuinely-different pair → differs. Over-invest here. | M |
| 4 | **The diff engine: `Superset` vs `Break`** — new `src/api_diff.rs`, a pure fn over two canonical descriptors: every old symbol present with a compatible sig + additions-only → `Superset` (drop-in); else `Break`, naming the offending symbols. | new `src/api_diff.rs` | a fixture pair per verdict class (pure additions → Superset; removed symbol → Break; layout change → Break) | S |
| 5 | **Signature-compatibility rules (the policy, isolated)** — factor "compatible signature" into one explicit rule set matching arc A: widened param OK, added-nullable / `?`-relax OK, removed / renamed / narrowed → break. Separate from the diff walk so the policy is one testable place. | `src/api_diff.rs` rule module; policy = `COMPATIBILITY.md` arc A | rule-matrix unit test (old-sig × new-sig → compatible \| break), one row per rule | S |
| 6 | **Verdict output — machine + human** — `loft api-surface <path>` writes the descriptor; `--diff <old> <new>` prints the verdict as machine JSON (for the C2 gate) + human text (for `loft upgrade`). | `src/main.rs` (`--diff` flag, cf. the `introspect --diff` precedent); `src/api_surface.rs` writer | JSON round-trips; human text names broken symbols; a `--diff` smoke on two fixture versions | S |
| 7 | **Dogfood + CI additivity gate** — run C1 on our own published pairs (`web` 0.2→0.3, `server` 0.2→0.3, …), hand-check verdicts; wire a `make ci` check that our libs' surface stays additive (flags an intended break). | `make ci`; `tests/` fixture pairs graduated from earlier commits | hand-checked verdicts match reality; the gate is green now AND **fires on an injected break** (positive control — no vacuous green) | S |

Shape: keystone with the risk in commit 3 and the proof (dogfood + positive control) in commit
7. Commits 1–3 = design's C1.1 · 4–5 = C1.2 · 6 = C1.3 · 7 = the *Verify* line above, made a
standing gate. Nothing here touches the registry — C1 is entirely in-repo, which is why it's
buildable now while C2/C3 wait on it.

### C2 — The registry compatibility gate (arc B-registry) · repo: loft-lang/registry

`pr-validate` becomes the curated gate from COMPATIBILITY.md § Two populations. It ORCHESTRATES the
loft CLI; it does not re-implement compilation.

- **C2.1 — API verification.** Run `loft api-surface` (C1) on the submitted version and diff against
  the *last accepted* version's stored surface (C3). Record the verdict.
- **C2.2 — old-tests regression.** In a sandbox, run the *previous* accepted version's test suite
  against the *new* implementation (`loft test`, `src/main.rs:4123`). A failure is a break. (As
  strong as the lib's tests — COMPATIBILITY.md is explicit about the limit.)
- **C2.3 — sandboxed all-target build + run.** `loft build`/`test` on interpret · `--native` ·
  `--native-wasm` · `--html` (the loft-ship parity gate). Verify, don't trust a claimed pass — *the
  registry produces the pass*. Bound with `LOFT_TIMEOUT`.
- **C2.4 — store more than the author made** (C3): the built cross-target artifacts + a captured
  behavioural/API baseline (reuse `src/ss.rs`), frozen at acceptance.
- **C2.5 — on a break, preserve first.** Guarantee the prior version's artifact + baseline stay
  hosted (append-only) *before* the new version lands as a distinct opt-in version. Never overwrite.
- **C2.6 — human acceptance pass.** The automation attaches the evidence (verdict + logs +
  baseline) to the submission; **a human makes the accept/reject call** (is the baseline *adequate*?
  worth the perpetual commitment?). The gate is a review aid, not an auto-merge.
- *Verify:* a `testpkg_compat_*` fixture set — a clean additive bump (→ accept, verdict Superset), a
  breaking bump (→ verdict Break, old preserved, new is a new epoch), a weak-tests case (→ flagged
  for the human).

### C3 — Stored status + data model · repo: loft-lang/registry index

- **C3.1 — per-version-transition verdict field.** Extend the registry index (`index.json`) entry
  with the C1 verdict (`compat: "superset" | "break"`, plus the broken symbols) **auto-computed on
  import, never author-declared.** This is the NEW status (COMPATIBILITY.md § What it means for the
  programmer) — today the index carries only name/version/url/sha256/deps.
- **C3.2 — baseline + built-artifact storage** (C2.4) keyed by (name, version, target).
- **C3.3 — surface it to the consumer.** `src/registry.rs` (`PackageStatus`) gains a compat-aware
  variant so `loft install`/a new `loft upgrade` can print *"0.3.0 is a drop-in over 0.2.0"* vs
  *"0.3.0 is a breaking change — migration needed"* before the user moves. Re-sign the index (the
  loft-ship re-sign foot-gun — editing `index.json` without re-signing breaks every install).

### C4 — Contract-keying runtime (leg 3) — mostly built

Loader done (above). The **only** new build is the escape-valve *behaviour split*, and it is
**deferred by design** (COMPATIBILITY.md § open decisions): when the first keyed change is genuinely
needed, design how loft serves behaviour by the program's declared `contract` (a `contract >= N`
branch in the relevant op/typing site), edition-style — not speculatively.

### C5 — The folding discipline (leg 1 / arc C) — process + one lint

Folding is mostly discipline, not code, but it can be *guarded*:

- **C5.1 — the discipline:** every recommended-idiom steer ships *with* its fold — the old name
  reimplemented as a thin shim over the new primitive, old code deleted, name kept. Home:
  `default/*.loft` for stdlib; a library's own source for libs.
- **C5.2 — a fold lint (optional):** a check that a symbol marked "superseded" (a new
  `#superseded("use X")` attribute, say) is implemented *via* its successor, not carried as
  independent code — so a steer cannot be shipped without its fold. Code-point: a pass over the
  parsed `Data`, run in `make ci`. Design the attribute only when the first real steer lands (same
  "when genuinely needed" rule as C4).

## Build order (sequencing)

1. **C1 (surface + diff)** — the keystone; standalone value (CI can gate *our own* libs' additivity
   immediately, and it dogfoods the verdict engine before the registry depends on it).
2. **C3.1 (the verdict field + surfacing)** — thread C1's output into the index + `loft upgrade`;
   delivers "informed migration" with the smallest registry change.
3. **C2 (the full gate)** — the largest, cross-repo, human-in-the-loop piece; build on C1 + C3.
4. **C4 / C5** — deferred until a real keyed change / real steer forces them; do not build ahead.

## Falsification / how it breaks (design-protocol step 3)

- **Maintainer forgets to bump `CONTRACT_VERSION` on a silent break** — THE brittleness
  (versioning-decision § failure paths). Mitigation is *outside* this mechanism: the layout-hash /
  golden-corpus CI gates (`src/ss.rs`, @PLN97) that make an omitted bump loud.
- **Gate only as strong as the lib's tests** — C2.2/C2.3 cannot see behaviour the author never
  pinned; C2.6 (the human) and characterization inputs are the backstop, not a proof.
- **API-surface non-determinism** — if C1.1 is not canonical, cosmetic edits read as breaks and the
  status is noise. This is the make-or-break of C1; over-invest in the normalisation + a fixture
  corpus that pins "these two spellings are the same surface."
- **Private artifacts** — hosting (leg 2) covers only the *published* ecosystem; a vendored private
  lib is carried by leg 3 (contract-keying) alone. The gate makes no claim about what it never saw.

## See also

- [COMPATIBILITY.md](../../COMPATIBILITY.md) — the doctrine (§ Folding, § Two populations, § What it
  means for the programmer) this builds.
- [README.md](README.md) — the arc table (A ✅ · B-mechanical/semantic ✅ · **B-registry** = C2/C3 ·
  **C** = C5 · D · E).
- [versioning-decision.md](versioning-decision.md) — the `contract` integer + the failure paths.
- `src/manifest.rs` (leg 3 loader) · `src/registry.rs` (client) · `src/ss.rs` (baseline substrate) ·
  `src/main.rs` (CLI dispatch — where `api-surface`/`upgrade` land).
