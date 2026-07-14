<!-- SPDX-License-Identifier: LGPL-3.0-or-later -->
# Nullable heap value consumed inline — the `??` / null-comparison ownership leak

**Status (2026-07-14):** `== null` / `!= null` half FIXED (`4d35f0d3`); the `??`
subject-consumed-inline half **FIXED** (this pass) for an OWNED `vector<T>` subject via the
`if`/`match` view-model — 562 is leak-clean on both backends and `loft_suite` is green.
Leak-diagnostics tooling that found it shipped (`5d1b8a70`). Branch `tuxedo-compat-gate-design`.

**One residual remains, newly characterized and SEPARATE** (see § the default-taken residual):
`owned_call() ?? d` when the call returns **null at runtime** (the default is taken, e.g.
`read_bytes("missing") ?? d`) leaks the DEFAULT record on both backends. It reproduces
IDENTICALLY before this pass, is NOT the subject-consumed-inline leak, and is NOT exercised by
562 (562's `??` cells all have present-but-empty subjects). Routed, not forced — it lives in the
shared `??` default-literal delivery (high blast radius near the freeze).

## The bug (reproduces on `main` — no fs/H4 needed)

A heap value (`vector<T>` or a reference struct) **returned by a call** and **consumed
inline** leaks its store, unless the result is bound to a direct-assign local:

```loft
fn mkv() -> vector<integer>? { [1,2,3] }
fn test() {
  x = mkv() ?? [1];          // ADOPT  — clean (x owns + frees)
  assert(mkv() != null, ...) // inline compare — LEAKED (now fixed)
  assert((mkv() ?? [1]).len() == 3, ...) // inline method — STILL LEAKS
  return mkv() ?? [1];       // STILL LEAKS
  out += [mkv() ?? [1]];     // STILL LEAKS
}
```

Surfaced by `tests/scripts/562-file-read-missing-null.loft` (H4): `read_bytes`/`list_dir`
return `vector<T>?`, consumed inline in asserts. The leak reported as `kt=27 File×4` — a
**stale slot-reuse label**; the real leaked object is the nullable-vector temp.

## The instrument that found it — `LOFT_LEAK_SITES` provenance (`5d1b8a70`)

`LOFT_LEAK_SITES=1` groups leaked stores by allocation site → source line, but `created_at`
was only stamped by `OpDatabase` (`alloc_record_at`); reuse/copy allocations kept
`created_at=0` → "line 0", unattributable. Fix: `Stores.alloc_pc` (republished per-op by the
interpreter dispatch loop, `state/mod.rs:3921`) stamped into `created_at` at the allocation
chokepoint (`database_named`, `allocation.rs`). Now every leaked store names its site. This is
the "attribution instrument" now in the engineering-rigor skill — it reframed a mysterious
`File×4` accumulation into the precise general bug above.

Run it: `LOFT_LEAK_SITES=1 LOFT_STORES=warn loft --interpret prog.loft`.

## Fixed half — `== null` / `!= null` (`4d35f0d3`)

`f() != null` (heap `f()`) lowered to `OpVectorIsNull(f())`, which reads only the null
sentinel and discards the returned store. Fix in `parser/operators.rs` `vec_null` branch: when
the operand is a heap TEMP (non-`Var`), bind it to a work-ref (`v_block([Set(w, operand),
OpVectorIsNull(w)])`) so `scopes.rs` frees it. A `Var` operand already owns its store — left
untouched. Both backends; empty-vs-null preserved. Regression guard:
`tests/nullflow_phase5.rs::text_opt_format_parity_*` covers the text-format sibling; the
vec_null cell should get an explicit leak-free test when the `??` half lands.

## The `??` subject-consumed-inline leak — FIXED (this pass)

### Why it leaked (the mechanism, fully earned via matrix + capture-and-diff)

`x ?? d` lowers (`build_null_coalesce_default`, `operators.rs:~1735`) to
`{ __ncc_1 = x; if (__ncc_1 != null) __ncc_1 else d }`. The subject temp `__ncc_1` was
`set_skip_free` + `register_work_ref`. `skip_free` suppresses its free and hands ownership to
the CONSUMER — which only a direct assign adopts (`x = …` → `OpFreeRef(x)`). Every other consumer
(method-chain, return, append, discard) had no owner → the present-path store leaked.

### The working sibling = the spec: `if` / `match`

`if`/`match` heap blocks are **clean in every consumer**. Each branch materializes into a
function-scope owner (`__vdb` for a literal; the call result itself for a call), **always
freed**, and consumers are dep-backed **views** (`x(1):vector<integer>["__vdb_1"]`). Both branch
owners are freed unconditionally at function scope (the untaken one is null → the free is a
no-op); the result names one arm's owner, and whichever store is live belongs to one of the
freed owners. Captured with `loft introspect` as the target IR.

### The fix (OWNED `vector<T>` subject → view-model)

Three coordinated changes in `build_null_coalesce_default`, gated on
`owned_vector = matches!(lhs_type, Type::Vector(_, dep) if dep.is_empty())` — loft's core
ownership convention (`dep.is_empty()` == owned):

1. **`inline_ref` instead of `skip_free`.** `skip_free` conflated two facts: "don't ALLOCATE in
   the preamble" AND "don't FREE". They are separable — the keyed twin
   (`gen_set_first_vector_null`'s sibling, `codegen.rs:1465`) already de-conflated them.
   `inline_ref` alone gives the non-allocating null-sentinel preamble (so the call result does
   not orphan an empty-vector `OpDatabase`), while `get_free_vars` STILL emits
   `OpFreeRef(__ncc_1)` (it checks `is_skip_free`, not `is_inline_ref`). This is exactly the
   `__lift_N` owns-an-inline-call pattern (`scopes.rs::new_lift_var`). The `inline_ref`
   relocation in `expressions.rs` also places the slot-reserving null-init, so `register_work_ref`
   is dropped for the owned Vector.
2. **Block result = dep-backed VIEW** naming `__ncc_1` (`Deps::frame(vec![tmp])`), set on both
   the block type and `*ctp`. The consuming assign then makes `x` a BORROW → no second
   `OpFreeRef` → no double-free; transient consumers borrow and never free.
3. **BORROWED subjects keep the original skip_free path** untouched (their store is owned
   elsewhere; freeing it is a UAF — this is the 85-ncc-literal-return `vv[i] ?? [7]` case).

### The two prior "falsifications" were misread — CORRECTED

The earlier writeup said (1) dropping `skip_free` leaves the assign leaking because "`__ncc_1`'s
free is never emitted" and (2) "`get_free_vars` never frees `__ncc_1`". **Both were wrong.**
`get_free_vars` DOES emit `OpFreeRef(__ncc_1)` with `skip_free` off (the IR shows it). The real
failure of the naive drop was two DIFFERENT effects, exposed by reading the post-pass **bytecode**
(not just the IR): (a) the preamble flipped `__ncc_1` from a null sentinel to an empty-vector
`OpDatabase` that `mkv()` immediately ORPHANS — the actual leak; and (b) `x` stayed an owner, so
`x` and `__ncc_1` double-owned the same store. The view dep fixes (b); `inline_ref` fixes (a). The
lesson (now in the engineering-rigor skill): a "byte-identical IR" was actually a stale read — the
preamble op differs in the compiled bytecode.

### Gate (all green, both backends)

Matrix (`vector<integer>` OWNED subject × {assign, return, append, method-chain, method-value,
discard, `!= null`, `== null`, nested `?? ??`, reused-twice}): value + length + leak +
`LOFT_POISON=1` all clean on `--interpret` AND `--native`. Regression:
`tests/scripts/562-ncc-owned-subject-consumers.loft`. `562-file-read-missing-null.loft` (the
original surfacer) now leak-clean. `85-ncc-literal-return-delivery.loft` (the borrowed twin) still
green. `loft_suite` green; full suite has only the known env flakes (`engine_host_kernel` s5/s7
parallelism, `wasm_debug_relay`, `viewer_markdown` golden).

## The default-taken residual — SEPARATE, pre-existing, ROUTED

`owned_call() ?? d` when the call returns **null at runtime** (default taken — e.g.
`read_bytes("missing") ?? d`, or the probe `fn nun() -> vector<integer>? { null }; nun() ?? [7,8]`)
leaks the DEFAULT record on both backends. Characterized precisely:

- **Not a double-free** (`LOFT_POISON` clean) — a genuine unfreed store.
- `LOFT_STORES=timeline` shows the default `[7,8]` allocates **2 vector stores, frees 1** — the
  record (`__vdb_1`, `kt=21`) leaks while the second is freed.
- **Identical before this pass** (checked against the pristine `operators.rs`) — so it is NOT the
  subject-consumed-inline leak and NOT caused by the view-model migration.
- **Not exercised by 562** — 562's `??` cells all use present-but-empty subjects (a missing file
  is tested with `== null`, not `?? d`).

Suspected root: the `??` default-literal handling CLEARS `_vec_1`'s view dep
(`operators.rs:~1583`) to make it an owned local for slot reservation, diverging from the
`if`-sibling (which keeps `_vec_1["__vdb_1"]` a view and frees only `__vdb_1`). Freeing the
field-0 view `_vec_1` before the record `__vdb_1` appears to corrupt the record's reclamation.
**Routed, not forced:** the default-literal delivery is shared by every `?? [literal]` shape
(borrowed subject, all element types, the 150+ coalesce scripts), so a fix there has high blast
radius and belongs in a dedicated pass — not near the freeze. The `if`-sibling view-preserving
mechanism (a view-typed `_vec` null-init, freeing only `__vdb`) is the target.
