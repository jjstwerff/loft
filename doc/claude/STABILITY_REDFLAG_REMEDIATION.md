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
3. **A.3 — replace the 11 `has_ref_params` sites + the bind/reassign forests**
   (`parser/expressions.rs::assign`, the `codegen.rs` `gen_set_first_at_tos` region)
   with the dep read. *Gate:* `grep -c has_ref_params` → ~0 in decision positions.
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
- [ ] A.1 (compute the return-source SET on `Deps`) — the set version
      `collect_return_sources` EXISTS but is gated behind the single-`u16`
      `returned_var` (`scopes.rs:1348`, narrowed to `ret_var==MAX && Vector`).
      Making the set the primary path is blocked by the keyed/enum free-suppress
      regression (§6h Edge A: ~13 tests) — needs A.2 first.
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
- [ ] A.3 (replace the 11 `has_ref_params` sites + bind/reassign forests) —
      partially enabled: `is_borrowed_view` now has one home; the
      `has_ref_params` adopt-vs-copy sites still re-derive (they encode "any
      visible ref param", a coarser proxy than "the return borrows a param").
      Their clean collapse depends on A.2.
- [ ] C.0–C.5 (the keystone)
- [ ] D.1–D.2 (the sentinel table)
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
