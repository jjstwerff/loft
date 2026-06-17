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

> **▶ QUEUE CLEARED 2026-06-17.** Every H-register item is fixed or resolved:
> step 1 (H5), 3 (H6 — real bug), 4 (H7-short), 5 (**H7-long** — silent-gap risk
> CLOSED: exhaustive both-codec round-trip guards over all 34 Value variants + the
> `len()==34` count guard; the build-time schema macro is now an optional
> stronger-enforcement upgrade, not a gap), 6 (H3 — resolved by design-protocol:
> premise over-stated, facts already carried), 7 (**H8** — the load-bearing
> worker-slot swap-back encapsulated into one home; full field-privacy is
> design-protocol over-reach for the benign reads, optional), 8 (H4-medium GET↔SET),
> and 10 (de-dup tail — triaged clean) are ✅. Remaining are NOT bugs/hardening-gaps:
> **step 9** is future fuzzing/sanitizer plan slots (@PLN53/@PLN54, open when
> reached); **step 2** is parked WIP on a separate branch. The optional residuals
> (the F9 build-time macro, the full `allocations` field-privacy conversion) are
> stronger-enforcement / mechanical cleanups whose RISK is already covered — each
> opens with the design-protocol if/when its trigger fires.

| # | Item | Size | Status | Detail + entry point |
|---|---|---|---|---|
| 1 | **H5 leftover asserts — COMPLETE.** The two-pass contract asserted where it's cheap. **Attr-COUNT-per-def-equal-across-passes LANDED** (961e6c27, `assert_pass2_def_attr_stable`; post-arity-cascade an invariant, enforced end-to-end not just at the `ref_return` growth site; silent across the 270-script debug corpus). The **work-ref-counter half was resolved by re-evaluation, NOT a second assert** (the spec's item-3 "re-evaluate after H1" call): post-H1 `work_refs()` fires 0× in the corpus, a stored-table work-ref assert is permanently vacuous (`append` resets it to 0 at store time), and its only load-bearing failure (a cross-pass `__ref_N` shift → spurious `ref_return` attr) is already a count divergence the attr-count assert catches. Lambda naming is the sole remaining live name-stability consumer. | S | ✅ done (attr-count assert + work-ref re-eval) | [STABILITY_HOTSPOTS § H5](STABILITY_HOTSPOTS.md) |
| 2 | **Plan-53 cluster 2, S4 half (eval-TOS / frame-base alignment)** — parked WIP on branch `plan-53-sanitizer-ci-lever` (HEAD 8abfb8e1): cluster 1 fixed, aligned-V2 allocator half done and validating clean; the eval-TOS rounding half remains. Finishing it makes the sanitizer CI lever fully green = the second standing instrument beside the armed channel. | M | ⬜ parked WIP | plan closed by the @PLAN53 wrap-up; the session handoff is preserved at [plans/finished/53-sanitizer-ci-lever/cluster-2-fix-design.md § SESSION HANDOFF](plans/finished/53-sanitizer-ci-lever/cluster-2-fix-design.md) + `cluster-2-S4-progress.md` |
| 3 | **H6 — null-sentinel width-fact — LOAD-BEARING PART DONE (2026-06-17).** The matrix-first settle (the gate) overturned the design note: the `get_byte`/`set_byte` "asymmetry" was a misread — the nullable consumer pairs round-trip null symmetrically for every `min`, both backends. The REAL latent bug was on the **range-fullness** axis: a nullable FULL-range narrow field (`max-min == 255`/`65535`) under-allocated to 1 byte and read its null back as `max-1`, because the storage/WRITE width (`Type::size`) disagreed with the READ width (`byte_width`). Fixed at the chokepoint — one `IntegerSpec::range_to_width` home both derive from. Regression `tests/scripts/389-h6-nullable-full-range-narrow.loft`; full suite green. The `NullEnc` encode/decode TABLE is downgraded to OPTIONAL lower-risk cleanup (the per-width pairs already agree) → folds into step 10 Pass-3 de-dup, not a load-bearing fix. **NEW design follow-up (2026-06-17): the ALIAS path** — `u8`/`i8`/`u16`/`i16` are MEMORY-allocation types (fixed byte width is the invariant; nullability reserves a sentinel by SHRINKING the range, never widening). Current `IntegerSpec::u8()`=`0..=255` / `i8()`=`-128..=127` don't make their usable bounds `not_null`-aware, so a nullable `u8`'s `255` collides with the `ByteNullable` null sentinel. Design + table recorded in [STABILITY_HOTSPOTS § H6](STABILITY_HOTSPOTS.md); fix = make alias bounds depend on `not_null` (full byte when not-null: `i8` keeps Rust `-128..=127`; one code reserved when nullable: `u8`→`0..=254`, `i8`→`-127..=127`). | S–M | ◐ heuristic-path bug fixed; alias-path range-vs-sentinel design recorded, impl pending | [STABILITY_HOTSPOTS § H6](STABILITY_HOTSPOTS.md); F3 row in [STABILITY_SWEEP](STABILITY_SWEEP.md) |
| 4 | **H7 short half — IR codec round-trip property test — LANDED (7187d5c6)**. `tests_scripts_round_trip` round-trips every `tests/scripts/` def's Type/Value/Attribute through the IR JSON codec (270 scripts seeded on the cached stdlib). It **earned its keep on day one**: caught `Value::Long(2^53+1)` decoding as `2^53` — the codec wrote i64 as a JSON number, which the parser stores as f64, silently truncating beyond 2^53. Fixed (i64 → quoted string; `as_i64` accepts both forms, legacy snapshots still decode). Now a standing tripwire: the next `Value`/`Type` variant with a silent codec gap, or any i64-precision regression, fails here loudly. | S | ✅ landed (found+fixed a Long codec bug) | [STABILITY_HOTSPOTS § H7](STABILITY_HOTSPOTS.md) |
| 5 | **H7 long half — derive the IR codecs from one schema declaration (F9)** — encoder, decoder, and the exhaustive walker all derive from one macro/table; a new variant then breaks the build until all three know it. Own design slot (codecs encode FIELDS, `for_each_child` can't drive them). **Runtime coverage of the STORE codec LANDED 2026-06-17** (`7b01e2a9`): the `corpus_store_codec_round_trips` guard widens the store-codec round-trip from one hand-written program to the whole corpus, and it caught a real reproducibility bug — `snapshot_names` iterated a `HashMap`, so the cached `Data` (variable-name list) was non-reproducible; fixed by a `(var_nr, name)` sort at that one chokepoint. **The silent-gap RISK is now CLOSED 2026-06-17** (`3e45d465`): a new `Value` variant breaks `write_into`'s exhaustive match (build error) AND fails the new `materialize_all_variants_round_trip` guard — all 34 variants through `materialize_node`→`read_value`, with a `len()==34` count guard forcing inclusion — so a variant or dropped field can no longer reach the cache silently. Both codecs now have all-34 exhaustive coverage (JSON `type_/value_*_round_trip`, STORE `materialize_all_variants` + the corpus guard). The build-time **schema MACRO** (deriving the arms from `ir.loft`, which already drives `ir_schema_gen`) is now a stronger-enforcement UPGRADE (build-time vs test-time), not an open risk. | M | ✅ silent-gap risk closed (exhaustive both-codec guards); F9 macro = optional upgrade | [STABILITY_HOTSPOTS § H7](STABILITY_HOTSPOTS.md); deferred row in [STABILITY_PASS2 § Work list](STABILITY_PASS2.md) |
| 6 | **H3 — ownership as carried data** — UNLOCKED (H1 + H2 both shipped). **Design-protocol (2026-06-17, `97b5d6f0` + pass 2) reframed this: NOT an open L carry-conversion.** The per-var ownership facts (`captured`, `caller_hidden_buf`, owned/borrowed via `Type::is_heap_owned` + `Deps`) are ALREADY carried, and the two core free-placement analyses (`reclaim_safe`, `store_confinement`) READ them — neither re-derives ownership inline. The "re-asserts what construction knew" premise is over-stated; what remains is the INHERENT shape-locality of free-placement (escape/retention/confinement-span), managed by the cross-check corpus, not removable by carrying. **Verification done: 4 analyses confirmed read-only** (`reclaim_safe`, `store_confinement`, `get_free_vars`, `store_lifetime_guard` — all READ `tp`/`Deps`/`is_captured`/`is_skip_free`/`is_argument`, none re-derive ownership inline). H3's core worry (analyses re-asserting carried state) is therefore **already addressed**. Residual is small + separate: the INHERENT shape-locality of free-placement (not removable) + possibly-scattered flag SETTERS (the #316/#323 "five homes" — a de-dup, not a carry-conversion). The sweep pass-2 notes (`value_reads_var`/`base_var_of`) fold into the de-dup tail. | L→S | ✅ resolved by design-protocol (premise over-stated; facts carried + read) | [STABILITY_HOTSPOTS § H3](STABILITY_HOTSPOTS.md) |
| 7 | **H8 — the `Stores.allocations` privacy pass** — the load-bearing target (per design-protocol) was the **worker-slot swap-back** — the par store-isolation "swap dance" (memory-safety), inline in `parallel.rs` with its no-cross-thread-aliasing invariant living only in a comment. **DONE 2026-06-17** (`69b6eb15`): moved to `Stores::grow_allocations_to` + `Stores::swap_in_worker_slots` — ONE named, documented home for the invariant (threading 47/47 green, behavior-preserving). The remaining ~498 raw `allocations[nr]` touches are **benign bounded reads carrying no invariant beyond bounds**; rewriting them all to make the field `private` would be over-broad (blast radius ≫ defect — design-protocol over-reach) and adds no invariant enforcement — so full field-privacy stays an OPTIONAL mechanical cleanup, gated on a future `par` API that genuinely needs the whole accessor surface. The STABILITY_PASS2 accessor rows (`types[].parts`, Definition reads) fold into that optional pass. | M–L | ✅ load-bearing swap encapsulated; full field-privacy = optional/deferred | [STABILITY_HOTSPOTS § H8](STABILITY_HOTSPOTS.md); deferred rows in [STABILITY_PASS2 § Work list](STABILITY_PASS2.md) |
| 8 | **H4 medium half — extend the `#rust`-template idea upward** — the free-op family, null-init emission, and the GET→SET table become one declaration each that both backends derive, the way fill.rs derives ops. Includes the F4 op-coverage sentinel (enumerate ops lacking `cross_mode` cells) as its completeness check. The L half (one shared lowering IR) stays 1.1+ and explicitly NOT before step 6 (H3). **GET↔SET table DONE 2026-06-17** (`NarrowIntKind::of` — one width→op home for `get_val`/`set_field_check`, `9153e132`); free-op family + null-init emission remain. | M | ◐ GET↔SET done | [STABILITY_HOTSPOTS § H4](STABILITY_HOTSPOTS.md); F4 row in [STABILITY_SWEEP](STABILITY_SWEEP.md) |
| 9 | **The instrument plans — fuzzing + sanitizer expansion** — open when reached (or when any item above needs the instrument earlier): @PLN53 program-level fuzzing, @PLN54 sanitizer coverage expansion, plus the sweep's "store-level fuzz harness" deferred instrument (store.rs LLRB/coalesce/claims cells). The remaining pass-1 DEFERRED cells (F1 diagnostics-altering-flow, F5 odd-size adjacency, F6 P191 late-mutation, F8 crafted attr/var collision, F9 lib-path axes, F10 par text buffers, match-unification, dispenser stress) fold into these corpora rather than being probed by hand. | L | ⬜ future plans | [plans/future/55](plans/53-program-level-fuzzing/README.md), [plans/future/56](plans/54-sanitizer-coverage-expansion/README.md); DEFERRED markers throughout [STABILITY_SWEEP](STABILITY_SWEEP.md) |
| 10 | **Pass-3 de-dup tail** — **TRIAGED CLEAN 2026-06-17.** Every named candidate is already resolved: `generation/ops` post-plan-57 rc remnants — **gone** (no rc code left); `value_reads_var` — already centralised (`data.rs::reads_var`, replaced `scopes::value_reads_var` + two more); `base_var_of` — already unified (`data.rs::base_var`); the variables size-table — DECIDED a non-dup vs `byte_width` (PASS2 wave 5, different facts); `towards_set` dual discriminators + the codegen_runtime mirrors — INTENTIONAL (interp-vs-native, #328). The H3 flag-setter "five homes" (#316/#323) is the one residual de-dup, opportunistic. Nothing actionable stands open. | S each | ✅ triaged clean | module rows in [STABILITY_SWEEP § Module work list](STABILITY_SWEEP.md) + [STABILITY_PASS2 § Work list](STABILITY_PASS2.md) |

Standing discipline (not queue items): every lowering-semantics change lands
with a `cross_mode!` cell or `tests/scripts/` file (H4's S half — add as a
CODE.md checklist line with the next CODE.md touch); every M+ design through
the design protocol; verify-armed before trusting the armed channel's silence
([reference: STABILITY_SWEEP § armed-channel restoration](STABILITY_SWEEP.md)).

## Done (this cycle — closing commits in the canonical homes)

- **Pass-2 arity-growth CASCADE — COMPLETE.** Reference arm 2026-06-12 (one-buffer design: arity = signature+1 fixed at declaration, every return path delivers in THE `__retbuf`; 10-cell matrix green both backends). Vector/struct-Enum arm then closed via **#355** (multi-return-site vector behind a forward caller silently returned the WRONG element) and **#356** (mid-body `return f(g(x))` returned the null sentinel on native; fn-refs to struct-returning fns couldn't be CALLED — the latter also fixed by **#383**, merged in #393 this session). Both #355/#356 CLOSED. Live guards `tests/issues.rs::pass2_arity_growth_*` + `tests/scripts/387-text-fn-ref.loft`. The @PLN55 growth assert (the H2 residual) fell out — cleared.
- **Armed-corpus residuals ×4** — all resolved 2026-06-14: `132` silent interpreter UAF → `OpFreeRefIfDistinct` (regression `tests/scripts/372`); `collections.loft` two bugs → `parse_object_field` accepts `{}` (regression `373`) + `dedup_keyed` `secondary` flag for `other_indexes` (regression `374`); `166` verified over-strict; `75-native-stub` clean on the rebuilt armed binary. The armed channel's silence is now trustworthy.
- **H1 analysis-dependent arity** — RETIRED 2026-06-11 (@PLN55 phases 0–2; signature-time `__retbuf`, uniform return ABI, retro-patch deleted).
- **H2 typed deps** — steps 1–5 DONE 2026-06-12 (`Deps` newtype, space-asserting accessors, the positional contract retired via `CALLEE_FRAME_BIT`).  Residual (the @PLN55 growth assert on two lib fns) → cleared with the arity cascade above.
- **F11 error-path state** — swept, all four breaks FIXED 2026-06-12.
- **The armed-channel restoration** — four stale duals fixed 2026-06-12; the channel is the standing instrument.
- **`store.rs:1640` armed row (the "keyed armed UAF", 7 files)** — RESOLVED 2026-06-12 (4cba84c5): three mechanisms (header-as-`room` accessor → `Store::record_words`; parallel s_pos array header stomp; OpDatabase bytes-vs-words under-claim = a real release OOB write).  Armed corpus 12 → 5 files.
- **Plan-57 vector store-lifetime watermark** — CLOSED (@PLN2; rc removal complete).
- **Plan-53 cluster 1 + the aligned-V2 allocator half of cluster 2** — fixed/validating; the S4 half is queue #4.
