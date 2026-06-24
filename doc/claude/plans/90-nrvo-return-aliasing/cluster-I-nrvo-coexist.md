<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Cluster I — multi-arm vector `match` drops a later arm's return buffer

**Severity (split by failure mode):**
- **Corruption (silent):** wrong value, no crash — the trust-fatal face. Interp clobbers a
  live result; native reads an empty/null buffer.
- **Leak / double-free:** none observed (the freed store is *reused*, not leaked).

**Affected probes:** 02 (cbor), 06 (raw min), 07 (nested-if), 08 (materialized min)
**Backend asymmetry:** both — interp `ki=3` (freed store reused), native `ki=99` (freed
buffer read as empty). Different symptom, same root.

## Mechanism (VERIFIED — agent IR/runtime traces + probe matrix)

A vector-returning function whose tail is a **multi-arm `match`/`if` where each arm
tail-calls a `__retbuf` function** (e.g. `head`, made a `__retbuf` fn by #437's NRVO).
#437 registers only the **first** arm's buffer work-ref (`__ref_1`) in the function's
return-buffer dep. Every later arm's `__ref_N` is **allocated but unregistered**, so scope
analysis **frees it at function exit** — the function returns a **dangling DbRef into a
freed store**. When two such results coexist (`ki = encode(a); … encode(b) …`), the second
call's `__ref_N` reuses the just-freed store (free-list reuse) and the two alias → clobber.

**Ownership framing (`formal/ownership.md`):** the unregistered arm violates **O-Move** —
its store was transferred out as the return value, but the callee still freed it. The fix
is to enforce *"every arm of a single-tail vector match delivers into the one return
buffer"* — i.e. collect **all** arms' buffer refs, not just the first.

**Runtime proof (probe 06, interp):** `n_encode`'s `ref_return` got `ls=[__ref_1]`
(current) vs `[__ref_1, __ref_2]` (loft2). Arm-2's store is freed (`OpFreeRef(__ref_2)`)
before `return`, then `main`'s second `encode()` reallocates the same store.

## Two sub-shapes (the matrix split them)

### I-a — RAW match-tail (FIXED, control.rs:906)
The match stays a raw expression tail (homogeneous tail-call arms; probes 06, 07). It
reaches the vector `BlockTail` path at `control.rs:906`, where `ref_return` is fed the
match's type dep `ls` — incomplete (`[__ref_1]`). **Fix:** union `ls` with
`collect_hidden_ref_args(tail)` so every arm's `__ref_N` is recovered:

```rust
let mut full: Vec<u16> = ls.to_vec();
if let Some(last) = l.last() {
    for w in Self::collect_hidden_ref_args(last, &self.data) {
        if !full.contains(&w) { full.push(w); }
    }
}
self.ref_return(&full, l, RetSite::BlockTail);
```
Verified: 06/07 → `ki=1`, both backends; `01` (#437's ct/ci) stays `len=2`; the 5
return-buffer regression scripts (298/137/100/150/139) stay green.

### I-b — MATERIALIZED match (FIXED, control.rs:4101)
When the match has a **block-bodied arm** (`{ buf=head(); …; buf }` — cbor's
CBytes/CText/CArray) `vec_match_candidate` fires (block_result ~669) and the tail is lowered
by **`materialize_vector_arms_into` (control.rs:4063)**, which sets `vec_arm_handled=true` and
**gates off** the 906 recovery (line 785). That materialiser rewrote only `Var`-terminal arms
(4081) and left a **`Call`-terminal arm** (`CI => head(0,value)`) at the `_ => false` fallthrough
— so its own `__ref_N` buffer stayed a separate store, which the epilogue freed while it was the
returned value → dangling ref → clobber (interp `ki=3`, native `ki=99`). loft2 escapes only by
*leaking* that store (its "1 stores not freed" warning) so the returned ref stays valid.

**Fix:** add a `Value::Call` arm to `materialize_vector_arms_into` (before `_ => false`) that
recovers the arm's hidden buffer ref via `collect_hidden_ref_args` and substitutes it onto the
shared return buffer `w` + `unregister_work_ref`s it — the exact pattern `ref_return` uses for a
bare-call return (control.rs:4620-4627). Now every arm of a materialised single-tail vector match
delivers into the one buffer; `buf == w` is a no-op (idempotent). Verified: probe 08 → `ki=1`,
**cbor (02) → `[162 1 2 3 4]`**, both backends; all I-a/#437 probes stay green.

`unify_if_branches_work_refs` was **refuted** as the path (agent-confirmed): it bails for 08
because the CB arm's terminal `buf` is a named local, not a work-ref → `all_work_refs=false`.

### I-c — vector ADOPT free at the witness-pairing gate (OPEN — cluster-1 red flag)
One level DOWN from I-a/I-b: not the match's arm-delivery, but inside `encode_map`,
`buf = head(5,n)` **adopts** head's `["??"]`-NRVO store (`__ref_1`). The witness-pairing gate
(`scopes.rs:956`) that records `__ref_N → v` for `OpFreeRefIfDistinct` covers only
`Reference`/`Enum`, **not `Type::Vector`** — so the epilogue emits a plain `OpFreeRef(__ref_1)`,
freeing the store `buf` aliases → dangling → a coexisting `encode` reuses it (probe 09; the ztcbor
`encode∘decode` clobber). Surfaced because ztcbor re-encodes a *decoded* `CMap`, and cbor's own
`roundtrip()` never exercises `CMap` — a test coverage gap, not a decode bug.

**Red-flag framing — `STABILITY_REDFLAGS.md` cluster 1 (return/bind ownership re-derived per-site).**
The "does this binding adopt its callee's store?" fact lives at ~6 sites that **disagree** on the
vector-NRVO case: `suppress_source` (scopes.rs:1643) + the bind-site codegen include `Type::Vector`;
the witness-pairing gate (956) does not; I-a (906) + I-b (4101) are two more re-derivations of the
same fact. **The proof the red flag is real:** the blanket fix (add `Type::Vector` at 956)
REGRESSED cbor 02 (`[162 1 2 3 4]` → `[162 1 2 4]`) — it over-fires for the APPENDED temp
`ki = encode(); buf += ki` (which must free UNCONDITIONALLY after the copy), the very case site 4101
owns. **A patch to site 956 broke the fix at site 4101** — the signature of a fact with no single home.

**Owned IN-PLAN (not filed, not routed to @PLN85).** An investigation owns every problem it
surfaces; plan-90 closes only when ztcbor is green with no regression.

**FIXED — `scopes.rs`, two hunks (the consistent, narrow fix).** The distinguishing signal is
**escape**: `buf` ∈ `return_sources` (its store flows to the fn return); `ki` ∉ (it's copied into
`buf` by `OpAppendVector` and discarded) — the same signal `suppress_source` (1643) already uses.
- **Hunk 1 (~956):** record the `__ref_N → buf` vector witness pairing **WITHOUT** the
  Reference/Enum `make_independent` dep-strip. (The blanket fix's regression came from the strip: it
  flipped the appended temp `ki` to `owns=true` → a per-loop `OpFreeRef(ki)` that freed `__ref_2`
  mid-loop. Skipping the strip keeps `ki`'s `["__ref_2"]` dep and its once-at-fn-exit free.)
- **Hunk 2 (~1856):** emit the conditional `OpFreeRefIfDistinct(__ref_N, buf)` **only when the
  vector witness is a return source** (`witness_ok`); Reference/Enum keep their unconditional
  pairing. So the escaping `buf` gets the safe conditional free; the appended `ki` is untouched.

Verified BOTH backends: probe 09 `a len=4`; cbor 02 `[162 1 2 3 4]` (the case the blanket fix broke
— intact); **ztcbor suite 3/3** (the I-c gate); cbor suite green; I-a/I-b probes (06/07/08) +
01/03/04/05 green; guards 137/298. The deeper root (the `+=` path re-pointing `buf`'s dep — a
one-source-of-truth `operators.rs` change with large blast radius) is recorded but deliberately NOT
taken here: the contained free-site fix restores consistency without that risk.

## Hazards (agent-enumerated; check on I-b too)
1. **Native parity** — both backends corrupt differently; verify each post-fix.
2. **Don't over-unify into the multi-return-*statement* shape** —
   `tests/scripts/298-multi-return-site-ref-buffer.loft` deliberately keeps return-site 2+
   un-promoted (copy; arity must not grow). Keep the fix scoped to single-tail
   `match`/`if`, not multiple `return` statements.
3. **Leak watch** — the union must not promote a ref that shouldn't transfer (would skip a
   needed free). The 5 guard scripts + `LOFT_STORES=warn` are the gate.

## What we know vs don't

| | Status |
|---|---|
| I-a mechanism (raw match, arm-2 ref unregistered → freed → dangling) | ✅ VERIFIED (IR + runtime + probe matrix) |
| I-a fix (union at 906) correct + non-regressing | ✅ VERIFIED (06/07 both backends; 01 + 5 guards green) |
| I-b is the same root via the materialization path | 🟢 strong (probe 08 reproduces; cbor = this shape) |
| I-b fix site (the match→`result` materialization) | 🤔 located by symptom (the `"result"` var at 906); exact lowering line TBD |
