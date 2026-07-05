<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# formal/formatting.md — semantics for text formatting & interpolation (strict)

**Catalogue:** @F (text formatting), @PLN89 (differential oracle). Behaviour sources:
[LOFT.md § Strings](../LOFT.md), [STDLIB.md](../STDLIB.md); fault-safety is C66 / @P376.

> **Rules then deviations** (see [README](README.md)). This is the relation for **string
> interpolation** — the `"…{e}…"` template — and the **value→text rendering** every scalar and
> heap type obeys. It extends [operational.md](operational.md) (eval order, the null sentinels,
> uncomputable→null) and [heap.md](heap.md) (a struct/vector renders by walking its store). Every
> rule is a **user-visible contract** verified on both backends.

## Notation

Uses [operational.md](operational.md)'s `⟨e, σ⟩ → ⟨e', σ'⟩`. Write `⟦v⟧_f` for the **text
rendering** of value `v` under an (optional) format spec `f` — the text that an interpolation
`{e:f}` produces once `e` has evaluated to `v`. A **template** is a string literal `"s₀{e₁:f₁}s₁…"`
of interleaved literal runs `sᵢ` and interpolations `{eᵢ:fᵢ}`.

---

## Rules

### Interpolation & escape — `{e}` splices a rendered expression

```
  (F-Interp)  a template "s₀{e₁:f₁}s₁…{eₙ:fₙ}sₙ" evaluates each eᵢ LEFT TO RIGHT
              (operational.md E-Left) and produces the text  s₀ · ⟦v₁⟧_f₁ · s₁ · … · ⟦vₙ⟧_fₙ · sₙ
              (`·` is concatenation).  eᵢ is an ARBITRARY expression — a variable, field access
              `a.b`, index `v[i]`, call `f(x)`, arithmetic — parsed at full language level.
  (F-Escape)  inside a template `{{` denotes a literal `{` and `}}` a literal `}`; a single `{`
              opens an interpolation.  Everything outside `{…}` is copied verbatim.
```

**In words.** `"hi {name}, {a + b} left"` renders the literals unchanged and splices each
`{…}` by evaluating the expression and rendering its value; the pieces are concatenated in source
order. To put a real brace in the output, double it — `"{{lit}}"` renders `{lit}`. The braces
hold *any* expression, not just a bare name (verified: `{42:#x}`, `{col}`, `{a / b}`).

### Desugaring — a template is a text accumulator, one sink for both backends

```
  (F-Desugar)  a template compiles to a fresh TEXT accumulator: clear it, then for each piece
               append the literal run and append ⟦vᵢ⟧_fᵢ; the template's value is that text
               (type `text`).  Both backends route rendering through the SAME functions
               (`ops::format_*`, and the store walker for heap values), so ⟦v⟧_f is
               backend-independent by construction.
```

**In words.** `"…{e}…"` is not a hidden `+`-chain of separate strings; it builds one text buffer
by appending each rendered piece. Because the interpreter and `--native` call the identical
rendering code, the produced text is the same on both — this is what makes formatting a *rules*
doc (a written contract) rather than a place the backends can drift.

### Rendering per type — the default form, and null

```
  (F-Render)  ⟦v⟧ (no spec) renders by the value's type:
                integer / long      decimal digits            null (i64::MIN)  → "null"
                float / single      shortest round-trip form  null (NaN)       → "null"
                boolean             "true" / "false"          null (0xFF)      → "null"
                text                the characters as-is      null (STRING_NULL) → "null"
                character           the single codepoint      null (0)         → ""  (nothing)
                enum                the variant name          null (0xFF disc) → "null"
                vector              "[e₀,e₁,…]"  (compact, elements rendered by F-Render)
                struct              "{field:value,…}"  (compact loft form; the `:j` spec → JSON)
```

**In words.** Each type has one canonical text form. A **null** of any type renders as the literal
word `null` — the single exception is a null **character** (codepoint 0), which renders as nothing
(so iterating text past its end appends no garbage). A vector is a compact bracketed list; a struct
is a compact `{field:value}` form, and the `:j` spec switches it to JSON with quoted keys (verified:
`{r:128,g:0,b:128}` vs `{"r":128,"g":0,"b":128}`).

### Format spec — width, alignment, padding, precision, radix, sign

```
  (F-Spec)   after `:` a spec tunes ⟦v⟧_f:
               width N            minimum field width (numbers right-, text left-aligned by default)
               < > ^              force left / right / centre alignment
               0N                 zero-pad a number to width N
               .P                 fixed P fractional digits (float/single)
               #x  x  b  o        integer radix: hex-with-0x / hex / binary / octal
               +                  always show a sign on a number
             width counts Unicode CODEPOINTS, not bytes.
```

**In words.** `{1:03}` is `001`, `{42:#x}` is `0x2a`, `{334.1:.2}` is `334.10`, `{"abc":>7}` is
`    abc`. Numbers pad on the left (right-aligned) and text pads on the right (left-aligned) unless
an explicit `<`/`>`/`^` overrides it; the width is measured in characters, so a multi-byte glyph
still counts as one column.

### Fault-safety — an uncomputable inside `{…}` renders a tagged null, never halts

```
  (F-FaultSafe)  a fault-prone operation inside an interpolation (÷0, index OOB, a field of null)
                 follows operational.md E-Uncomp: it yields the null VALUE, and the interpolation
                 renders it as "null" annotated with the fault cause, e.g. "null(/0)".  Formatting
                 a value NEVER traps or halts — the template always produces text.
```

**In words.** `"{a / b}"` with `b = 0` renders `null(/0)` rather than crashing the program — the
formatter rewrites fault-prone operations in interpolation position to their nullable peers (C66 /
@P376), so building a diagnostic string can never itself fault. The tag (`/0`, an out-of-range
index, …) names *why* the value is null, which is exactly what a `"{x}"` in a log line wants.

---

## Deviations

OPEN: **0** (a *rules* doc — it shrinks operational.md's D-op-1, adds no code deviation).

- **Conformance is differential** — formatting is enforced across the two backends by the @PLN89
  oracle (D-op-1) plus the dedicated `tests/scripts/14-formatting.loft` / `tests/docs/30-formatting.loft`
  suites, which pin the exact rendered strings and run on both backends. A divergence in a rendered
  value, a null form, a struct/vector Display, or a format-spec result is caught there.
- **Rendering `null` as the word `null` is the contract, not a gap** — the in-band sentinels
  (`i64::MIN`, `NaN`, `STRING_NULL`, `0xFF`) are an operational decision (operational.md E-Null);
  their uniform `"null"` text (character-0 excepted) is intended, so it is a rule here, not a
  deviation.

---

## Conformance

- **Interpolation + escape (`F-Interp` / `F-Escape`)** — `"{{lit}}"` is `{lit}`; `"{40 + 2}"` is
  `42`; `"{col}"` renders the struct.
- **Per-type render + null (`F-Render`)** — `"{true}"` is `true`; `"{[1,2,3]}"` is `[1,2,3]`;
  `"{col}"` is `{r:128,g:0,b:128}` and `"{col:j}"` is `{"r":128,"g":0,"b":128}`; `null as integer?`
  renders `null`.
- **Format spec (`F-Spec`)** — `"{1:03}"` is `001`, `"{42:#x}"` is `0x2a`, `"{334.1:.2}"` is
  `334.10`, `"{\"abc\":>7}"` is `    abc`.
- **Fault-safety (`F-FaultSafe`)** — `a = 5; b = 0; "{a / b}"` is `null(/0)` on both backends, and
  the program continues.

D-op-1's falsifier applies: any program where the interpreter and `--native` disagree on a rendered
string is the definitional error this doc names.
