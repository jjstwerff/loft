
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
- [9. Text/Character Split: Indexing and Slicing Return Different Types](#9-textcharacter-split-indexing-and-slicing-return-different-types)
- [12. Index Range-Query Second-Key Semantics Depend on Sort Direction](#12-index-range-query-second-key-semantics-depend-on-sort-direction)
- [17. Implicit Type Coercion Rules Are Not Uniform](#17-implicit-type-coercion-rules-are-not-uniform)
- [18. `#break` Reuses the `#attribute` Syntax for a Control-Flow Statement](#18-break-reuses-the-attribute-syntax-for-a-control-flow-statement)
- [26. Match Exhaustiveness Ignores Guarded Arms](#26-match-exhaustiveness-ignores-guarded-arms)
- [27. `break` Keyword and `x#break` Attribute Are Two Mechanisms for the Same Action](#27-break-keyword-and-xbreak-attribute-are-two-mechanisms-for-the-same-action)
- [28. Vector Slice Syntax Has No Grammar Entry and Diverges From Range Syntax](#28-vector-slice-syntax-has-no-grammar-entry-and-diverges-from-range-syntax)
- [30. `{...}` Is Both Anonymous Struct Initialisation and a Block Expression](#30--is-both-anonymous-struct-initialisation-and-a-block-expression)
- [31. Open-Ended Range Syntax in `for` Has No Documented Counterpart in `match`](#31-open-ended-range-syntax-in-for-has-no-documented-counterpart-in-match)
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

---

## 9. Text/Character Split: Indexing and Slicing Return Different Types

**Severity: Low**

```loft
txt = "hello"
txt[0]      // character — a single Unicode scalar value
txt[0..1]   // text — a one-character string

vec = [1, 2, 3]
vec[0]      // integer
vec[0..1]   // vector<integer> — consistent: element type vs. collection type
```

`txt[i]` and `txt[i..i+1]` are different types (`character` vs. `text`), making string
manipulation awkward: building a text from characters requires `"{c}"` interpolation, not
direct concatenation with `+`. The vector pattern (element vs. slice of same collection
type) would be cleaner.

---

## 12. Index Range-Query Second-Key Semantics Depend on Sort Direction

**Severity: Medium**

```loft
struct Elm { nr: integer, key: text, value: integer }
struct Db { map: index<Elm[nr, -key]> }   // key is DESCENDING

for v in db.map[83..92, "Two"] { }
// Means: nr ∈ [83, 92) AND key from "Two" going DOWNWARD
// because the key field is declared descending
```

The second position in a range query is not a range — it is a boundary in the sort
direction of that field. If the field is ascending, `"Two"` means "from Two upward"; if
descending it means "from Two downward". The sort direction is declared at the struct
definition, which may be far from the query. This makes range queries hard to reason about
without constantly checking the index declaration.

---

## 17. Implicit Type Coercion Rules Are Not Uniform

**Severity: Low**

| Conversion | Form required |
|---|---|
| Any type → boolean (`if v`, `!v`, `assert`) | Implicit |
| Integer ↔ float in arithmetic | Implicit widening |
| Float → integer (truncate) | Explicit: `f as integer` |
| Text → integer/float | Explicit: `"5" as integer` |
| Integer → text | Implicit inside `"{v}"` only |
| Plain-enum name → enum | Explicit: `"West" as Direction` |
| Struct-enum variant → parent enum | Implicit on assignment |

There is no single rule. Boolean coercion is always implicit; most numeric conversions
require `as`; format-string interpolation converts silently. A user cannot predict from
first principles whether a given conversion is automatic or requires an explicit cast.

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

There is also an asymmetry: `x#break` has no `x#continue` counterpart, so skipping the
remainder of a named outer loop requires code restructuring.

**Advice:** Consider replacing `x#break` and introducing `break x` / `continue x` as
labelled-break forms, consistent with how other languages handle named loop exits
(`break 'label` in Rust, `break label` in Java). The existing `#` notation could be
kept for read-only loop metadata (`#first`, `#count`, `#index`) and `#remove`, which
are genuine attribute reads.

---

## 28. Vector Slice Syntax Has No Grammar Entry and Diverges From Range Syntax

**Severity: Low**

```loft
v[start..end]   // slice — end exclusive (matches for-loop range)
v[start..]      // open end  (also valid in for-loop)
v[..end]        // open start — NO for-loop counterpart
v[2..-1]        // negative index — NO for-loop counterpart
v[1..=3]        // inclusive end — valid in for-loop and match; undocumented for slices
```

The grammar summary defines `range_expr` for `for` loops and `match` arms but does not
include the slice forms `[..end]` and `[n..-1]`. Users cannot tell from the grammar
whether `v[1..=3]` (inclusive slice) is supported.

**Advice:** Add a `slice_expr` production to the grammar summary that enumerates all
valid slice forms and documents which are shared with `range_expr`. Clarify whether
`..=` is supported in slices.

---

## 30. `{...}` Is Both Anonymous Struct Initialisation and a Block Expression

**Severity: Low**

```loft
point = { x: 1.0, y: 2.0 }     // anonymous struct init (type inferred from context)
result = { compute(); value }   // block expression returning last value
```

The opening `{` alone does not indicate which form is being used. The parser resolves
the ambiguity by looking ahead for `ident ':'` (struct field assignment). A typo such
as `{ x, y }` (missing colons) silently becomes a block expression that evaluates `x`
and `y` as separate statements and returns `y`.

**Advice:** Consider requiring an explicit type name for anonymous struct init in
contexts where a block expression is also valid, e.g. `Point { x: 1.0, y: 2.0 }`.
Alternatively, document the lookahead rule prominently in the grammar summary so users
know what to expect when `{` is ambiguous.

---

## 31. Open-Ended Range Syntax in `for` Has No Documented Counterpart in `match`

**Severity: Low**

```loft
for i in 10.. { }          // valid — open-ended range (iterate from 10 upward)

match score {
    90..=100 => "A",        // valid two-sided inclusive range
    80..90   => "B",        // valid two-sided exclusive range
    10..     => "passing",  // undocumented — is this valid?
    ..80     => "failing",  // undocumented — is this valid?
    _        => "other"
}
```

The grammar defines `range_expr` with an open-end form (`expr '..'`) for `for` loops,
but the `pattern` production only lists `range` without specifying whether open-ended
forms are allowed in `match` arms. Users writing match arms for "90 or above" must use
`90..=i32::MAX` or a guard (`n if n >= 90`) instead of `90..`.

**Advice:** Decide whether open-ended range patterns in `match` are supported and
document the answer explicitly in the grammar. If not supported, document the
`n if n >= threshold` idiom as the canonical alternative.

---

## Summary by Severity

### High (silent wrong behaviour)
_All fixed — see CHANGELOG.md._

### Medium (surprising but safe)
| # | Issue |
|---|---|
| 12 | Index range-query second-key boundary depends on undeclared sort direction |
| 26 | Match exhaustiveness ignores guarded arms — wildcard still required |
| 27 | `break` keyword and `x#break` attribute are two mechanisms for the same action; no `x#continue` |

### Low (cosmetic or minor)
| # | Issue |
|---|---|
| 2 | `#first`/`#index`/`#remove` availability varies by collection type |
| 8 | Method vs. free function assignment is arbitrary in the standard library |
| 9 | `txt[i]` is `character`; `txt[i..i+1]` is `text` — different types |
| 17 | Type coercion rules are not uniform (implicit / explicit / format-only) |
| 18 | `x#break` is a jump statement, reusing the `#attribute` expression syntax |
| 28 | Vector slice forms `[..end]` and `[n..-1]` absent from grammar; `..=` undocumented for slices |
| 30 | `{...}` is both anonymous struct init and block expression; typos silently become blocks |
| 31 | Open-ended range `10..` is valid in `for`; not documented for `match` arms |

### Resolved as design point (documented + regression-guarded)

These were inconsistencies whose semantics are now (a) explicitly documented in
LOFT.md as a "Gotcha" or asymmetry callout and (b) locked by regression tests
in `tests/issues.rs`.  They remain inconsistencies — but they're acknowledged
ones, not silent surprises.  Removed from the severity tables above.

| # | Issue | Doc + Tests |
|---|---|---|
| 3 | `#index` byte-offset on text vs. element-position on vector | LOFT.md § Loop attributes (Gotcha block); `inc3_*` regression tests |
| 29 | `!b` on boolean catches false and null; `!n` on integer catches null only | LOFT.md null-sentinel table (`!value` asymmetry subsection); `inc29_*` regression tests |

---

## See also
- [PROBLEMS.md](PROBLEMS.md) — Known bugs, limitations, workarounds, and fix plans
- [PLANNING.md](PLANNING.md) — Priority-ordered enhancement backlog
