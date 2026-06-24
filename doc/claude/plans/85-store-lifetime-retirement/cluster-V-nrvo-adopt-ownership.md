<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN85 cluster V — NRVO adopt/append ownership-dep (the #437 return-aliasing regression)

> Formerly the standalone **plan-90** investigation (the ztcbor-consumer-reported #437 NRVO
> regression). Folded into @PLN85 as cluster V: it is the same class — *a vector local's `dep`
> must equal the store it owns*. The sub-clusters below (I-a/b/c = the corruption mechanism,
> I-d = the leak) are the #437 detail; the **Resolution — IMPLEMENTED** section is the landed fix
> (one invariant enforced at three sites, the per-site dep thicket reduced 4 → 3 — I-c deleted as subsumed; the `+=` backing-preserve retained, load-bearing on native).

## I-a/b/c — multi-arm vector `match` drops a later arm's return buffer (corruption)

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

---

## Cluster I-d — the leak residual: the `+= call()` append-source class (the deep root, now taken)

**The I-c fix stopped the corruption but left a LEAK.** The graduated guard
`tests/scripts/437-nrvo-return-aliasing.loft` — added to close the class — *itself*
trips the `wrap::loft_suite` leak gate (`1 store(s) leaked at program exit:
main_vector<integer(0,255)>`), reddening #443's CI across 5 jobs (quick suite, Test
ubuntu/macos, stack_align sweep, ASan sweep) plus a trivial Clippy nit at
`scopes.rs:1865` (`map(..).unwrap_or(false)` → `is_some_and`). The guard passes its
*value* asserts (corruption is fixed) but leaks. It slipped through because bare
`loft --tests` runs each fn in an isolated state; only the `wrap` harness reuses one
state across the file and leak-checks at the end — and the green-suite claim predated
the guard file.

### Boundary matrix (each cell its own program; interpreter + native; branch vs a fresh `origin/main`)

| shape | leaks? | main | verdict |
|---|---|---|---|
| `buf=[]; return buf` / `buf=[1,2]; return buf` | no | clean | — |
| `buf=head(); return buf` (adopt-return) | no | clean | — |
| `return head()` (direct) | no | clean | — |
| `buf=[]; buf+=[9]; return buf` (`+=` **literal**) | no | clean | — |
| `buf=[]; buf+=otherVar; return buf` (`+=` **var**) | no | clean | — |
| `buf=[]; buf+=head();` **(local, not returned)** | **no** | clean | — |
| `buf=[]; buf+=head(); return buf` | **LEAK** | leak | **pre-existing on main** |
| `buf=head(); buf+=head()×{1,2}; return buf` | **LEAK** | leak | **pre-existing** |
| adopt + `for{ buf+=head; buf+=head }` (encode_map_ic) | **LEAK** | leak | **pre-existing** |
| `a = head()+head(); return a` (inline concat) | **LEAK ×1** | **clean** | **REGRESSION (I-c)** |
| full `encode_ic` (struct+match+loop) | **LEAK ×2** | **leak ×1** | pre-existing **+1 from I-c** |
| full 437 guard (all 3) | LEAK ×4 | **CORRUPTS** | corruption fixed; leak remains |

### The single rule
A leak occurs **iff `vec += <call-returning-a-vector>` AND the accumulator escapes
(is returned).** Nothing else leaks — `+= literal`, `+= var`, adopt-and-return,
direct-return, and the *same* `+= call()` when the accumulator stays **local** are all
clean. It is a **resource leak, not corruption** (values always correct) — it matters
because a long-running encoder (the cbor/ztcbor signer) grows memory per encode.

### How much
**Exactly one store per escaping function that ends in `… += call()`** — the *last*
appended call's hidden buffer. Multiple appends still leak ×1 (earlier append-sources
are freed; only the last is mis-attributed to the accumulator). `encode_ic` → ×2 (one
append-source + one adopt-return store, the I-c face-flip); the full guard → ×4 (two
`encode_ic` calls × 2).

### Where, in the code — three sites, one root
1. **Root — the append inherits the source's NRVO dep.** `buf += head(..,__ref_N)`
   leaves `buf` with dep `["__ref_N"]` (the consumed source's work-ref) instead of its
   real backing `__vdb_1`. Under the dep model (`scopes.rs:14-19`) that means *“buf
   borrows from __ref_N”*. The **`=` path explicitly strips this inherited dep** via
   `make_independent` (`src/parser/expressions.rs:1871-1897`) — the whole @P292 / @P394
   / #415 / #426 family. **The `+=`/append path has no equivalent strip** → it is the
   unfixed sibling of that family.
2. **No source-free on vector-append.** The **struct**-returning append frees its
   source temp via the `0x8000` "free source after copy" bit (`copy_ref`,
   `src/parser/operators.rs:304-335`); `OpAppendVector` (`src/parser/vectors.rs:13-113`)
   has **no equivalent**, so head's `__ref_N` is deep-copied in and orphaned.
3. **The free-decision reads the wrong dep.** `get_free_vars`
   (`src/scopes.rs:1776-1893`): because `buf` borrows `__ref_N` and `buf` escapes, the
   source's free is suppressed → leak. The **I-c** hunk lives here
   (`scopes.rs:1859-1880`) and is what flipped the adopt-return store from *corruption*
   (main: freed-while-returned) to *leak* (branch: not freed).

### Regression vs pre-existing
The core `+= call(); return` leak is **PRE-EXISTING on `origin/main`** (identical on
both, both backends) — the new guard merely exposes a main bug no prior test covered
(the cbor `CMap` coverage gap). plan-90's I-c **added two** branch-internal leaks (the
inline-concat `N` and `encode_ic`'s second store) by conditionalizing the adopt-free,
while correctly fixing the corruption. So this is no longer “a plan-90 residual” — it
is the deep `+=` append-source ownership root (STABILITY_REDFLAGS cluster 1 / @PLN85
territory) that the I-c commit deferred; we take it here now.

### The chokepoint
One invariant fixes the class: **a vector local's `dep` = the heap store it actually
owns after its last assignment/adopt** — computed where each shape's store is decided,
not re-derived per-site with conflicting rules.

### Resolution — IMPLEMENTED (the red-flag thicket collapsed 4 → 3)

Driving the matrix to zero bugs, the four conflicting per-site dep rules reduced to
**three principled enforcements of the one invariant**, and the fourth (I-c) proved
**redundant and was DELETED**:

- **KEPT — `+=` backing-preserve** (`src/parser/expressions.rs`): a vector `+=` is an
  IN-PLACE append; `buf` keeps its OWN backing dep across `change_var` so the append
  never re-points `buf` onto the appended source's `__ref_N`. **Load-bearing on native**
  (see the correction below). Fixes the `buf += call(); return buf` family.
- **KEPT — `ref_return` adopt promotion** (`src/parser/control.rs`, the
  `site_adopts_v` exception): when the returned local ADOPTS a work-ref
  (`buf = head(..); return buf`, `buf`'s dep names the call's `__ref`), promote that
  `__ref` to `__retbuf` so `buf == __retbuf` (true NRVO). Fixes the mixed-arm #2 leak.
- **KEPT — concat-adopt owns the call's store** (`src/parser/vectors.rs` +
  `src/parser/operators.rs`): `a = <call> + …` returns the *adopted* store's dep, and
  `create_vector` SKIPS the redundant `__vdb` backing when the first operand is an
  adopting call (`body_adopts_call`). Fixes N.
- **DELETED — I-c witness-pairing** (`src/scopes.rs`, the escape-gated
  `OpFreeRefIfDistinct` + the `__ref_N → buf` vector pairing): once the bind/adopt deps
  are correct, the standard return-source suppression frees the adopt store exactly
  once. The red-flag-flagged escape-gate is gone (−40 lines), and the original #443
  Clippy failure (`scopes.rs:1865`) vanishes with it.

**Correction (the `+=` backing-preserve is NOT subsumed — a both-backends lesson).** An
earlier pass deleted the `+=` backing-preserve too (claiming a 4 → 2 collapse), verified
by env-gated removal showing the interp matrix + `wrap`/`issues`/`leak_cases` green. That
verification was **interp-only and leak-only** — it missed a **native value corruption**:
without the `+=` preserve, `change_var` re-points `buf`'s dep onto the LAST append source
`__ref_N`; `ref_return` then promotes that `__ref_N` to `__retbuf`, but `__ref_N` is ALSO
that append call's scratch buffer, so on **native** the call writes into `buf`'s own
backing and clips the result (`encode_map_ic`'s second `+= head()` → `len 1`; the 437
guard's I-c assert panics under the `native_scripts` suite). Interp tolerated the
aliasing; native did not. The `+=` preserve is therefore **retained** — this is precisely
the **interp/native divergence the differential oracle (D-op-1/2, @PLN89) exists to
catch**: a subsumption claim must be validated on BOTH backends *and* on values, not just
interp + the leak gate.

**Validation.** Full boundary matrix CLEAN on **both backends** (interp + `--native`),
including values; `wrap` 51/51 (437 guard passes), `issues` 746, `leak_cases` green; 437
and 438 pass under `--native --tests`. `fmt` + `clippy` clean.

The remaining @PLN85 reduction (deriving the owned-store dep in ONE computation rather
than three enforcement sites) is the next step, but the class is closed.
