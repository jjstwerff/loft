<!-- SPDX-License-Identifier: LGPL-3.0-or-later -->
# Nullable heap value consumed inline — the `??` / null-comparison ownership leak

**Status (2026-07-14):** `== null` / `!= null` half FIXED (`4d35f0d3`); the `??` half is a
LOCALIZED, DESIGNED, but **unfixed** deps-substrate residual. Leak-diagnostics tooling to
find it shipped (`5d1b8a70`). Branch `tuxedo-compat-gate-design`.

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

## The `??` residual — DESIGN COMPLETE, NOT IMPLEMENTED

### Why it leaks (the mechanism, fully earned via matrix + capture-and-diff)

`x ?? d` lowers (`build_null_coalesce_default`, `operators.rs:~1735`) to
`{ __ncc_1 = x; if (__ncc_1 != null) __ncc_1 else d }`. The subject temp `__ncc_1` is
`set_skip_free` (`:1754`) + `register_work_ref` (`:1767`). `skip_free` suppresses its free and
hands ownership to the CONSUMER — which only a direct assign adopts (`x = …` → `OpFreeRef(x)`).
Every other consumer (method-chain, return, append, discard) has no owner → leak.

### The working sibling = the spec: `if` / `match`

`if`/`match` heap blocks are **clean in every consumer**. They use the OPPOSITE model: each
branch materializes into a function-scope owner `__vdb` (always freed), and consumers are
dep-backed **views** (`x(1):vector<integer>["__vdb_1"]`). The `??` must migrate to this
view-model: make `__ncc_1` a `get_free_vars`-visible function-scope owner + consumers views.

### Why it is NOT a session-tail patch (two falsifications)

1. **Dropping `skip_free` for Vector made it WORSE** — the assign case then leaked too. It
   flips `x` from owner to view but `__ncc_1`'s free is still never emitted.
2. **`get_free_vars` never frees `__ncc_1`** even as a non-`skip_free` work-ref: its only
   `Set(__ncc_1, x)` is block-internal, which the scope scan does not walk (the #319 gap —
   `operators.rs:1756` comment). And the skip_free-off assign IR is **byte-identical** to
   skip_free-on yet behaves differently → a `ref_debug`-level value-delivery subtlety (owner
   vs view), not visible in the static dump.

This is the deps/ownership substrate (loft's #1 weakness). Forcing it near the freeze, after
two falsifications, is the trap the engineering-rigor skill forbids.

### The implementation plan (for a dedicated pass)

- **Spec:** the `if`/`match` view-model (capture its IR with `loft introspect` as the target).
- **Two required parts:** (a) `__ncc_1` becomes a function-scope owner whose free `get_free_vars`
  emits (fix the #319 block-internal-`Set` scan visibility, or hoist the `Set` to the enclosing
  scope); (b) consumers become dep-backed views so the adopting assign transfers ownership
  without double-free. Trace the assign owner-vs-view flip with `LOFT_LOG=ref_debug`.
- **Scope carefully:** `skip_free` is kept for Text/Reference (Set-E probes 21/22/23/36/41/50
  depend on it) — only migrate `Vector` (and likely `Sorted`/`Hash`/`Index`) first.
- **Gate (the matrix, both backends, `LOFT_POISON=1` for UAF/double-free):** assign, return,
  append, method-chain, discard, `!= null`, null-subject-takes-default — all leak-free + correct;
  Set-E text probes green; full suite.

### Meanwhile — 562

After the `!= null` fix, 562 leaks `File×2` (lines 16, 26 = the two `(… ?? d).len()`). Options:
grandfather 562 in `tests/wrap.rs::SCRIPTS_LEAK_ALLOW` with a comment pointing here (honest:
tracked `??`-ownership leak, not an intentional alloc) to unblock `loft_suite`; or leave red as
the marker. **Not yet done** — left for the owner's call.

**No safe targeted patch for 562's method case** exists like the `!= null` one: the block is a
call *argument*, freed only by `scan_args`, which does not lift heap Blocks — so there is no
shortcut; it needs the deps view-model fix.
