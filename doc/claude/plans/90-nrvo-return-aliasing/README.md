<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 90 — NRVO return-aliasing (the #437 fix's regression)

## Status

| Stage | Status |
|---|---|
| A — Probe catalogue | 🟡 in progress — 5 probes; minimal trigger NOT yet isolated |
| B — Mechanism investigation | 🟡 hypothesis formed; code-only agent dispatched to pin it from bytecode |
| C — Fix design | ⏸️ pending B — multiple sites possible (work-ref alloc / materialize / slot liveness) |
| D — Implementation | ⏸️ pending C |

**What triggered this.** The zero-trust consumer (dogfood) reported that the
released loft (`2026.6.0`, carrying the #437 fix merged as #440) **regressed** —
9/41 of its project failed, all rooted in the shared `lib/ztcbor` CBOR
decode-and-read layer, which sits on the `cbor` library. Bisected by the ZT
agent: green on `../loft2` (pre-#437), red on the installed loft (#437). Confirmed
here at the `cbor` level: `cbor`'s own test suite fails `map 1to2 3to4` on current
loft and passes on loft2. **The #437 fix is a net regression on the release branch
— it fixed one struct-return shape (#437/ct-ci) but broke an entire shared library.**

**Scope going in.** The #437 fix (`src/parser/control.rs`, the `tail_ret_local`
block ~L748 → `ref_return` + `nrvo_collapse_tail_set`) makes a function ending in
`return <fresh-local vector>` use **NRVO**: the returned vector becomes the
caller's `__retbuf`, written in place rather than a fresh store. That is safe in
isolation but **unsound when multiple NRVO'd results coexist** — they alias one
buffer and clobber each other. The user's directive: keep #437's value AND fix the
regression — make NRVO-on-vector-return **correct in all cases**.

## Goal

Make explicit-`return <vector>` NRVO **sound under coexistence**: both the #437
struct case (probe 01) and the cbor map case (probe 02) correct on both backends,
with the whole probe suite green and the `cbor`/`ztcbor` suites restored. Land it
keeping #437's fix (no revert), guarded by graduated regression tests.

## The matrix so far (all on current loft #437)

| probe | shape | result | reads |
|---|---|---|---|
| 01 | ct/ci: `o=[]; for{o+=[lit]} return o`, then `xs=ct(); xs+=[…]`, into a struct | needs the fix (fails WITHOUT #437) | #437 IS for this |
| 02 | cbor `encode_map`: `buf=head(); … ki=encode(key); byte_lt(encode(key_j),ki); buf+=ki` | **`162 3 2 1 4` WRONG** (keys clobbered) | the regression |
| 03 | `CArray`: init-from-call + nested-call appends in a loop, **no coexisting temps** | `131 1 2 3` correct | NRVO nested-append alone is fine |
| 04 | a temp `head()` result coexists with a named `ki` (no loop) | correct | refutes "temp coexistence alone" |
| 05 | loop-structured temp coexistence + `ki` appended to acc | correct | refutes "loop alone" |

**The minimal trigger is not yet isolated.** 03/04/05 each strip one suspected
axis and still pass; only the full `encode_map` (02) reproduces. The remaining
candidate axes (to confirm via the agent's code read + a targeted probe):
`encode`'s **match-dispatch** wrapping `head`; the **vector-of-struct field**
argument `entries[i].key`; `ki`'s **liveness across the inner `byte_lt` loop**.
This unfinished matrix is *why* this is an investigation plan, not a patch.

**Verified pivot:** toggling the `tail_ret_local` intercept OFF (a one-line env
gate, since removed) flips probe 02 to correct `162 1 2 3 4` and probe 01 back to
broken `len=1`. So the intercept is the sole cause, and there is **no
function-local discriminator** (`ct` and `head` have identical bodies
`x=[]; x+=…; return x`) — the fix must live in caller-side buffer allocation, not
a narrowing of the return-site condition.

## Cluster catalogue

| ID | Cluster | Severity | Backend asymmetry | Probes | Doc |
|---|---|---|---|---|---|
| I | NRVO `__retbuf`/work-ref aliasing among coexisting returned-vector results | corruption (silent wrong data) | both (interp confirmed; native TBD by agent) | 01–05 | `cluster-I-nrvo-coexist.md` (pending agent) |

## Probe suite

| File | Shape | Cluster | Status |
|---|---|---|---|
| `01-ct-ci-structs-needs-fix.loft` | struct of 3 copy+append vectors | I (reference for the fix) | PASS with #437; FAIL without — the fix's purpose |
| `02-cbor-map-encode-FAILS.loft` | cbor canonical map encode (real-library extraction) | I | **FAIL** `162 3 2 1 4` on current; correct on loft2 |
| `03-carray-no-coexist-PASS.loft` | nested-call appends, no coexisting temp | I (boundary) | PASS — isolates the boundary |
| `04-temp-coexist-named-PASS.loft` | temp result + live named, no loop | I (refuter) | PASS — refutes simple coexistence |
| `05-loop-coexist-PASS.loft` | loop temp coexistence + append | I (refuter) | PASS — refutes loop-alone |

Probes graduate to `tests/scripts/NN-*.loft` when the cluster fix lands (gate:
assertions pass · clean exit · no leak · bounded runtime).

## Reference ↔ problem pairings

| Problem | Reference | What the diff reveals |
|---|---|---|
| 02 | 03 | both init-from-call + nested-call appends; 02 adds coexisting temp results (`byte_lt(encode(key_j), ki)` while `ki` live) → the aliasing |
| 02 | loft2's 02 | loft2 allocates a fresh store per call (no NRVO); current reuses one `__retbuf` → the divergent allocation |

## Status & next-session roadmap

| Cluster | Mechanism status | Action needed | Effort |
|---|---|---|---|
| I | 🤔 hypothesized (NRVO retbuf aliasing under coexistence) | code-only agent (dispatched) to pin the exact alloc site from bytecode; then a confirmed minimal probe; then the caller-side distinct-buffer fix | M (substrate / store-lifetime) |

Next: (1) incorporate the agent's mechanism report into `cluster-I-*.md`; (2) build
the confirmed minimal probe from its predicted trigger; (3) fix at the pinned site
(distinct buffers for coexisting `__retbuf` results); (4) verify the FULL suite on
**both backends** + the `cbor`/`ztcbor` suites + the loft suite; (5) graduate
guarantee probes to `tests/scripts/`.

## See also

- `src/parser/control.rs` — `tail_ret_local` (#437 intercept), `ref_return`, `nrvo_collapse_tail_set`
- `doc/claude/OWNERSHIP_MODEL.md` / `doc/claude/formal/ownership.md` — the deps/NRVO-safety substrate this feeds (`@PLN85`)
- `doc/claude/STABILITY_REDFLAGS.md` cluster 1 — "return/bind ownership re-derived per-site" (this bug is that cluster biting)
- `../loft-libs-core/cbor/src/cbor.loft` — the regressed library; `../loft2` — the clean pre-#437 reference binary
- GitHub #437 / #440 — the original bug + its (regressing) fix
