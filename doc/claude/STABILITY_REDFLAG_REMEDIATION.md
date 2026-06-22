<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->
# STABILITY_REDFLAG_REMEDIATION.md — specific steps to land the missing facts

The **actionable companion** to [STABILITY_REDFLAGS.md](STABILITY_REDFLAGS.md).
That doc names *which* re-derived facts manufacture bugs (5 clusters A–E) and the
leverage-first landing order; this one is the *how* — ordered, site-specific steps
to land each fact, each with its verification gate. Site lines validated against the
loft2 tree on 2026-06-21; re-`grep` before editing (line numbers drift).

## The discipline every step obeys (non-negotiable)

Per [CODEGEN_METHOD.md](CODEGEN_METHOD.md) + the `loft-codegen` skill: each step
**lands one fact computed once**, not a patch sprayed over N sites. So every step:

1. **Prove the working-vs-broken bytecode STANDALONE on both backends first** —
   `--interpret` AND `--native` — *before* touching the generator. Skipping this is
   how a codegen fix flails (the probe-04 anti-example).
2. **Build the boundary matrix** for the fact's class (CLAUDE.md matrix-first):
   vary type-kind / construction-path / depth / null / backend; hand-compute each
   cell's expected value; keep one deliberately-broken control cell red.
3. **Land at the chokepoint, enforce exactly the invariant** the failing region
   violates — no narrower (per-case patch), no wider (false unification).
4. **Verify against the full matrix on BOTH backends** + graduate a guard to
   `tests/scripts/`. Interp-vs-native divergence is a real hazard for every cluster
   here (it IS cluster A's `is_borrowed_view` and cluster C's H4 row).

## Tactical vs strategic — read this before picking an order

There are two valid entry points, and they trade off:

- **Strategic (default — the durable solve):** the leverage order **A → C → B → D**.
  Landing the *fact* dissolves the cluster; siblings and future cases arrive *with*
  the fact. This is what STABILITY_REDFLAGS.md prescribes.
- **Tactical (quick wins, but stopgaps):** **Cluster B** alone is small + bounded
  and can ship first for immediate correctness — BUT a B-patch without A leaves the
  wrong-signal *family* intact (you fixed 2 of N keyings). Treat B-first as buying
  a latent-bug fix, not as progress on the cluster.

Each numbered step below is independently shippable; the **prereq** lines encode the
real ordering constraints (only A and C have a hard prereq).

---

## Cluster A — carried return/bind ownership dep  *(do first; highest reuse)*

**The fact to land** (OWNERSHIP_MODEL rows 99/102/104, verbatim): a heap binding
reads ONE carried answer — *return-dep empty ⇒ **adopt** (move the store);
`{Attr(src)}` ⇒ **copy** on bind/escape* (row 102) — plus the *return-source SET
over arms* (row 99) and *one funnelled return path* (row 104). **Prereq:** typed
`Deps` (H2 / DEPS_INVENTORY.md) must carry the dep — land that first if not already.

**Steps:**

1. **A.0 — instrument the class.** Add a `LOFT_TRACE_RETOWN` env-gated `eprintln`
   at the *one* place a return value is bound (`parser/control.rs::block_result` /
   `ref_return`). Run the cluster-II matrix (the `@PLN85` probes 02–06) and **read
   off** which sites actually fire per shape — turns the "~10 sites" guess into a
   runtime fact (usage-sentinel; prove it CAN fire on a known-live case first).
2. **A.1 — compute the dep once.** In the callee analysis, compute the
   return-ownership dep as a *set* of sources (row 99): `∅ ⇒ Adopt`, `{Attr(src)} ⇒
   CopyFrom(src)`, mixed-over-arms ⇒ the union. Store it on the typed `Deps`, not
   re-derived. *Gate:* the dep value matches the hand-computed matrix for every arm
   shape, both backends, BEFORE any codegen reads it.
3. **A.2 — funnel the return path** (row 104): make `block_result` /
   mid-return / native-forwarder converge on ONE emit that *reads* the A.1 dep —
   delete the per-callee-shape adopt/copy/borrow arms + the `RetSite` fork.
   *Gate:* probe 05 green both backends; the `@PLN85` suite green.
4. **A.3 — replace the heuristics with the dep read.** At each of the **11
   `has_ref_params` sites** (`scopes.rs` ×7, `state/codegen.rs` ×4) and the bind-site
   (`parser/expressions.rs::assign`) + reassign forest (`codegen.rs` near
   `gen_set_first_at_tos`), replace the `has_ref_params && …` / `is_argument` /
   `vector_bound` forest with the carried dep. *Gate:* `grep -c has_ref_params`
   trends to ~0 in decision positions; full matrix both backends.
5. **A.4 — delete the runtime witnesses.** With the static fact present,
   `scan_set`'s runtime store-nr comparison and the `OpFreeRefIfDistinct` fallback
   become dead — remove them and confirm the bytecode no longer emits them.
   *Gate:* a `--static` dump shows the witness ops gone; matrix green.

**Dissolves on landing:** Cluster E (manifestation guards #405/@P290/@P317/@P377)
and the `is_borrowed_view` divergence (below) — re-run their repros to confirm.

---

## Cluster C — the `for_each_owned_child` traversal keystone  *(parallel to A)*

**The fact to land:** one per-`Parts` walk —
`for_each_owned_child(tp, rec) -> Iterator<(child, child_tp, stride)>` — carried,
with copy / free / validate / construct as **thin visitors** over it. This is H7's
`for_each_child` keystone on the heap-*cascade* side (file a new H-row when picked
up). Independent of A.

**Steps:**

1. **C.0 — pin the drift as the test.** The triad already diverges: per-`Parts`
   arm counts are **`copy_claims`=9 / `remove_claims`=10 / `validate_claims`=16**
   (`database/allocation.rs:1374 / 1682 / 984`), and only `copy_claims` has `_body`
   helpers (`:1163/:1233/:1303`). Write a matrix that exercises **every** container
   kind × {copy, remove, validate, construct} and hand-computes the word/slot
   layout — the divergent arms are where cells will disagree.
2. **C.1 — define the keystone** `for_each_owned_child` from the single per-`Parts`
   descriptor (element type, stride, container walk). *Gate:* for each kind it
   yields exactly the children the *union* of the three current dispatchers visits
   (the matrix from C.0).
3. **C.2 — rewrite `copy_claims` as a visitor** over the keystone first (it already
   has the helper structure). *Gate:* byte-identical heap to today on the full
   matrix, both backends; the `@P290`/`@P306`/`@P318`/`@P309` repros stay fixed.
4. **C.3 — rewrite `remove_claims` + `validate_claims`** as visitors; the 10/16-arm
   bodies collapse to the keystone walk. *Gate:* arm-count parity (all three now
   read ONE walk); leak/UAF probes green.
5. **C.4 — fold construction in:** `record_new`/`record_finish`/`insert_record`
   (`database/structures.rs`) and the `gen_set_first_*_null` codegen family
   (`codegen.rs:1091/1143/1303/1378/1390` + the `gen_set_first_at_tos` ladder) read
   the same element-word-count from the keystone. *Gate:* the `#260`/`#330` null-init
   repros green both backends.
6. **C.5 — unify the keyed re-dispatch** (`Type::{Sorted,Hash,Index,Spacial} →
   database.{kind}`) that today repeats in ≥4 files (`codegen.rs`,
   `parser/vectors.rs`, `generation/dispatch.rs`) into one keystone-keyed table —
   closes the interp/native H4 drift on this axis.

---

## Cluster B — apply the stack-delta template to the wrong-signal siblings

**The fact to land:** "did this branch leave a value, and how many bytes?" is the
**runtime net stack delta** (`true_stack != stack_pos`), never the last-expr or
function-return *type*. The `gen_if` `null_else` gate (#405) is the worked template.
Small + bounded — but see "tactical vs strategic" above.

**Steps:**

1. **B.0 — expose the masked shapes first.** Both bugs are latent. Write probes
   that *unmask* them: for **B5**, a non-tail value-`if`/`match` whose result type
   **≠** the function return (so `tp != returned()`), with eval-stack-divergent arms;
   for **`size_code`**, an `if` with a divergent then-arm and a value-pushing
   else-arm. Confirm each probe is RED on today's binary, both backends (a green
   probe can't validate the fix).
2. **B.1 — fix B5** (`state/codegen.rs`, the rebalance after both arms): change
   `ret_size = size(stack.data.def(stack.def_nr).returned(), …)` →
   `size(tp, …)` (the if-**expression's** result type). *Gate:* B.0's B5 probe flips
   to green, both backends; no regression in the `runtime_warnings`/codegen suites.
3. **B.2 — fix `size_code`** (`stack.rs`): read the drop-size from the net delta /
   the value-pushing arm, not the then-arm's static type. *Gate:* B.0's `size_code`
   probe green both backends.
4. **B.3 — graduate** both probes to `tests/scripts/` as the regression guard.

---

## Cluster D — converge the H6 `sentinel(tp)` consumers  *(S-sized, any gap)*

**The fact to land:** one `sentinel(tp)` encode/decode table (H6) every
null-sentinel site reads, instead of ~3 sites × 3 drifting copies.

**Steps:**

1. **D.1 — make the H6 `sentinel(tp)` the single source** (if not already a single
   fn, extract it). *Gate:* every consumer call-site computes the sentinel via it.
2. **D.2 — point the ~3 drifting copies at it**, delete the local re-encodings.
   *Gate:* a null round-trip matrix (per width / per kind) is byte-identical both
   backends.

---

## Standalone — the `is_borrowed_view` interp/native divergence  *(folds into A)*

If A is deferred, this is a moderate independent fix: the `0x8000` source-free bit
is derived **twice** — interp `state/codegen.rs:1727` (`let is_borrowed_view = {…}`)
vs native `generation/dispatch.rs:178` (`= self…`). Compute it **once** (a shared
fn over the carried dep) and have both backends read it. *Gate:* the OOB / hidden-
only edge that drifts today (H4) agrees on both backends across the matrix. Landing
Cluster A deletes this entirely — prefer A; do this only as an interim de-risk.

---

## Tracking

No bugs to file — open items map to existing `OWNERSHIP_MODEL` holes (99/102/103/104)
and H-rows (H2/H3/H4/H6); the genuinely-NEW ones (Cluster C keystone, Cluster B
`gen_if`/`size_code` siblings) get a forward H-row **when picked up**, per
STABILITY_REDFLAGS.md § What is NEW. Update OWNERSHIP_MODEL's hole table as each
step lands.
