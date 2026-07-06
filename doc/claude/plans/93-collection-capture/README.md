<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 93 — Collection capture into closures

## Status

**Step 0 done (falsification gate) — chokepoint VALIDATED, boundaries mapped.**
Approach B (store a captured collection as a `Reference`-DbRef, recover the collection
type for the body from `capture_context`) works. Findings:

- **C1 ✓ / C2 ✓** — read/lookup `h[key]` on a captured hash/vector/sorted/index works on
  **both backends** (~3 edited sites: `synthesize_closure_record` + the two body-read
  paths in `objects.rs`; the body override is 2 sites because pass-1 and pass-2 reads
  take different paths — a small, recorded deviation from the N≈2 prediction).
- **C4 ✓** — vector / sorted / index are the SAME family under Approach B (no separate
  arm); lookup works for all.
- **C3 ✗ FALSIFIED** — mutation-through a *bare* capture: an insert's store-realloc
  DbRef update never writes back to the closure record (a captured read is an rvalue
  temp, unlike a struct *field* lvalue). → **read-only; reject mutation loudly.**
- **Iteration native bug** — `for e in h` over a captured collection corrupts native
  state so a *subsequent* closure capturing the same collection reads an empty DbRef
  (interpret is fine; the outer collection and repeated *lookups* are fine). A deep
  native-codegen defect in iteration-over-capture. → **reject iteration loudly for now**
  (Phase 3b); lookup covers the routing use case.
- **C5** — leak-clean on the lookup surface (pending final `LOFT_STORES=warn` gate).

**LANDED — read + lookup + iteration + point-assign, both backends** (Steps 0–6a,
6b-partial). A bare `hash`/`vector`/`sorted`/`index` captured into a closure supports
key/index lookup, `for e in h` iteration, and `h[key] = value` point-assignment on
interpret + native, leak-clean, including escape past scope. The one remaining gap is
**append (`h += …`)**, rejected with a clear parse-time error pointing at the working
`h[key] = value` form (no silent breakage). Tests `tests/scripts/505` (working) + `506`
(the append rejection); LOFT.md § Closures documents the contract. Promoted from
[loft-lang/loft#511](https://github.com/loft-lang/loft/issues/511). Tracked as `@PLN93`.

## Goal

A bare collection variable (`hash` / `vector` / `sorted` / `index` / `spacial`) may be
captured into a closure body and used there for read, index, iteration, and
mutation-through — identically on the interpreter and `--native`.

## Effort + design

- **Effort:** MH
- **Design:** ✓ (detailed — invariant named, chokepoint chosen, claims to falsify listed)
- **Last touched:** 2026-07-06

## The one invariant

> A captured collection is a **borrowed 12-byte DbRef** into the outer scope's
> collection store. The closure record holds the *pointer*, never a copy; every site
> that stores, loads, lays out, or frees the capture treats it as a shared DbRef whose
> referent the outer scope solely owns.

This is exactly the invariant a captured struct `Reference` already satisfies — and
struct captures already work end-to-end on **both backends** (verified: a
`struct Box { h: hash<K[id]> }` captured into a closure reads *and mutates* `b.h[key]`
on interpret and native). A collection variable is *already* a 12-byte DbRef
(`variables/mod.rs` — Vector/Hash/Sorted/Index/Spacial are all `size_of::<DbRef>()`),
and `Parts::DbRef` is a non-owning leaf in the free cascade
(`database/allocation.rs`). The representation already satisfies the invariant; the
work is making every site honor it.

## Design decision — chokepoint, not spray

**Approach A (naive):** keep the attr typed `Hash/Vector/…` + a `share_sentinel` dep,
and teach every storage/codegen site to special-case it. "Treat as DbRef" must be
independently re-stated at **~11 sites**, every omission **silent**: `synthesize_closure_record`,
`typedef.rs` fill_database (keyed arm), `typedef.rs` (vector arm — separate!),
`get_field`, `set_field_check`, native `emit_field`, native `emit_def_create_recurse_fields`,
native read-op, native write-op, the body index/iterate path, and write-through
lvalue. The spike already shows the bite: interp hash-read works, but native panics
(`u16::MAX` used as a content type), vector read is wrong, mutation silently no-ops.
`N × silence` is high — **do not ship Approach A.**

**Approach B (chosen):** store the captured collection attr as a **`Reference`-DbRef —
the same representation struct captures use** — so schema / read / write on both
backends reuse the *proven* Reference path (P1 proves it on native). Recover the
collection *type* for the body from `capture_context`, which already carries it. Two
sites remain, neither silent-if-omitted:

1. `synthesize_closure_record` — emit the collection capture as `Reference`-DbRef.
2. body resolution (`objects.rs:346-364`) — type the captured name as its original
   collection type (from `capture_context`), so `h[key]` / iterate / `+=` type-check,
   while the value is the `OpGetDbRef` 12-byte DbRef.

**N ≈ 2 vs ≈ 11**, and the collapsed sites are proven code, not new special-cases.
**Prediction to validate the build against:** ~2 edited sites + the body override. If
it balloons, that is the alarm that a load-bearing claim (C1/C4) was false.

## Composition matrix — Stage A

Harness: `tests/scripts/`-bound boundary matrix (hash read/miss/iterate, vector,
sorted, index, spacial, multi-capture, mutation-through, non-zero default), every cell
hand-computed, re-run on `--interpret` **and** `--native` plus a `LOFT_STORES=warn`
leak check at each step. The feature is done when every cell is green on both backends
and the probes are graduated to `tests/scripts/NNN-collection-capture.loft`.

**Load-bearing claims to falsify first (Step 0 — expect to falsify):**

| Claim | Probe (both backends) | On falsify |
|---|---|---|
| **C1** hash/vector/… DbRef == struct Reference DbRef in shape | store a capture as Reference-DbRef, read+`h[key]` in body | reassess — invariant wrong |
| **C2** body type-override clean (attr=Reference, body types=collection) | same test — parser accepts `h[key]` on overridden type | reassess |
| **C3** write-through works via the Reference path | mutate inside closure, assert outer changed | **read-only + LOUD parse rejection of mutation (never a silent no-op)** |
| **C4** all 5 kinds one family (vector is position-indexed, separate arm) | run vector cell separately | real domain axis — record, don't force |
| **C5** borrow lifetime — record death frees only record; escape guarded; leak-clean | `LOFT_STORES=warn`; closure escaping the collection scope | rely on / extend the #318 escape guard |

## Sub-arcs

| Item | Status |
|---|---|
| **0** — Falsify C1–C5; clean baseline | **Done** — chokepoint validated; C3 (mutation) + iteration falsified |
| **1** — hash, read-only, both backends (synthesize Reference-DbRef + body override) | **Done** — interp + native, leak-clean |
| **2** — vector / sorted / index read-only (C4: one family) | **Done** — all lookup, both backends |
| **3a** — reject mutation through a bare capture (loud) | **Done** — `+=` / `h[k]=` rejected (parser/expressions.rs) |
| **3b** — reject iteration through a bare capture (loud; native defect) | **Done** — `for e in h` rejected (parser/collections.rs) |
| **4** — escape / lifetime guard (C5) | **Done** — a closure capturing a local collection returned past its scope reads correctly + leak-clean on both backends (the store survives with the escaping closure) |
| **5** — harden + land: `tests/scripts/505` (lookup) + `506` (rejections), full suites, docs, close #511 | **Done** — full suite green (canonical rebuild order), LOFT.md § Closures updated |
| **6a** — iteration over a bare capture | **Done** — the loop element is bound as a BORROW of the closure so the per-iteration free never whole-store-frees the shared collection (native only; mirrors the #481 coroutine fix, `parser/collections.rs`). Both backends, leak-clean |
| **6b** — mutation through a bare capture | **Partial** — point-assignment `h[key] = value` works (update + insert, both backends); **append `h += …` still rejected** — the keyed-`+=` insert is gated on a local-var target, so a captured DbRef target emits NO insert (see § below). Workaround: `h[key] = value`, or a struct-field wrap |

## Out of scope (record, don't absorb)

- Capturing a collection **by value** (copy) — borrow is the semantics, not a copy.
- Non-inline closure sources (returning a capturing fn-ref) beyond the existing #318
  rules.

## Phase 6b remainder — bare `h += …` append: mechanism + fix plan

**Symptom.** `h += entry` on a *bare* captured collection silently does nothing (the
outer collection is unchanged). It is rejected at parse time (lexical two-token guard
in `parse_assign`, `src/parser/expressions.rs`) so the no-op never ships silently.

**Mechanism (verified via `loft introspect`).** It is an *omitted insert*, not a
write-back problem. The lambda body for `h += K{…}` builds the `K` record in a fresh
store and immediately `OpFreeRef`s it — no `new_record` / `hash::add` /
`OpFinishRecord(h)` is emitted at all. The cause is the keyed-`+=` insert path,
`src/parser/expressions.rs:1590` (and the `+= [items]` twin at `:1140`):

```rust
if op == "+=" && var_nr != u16::MAX && matches!(f_type, Sorted|Hash|Index|Spacial) {
    … new_record(&mut Value::Var(var_nr), …) …   // insert, targeting a LOCAL var
}
```

The insert is **gated on `var_nr != u16::MAX`** (a local-variable target). A bare
captured collection resolves to an `OpGetDbRef` of the closure-record field, so
`var_nr == u16::MAX` → the branch is skipped → nothing is inserted.

**Why the workarounds already work** (neither goes through the `var_nr` gate):
- `h[key] = value` → the element-set path inserts into the captured **DbRef** directly.
- `st.coll += …` → the field-`+=` retarget (`expressions.rs:~755`) inserts into the
  field **DbRef**.
So the insert-into-a-DbRef machinery exists and is proven; only the *bare* `+=` path
insists on a local var.

**Verifiable steps** (each ends GREEN on `--interpret` AND `--native`, `LOFT_STORES=warn`
leak-clean; write each probe under `/tmp` first, graduate to `tests/scripts/` at the end).

- **Step 0 — falsify the one unknown (do this BEFORE any code).** Does the `new_record`
  insert path re-root a hash on a GROW, so an insert through a shared DbRef would need a
  header write-back? Probe with the *working* struct-field append, which already uses the
  same `new_record`→DbRef path: in a closure capturing a struct, `for i in 0..5000 { st.h
  += K{ id: i, … } }`, then assert the OUTER `len(st.h) == 5000` and a sample of keys read
  back. **Gate:** green → the DbRef target persists across grows, so **Step 3 is NOT
  needed** and the fix is Steps 1–2 only. Red at some size → Step 3 (header write-back) is
  required; record the grow boundary.

- **Step 1 — route the captured DbRef through the keyed-`+= <record>` insert
  (`expressions.rs:1590`).** Add a sibling branch: when `op == "+="`, `var_nr == u16::MAX`,
  `f_type` is a collection AND `f_type.depend()` carries `closure_param`, emit `new_record`
  targeting the captured collection's DbRef (`code`) instead of `Value::Var(var_nr)`.
  **Verify:** `h += K{…}` in a closure → `len(h)` grows by 1, `h[key]` reads the inserted
  value, both backends; and the already-working cells (lookup, iterate, `h[key]=v`,
  struct-field `+=`, `505`) still pass (no regression).

- **Step 2 — route the `+= [items]` twin (`expressions.rs:1140`).** Same retarget for the
  vector-literal push path. **Verify:** `h += [K{…}]` and captured `vector` `v += [x]`
  persist to the outer collection, both backends.

- **Step 3 — header write-back (ONLY if Step 0 was red).** After the insert, write the
  re-rooted collection header back to the closure-record field (mirror the struct-field
  case). **Verify:** the grow-boundary cell from Step 0 now persists through a *bare*
  capture, both backends.

- **Step 4 — boundary matrix, both backends + leak.** append-one · append-many ·
  append-that-grows · append-then-read · append-then-iterate · append + point-assign mix;
  the outer collection sees every insert, leak-clean. **Verify:** all cells green on both
  backends.

- **Step 5 — land.** Remove the append guard (`parse_assign`) + `506`'s append-rejection
  test; add the append cells to `505`; update `LOFT.md § Closures` (drop the append
  caveat) and this Sub-arcs row → Done. **Verify:** full `wrap` + `native` suites green;
  `505` (with append) green both backends; `506` reduced/removed.

**Effort ~M, risk medium** — the DbRef-insert machinery already exists (`h[key]=v`,
struct-field `+=`); the only real risk is Step 0's re-root question, which Step 0
resolves before any code is written.

## See also

- Implements the closures section of [`../../LOFT.md`](../../LOFT.md); ownership beacon
  [`../../OWNERSHIP_MODEL.md`](../../OWNERSHIP_MODEL.md) (DbRef = borrow, non-owning leaf).
- Source issue: [loft-lang/loft#511](https://github.com/loft-lang/loft/issues/511).
- Tracker: `@PLN93` ([loft-lang/plans#93](https://github.com/loft-lang/plans/issues/93)).
- Design method: `.claude/skills/design-protocol` (the N-count + falsify-the-claim discipline).
