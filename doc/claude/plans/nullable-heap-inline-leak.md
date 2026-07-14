<!-- SPDX-License-Identifier: LGPL-3.0-or-later -->
# Nullable heap value consumed inline — the `??` / null-comparison ownership leak

**Status (2026-07-14):** ALL THREE `??` heap-ownership leaks FIXED for an OWNED `vector<T>`
subject, plus the earlier `== null` / `!= null` half (`4d35f0d3`):
1. `== null` / `!= null` on a heap temp (`4d35f0d3`).
2. **Subject consumed inline** (return/append/method-chain/discard) — the view-model migration.
3. **Default taken** (`owned_call() ?? d` with the call null at runtime, e.g.
   `read_bytes("missing") ?? d`) — the default record's field-0 view no longer orphans a
   preamble alloc.

562 is leak-clean on both backends, `loft_suite` is green, and the full owned × {present,absent}
consumer matrix (value + length + leak + `LOFT_POISON`) is clean on `--interpret` AND `--native`.
Leak-diagnostics tooling that found them shipped (`5d1b8a70`). Branch `tuxedo-compat-gate-design`.
A BORROWED subject (`vv[i]`) keeps the original skip_free hand-off throughout (freeing it is a
UAF; the return-delivery materializer owns its default's free — 85-ncc-literal-return-delivery).

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

## The default-taken leak — FIXED (same view-model insight, OWNED subject)

`owned_call() ?? d` when the call returns **null at runtime** (default taken — e.g.
`read_bytes("missing") ?? d`, or the probe `fn nun() -> vector<integer>? { null }; nun() ?? [7,8]`)
leaked the DEFAULT record on both backends.

### The mechanism (same orphan as the subject leak, one level over)

`LOFT_STORES=timeline` on `nun() ?? [7,8]` (assign) showed **2 vector stores allocated, 1 freed**
(vs the plain literal's 1-alloc-1-free) — the record (`kt=21`) leaks. Root: the `??` default-literal
handling (`operators.rs:~1575`) CLEARS `_vec_N`'s view dep so `gen_set_first_vector_null` takes the
**owned-init** path — which `OpDatabase`-allocates a `main_vector` store at `_vec_N`'s slot. But
`_vec_N` is ALWAYS field 0 of the record (`_vec_N = OpGetField(__vdb_N, 0)`), so that preamble store
is immediately OVERWRITTEN and ORPHANED — the **exact same orphan mechanism** as the subject temp,
one level over (the default arm instead of the present arm). It surfaces only when the default arm
actually runs AND no return-delivery materializer sweeps `_vec_N` (i.e. an assign / method / append
of an OWNED-subject `??`); the present-subject cases never run the else-arm, so the orphan is never
created there.

### The fix (`skip_free` on `_vec_N`, gated on an OWNED subject)

Keep the dep-clear (it reserves the slot — dropping it re-introduces the `_vec_N[65535]` slot panic
and a fresh leak), but ALSO `set_skip_free(_vec_N)`: that lowers the preamble to a null sentinel
(no alloc → no orphan; the slot is still reserved by the frame bump) and suppresses its scope-exit
`OpFreeRef`, leaving the record `__vdb_N` the SOLE owner (its free cascades to the whole vector).
This mirrors the `if`/`match` view-model, where the arm's `_vec` is a never-freed view of its `__vdb`.

**Gated on an OWNED subject** (`matches!(lhs_type, Type::Vector(_, dep) if dep.is_empty())`) — the
same condition as the subject-leak fix, and for the same reason: a BORROWED subject's `??` in
return-tail position hands `_vec_N` to the return-delivery materializer, which OWNS its free
(free-after-append + the cross-arm sweep). Suppressing it there double-drops and panics
(`keys.rs`); the borrowed-subject default-taken path was already leak-clean, so the gate touches
only the leaking class. Verified: owned × {assign, return, method, append, discard, empty-default,
nested, twice} clean both backends + `LOFT_POISON`; 85-ncc-literal-return-delivery (borrowed twin)
untouched. Regression cells added to `tests/scripts/562-ncc-owned-subject-consumers.loft`.
