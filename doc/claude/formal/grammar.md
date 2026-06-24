<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# formal/grammar.md — concrete grammar & operator precedence (strict)

> **Rules then deviations** (see [README](README.md)). This area pins what the informal
> grammar in [LOFT.md § Summary of grammar](../LOFT.md) leaves out — **operator
> precedence and associativity** — and records where the surface is *not* context-free.
> Rough spot #4 from [FORMALIZATION.md](../FORMALIZATION.md), which the runtime-scoped
> red-flag map structurally misses.
>
> Ground truth: the precedence table `OPERATORS` (`src/parser/mod.rs:375`) and the
> precedence-climbing walk `parse_operators` (`src/parser/operators.rs:446`). The full
> statement/expression shapes stay in [LOFT.md](../LOFT.md); this doc adds the one fact
> that lives *only* in the parser.

## Notation

- A binary operator **binds tighter** than another if it groups first: in `a + b * c`,
  `*` binds tighter, so it reads `a + (b * c)`.
- **Left-associative**: equal-precedence operators group left-to-right — `a - b - c` is
  `(a - b) - c`.

---

## Rules

### Operator precedence — twelve levels, loosest first (the source order)

```
  level   operators                          (each row binds TIGHTER than the one above)
  ──────  ─────────────────────────────────
   0      ??                                 ← loosest (groups last / outermost)
   1      ||   or
   2      &&   and
   3      ==   !=   <   <=   >   >=
   4      |                                  (bitwise or)
   5      ^                                  (bitwise xor)
   6      &                                  (bitwise AND — infix `&`, see below)
   7      <<   >>                            (shifts)
   8      +    -                             (additive)
   9      *    /    %                        (multiplicative)
  10      **                                 (power — RIGHT-associative; all others left)
  11      as                                 ← tightest (cast; groups first / innermost)
```

```
  (G-Prec)   for binary operators, level N binds tighter than level N-1.  Parsing is
             precedence-climbing: `parse_operators(p)` parses operands at level p+1, so a
             higher level groups deeper.
  (G-Assoc)  binary levels are LEFT-associative, EXCEPT `**` (power), which is RIGHT-assoc:
                 2 ** 3 ** 2  ==  2 ** (3 ** 2)  ==  512       (matches maths / most languages)
                 x as integer as float  ==  (x as integer) as float   (`as` stays left-assoc)
```

**In words.** loft has one precedence ladder, twelve rungs. The closer to the bottom
(`as`, `**`, `*`), the tighter an operator grips its operands; the closer to the top
(`??`, `||`), the later it groups. Operators group left-to-right, with **one exception**:
`**` (power) is **right**-associative, so `2 ** 3 ** 2` is `2 ** (3 ** 2) = 512` — matching
maths and most languages (it was left-associative = 64 before; the maker should not carry a
surprise there).

### Prefix `&` is NOT an operator — it is the reference annotation

```
  (G-Amp-Infix)   an INFIX `&` is the bitwise-AND operator (level 6): `a & b`.
  (G-Amp-Prefix)  a PREFIX `&` is the reference-type annotation, parsed in the primary
                  expression — NOT a unary operator.  `&a` at a binding makes the bound
                  variable a link (see [binding.md](binding.md)); a prefix `&` anywhere
                  else is a parse error (binding.md B-Ref-AnnotationOnly).
  (G-Amp-Disambig) the two are told apart by POSITION: a `&` with a left operand is
                  infix bitwise-and; a leading `&` is the reference annotation.
```

**In words.** `&` does double duty. Between two values (`a & b`) it is bitwise-and, an
ordinary operator at level 6 (looser than `+`, so `1 + 2 & 3` is `(1+2) & 3`). At the
*front* of a binding (`b = &a`) it is the reference annotation from `binding.md`, and it
is the only place a leading `&` is allowed. The parser disambiguates purely by position.

---

## Deviations

OPEN: **2** — (was 4; D-gram-3 `**`-associativity + D-gram-1 precedence-doc both closed)

> **D-gram-1 (CLOSED this cycle) — the written grammar now states precedence + associativity.**
> [LOFT.md § Operators](../LOFT.md#operators) carries the full twelve-level ladder (the stale
> table is fixed: `**` added at level 10, `as` moved to 11) with an explicit associativity
> statement (all left-assoc except `**` right-assoc) and the unary-binds-tightest note
> (`-2 ** 2 == 4`). [LOFT.md § Summary of grammar](../LOFT.md) no longer collapses operators
> into one undefined `op`: it enumerates `binary_op` and cross-references § Operators for the
> grouping. The "read-the-parser tax" (FORMALIZATION.md rough spot #4) is paid down — the two
> user-facing statements pin every expression's shape, matching `OPERATORS` / `parse_operators`.

### D-gram-2 — the surface is not context-free
- **Violates:** the implicit goal that the grammar is a context-free spec
- **Where:** speculative backtracking in the parser — type-vs-variable, struct-init
  (`S { … }`) vs block (`{ … }`) — plus the lexer's Formatting mode for string
  interpolation (`"{e}"`). The token stream's meaning depends on parse context.
- **Effect:** there is no context-free grammar that accepts exactly loft; a generated
  parser / external tool can't be derived from a CFG.
- **Status:** OPEN — a known, accepted shape today; recorded so a formal grammar attempt
  knows the boundary.
- **Removal:** (long-horizon) a grammar whose ambiguities are resolved without unbounded
  backtracking, or an explicit statement of the context-sensitive productions.

### D-gram-4 — `&` is overloaded (bitwise-and vs reference annotation)
- **Violates:** the clean separation G-Amp-* relies on position alone
- **Where:** infix `&` (bitwise-and, level 6) and prefix `&` (reference annotation) share
  one token; disambiguation is positional.
- **Effect:** the overload couples this grammar to binding.md: enforcing "prefix `&` only
  at a binding" is a grammar obligation, not just a binding one.
- **Status:** OPEN but **its removal condition is now MET** — binding.md D-bind-0..D-bind-7
  are all closed: a prefix `&` is rejected in every non-binding position (the last, the bare
  statement, closed by A1 this cycle), so it can never reach an expression slot. This is now
  the roadmap **B3 decision**: reclassify D-gram-4 as a *decided edge* (an accepted positional
  overload, like Rust's `&`) rather than a deviation — the residual the roadmap said to
  "fold/close once A1 lands."

---

## Conformance

The precedence rules are checkable by evaluation: `1 + 2 & 3 == 3` (additive tighter than
bitwise-and), `2 ** 3 ** 2 == 512` (RIGHT-assoc `**`; guarded by
`tests/issues.rs::power_is_right_associative`), `x as integer as float` (left-assoc `as`).
D-gram-2's falsifier is structural (no CFG accepts loft); D-gram-4's is binding.md's `[&a]` →
internal-error case.
