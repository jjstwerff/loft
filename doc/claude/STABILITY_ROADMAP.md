<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# STABILITY_ROADMAP.md — every open stability item, in finishing order

> **STANDING RULE — in stability work, bugs get FIXED, not filed.**
> This queue is this agent's stream (feature work — gaming/engine —
> belongs to a parallel agent) and it is **work-limited, not
> time-limited**: done when the queue is finished, not at a date.  A
> surfaced bug gets fixed in the same working session; the bug-filing
> escape hatches (blocks-the-task, too-big-now) do not apply here.
> This is the same standing rule long documented for investigation
> plans (findings live in the plan's catalog and get fixed, never
> double-filed), generalized to all stability work: fixing IS the
> work, so there is no "later" to file for.  Filing re-pays the
> scope/repro/mechanism derivation later and grows a backlog instead
> of shrinking the bug count; with diagnostics warm, the fix is the
> cheapest it will ever be.  An issue is acceptable only as the RECORD
> of a fix in flight (fixed-pending-merge), never as a deferral.

The ONE tracking view over the open stability work that is otherwise spread
across [STABILITY_SWEEP.md](STABILITY_SWEEP.md) (the pass-1 catalog),
[STABILITY_HOTSPOTS.md](STABILITY_HOTSPOTS.md) (the H register),
[STABILITY_PASS2.md](STABILITY_PASS2.md) (relocations),
[DEPS_INVENTORY.md](DEPS_INVENTORY.md) (H2), and `plans/`.  Detail stays in
those canonical homes — this file holds only the ORDER, the size, and the
live status.  When an item lands: flip its row to ✅ with the closing
commit, and update the canonical home as usual.  When new stability work
surfaces: insert a row at its priority, don't append.

Sizes as in STABILITY_HOTSPOTS § Reading the sizes (`S` under a day, `M`
days, `L` a plan with phases).  Every M+ design round runs
[DESIGN_PROTOCOL](DESIGN_PROTOCOL.md) (the `design-protocol` skill); every
fix runs matrix-first (CLAUDE.md § Debugging policy).

## The queue

The bug-level stability work — the F-family sweep, the armed-corpus residuals,
the store-lifetime UAFs, and the **Pass-2 arity-growth cascade** (Reference +
Vector/struct-Enum, #355/#356) — is **complete** (see Done below); nothing in
this queue is an active failure. What remains is **forward-risk hardening** (the
H-register): asserts and tripwires that lock in finished work, then structural
refactors that retire whole future-bug *classes*. In finishing order:

> **▶ NEXT: step 1 (H5).** Its blocker — the arity cascade — is done, so it is
> ready now and is the cheapest high-value move (the validation half of work
> that just landed). Steps 6–8 (H3 / H8 / H4) are the L/M design rounds that
> hold the real forward-risk leverage; each opens with the design-protocol.

| # | Item | Size | Status | Detail + entry point |
|---|---|---|---|---|
| 1 | **H5 leftover asserts — READY (blocker cleared)** — `debug_assert` the two-pass contract where it's now cheap: attr COUNT per def equal at end of both passes (now an invariant post-arity-cascade, not an aspiration); work-ref counter equality per fn. The natural validation hardening of the just-completed arity work — land it to lock the contract in. | S | ⬜ **next** | [STABILITY_HOTSPOTS § H5](STABILITY_HOTSPOTS.md) |
| 2 | **Plan-53 cluster 2, S4 half (eval-TOS / frame-base alignment)** — parked WIP on branch `plan-53-sanitizer-ci-lever` (HEAD 8abfb8e1): cluster 1 fixed, aligned-V2 allocator half done and validating clean; the eval-TOS rounding half remains. Finishing it makes the sanitizer CI lever fully green = the second standing instrument beside the armed channel. | M | ⬜ parked WIP | plan closed by the @PLAN53 wrap-up; the session handoff is preserved at [plans/finished/53-sanitizer-ci-lever/cluster-2-fix-design.md § SESSION HANDOFF](plans/finished/53-sanitizer-ci-lever/cluster-2-fix-design.md) + `cluster-2-S4-progress.md` |
| 3 | **H6 — one null-sentinel table** — `sentinel(tp) -> Encoding` in `src/data.rs` next to `byte_width`; fill.rs templates, `default_native_value`, `emit_typed_null`, narrow-vector paths consume it. Absorbs the F3 deferred cell (dual DbRef null encoding — sentinel vs zero-default — as latent byte-level risk). Independent; "do in any gap". | S–M | ⬜ | [STABILITY_HOTSPOTS § H6](STABILITY_HOTSPOTS.md); F3 row in [STABILITY_SWEEP](STABILITY_SWEEP.md) |
| 4 | **H7 short half — IR codec round-trip property test** — materialize every `tests/scripts/` construct into a store, read back, assert IR equality. DUE the moment the next `Value`/`Type` variant lands (a new variant currently compiles cleanly with a silent codec gap). | S | ⬜ tripwire | [STABILITY_HOTSPOTS § H7](STABILITY_HOTSPOTS.md) |
| 5 | **H7 long half — derive the IR codecs from one schema declaration (F9)** — encoder, decoder, and the exhaustive walker all derive from one macro/table; a new variant then breaks the build until all three know it. Own design slot (codecs encode FIELDS, `for_each_child` can't drive them). | M | ⬜ design | [STABILITY_HOTSPOTS § H7](STABILITY_HOTSPOTS.md); deferred row in [STABILITY_PASS2 § Work list](STABILITY_PASS2.md) |
| 6 | **H3 — ownership as carried data** — UNLOCKED (H1 + H2 both shipped). The variable table becomes the single home for per-var ownership state (owned / borrowed-view / caller-buffer / captured), written once at construction, read by every scopes analysis instead of re-derived; convert one analysis at a time with `debug_assert_eq!(carried, derived)` cross-checks. Absorbs the sweep's pass-2 notes: `value_reads_var` predicate centralisation (+ its ~30 default-arm variant audit, F2), `base_var_of` unification. **The highest-leverage forward-risk item.** | L | ⬜ design round | [STABILITY_HOTSPOTS § H3](STABILITY_HOTSPOTS.md) |
| 7 | **H8 — the `Stores.allocations` privacy pass** — design the accessor surface (worker-slot claim/release, swap-back, lock APIs) as ONE batch with THREADING.md in hand, then convert the 60+ direct touches mechanically. Absorbs the STABILITY_PASS2 deferred accessor rows (`types[].parts` reads, Definition field reads). MUST precede any new `par` feature work. | M–L | ⬜ | [STABILITY_HOTSPOTS § H8](STABILITY_HOTSPOTS.md); deferred rows in [STABILITY_PASS2 § Work list](STABILITY_PASS2.md) |
| 8 | **H4 medium half — extend the `#rust`-template idea upward** — the free-op family, null-init emission, and the GET→SET table become one declaration each that both backends derive, the way fill.rs derives ops. Includes the F4 op-coverage sentinel (enumerate ops lacking `cross_mode` cells) as its completeness check. The L half (one shared lowering IR) stays 1.1+ and explicitly NOT before step 6 (H3). | M | ⬜ | [STABILITY_HOTSPOTS § H4](STABILITY_HOTSPOTS.md); F4 row in [STABILITY_SWEEP](STABILITY_SWEEP.md) |
| 9 | **The instrument plans — fuzzing + sanitizer expansion** — open when reached (or when any item above needs the instrument earlier): @PLAN55 program-level fuzzing, @PLAN56 sanitizer coverage expansion, plus the sweep's "store-level fuzz harness" deferred instrument (store.rs LLRB/coalesce/claims cells). The remaining pass-1 DEFERRED cells (F1 diagnostics-altering-flow, F5 odd-size adjacency, F6 P191 late-mutation, F8 crafted attr/var collision, F9 lib-path axes, F10 par text buffers, match-unification, dispenser stress) fold into these corpora rather than being probed by hand. | L | ⬜ future plans | [plans/future/55](plans/future/55-program-level-fuzzing/README.md), [plans/future/56](plans/future/56-sanitizer-coverage-expansion/README.md); DEFERRED markers throughout [STABILITY_SWEEP](STABILITY_SWEEP.md) |
| 10 | **Pass-3 de-dup tail** — deletion candidates by usage sentinel: `generation/ops` post-plan-57 rc remnants, the codegen_runtime helper-duplication inventory, `towards_set` dual discriminators, the variables size-table exhaustive-match chokepoint. Opportunistic; each is small once its neighbourhood is touched by steps 6–8. | S each | ⬜ opportunistic | module rows in [STABILITY_SWEEP § Module work list](STABILITY_SWEEP.md) + [STABILITY_PASS2 § Work list](STABILITY_PASS2.md) |

Standing discipline (not queue items): every lowering-semantics change lands
with a `cross_mode!` cell or `tests/scripts/` file (H4's S half — add as a
CODE.md checklist line with the next CODE.md touch); every M+ design through
the design protocol; verify-armed before trusting the armed channel's silence
([reference: STABILITY_SWEEP § armed-channel restoration](STABILITY_SWEEP.md)).

## Done (this cycle — closing commits in the canonical homes)

- **Pass-2 arity-growth CASCADE — COMPLETE.** Reference arm 2026-06-12 (one-buffer design: arity = signature+1 fixed at declaration, every return path delivers in THE `__retbuf`; 10-cell matrix green both backends). Vector/struct-Enum arm then closed via **#355** (multi-return-site vector behind a forward caller silently returned the WRONG element) and **#356** (mid-body `return f(g(x))` returned the null sentinel on native; fn-refs to struct-returning fns couldn't be CALLED — the latter also fixed by **#383**, merged in #393 this session). Both #355/#356 CLOSED. Live guards `tests/issues.rs::pass2_arity_growth_*` + `tests/scripts/387-text-fn-ref.loft`. The @PLAN59 growth assert (the H2 residual) fell out — cleared.
- **Armed-corpus residuals ×4** — all resolved 2026-06-14: `132` silent interpreter UAF → `OpFreeRefIfDistinct` (regression `tests/scripts/372`); `collections.loft` two bugs → `parse_object_field` accepts `{}` (regression `373`) + `dedup_keyed` `secondary` flag for `other_indexes` (regression `374`); `166` verified over-strict; `75-native-stub` clean on the rebuilt armed binary. The armed channel's silence is now trustworthy.
- **H1 analysis-dependent arity** — RETIRED 2026-06-11 (@PLAN59 phases 0–2; signature-time `__retbuf`, uniform return ABI, retro-patch deleted).
- **H2 typed deps** — steps 1–5 DONE 2026-06-12 (`Deps` newtype, space-asserting accessors, the positional contract retired via `CALLEE_FRAME_BIT`).  Residual (the @PLAN59 growth assert on two lib fns) → cleared with the arity cascade above.
- **F11 error-path state** — swept, all four breaks FIXED 2026-06-12.
- **The armed-channel restoration** — four stale duals fixed 2026-06-12; the channel is the standing instrument.
- **`store.rs:1640` armed row (the "keyed armed UAF", 7 files)** — RESOLVED 2026-06-12 (4cba84c5): three mechanisms (header-as-`room` accessor → `Store::record_words`; parallel s_pos array header stomp; OpDatabase bytes-vs-words under-claim = a real release OOB write).  Armed corpus 12 → 5 files.
- **Plan-57 vector store-lifetime watermark** — CLOSED (@PLN2; rc removal complete).
- **Plan-53 cluster 1 + the aligned-V2 allocator half of cluster 2** — fixed/validating; the S4 half is queue #4.
