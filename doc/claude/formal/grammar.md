<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# formal/grammar.md — concrete grammar & operator precedence (strict)

**Catalogue:** @I58 (parser), @F37 (operator precedence). Context-sensitive points: @F35 (string interpolation), @F12 (struct literals).

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
   3      ==   !=   <   <=   >   >=          (comparison — NON-associative; no chaining)
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
  (G-Assoc)  binary levels are LEFT-associative, EXCEPT `**` (power = RIGHT-assoc) and the
             comparison level 3 (NON-associative — chaining is rejected):
                 2 ** 3 ** 2  ==  2 ** (3 ** 2)  ==  512       (matches maths / most languages)
                 x as integer as float  ==  (x as integer) as float   (`as` stays left-assoc)
                 a == b == c   → COMPILE ERROR (would be `(a == b) == c`, a bool vs c compare;
                                parenthesise, or use `&&` for a range: `1 < x && x < 10`)
```

**In words.** loft has one precedence ladder, twelve rungs. The closer to the bottom
(`as`, `**`, `*`), the tighter an operator grips its operands; the closer to the top
(`??`, `||`), the later it groups. Operators group left-to-right, with **two exceptions**:
`**` (power) is **right**-associative, so `2 ** 3 ** 2` is `2 ** (3 ** 2) = 512` — matching
maths and most languages (it was left-associative = 64 before; the maker should not carry a
surprise there); and the **comparison** level is **non-associative** — `a == b == c` /
`1 < x < 10` are rejected at parse time, because left-associative grouping would silently
compare a boolean to the third operand. Parenthesise, or use `&&` for a range test.

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

### Pattern-operator precedence (@PLN35, SPEC-FIRST · planned, NOT yet implemented)

> **@PLN35 · SPEC-FIRST** — the target for the PEG pattern grammar
> ([matching.md § Rules — PEG patterns](matching.md)), written ahead of the code. The pattern
> *productions* live in [LOFT.md § Summary of grammar](../LOFT.md); this pins their operator
> precedence, the one fact that lives only in the parser. Design:
> [../plans/35-match-peg/FORMAL-DESIGN.md](../plans/35-match-peg/FORMAL-DESIGN.md).

A pattern has its own small precedence ladder (separate from the expression ladder above), loosest
first:

```
  pattern level   form                              binds
  ─────────────   ───────────────────────────────   ──────────────────────────────────
   0  ` , `        multi-pattern arm separator        (loosest — separates whole alternative shapes)
   1  ` | `        alternation inside a group
   2  sequence     juxtaposition inside `[ … ]`
   3  ` : `        capture  name:pat
   4  postfix       `?`  `*`  `+`  `*(sep)`           (tightest — bind to the nearest pattern)
      prefix        `..name`  (rest — only as a slice tail)
```

```
  (G-Pat-Prec)   for pattern operators, level N binds tighter than level N-1: `a:V | b:W` is
                 `(a:V) | (b:W)`; `x:p*` is `x:(p*)`.
  (G-Pat-Group)  a `(…)` inside a pattern is an alternation / optional / repetition GROUP, told apart
                 from a tuple pattern by its operators (`|`, `?`, `*`, `+`) — the same speculative
                 parse the surface already uses (a DECIDED EDGE like D-gram-2; no CFG is owed,
                 tooling reuses the hand-written parser).
```

**In words.** Inside a pattern, `,` (separating whole alternative shapes in a multi-pattern arm) is
loosest, then `|` (ordered choice), then a sequence of sub-patterns, then `:` (a capture); the
postfix quantifiers `? * +` bind tightest to the pattern right before them — with **no** parens
around a **single** element (`Num { value: n }*`, `Ident { name }?`), so `( … )` groups only a
multi-element sequence or an alternation (`( Colon, Ident { name: ty } )?`). `..name` is a prefix
rest, allowed only as the last element of a slice. A parenthesised pattern is a *group* (not a
tuple) when it carries a pattern operator — resolved by the same position-based, deliberately
non-context-free parse this doc already documents for `&` and struct-vs-block.

---

## Deviations

OPEN: **0** — (was 4). D-gram-3 (`**` right-assoc) + D-gram-1 (precedence-doc) closed in code/
doc; D-gram-2 + D-gram-4 resolved as **decided edges** (the spec-may-adjust outcome the roadmap
predicted — they leave formal/ rather than being driven to zero).

> **D-gram-1 (CLOSED) — the written grammar now states precedence + associativity.**
> [LOFT.md § Operators](../LOFT.md#operators) carries the full twelve-level ladder (the stale
> table is fixed: `**` added at level 10, `as` moved to 11) with an explicit associativity
> statement (all left-assoc except `**` right-assoc) and the unary-binds-tightest note
> (`-2 ** 2 == 4`). [LOFT.md § Summary of grammar](../LOFT.md) no longer collapses operators
> into one undefined `op`: it enumerates `binary_op` and cross-references § Operators for the
> grouping. The "read-the-parser tax" (FORMALIZATION.md rough spot #4) is paid down — the two
> user-facing statements pin every expression's shape, matching `OPERATORS` / `parse_operators`.

> **D-gram-2 (RESOLVED — decided edge) — the surface is deliberately not context-free.**
> The speculative backtracking (type-vs-variable, `S { … }`-vs-block) + lexer interpolation
> modes are accepted on purpose: they buy real ergonomics and no consumer needs a CFG (tooling
> reuses the hand-written parser, which IS the spec). Decided, not chased → [DESIGN_DECISIONS.md
> C82](../DESIGN_DECISIONS.md#c82--lofts-surface-is-deliberately-not-context-free).

> **D-gram-4 (RESOLVED — decided edge) — `&` stays one token, disambiguated by position.**
> Infix `&` is bitwise-and; a *leading* `&` is the reference annotation. A1 (binding.md
> D-bind-7) made prefix `&` a parse error in every non-binding position, so the positional rule
> is now **total** — like Rust, one `&` token is kept. Decided → [DESIGN_DECISIONS.md
> C81](../DESIGN_DECISIONS.md#c81---stays-one-token-disambiguated-by-position-bitwise-and-vs-reference).

---

## Conformance

The precedence rules are checkable by evaluation: `1 + 2 & 3 == 3` (additive tighter than
bitwise-and), `2 ** 3 ** 2 == 512` (RIGHT-assoc `**`; guarded by
`tests/issues.rs::power_is_right_associative`), `x as integer as float` (left-assoc `as`). The
two decided edges have no falsifier *to drive to zero* — they are accepted shapes: the
positional `&` rule is enforced by binding.md's `pln87_amp_*` parse-error tests, and the
non-CFG surface is the parser's own behaviour (COMPILER.md documents the disambiguation points).
