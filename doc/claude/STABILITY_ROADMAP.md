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

## The wide-release bar — what must be true before loft goes to many people

> This roadmap exists to clear **one** thing: the GOALS.md promise of **"a floor that
> does not betray you"** ([GOALS.md § The deeper aim](GOALS.md)). The H-register and the
> live store-lifetime stream below both serve it. This section is the explicit gate —
> the bar loft must clear before it is handed to a lot of people (and to AI writing
> loft). In priority order; **gate 1 is the definition of "stabilized," not one item
> among five.**

1. **Seal the memory model — the non-negotiable gate.** The store-lifetime /
   return-bind-ownership class (loft's stated #1 weakness, REOPENED 2026-06-21) must be
   **closed, not merely quiet**. At one dogfooding agent a residual UAF/over-free
   surfaces occasionally; at many users + AI hitting every composition it surfaces
   constantly — and a substrate that *sometimes* corrupts invalidates the whole pitch
   (the language carries correctness so the maker never does — DESIGN_DECISIONS C79/C80).
   "Closed" = pin the ONE ownership invariant at the `deps` chokepoint (Cluster C / H10,
   not symptom-by-symptom), THEN prove the class gone *by construction* — graduate the
   boundary matrices into the fuzz/sanitizer corpora (@PLN53/@PLN54, queue step 9) so the
   silence is earned, not anecdotal. Live work:
   [§ Red-flag remediation](#red-flag-remediation--the-live-store-lifetime-stream-2026-06-21-)
   + @PLN85. **This gate's fuzz-proof IS the definition of stabilized.**
   **Instrument status (2026-07):** the fuzz-proof instrument is BUILT and live —
   the standing `tests/ownership_fuzz_gate.rs` job, the in-process libfuzzer target
   (caught + closed 2 real bugs in its first five minutes), `LOFT_POISON=1 cargo test`
   fully green (24 latent memory bugs fixed across the poison campaign), and the
   debug-assertions calibration run
   ([DEBUG.md](DEBUG.md#the-debug-assertions-calibration-run-target-da)).  The residue
   is enumerated, not anecdotal: the open DA cells + unfuzzed axes in
   [plans/85 fuzz-proof-gate.md](plans/85-store-lifetime-retirement/fuzz-proof-gate.md).
   **Build-order dependency — RESOLVED.** Gate 1 was blocked by gate 2: the ownership invariant
   could not be *defined* until the value/null model settled, because ownership flows through the
   `deps` facts and what a vector/value *is* (dense vs nullable, how it copies vs borrows) is
   exactly what @PLN25 decided. @PLN25 CLOSED 2026-07-02, so the foundation is in place.
   (This is why earlier @PLN85 attempts flailed — there wasn't enough of @PLN25 settled to know
   what to build.)
   **Gate 1 — the last item landed 2026-07-10.** Every *tracked* store-lifetime bug is closed; the
   **fuzz-proof half** is done (@PLN53 harness #542, @PLN54 sanitizer stack + S4 LSan, both CLOSED via
   #547, only S9's toolchain-blocked cdylib ASan spun out); and the **Cluster C / H10** `copy_claims`
   keystone fold — the last structural item — is **✅ done** (branch `tuxedo-cluster-c`, see the C row
   below), so the three divergent source re-encodings that produced the densest historical bug cluster
   are gone by construction. What remains before the gate is *sealed* is only the standing verification:
   the corpora built by @PLN53/@PLN54 keep running over the folded code, and the branch merges to `main`
   green. The invariant is enforced; this is now "keep it enforced," not "define it."

2. **One coherent null model — and the substrate gate 1 is built on. MODEL LANDED 2026-07-02 (#480);
   the gate is NOT yet cleared.** @PLN25 (nullable sequences / dense-default) is closed as a *plan*:
   vectors, scalars and DN1–DN6 all landed default-on across both backends, `formal/types.md` is at
   0 open deviations, and `not null` is now a **deprecated no-op** — it still parses, with a warning,
   so not-yet-republished libraries keep loading, and the hard "retired" error stays blocked on the
   registry republish (#546). **Load-bearing half realised:** the value shapes and `deps` ownership
   facts the memory-model fix reads are settled, which is what makes **gate 1's invariant
   *knowable*** — so gate 1 is unblocked, and that was gate 2's job for it.
   **What keeps the gate open** is close-out plus a set of **verified-open edge cases** — among them
   a **`?? null` unsoundness** (`y: integer = x ?? null` is accepted and a non-null slot ends up
   holding `null`, both backends), the **call-arg N-Store hole** (a nullable passed into a non-null
   *parameter* is silently accepted, though into a non-null *local* it is correctly rejected), a
   **`u8?`-return native codegen failure**, the registry-gated `not null` hard-reject, and the F6
   bookkeeping close-out. A null reaching a non-null slot is a soundness edge, not cosmetics: it is
   the user-facing incoherence this gate exists to remove, so the gate reads **open**.
   *Provenance:* re-probed on both backends 2026-07-10 by the @PLN25 stream — the authoritative list
   is [RESUME.md § VERIFIED-OPEN RESIDUALS](plans/25-nullable-sequences/RESUME.md#verified-open-residuals-re-probed-both-backends-2026-07-10).
   These are **not** independently re-verified here.

3. **First-contact developer experience. ✅ CLEARED 2026-07-07.** GOALS' acceptance test is
   *"done = picking it up is fun,"* and first contact is dominated by what happens when the user
   is **wrong**. Error messages (@PLN28) and developer experience (@PLN36) are **both CLOSED**
   (`status:finished`): `file:line:col` + caret across parser/type/runtime, did-you-mean
   suggestions, concrete type-mismatch + match-pattern checks. Residual is two non-blocking
   polish slices (finer format-null tokens, the `= note:` renderer) — not a gate.

4. **Durability.** The "trust and forget your data" half — opt-in mmap so a crash or edit
   never loses the store (@PLN43). Skippable for throwaway prototypes, load-bearing for
   real projects.

5. **A stability contract for scale. ▶ TRIGGER FIRED 2026-07-10 — plan OPENED as
   [@PLN102](https://github.com/loft-lang/plans/issues/102)** (`status:next`,
   [plans/102-stability-contract/](plans/102-stability-contract/README.md)). A stated
   semver / compatibility promise, a **public** bug-intake path (the fix-not-file discipline is
   internal-only and doesn't reach strangers), and a 1.0 line — what is frozen vs still moving.
   The opening condition was "open one when gate 1 is in sight"; gate 1 is now one work item away.
   **Verified mechanism gap:** `manifest::check_version` honours only a `>=` lower bound — an upper
   bound, exact pin, caret, or malformed constraint is silently accepted as "any version" — and
   under calendar versioning (`2026.7.1`) the `>=0.8` that every published library carries is
   permanently vacuous. So a library *cannot* declare incompatibility even if its author knows.
   **The failure mode it prevents is already live.** `hex_terrain 0.1.0` fails its own registry
   test with `0 land cells`: it uses the plain-bind write-through idiom (`th = t.tr_h; th[i] = v`),
   and loft now **copies on plain bind** (C86 H-Copy), so the heights land in throwaway copies.
   `graphics` hit the identical class and was migrated to `&self.data`; `hex_terrain` never was.
   Both pin `loft = ">=0.8"`, so nothing guarded them, and the library does not crash — it
   computes a plausible-looking wrong answer. That is precisely what
   [GOALS.md](GOALS.md) forbids of the platform: *"the platform never broke its users; the cost of
   change was paid by the maker, not the customer."* A compat promise with a deprecation channel
   is the mechanism that would have caught it.

**Sequence — gate 3 is CLEARED; gate 2 delivered what gate 1 needed but is not itself closed; gate 1
is the live one.** Gate 2 (@PLN25) settled the value model and the `deps` ownership facts that gate 1's
invariant is defined against, exactly as the build order required — so gate 1 is **unblocked** even
though gate 2 still carries verified-open soundness edges of its own (see gate 2 above; they do not
block gate 1, because the *model* is what gate 1 reads). Gate 3 (@PLN28 + @PLN36) is closed. So the
order now reads: **finish gate 1** (the Cluster C fold — the fuzz/sanitizer corpora it must run under
are now standing, @PLN53/@PLN54 closed) and **drain gate 2's edge cases** in parallel, **then open
gate 5** (opened, @PLN102), with gate 4 (@PLN43, parked) after. Performance (the copy-vs-borrow
elision, an @PLN25 sub-thread) is "good enough for prototyping" — fold in opportunistically, not a
blocker.

**Readiness today (2026-07-10).** The 2026-07 stability + type-safety release SHIPPED as
`2026.7.1`. Gate 3 is CLOSED; gate 2's *model* is landed but the gate is **open** on verified soundness
edges (a `?? null` unsoundness, the call-arg N-Store hole — see gate 2 above). Gate 1's tracked
store-lifetime bugs are all CLOSED (#460 / #461 / #462 / #465, and A1b via @PLN90 #516) **and its
fuzz-proof is now standing** (@PLN53/@PLN54 closed, #547); the single remaining item is the Cluster C
fold — forward-risk hardening, not an active bug.

**Why the tracker is empty — and what to read instead.** This stream's standing rule at the top of
this file is *fix, don't file*, and the cycle runs under a warm feature freeze
([ROADMAP § Feature freeze](ROADMAP.md)): **a known defect cannot be parked** — it is fixed in the
session that surfaces it, with a regression test, and new feature work stops until what we can see
works. So "zero open bug issues" is not bookkeeping; it is the *consequence* of refusing to tolerate a
defect, and it is why nothing accumulates. What the number is **not** is the ledger. The known
remainder is **recorded, scoped and owned** in each open plan's residual list — plus this queue — and
it is not all comfortable: gate 2's `?? null` unsoundness above is a real soundness edge, named rather
than parked. **Read those lists, not the issue count.**

The discipline earns its keep because *the person who finds a bug is the person who fixes it*: repro
warm, paths loaded, no scope/mechanism re-derivation to re-pay later. It does not survive contact with
anyone who **cannot** fix — filing is a stranger's only available move — which is why a public intake
path is its own arc of gate 5 ([@PLN102](https://github.com/loft-lang/plans/issues/102)) rather than an
afterthought. The policy is right; its boundary is scale, not size.

Two standing gates were RED; **both are now resolved except one external library**:
- **`main` on the differential oracle — ✅ GREEN (2026-07-10).**
  `tests/oracle/27-native-tailcall-return-heap.loft` was the `a7_match_arm_tail` divergence
  (a `-> text` fn whose tail `match` arm calls a caller-buffer callee → rustc E0599); it was
  fixed in `b1426f9e` (#548) on `main` (`if_tail_yields_text` now sees through the `scalar_match`
  block) and pinned by `tests/scripts/536-text-match-tail-buffer-callee.loft`.
  `oracle_corpus_agrees_across_backends` passes both backends. The corpus cell added by @PLN97 did
  its job.
- **`registry-validation` — graphics leg FIXED (2026-07-10); one library still red.** `graphics`
  failed at native-crate build (`alsa-sys` needs `libasound2-dev`; the workflow installed only
  `mold`) — a provisioning gap, now closed by mirroring the main CI Test job's Linux install
  (`libasound2-dev xvfb libgl1-mesa-dri`) into `registry-validation.yml`. The remaining red leg is
  **`hex_terrain 0.1.0`**, a real published-library bug (the C86 plain-bind write-through idiom
  lands its heights in throwaway copies — see gate 5): it needs a **library republish in
  loft-libs-game**, out of this repo's scope, and is the motivating case for the @PLN102 compat
  promise. Not a network flake — the other ~20 pass.

**Coverage gaps against the GOALS.md Checks — both CLOSED 2026-07-10** (the Checks are the bar and
stay as written; these were *results*):
- Goal A (`stack_align_guard` fires zero across every test binary): **✅ widened.** The guard fires
  only IN-PROCESS — a `cross_mode!` matrix cell / mixed-boundary suite shells out to a spawned
  `--native`/`--wasm` binary the sweep can't observe (`tests/n3_parity.rs` states this), so the
  reachable corpus IS the in-process interpreter binaries. The `guard` sweep now runs all of them:
  `issues/wrap/strings/frame_vars` **plus** `expressions`, `expressions_auto_convert`, `slots`,
  `slot_v2_baseline`, `value_struct_alloc`, `dispatch_reentry`, `format` (each verified zero-fires
  under the feature). `library_suite` stays excluded (native cdylibs + GL/ALSA the lean job omits;
  guard-blind anyway).
- Goal E (`LOFT_STORE_GUARD=1` silent across the corpus, promoted to a `cfg(debug_assertions)`
  assertion): **✅ wired + widened.** The enforced twin — the `reclaim_guard`
  `reclaim_unfreed_eligible == 0` `assert_eq!` — now hard-gates across the interpreter corpus
  because the **nightly debug-assertions gate was widened** from `--lib --test issues` to
  `--test wrap --test strings --test frame_vars` (`library_suite` excluded), the plan-85
  DA-inventory chain having been cleared (below). `LOFT_STORE_GUARD=1` is now set on that gate too
  (closing "set in no workflow"), additionally running the block-confinement `store_lifetime_guard`
  detector; both verified silent corpus-wide, positive-controlled by
  `watermark.rs::phase4_goal_e_guard_is_falsifiable`.

### The `wrap` loft_suite DA-gate residuals — the widen-the-gate worklist (✅ CLEARED 2026-07-10)

**DONE — the nightly DA gate now spans `--lib --test issues --test wrap --test strings
--test frame_vars` (`library_suite` excluded), matching the per-PR `stack_align_guard`
sweep scope.** Widening was blocked by a **chain of debug-assert tripwires**: under
`RUSTFLAGS='-C debug-assertions=on' … cargo test --release --test wrap`, the
`loft_suite` test (one test that runs every `tests/scripts/*.loft`) aborts at the
FIRST script that trips an assert — so each fix unmasked the next.  Cleared on
`tuxedo-cluster-c` (UNMERGED).  **Most were FALSE ALARMS** — over-eager
sentinels firing on correct-but-flagged cases (the H2 sentinel's OWN advice, "re-add
the read", would have *leaked*; the relocate one tempted a wide-blast-radius "complete
the traversal" that wasn't needed); the read-surface one (86) was the same shape.  Two
were real latent bugs.  Lesson: before obeying a sentinel's "this shouldn't happen"
premise, verify the flagged behaviour (value + leak + `LOFT_POISON` + the DA store-free
asserts, BOTH backends) — a debug tripwire is a hypothesis, not a verdict.

Cleared (each verified: the fixed case correct on both backends; a non-vacuous
`tests/issues.rs` guard):

| Script(s) | Assert | Commit | Was it real? |
|---|---|---|---|
| `156-plan52-chained-coalesce` | `text.rs:334` double free | `afacd148` | **REAL** — a chained `??` double-freed an owned `__ncc` coalesce temp (interp; `collect_consumed_ncc_text` double-collected a nested-`??` temp). |
| `387-text-fn-ref`, `85-ncc-container-text-return`, `85-poison-return-tail-uaf` | `parser/mod.rs:1195` (H5 two-pass contract) | `cd9c1f94` | **REAL, latent** — a pass-2-only `__tret` hidden `&text` signature buffer → forward-ref caller "Too few parameters" crash (BOTH backends, release, not just DA). Gate pass-2 tret promotion on pass-1. |
| `450-struct-field-vector-return`, `508-empty-arm-real-empty-vector`, `repro_p365`, 4× `85-store-lifetime-*` | `scopes.rs` (H2 step-5 `tp_alone` sentinel) | `e1d594cb` | **FALSE ALARM** — retired positional block-result read; a field/enum-arm vector return copies its source into the retbuf, so freeing the local source is correct (re-adding the read would leak). Sentinel removed. |
| `501-map-filter-literal-receiver`, `85-short-lambda-capture` | `scopes.rs` (`relocate_null_init`) | `097879bb` | **FALSE ALARM** — the best-effort Plan-57 null-init relocation can't reach a confined block off the control-flow spine (a `map`/`filter`/lambda body); the body-0 fallback is correct. Assert softened to fire only on genuine scope-absence. |
| `86-writeread-struct` | `scopes.rs` (`check_ref_leaks`) | `d5b6212a` | **FALSE ALARM** — `_read_1`, the `#reading file` surface temp behind `q = f#read as S`, is a MOVE source: the block allocs one record and PutRef-adopts it into `q`, which IS freed. `get_free_vars` already elides the temp's free; `check_ref_leaks` didn't model the adoption. Both backends clean (values + empty-allowlist leak gate + POISON). Fixed by crediting the adopted block-tail temp (`collect_adopted_block_results`) — narrow: a plain-bind COPY has a bare-`Var` RHS and its source is freed separately, and the credit requires `lhs ∈ freed`, so it can't mask a real leak. |

Remaining — **NONE for the interpreter gate.** The two that were listed are outside the
gate's scope by construction (the gate covers the in-process interpreter corpus,
`library_suite`/native excluded — exactly like the alignment sweep):

- **`75-native-stub`** — INFRA (needs a rebuilt native cdylib); native-only, so it is
  never hit by the interpreter DA gate (which runs `--interpret`).  `find_problems.sh`
  (runs `make rebuild-native-cdylibs`) passes it.  Only a hypothetical *native* DA sweep
  would need this; not built.
- **`audience_crystal/03-crystal-incr`** — `mod.rs:4014` op-count watchdog.  Lives in
  `library_suite` (excluded).  NOT a runaway: under normal CI it runs NATIVE (cdylib
  built) with no op watchdog; a DA `library_suite` attempt forces interpreter fallback
  (the cdylib isn't rebuilt against the DA `libloft`), and the compute-heavy crystal-incr
  legitimately exceeds the *interpreter's* op limit — an artifact of interpreter fallback,
  not a bug.  Out of scope unless `library_suite` is ever run under DA.

Order of the chain (each masked the next): `156 → H5 → 3787(H2) → 280(relocate) → 86`.
All cleared — the `wrap` (+ `strings`/`frame_vars`) suites joined the nightly DA gate in
`d5b6212a`. `75` (native infra) and `audience_crystal` (library_suite op-limit) sit in the
excluded native/library domain, out of the interpreter gate by construction (above).

## The queue

The bug-level stability work — the F-family sweep, the armed-corpus residuals,
the store-lifetime UAFs *as of that pass*, and the **Pass-2 arity-growth cascade**
(Reference + Vector/struct-Enum, #355/#356) — is **complete** (see Done below). The
store-lifetime class REOPENED 2026-06-21, but as of 2026-07-09 **all its tracked bugs are CLOSED**
(see [§ Red-flag remediation](#red-flag-remediation--the-live-store-lifetime-stream-2026-06-21-)
below); the one remaining item there is the **Cluster C** `copy_claims` keystone fold — forward-risk
hardening, not an active failure. Nothing in the
H-register queue below is an active failure either. What remains *there* is **forward-risk hardening** (the
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
> **step 9** is the fuzzing/sanitizer instrument pair (@PLN53/@PLN54) — **✅ BOTH CLOSED 2026-07-10**
> (#547); it was gate 1's fuzz-proof and it now stands (only S9's toolchain-blocked cdylib-boundary
> ASan spun out); **step 2** is parked WIP on a separate branch.  The live open queue is
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
| 9 | **The instrument plans — fuzzing + sanitizer expansion** — **✅ BOTH CLOSED 2026-07-10.** @PLN53 program-level fuzzing (harness shipped #542; continuous-run/OSS-Fuzz decision-deferred) + @PLN54 sanitizer coverage expansion (S1/S2/S3/S5/S6/S7 green; S4 LSan `detect_leaks=1` unblocked by @PLN85 + green; S8 MSan deferred; only S9 mixed-boundary cdylib ASan spun out, toolchain-blocked on curve25519 `E0463`). The sweep's "store-level fuzz harness" instrument + the remaining pass-1 DEFERRED cells (F1 diagnostics-altering-flow, F5 odd-size adjacency, F6 P191 late-mutation, F8 crafted attr/var collision, F9 lib-path axes, F10 par text buffers, match-unification, dispenser stress) fold into the shipped fuzz corpora (or re-open with S9) rather than being probed by hand. | L | ✅ both plans closed | [plans/53](plans/53-program-level-fuzzing/README.md), [plans/54](plans/54-sanitizer-coverage-expansion/README.md); DEFERRED markers throughout [STABILITY_SWEEP](STABILITY_SWEEP.md) |
| 10 | **Pass-3 de-dup tail** — **TRIAGED CLEAN 2026-06-17.** Every named candidate is already resolved: `generation/ops` post-plan-57 rc remnants — **gone** (no rc code left); `value_reads_var` — already centralised (`data.rs::reads_var`, replaced `scopes::value_reads_var` + two more); `base_var_of` — already unified (`data.rs::base_var`); the variables size-table — DECIDED a non-dup vs `byte_width` (PASS2 wave 5, different facts); `towards_set` dual discriminators + the codegen_runtime mirrors — INTENTIONAL (interp-vs-native, #328). The H3 flag-setter "five homes" (#316/#323) is the one residual de-dup, opportunistic. Nothing actionable stands open. | S each | ✅ triaged clean | module rows in [STABILITY_SWEEP § Module work list](STABILITY_SWEEP.md) + [STABILITY_PASS2 § Work list](STABILITY_PASS2.md) |
| 11 | **CI — docs-only PRs block on the skipped Test matrix** (surfaced 2026-06-17, PR #400). A pure docs-only diff skips the Test matrix at the JOB level, so the required `Test (<os>)` contexts never appeared → branch protection stayed `BLOCKED`. **✅ DONE** — the companion-job fix (a `test-skip` matrix job, `if: needs.changes.outputs.code == 'false'`, that posts `Test (ubuntu-latest)`/`Test (macos-latest)` green on a docs-only diff) was landed in `b79c3798` (PR #400 itself) and is on `main`; the row was just never flipped. Verified present on `origin/main`. | S | ✅ done (`b79c3798`) | `.github/workflows/ci.yml` (`test-skip` job) |
| 12 | **H6 follow-up — 4-byte `i32` reclaims `i32::MIN` (not-null full range).** A not-null `i32` cannot hold `i32::MIN`: `OpGetInt4` decodes a stored `i32::MIN → i64::MIN` (null) — the sentinel-decode the 1-byte `Byte` and 2-byte `ShortFull` reads avoid. Fix = the 4-byte twin: a no-sentinel read (`get_i32_raw` direct → `i64::from`), a `NarrowIntKind` arm splitting not-null vs nullable at width 4, `reserves_narrow_sentinel` + `usable_min/max` coverage for `Some(4)`; the compile + runtime sentinel communication then falls out of the shared `usable_min/max` home. LOW value — reclaims one extreme value. **Matrix-first (2026-06-17) revised this to M, not S, and KEPT IT DEFERRED:** the `i32::MIN` *literal* is itself narrowing-rejected — `x: i32 = -2147483648` → "cannot implicitly narrow integer to i32", because the unary-minus isn't const-folded before `int_value_fits`, which sees the positive `2147483648` (> `i32::MAX`). So a real fix needs negative-literal const-folding PLUS the not-null no-sentinel read PLUS the width-4 not_null distinction — M effort for a value (`i32::MIN`) that is virtually never real data. **`i64`/`integer` likewise DEFERRED**: its null IS the universal stack/register sentinel (`i64::MIN`, 60+ sites in `ops.rs` alone), so reclaiming it is a null-model rearchitecture. | M (was S) | ⏸ deferred — low value | [STABILITY_HOTSPOTS § H6](STABILITY_HOTSPOTS.md) |

Standing discipline (not queue items): every lowering-semantics change lands
with a `cross_mode!` cell or `tests/scripts/` file (H4's S half — add as a
CODE.md checklist line with the next CODE.md touch); every M+ design through
the design protocol; verify-armed before trusting the armed channel's silence
([reference: STABILITY_SWEEP § armed-channel restoration](STABILITY_SWEEP.md)).

## Red-flag remediation — the live store-lifetime stream (2026-06-21 →)

> **The H-register above is the forward-risk hardening (cleared 2026-06-17). This stream
> tracked the store-lifetime / return-bind-ownership class that reopened 2026-06-21**
> ([STABILITY_REDFLAGS.md](STABILITY_REDFLAGS.md)). **As of 2026-07-09 every tracked bug in it
> is CLOSED** — the Cluster A residuals (#426, #429), the #462 leak, the native mixed-mode
> boundary (#460, #461), and the A1b temporary-subject UAF (@PLN90 #516). What remains is **one
> untracked refactor, not an active bug**: Cluster C, folding the **`copy_claims` source
> enumeration** onto the keystone (`validate_claims` and construction were probed and ruled OUT
> of scope — see the C row) — this retires the densest historical bug cluster *by construction*.
> Finishing order:

| # | Item | Size | Status |
|---|---|---|---|
| **A** | **Cluster A — return/bind ownership** (collapse the per-site ownership re-derivation onto one carried `deps` fact). A.4 / A.3 / A.2-a7 + the native-FFI fixes merged (#423); A.1 part i (free-suppress, return-source SET) + the parser-counter substrate / #426B / #425-sibling / native-leak fixes on `tuxedo-substrate-followup`. **Residuals #426 + #429 both CLOSED** (2026-06-22). #429 (borrowed-view return over-free) landed the borrow-classify in `ref_return` + the nullable-enum copy-bind path in `gen_set_first_at_tos`; regression `tests/scripts/85-store-lifetime-enum-match-borrowed-view-overfree.loft` passes both backends. Cluster A's tracked bugs are done; the remaining ownership-substrate work is Cluster C. | — | ✅ done (residuals closed) |
| **C** | **Cluster C — fold `copy_claims` onto the keystone** (was: "per-`Parts` container taxonomy"). `remove_claims` already collapsed onto `for_each_owned_child` (C.0–C.3, merged) — that is the model thin-visitor and the proof the fold works. **The remaining scope is `copy_claims` ALONE.** A 2026-06-22 design probe *falsified* the wider framing: `validate_claims` does **NOT** fold (a defensive walk over suspected-corrupt heaps — it bounds-checks before following a pointer, where the keystone trusts it), and `record_new`/`record_finish` is a WRITE path, so forcing it onto a read-walk is over-reach. Retires the densest HISTORICAL bug cluster (@P290 SIGSEGV, @P306/@P318 hash slot-drift, @P309, #260/#330) **by construction**. Now H10. **This was the last item of gate 1.** A work item under the light flow, not a plan — the design was settled and the phases were three mechanical helper folds, so *this row is its lifecycle*. **✅ DONE 2026-07-10** (branch `tuxedo-cluster-c`): `for_each_owned_child` is now the single source enumeration for `remove_claims` and all four `copy_claims` kinds (`hash_body` already read it; phases 1–3 folded `index_body` → `array_body` → `seq_vector`). Each phase verified on both backends against the keystone guard + the phase's named regressions + the leak gate; a per-fold count `debug_assert` (proven non-vacuous) closes the length-vs-count gap `LOFT_COPY_CHECK` leaves open (phase 0 calibration). Destination build stays per-kind. | **S per copy helper** (was mis-sized M–L against the falsified wider scope) | ✅ done — [STABILITY_REDFLAG_REMEDIATION § Cluster C / H10](STABILITY_REDFLAG_REMEDIATION.md#cluster-c--h10--fold-copy_claims-source-enumeration-onto-the-keystone) |
| **@PLN87** | **Reference-default `&`-binding semantics — DONE.** `&` binds a LIVE REFERENCE to an addressable source (variable / field / element): reads see the source, writes and field/element mutation write through, uniform across scalars and heap. Shipped via PR #436 (the L1–L7 ladder, both backends) + #506 (`&`-write-back to a computed lvalue) + the W4 redundant-`&` lint (#510, on by default). The corrected live-reference model supersedes the original write-back framing; realizes the OWNERSHIP_MODEL binding rule. | M | ✅ done ([@PLN87](https://github.com/loft-lang/plans/issues/87)) |
| **B** | **Cluster B — stack-delta wrong-signal.** Deferred — unverifiable, no RED probe fires; latent. Pick up only on a real trigger. | — | ⏸ deferred |
| **462-leak** | **#462 residual store leak — CLOSED (2026-06-26).** The native-only `MonsterDef` record leak (the `mon_*` borrowed-view shape) is fixed and landed; issue #462 closed. | S–M | ✅ done |
| **N** | **Native mixed-mode boundary — CLOSED (2026-06-27).** #460 (`--interpret` aborts when a main-program fn is marked for cdylib dispatch with no cdylib built) and #461 (interpret→native shared-store call corrupts a complex nested struct arg) are both fixed and landed. | M | ✅ done |
| **A1b** | **Temporary-subject borrow UAF — FIXED (2026-07-06, default ON, @PLN90 #516).** A borrowed return whose subject is freed before the result's last use is now materialised by the caller so the subject out-lives the result (the `deps` decision), on both backends. | M | ✅ done |

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
