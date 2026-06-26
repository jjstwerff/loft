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
the store-lifetime UAFs *as of that pass*, and the **Pass-2 arity-growth cascade**
(Reference + Vector/struct-Enum, #355/#356) — is **complete** (see Done below) —
**EXCEPT the store-lifetime class, which REOPENED 2026-06-21: see
[§ Red-flag remediation](#red-flag-remediation--the-live-store-lifetime-stream-2026-06-21-) below.
It, not this H-register, is the live open stability work.** Nothing in the
H-register queue below is an active failure. What remains *there* is **forward-risk hardening** (the
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
> reached); **step 2** is parked WIP on a separate branch.  The live open queue is just
> **step 11** (CI docs-only matrix-skip — risky CI surgery, validate via a docs-only PR).
> **Step 8** (H4-medium) is now ✅ (design-protocol — free-op already single-homed, null-init
> are different facts; premise over-stated like H3) and **step 12** (i32 reclaim) is deferred
> (matrix-revised to M + low-value). The optional residuals
> (the F9 build-time macro, the full `allocations` field-privacy conversion) are
> stronger-enforcement / mechanical cleanups whose RISK is already covered — each
> opens with the design-protocol if/when its trigger fires.

| # | Item | Size | Status | Detail + entry point |
|---|---|---|---|---|
| 1 | **H5 leftover asserts — COMPLETE.** The two-pass contract asserted where it's cheap. **Attr-COUNT-per-def-equal-across-passes LANDED** (961e6c27, `assert_pass2_def_attr_stable`; post-arity-cascade an invariant, enforced end-to-end not just at the `ref_return` growth site; silent across the 270-script debug corpus). The **work-ref-counter half was resolved by re-evaluation, NOT a second assert** (the spec's item-3 "re-evaluate after H1" call): post-H1 `work_refs()` fires 0× in the corpus, a stored-table work-ref assert is permanently vacuous (`append` resets it to 0 at store time), and its only load-bearing failure (a cross-pass `__ref_N` shift → spurious `ref_return` attr) is already a count divergence the attr-count assert catches. Lambda naming is the sole remaining live name-stability consumer. | S | ✅ done (attr-count assert + work-ref re-eval) | [STABILITY_HOTSPOTS § H5](STABILITY_HOTSPOTS.md) |
| 2 | **Plan-53 cluster 2, S4 half (eval-TOS / frame-base alignment)** — parked WIP on branch `plan-53-sanitizer-ci-lever` (HEAD 8abfb8e1): cluster 1 fixed, aligned-V2 allocator half done and validating clean; the eval-TOS rounding half remains. Finishing it makes the sanitizer CI lever fully green = the second standing instrument beside the armed channel. | M | ⬜ parked WIP | plan closed by the @PLAN53 wrap-up; the session handoff is preserved at [plans/finished/53-sanitizer-ci-lever/cluster-2-fix-design.md § SESSION HANDOFF](plans/finished/53-sanitizer-ci-lever/cluster-2-fix-design.md) + `cluster-2-S4-progress.md` |
| 3 | **H6 — null-sentinel width-fact — LOAD-BEARING PART DONE (2026-06-17).** The matrix-first settle (the gate) overturned the design note: the `get_byte`/`set_byte` "asymmetry" was a misread — the nullable consumer pairs round-trip null symmetrically for every `min`, both backends. The REAL latent bug was on the **range-fullness** axis: a nullable FULL-range narrow field (`max-min == 255`/`65535`) under-allocated to 1 byte and read its null back as `max-1`, because the storage/WRITE width (`Type::size`) disagreed with the READ width (`byte_width`). Fixed at the chokepoint — one `IntegerSpec::range_to_width` home both derive from. Regression `tests/scripts/389-h6-nullable-full-range-narrow.loft`; full suite green. The `NullEnc` encode/decode TABLE is downgraded to OPTIONAL lower-risk cleanup (the per-width pairs already agree) → folds into step 10 Pass-3 de-dup, not a load-bearing fix. **NEW design follow-up (2026-06-17): the ALIAS path** — `u8`/`i8`/`u16`/`i16` are MEMORY-allocation types (fixed byte width is the invariant; nullability reserves a sentinel by SHRINKING the range, never widening). Current `IntegerSpec::u8()`=`0..=255` / `i8()`=`-128..=127` don't make their usable bounds `not_null`-aware, so a nullable `u8`'s `255` collides with the `ByteNullable` null sentinel. **ALIAS path DONE 2026-06-17 (`4a632251`), full suite green both backends.** `IntegerSpec::usable_min`/`usable_max` (one home) wired at the read op, write op, and `int_value_fits` (gained a `narrow_field` flag — a field STORE reserves the sentinel; a param/cast is full-width, so `f(65535)` to a `u16` param stays legal). Nullable is SYMMETRIC for signed (`-127..=127`, `-32767..=32767`), top-trimmed for unsigned (`0..=254`, `0..=65534`); the all-ones byte is the uniform sentinel, only `min` shifts. The SEPARATE 2-byte not-null-max bug was fixed in the same pass via `NarrowIntKind::ShortFull` / `OpGetShortFull` (direct read+min, no sentinel — the 2-byte twin of `Byte`). Field nullability now stamped onto the stored `IntegerSpec.not_null`; `lib/code.loft` `cur_arg` → `u8 not null`; `fill.rs` regenerated. Regression `tests/scripts/389-narrow-alias-ranges.loft`. Open separate/pre-existing follow-up: inline `Struct{..}.x` byte read fails on native (pre-eval gap, breaks plain `OpGetByte` too). | M | ✅ heuristic-path + alias path + 2-byte not-null all done | [STABILITY_HOTSPOTS § H6](STABILITY_HOTSPOTS.md); F3 row in [STABILITY_SWEEP](STABILITY_SWEEP.md) |
| 4 | **H7 short half — IR codec round-trip property test — LANDED (7187d5c6)**. `tests_scripts_round_trip` round-trips every `tests/scripts/` def's Type/Value/Attribute through the IR JSON codec (270 scripts seeded on the cached stdlib). It **earned its keep on day one**: caught `Value::Long(2^53+1)` decoding as `2^53` — the codec wrote i64 as a JSON number, which the parser stores as f64, silently truncating beyond 2^53. Fixed (i64 → quoted string; `as_i64` accepts both forms, legacy snapshots still decode). Now a standing tripwire: the next `Value`/`Type` variant with a silent codec gap, or any i64-precision regression, fails here loudly. | S | ✅ landed (found+fixed a Long codec bug) | [STABILITY_HOTSPOTS § H7](STABILITY_HOTSPOTS.md) |
| 5 | **H7 long half — derive the IR codecs from one schema declaration (F9)** — encoder, decoder, and the exhaustive walker all derive from one macro/table; a new variant then breaks the build until all three know it. Own design slot (codecs encode FIELDS, `for_each_child` can't drive them). **Runtime coverage of the STORE codec LANDED 2026-06-17** (`7b01e2a9`): the `corpus_store_codec_round_trips` guard widens the store-codec round-trip from one hand-written program to the whole corpus, and it caught a real reproducibility bug — `snapshot_names` iterated a `HashMap`, so the cached `Data` (variable-name list) was non-reproducible; fixed by a `(var_nr, name)` sort at that one chokepoint. **The silent-gap RISK is now CLOSED 2026-06-17** (`3e45d465`): a new `Value` variant breaks `write_into`'s exhaustive match (build error) AND fails the new `materialize_all_variants_round_trip` guard — all 34 variants through `materialize_node`→`read_value`, with a `len()==34` count guard forcing inclusion — so a variant or dropped field can no longer reach the cache silently. Both codecs now have all-34 exhaustive coverage (JSON `type_/value_*_round_trip`, STORE `materialize_all_variants` + the corpus guard). The build-time **schema MACRO** (deriving the arms from `ir.loft`, which already drives `ir_schema_gen`) is now a stronger-enforcement UPGRADE (build-time vs test-time), not an open risk. | M | ✅ silent-gap risk closed (exhaustive both-codec guards); F9 macro = optional upgrade | [STABILITY_HOTSPOTS § H7](STABILITY_HOTSPOTS.md); deferred row in [STABILITY_PASS2 § Work list](STABILITY_PASS2.md) |
| 6 | **H3 — ownership as carried data** — UNLOCKED (H1 + H2 both shipped). **Design-protocol (2026-06-17, `97b5d6f0` + pass 2) reframed this: NOT an open L carry-conversion.** The per-var ownership facts (`captured`, `caller_hidden_buf`, owned/borrowed via `Type::is_heap_owned` + `Deps`) are ALREADY carried, and the two core free-placement analyses (`reclaim_safe`, `store_confinement`) READ them — neither re-derives ownership inline. The "re-asserts what construction knew" premise is over-stated; what remains is the INHERENT shape-locality of free-placement (escape/retention/confinement-span), managed by the cross-check corpus, not removable by carrying. **Verification done: 4 analyses confirmed read-only** (`reclaim_safe`, `store_confinement`, `get_free_vars`, `store_lifetime_guard` — all READ `tp`/`Deps`/`is_captured`/`is_skip_free`/`is_argument`, none re-derive ownership inline). H3's core worry (analyses re-asserting carried state) is therefore **already addressed**. Residual is small + separate: the INHERENT shape-locality of free-placement (not removable) + possibly-scattered flag SETTERS (the #316/#323 "five homes" — a de-dup, not a carry-conversion). The sweep pass-2 notes (`value_reads_var`/`base_var_of`) fold into the de-dup tail. | L→S | ✅ resolved by design-protocol (premise over-stated; facts carried + read) | [STABILITY_HOTSPOTS § H3](STABILITY_HOTSPOTS.md) |
| 7 | **H8 — the `Stores.allocations` privacy pass** — the load-bearing target (per design-protocol) was the **worker-slot swap-back** — the par store-isolation "swap dance" (memory-safety), inline in `parallel.rs` with its no-cross-thread-aliasing invariant living only in a comment. **DONE 2026-06-17** (`69b6eb15`): moved to `Stores::grow_allocations_to` + `Stores::swap_in_worker_slots` — ONE named, documented home for the invariant (threading 47/47 green, behavior-preserving). The remaining ~498 raw `allocations[nr]` touches are **benign bounded reads carrying no invariant beyond bounds**; rewriting them all to make the field `private` would be over-broad (blast radius ≫ defect — design-protocol over-reach) and adds no invariant enforcement — so full field-privacy stays an OPTIONAL mechanical cleanup, gated on a future `par` API that genuinely needs the whole accessor surface. The STABILITY_PASS2 accessor rows (`types[].parts`, Definition reads) fold into that optional pass. | M–L | ✅ load-bearing swap encapsulated; full field-privacy = optional/deferred | [STABILITY_HOTSPOTS § H8](STABILITY_HOTSPOTS.md); deferred rows in [STABILITY_PASS2 § Work list](STABILITY_PASS2.md) |
| 8 | **H4 medium half — extend the `#rust`-template idea upward** — the free-op family, null-init emission, and the GET→SET table become one declaration each that both backends derive, the way fill.rs derives ops. Includes the F4 op-coverage sentinel (enumerate ops lacking `cross_mode` cells) as its completeness check. The L half (one shared lowering IR) stays 1.1+ and explicitly NOT before step 6 (H3). **GET↔SET table DONE 2026-06-17** (`NarrowIntKind::of` — one width→op home for `get_val`/`set_field_check`, `9153e132`); the other two halves RESOLVED by design-protocol (2026-06-17, "start Row 8"): the **free-op family** is already single-homed (`scopes.rs` selection + de-duped `pre_eval::free_op_var` recognizer — no per-backend table); the **null-init** pair are DIFFERENT facts (`emit_typed_null` live NULL sentinel vs `default_native_value` default-INIT placeholder — probed identical live-null round-trip on both backends; `floatvar=null` type-rejected; merging would be the H6-`NullEnc`-phantom), with the lone residual (`default_native_value`'s conflated contract) fixed by a clarifying doc comment. Premise over-stated, like H3. | M | ✅ done — GET↔SET shipped; free-op + null-init resolved by design-protocol | [STABILITY_HOTSPOTS § H4](STABILITY_HOTSPOTS.md); F4 row in [STABILITY_SWEEP](STABILITY_SWEEP.md) |
| 9 | **The instrument plans — fuzzing + sanitizer expansion** — open when reached (or when any item above needs the instrument earlier): @PLN53 program-level fuzzing, @PLN54 sanitizer coverage expansion, plus the sweep's "store-level fuzz harness" deferred instrument (store.rs LLRB/coalesce/claims cells). The remaining pass-1 DEFERRED cells (F1 diagnostics-altering-flow, F5 odd-size adjacency, F6 P191 late-mutation, F8 crafted attr/var collision, F9 lib-path axes, F10 par text buffers, match-unification, dispenser stress) fold into these corpora rather than being probed by hand. | L | ⬜ future plans | [plans/future/55](plans/53-program-level-fuzzing/README.md), [plans/future/56](plans/54-sanitizer-coverage-expansion/README.md); DEFERRED markers throughout [STABILITY_SWEEP](STABILITY_SWEEP.md) |
| 10 | **Pass-3 de-dup tail** — **TRIAGED CLEAN 2026-06-17.** Every named candidate is already resolved: `generation/ops` post-plan-57 rc remnants — **gone** (no rc code left); `value_reads_var` — already centralised (`data.rs::reads_var`, replaced `scopes::value_reads_var` + two more); `base_var_of` — already unified (`data.rs::base_var`); the variables size-table — DECIDED a non-dup vs `byte_width` (PASS2 wave 5, different facts); `towards_set` dual discriminators + the codegen_runtime mirrors — INTENTIONAL (interp-vs-native, #328). The H3 flag-setter "five homes" (#316/#323) is the one residual de-dup, opportunistic. Nothing actionable stands open. | S each | ✅ triaged clean | module rows in [STABILITY_SWEEP § Module work list](STABILITY_SWEEP.md) + [STABILITY_PASS2 § Work list](STABILITY_PASS2.md) |
| 11 | **CI — docs-only PRs block on the skipped Test matrix** (surfaced 2026-06-17, PR #400). A pure docs-only diff skips the Test matrix at the JOB level (`if: needs.changes.outputs.code == 'true'`), so the matrix never EXPANDS: only the unexpanded `Test (${{ matrix.os }})` reports (SKIPPED) and the required `Test (ubuntu/macos/windows)` names never appear → branch protection stays `BLOCKED`. The workflow's "a skipped required check counts as satisfied" comment holds only when the matrix expands (skip at STEP level). #400 dodged it (its `tests/fixtures/` change classifies as code). Fix: move the `if` from the matrix job onto its heavy STEPS (each leg still reports its required name), or a companion job that posts the three names green on a docs-only diff. | S | ⬜ open | `.github/workflows/ci.yml` (`changes` + `Test` jobs) |
| 12 | **H6 follow-up — 4-byte `i32` reclaims `i32::MIN` (not-null full range).** A not-null `i32` cannot hold `i32::MIN`: `OpGetInt4` decodes a stored `i32::MIN → i64::MIN` (null) — the sentinel-decode the 1-byte `Byte` and 2-byte `ShortFull` reads avoid. Fix = the 4-byte twin: a no-sentinel read (`get_i32_raw` direct → `i64::from`), a `NarrowIntKind` arm splitting not-null vs nullable at width 4, `reserves_narrow_sentinel` + `usable_min/max` coverage for `Some(4)`; the compile + runtime sentinel communication then falls out of the shared `usable_min/max` home. LOW value — reclaims one extreme value. **Matrix-first (2026-06-17) revised this to M, not S, and KEPT IT DEFERRED:** the `i32::MIN` *literal* is itself narrowing-rejected — `x: i32 = -2147483648` → "cannot implicitly narrow integer to i32", because the unary-minus isn't const-folded before `int_value_fits`, which sees the positive `2147483648` (> `i32::MAX`). So a real fix needs negative-literal const-folding PLUS the not-null no-sentinel read PLUS the width-4 not_null distinction — M effort for a value (`i32::MIN`) that is virtually never real data. **`i64`/`integer` likewise DEFERRED**: its null IS the universal stack/register sentinel (`i64::MIN`, 60+ sites in `ops.rs` alone), so reclaiming it is a null-model rearchitecture. | M (was S) | ⏸ deferred — low value | [STABILITY_HOTSPOTS § H6](STABILITY_HOTSPOTS.md) |

Standing discipline (not queue items): every lowering-semantics change lands
with a `cross_mode!` cell or `tests/scripts/` file (H4's S half — add as a
CODE.md checklist line with the next CODE.md touch); every M+ design through
the design protocol; verify-armed before trusting the armed channel's silence
([reference: STABILITY_SWEEP § armed-channel restoration](STABILITY_SWEEP.md)).

## Red-flag remediation — the live store-lifetime stream (2026-06-21 →)

> **The H-register above is the forward-risk hardening (genuinely cleared
> 2026-06-17). This stream is different and LIVE:** a later cross-cut
> ([STABILITY_REDFLAGS.md](STABILITY_REDFLAGS.md), 2026-06-21) plus the live tracker
> show the **store-lifetime / return-bind-ownership class is still spawning bugs** —
> the Cluster A residuals (#426, #429) are both now CLOSED, but the open native issues
> (#460, #461) are the mixed interpret→native shared-store boundary — store-lifetime is
> **mid-migration (@PLN85), not complete.**
> Finishing order:

| # | Item | Size | Status |
|---|---|---|---|
| **A** | **Cluster A — return/bind ownership** (collapse the per-site ownership re-derivation onto one carried `deps` fact). A.4 / A.3 / A.2-a7 + the native-FFI fixes merged (#423); A.1 part i (free-suppress, return-source SET) + the parser-counter substrate / #426B / #425-sibling / native-leak fixes on `tuxedo-substrate-followup`. **Residuals #426 + #429 both CLOSED** (2026-06-22). #429 (borrowed-view return over-free) landed the borrow-classify in `ref_return` + the nullable-enum copy-bind path in `gen_set_first_at_tos`; regression `tests/scripts/85-store-lifetime-enum-match-borrowed-view-overfree.loft` passes both backends. Cluster A's tracked bugs are done; the remaining ownership-substrate work is Cluster C. | — | ✅ done (residuals closed) |
| **C** | **Cluster C — per-`Parts` container taxonomy** (copy/remove/validate/construct heap-cascade). `remove_claims` collapsed onto `for_each_owned_child` (C.0–C.3, merged), but **copy / validate / construct still drift** — `copy_claims` split 4 ways, `validate_claims` monolithic, ~53 `Parts::` arms across 3 dispatchers. The densest HISTORICAL bug cluster (@P290 SIGSEGV, @P306/@P318 hash slot-drift, @P309, #260/#330) and the highest-leverage **UN-TRACKED** hotspot. Fix: fold copy/validate/construct onto the keystone. Now H10. | M–L | ⬜ open — the next pass after A |
| **@PLN87** | **Reference-default `&`-binding semantics** — `&`-to-reassign + the W4 redundant-`&` lint; the OWNERSHIP_MODEL binding rule realized. | M | ⬜ planned ([@PLN87](https://github.com/loft-lang/plans/issues/87), loft2 implements) |
| **B** | **Cluster B — stack-delta wrong-signal.** Deferred — unverifiable, no RED probe fires; latent. Pick up only on a real trigger. | — | ⏸ deferred |
| **462-leak** | **#462 residual store leak** — native-only `MonsterDef×216` **record** leak, the `mon_*` borrowed-view shape (a Cluster-A/C-family borrow over-free, NOT the adopt-and-re-return chain, which is fixed). Reproduces `--native`: `probes/over-free-sweep/P3-monone-cond` `M×36`, `M-462repro` `M×90`; interp clean. [cluster-462 item 5](plans/85-store-lifetime-retirement/cluster-462-slot-reuse-uaf.md). | S–M | ⬜ open |
| **N** | **Native mixed-mode boundary** — #460 (`--interpret` aborts when a main-program fn is marked for cdylib dispatch with no cdylib built) + #461 (interpret→native shared-store call corrupts a complex nested struct arg). Both sev:high, hit-by:crawler/moros. Could not be cleanly reproduced on 2026-06-26 (cdylibs present masked #460; #461 guard scene too small) — verdict needs a clean-room native build. | M | ⬜ open |

D (typed-null encoders) merged; E (manifestation guards) dissolves behind A. Full
detail: [STABILITY_REDFLAG_REMEDIATION.md](STABILITY_REDFLAG_REMEDIATION.md),
[STABILITY_REDFLAGS.md](STABILITY_REDFLAGS.md), @PLN85.

## Done (this cycle — closing commits in the canonical homes)

- **#462 adopt-and-re-return vector leak — FIXED** 2026-06-26 (commit `cafe98a0`). A vector-returning fn that adopts a call result and re-returns it (`t = base(); t`), and the `t = base(); t += …; t` merge shape (`game_items()`/`game_monsters()`), leaked one store per call: the NRVO collapse redirected the inner call's hidden `__ref_N` buffer onto the retbuf but left its eager allocation orphaned. Fix in `parser/control.rs` (`nrvo_collapse_tail_set` + new `nrvo_collapse_defining_call`); vector-only. Crawler interp 531→0, native 752→216 (the 216 is the 462-leak row above). Regression `tests/leak_cases/clean/p462_adopt_rereturn_vector.loft`, both backends. #462 stays open for the record-leak residual.
- **Cluster A residuals #426 + #429 — both CLOSED** 2026-06-22 (see the A row above).

- **Pass-2 arity-growth CASCADE — COMPLETE.** Reference arm 2026-06-12 (one-buffer design: arity = signature+1 fixed at declaration, every return path delivers in THE `__retbuf`; 10-cell matrix green both backends). Vector/struct-Enum arm then closed via **#355** (multi-return-site vector behind a forward caller silently returned the WRONG element) and **#356** (mid-body `return f(g(x))` returned the null sentinel on native; fn-refs to struct-returning fns couldn't be CALLED — the latter also fixed by **#383**, merged in #393 this session). Both #355/#356 CLOSED. Live guards `tests/issues.rs::pass2_arity_growth_*` + `tests/scripts/387-text-fn-ref.loft`. The @PLN55 growth assert (the H2 residual) fell out — cleared.
- **Armed-corpus residuals ×4** — all resolved 2026-06-14: `132` silent interpreter UAF → `OpFreeRefIfDistinct` (regression `tests/scripts/372`); `collections.loft` two bugs → `parse_object_field` accepts `{}` (regression `373`) + `dedup_keyed` `secondary` flag for `other_indexes` (regression `374`); `166` verified over-strict; `75-native-stub` clean on the rebuilt armed binary. The armed channel's silence is now trustworthy.
- **H1 analysis-dependent arity** — RETIRED 2026-06-11 (@PLN55 phases 0–2; signature-time `__retbuf`, uniform return ABI, retro-patch deleted).
- **H2 typed deps** — steps 1–5 DONE 2026-06-12 (`Deps` newtype, space-asserting accessors, the positional contract retired via `CALLEE_FRAME_BIT`).  Residual (the @PLN55 growth assert on two lib fns) → cleared with the arity cascade above.
- **F11 error-path state** — swept, all four breaks FIXED 2026-06-12.
- **The armed-channel restoration** — four stale duals fixed 2026-06-12; the channel is the standing instrument.
- **`store.rs:1640` armed row (the "keyed armed UAF", 7 files)** — RESOLVED 2026-06-12 (4cba84c5): three mechanisms (header-as-`room` accessor → `Store::record_words`; parallel s_pos array header stomp; OpDatabase bytes-vs-words under-claim = a real release OOB write).  Armed corpus 12 → 5 files.
- **Plan-57 vector store-lifetime watermark** — CLOSED (@PLN2; rc removal complete).
- **Plan-53 cluster 1 + the aligned-V2 allocator half of cluster 2** — fixed/validating; the S4 half is queue #4.
