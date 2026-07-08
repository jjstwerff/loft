<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN101 — Detailed implementation steps

Reading order: [README.md](README.md) (design + status) → this file (verifiable steps) →
[INSPECTION.md](INSPECTION.md) (the code mechanism behind every step, confirmed 2026-07-08).
Each step states **what to change**, **where** (file/fn), and a **verification** that must
pass on **both backends** (`--interpret` + `--native`).

> **Headline from the inspection:** `Stores::finish_type` (`database/types.rs:317`) ALREADY
> computes every struct's inline byte layout (`types[t_nr].size`/`.align`/field `position`).
> A value struct is the SAME record bytes stored **inline** instead of behind a `DbRef` — an
> *inlining* change, not a layout redesign. Reference + value structs share the one layout
> home. Full mechanism map: INSPECTION.md.

> **Line-ref provenance:** refs marked **[confirmed 2026-07-08]** were read on today's tree;
> refs marked **[locate]** are the mechanism to find + confirm at implementation time. As in
> @PLN99, re-grep before editing — the tree moves.

> **The governing invariant (single home):** a value struct's *inline byte layout* has ONE
> home — the same `element_offsets` / `calculate_positions_with_groups` that lays out tuples
> and struct records today. Every path (size, construct, field r/w, copy, native codegen)
> reads that one layout; none re-derives it. This is the @PLN99/58 lesson applied to
> representation: single-home invariants make the off-diagonal matrix cells unable to disagree.

---

## Slice 0 — the composition matrix + the alloc-count harness (SPEC, before any code)

The matrix is the acceptance oracle; the alloc harness makes "zero-cost" a hard assertion.

- **0.1 — alloc-count harness.** The store tracker already counts allocations (the leak
  gate / "stores not freed" path — `LOFT_STORES` / the ownership-oracle harness **[locate:
  the store-allocation counter used by `tests/leak.rs` / ownership_oracle]**). Add a probe
  mode that reports **total store records allocated by a program run** (not just leaked), so
  a probe can assert `allocs == N`. Value-struct cells assert this count is **unchanged**
  vs. an all-scalar baseline (the struct added zero heap records).
  *Verify:* a reference-`struct` program reports `allocs > 0`; the same program with the
  struct flipped to `value struct` reports the baseline count.
- **0.2 — the matrix probes** (`/tmp`, `--interpret` first, then both). Vary ONE axis per
  probe; hand-compute value AND alloc-count AND leak:
  - **placement** × {local, temp, field-in-reference-struct, field-in-value-struct,
    `vector<V>` element, fn arg (by value), fn return}
  - **operation** × {construct, field read, field write, `<`/`==`/`-`, `{x}`/`{x:spec}`, `as`}
  - **field shape** × {all-scalar (pure-value), nested value struct, (Slice 4) text/vector}
  - **backend** × {interpret, native}
- **0.3 — freeze the spec.** Record the pass/fail + alloc-count table in this file; it is the
  Slice-1..5 acceptance oracle. Probes graduate to `tests/scripts/` as each slice lands.

### Slice 0 findings (2026-07-08, CORRECTED) — the real cost is the per-construction scratch store

Harness landed: two counters on `Stores` (`database/mod.rs`) — `records_created` (`record_new`
events, `structures.rs:60`) and **`stores_allocated`** (store-SLOT go-live events,
`allocation.rs`) — reported at exit via `LOFT_ALLOC_REPORT=1` (`state/mod.rs::check_store_leaks`)
→ `loft-alloc: stores=S records=R`.

**`record_new` MISLED (first write-up was wrong).** It counts logical records (0 for locals,
100 for vector appends), which hid the real cost. Report is now
`loft-alloc: peak=P allocs=A records=R` — **`peak`** = max LIVE stores (memory), **`allocs`** =
store alloc/free CYCLES (the per-construction abstraction cost). Measured (`--interpret`,
100× loops):

| shape | peak (memory) | allocs (cycles) | reading |
|---|---|---|---|
| scalar loop | 2 | 2 | baseline |
| **local `struct`**, 100× | **3** | **102** | **memory is FINE — the struct's store is REUSED each iteration (peak flat).** The cost is 100 per-construction **alloc/free cycles + DbRef indirection**, not memory (user: "an individual variable of T can be its own store… the same store will be reused") |
| `vector<P>` (flat), 100 elems | 4 | 4 | elements **inlined into the backing** (store #1); no per-element cycles |
| record field (`struct` in `struct`) | — | — | **already inline** (`finish_type` sizes a field by the nested record's full `size`, not a DbRef) |

**Goal = "zero abstraction structures" (user):** a `value struct` must have **NO** per-construction
alloc/free cycle **and NO** DbRef indirection — inline bytes wherever it lives. The per-slice
metric is **`allocs` delta → 0** vs the reference-struct baseline (peak is already reuse-bounded,
so it is NOT the target — cycles + indirection are), plus a structural "no scratch store, no DbRef".

**Corrected consequences (this REVISES the earlier note):**
1. The big win is **transient locals / temporaries / args / returns** — every reference-struct
   value you construct allocates a scratch store (100 loops → 100 stores). A `value struct`
   built inline allocates **none**. So **Slice 1 (value struct as a local) is the highest-value
   slice, not moot** — the earlier "locals are free" was a `record_new` artifact.
2. **Record fields are already inline** (storage-wise) — "zero cost inside records" is largely
   done for STORAGE; the residual value-struct win there is dropping the `field_ref` access
   DbRef + value/copy semantics.
3. **`vector<P>` already inlines P** into a single backing store (#1); it is NOT a single total
   allocation (4 slots: stack + backing + per-element construction scratch). The residual win
   is eliminating the per-element construction scratch (build P directly in the element slot).
4. **Metric for every slice:** `stores_allocated` DELTA vs the scalar baseline on a fixed
   program must collapse to ~0 for the value-struct version (plus a structural "no scratch
   store, no DbRef" check).

---

## Slice 1 — value structs as fields + vector elements, via an ISOLATED copy pass

**Target (user):** value structs are normally used **as fields of other records** and **inside
vectors**; those must be **zero-overhead**. Storage already IS inline out-of-the-box (proven: a
`struct` element in a `vector<P>` stores inline — `finish_type` sizes a field by the nested
record's full size; `vector<P>` inlines elements into one backing store). So the ONLY real change
is **value (copy) semantics**: reading a value struct out of a field/element must give a COPY, not
an alias (proven bug today: `e = b.items[0]; e.x = 99` writes back → `b.items[0].x == 99`).

**Scope cut (user):** do NOT bother making a standalone LOCAL variable DbRef-free — a reused store
+ a pointer is negligible. Copy fires only on a local bind; method params (`self`/`both`) stay
DbRef-into-store (zero-copy hot path).

**Approach (user, 2026-07-08): an ISOLATED pass — NOT wired into the type system.** A distinct
`Type::Value` threaded through the types caused a broad blast radius (an `if let Type::Reference`
sweep + an unbounded embedded-size refinement); reverted (commit 1240f459). Value structs stay
`Type::Reference`, marked only by `Data.value_structs` (Step 1.1). Value semantics is a single
self-contained IR pass — no type wiring, no assignment-path surgery, no ownership-oracle change.

Driver: flip `tests/scripts/515` DateTime/Duration to `value struct`, PLUS `vector<DateTime>` and
a `struct` with a `DateTime` field; every assertion green + no leak, both backends.

### Step 1.1 — parse + mark the kind — DONE (commit 5f43c13b)
`value struct T { … }` parses (a `value` soft-keyword prefix, peeked as an identifier —
`has_token` only matches keyword lexemes) and marks T in `Data.value_structs` (a set, NOT a
`Definition` field — those serialize through `ir_schema`; `Data::is_value_struct(d_nr)` reads it).
Verified: parses + a value struct as a vector element stores inline out-of-the-box, both backends;
no regression (issues 748).

### Step 1.2 — the isolated copy pass — DONE (commit 8a0abc79)
Implemented `value_struct_copy(data)` + `vs_copy_walk` in `src/scopes.rs` exactly as specified
below. **Works, both backends:** `e = b.items[0]; e.x = 99` ⇒ `b.items[0].x == 1` (copy);
field-read + vector-element-read + loop-bind all copy; operators (@PLN99) still dispatch;
leak-free, clean exit. **No regression** — full suite 2690 passed / 7 pre-existing cdylib+WASM
flakes. ~100 lines, no type-system wiring. (Broaden the `Borrowed`-only trigger to local-to-local
value-struct binds later if needed; the field/element target — the stated scope — is covered.)

#### Step 1.2 spec (as implemented)
Called from `scopes::check()` AFTER `move_elide` and BEFORE the ownership scan (`scan_set`).
Mirrors `move_elide`'s shape: clone each fn's `code`, rewrite, write back. For each `Set(v, rhs)`
where `v`'s type is `Type::Reference(P)` with `is_value_struct(P)` AND `rhs` is a VIEW
(`use_analysis::ownership_of(data, d_nr, rhs) == Own::Borrowed` — a field/element read):

```
Set(v, rhs)  →  Insert([
    Set(v, Null),
    Call(OpDatabase,   [Var(v), Int(P.known_type)]),         // v mints its own store
    Call(OpCopyRecord, [rhs,    Var(v), Int(P.known_type)]), // deep-copy the view into v
])
```

Then clear `v`'s view dep (`variables.tp(v)` → `Type::Reference(P, Deps::none())`) so `v` is Owned
and freed at scope exit (no leak, no double-free). Because `v = OpDatabase(…)`, the ownership scan
classifies `v` `Owned` correctly (via `db_vars`) — sound, no oracle change. Copy-elision is free
(an already-Owned rhs — a fresh `P{…}` — is not a view → skipped). Method params untouched (a call
arg is not a `Set(local, …)`, so it stays a DbRef into the store).

Recursive walk: mirror `variables/validate.rs::build_scope_parents` (Block/Loop → operators,
Insert → ops, If → cond/then/else, Set/Return/Drop/Span → inner, Call/Iter → children).

### Step 1.3 — verify + regression — DONE (commit pending)
Regression `tests/scripts/516-value-struct-copy.loft` (value-struct field-read, vector-element
read, loop-bind all copy; a plain reference `struct` control still aliases; operators dispatch) —
both backends, leak-free. `tests/scripts/515` flipped: `DateTime`/`Duration` are now `value
struct` — the full first-grade type (operators + format + conversions) passes on both backends
with value semantics. `ownership_oracle` clean; leak-scan ratchet over all scripts green; only
pre-existing cdylib/WASM flakes fail.

#### Step 1.3 (original) — verify (both backends + leak) — the acceptance gate
`value struct P { x: integer }`, `b.items=[P{x:1}]`, `e = b.items[0]; e.x = 99` ⇒
`b.items[0].x == 1` (copy, not 99) on `--interpret` AND `--native`; `ownership_oracle` clean (no
leak/UAF); full suite green. Then flip `515` + add a `vector<DateTime>` / DateTime-field
regression and graduate to `tests/scripts/`.

### Step 1.4 — non-null enforcement — DONE (commit pending)
A `value struct` is inline bytes with no `store_nr` null sentinel, so it has no null:
- **`<value struct>?` is REJECTED** at the `?` type-suffix (`parse_type`, definitions.rs:1553)
  with a clear diagnostic — new. Negative regression `tests/parse_errors.rs::value_struct_no_nullable`.
- **`p: P = null` is REJECTED** by DN1 (a plain non-null slot rejects null) — existing, no work.
- **An omitted value-struct field ZERO-inits** (a valid `P{cents:0}` — a default value, NOT null)
  — the implicit default satisfies "a value or a declared default"; no rejection needed.
- Reference `struct?` stays nullable — the rule is value-struct-specific. No regression
  (issues 748, parse_errors 159, wrap 51); clippy clean.

### Step 1.5 — @PLN99 dispatch + native (should be free)
Operators/format/conversions dispatch by def (`find_op_method`) — unchanged (value structs keep
their def). Native: struct records already compile and `OpDatabase`/`OpCopyRecord` are already
native, so the copy compiles as-is. Verify `515` (as `value struct`) passes native.

### Retired approaches (mechanism reference only)
- **`Type::Value` distinct value type** (variant + 7 arms + minting) — reverted (1240f459): broad
  `if let Type::Reference` blast radius + unbounded embedded-size refinement. The `Data.value_structs`
  marker survives to gate the copy pass.
- **Parse-level copy injected into `parse_assign_op`** — that assignment codegen is the documented
  UAF-hazard zone; the isolated pass does the same rewrite off to the side instead.
- **Flipping the ownership oracle to `Owned`** — CONFIRMED unsound (double-free: `scan_set` sets
  `owned_refs` but neither clears the dep nor emits a copy). The pass emits a real `OpDatabase`, so
  `Owned` is *earned*, not asserted.

## Slice 2 — value structs INSIDE records ("records too")

Record fields are ALREADY stored inline (`finish_type` sizes a struct field by the nested
record's full size — no DbRef, no per-field record), so the storage is done. The copy pass (1.2)
already covers reading a value-struct field into a local (`d = event.when` copies). A value-struct
field passed by value / returned copies via the same `Set(local, view)` rule when the callee binds
it; otherwise it stays a DbRef into the parent (read-in-place — fine).
- *Verify:* `struct Event { when: DateTime }` with `value struct DateTime`: `d = event.when;
  d.ms = 0` leaves `event.when.ms` unchanged; constructing Events adds no per-Event record; both
  backends.

## Slice 3 — value structs inside vectors (the DB-column win)

Vector elements are ALREADY inline (stride = record size; `vector<P>` inlines into one backing),
so storage is done; the copy pass covers `e = vec[i]`.
- *Verify:* `vector<DateTime>` of N elements adds no per-element record; `e = v[i]; e.ms = 0`
  leaves `v[i]` unchanged; bulk read stays inline; both backends.

## Slice 4 — lifetime-bearing value structs (text/vector fields) — DONE / SUPPORTED (commit pending)

**No rejection, no new code needed — they already work.** A value struct with a `text`/`vector`
field owns an inner handle, and the copy pass's `OpCopyRecord` (the existing deep-copy op, also
used by `v[i] = struct`) recursively DEEP-copies inner handles. So reading a lifetime-bearing
value struct out of a field/element gives a fully independent copy — the local's `text`/`vector`
is its own; freeing the local frees only the copy (no alias, no double-free, no leak).
- *Verified:* `tests/scripts/517-value-struct-lifetime.loft` — text+vector fields deep-copy;
  two independent copies from one source; loop-of-copies leak-free — both backends; the
  ownership_oracle leak-scan ratchet (sweeps all scripts under check-leak) is green.
- (A value-struct field that is itself a `reference<T>` / another owned struct is the same
  deep-copy path; not separately exercised yet — extend 517 if a consumer needs it.)

## Slice 5 — native + regression suite

Value structs are `Type::Reference` records — native already compiles them, and `OpDatabase` +
`OpCopyRecord` are already native, so the copy pass compiles as-is (no ABI work). Land the
regression: flip `515`, add `vector<DateTime>` + a DateTime-field struct with the copy-semantics +
no-leak assertions in `tests/scripts/`.
- *Verify:* full suite green both backends.

---

## Open questions

- **Q1 (size access) — MOOT.** No `Type::Value` variant, so no `&Type`-only size problem; value
  structs are `Type::Reference` records sized by `finish_type` as today.
- **Q2 — `&`-ref value structs.** A `&V` param already writes through to the caller; the copy pass
  fires only on a plain `Set(local, view)`, so `&V` (explicit in-place mutation) is unaffected.
- **Q3 (ownership) — HANDLED by the pass.** The copy emits a real `OpDatabase`, so the ownership
  scan classifies the local `Owned` (via `db_vars`); clearing the view dep frees it soundly. No
  oracle change.
- **Q4 — nullable `value struct?`.** No `store_nr` sentinel for an inline value; needs an inline
  representation (`Optional` + sentinel/discriminant) — ties into @PLN25. Deferred: start non-null
  (reject uninitialised, Step 1.4).
- **Q5 (keyword) — DECIDED:** `value struct`.
- **Q6 (@PLN97 layout contract).** Value structs do NOT change record layout (same record as a
  reference struct), so no layout-hash change; add a value-struct conformance case for the copy
  semantics.
