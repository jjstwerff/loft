<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Site inventory — nullable / aggregate value materialisation

Companion to [nullable-materialization-field-map.md](nullable-materialization-field-map.md)
(the shapes) — this is the **code-site picture**: every place that decides how a
*constructed value* (literal, fn-call element, coalesce default, returned/appended
aggregate) is typed, materialised, and owned at the `__nullable` / vector boundary.

The point of the inventory: the three bug families live at **three layers**, and only
within a layer do they share a routine. The design question "one fundamentally-correct
routine?" is answered **per layer**, not globally (forcing all three into one routine
is the over-unification the design-protocol warns against). Verdicts are from the
probe sweep (`probes/`, `*-field-map.md`).

Legend: ✅ handles a constructed value correctly · 🔴 bug · ◆ chokepoint (one place
many paths consult) · ⟂ re-asserted per-site (the brittleness).

---

## Layer 1 — TYPING (front-end): "what type does this constructed value have?"

The relation is bidirectional checking (`TYPING_RELATION.md`): synthesis `⇒` /
checking `⇐ τ`, with the expected `τ` carried by the **one** consolidated
`Parser.expected` field (R1, already done).

### 1a. The `⇐` channel — one field, one set of readers ◆

| Site | file:line | role |
|---|---|---|
| `Parser.expected` field + readers `vector_hint()` / `enum_hint()` / `lambda_hint()` / `read_target_type()` | `parser/mod.rs` | the single checking-mode `τ` (R1 consolidation) |
| conversion relation `convert` / `cast` / `can_convert` | `parser/mod.rs ~1552 / ~1848 / ~1934` | `τ ⤳ τ′` — does a value of `τ` flow where `τ′` is expected |

### 1b. The nullable TYPE rewrite — one chokepoint ◆ (this part is right)

| Site | file:line | role | verdict |
|---|---|---|---|
| `sub_type` vector arm → `rewrite ... _to_nullable` | `parser/definitions.rs` (via `parser/expressions.rs:2345`) | the ONE place every inline `vector<S>` element resolves to `vector<__nullable<S>>`; **default = nullable**, `S not null` opts out (E3); gated `e2_rewrite_enabled` | ◆ correct — one home |

### 1c. The VALUE-side element typing — re-asserted per position ⟂ (Family N lives here)

This is the gap: the *type* is chokepointed (1b), but deciding a constructed
*element's* nullability is threaded TWO ways, and one is incomplete.

| Site | file:line | mechanism | constructed elem (fn-call/ternary) |
|---|---|---|---|
| vector-literal element dispatch | `parser/vectors.rs:382–400` | uses `var_tp` if typed, else `vector_hint`, else clears `expected` | depends on caller (below) |
| inferred-literal **struct-literal PEEK** → `__nullable` | `parser/vectors.rs:1424` | promotes element ONLY if first token is `S{…}` | 🔴 non-literal element stays dense → mismatches the rewritten type |
| inferred-**comprehension** PEEK (twin) | `parser/vectors.rs:1420` | same PEEK for `[expr for …]` | 🔴 same |
| `parse_vector` (element loop) | `parser/vectors.rs:1704` | builds each element against `elem_tp` | ✅ when `elem_tp` is the nullable type |

**Position verdicts** (same literal `[mk_s(1)]`, different `⇐` position):

| Position | file:line | threads type via | verdict |
|---|---|---|---|
| typed local assign `v: vector<S> = […]` | `parser/expressions.rs` (read_target) | `var_tp` | ✅ |
| struct field init `S { f: […] }` | `parser/objects.rs:160,2274` | `var_tp` (field type) | ✅ |
| match/if arm tail `=> […]` | `parser/control.rs:5625,6304` | `expected` pushed | ✅ |
| **block return** `fn -> vector<S> { […] }` | `parser/control.rs:349` | relies on inferred PEEK | 🔴 N |
| **fn argument** `take([…])` | `parser/control.rs:5905` (param loop) | `vector_hint`, but element via PEEK | 🔴 N |

**Routine (Layer 1):** when checking `[e₁…eₙ] ⇐ vector<τ>`, push `τ` into each `eᵢ` and
**check** it (synthesise its type, apply `τ ⤳`), materialising into `Some` when `τ` is
`__nullable`. This routine EXISTS (the `var_tp` path); the bug is it's **bypassed** at
return + argument, which fall back to the struct-literal-only PEEK. Fix = route those
two positions through the same element-checking instead of the PEEK. Collapses ⟂ → ◆.

---

## Layer 2 — CODEGEN: "materialise the constructed value into a slot"

| Site | file:line | role | verdict |
|---|---|---|---|
| `??` dispatch | `parser/operators.rs:1312` | splits `?? return` vs `?? default` | — |
| `build_null_coalesce_default` | `parser/operators.rs:1369` | emits the else-branch value for `a ?? b` | 🔴 **Family A** — a vector-**literal** `b` is not materialised into the result temp (native `else { () }` → E0308; interp slot panic) |
| `build_null_coalesce_return` | `parser/operators.rs:1325` | `a ?? return r` | ✅ (scalar paths) |
| vector-literal element clears `expected` | `parser/vectors.rs:393` | resets checking mode mid-literal | ⟂ contributes to A/N |

**Routine (Layer 2):** the coalesce default (and any constructed value placed into a
typed result temp) must be **materialised into an owned slot of the result type** — the
same "build a value of `τ` into this destination" operation the arms/assign paths use.
A's default-branch bypasses it for a literal. Likely shares the Layer-1 fix locus
(both are "construct value of `τ` here"); the field-map's single-locus hypothesis is
**Layer-1 + Layer-2 together**, NOT Layer 3.

---

## Layer 3 — RUNTIME lifetime/ownership: "transfer the materialised value's store"

Separate family (#462) — do NOT fold into Layers 1–2.

| Site | file:line | role | verdict |
|---|---|---|---|
| `copy_record` / `do_copy_record` | `state/io.rs:1333 / 1344` | deep-copy a record into a dest slot; `0x8000` = free source | 🔴 reads a **freed** source (#462 UAF) |
| `copy_block` / `copy_claims` / `remove_claims` | `store.rs:1716`, `database/structures.rs` | the byte + nested-claim copy | (victims of a wild DbRef) |
| `materialize_return_into` / `materialize_vector_arms_into` | `parser/control.rs:3972 / 1045` | NRVO delivery of a returned aggregate into `__retbuf` | ✅ (post #457/#459) for probed shapes |
| store free + slot reuse | `database/allocation.rs:417,572` | `free` + bitmap reuse | 🔴 premature free → stale-DbRef-after-reuse (#462) |
| dep/ownership model | `OWNERSHIP_MODEL.md` | the invariant *dep = the store a local owns* | the real chokepoint for Layer 3 |

**Routine (Layer 3):** the dep/borrow system (`OWNERSHIP_MODEL.md`) — *a local's dep is
exactly the store it owns; it is freed once, when the last owner dies.* #462 is a hole
in this, surfaced only at slot-reuse scale; needs the `LOFT_UAF` operand-stack/element
extension to pin the premature-free op.

---

## Synthesis — is there one fundamentally-correct routine?

**No single routine — two, cleanly separated, plus a third already-named one:**

1. **Layers 1+2 → ONE routine (the real win):** *"materialise a constructed value of
   type `τ` into a destination, by checking it against `τ`"* — the bidirectional
   checking relation reaching the constructed value (aggregate element AND coalesce
   default). It already exists for typed-assign / field-init / match-arm; it is
   **bypassed** at block-return, fn-argument (Layer 1) and the coalesce literal default
   (Layer 2), which substitute a syntactic PEEK / a missing materialise. Re-route those
   ~3 sites through the existing element-checking and the whole N+A class collapses.
   This is the chokepoint candidate to validate next (design-protocol step 5/6).
2. **Layer 3 → the ownership model** (`OWNERSHIP_MODEL.md`), a *different* relation; #462
   belongs here. Folding it into (1) would be over-unification.

The unifying intuition ("materialisation hole at the nullable boundary") is **true but
spans two relations** — the typing/conversion relation (1) and the ownership relation
(2). The correct generalisation is *"every constructed value crossing into a typed slot
is checked + materialised by the type relation, then owned by the dep relation"* — two
routines that meet at the boundary, not one.

### Re-assertion count (the design-protocol tell)

| Concept | chokepoint exists? | bypassed at | cure |
|---|---|---|---|
| nullable TYPE rewrite | ◆ yes (1b) | — | — |
| element/default VALUE checking (N+A) | partial (`var_tp` path) | return · arg · coalesce-literal | route the 3 through it |
| ownership transfer (#462) | ◆ the dep model | the premature-free path | pin via `LOFT_UAF` ext |

---

## Can we ELIMINATE sites by pushing up into the type system?

Test per site: **is it re-deriving a FACT the type already carries (→ eliminable), or
performing an ACTION (→ only de-duplicable)?**

### Eliminable NOW — the syntactic PEEK (Layer 1c) is pure re-derivation 🟢

The nullable element type is **already a type-system fact**: 1b chokepoints
`vector<S>` → `vector<__nullable<S>>`. So the struct-literal PEEK (`vectors.rs:1424`)
and its comprehension twin (`:1420`) **recompute, from syntax, a fact the declared
type already holds** — and recompute it *wrongly* (literal-only). Push the known
element type into element-checking uniformly and **both PEEK sites DELETE**, the
`var_tp`-vs-PEEK dual path collapses to one read, and the return/arg divergence
vanishes. This is the textbook "complex re-derivation in the front-end = a fact that
should be read, not recomputed." Highest leverage, lowest risk — **Family N is
eliminated, not patched.**

### Eliminable as a SPECIAL CASE — the coalesce default (Layer 2) 🟡

Materialising a value is an *action*, not a fact, so it can't move *into* the type.
But the **special-case** branch (`build_null_coalesce_default`'s literal arm) exists
only because the default isn't typed like any other constructed value. Type the
default as "a value of the result type `τ`" (the same Layer-1 check) and codegen
reuses the normal construct-into-slot path — the special branch **deletes**; the
action survives, shared. So: the *site* goes, the *work* is de-duplicated. Family A
folds into the Layer-1 routine.

### NOT eliminable by a small push — ownership (Layer 3) is the north-star 🔴

#462's per-site dep re-derivations (copy_record free-source, scope frees) ARE
re-derivations — of an **ownership** fact. But ownership is a *different relation*
from typing; "push into the type system" here means **make the dep/borrow relation a
sound, static, type-level fact** (the `OWNERSHIP_MODEL.md` north star: "every
store-lifetime codegen decision derives mechanically from a sound ownership system").
That eliminates the class by construction — but it's the major @PLN85 arc, not a
push. Folding ownership into the nullable-typing relation would be over-unification.

### The biggest lever — the DEFAULT itself (a semantics decision) ⚠️

Every materialisation site exists because **`vector<S>` defaults to nullable
elements** (1b, "default = nullable"). If nullability were **explicit in the type**
(dense by default, opt-IN `?`), the entire materialise-wrap machinery would be a
**no-op for the common case** and fire only where the type says so — shrinking the
surface, not just relocating it. This is the strongest "push it into the type
system": let the declaration carry nullability so downstream never wraps by default.
But it is a **language-semantics decision** with an active tension — `e2_rewrite_enabled`
notes default-on E2 breaks ~107 tree-wide tests because the *access* glue is
incomplete. So this lever is real and large; it belongs to the @PLN25 default-on
decision, not this cluster.

### Verdict

| Site / family | push up → | eliminable? |
|---|---|---|
| PEEK element-nullability (N) | read the type 1b already carries | 🟢 **yes — delete 2 sites now** |
| coalesce literal default (A) | type the default uniformly | 🟡 special-case deletes; action shared |
| dep re-derivations (#462) | sound ownership type (north star) | 🔴 only via the big arc |
| materialise burden everywhere | explicit/dense-default nullability | ⚠️ yes, but a semantics decision |

**Net:** pushing up genuinely *removes* Family N (a re-derived fact) and *de-special-cases*
Family A — that is the concrete win available now and it is a real subtraction, not a
relocation. #462 and the default-nullability lever are larger, separate arcs.
