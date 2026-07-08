<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN99 — detailed, verifiable implementation steps

Every step lists the **change** and a **Verify** with a concrete probe + the
hand-computed expected output, run on **both backends** (`--interpret` and
`--native`) — the step is done only when its probe passes on both. Reference type
throughout: `struct DT { ms: integer not null }`. Ordering is by dependency:
Stage-A matrix → Arc A (operators, incl. the null bug) → Arc B (format) → Arc C
(conversions). Arc D (value structs) is deferred (perf trigger) — no steps here.

> **Line-ref provenance:** Arc A + Step A5 refs are **analysis-confirmed on today's
> tree (2026-07-08)**. Arc B + Arc C refs are **DESIGN-era (2026-06-14)** and must be
> re-confirmed when picked up — line numbers drift (e.g. `call_op` moved
> `mod.rs:3964`→`:5304` since the DESIGN was written).

---

## Step 0 — Stage-A matrix (falsify the present state before any edit)

**Change:** none — build the boundary matrix as throwaway `/tmp` probes on
`--interpret`, one axis per cell, distinctive values, hand-computed expected.

**Verify** (record the pass/fail table; it is the acceptance oracle for Arc A):

| probe | today (baseline) | after Arc A |
|---|---|---|
| direct `a < b` (`a.ms=1,b.ms=5`, user `OpLt`) | ❌ "No matching operator '<'" | ✅ `true` |
| `a <= b`, `a > b`, `a >= b` | ❌ | ✅ |
| direct `a == b` | ✅ (pin: default struct-eq or user OpEq?) | ✅ unchanged |
| direct `a - b` (user subtraction op) | ❌ "No matching operator '-'" | ✅ ms delta |
| via `smaller<T: Ordered>(a,b){a<b}` | ✅ `true` | ✅ unchanged |
| `s: DT? = null; s == null` | ❌ `false` | ✅ `true` |
| `s != null` (null s) | ❌ `true` | ✅ `false` |
| `x: integer? = null; x == null` | ✅ `true` | ✅ unchanged |

The matrix must *prove it can fail* (the ❌ rows are real today, verified
2026-07-08). Done when the table is filled and each ✅/❌ is hand-checked.

---

## Arc A — direct concrete operator dispatch

### Step A1 — the resolution divergence *(ANALYSED 2026-07-08 — root cause found; no instrument step)*
`call_op` (`src/parser/mod.rs:5304`; DESIGN said `:3964` — moved) has two branches:
a generic-type-variable branch (the `t_<len>T_Op<Name>` stub, `:5320`) and a
**concrete** branch that iterates `self.data.get_possible("Op<Name>")` (`:5337`) —
the pre-built `possible` map. That map is populated **only** by `add_op`
(`src/data.rs:3937`), which registers a def into `possible["OpLt"]` iff its **name
starts with `"OpLt"`** (`:3947`). A built-in qualifies (`OpLtInt`); a user
`fn OpLt(self: DT, other: DT)` is stored as the method `t_2DT_OpLt` (or global
`n_OpLt`) — neither starts with `"OpLt"`, so it is **never in `possible`**. Hence
the concrete branch structurally cannot see user operators, while the generic
branch — and `find_fn` (`:3891`), which tries `t_<len><Type>_<fn>` then `n_<fn>`
then `possible` — can. **Confirmed; no instrumentation needed.** (`get_possible`
is `Data::get_possible`, `data.rs:3559` — a map read, not the resolver.)

### Step A2 — wire the concrete branch to `find_fn`
**Change:** in `call_op`'s concrete branch (`mod.rs:5335-5365`), before the "No
matching operator" error (`:5395`), resolve the user operator via
`self.data.find_fn(source, &format!("Op{}", rename(op)), &types[0])` — the **same**
resolver the method/generic path uses (`t_<len><Type>_Op<Name>` → `n_Op<Name>`).
If it returns `!= u32::MAX`, `call_nr` it and return its type. Built-in resolution
above is untouched; a struct with no such op still hits the error identically.
**Effort: S** — one resolver call reusing existing machinery.
**Verify:** Step-0 rows `<`/`<=`/`>`/`>=` flip to ✅ on both backends; a 2nd struct
with **no** `OpLt` still errors *identically* (no regression); `wrap`+`native`+
`expressions`+`issues` stay green (no false-match on built-ins / generics).

### Step A3 — the subtraction operator name
**Change:** confirm the real `Op<Name>` for `-` (the DESIGN said `OpMin`; verify
against the `OpCamelCase`↔symbol table in `src/create.rs`/`fill.rs` — `OpMin`
reads as *minimum*). Dispatch it for user structs.
**Verify:** `a - b` (`a.ms=5,b.ms=1`, user subtraction op returning `self.ms -
other.ms`) → `4` on both backends. `a - 5` (int) still errors (type safety kept).

### Step A4 — two-pass stability (falsify the desync risk)
**Change:** none beyond A2 — this is a verification gate.
**Verify:** a fn whose body uses `a < b` **before** the fn is textually reached in
pass 1 (call it from an earlier fn) still resolves; run the full `wrap` + `native`
suites — no new failures, no token-stream desync. (Pass 1 and pass 2 must agree
`<` is a user op; the interface path already relies on early signature
collection — prove it holds for concrete structs.)

### Step A5 — the null-equality bug (@PLN25 `== null` lowering; live `main` bug)
**Analysis (2026-07-08):** `== null` is lowered specially, not via a plain `OpEq`.
The @PLN25 model renders a struct nullable as `__nullable<S>` (an inline
discriminant enum; `src/parser/expressions.rs:2500+`), and `== null` should test
discriminant 0 — the **same** is-null the `??` path uses (the probe proved `??`
DOES see the null: `s ?? d` coalesced and `{s}` rendered `null`). For a custom
struct the two **disagree** — `s == null` → `false` while `??` sees null — so the
`== null` lowering for a struct `__nullable<S>` / null ref is the broken path
(built-in `integer?`/`text?` are fine). One more trace needed: where `s == null`
lowers for a reference/`__nullable<S>` operand and why it skips the discriminant
check `??`/is_null uses. **Effort: S–M** (align one lowering path).
**Change:** route `Reference`/`__nullable<S>` `== null` / `!= null` through the
is-null discriminant check (mirror `??`).
**Verify:** `s: DT? = null` → `s == null` → **`true`**, `!= null` → **`false`**;
`s = DT{ms:1}` → `false`; matches `integer?`/`text?`; both backends. **No
`i64::MIN` sentinel** — nullability stays standard reference-null.

### Step A6 — graduate regressions
**Change:** move the Step-0 matrix probes to `tests/scripts/NN-first-grade-ops.loft`
(3-backend) + a `tests/issues.rs` `code!` test for the null-equality cell.
**Verify:** the new tests pass in `wrap`, `native`, and the issues suite.

---

## Arc B — the `{x:spec}` custom-format hook

### Step B1 — drop the generic-only gate
**Change:** in `try_bound_to_text_call` (`src/parser/collections.rs:1037`) +
`append_data`'s `Type::Reference` arm (`:1166`), try the `t_<len><Type>_to_text`
lookup for **any** `Type::Reference(d_nr,_)`, not only the current bounded-generic
type variable.
**Verify:** `{d}` on a `DT` with `fn to_text(self: DT, spec: text) -> text` →
the `to_text` result (not `{ms:…}`); a `DT` **without** `to_text` → the generic
dump `{ms:…}` unchanged (no regression). Both backends.

### Step B2 — thread the spec
**Change:** pass the raw spec text as a `text` argument to `to_text` (`""` for
bare `{x}`); the hidden work-text output buffer carries through unchanged.
**Verify:** `{d}` → `to_text(d, "")`; a temporary `to_text` that echoes its spec
renders `""` for `{d}` and the spec string for `{d:SPEC}` (after B3).

### Step B3 — spec-parse branch on value type
**Change:** in the spec grammar (`src/parser/objects.rs:1367-1406`, `get_radix`
`:1455`/`:1470`), branch on the already-known value type `tp` (before `:` is
consumed): built-in → today's numeric grammar; custom struct with `to_text` →
read the spec as a **raw string up to the closing `}`** and hand it over.
**Verify:** `{dt:iso}`, `{dt:date}`, and `{dt:%Y-%m-%d}` all reach `to_text` as the
raw spec (no "unknown radix" error, no pad-char swallowing); built-in `{n:05d}` /
`{f:.2}` unchanged. Both backends.

### Step B4 — two-pass stability (the DESIGN's load-bearing claim)
**Verify:** the same format string in a fn body parses identically in pass 1
(numeric grammar attempted) and pass 2 (raw) — i.e. the `to_text` def is
discoverable in **both** passes so neither reads the spec with the wrong grammar.
Prove with a struct format used before its `to_text` def; `wrap`+`native` green.

### Step B5 — graduate + width/align decision
**Change:** graduate a `{x:spec}` script test. Record the v1 width/align decision
(type owns the whole spec, no outer padding).
**Verify:** the script test passes 3-backend; `{dt:date}` renders the date and
`{dt:date>12}` is documented as v1-unsupported (or v2 if it earns its keep).

---

## Arc C — user-defined conversions (`x as T`)

### Step C1 — pin the silent mis-cast (baseline)
**Verify:** `"2026-07-08" as DateTime` → `ms=null`, exit 0 today (confirmed
2026-07-08). This is the ❌ the arc removes.

### Step C2 — choose + probe the declaration shape
**Change:** decide: `OpConv<T>From<S>(v: S) -> T` (matches the built-in
`OpConv…From…` naming that `self.convert` resolves, `src/parser/fields.rs`) vs a
friendlier `fn from(s: S) -> T` dispatched by return type. Prototype the chosen
form.
**Verify:** the chosen declaration compiles and is discoverable at an `as` site in
both passes (same two-pass gate as A4/B4).

### Step C3 — dispatch `as T` to the user conversion, else clean-reject
**Change:** when `as T` targets a custom struct `T` and the source type is `S`,
look up the user conversion `S → T`; run it; if none exists, **error cleanly**
(not the silent `ms=null`).
**Verify:** `"2026-07-08" as DateTime` (with a `text→DateTime` parse) → a DateTime
whose `.ms` is the parsed epoch (hand-compute the epoch and assert it); `5 as
DateTime` with no `int→DateTime` conv → a **clean compile error**, never a silent
null. Both backends.

### Step C4 — `?? default` integration for fallible parses
**Change:** ensure `as T` composes with the checked-cast-with-fallback (#512) so a
runtime parse failure discharges to the fallback.
**Verify:** `"2026-07-08" as DateTime ?? epoch` → the parsed value;
`"not-a-date" as DateTime ?? epoch` → `epoch` (the fallback), no crash. Both
backends.

### Step C5 — graduate regressions
**Verify:** `text→DateTime` (parse) + the reject case + the `?? default` case land
as 3-backend script tests, green.

---

## Cross-cutting gates (apply after every arc)

- `cargo fmt --all -- --check` + `cargo clippy --release --all-targets -- -D
  warnings` + the `--no-default-features` clippy variant.
- The touched-subsystem suites (parser → `parse_errors`, `issues`, `expressions`,
  `format`; the 3 new script suites through `wrap` + `native`).
- Each arc's fix sits at the chokepoint enforcing exactly its invariant — no
  per-operator spray (Arc A is ONE lookup-wiring, not one branch per operator).

## See also
- [README.md](README.md) — the plan (arcs, probe evidence, open questions).
- [`../../lib_plans/21-datetime/DESIGN.md`](../../lib_plans/21-datetime/DESIGN.md) —
  the format-hook design (Arc B Part 1+2) + the DateTime library struct that consumes all of this.
