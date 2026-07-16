<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Plan — a parser-driven loft formatter, written in loft

**Status: DESIGN (groundwork). Owner sets the rules; this plan builds the framework.**

The current formatter (`src/formatter.rs`, 765 lines) is a token state machine with its own
scanner — no structure, so it can space and indent but cannot make a *structural* decision
("does this whole call fit? break the arguments"). We now want a real, `rustfmt`/`gofmt`-class
formatter — but **written in loft**, consuming loft's own surface syntax. That is the dogfood:
loft's formatter is a loft program.

## The finding that shapes everything (probed, not assumed)

*"We have a full parser, can we use it?"* — **No, not its output.** Two facts, verified in the
code:

1. The **lexer strips comments** (`lexer.rs`: "code with spaces, line ends and remarks removed")
   and the **Value IR has no trivia node** (`grep Comment/Trivia src/data.rs` → nothing). A
   formatter that loses comments is a non-starter.
2. The **Value IR is LOWERED / desugared** — `x += 1` becomes `OpAppendVector`/`Op…`, `for`
   becomes `Iter`, offsets are computed, synthetic nodes are inserted. It is a *semantic* tree,
   not a *surface* one. Re-emitting from it would print a different program than was written.

So the semantic parser's product is **lossy twice over** (no comments, desugared). This is the
over-unification guard firing early: the tempting "reuse the parser" claim is falsified before a
line of code. **The groundwork is therefore a fresh, lossless *surface* pipeline** — which the
owner already anticipated: *"we probably need a custom lexer."*  (The existing `formatter.rs`
scanner is exactly such a lexer, flat and token-level; the leap is to a **tree**.)

## The invariant (one sentence — the whole safety argument)

> The lexer+parser is **TOTAL and lossless** — every byte of the source belongs to exactly one
> token (comments and whitespace included, as *trivia* carried on the adjacent token), so the
> tree can reproduce the input **byte-for-byte**; formatting is then a pure function over that
> tree that rewrites **only trivia** (spaces / newlines / indentation), never the content, count,
> or order of real tokens.

Everything a formatter must guarantee falls out of this one property:

- **Semantic invariance** — `reparse(format(x))` yields the same real-token sequence as
  `reparse(x)` (only trivia moved). We never change a program's meaning.
- **Idempotency** — `format(format(x)) == format(x)` (the layout is a fixpoint of the rules).
- **Comment preservation** — comments are trivia on tokens; total coverage means none can be
  dropped.
- **Safe fallback** — a subtree the rules don't recognise renders **verbatim** (its original
  trivia), so an unformatted-but-correct region is always available. Parse errors degrade to
  verbatim, never to corruption.

These are the load-bearing claims; each gets a probe in the test harness (Step 0).

## The content-aware breaking model — the owner's rule, made a first-class concept

Standard formatters (Prettier, rustfmt) break a group **only when it exceeds the line width**.
The owner's rule is different and better for readable data: **break on content *complexity*,
width only as a fallback.** Stated by the owner:

- an **object (struct literal) without complex fields** stays on **one line** (unless too long);
- a **vector of numbers** stays on **one line**;
- a **vector of objects** goes **multi-line** (one object per line);
- *"we do not break if the content is simple and not taking a line for itself."*

Formalise this as **two orthogonal predicates on a node** (these ARE the rules the owner tunes;
the framework just consults them):

- **`inline_ok(node)`** — may this node render on a single line *at all*? A struct literal is
  `inline_ok` iff every field value is `inline_ok`; a vector iff every element is `inline_ok`; a
  leaf (number / string / ident / bool / char) is always `inline_ok`. (Then, and only then, does
  width decide — "unless too long".)
- **`owns_a_line(node)`** — is this node an *entity that deserves its own line* inside a
  container? A **struct literal owns a line** → a vector containing one breaks, one object per
  line. A number does not. This is what makes *"vector of objects → multi-line"* independent of
  width, while *"vector of numbers → one line"*.

The container layout is then a single rule (the chokepoint, below):

> A container renders **INLINE** iff *no* element `owns_a_line` **and** the whole fits the width
> budget; otherwise **BROKEN**, one element per line. Each element is *itself* formatted by the
> same rule, so `[ Point{x:1,y:2}, Point{x:3,y:4} ]` breaks the vector (objects own a line) yet
> keeps each object inline (its fields are simple).

`inline_ok` / `owns_a_line` / the width budget / the "simple leaf" set are the **owner's dials**.
The framework is rule-agnostic; this plan ships the two predicates as a small, replaceable module
seeded with the rules above.

### Comments (remarks) — the owner's trivia rules

Comments are trivia on tokens (per the invariant), and get their own layout policy:

- **A remark that fits stays at the end of its line** — a trailing `// …` is *trailing trivia*
  of the last real token on that line and is emitted there, not bumped to its own line.
- **Consecutive trailing remarks align to a common column** — over a *run* of adjacent lines that
  each carry a trailing remark, the `//` starts at the same character. This is **soft**: one line
  whose code is longer than the alignment column simply overruns (its remark sits one space after
  its code), and it does **not** drag the column out for the whole run — the others still align to
  the run's normal column.

Mechanically this is a **post-layout alignment pass** in the emit layer (Layer 4), *after* the
structural layout has fixed where every line breaks: group maximal runs of consecutive
trailing-comment lines, compute the run's alignment column (e.g. the common case width, ignoring a
rare overlong outlier), pad each remark to it, and let an outlier overrun. It touches only trivia,
so it cannot violate the invariant. The "how the column is chosen" (strict-max vs
majority/outlier-tolerant) is an owner dial — the soft phrasing says *outlier-tolerant*.

### The aim is ONE canonical format; the corpus VALIDATES the rules toward it

The goal is a single, opinionated, **universal** layout — one right way, no configuration, exactly
`gofmt`/`rustfmt`'s philosophy. The explicit rules above (objects / vectors / comments) plus the
ones still to be set define **the** canonical form; the formatter's job is to bring *any* input to
it.

The existing hand-written loft (`default/*.loft`, `tests/scripts/*.loft`, `lib/*.loft`, the sibling
libs) is therefore **not the definition of the style** — it is the **validation-and-tuning corpus**
for converging the rules on that universal form. It embodies good instincts about what reads well,
so running the rules over it is how we discover where a rule produces a **bad or inconsistent**
layout and fix it. This is how `gofmt` itself was tuned — run against real Go, adjust, until the
canonical form settled.

- A divergence between formatter and corpus is a **diagnostic to examine, not a churn to minimize**.
  Two outcomes, adjudicated by the owner *per class*: the rule produces a WORSE layout than the code
  → **fix the rule toward the canonical ideal**; or the code was non-canonical → **the reformat is
  correct, accept it**. Reformatting non-canonical corpus code is *expected* — the aim is the
  universal format, not preserving what happens to be there.
- So churn is a **lens** (where do the rules and good code disagree, so we can improve the rules?),
  **not the objective function**. The objective is a layout that is good and consistent everywhere;
  the corpus is how we stress the rules to get there.
- Once the rules settle, the formatter is a **fixpoint** on canonical code (idempotency), the corpus
  is reformatted once to the canonical form, and thereafter stays put — the ongoing invariant.

This is also the design-protocol *residual* in action: the micro-conventions no explicit rule
anticipated (exact wrap thresholds, chain-break points, blank-line habits) surface only by running
against real code, and each is then decided **as a universal rule**, not a per-file exception.

## Architecture — four loft layers (all dogfood; no new Rust parser)

```
source text ──▶ (1) lossless LEXER ──▶ (2) surface PARSER (CST) ──▶ (3) LAYOUT ──▶ (4) EMIT ──▶ text
   (loft)          tokens+trivia          a total tree of nodes     Doc + rules    trivia only
```

1. **Lossless lexer (loft).** `lex(src: text) -> vector<Token>`. Each `Token` = { kind, text,
   span, leading_trivia, trailing_trivia } where trivia = the comments + whitespace + newlines
   that precede/follow it. **Total**: concatenating every token's trivia+text reproduces `src`.
   (The `formatter.rs` `Tok`/`scan` is the working reference to port.)
2. **Surface parser → CST (loft).** `parse_cst(tokens) -> Node`. A *concrete* syntax tree: nodes
   are `Container([/{/( … )` , `StructLit(name, fields)`, `Call(callee, args)`, `Binary`, `Leaf`,
   `Block`, `Item(fn/struct/…)`, etc. **Lossless & error-tolerant**: unknown/half-typed input
   becomes a `Verbatim` node holding its exact tokens. This parser needs NO types, scopes, or
   lowering — far less than the semantic parser — so it is not a "second compiler", just a
   surface recogniser. (It may later be validated against the semantic parser: same file must
   both `parse_cst` and compile.)
3. **Layout (loft).** Walk the CST → a **`Doc`** (a Wadler/Prettier-style algebra: `Text`,
   `Line`, `Nest`, `Group`, `Trivia`). The container rule above is encoded as **one** `Group`
   variant whose break decision reads `owns_a_line`/`inline_ok`/width — the SINGLE chokepoint for
   every breaking decision (see re-assertion count).
4. **Emit (loft).** Render the `Doc` to text with the width budget, re-emitting each token's real
   text unchanged and its trivia per the layout. Comments re-attach at their anchor token.

**Written in loft, shipped bundled.** The whole formatter is a `.loft` program embedded like
`default/*.loft` (`include_str!` is already how the stdlib ships). `loft --format` /
`--format-check` (already wired in `main.rs:4186`) runs it. No compiler/parser reflection API is
needed — the formatter re-implements a *surface* lexer/parser in loft; that IS the dogfood, and
it stress-tests loft's text, enum, and recursion story exactly where a language wants exercise.

## Two modes — check (test) and active rewrite

Both modes share the identical pipeline (lex → CST → layout → emit); they differ only in what they
do with the emitted text:

- **Active rewrite** — `loft --format file.loft` (also a directory, and `-` for stdin→stdout):
  canonicalise in place, overwriting. The everyday "make it canonical" action.
- **Check / test** — `loft --format-check file.loft`: emit to a buffer, compare to the input; if
  they differ the file is **not canonical** → print the diff, **exit non-zero**, mutate nothing.
  This is the CI gate (a PR with non-canonical code fails) **and** the rule-validation instrument:
  during Step 4 we run check-mode over the whole corpus to *see every divergence* (the churn lens)
  without touching a byte, adjudicate rule-vs-accept per class, and only then does one active
  rewrite apply the settled format.

The two modes must be **provably the same decision**: `check(x)` passes **iff** `rewrite(x) == x`
(a file is canonical exactly when rewriting is a no-op). Enforcing that equivalence — a Step-0
invariant — keeps check-mode a true predicate for "already formatted" rather than a second,
drifting implementation. It is also what makes the cutover safe: check-mode reports the blast radius
before any active rewrite touches the tree.

## Re-assertion sites — count N (the brittleness tell)

The breaking decision must live in **one** place. If every node's rule re-implements
"do I fit? do I break?", that is N silent sites and the formatter will be inconsistent. So:

- **N = 1 for layout**: all breaking flows through the single `Group` resolver in Layer 3. Node
  rules only *build* `Doc`s (declare intent); they never decide width/breaks themselves.
- **Node coverage is loud, not silent**: the CST `Node` is a loft **enum**; the layout walk
  `match`es it. An unhandled kind is either a compile-time non-exhaustive-match (if we enumerate)
  or falls to the explicit `Verbatim` arm — never a wrong-but-silent format. Adding a node kind
  forces a rule.

## Failure paths (write them down — where the invariant is earned)

1. **A comment between two nodes** (`[ a, /*c*/ b ]`, or a trailing `// note`). Whose trivia is
   it? → Deterministic attachment rule (leading-trivia-of-next by default; trailing-line-comment
   stays trailing). A wrong-but-total attachment still round-trips; a *lost* comment does not —
   so totality is the guard, and Step-0 asserts comment count in == out.
2. **Idempotency break** — a rule whose output re-formats differently (e.g. a trailing comma the
   next pass strips). → Every rule is run through `format∘format == format` in Step 0.
3. **Blank-line policy** — users keep *some* blank lines (section breaks). → Trivia carries blank
   runs; the rule collapses runs to ≤1 (or ≤2) but never invents/deletes all. Owner's dial.
4. **Inside strings / format-literals** — never reformat token *interiors*. → Layer 4 emits a
   token's `text` verbatim; only trivia is rewritten. String/`{…}`-format bodies are one token.
5. **A file that does not parse** (mid-edit). → `Verbatim` nodes + error-tolerant `parse_cst`;
   worst case the file is returned unchanged. `--format-check` then reports "not canonical",
   never corrupts.
6. **Width vs complexity conflict** — a *simple* vector that is genuinely too long. → `inline_ok`
   gates *eligibility*, width makes the final call ("unless too long"); the two compose in the
   one `Group` resolver, not in scattered rules.

## Small safe steps (groundwork first; the rule-set is the owner's, plugged in last)

| # | Step | Verify |
|---|---|---|
| 0 | **The safety harness (before any formatting).** A corpus of `.loft` files (start with `default/*.loft`, `tests/scripts/*.loft`). Assert the invariants as tests: **lossless** (`emit_verbatim(parse_cst(lex(x))) == x`), **idempotent** (`fmt(fmt(x)) == fmt(x)`), **semantics** (real-token stream of `fmt(x)` == that of `x`), **comment count** in == out, and **check≡rewrite** (`check(x)` passes iff `rewrite(x) == x`). These run against every later step. | harness compiles + the identity/round-trip cells pass on the corpus with a no-op formatter |
| 1 | **Lossless lexer in loft.** Port `formatter.rs`'s `scan`/`Tok` to loft with trivia attachment; prove **totality** (concat == source) on the corpus. | Step-0 lossless cell green through the token layer |
| 2 | **Surface CST in loft.** `parse_cst` for the container-bearing forms first (`[]`, `{}`, `()`, struct-lit, call), everything else `Verbatim`. Lossless + error-tolerant. | Step-0 lossless cell green through the tree; `Verbatim` fallback exercised |
| 3 | **Doc algebra + the ONE `Group` resolver.** No rules yet — a `Group` that only ever stays inline. Emit == verbatim-but-renormalised-whitespace. Establish idempotency. | Step-0 idempotent + semantics cells green |
| 4 | **Seed rules + tune against the corpus.** Encode the explicit rules (`inline_ok`/`owns_a_line`/comments/width) as small functions, THEN run over the idiomatic corpus and **drive churn toward zero** — each divergence class adjudicated by the owner (tune-rule vs accept-reformat). A fixture per explicit rule + the churn report. | explicit-rule goldens match; corpus churn small + every remaining diff class owner-approved; Step-0 invariants still green |
| 5 | **Parity + cutover.** The idiomatic corpus (not the old formatter) is the primary oracle — `loft --format` must be a **fixpoint** on it. Diff against the current `formatter.rs` as a *secondary* parity check (deltas are the intended new opinions). Bundle the `.loft` formatter; point `loft --format`/`--format-check` at it; retire `formatter.rs`. | `make ci`; formatter idempotent on the whole corpus; owner sign-off on the churn diff |

Steps 0–3 are pure **groundwork** — lossless pipeline + layout engine, no opinions baked in.
Step 4 is where the owner's rules land (and keep landing — the module is meant to grow). Step 5
is the switch.

## What is the framework's vs the owner's

- **Framework (this plan builds):** lossless lexer, surface CST, the `Doc` algebra, the single
  `Group` resolver, the trivia/comment attachment, the invariant harness, the `loft --format`
  wiring. Rule-agnostic.
- **Owner (opinions, plugged into Step 4+):** the `inline_ok` / `owns_a_line` predicates, the
  width budget, the "simple leaf" set, blank-line policy, trailing-comma policy, indent width,
  alignment choices, and every other stylistic call. Seeded with the four rules stated above;
  designed to accrete more without touching the engine.

## Open questions for the owner (decide before Step 4)

- **Width budget** — a hard column (e.g. 100), or only ever break on complexity and *never* on
  width for a truly-simple line?
- **`owns_a_line` for nested vectors** — does a `vector<vector<int>>` break the outer per inner
  vector (inner "owns a line"), or stay inline when every inner is short?
- **Trailing comma** on a broken container (`,\n]` vs `\n]`) — present (git-friendlier) or absent?
- **Blank-line cap** — collapse runs to ≤1 or ≤2; keep a blank after the license header / between
  top-level items?
- **Alignment** — align struct-literal field values / `match` arms into columns, or single-space?
- **Trailing-comment run boundaries** — what ENDS a run of aligned remarks? A blank line? A line
  with no trailing remark? A change of indent level (so a nested block's remarks align among
  themselves)? (Almost certainly: same indent level, no interrupting blank/uncommented line.)
- **The soft outlier threshold** — how far past the column may a line reach before it is treated
  as the outlier that overruns (rather than widening the column for everyone)? A fixed slack, the
  width budget, or "the column is the width of all-but-the-longest"? This choice must be
  **idempotent** (re-aligning already-aligned output picks the same column) — a Step-0 cell.
