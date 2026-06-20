<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Stage C — design: the move / output-buffer calling convention for heap returns

**Status:** design (not implemented). This is the *target* the implementation must
produce, written and validated BEFORE touching loft's compiler. Method (per the
ticket's direction): design the correct bytecode + types → prototype it standalone
→ only then change the code to emit it. No poke-and-revert on the return path.

**Why this doc exists:** attempts 1–9 (see
[cluster-II](cluster-II-slot-init-dominance.md)) each patched a symptom and
regressed or were partial. The class is one structural gap — heap returns do not
transfer ownership — and the reliable fix is to define and prove the correct
convention first.

---

## 1. The minimal case (the whole class in one line)

```loft
fn enc(c: CV) -> vector<u8> { match c { CN => [0 as u8], CI { value } => [value as u8] } }
a = enc(k0);   // CI{1}
b = enc(k1);   // CI{3}
// correct: a==[1], b==[3]    observed: a==[3], b==[3]
```

`a` aliases a store that was freed-on-return and reused by the second call. Every
other shape (#405's conditional×unused loop; cbor `encode_map`'s multi-live-value
nest) is a composition of this one. If this is provably correct on BOTH backends,
the class is closed.

## 2. The invariant (what "reliable" means here)

> **A heap value returned from a function is materialized into a caller-owned
> output store passed as `__retbuf`. The callee BORROWS `__retbuf` for writing and
> never frees it. The caller allocates a DISTINCT store per binding (never reuses a
> work-ref while a prior binding is still live). Each store is freed exactly once,
> at its binding's scope exit.**

This is the C out-parameter / C++ construct-into-caller-storage (NRVO) model:
single owner at all times; ownership is the binding's from allocation to free. It
is the manual-memory equivalent of Rust/C++ move and of GC's "fresh object per
call" — all of which make `a = f(); b = f()` correct by giving each binding a
distinct object.

## 3. Correct convention vs. what loft does today

| Step | Today (broken) | Correct (move / out-buffer) |
|---|---|---|
| Caller pre-call | allocate work-ref `__ref_N`, **reused** per call-site across iterations | allocate a **fresh** store **into the binding's slot** (`OpDatabase(a_slot, vec_u8)`), distinct per binding |
| Pass | `f(args, __retbuf=__ref_N)` | `f(args, __retbuf=a)` |
| Callee fill | simple-literal arms build their OWN `__vdb` then "flow out"; head-call arms already write `__retbuf` | **every arm writes into `__retbuf`** (one buffer); no private return buffer |
| Callee exit | **frees its return-flowing buffer** (the bug — verified: `enc` emits `OpFreeRef(__vdb_2)` on the value it returns) | **never frees `__retbuf`** — it is the caller's output |
| Caller post | `a` **adopts** the returned store (already freed) | `a` already **owns** `S_a` (it allocated it); nothing to adopt |
| 2nd call | reuses `__ref_N` → allocator recycles `S_a` → alias | fresh `S_b` (allocator's `free_bits` skip the live `S_a`) |
| Scope exit | `a` & `__ref_N` both free → double-free / garbage | `OpFreeRef(a)`, `OpFreeRef(b)` — once each |

**The two concrete changes:** (1) callee never frees `__retbuf` and all arms write
it; (2) caller allocates the return store per-binding instead of a reused work-ref.

## 4. Target interpreter bytecode (the thing to prototype)

`main`, for `a = enc(k0); b = enc(k1)`:
```
OpDatabase(a_slot, vec_u8)          ; S_a := fresh empty vector, OWNED by a
OpVarRef(a_slot); OpSetInt4(4, 0)   ; len(S_a) = 0
... push c=k0 ...
OpCall(n_enc, args=[k0, __retbuf:=a]) ; enc appends into S_a
;  -- a now owns S_a, filled --
OpDatabase(b_slot, vec_u8)          ; S_b := fresh (S_a live ⇒ distinct slot)
OpVarRef(b_slot); OpSetInt4(4, 0)
... push c=k1 ...
OpCall(n_enc, args=[k1, __retbuf:=b])
... use a, b ...
OpFreeRef(b)                        ; free S_b once
OpFreeRef(a)                        ; free S_a once
```
`n_enc(c, __retbuf)`:
```
if   disc(c)==CN { OpClearVector(__retbuf); <append 0 into __retbuf> }
elif disc(c)==CI { value = OpGetField(c, CI.value); OpClearVector(__retbuf); <append value> }
return __retbuf                     ; the OUTPUT
;  -- NO OpFreeRef(__retbuf), NO private __vdb --
```
Decisive deltas vs. today's dump: **no `OpFreeRef(__vdb_*)` in `enc`**, and **`main`
does `OpDatabase` into `a`/`b`'s own slots** instead of one reused `__ref_N`.

## 5. Types / ownership annotations that make it work

- `__retbuf` is an **out-parameter**: the callee writes it but does NOT own it →
  marked **borrowed / skip-free** in the callee's scope analysis (its scope-exit
  must emit no `OpFreeRef(__retbuf)`).
- The binding `a` is the **sole owner** of its store: dep-empty owned vector,
  allocated by the caller into `a`'s slot, freed once by `a`'s scope-exit
  `OpFreeRef`. (This is the H3 "ownership carried, not re-derived" rule applied to
  the return path.)
- The callee's **return value type carries `["__retbuf"]`** (the block result is
  the out-buffer), so `returned_var` reports `__retbuf` as the return var and
  `get_free_vars` skips freeing it — generalizing what already works for a simple
  `return [literal]`.
- **Borrowed-view exception:** when the callee returns a value that ALIASES one of
  its args (e.g. `return some_arg`), the result is not freshly built into
  `__retbuf`; the callee must COPY into `__retbuf` (or the caller must), so the
  caller still owns a distinct store. Keep the conservative copy here (the existing
  `is_borrowed_view` guard).

## 6. The prototype — and the shortcut already validated

We already have a *source-level* prototype of the target bytecode: the owned-buffer
form
```loft
a: vector<u8> = []; a += enc(k0);   b: vector<u8> = []; b += enc(k1);
```
compiles to almost exactly the target (allocate `a`'s own store; append the call
result) and **worked on interp** (`a=1 b=3`, `/tmp/wa.loft`). So:

- **Interpreter target: VALIDATED.** The compiler fix reduces to "make
  `a = enc(k0)` emit what `a: vector=[]; a += enc(k0)` emits."
- **Native: the move bytecode itself is mistranslated.** The same `wa.loft` form
  returns garbage (`a=9`) on `--native`. So even the hand-correct move form is
  wrong on native — the native generator mis-implements the call-return / append
  ABI. This MUST be prototyped + fixed at the op level (in `src/generation/`)
  before any loft→bytecode change, or the interp/native divergence persists.

## 7. Execution plan (design → validate → build)

1. **Pin the interp target precisely.** Diff the bytecode of `a = enc(k0)` (broken)
   vs `a: vector=[]; a += enc(k0)` (correct). That diff IS the codegen-change spec —
   pure observation, zero compiler change.
2. **Native op-level prototype.** Run the *correct* op sequence through the native
   generator on a tiny fixture to pin which op the native ABI mistranslates
   (`OpCall` return delivery vs `OpAppendVector` vs the `__retbuf` arg). Fix the
   generator there, validated standalone.
3. **Only then** modify the compiler to emit the target (caller: `OpDatabase` into
   the binding slot; callee: write `__retbuf` on every arm, never free it). The
   step-1 diff says exactly where.
4. **Gates (both backends, no exceptions):** the minimal case → probe 05 (matrix
   A–F) → the cbor map suite → audience_crystal 02/03 → the leak gate
   (`LOFT_STORES=warn`) → the full suite with fresh cdylibs. A probe graduates to
   `tests/scripts/85-*.loft` only when all pass.

## 8. Why this is the structural fix (not another patch)

The "three mechanisms" of attempt 9 (simple-match free-on-return · `encode_map`
multi-live interaction · native ABI) are all the SAME missing invariant: ownership
is not transferred to the binding on a heap return. Enforcing §2 at the return-ABI
chokepoint — caller allocates per-binding, callee borrows-and-never-frees
`__retbuf` — closes all of them at once. The per-mechanism symptoms differ only in
how many live buffers expose the missing transfer.

---

## Anchors

- Mechanism + the 9 attempts: [cluster-II](cluster-II-slot-init-dominance.md)
- The allocator liveness this leans on: `free_bits` / `find_free_slot` in
  `src/database/allocation.rs`
- The return/branch lowering to change: `parser/control.rs::unify_if_branches_work_refs`,
  `scopes.rs::returned_var` / `get_free_vars`, `state/codegen.rs::gen_set_first_*`,
  and the native return ABI in `src/generation/`
- Ownership model: H3 (carried, not re-derived) · [LIFETIME.md](../../LIFETIME.md) ·
  [DEPS_INVENTORY.md](../../DEPS_INVENTORY.md)
