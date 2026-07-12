<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN102 — Building the never-break mechanism (the compat gate + status + folding)

> **Status: design, buildable (2026-07-12).** Implementation plan for the compatibility doctrine in
> [COMPATIBILITY.md](../../COMPATIBILITY.md) — the three legs (**fold** / **host** / **key**) and the
> per-version API-compat *indicator*. **Scoped in two tiers** (§ Two tiers below): **Tier 1** — an
> author-facing, non-blocking PR compat check (C1, build now) — and **Tier 2** — the ecosystem
> label+preserve registry + application tester (designed here, deferred). Read COMPATIBILITY.md for
> the *why*; this is the *how*.

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

## Two tiers — the MVP now, the ecosystem later

The mechanism splits cleanly into **what one author needs to release with confidence** (build now)
and **what the ecosystem needs to guarantee never-break for other people's consumers** (deferred
until it is actually load-bearing). Same verdict engine underneath; different amount of
infrastructure around it.

### Tier 1 — the MVP: an author-facing, non-blocking PR compat check (build now)

The immediate, self-contained value: *"I added a few methods, changed a few things — is my library
still a drop-in?"* Answered by **C1 (the surface diff) wired as a red/non-blocking PR check** on the
library's own repo. Entirely in this repo — no registry, no hosting, no app tester.

- **The verdict is an INDICATOR, not a gate.** It never refuses a change; it *labels* it. Green =
  additive superset (a drop-in); **red-but-non-blocking** = a break, with the offending symbols
  named. You still merge — the check informs, it does not obstruct.
- **Strict.** At the library level *any* change to the existing public surface is a break — even a
  "safe" widening. No per-consumer cleverness, no compat-rules grace: **identical-or-added** is the
  whole rule. Strictness is what makes a green mean something.
- **The baseline is a checked-in `api-surface.json`, regenerated on release.** Backwards-compat is a
  claim about the last *release*, not the previous commit — so the PR check regenerates the surface
  from the branch and diffs it against the committed baseline; refresh the baseline when you cut a
  release. No external state, the diff is visible in the PR, and the baseline is exactly what Tier 2
  later stores (nothing thrown away).

That is C1 (commits 1–6 below) and nothing else.

### Tier 2 — the ecosystem: never-break for *other people's* consumers (deferred)

Only needed once you guarantee that a stranger's program never breaks — not for your own release
confidence. **The trust boundary is the whole reason Tier 1 is not enough here: a lib can lie
through its PR.** The Tier-1 check runs in the *author's* repo, on the author's CI, against a
baseline the author commits — trustworthy for the author (you do not lie to yourself), worthless as
a promise to a *consumer* who did not watch it run. So the ecosystem needs a **gatekeeper**: the
registry INDEPENDENTLY recomputes the verdict on the actual submitted artifact — *the registry
produces the pass, it never trusts a claimed one* — so consumers trust the registry, not the
author's self-report. Note the gatekeeper still gatekeeps **truth, not permission**: it verifies
whether the compat claim is *honest* and preserves the old version; it does **not** forbid a break.

The guarantee is **not** prevention (you cannot stop a third party from breaking, and you do not
want to): it is **preservation**. You **hold the old version forever**, so a consumer who does not
migrate keeps resolving to the exact hosted old version and never sees the break. The price of a
break is a **hosting cost** — storage — and that is the accepted price of never-break. So the
ecosystem layer is **independently-verified label + preserve**, not gate + reject:

- **C2 / C3 — the registry** records the same C1 indicator per version transition and, on a break,
  **preserves the old version first** (C2.5 is the load-bearing act). The human accepts the
  *perpetual hosting commitment*, never *permits the break*.
- **C-app — the application's "am I in danger?" tools.** The app sees the lib's **break points**
  (the symbols C1 named) and asks *do I need to worry if I upgrade?* — answered first statically
  (do I use any of those symbols?) then empirically (do my tests still pass against the upgrade?).
  Because the old version is always hosted, this only ever answers "can I safely move *forward*?".

All of Tier 2 is designed below and deferred; none of it blocks Tier 1.

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

#### C1 — the commit ladder (the MVP: Tier 1, small verifiable steps)

Six commits, each complete + tested before the next (PLANNING.md § goal 5). **The deliverable is
commit 6** — the author-facing non-blocking PR check; commits 1–5 build the verdict it posts.
**Ordering principle: front-load determinism** — the canonical descriptor and its corpus are locked
*before* the diff consumes them, because under a STRICT check a cosmetic re-spelling turns into a
false red on every unrelated PR, and a check you learn to ignore is worthless (§ Falsification names
this the make-or-break). Entirely in-repo — no registry.

| # | Commit | Code-points | Verify | E |
|---|---|---|---|---|
| 1 | **`api-surface`: enumerate the public surface** — new `loft api-surface <path>` + `src/api_surface.rs`; walk `Data` for `pub`-marked, `[library] entry`-reachable defs (fn / struct / enum / typedef / operator); emit `name · kind`, sorted. No signatures yet. | dispatch beside `introspect` (`src/main.rs:~3851`); pub-mark (`src/parser/mod.rs:216`,`:6526`); `Definition.def_type` (`src/data.rs:2513`); `[library] entry` (`src/manifest.rs:102`) | a fixture prints exactly its N pub symbols; a `pub` / non-`pub` / unreachable fixture proves the filter | S |
| 2 | **Attach resolved signatures** — fn params (name+type) + return (+ `returned_not_null`), struct/enum field & variant layout (value types), operator sigs; spellings via `Type::show`. | `Definition.{returned,returned_not_null,variables}` (`src/data.rs:2526`+); `Type::show` (`src/data.rs:1751`) | golden descriptor over every kind matches byte-for-byte | S |
| 3 | **Canonicalise → determinism (THE make-or-break)** — normalise type spellings + ordering so a cosmetic edit is not a diff: canonical type form (alias resolution, stable rendering), stable sort key, whitespace-normalised. | `src/api_surface.rs` canonicaliser; `Type::show` normalisation | **determinism corpus**: cosmetically-different-identical-surface pairs (reordered defs, renamed private local, reformatted, alias-vs-expanded) → byte-identical; a real change → differs. Over-invest — strict has no escape valve for a false break. | M |
| 4 | **The diff engine — strict `Superset` vs `Break`** — new `src/api_diff.rs`, a pure fn over two canonical descriptors. **Identical-or-added is the whole rule:** every existing symbol present byte-for-byte + additions-only → `Superset`; ANY change to an existing symbol → `Break`, naming it. No "compatible widening" grace — that *is* the strictness (and it collapses the old separate compat-rules step). | new `src/api_diff.rs` | a fixture pair per class (pure additions → Superset; changed / removed / reordered-layout → Break; a "safe" widening → Break, proving strict) | S |
| 5 | **Verdict output — machine + human** — `loft api-surface <path>` writes the descriptor; `--diff <base> <new>` emits the verdict as machine JSON (for the CI to annotate) + human text (the PR comment), **naming the broken symbols** (the break points Tier-2's C-app later consumes). | `src/main.rs` (`--diff` flag, cf. `introspect --diff`); `src/api_surface.rs` writer | JSON round-trips; human text names broken symbols; a `--diff` smoke on two fixture versions | S |
| 6 | **The non-blocking PR check (THE deliverable)** — a checked-in `api-surface.json` baseline per library (regenerated on release); a CI job regenerates the surface from the branch, `--diff`s against the baseline, and posts **red-but-non-blocking** on a non-superset, naming the symbols. Dogfood on our own libs. | CI workflow (a NON-required status / annotation); `make ci`; committed `api-surface.json`; run over `web` / `server` / … | on our libs: green today; **fires red on an injected break** (positive control — no vacuous green); the red does **not** block merge | S |

Shape: keystone with the risk in commit 3 and the proof (dogfood + positive control) in commit 6.
Commits 1–3 = design's C1.1 · 4 = C1.2 (strict, so the old compat-rules step is gone) · 5 = C1.3 ·
6 = the author-facing PR check — the whole Tier-1 MVP. Entirely in-repo — no registry — which is why
it is buildable now while Tier 2 waits on it.

### C2 — The registry: independently-verified label + preserve (arc B-registry) · TIER 2, DEFERRED · repo: loft-lang/registry

The **gatekeeper** — the ecosystem's trust anchor, needed because *a lib can lie through its own PR*
(Tier 1 runs on the author's CI, against a baseline the author controls). `pr-validate`
INDEPENDENTLY recomputes the verdict on the submitted artifact — *the registry produces the pass, it
never trusts a claimed one* — and on a break preserves the old version. It gatekeeps **truth +
preservation, not permission**: it verifies whether the compat claim is honest and archives the old
version; it never forbids a break. It ORCHESTRATES the loft CLI; it does not re-implement compilation.

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
- **C2.5 — on a break, preserve first (THE guarantee).** Before the new version lands as a distinct
  opt-in version, guarantee the prior version's artifact + baseline stay hosted (append-only); never
  overwrite. This preservation — not any gate — is what makes never-break true; the hosting cost is
  its accepted price.
- **C2.6 — human acceptance pass.** The automation attaches the evidence (verdict + logs +
  baseline); a human accepts the **perpetual hosting commitment** (is the baseline *adequate*? worth
  carrying forever?) — never *permits/forbids the break* (the break is always allowed; the indicator
  just labels it). Reject survives only for the usual reasons — malicious, won't build — never for
  "it's a break".
- *Verify:* a `testpkg_compat_*` fixture set — a clean additive bump (→ accept, verdict Superset), a
  breaking bump (→ verdict Break, old preserved, new is a new epoch), a weak-tests case (→ flagged
  for the human).

### C3 — Stored status + data model · TIER 2, DEFERRED · repo: loft-lang/registry index

- **C3.1 — per-version-transition verdict field.** Extend the registry index (`index.json`) entry
  with the C1 verdict (`compat: "superset" | "break"`, plus the broken symbols) **auto-computed on
  import, never author-declared.** This is the NEW status (COMPATIBILITY.md § What it means for the
  programmer) — today the index carries only name/version/url/sha256/deps.
- **C3.2 — baseline + built-artifact storage** (C2.4) keyed by (name, version, target).
- **C3.3 — surface it to the consumer.** `src/registry.rs` (`PackageStatus`) gains a compat-aware
  variant so `loft install`/a new `loft upgrade` can print *"0.3.0 is a drop-in over 0.2.0"* vs
  *"0.3.0 is a breaking change — migration needed"* before the user moves. Re-sign the index (the
  loft-ship re-sign foot-gun — editing `index.json` without re-signing breaks every install).

### C-app — "Am I in danger?": the application's upgrade-risk tools · TIER 2, DEFERRED · repo: loft

Given a candidate library upgrade, an APP asks the question the lib's own PR check cannot answer for
it: *do I need to worry — am I in danger if I upgrade?* Two layers, static-then-empirical:

- **C-app.1 — the static danger check (surface).** Take the lib's **break points** — the named
  broken symbols C1 already computes (published per version by C3, or computed locally by diffing the
  two lib versions) — and intersect them with what the app actually USES: a usage-side surface walk,
  the sibling of commit 1 (the app's external references, not a lib's exports). Result: the exact
  symbols the app touches that broke. **Empty → not in danger (surface-wise); non-empty → here are
  your danger points.** Instant — and it is *why C1 must name symbols, not just say break/superset*.
- **C-app.2 — the empirical tester (behaviour).** The static check is blind to a symbol that kept its
  signature but changed behaviour, so resolve the app against the upgrade and run the app's OWN tests
  (`loft test`) — pass = safe to adopt, fail = a break *for this app*. Deeper option: characterization
  (capture the app's observable behaviour on the old resolution, replay on the new, diff — reuses
  `src/ss.rs`) for an app with thin tests. Build C-app.1 first; add C-app.2 only if thin app-tests
  prove inadequate.

Because the old version is always hosted (C2.5), staying put is the guaranteed backstop, so C-app
only ever answers "can I safely move *forward*?", never "am I forced to?". The static layer scopes
the worry to your own touch-points; the tester confirms whether they actually bite.

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

1. **Tier 1 — C1 (commits 1–6): the author's non-blocking PR compat check.** The MVP; entirely
   in-repo; delivers "release small library changes with confidence" on its own. Everything below is
   Tier 2, deferred until ecosystem-scale never-break is actually load-bearing.
2. **C3.1 (the verdict field + surfacing)** — the smallest registry change: thread C1's output +
   named break points into the index so the local check becomes a stored per-version status
   consumers (and C-app) can read.
3. **C2 + C-app (the independently-verified gatekeeper + the app "am I in danger?" tools)** — the
   largest, cross-repo, human-in-the-loop pieces. C2 is the trust anchor *because a lib can lie
   through its PR*; C-app is the consumer's move-forward risk check. Build on C1 + C3.
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
