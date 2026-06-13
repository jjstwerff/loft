
# Loft Language Design Inconsistencies

This document catalogues asymmetries, surprising behaviours, and design tensions in the
loft language. It focuses on things that *work* but feel inconsistent — bugs and crashes
belong in [PROBLEMS.md](PROBLEMS.md) instead.

Each entry notes **Severity**: High = silent wrong behaviour; Medium = surprising but safe;
Low = cosmetic or minor. Where a path to resolution is obvious it is included.

Fixed items have been removed from this file; their resolutions are in CHANGELOG.md.

---

## Contents

- [2. Vector Has a Much Richer API Than Sorted / Index / Hash](#2-vector-has-a-much-richer-api-than-sorted--index--hash)
- [8. Method vs. Free Function Is an Arbitrary Standard-Library Choice](#8-method-vs-free-function-is-an-arbitrary-standard-library-choice)
- [18. `#break` Reuses the `#attribute` Syntax for a Control-Flow Statement](#18-break-reuses-the-attribute-syntax-for-a-control-flow-statement)
- [27. `break` Keyword and `x#break` Attribute Are Two Mechanisms for the Same Action](#27-break-keyword-and-xbreak-attribute-are-two-mechanisms-for-the-same-action)
- [33. `const` Applies to Locals and Parameters but Not Fields](#33-const-applies-to-locals-and-parameters-but-not-fields)
- [34. Definitions Share One Flat Namespace — Enum Variants Collide by Bare Name](#34-definitions-share-one-flat-namespace--enum-variants-collide-by-bare-name)
- [Summary by Severity](#summary-by-severity)

---

## 2. Vector Has a Much Richer API Than Sorted / Index / Hash

**Severity: Medium** — partially resolved 2026-03-14 (loop attributes documented/implemented)

All four collection types use the same `+=` and `for` syntax.  The iteration toolkit is:

| Feature | `vector` | `sorted` | `index` | `hash` |
|---|---|---|---|---|
| `#first`, `#count` in loop | ✓ | ✓ | ✓ | N/A (cannot iterate) |
| `#index` in loop | ✓ (0-based) | ✓ (0-based array pos) | ✗ compile error | N/A |
| `e#remove` in filtered loop | ✓ | ✓ | ✓ | use `h[key] = null` |
| `rev()` reverse iteration | ✓ (via range) | ✓ | ✓ | N/A |
| Slicing `[a..b]` | ✓ | ✗ | ✗ | ✗ |
| Comprehension `[for x in ...]` | ✓ | ✗ | ✗ | ✗ |
| Filtered `for x in c if cond` | ✓ | ✓ | ✓ | N/A |

Remaining API gaps (slicing, comprehension) are structural and not planned.

**Status (2026-04-13):** Documented in
[LOFT.md § Key-based collections](LOFT.md) under a "Gotcha (INC#2)"
paragraph covering the comprehension gap, the `#index`-on-index compile
error, and the shared `for` / `+=` / subscript-removal contract that
*does* cross collection types.  Two regression guards in
`tests/issues.rs` lock the positive baseline for both halves:
`inc02_vector_comprehension_works` (vector comprehension with an `if`
filter) and `inc02_sorted_is_iterable` (sorted shares the `for` API, so
ports from vector keep working).  The remaining API gaps are an
acknowledged structural choice, not a silent surprise.

---

## 8. Method vs. Free Function Is an Arbitrary Standard-Library Choice

**Severity: Low**

There is no language-level rule about what becomes a method vs. a free function; it
depends entirely on whether the standard library defines the first parameter as `self`.

```loft
length(v)           // free function — NOT v.length()
length(text)        // same free function name for both text and vector

text.starts_with(s) // method — defined as fn starts_with(self: text, ...)
text.find(s)        // method
abs(n)              // free function — NOT n.abs()
pow(b, e)           // free function
```

A user cannot predict whether an operation is a method or a free function without looking
it up. Some text operations are methods (`starts_with`, `find`, `trim`) while the most
basic one (`length`) is a free function. The language allows both forms equally; the
inconsistency is in the standard-library naming choices.

**Status (2026-04-13):** Documented in
[LOFT.md § Methods and function calls](LOFT.md) under a "Gotcha (INC#8)"
paragraph with current stdlib examples covering all three call-form classes
(method-only `self:`, dual `both:`, free-only).  Three regression guards in
`tests/issues.rs` lock the behaviour across the classes:
`inc08_starts_with_is_method_not_free_function` (method-only works via dot
syntax), `inc08_sum_of_is_free_function_only` (method syntax on a free-only
function produces an "Unknown field" compile error), and
`inc08_len_with_both_works_either_way` (a `both:` first-parameter registers
both forms — the escape hatch for authors who want the asymmetry to vanish).
The asymmetry is an acknowledged stdlib naming choice, not a language bug.

---


## 18. `#break` Reuses the `#attribute` Syntax for a Control-Flow Statement

**Severity: Low**

```loft
for x in 1..5 {
    for y in 1..5 {
        if cond { x#break; }   // breaks out of the x loop
    }
}
```

The `#` suffix notation is used for two completely different purposes:
- **Loop metadata** — `x#first`, `x#count`, `x#index`, `x#next`, `x#remove`: read or
  write properties that are expressions or assignment targets.
- **Loop control** — `x#break`: a statement that transfers control and has no value.

Making `x#attr` sometimes an expression and sometimes a jump instruction complicates the
mental model for the `#` notation.

**Status (2026-04-13):** Documented in
[LOFT.md § Break and continue](LOFT.md) under a "Labelled break — `loop_var#break`"
paragraph with a worked nested-loop example and an explicit gotcha callout
(no `x#continue` counterpart, not a value expression).  Two regression guards
in `tests/issues.rs` lock the behaviour:
`inc18_labelled_break_exits_outer_loop` (confirms `x#break` from the inner loop
terminates the outer `x` loop, not just the innermost), and
`inc18_bare_break_exits_innermost_only` (pairs as the control — a bare `break`
only exits the enclosing inner loop).  The two-mechanism design is an
acknowledged ergonomics asymmetry rather than a silent surprise.

---

## 26. Match Exhaustiveness Ignores Guarded Arms

**Severity: Medium**

```loft
match c {
    Red if some_cond => "red",     // does NOT count as covering Red
    Green => "green",
    Blue => "blue",
    _ => "fallback"                // still required even though Red has an arm
}
```

A guarded arm (`pattern if guard => body`) does not count as covering that variant for
exhaustiveness checking (`control.rs:642`). This is correct — the guard might fail at
runtime — but it means a programmer who writes guards on every variant still needs a
wildcard arm or will get a non-exhaustive error. The interaction between guards and
exhaustiveness is not obvious from the syntax.

**Status (2026-04-13):** Documented in
[LOFT.md § Pattern matching](LOFT.md) under the "Guard clauses"
paragraph with a worked example (Red-if-bright / Green-if-bright / Blue / `_`).
Three regression guards in `tests/issues.rs` lock the behaviour:
`inc26_guarded_arm_without_wildcard_is_rejected` (asserts the compile error
wording, including the `'_ =>' wildcard` fix-it hint),
`inc26_guarded_arm_with_wildcard_compiles` (compiles + runs when the wildcard
is present), and `inc26_guarded_arm_falls_through_when_guard_false` (runtime
fall-through to a subsequent unguarded arm).  The wildcard requirement is an
acknowledged soundness property, not a silent surprise.

---

## 27. `break` Keyword and `x#break` Attribute Are Two Mechanisms for the Same Action

**Severity: Medium**

```loft
break           // exits the innermost loop — keyword
x#break         // exits the loop whose variable is x — attribute expression
```

`break` and `continue` are reserved keywords that appear as statements. `x#break` uses
the `#attribute` syntax on a loop variable to jump out of a named loop. The two forms
look unrelated: a reader encountering `x#break` would guess it reads a property named
`break` from `x`, not that it is a control-flow jump.

(Correction 2026-04-13: `x#continue` **does** exist and works as a
labelled-continue — initial writeup was based on a test that couldn't
distinguish bare- from labelled-continue semantics.)

**Advice:** Consider replacing `x#break` and introducing `break x` / `continue x` as
labelled-break forms, consistent with how other languages handle named loop exits
(`break 'label` in Rust, `break label` in Java). The existing `#` notation could be
kept for read-only loop metadata (`#first`, `#count`, `#index`) and `#remove`, which
are genuine attribute reads.

**Status (2026-04-13):** Documented in
[LOFT.md § Break and continue](LOFT.md) — the feature is implemented:
`x#continue` is a true labelled-continue symmetric to `x#break`.
Regression guard `inc27_x_continue_is_labelled_continue` uses an
outer-body operation (inner-vs-outer counter pair) that distinguishes
bare- from labelled-continue and pins the labelled result (106).
Earlier writeup here claimed silent-miscompile behaviour; that was a
misreading from a reproducer whose sum happened to be the same under
both semantics.

---

## 33. `const` Applies to Locals and Parameters but Not Fields

**Severity: Low**

After [@P246](PROBLEMS.md) closed (file-scope `const NAME = expr;`
accepted) and the UPPER_CASE-non-const warning landed, loft's
immutability story is uniform at every scope **except** struct
fields:

| Where | Immutable form |
|---|---|
| File-scope constant | `const PI = 3.14;` (or bare-name `PI = 3.14;`) |
| Local variable | `const x = 5;` |
| Function parameter | `fn f(const x: T)` |
| **Struct field** | **(no equivalent — every field is implicitly mutable)** |

The asymmetry is the only place where "I want this thing
immutable" can't be expressed.  The first concrete consumer is
[lib/hex_world's `Cell`](../../lib/hex_world/src/hex_world.loft):
`c_color`, `c_height`, `c_age` should ALL be const after
construction (the tick loop rebuilds the entire cell via
`chunk.ck_cells[idx] = Cell{…}`, never via in-place field
writes), but loft can't express the constraint, so a future
contributor writing `cell.c_color = 0;` gets no compiler help.

**Advice:** Extend `const` to struct fields as a write-once-at-
construction modifier.  Purely static check, zero runtime cost,
no schema change.  Design + sequencing in
[plans/future/33-const-fields/README.md](plans/future/33-const-fields/README.md).

**Plan (2026-05-11):** Documented as @PLAN33 (future).  Not
yet implemented; the gap is currently silent (the parser
accepts mutable fields with names that look like they should be
const).  No regression guard yet — none possible until the
feature is implemented.  (Other INC entries use a `Status`
marker that means "resolved as design point" — INC#33 is
genuinely open, so a `Plan` marker is used here instead.  The
`doc_hygiene::inconsistencies_status_blocks_listed_as_resolved`
test scans for the resolved marker; the open-status `Plan`
form intentionally falls outside its match.)

---

## 34. Definitions Share One Flat Namespace — Enum Variants Collide by Bare Name

**Severity: Medium**

`Data::add_def` keys every definition by `(name, source)` (`src/data.rs:3001`),
so enum variants live in one flat per-source namespace. Consequences:

- **Two enums cannot share a variant name** — `enum A { Red }` + `enum B { Red }`
  *panics* at parse time: `Dual definition of Red` (`src/data.rs:3003`).
- **A user type collides with stdlib names** — `enum E { … }` is rejected because
  `E` is the stdlib Euler constant ("pick a different name").

Resolution is *already* context-aware — a bare variant resolves against the
contextual enum in match arms / typed locals / comparisons / fn args, and
`Enum::Variant` / `lib::Variant` qualify — but **definition is flat**, so both the
qualified path (`parse_constant_value`) and the bare value-position paths
ultimately depend on the global bare key. (Verified: de-globalizing variants flips
`Light::Red == Light::Red` from `true` to `false`, because the qualified path also
reads the global key.)

**Status / fix:** being fixed by
[@PLN22](plans/22-namespaces/README.md) — variants become `(enum, variant)`-keyed
(members of their enum), resolved through one `variant_of(enum, name)` chokepoint,
with `use … as …` aliasing + a shadowable prelude for the top-level cases. Phase 1
(variant scoping) design + chokepoint-first build order validated; build pending.

---


## Summary by Severity

### High (silent wrong behaviour)
_All fixed — see CHANGELOG.md._

### Medium (surprising but safe)
_All documented + regression-guarded — see the Resolved-as-design-point table below._

### Low (cosmetic or minor)
- [33. `const` Applies to Locals and Parameters but Not Fields](#33-const-applies-to-locals-and-parameters-but-not-fields) — open; resolution path in [plans/future/33-const-fields/](plans/future/33-const-fields/README.md)
_Other Low items: documented + regression-guarded — see the Resolved-as-design-point table below._

### Resolved as design point (documented + regression-guarded)

These were inconsistencies whose semantics are now (a) explicitly documented in
LOFT.md as a "Gotcha" or asymmetry callout and (b) locked by regression tests
in `tests/issues.rs`.  They remain inconsistencies — but they're acknowledged
ones, not silent surprises.  Removed from the severity tables above.

| # | Issue | Doc + Tests |
|---|---|---|
| 2 | Vector has comprehensions; sorted / index / hash do not, and `#index` is invalid on index collections | LOFT.md § Key-based collections (Gotcha block); `inc02_vector_comprehension_works`, `inc02_sorted_is_iterable` |
| 3 | `#index` byte-offset on text vs. element-position on vector | LOFT.md § Loop attributes (Gotcha block); `inc3_*` regression tests |
| 8 | Method vs. free function is the stdlib author's per-function choice (`self:` / `both:` / free-only) | LOFT.md § Methods and function calls (Gotcha block); `inc08_starts_with_is_method_not_free_function`, `inc08_sum_of_is_free_function_only`, `inc08_len_with_both_works_either_way` |
| 9 | `txt[i]` returns `character`, `txt[i..j]` returns `text` — deliberate asymmetry (character is a distinct scalar, not a length-1 text); LOFT.md § String literals carries a Gotcha callout with concat rules + the B7-family SIGSEGV caveat | `inc9_text_index_returns_character`, `inc9_text_slice_returns_text`, `inc9_text_slices_concatenate_with_plus`, `inc9_character_plus_is_arithmetic_not_concat` |
| 12 | Sort direction declared on struct drives iteration direction of every query | LOFT.md § Collection types (Gotcha block); `inc12_sorted_ascending_*` / `inc12_sorted_descending_*` regression tests |
| 18 | `x#break` is a jump statement, reusing the `#attribute` expression syntax | LOFT.md § Break and continue (Labelled break + Gotcha block); `inc18_labelled_break_exits_outer_loop`, `inc18_bare_break_exits_innermost_only` |
| 17 | Type-conversion rules stratified into implicit / format-only / explicit modes, mode driven by type pair not context.  LOFT.md § The `as` operator now carries a "Type-conversion rules" table covering 11 pairs with a rule-of-thumb: fallible conversions explicit, infallible implicit, format-interpolation is its own mode | `inc17_any_to_boolean_is_implicit`, `inc17_integer_widens_to_float_in_arithmetic`, `inc17_float_to_integer_requires_as`, `inc17_text_to_integer_requires_as`, `inc17_integer_to_text_is_format_only`, `inc17_plain_enum_name_to_enum_requires_as` |
| 26 | Match exhaustiveness ignores guarded arms — wildcard still required | LOFT.md § Pattern matching (Guard clauses paragraph); `inc26_*` regression tests |
| 27 | No `x#continue` counterpart to `x#break`; `x#continue` silently parses as a bare inner `continue` | LOFT.md § Break and continue (Gotcha block paired with INC#18); `inc27_x_continue_behaves_as_bare_continue` |
| 29 | `!b` on boolean catches false and null; `!n` on integer catches null only | LOFT.md null-sentinel table (`!value` asymmetry subsection); `inc29_*` regression tests |
| 30 | `{...}` double-duty (struct init vs. block) — claimed silent-typo case is not reproducible on current loft; the `{ x, y }` typo parses as a struct-init attempt and fails on the missing colon | `inc30_struct_init_with_colons_works`, `inc30_block_expression_returns_last_value`, `inc30_typo_comma_without_colon_is_rejected` |
| 28 | Vector slice grammar — inclusive form `v[start..=end]` works but was undocumented; `v[2..-1]` was documented as "negative-index from end" but the form has always produced an empty iterator, never "all but the last element".  LOFT.md § Vectors now documents the four supported forms + an explicit note that negative indexing isn't implemented | `inc28_slice_exclusive_range`, `inc28_slice_inclusive_range`, `inc28_slice_open_end`, `inc28_slice_open_start`, `inc28_negative_index_in_slice_yields_empty` |
| 31 | Open-ended range patterns (`10..`, `..10`) in match arms were silently broken (interpreter: never matches; native: rustc crash).  Parser now emits a compile-time diagnostic pointing at the two-sided form or a guard idiom | `inc31_two_sided_exclusive_range_matches`, `inc31_two_sided_inclusive_range_matches`, `inc31_open_end_range_is_rejected`, `inc31_open_start_range_is_rejected` |

---

## See also
- [PROBLEMS.md](PROBLEMS.md) — Known bugs, limitations, workarounds, and fix plans
- [PLANNING.md](PLANNING.md) — Priority-ordered enhancement backlog
