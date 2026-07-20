<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Companion rollout (pre-1) — flip len/size now, leave no gaps

**This is the *near-term* sibling of @PLN113, not the post-1 contract-keying design.** The
semantic-keying arcs (A/B/C) stay held for post-1. This runbook does one thing: **complete the
@PLN110 `len/size` flip end-to-end now — a released flipped loft + every published lib surviving it —
so the games (the contract-1 validation gate) build on a stable, gap-free base.** Contract 1 (the
freeze) is a *separate* milestone targeted for **early September 2026**; this is the foundation it is
built on, not the freeze itself.

It pulls **arc D** (the resolver dependency-gate) forward to now — that gate is what lets the libraries
survive — while leaving arcs A/B/C (per-source semantic keying) held.

## The gap this closes (state it honestly)

The flip is already in `main` (#587), but the **released** loft is `2026.7.1`, which predates it. The
resolver's only version axis is `contract`, and **contract stays 0 until the September freeze** — so the
resolver *cannot today* tell a pre-flip binary from a post-flip one. Publish a flip-fixed lib as
`@latest` and a fresh `loft install` on `2026.7.1` pulls it and mis-reads every multi-byte string.
Pre-1 carries no *absolute* promise, but "stable base without gaps" means we **build the gate that
contains this** (and every future loft-version change), not just accept the break.

## Releases: cheap to make, but keep the coupled list bounded

Producing a release is free (CI builds every target binary), but **the preference is fewer releases on
the monthly rhythm, not a proliferation** (owner, 2026-07-20), and the set of changes that *force* a
release stays bounded ([RELEASE.md § What forces a release](../../RELEASE.md)). So the rollout's release
count is driven by need, not convenience:

- **Step 0 decides whether a gate is even needed.** If the released `2026.7.1` resolver already honors a
  `requires loft` bound, flip-fixed libs just declare it, old binaries fall back on their own, and the
  rollout is **one release** (the flip) with no new gate — the minimal path.
- **If `2026.7.1` does NOT honor it,** build the gate (Step 1) and, by default, **fold gate + flip into
  the one monthly flip release**, accepting the residual (fresh-install on a pre-gate binary upgrades —
  pre-1, no promise, tiny cost). The gate then earns its keep protecting *future* binary changes.
- **Stage into two releases only if** the residual genuinely matters *and* a natural earlier monthly
  release can carry the gate — never cut an extra off-beat point release just to shrink a pre-1
  residual. Staging is a safety lever, not the default.

(Releases are the loft *binary* — orthogonal to lib versions, the churn @PLN113 arc D exists to avoid:
the gate keeps libraries *off* the release axis, which is exactly how the coupled list stays bounded.)

## The one safety invariant

**No flip-fixed lib is published until the survival gate is built and verified, and libs are published
one at a time with old-loft survival re-verified after each.** That ordering means there is never a
moment where a flipped lib can reach an old binary unguarded — the base is gap-free at every step.
Rollback at any step: **yank** the just-published version (lockfile-pinned consumers are untouched;
`@latest` falls back).

## Safe small steps

### Step 0 — Measure the real exposure (read before building)
Determine exactly what the **released `2026.7.1`** `loft install` does with a `requires loft` bound it
doesn't satisfy: does it *skip* the too-new version, or ignore the bound and install it? Build a
throwaway registry fixture with a lib version carrying an unsatisfiable bound; run the `2026.7.1`
binary against it. **This decides the residual:** if `2026.7.1` already gates, old binaries are
protected; if it ignores the bound, fresh-install-on-`2026.7.1` is the one accepted pre-1 break (Step
4). Do not build on an assumption here — instrument it.

### Step 1 — Build the loft-release-version gate in the resolver (*what we need*)
Add an enforced **`requires loft-release >= <version>`** to the manifest + `loft install`: when
resolving, skip candidate lib versions whose bound the running binary fails, pick the newest
satisfiable version, and only error (`package X requires loft Y, you have Z`) when none qualifies. This
is distinct from `check_contract` (that gates on the *contract* integer, useless here since the flip
doesn't bump contract). Gate it behind a fixture-registry test: **old binary → falls back to the last
pre-flip lib version; new binary → resolves the flipped version.** Land + verify this **before any lib
is published.** (This is arc D in its version-keyed form; it also serves every future loft-version
behavior change, not just this flip.)

### Step 2 — Release the flip (prefer one; stage only if Step 0 forces it)
**Default — one monthly release** carrying the flip (and the Step-1 gate, if it was needed): e.g. the
`2026-08` monthly `2026.8.0`. Verify on the artifact: `len("café")` = 4, `size("café")` = 5, both
backends; and the gate skips a too-new lib. This release is the floor every flip-fixed lib requires
(`requires loft-release >= 2026.8.0`).

**Stage into two only if** Step 0 showed `2026.7.1` won't gate **and** the residual matters: ship the
gate in an *earlier monthly* release (additive, no flip, risk-free upgrade), let it propagate, then the
flip release — so gate-aware binaries exist before any flipped lib is published. Weigh the extra release
against the fewer-releases preference; pre-1, the residual is a trivial upgrade, so one release is
usually right.

### Step 3 — Publish the flip-fixed libs, one at a time, each gated
Re-verify the held PRs first (state may have moved since they were drafted): docs/markdown, graphics/glb,
game/time, net/web, **plus cbor**. Then, **for each lib in isolation**:
1. Set its manifest **`requires loft-release >= <flip release>`** (so no old binary resolves it).
2. Un-draft the PR, run the lib's **parity gate** (`--interpret` == `--native` == wasm where claimed;
   the multi-byte test each PR added), merge.
3. **Touch-sign + publish** via the loft-ship skill — **re-sign `index.json`** (skipping the re-sign
   breaks *all* installs; that is the publish foot-gun).
4. **Verify survival** before moving on: new loft installs + runs the flipped lib; old loft
   `loft install` resolves the **last pre-flip version** (the gate holds), never the flipped one;
   lockfile-pinned users unchanged.

One lib per step. The base stays gap-free throughout: each lib is at all times either old-compatible
*or* new-gated, never in a broken middle state.

### Step 4 — Close the gaps + verify the whole base
- **Registry sweep:** every published lib is either flip-agnostic (no byte-intent `len/size` — provably
  unaffected) or flip-fixed+gated. Nothing is left half-converted. Record the sweep so "no gaps" is a
  checked fact, not a hope.
- **End-to-end matrix:** `{old loft, new loft} × {fresh install, lockfile} × {each lib}` — old never
  pulls a flipped lib, new works, locked is unchanged.
- **Document the single accepted residual:** a fresh install on a binary that lacks the gate (`<` the
  release that first carries it — the flip release if folded, the gate release if staged) may pull a
  flip-fixed lib and break; the fix is a trivial loft upgrade (pre-1, no promise). Staging only shrinks
  *which* binaries are exposed — it never removes the residual for binaries already in the wild without
  the gate. A contained break on exactly one path — logged in DESIGN_DECISIONS.md, never silent.

### Step 5 — Hand the stable base to the games
With a flipped, gapless base, the games — the contract-1 validation gate ([COMPATIBILITY.md § road to
contract 1](../../COMPATIBILITY.md)) — build on it. Gaps a game surfaces are fixed pre-freeze (contract
0 still permits it), and *that* dogfooding is what earns the September freeze.

## Definition of done
A released flipped loft + the Step-1 gate live; every published lib flip-fixed+gated or provably
unaffected; the Step-4 matrix green; the one residual (if any) documented. No published lib mis-reads
multi-byte text on the loft binary it declares it needs, and no old binary is handed a lib it cannot
run. The base is stable and without gaps — ready for the games.

## See also
- [README.md](README.md) — @PLN113 status + the held post-1 arcs (A/B/C).
- [plans/110-len-size-semantics/](../110-len-size-semantics/) — @PLN110, the flip itself (in-repo,
  merged #587); this is its rollout tail, pulled forward from the freeze to now.
- [COMPATIBILITY.md § The road to contract 1](../../COMPATIBILITY.md) — the freeze (early Sept 2026) +
  the games validation gate this base feeds.
