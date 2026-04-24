
// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

# Regex — standalone library design

> **Status: design draft.**  Regex lives as a library, not as a
> language-level literal or match-pattern kind.  This replaces the
> earlier `r"..."` raw-regex-literal plan and the "regex arm in match"
> plan that were sketched in LAZY_STDLIB.md.

---

## Why a library, not a language feature

Two earlier plans are cancelled:

1. **`r"..."` raw-regex literals** at the lexer level.  Cost: a new
   literal form with its own escape rules, a second string-like type
   to propagate through the type system, and an always-on dependency
   on whatever regex engine ships.  Benefit: three characters saved
   at call sites.  Not worth it.
2. **"Regex arm" inside `match`**, sharing the pattern-matching
   compile pipeline.  Cost: a whole second pattern language embedded
   in the compiler, competing with the PEG-style sequence patterns in
   [MATCH_PEG.md](MATCH_PEG.md), with a closed character vocabulary
   that inevitably grows.  Benefit: a fused syntax for text arms.
   Not worth it — PEG patterns cover structural matching; regex
   covers text; keeping them separate is cleaner than fusing them.

The PEG-style match extension ([MATCH_PEG.md](MATCH_PEG.md)) stays as
designed, but its scope is **structural** — vectors, enum shapes, and
iterators.  Text matching is this library's job.  One text-pattern
language is easier to learn than two, so the originally-sketched
backtick character template (MATCH_PEG L3.5) has been withdrawn in
favour of routing all text through here.

The library approach gives regex users **no artificial limits**:
custom character classes, anchors, lookaround, non-greedy quantifiers,
named groups, Unicode properties — whatever a production regex engine
supports, the library exposes.

---

## Surface

```loft
// Compile once, re-use.  Syntax errors in the pattern are a
// compile-time diagnostic at the call site.
re: Regex = regex("^(\w+)=(.+)$")

// One-shot full-string match
match re.match(line) {
    Some(m) => use(m.group(1), m.group(2)),
    None    => error("bad line"),
}

// First occurrence anywhere in the string
re.find(line) -> option<Match>

// All non-overlapping occurrences
re.find_all(line) -> vec<Match>

// Replace
re.replace(line, "$1 -> $2")        -> text
re.replace_all(line, template)      -> text

// Split on matches
re.split(line)                      -> vec<text>

// Named groups
re2: Regex = regex("HTTP/(?<maj>\d)\.(?<min>\d)")
match re2.match(line) {
    Some(m) => version(m.name("maj"), m.name("min")),
    None    => error("not HTTP"),
}
```

### The `Match` value

```loft
struct Match {
    text:   text,            // whole matched substring
    start:  i64,             // byte offset in source
    end:    i64,             // byte offset (exclusive)
    groups: vec<text>,       // group 0 = whole match; 1..n = captures
    names:  map<text, text>, // named group lookup (empty if none)
}
```

`Match` is a plain struct — it destructures in a regular `match` arm
like any other struct, which is how regex integrates with pattern
matching **without** any special language support:

```loft
match re.match(line) {
    Some(Match { groups: [_, key, value], ... }) => bind(key, value),
    _ => error("bad line"),
}
```

---

## Supported features (target: PCRE-parity for common cases)

| Feature | Supported | Notes |
|---|---|---|
| Literals, dot, escapes | yes | Standard regex escapes |
| Char classes `[...]`, `[^...]`, ranges | yes | Including Unicode ranges |
| Quantifiers `*`, `+`, `?`, `{n,m}` | yes | Greedy by default |
| Non-greedy `*?`, `+?`, `??` | yes | |
| Possessive `*+`, `++` | yes | No-backtrack variants |
| Anchors `^`, `$`, `\b`, `\B` | yes | Multiline mode flips `^`/`$` |
| Groups `(...)` | yes | Capturing |
| Non-capturing `(?:...)` | yes | |
| Named groups `(?<name>...)` | yes | `m.name("…")` lookup |
| Alternation `a\|b` | yes | |
| Backreferences `\1`, `\k<name>` | yes | |
| Lookaround `(?=)`, `(?!)`, `(?<=)`, `(?<!)` | yes | Bounded-width for fast path |
| Unicode properties `\p{L}` | yes | Via stdlib Unicode tables |
| Inline flags `(?i)`, `(?s)`, `(?m)` | yes | |
| Comments `(?#...)` and `(?x)` verbose | yes | |

Anything common in PCRE or Rust's `regex-syntax` is in scope; recursion
and subroutine calls (`(?R)`, `(?P>name)`) are deferred until demand
is clear.

---

## Engine

Two engines, one surface:

1. **Default: linear-time NFA** (Thompson / Pike VM).  Handles almost
   all patterns.  No catastrophic backtracking.  Features requiring
   unbounded lookaround or backreferences fall through to engine 2.
2. **Fallback: backtracking engine** for features the linear engine
   doesn't cover (backrefs, variable-width lookaround).  Users
   opt in per-compile via `regex_bt("...")` if they want the fallback
   on purpose; otherwise the library picks based on the pattern's
   feature set.

Compilation reports which engine a pattern will run on, so performance-
sensitive users can see it at a glance.

### Safety

- The default engine is linear-time in input length.  ReDoS-class
  inputs cannot blow up.
- The backtracking engine has a configurable step limit
  (`regex_bt(..., max_steps: 1_000_000)`) so an accidental pathological
  pattern fails fast instead of hanging.
- Pattern compilation is a pure function — no I/O, no global state.

---

## Integration with `match`

Regex returns structs; structs destructure in `match` today.  No new
syntax required.  Three common shapes:

```loft
// 1. Whole-line match with positional groups
match re.match(line) {
    Some(Match { groups: [_, a, b], ... }) => use(a, b),
    _ => fallback(),
}

// 2. Named groups — use the names map
match re.match(line) {
    Some(m) if m.names.has("status") => dispatch(m.name("status")),
    _ => fallback(),
}

// 3. First-of-many patterns — try each
dispatch = null
for re, handler in routes {
    if (m := re.match(line)) != null {
        dispatch = handler(m)
        break
    }
}
```

Pattern 3 is the "route table" shape that a `r"..."` literal was meant
to optimise syntactically.  Compiled once in the table, reused per
request — the library shape is already idiomatic.

---

## Lazy loading

Triggers (from LAZY_STDLIB.md):

- Type reference: `Regex`, `Match`.
- Function call: `regex(...)`, `regex_bt(...)`.

No token-level trigger, no match-kind trigger.  Cold-start cost for
programs that never touch regex: zero.

---

## Implementation scope

| Phase | Scope |
|-------|-------|
| **R1** | Pattern parser → AST; compile to NFA.  Linear-time VM.  `match`, `find`, `find_all`, `replace`, `split`.  Unnamed groups. |
| **R2** | Named groups; `m.name(...)`; `$name` in replace templates. |
| **R3** | Unicode properties `\p{...}` — hooked into stdlib Unicode tables. |
| **R4** | Backtracking fallback engine; `regex_bt(...)` explicit entry point; step-limit safety. |
| **R5** | Compile-time DFA generation for hot paths — opt-in, replaces NFA for a given `Regex` without changing the user's source. |

Phases are strictly additive.  R1 alone covers the 95% case.

**Ship order relative to MATCH_PEG:** R1 ships **first**, before any
MATCH_PEG phase.  It is the smaller, library-scoped change, validates
the lazy-loading mechanism from [LAZY_STDLIB.md](LAZY_STDLIB.md), and
delivers immediate value for CLI / server / log use cases.  See
[MATCH_PEG.md](MATCH_PEG.md) § "Ship order" for the combined timeline.

---

## See also

- [LAZY_STDLIB.md](LAZY_STDLIB.md) — lazy-loading mechanism; regex is
  the first new consumer.
- [LOFT.md](LOFT.md) § Match expressions — base match semantics.
- [MATCH_PEG.md](MATCH_PEG.md) — PEG-style sequence patterns on
  vectors, enums, iterators, and (simple) text.  Regex is the tool
  for *complex* text; these two systems intentionally do not share a
  pattern language.
- [STDLIB.md](STDLIB.md) — where the library's public API will be
  documented once shipped.
- [PACKAGES.md](PACKAGES.md) — stdlib module layout.
