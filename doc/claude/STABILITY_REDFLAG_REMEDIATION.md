<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->
# STABILITY_REDFLAG_REMEDIATION.md — collapse the complex code, the bugs fall out

The executable plan behind [STABILITY_REDFLAGS.md](STABILITY_REDFLAGS.md) (the
5-cluster map). Each cluster is **complex code that re-derives one missing fact
across N sites**. The job is **not** to patch the N sites — it is to **land the
fact once**: the N-site code collapses to simple reads, and **the bug family
dissolves with it, by construction.** Sites confirmed against the loft tree on
2026-06-21; re-`grep` before editing.

## Worked precedent (this repo — the template for every step)

The cluster-A `match`-return-over-borrowed native E0308 was fixed by landing **one
fact**: `infer_type` now answers for `Value::Insert` (its tail-item type) — a
6-line case in `src/generation/emit.rs`. That single read made the existing
typed-null handler fire for *every* Insert-arm value-if, so the special-case gap
closed AND the bug auto-fixed (commit `6b29fe30`, regression
`tests/scripts/85-store-lifetime-match-return-borrowed.loft`). **That is the shape
of every step below: a missing fact, computed once, at the chokepoint — the bug is
a side effect of the collapse, not a separate fix.**

## The discipline every step obeys (the loft-codegen gate — non-negotiable)

1. **Prove the working-vs-broken bytecode/source STANDALONE on BOTH backends**
   (`--interpret` AND `--native`) *before* editing the generator. (probe-04 is the
   anti-example.)
2. **Build the boundary matrix** for the fact's class; hand-compute each cell's
   expected value; keep one deliberately-broken control cell red (prove it can fail).
3. **Land at the chokepoint, enforce exactly the invariant** — no narrower (a
   per-site patch leaves the siblings), no wider (a false unification).
4. **Verify the full matrix on BOTH backends + the suite**; graduate a guard to
   `tests/scripts/`. Interp-vs-native divergence IS the failure mode for A and C.

## Leverage order: A → C → B → D

A and C collapse the bulk; B's siblings and E dissolve behind A; D is S-sized
cleanup. Each numbered step is independently shippable + gated; the `prereq` lines
are the only hard ordering constraints.

---

## Cluster A — carry the return/bind ownership dep

**Complex code to delete (loft):** the **11 `has_ref_params` decision sites**
(`src/scopes.rs` ×7, `src/state/codegen.rs` ×4), the **8 `is_borrowed_view`
derivations** (interp `state/codegen.rs` vs native `generation/dispatch.rs` —
computed twice, divergently), the `block_result`/`ref_return` return thicket
(`src/parser/control.rs`), and the `scan_set` runtime store-nr witness
(`src/scopes.rs`).

**Fact to land** (OWNERSHIP_MODEL rows 99/102/104): a heap binding reads ONE
carried answer — *return-dep empty ⇒ **adopt**; `{Attr(src)}` ⇒ **copy** on
escape* (102), the *return-source SET over arms* (99), *one funnelled return path*
(104). **Prereq:** typed `Deps` carries the dep (H2).

**Steps:**
1. **A.1 — compute the dep once.** In the callee analysis, compute return-ownership
   as a *set* of sources (∅⇒Adopt, `{Attr(s)}`⇒CopyFrom(s), mixed-arms⇒union); store
   on `Deps`. *Gate:* the value matches the hand-computed matrix per arm-shape, both
   backends, before any codegen reads it.
2. **A.2 — funnel the return path.** `block_result`/mid-return/native-forwarder
   converge on ONE emit that *reads* A.1 — delete the per-shape adopt/copy/borrow
   arms + the `RetSite` fork. *Gate:* the @PLN85 probe suite green both backends.
3. **A.3 — replace the 11 `has_ref_params` sites with the dep read.** ✅ DONE
   (the 3 live sites — `scopes.rs`, `codegen.rs` `gen_set_first_at_tos` + `gen_set_first_ref_copy`)
   read `Definition::return_adopts_fresh_store()`. *Gate met:* `grep -c has_ref_params`
   → 0 in decision positions; matrix green both backends. See the A.3 progress note
   for the over-unification finding (the dep is broader than `returns_borrowed_view`).
4. **A.4 — unify `is_borrowed_view`** (compute once, shared interp+native) and
   **delete the `scan_set` runtime witness**. *Gate:* the OOB/hidden-only edge agrees
   both backends; a `--static` dump shows the witness ops gone.

**Bug family that auto-fixes:** #405/#406/#409/#410 class, the field-read/`return v`
aliasing, the `infer_type(Insert)` E0308 (already done — one site of this cluster),
and Cluster E's manifestation guards.

---

## Cluster C — the `for_each_owned_child` traversal keystone

**Complex code to delete (loft):** the `claims` triad **already drifted to
19/10/7 `Parts` arms** (`copy_claims` `src/database/allocation.rs:1374`,
`remove_claims:1682`, `validate_claims:984`) + the `copy_claims_*_body` helpers
that `remove`/`validate` lack; the `record_new`/`record_finish`/`insert_record`
construction (`src/database/structures.rs`); the `gen_set_first_*_null` codegen
family; the keyed `Type::{Sorted,Hash,Index,Spacial}→database.{kind}` re-dispatch
repeated across ≥4 files.

**Fact to land:** one carried walk —
`for_each_owned_child(tp, rec) -> Iterator<(child, child_tp, stride)>` (the
per-`Parts` descriptor), with copy/free/validate/construct as **thin visitors**.

**Steps:**
1. **C.0 — pin the drift as the test.** Matrix every container kind ×
   {copy, remove, validate, construct}, hand-computing word/slot layout; the
   divergent arms (19/10/7) are where cells will disagree.
2. **C.1 — define the keystone** from the single descriptor. *Gate:* per kind it
   yields exactly the union of children the three dispatchers visit today.
3. **C.2/C.3 — rewrite copy → remove → validate as visitors.** *Gate:* byte-identical
   heap on the matrix, both backends; @P290/@P306/@P318/@P309 repros stay fixed; the
   arm counts reach parity (all read ONE walk).
4. **C.4 — fold construction + `gen_set_first_*_null` in** (read element-word-count
   from the keystone). *Gate:* #260/#330 null-init repros green both backends.
5. **C.5 — unify the keyed re-dispatch** into one keystone-keyed table — closes the
   interp/native H4 drift on this axis.

**Highest-leverage NEW finding** (file an H-row when picked up).

---

## Cluster B — stack-delta signal (the wrong-signal siblings)

**Fact:** "did this branch leave a value, how many bytes?" is the runtime net stack
delta (`true_stack != stack_pos`), never the last-expr/function-return *type*. The
`gen_if null_else` (#405) fix is the template.

**Status:** **deferred — currently UNVERIFIABLE.** B.0 (2026-06-21) could not unmask
B5 (`codegen.rs` rebalance, `size(returned())` vs `size(tp)`) — the divergent-arm
precondition doesn't fire for reasonable shapes (loft equalizes arm stack levels
before the join), so a fix has no RED probe and the gate forbids patching it. Pick
up **only when a real trigger appears**, or behind Cluster A (the carried fact makes
the signal explicit). `size_code` (`stack.rs`) is the same family, same disposition.

---

## Cluster D — converge the typed-null encoders  *(S-sized, any gap)*

**Complex code to delete (loft):** the drifting null encoders —
`write_typed_null` (`emit.rs:1029`, native), `emit_typed_null` (interp,
`state/codegen.rs`), `STRING_NULL`/`init_ref_sentinel` (`state/mod.rs`).

**Fact to land:** one `sentinel(tp)` encode/decode table (H6) that every site reads.

**Steps:**
1. **D.1 — make `sentinel(tp)` the single source**; point the native + interp +
   string-null + ref-sentinel sites at it.
2. **D.2 — delete the local re-encodings.** *Gate:* a null round-trip matrix
   (per width / per kind) byte-identical both backends.

---

## Progress (this repo)

- [x] **Cluster A · one site** — `infer_type(Insert)` value-position typed-null gap
      (`emit.rs`). Commit `6b29fe30`; bug auto-fixed: match-return-over-borrowed
      native E0308. *Demonstrates the model: land the fact → bug falls out.*
- [x] **A.4 — unify `is_borrowed_view`** (the divergence fix). The
      `is_borrowed_view` fact was derived THREE times, structurally identically
      but separately (interp `state/codegen.rs` ×2, native `generation/dispatch.rs`
      ×1) — the H4 drift risk. Collapsed onto ONE method,
      `Definition::returns_borrowed_view()`, read by both backends (commit
      `20610eaf`, net −56/+72). Behaviour-preserving (the A.0 return-ownership
      boundary matrix is identical before/after on both backends); full suite
      clean both backends; regression
      `tests/scripts/85-store-lifetime-borrowed-view-query.loft`.
      **`scan_set` runtime witness DELIBERATELY retained** (the over-unification
      guard): `paired_witness`/`OpFreeRefIfDistinct` guards a *statically-
      unresolvable* adopt-vs-orphan case (a callee like `map_from_json` that
      adopts-or-allocates at RUNTIME on its input) — a genuinely different fact
      the single static return-dep cannot carry; forcing it onto the dep regresses
      that case.
- [ ] A.1 — **BOTH parts ROUTED: each blocks on a store-lifetime SUBSTRATE bug the
      prereqs did NOT actually unblock (2026-06-22 investigation).** No code landed
      (behaviour-preserving on `main`); the value is the precise localization + two
      substrate bugs to file.
      **(i) Return-source SET — hypothesis FALSIFIED by the leak suite.** Dropping
      the `ret_var==MAX` gate (make the union-of-arms SET primary) over-suppresses a
      free even WITH the companion `skip_free` de-conflation (`1ff929f5`), LEAKING:
      `repro_p365` (`main_vector<integer>`) for the vector widening, and
      `25-nullable-sequences` (`__nullable<NRow>::Some`) for the struct-enum
      widening (both caught by `wrap::loft_suite`'s strict per-script leak
      accounting; the authoritative `leak` + `leak_cases` suites and standalone
      `LOFT_STORES=warn` pass — it's an aggregate-only leak the gate was protecting
      against).  The `ret_var==MAX` gate is LOAD-BEARING for free-suppress
      correctness — it confines the set-path to the multi-arm case `returned_var`
      can't see; single-arm returns already free correctly via `in_ret`, and the
      set-path's `skip_free` mark leaks them.  The companion fix unblocked only the
      keyed ALLOCATION crash, not this broader free-suppress correctness.  Kept the
      original gated `free_vars` (`scopes.rs`).
      **(ii) #426 bind-site/return copy — store-reuse-after-free substrate.** The
      dep-driven copy ("any `OpGetField` whose result type carries a borrow dep ⇒
      allocate + `OpAppendVector`") is CORRECT per case (A `a=vv[0]`, C `c=o.inner.v`
      copy cleanly in ISOLATION, both backends), BUT it makes the source store DEAD
      at the read → FREED at the read site, and a subsequent NESTED 3-deep vector
      build (`vector<vector<vector<T>>>`) into the recycled store corrupts (`len 0`).
      The EXISTING #415 struct-field copy already hits this latently (no test
      followed it with a 3-deep append; 2-deep is fine, only 3-deep corrupts);
      widening to the index / nested reads turned the latent corruption into a real
      regression (`185-nested-boolean-vector`).  The return-path (B `b=idx0(w){w[0]}`)
      is the same substrate — the index-read tail gets the `["??"]` buffer ABI but
      never copies `w[0]` into it (the a7 class).  So #426 stays ALIASED and the
      #415 `expressions.rs` special-case is RETAINED (NOT deleted).
      **TWO substrate bugs to file (pre-existing on `main`):** (1) the
      store-reuse-after-free 3-deep corruption (repro
      `/tmp/p_followups/p426_store_reuse_3deep.loft`); (2) the return-buffer index-tail
      aliasing (#426B, the a7 sibling).  Probe
      `probes/07-borrowing-read-aliasing.loft` documents the residual (RED until the
      substrate fix).  Effort to UNBLOCK A.1: the store-reuse / return-buffer
      substrate (its own investigation — the a7 sibling — NOT a localized
      dep-driven copy).
      **WAY OUT (the unblocking sequence — substrate FIRST, then A.1 lands clean):**
      1. **Fix the store-reuse-after-free substrate** (the deeper root). A
         borrowing-read copy frees its source store at the read, the freed `store_nr`
         is recycled, and a later 3-deep nested build into it corrupts (`len 0`, both
         backends — `p426_store_reuse_3deep.loft`). Matrix-first on the allocator
         recycle path; two candidate chokepoints: (a) the allocator must not hand
         back a `store_nr` while a live downstream build still targets it (a
         use-after-recycle guard on the free-list), or (b) the borrowing-read copy
         must keep the source alive past the read (defer its free to true scope-exit).
         #415's existing copy hits this latently, so the fix is a net stability win
         beyond A.1.
      2. **Fix the free-suppress correctness** (the return-source SET). The
         `ret_var==MAX` gate is load-bearing because the SET marks EVERY source
         `skip_free`, suppressing the free even for a single-arm / owned return that
         SHOULD free. The dep must carry, per source, *owned-and-returned (suppress
         free)* vs *owned-and-freed (don't)* — not just source identity. Then the
         set-path can replace `in_ret` without the `p365` / `25-nullable-sequences`
         leaks.
      3. **THEN A.1 lands clean:** drop the `ret_var==MAX` gate (part i) and the
         dep-driven bind-site/return copy (part ii) read the now-trustworthy dep —
         closing #415 + the #425 inline-call sibling + #426 and DELETING the #415
         `expressions.rs` special-case (the original A.1 goal). probe 07 + the
         row-99/100 matrix are the both-backends gate.
      Track as a **store-allocator substrate investigation** (its own plan); the two
      substrate bugs are its entry points, repros banked in `/tmp/p_followups/`.
- [~] A.2 (funnel the return path) — **a2 LANDED; a7 localized + routed.** The
      implicit-tail whole-arg vector return (`fn idv(v) -> vector { v }`, matrix a2)
      now funnels through the SAME copy-into-`__retbuf` the struct-field tail (#415)
      and the explicit `return v` (`parse_return`) use: one shared helper
      `copy_borrow_tail_into_retbuf`, reached via a new `tail_whole_arg_vector`
      predicate gated to `context == "return from block"` (the one funnelled return
      path).  a2 is RED→GREEN on BOTH backends through bytecode (signature
      `-> vector["__retbuf"]`, not the old `["v"]` borrow); regression
      `tests/scripts/85-store-lifetime-implicit-param-return-copy.loft`.  The #415
      inline copy collapsed into the shared helper (net: 2 borrow-tail cases, 1
      copy site).  The earlier "fires in IR, bytecode reverts" symptom was the leaf
      fix landing in `ref_return` (which records a param `ls` as a borrow dep); the
      funnel sidesteps `ref_return` for the borrow-tail case entirely.  **a7 and a10
      are NOT the row-104 funnel** (see the A.0 table) — a10 was already fixed by the
      #409 forwarder branch on the A.4 floor (green both backends); a7 is a distinct
      *if-return buffer-model* substrate bug (the function buffer is named `__vdb_1`
      and the `if` true arm reuses it as its own build target via
      `OpGetField(__vdb_1, 0)`, then clears+frees the buffer mid-arm).  Its root is
      the `if`/`else` arm `result`-type asymmetry (the ELSE arm alone gets the
      concrete heap type at `parse_if` ~1404, so it runs per-arm buffer delivery;
      the true arm + every `match_arm` get `Unknown` and skip it), which a clean fix
      must reconcile — out of the row-104 funnel's tight scope, routed forward.
- [x] **A.3 — replace the 11 `has_ref_params` sites with the carried adopt-vs-copy
      fact.** The 3 LIVE decision sites (`scopes.rs:946`, `state/codegen.rs:2066`
      + `:2201`; codegen2201 is dead — the whole-suite usage sentinel found zero
      fires across `tests/scripts` + `tests/docs`, kept consistent for revival)
      now read ONE accessor `Definition::return_adopts_fresh_store()` (`data.rs`):
      **dep empty OR the `["??"]` one-buffer marker ⇒ adopt; any real attr index
      ⇒ copy**. `grep -c has_ref_params` in decision positions: 3 → 0.
      **Over-unification guard (the load-bearing finding):** the A.3 fact is
      STRICTLY BROADER than A.4's `returns_borrowed_view()` — that method checks
      only VISIBLE attrs, but the adopt-vs-copy decision must ALSO copy a HIDDEN
      `ref_return` work-ref return (dep `["cv"]`, the caller-reused `__ref_N`
      buffer). The first collapse onto `returns_borrowed_view` REGRESSED
      `143-plan51-cluster3` (cross-iteration `kt=66 Canvas` alias, interp-correct
      / native-leak — caught by the both-backends gate); the broader
      `return_adopts_fresh_store` fact splits `["??"]` (adopt) from `["cv"]`
      (copy) and fixes it. The refinement flips copy→adopt ONLY for the
      genuinely-fresh case (a ref param whose return is a fresh literal). The
      return-ownership boundary matrix (15 cells + control, both backends) is
      green before/after; full `cargo test` clean (the known stale-cdylib
      registry fixtures `p310`/`p171`/`imaging`/`v2_*` are unrelated — they pass
      on a fresh worktree build). `scan_set`/`OpFreeRefIfDistinct` +
      `paired_witness` adopt-vs-orphan witness DELIBERATELY retained (A.4 guard).
      Regression `tests/scripts/85-store-lifetime-return-ownership-adopt.loft`.
- [x] **C.0–C.3 — `for_each_owned_child` keystone** (commit `4ff673f8`): `remove_claims`
      collapsed 9 arms/174 lines → 2 arms/41 lines onto one carried heap-cascade walk;
      `copy_claims_hash_body` reads the keystone spine. Over-unification guard:
      `validate_claims` (defensive mirror, bounds-checks before deref) + `copy_claims`
      destination construction (genuinely per-kind) left separate. Matrix byte-identical
      both backends; @P290/@P306/@P318/@P309 repros green. C.4/C.5 (construction/null-init
      + keyed re-dispatch) routed forward.
- [x] **D.1–D.2 — typed-null encoders converged onto `DbRef::NULL`** (commit `526cac22`
      → cherry-picked `83d0484e`): every heap-ref null encoder reads one `DbRef::NULL`
      const (`keys.rs`) instead of re-spelling `DbRef { store_nr: MAX, … }`; the
      `pos:0`/`pos:8` byte-drift is gone. **Bonus H4 fix:** `fn f() -> character { return
      null }` was native-E0308 / interp-tolerant (the `character` null was routed to the
      integer sentinel) — now `OpConvCharacterFromNull`, byte-identical both backends.
      Over-unification guard: `STRING_NULL` (text `Str` sentinel) + interp `database.null()`
      (allocates a real null *store*, a different mechanism) left distinct. Regression
      `tests/scripts/407-cluster-d-null-sentinel-roundtrip.loft`.
- [ ] B (deferred — unverifiable until a trigger appears)

### A.0 boundary-matrix RED findings (pre-existing return-ownership bugs)

The A.0 matrix (`/tmp` corpus, both backends) surfaced three live, pre-existing
return-ownership bugs — the bug family A.2/A.3 will dissolve, precisely localized:

| Cell | Shape | Symptom (interp / native) | Localization / status |
|---|---|---|---|
| a2 | implicit-tail whole-arg `{ v }` borrow-return | ALIASES (`a.len 4` / 4) | ✅ **FIXED (A.2):** funnelled to the shared `copy_borrow_tail_into_retbuf` (the #415 / explicit-`return v` copy), `context == "return from block"` gate; GREEN both backends |
| a7 | `if`-return over owned literal arms | corrupt (`p0=null(oob)`, both backends agree wrong) | ⏳ **localized, routed** — NOT the funnel: the function return buffer is named `__vdb_1`, the `if` true arm reuses it as its own build target (`OpGetField(__vdb_1, 0)`) then clears+frees it mid-arm; root is the `if`/`else` arm `result`-type asymmetry (`parse_if` ~1404: only the ELSE arm gets the concrete heap type → runs per-arm delivery; true arm + `match_arm` get `Unknown`). A separate if-return-buffer-model fix. |
| a10 | forwarder `return mk(n)` | (on the A.4 floor) no leak, correct value | ✅ already GREEN — the #409 `native_forwarder` branch (`block_result`) copies the forward into `__retbuf`; the A.0 "interp LEAK ×2 kt=19" finding predates that landing |

The explicit `return v` (a1), `match`-return owned arms (a6), struct param return
(a8), field-of-param return (a5), and now the implicit-tail whole-arg return (a2)
all PASS both backends — the working templates the funnel converges onto.  Matrix
corpus reconstructed at `/tmp/clusterA_matrix` (13 cells + control), baseline-
classified against the A.4-floor binary: only a2 flipped FAIL→PASS, no green cell
regressed, both backends.

## Tracking

No bugs filed — open items map to existing `OWNERSHIP_MODEL` holes (99/102/103/104)
and H-rows (H2/H3/H4/H6); the genuinely-NEW ones (C keystone, B siblings) get a
forward H-row when picked up. Each landed step updates OWNERSHIP_MODEL's hole table.
