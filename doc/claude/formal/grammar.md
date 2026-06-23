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
  10      **                                 (power)
  11      as                                 ← tightest (cast; groups first / innermost)
```

```
  (G-Prec)   for binary operators, level N binds tighter than level N-1.  Parsing is
             precedence-climbing: `parse_operators(p)` parses operands at level p+1, so a
             higher level groups deeper.
  (G-Assoc)  ALL binary levels are LEFT-associative, including `**` and `as`:
                 2 ** 3 ** 2  ==  (2 ** 3) ** 2  ==  64        (NOT 2 ** (3 ** 2) == 512)
                 x as integer as float  ==  (x as integer) as float
```

**In words.** loft has one precedence ladder, twelve rungs. The closer to the bottom
(`as`, `**`, `*`), the tighter an operator grips its operands; the closer to the top
(`??`, `||`), the later it groups. Everything is left-to-right — there is no
right-associative operator, so `2 ** 3 ** 2` is `(2**3)**2 = 64`, which is **not** the
usual maths convention (most languages make `**` right-associative → `512`).

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

OPEN: **4**

### D-gram-1 — the written grammar omits precedence entirely
- **Violates:** G-Prec / G-Assoc (they exist only in code)
- **Where:** [LOFT.md § Summary of grammar](../LOFT.md) collapses every binary operator
  into one `expr OP expr` rule; the twelve levels + left-associativity live only in
  `src/parser/mod.rs:375` (`OPERATORS`) and `operators.rs:446` (`parse_operators`).
- **Effect:** reasoning about how an expression groups means reading the parser — the
  "read-the-parser tax" (FORMALIZATION.md rough spot #4).
- **Status:** OPEN — closing it = lifting the table above into LOFT.md's grammar.
- **Removal:** state the precedence ladder + left-associativity in the user-facing grammar.

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

### D-gram-3 — `**` is left-associative, against convention
- **Violates:** least-surprise (not a soundness rule — a decided edge to pin)
- **Where:** `**` sits in the uniform left-associative precedence-climbing loop
  (`mod.rs:375` level 10); measured `2 ** 3 ** 2 == 64`.
- **Effect:** `a ** b ** c` means `(a**b)**c`, where maths and most languages mean
  `a**(b**c)`. A real footgun for a ported formula.
- **Status:** OPEN — decide: keep (document loudly in LOFT.md) or make `**`
  right-associative. If kept, this becomes an INCONSISTENCIES.md entry, not a deviation.
- **Removal:** either right-associate `**`, or move this to the decided-edge register.

### D-gram-4 — `&` is overloaded (bitwise-and vs reference annotation)
- **Violates:** the clean separation G-Amp-* relies on position alone
- **Where:** infix `&` (bitwise-and, level 6) and prefix `&` (reference annotation) share
  one token; disambiguation is positional, and the prefix case is exactly the leak
  [binding.md D-bind-7](binding.md) records (a stray prefix `&` parses, then mis-elaborates).
- **Effect:** the overload couples this grammar to binding.md: enforcing "prefix `&` only
  at a binding" is a grammar obligation, not just a binding one.
- **Status:** OPEN — coupled to binding.md D-bind-0/D-bind-7.
- **Removal:** make prefix `&` accepted *only* in reference-annotation positions at the
  grammar level (so it can never reach an expression slot), which also closes D-bind-7.

---

## Conformance

The precedence rules are checkable by evaluation: `1 + 2 & 3 == 3` (additive tighter than
bitwise-and), `2 ** 3 ** 2 == 64` (left-assoc `**`), `x as integer as float` (left-assoc
`as`). D-gram-2's falsifier is structural (no CFG accepts loft); D-gram-3's is the `**`
result; D-gram-4's is binding.md's `[&a]` → internal-error case.
