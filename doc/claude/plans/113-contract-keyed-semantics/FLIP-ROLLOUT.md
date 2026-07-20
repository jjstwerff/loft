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

- **Step 0 decided it — the minimal path holds.** `2026.7.1` already hard-rejects a too-new
  `[package] loft = ">=X"` lib at load (§ Step 0), so flip-fixed libs just declare `loft = ">=<flip
  release>"`, old binaries reject them cleanly (never run them), and the rollout is **one release**
  (the flip) — **no gate to build, no staging.**
- The gate-building / staged-release contingencies (Steps 1–2) are therefore **not triggered**; they
  stay documented only as the fallback had `2026.7.1` ignored the bound (it doesn't).

(Releases are the loft *binary* — orthogonal to lib versions, the churn @PLN113 arc D exists to avoid:
the gate keeps libraries *off* the release axis, which is exactly how the coupled list stays bounded.)

## The one safety invariant

**No flip-fixed lib is published without its `loft = ">=<flip release>"` bound set** (so an old binary
hard-rejects it at load instead of running it — Step 0 proved `2026.7.1` does exactly that), **and libs
are published one at a time with old-loft rejection re-verified after each.** That means there is never
a moment where a flipped lib can silently run on an old binary — the base is gap-free at every step.
Rollback at any step: **yank** the just-published version (lockfile-pinned consumers are untouched).

## Safe small steps

### Step 0 — Measure the real exposure — ✅ DONE (2026-07-20)
**Result: `2026.7.1` already enforces a `[package] loft = ">=X"` bound at package-load time — a
too-new-requiring package is hard-rejected (`Fatal: "requires loft X but interpreter is 2026.7.1"`,
package not loaded), never run with wrong semantics.** Confirmed two ways: the enforcement is present
in the load path at the `v2026.7.1` tag (`parser/mod.rs:7376` → `check_version` → `Fatal` + return
`None`; the manifest parses `[package] loft`), and empirically on a `2026.7.1`-version binary against a
fixture matrix — `>=2026.0.0`/`>=2026.7.1` load (controls prove the harness can pass), `>=2026.7.2` and
`>=2026.8.0` reject (calendar comparison is patch-precise). The `contract` axis did **not** exist at the
tag, but the calendar-version axis is what the flip uses and it is sufficient.

**Consequences (they simplify the rollout):**
- The **load-time survival gate already exists** — no gate to build for safety (Step 1 collapses).
- What does *not* exist is **resolve-time fallback**: `loft install foo` picks the highest version
  (`install.rs`), so a `2026.7.1` user *downloads* a flip-fixed `foo@latest` and then **hard-fails at
  load** with the clear `requires loft` message — loud, not silent. Graceful auto-fallback to the last
  compatible version is a UX nicety, not a survival requirement.
- Therefore the survival mechanism is simply: **every flip-fixed lib declares `loft = ">=<flip
  release>"`**, and `2026.7.1`'s existing `check_version` does the rest. **No new gate, no staging —
  the minimal one-release path.**

### Step 1 — Gate already exists (Step 0). Optional: resolve-time fallback
The load-time `[package] loft = ">=X"` enforcement (`check_version`) already ships in `2026.7.1`, so
**nothing needs building for safety.** The one optional improvement is **resolve-time fallback** in
`loft install`: skip candidate lib versions whose `loft` bound the running binary fails and pick the
newest satisfiable one, so an old binary installs the last *compatible* lib version instead of
downloading a too-new one and hard-failing at load. That upgrades the experience from "clear error" to
"just works, on the older lib" — a UX nicety, **not** a survival requirement, safely deferrable (even
post-flip). If built, gate it behind a fixture-registry test (old binary → last compatible version; new
binary → flipped version). This is arc D in its version-keyed form.

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
1. Set its manifest **`[package] loft = ">=<flip release>"`** — the bound `2026.7.1`'s `check_version`
   already enforces (Step 0), so an old binary rejects the flipped lib at load instead of running it.
2. Un-draft the PR, run the lib's **parity gate** (`--interpret` == `--native` == wasm where claimed;
   the multi-byte test each PR added), merge.
3. **Touch-sign + publish** via the loft-ship skill — **re-sign `index.json`** (skipping the re-sign
   breaks *all* installs; that is the publish foot-gun).
4. **Verify survival** before moving on: new loft installs + runs the flipped lib; **old loft
   hard-rejects it** with `requires loft >=<flip release> but interpreter is 2026.7.1` (never runs it
   with wrong semantics — the Step-0 behavior); lockfile-pinned users unchanged. (If the optional
   resolve-time fallback of Step 1 was built, old loft instead installs the last compatible version.)

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
