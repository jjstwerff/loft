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
- [3. Loop Attribute `#index` Has Different Semantics on Text vs. Vector](#3-loop-attribute-index-has-different-semantics-on-text-vs-vector)
- [8. Method vs. Free Function Is an Arbitrary Standard-Library Choice](#8-method-vs-free-function-is-an-arbitrary-standard-library-choice)
- [9. Text/Character Split: Indexing and Slicing Return Different Types](#9-textcharacter-split-indexing-and-slicing-return-different-types)
- [12. Index Range-Query Second-Key Semantics Depend on Sort Direction](#12-index-range-query-second-key-semantics-depend-on-sort-direction)
- [17. Implicit Type Coercion Rules Are Not Uniform](#17-implicit-type-coercion-rules-are-not-uniform)
- [18. `#break` Reuses the `#attribute` Syntax for a Control-Flow Statement](#18-break-reuses-the-attribute-syntax-for-a-control-flow-statement)
- [26. Match Exhaustiveness Ignores Guarded Arms](#26-match-exhaustiveness-ignores-guarded-arms)
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

## 3. Loop Attribute `#index` Has Different Semantics on Text vs. Vector

**Severity: Medium** (also [PROBLEMS.md](PROBLEMS.md) #23)

```loft
txt = "12😊🙃45"
for c in txt {
    c#index   // UTF-8 byte offset of the START of this character (pre-advance)
    c#next    // byte offset AFTER this character — where the next character begins
    c#count   // 0-based character position (counts whole characters)
}

for v in vec {
    v#index   // 0-based element position (counts whole elements)
    v#count   // same as v#index for vectors
    // no v#next
}
```

Both use `#index` but the semantics differ: on text it is a **UTF-8 byte offset**
(useful for slicing `txt[c#index..c#next]`), on vectors it is an **element position**.
Use `c#count` for a 0-based character count that matches vector `v#index` semantics.

Note: the text `c#index` value equals `c#count` only for ASCII text (one byte per
character). For multi-byte characters (emoji, CJK, accented letters), the byte offset
advances by 2–4 per character.

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
exhaustiveness checking (`control.rs:582`). This is correct — the guard might fail at
runtime — but it means a programmer who writes guards on every variant still needs a
wildcard arm or will get a non-exhaustive error. The interaction between guards and
exhaustiveness is not obvious from the syntax.

---

## Summary by Severity

### High (silent wrong behaviour)
_All fixed — see CHANGELOG.md._

### Medium (surprising but safe)
| # | Issue |
|---|---|
| 3 | `#index` is byte-offset on text, element-position on vector |
| 12 | Index range-query second-key boundary depends on undeclared sort direction |
| 26 | Match exhaustiveness ignores guarded arms — wildcard still required |

### Low (cosmetic or minor)
| # | Issue |
|---|---|
| 2 | `#first`/`#index`/`#remove` availability varies by collection type |
| 8 | Method vs. free function assignment is arbitrary in the standard library |
| 9 | `txt[i]` is `character`; `txt[i..i+1]` is `text` — different types |
| 17 | Type coercion rules are not uniform (implicit / explicit / format-only) |
| 18 | `x#break` is a jump statement, reusing the `#attribute` expression syntax |

---

## See also
- [PROBLEMS.md](PROBLEMS.md) — Known bugs, limitations, workarounds, and fix plans
- [PLANNING.md](PLANNING.md) — Priority-ordered enhancement backlog
