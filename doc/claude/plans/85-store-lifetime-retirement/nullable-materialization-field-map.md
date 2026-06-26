<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Field map — nullable value materialization (crawler dogfood wave, 2026-06-25)

**Goal of this doc:** get *all* the shapes in this problem field first, document the
complexity of each failing point, then name the pattern between them — before fixing
anything. Driven by the crawler consumer wave ([#460](https://github.com/loft-lang/loft/issues/460)
/ [#461](https://github.com/loft-lang/loft/issues/461) / [#462](https://github.com/loft-lang/loft/issues/462)).

## Method

A probe **generator** (`gen_sweep.py`, kept with the session scratchpad) emits the
cross-product of axes and runs each shape on **both backends** under
`LOFT_STORES=warn`, classifying by a **cross-mode differential oracle**: interpreter
and native must agree and exit 0; a crash, a compile failure, a leak, or an
interp≠native divergence is a bug. ~95 shapes swept. Axes:

- **construction path** — literal · fn-return · element-read · field-read · ternary · coalesce · concat
- **element type** — `integer` · `text` · `struct` · `vector<int>` (nested) · `vector<struct>`
- **`??` default kind** — scalar-literal · vector-literal (`[]`, `[x]`) · struct-literal · element · fn-return
- **use context** — bind · inline · `len()` · for-loop · `+=` append · return · field-assign
- **delivery** — single arm · if/else · 3-arm match · ± a churn tail (force slot reuse)
- **nullability** — plain vs `__nullable` element · **backend** — interpret vs native

The **clean** axes (proves the safe region — do not let a fix break these): all
delivery shapes D-* (vector/text/struct/nested across single/ifelse/match ± churn);
all append shapes P-* (int/text/struct, plain & nullable, ± churn, to 150 elems);
and every coalesce whose default is a **scalar-literal**, **fn-return**, **element**,
or **struct-literal**. The bugs are NOT in delivery or append at minimal scale — they
are concentrated in two families below, plus the #462 slot-reuse class.

---

## Family A — `?? <vector-literal>` default is not materialized

**The one trigger:** a vector-typed nullable coalesced with a **vector *literal***
default — `nested_vec[i] ?? []` or `?? [99]`. The else-branch never materialises the
literal as a vector value.

**Sibling controls that are CLEAN** (the boundary): `?? scalar-literal`,
`?? fn_returning_vector()`, `?? another_element`, and (for struct elements)
`?? struct_literal`. So it is specifically the **vector-literal default**, not
coalesce-of-vectors in general. LHS source is irrelevant — index-read, field-read,
and 3-deep nesting all fail identically; a generated nullable var fails too.

**Complexity — the failure mode is not uniform; it varies on two sub-axes:**

| LHS elem type | default | `len()` / for-loop | bind / `+=` |
|---|---|---|---|
| `vector<integer>` | `[]` (empty) | 🟠 native E0308 (interp OK) | 🔴 panic BOTH |
| `vector<integer>` | `[99]` (non-empty) | 🔴 panic BOTH | 🔴 panic BOTH |
| `vector<struct>` | `[]` | 🔴 native compile BOTH contexts | 🔴 native compile (interp OK) |

- **native** (`E0308`): generated Rust is `if (ncc.rec != 0) {ncc} else { () }` — the
  default branch yields `()` instead of a vector. VERIFIED (`--native` rustc error).
- **interp panic**: `codegen.rs:3065` "Incorrect var `_vec_1[65535]` versus N" — the
  literal default's temp slot is mis-assigned at bytecode-gen. VERIFIED.
- **non-empty `[99]` is strictly worse than `[]`** — it panics even in `len`/for-loop
  where the empty default only failed native. (HYPOTHESIZED cause: the non-empty
  literal forces a populated temp the slot-assignment can't place.)

**Severity:** corruption/compile 🔴 (deterministic, both backends in most cells) ·
leak n/a (fails before run). **Crisp + minimal** — graduate-ready once the coalesce
else-branch materialises the literal default into the result temp.

Probes: `sib-nullcoalesce-nested-len.loft` (native-only), `sib-nullcoalesce-nested-bind.loft`
(both), `46A-coalesce-veclit-nonempty.loft` (`[99]`, both), `46A-coalesce-vstruct-veclit.loft`
(vstruct, native-only).

---

## Family N — vector-literal element is nullable-promoted by construction path

**The one trigger:** a vector literal `[elem]` assigned/returned where the context
wants a **non-nullable** `vector<S>`, but `elem` is **not a direct struct-literal**.

| element of `[ ... ]` | inferred type | result |
|---|---|---|
| `S { a: 1, b: "x" }` (struct literal) | `vector<S>` | ✅ OK |
| `mk_s(1)` (fn call → S) | `vector<__nullable<S>>` | 🔴 compile BOTH |
| `src.field` (field read → S) | `vector<__nullable<S>>` | 🔴 compile BOTH |
| `if c { mk_s(1) } else { mk_s(2) }` (ternary) | `vector<__nullable<S>>` | 🔴 compile BOTH |
| `opt ?? mk_s(0)` (coalesce) | `vector<__nullable<S>>` | 🔴 compile BOTH |

Error: `expected vector<__nullable<S>>, got vector<S> on return from block`.
VERIFIED both backends. The type system decides element-nullability on the element's
**syntactic shape** (is it a literal?) rather than its **type** (the fn provably
returns non-null `S`). Any non-literal element → over-promoted to nullable → mismatch
with a declared/inferred non-nullable vector.

**Severity:** compile-time 🔴 both backends (a type-inference defect, not codegen).
**Crisp + minimal.** This is why crawler's `enemies` / monster tables turn nullable
the moment a builder uses `vec += [mk_thing(...)]` or returns `[choose(...)]`.

Probes: `46N-litelem-fncall-promote.loft`, `46N-litelem-ternary-promote.loft`.

---

## Family #462 — stale-DbRef-after-slot-reuse UAF (the runtime crash)

Mechanism class VERIFIED, exact shape OPEN — full detail in
[cluster-462-slot-reuse-uaf.md](cluster-462-slot-reuse-uaf.md). Premature struct-store
free → slot reused → live stale DbRef corrupts the new occupant; `LOFT_NO_SLOT_REUSE=1`
makes it vanish. Does not shrink (needs ~190-store accumulation); the minimal `462-*`
and `sib-462-*` probes are clean controls.

---

## The pattern between them

All three families live at one seam: **materialising a non-null vector/struct *value*
where the @PLN25 nullable layer expects (or inserts) a `__nullable` slot.** The layer
handles *already-materialised* values (a variable, an element, a fn-return) but
mishandles a **freshly-constructed value** at that boundary, and it mishandles it
differently per phase:

| Phase | Family | The freshly-constructed value | Failure |
|---|---|---|---|
| **type inference** | N | a vector-literal *element* that isn't a literal | over-promotes the element to `__nullable` |
| **codegen** | A | a vector-*literal* coalesce default | fails to materialise it in the else-branch (`()` / mis-slot) |
| **runtime lifetime** | #462 | a fn-returned struct appended to a nullable vector | frees its store while still referenced |

The common deficiency: **the nullable layer reasons about values by syntactic shape,
not by type, and its materialisation path is only proven for the "value already
exists" cases.** A literal/fn-built value crossing into a nullable context is the
un-handled case — over-promoted (N), unmaterialised (A), or mis-owned (#462). The
"only a direct struct-literal element is safe" rule (N) and the "only a non-literal
default is safe" rule (A) are the same gap seen from two sides: the boundary has a
**materialisation hole for constructed vector/struct values**.

This predicts the fix locus is **one** place — *materialise the constructed value into
an owned temp at the nullable boundary, then let nullability wrap it* — rather than
three independent patches. (Design hypothesis, to be confirmed against the codegen
chokepoint.)

---

## Roadmap

1. **[S]** Family N — fix element-nullability to key off the element's **type**
   (non-null fn return / field stays non-null), not its literal-ness. Most crisp.
2. **[S/M]** Family A — materialise the vector-literal default into the coalesce
   result temp (both the bytecode slot and the native else-branch). Crisp.
3. **[M]** Confirm the single-locus hypothesis: do (1) and (2) share a chokepoint?
   If yes, fold; if no, the pattern was a false unification — keep them separate.
4. **[M/L]** #462 — needs the `LOFT_UAF` operand-stack/element extension first
   (cluster-462 roadmap).
5. Graduate the curated `46A-*` / `46N-*` probes per family as each fix lands.
