// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

# @PLN35 — Match PEG Patterns · Implementation Plan (step-by-step, verifiable)

> **Companion to [README.md](README.md)** (the design draft) and
> **[FORMAL-DESIGN.md](FORMAL-DESIGN.md)** (the strict-spec changes that GATE this
> build — read it first: the rules are the target these phases are built to satisfy,
> and §5 there maps each phase below to the formal rules it discharges + pins). This
> doc is the *build* plan: ordered phases, each independently shippable and verifiable
> on **both backends** (`--interpret` + `--native`), with the exact code points to
> touch. The README stays the design/rationale reference; keep design prose there,
> keep build steps here. The **sub-rule / parser-combinator layer** (invoking named grammar rules
> inside a pattern — the keystone) is its own design + steps in
> [SUBRULE-DESIGN.md](SUBRULE-DESIGN.md) (phases PC1–PC5, layered after the core P1–P7).
>
> **Weight note.** This is the **last language-syntax feature** planned for loft
> (everything after is tooling, optimization, ANSI-C library reach). It earns
> design rigor: every phase names its invariant and proves it with a red→green
> cross-mode probe before moving on.

---

## 1. What the code actually looks like (reconciliation with the design draft)

The README was written against an idealized syntax. Six facts from a full read of
the current `match` implementation change the plan. **Read these first — they are
why the phase order below differs from the README's L3.1→L3.7 list.**

| # | Finding | Code anchor | Consequence for the plan |
|---|---|---|---|
| **F1** | **There is no `match`/pattern IR node.** `match` is desugared *entirely in the parser* into nested `Value::If` chains + primitive ops. Both backends consume the same `If` IR. | `src/data.rs:476` (`enum Value` — has `If` at `:530`, no `Match`); `src/parser/control.rs:2620` (`parse_match`) | **Do not add an IR node.** All of L2–L3.4 is parser desugaring → both backends "for free". Adding a `Value::Match` variant would double the work (teach `fill.rs` *and* `generation/emit.rs`). |
| **F2** | **Slice backtracking needs no new opcode.** A slice cursor is a `usize` index; "revert" = reset an index temp. `parse_vector_match` already does index arithmetic with `OpGetVector`/`OpLengthVector`. | `src/parser/control.rs:3960`, `:4010`, `:4063` | `OpMatchAnchor`/`OpMatchRevert` are needed **only for iterator input (L3.6)**. L3.1–L3.4 emit no new ops. This is the single biggest de-risking. |
| **F3** | **Four separate arm-parsers, one per subject kind** — enum (inline in `parse_match`), scalar, vector, tuple. Not one recursive pattern grammar. | `control.rs:2620` (enum), `:3797` (scalar), `:3960` (vector), `:4169` (tuple) | PEG needs sub-patterns in *any* position. Phase 1 introduces one recursive `parse_pattern()` the four handlers delegate to. This unification is the backbone. |
| **F4** | **Nested sub-patterns don't parse** (`V { field: Inner { x } }`). A field with `:` only accepts a plain-enum variant name, `_`, or a scalar literal; a struct/struct-enum field falls to `self.expression()` and chokes on the inner `{`. | `control.rs:3554` (`parse_field_sub_pattern`), guard at `:3556` (`Type::Enum(_, false, _)`), scalar fallback `:3633-3636` | This is **L2**, the stated prerequisite, and it is unimplemented. It is Phase 1. |
| **F5** | **`...rest` named tail-capture does not exist.** `..` is a length-flex marker binding nothing; `[a, ..rest]` mis-parses (`rest` becomes a single last element). | `control.rs:3990` (`..` marker), element loop `:3986-4007` (bare `Vec<String>`) | True named-rest capture is a genuine gap added in Phase 2 (L3.1). |
| **F6** | **Exhaustiveness/totality is enforced for enums only.** Vector/tuple/scalar handlers fall back to a typed null with no coverage check. | enum check `control.rs:3138-3156`; vector fallback `:4141-4146` | The README rule "sequence/repetition are non-total → require a trailing `_`" has **no existing home** in the vector handler. Phase 2 adds it there. |

Two more facts inform the design but aren't obstacles:

- **F7 — loft has no tuple-style enum variants.** Only struct variants exist
  (`Num { v: integer }`, not `Num(integer)`). Every README example must be
  restated in struct-variant syntax. See **Decision D1**. (`tests/scripts/05-enums.loft:11`.)
- **F8 — recursion goes through a collection/reference, not an inline field.**
  `Neg { inner: Expr }` fails type-layout (`src/database/types.rs:364` skips the
  self-referential field → `u16::MAX` size → `:1021` "has no position"); `struct
  Node { kids: vector<Node> }` works because a collection is a fixed-size DbRef.
  AST enums in tests/docs must use `vector<Expr>` (or a ref). Plan note only.

---

## 2. Load-bearing design decisions

Each is my recommendation with the reasoning; flagged so they can be overridden
before Phase 1 code lands. **D2 and D3 are the ones I'd most want a second look at.**

- **D1 — Struct-variant syntax only; tuple variants are a PERMANENT non-goal ([C89](../../DESIGN_DECISIONS.md)).**
  loft enum payloads are always named fields (F7); positional `Num(i64)` / `Ok(a)` are never
  planned — they force match-to-read and breed a mitigation-syntax sprawl (the C89 rationale).
  Every example uses `Num { v: integer }`. **Cost:** AST examples read more verbosely — accepted.
  **Corollary (D6):** the pattern surface must read like grammar notation, not regex, even at the
  price of extra parser logic (C89) — patterns are the standardized readable parser notation
  (`|`/`?`/`*`/`+` on named elements), text/regex stays a library.

- **D2 — Parser desugaring, no new IR node, for L2–L3.4.** Follows F1/F2. Keep the
  proven "lower to `If` + existing ops" path; both backends stay free. *Rationale:*
  the alternative (a first-class PEG IR node) doubles backend work for no user-visible
  gain until iterators (L3.6). **This is the architecture bet the whole plan rests on
  — if it's wrong, the phase order is wrong.** It is falsified in Phase 0 by a probe
  (below) before any feature code.

- **D3 — One recursive `parse_pattern()`; the four handlers become callers.**
  Signature (working):
  ```rust
  // returns the boolean test (None = always-matches) + the binding statements
  // this pattern contributes, given a Value that reads the subject and its type.
  fn parse_pattern(&mut self, subject_read: Value, subject_type: &Type)
      -> PatternMatch;                 // { cond: Option<Value>, binds: Vec<Value>, total: bool }
  ```
  *Rationale:* PEG requires a sub-pattern in field, element, alternation, optional,
  and repetition positions — that is only expressible with recursion. The current
  `parse_field_sub_pattern` (`control.rs:3554`) is a degenerate 3-case version of
  exactly this; Phase 1 generalizes it and routes the field/element positions
  through it. *Risk:* it is a behavior-preserving refactor of live match lowering →
  gated by the loft-codegen rule (prove emitted IR byte-identical before adding any
  new case; see Phase 1).

- **D4 — Backtracking = save/reset an index temp (slices); a shadow anchor stack
  (iterators).** For slices (L3.2–L3.4) a "cursor" is a `usize` local; anchor =
  `Set(save, pos)`, revert = `Set(pos, save)`. For iterators (L3.6) add
  `anchors: Vec<(u32 pos, u32 epoch)>` on `State`, modeled on the existing shadow
  stacks `call_stack`/`coroutines` (`src/state/mod.rs:164,170`), plus the two new
  opcodes. *Rationale:* mirrors the design's own "Validated input shapes" table
  (slice revert cost = 1 word, no memo; iterator needs memo).

- **D5 — Provisional bindings reuse the existing per-arm name-scoping.** Bindings are
  already permanent slots whose *name* visibility is saved/restored per arm
  (`control.rs:3075-3081`), with `skip_free` so an untaken arm doesn't free a slot it
  never wrote (`:3472-3474`). A failed alternative simply takes the `else` branch and
  never reaches the body — its slot is written-but-unused, not corrupt. *Rationale:*
  no new rollback machinery for slices; the "bindings epoch" in the README is only
  needed for the iterator memo (L3.6).

---

## 3. Backbone: the unified recursive pattern parser

Everything hangs off D3. The target end-state (reached incrementally, not all in
Phase 1):

```
parse_pattern(subject_read, subject_type) -> PatternMatch
├─ literal / range        → OpEq* / range test        (exists: parse_match_pattern control.rs:3663)
├─ wildcard `_`           → cond=None, binds=[]        (exists, scattered)
├─ binding `name`         → Set(mv, subject_read)      (exists: control.rs:3442)
├─ enum variant `V {..}`  → tag test + recurse fields  (Phase 1 — generalize :3403/:3554)
├─ slice `[p, p, ...r]`   → len test + recurse elems   (Phase 1/2 — generalize :3960)
├─ tuple `(p, p)`         → recurse elems              (Phase 1 — via :4169)
├─ alternation `(a|b)`    → anchor; try a; revert; try b   (Phase 4)
├─ optional `(a)?`        → anchor; try a; else bind null   (Phase 5)
└─ repetition `(a)* (a)+` → bounded loop of anchored a   (Phase 6)
```

`PatternMatch { cond: Option<Value>, binds: Vec<Value>, total: bool }` is the
uniform currency: a handler builds its arm by AND-folding child `cond`s (as
`control.rs:3016-3028` already does for field sub-conditions + guard) and
concatenating child `binds`, then `total` drives the exhaustiveness gate (F6).

---

## 4. Phases

Ordering rationale: prerequisite first (L2), then the highest-value/lowest-risk
parser-only phases (L3.1, L3.7), then the backtracking trio (L3.2→L3.3→L3.4) which
share the slice-cursor desugaring, and finally the one phase that needs new runtime
primitives (L3.6). Each phase is a shippable PR.

Every phase's verification runs on **both backends** and is **leak-checked**
(`--interpret` runs `check_store_leaks`; native via the harness). Probe gate:
assertions pass · clean exit code · no leak warning · bounded runtime.

---

### Phase 0 — Reconciliation + executable spec (no feature code)

**Goal.** Lock the design to reality and write the red tests the later phases turn
green. Falsify D2 before building on it.

**Steps & verification.**
0.1 Restate README examples in struct-variant syntax (D1); add F1–F8 notes and the
    D1–D5 decisions to README § "Reconciliation". *Verify:* `make view` renders;
    doc-hygiene clean.
0.2 **D2 falsification probe.** Hand-write the *desugared* form of one alternation
    arm (`(0,x) | (1,x)`) as explicit `If`+index-temp loft and confirm it runs
    identically on both backends. *Verify:* if the hand-desugaring can express
    save/reset-index backtracking with existing ops → D2 holds, proceed. If not →
    stop and escalate to opcodes earlier. *(This is the cheap probe that could prove
    the architecture bet wrong.)*
0.3 Author the cross-mode golden files (initially `@EXPECT_ERROR`/ignored, flipped
    per phase): `tests/scripts/35-nested-match.loft` (P1),
    `35b-sequence-capture.loft` (P2), `35c-multi-pattern-arm.loft` (P3),
    `35d-alternation.loft` (P4), `35e-optional.loft` (P5), `35f-repetition.loft`
    (P6), `35g-iterator-match.loft` (P6/L3.6). *Verify:* each file exists and is red.

**Exit:** README reconciled; D2 confirmed by probe; 7 red golden files committed.

---

### Phase 1 — L2: nested sub-patterns + the recursive `parse_pattern` backbone

**Goal.** `match e { Neg { inner: Num { v } } => … }` and
`match xs { [a, Inner { x }, c] => … }` work on both backends.

**Design.** Two moves: (a) *behavior-preserving* extraction of a recursive
`parse_pattern()` (D3) that reproduces today's lowering byte-for-byte; then (b)
*additive* — route struct/struct-enum field positions and slice element positions
through it so a sub-pattern recurses.

**Code points.**
- `src/parser/control.rs:3554` `parse_field_sub_pattern` → generalize into
  `parse_pattern`; the guard at `:3556` (`Type::Enum(_, false, _)`) is the exact spot
  to also accept `Type::Enum(_, true, _)` (struct-enum) and struct `Reference` fields
  by recursing instead of falling to the scalar path at `:3633-3636`.
- `src/parser/control.rs:3403` `parse_match_enum_field_bindings` — field with `:`
  already routes to the sub-pattern parser at `:3437-3439`; point it at the new
  recursive entry.
- `src/parser/control.rs:3960` `parse_vector_match` element loop `:3986-4007` —
  replace the bare `has_identifier()` element read at `:3992` with a `parse_pattern`
  call per element; element binding at `:4055-4079` consumes `PatternMatch.binds`.
- Capture typing: binding type is read from the variant attribute at
  `control.rs:3425-3432` and assigned at `:3442`; heap bindings get the
  `Deps::frame1(src)` borrow-dep rewrite at `:3490-3508` — recursion must preserve
  both so backends don't diverge on free (F1 corollary).

**Steps & verification.**
1.1 Extract `parse_pattern` with only the existing cases; wire the four handlers to
    it. *Verify (loft-codegen gate):* `loft introspect` on a corpus of current match
    tests emits **byte-identical IR and native Rust** before/after. No behavior
    change yet.
1.2 Add the struct-enum / struct-field recursive branch (F4). *Verify:*
    `35-nested-match.loft` nested-enum arm green on both backends; leak-clean.
1.3 Add per-element recursion in slices. *Verify:* `[a, Inner { x }, c]` arm green
    both backends.
1.4 Parser diagnostics: nested depth is unbounded but well-formed; a malformed nested
    pattern gives a clean error, not a panic. *Verify:* add
    `tests/parse_errors.rs::nested_pattern_*` cases.

**Exit:** nested patterns work both backends; `parse_pattern` is the single pattern
entry; IR-identical refactor proven; `35-nested-match.loft` in the suite.

---

### Phase 2 — L3.1: sequence patterns + named sub-pattern capture + `...rest`

**Goal.** A slice arm reads like a parser rule:
`[ first:Ident, mid:Expr, ...rest ] => …`, with `rest` bound to the sub-slice.

**Design.** Generalize the slice grammar from "bare names + one `..` marker" to a
sequence of `parse_pattern` elements, each optionally `name:pat`, with **one** real
named-rest capture. `...rest` binds `subject[k..len-t]` as a sub-vector (new: F5).
Add the totality gate (F6): a slice arm using a sequence/rest is non-total → require a
trailing `_` or a full-cover set.

**Code points.**
- `src/parser/control.rs:3960` `parse_vector_match` — the head/tail `Vec<String>`
  (`:3983-3984`) becomes `Vec<PatternMatch>`; length test `:4010-4052` unchanged for
  fixed arity, `<=` branch (`:4013`) generalizes to "≥ min fixed elements" when a rest
  is present.
- Named-rest: new binding that materializes `subject[k..len-t]` — model the slice
  read on `OpGetVector` (`:4063`) + `OpLengthVector` (`:4010`); the sub-vector
  construction reuses the vector-build path (cross-check against how `[...]` literals
  build in `src/parser/vectors.rs`).
- Totality gate: add a coverage/`total` check in the vector handler's non-wildcard
  fallback at `control.rs:4141-4146` (today it silently returns typed null).

**Steps & verification.**
2.1 `name:pat` capture in element position. *Verify:* `35b-sequence-capture.loft`
    binds each named element, both backends.
2.2 `...rest` sub-slice capture. *Verify:* `rest` equals `subject[k..len-t]` by
    value **and length**; leak-clean on both backends (heap sub-vector must free
    correctly — assert store balance).
2.3 Totality gate + diagnostic. *Verify:* a sequence arm with no trailing `_`
    produces the "non-exhaustive — add a `_`" error (`tests/parse_errors.rs`).

**Exit:** parser-rule-shaped arms with captures + `...rest` work both backends,
leak-clean; totality enforced.

---

### Phase 3 — L3.7: multi-pattern arms (comma-separated patterns)

**Goal.** One arm, several shapes; first match commits its captures.
```
match input { [Verb { v }, Obj { o }], Parsed { verb: v, object: o } => dispatch(v, o), … }
```

**Design.** Purely additive over Phase 1 (README: "no new cursor work"). Each listed
pattern compiles via `parse_pattern` as today; the arm becomes an OR over the
patterns' `cond`s, committing the matching pattern's `binds`. Names present in every
pattern with a common type bind at that type; names in only some become `option<T>`
(same rule as alternation — reuse the Phase-4 unify helper, or land D-simple: require
identical name sets in P3 and defer partial-overlap to P4).

**Code points.**
- Arm loop in `parse_match` `control.rs:2714-3133`: accept a comma-separated pattern
  list before `=>`; build the arm as nested `If(cond_i, {binds_i; body}, next_pattern)`.
- Exhaustiveness (F6): arm counts as total only if *every* listed pattern is total
  (`control.rs:3138-3156` coverage set — union across the listed patterns).

**Steps & verification.**
3.1 Two-shape arm, identical name sets. *Verify:* `35c-multi-pattern-arm.loft` binds
    from whichever shape matches; first-match commit (later patterns not tried) —
    both backends.
3.2 Exhaustiveness union. *Verify:* a multi-pattern arm that jointly covers an enum
    needs no `_`; a partial one does.

**Exit:** multi-pattern arms work both backends; first-match commit verified.

---

### Phase 4 — L3.2: alternation `(a | b)` + capture unification + slice cursor

**Goal.** `[ (Ident { n } | Str { n }), rest… ]` — ordered choice with capture
promotion.

**Design.** Introduce the **slice cursor desugaring** (D4, slice half): a `pos: usize`
temp; anchor = `save = pos`; try `a`; on its `cond` false, `pos = save` and try `b`;
commit = drop save. All expressible as `If` + `Set` + `OpGetVector` — **no new
opcode** (this is the F2 payoff). Capture unification: same-name across alternatives
→ unify types (compile error if incompatible); different names → each promotes to
`option<T>`.

**Code points.**
- `parse_pattern` gets the alternation case: parse `( … | … )`, emit the
  anchor/try/revert `If` structure over the shared `pos` temp.
- Capture typing hook: **before** `create_unique` at `control.rs:3442`, run a unify
  pass over the branch captures — same-name uses `match_arm_types_unify`
  (`control.rs:379`); missing-in-some-branch wraps in `Type::optional(...)` (helper
  used at `:3260`).
- Longest-partial error position (README open-Q #3, design target): track the max
  `pos` reached across reverts in a temp; feed the match-fail error. *Optional in P4,
  can slip to a follow-up.*

**Steps & verification.**
4.1 Same-name alternation unifies. *Verify:* `35d-alternation.loft` binds `n` at the
    unified type; both backends.
4.2 Different-name alternation promotes to `option<T>`. *Verify:* absent branch's
    capture reads null.
4.3 Backtracking correctness. *Verify:* an alternative that partially matches then
    fails leaves `pos` reset (assert the following element still matches) — the
    core PEG invariant, on both backends.

**Exit:** alternation with capture unification + slice backtracking, both backends,
no new opcodes.

---

### Phase 5 — L3.3: optional `(...)?`

**Goal.** `( Else { body } )?` → `body: option<block>`.

**Design.** Degenerate alternation: `anchor; try a; on fail revert and bind all of
`a`'s captures to null`. Reuses Phase-4 cursor + the `Type::optional` promotion.

**Code points.** `parse_pattern` optional case; capture nullable-promotion at the same
hook as 4.2 (`control.rs:3442`, `Type::optional`).

**Verification.** `35e-optional.loft`: present → `Some`-shaped; absent → null capture;
following elements still match after a skipped optional (cursor intact); both backends.

**Exit:** optional groups with nullable captures, both backends.

---

### Phase 6 — L3.4: repetition `(...)*` / `(...)+` + separator

**Goal.** `[ (args:Expr *(Comma))? ]` → `args: vector<Expr>`.

**Design.** A **bounded loop** over the slice index: each iteration anchors, tries the
body, on fail reverts and breaks, on success appends captures to an accumulator
vector. `+` = one mandatory body then `*`. Separator consumed but not captured. For a
slice the loop bound is `len` (termination guaranteed). Build the loop IR modeled on
how `for`/`while` construct loop `Value`s in `src/parser/collections.rs` (verify the
exact constructor there during the phase).

**Code points.**
- `parse_pattern` repetition case: emit index-cursor loop + accumulator-vector build
  (reuse vector-build from `src/parser/vectors.rs`).
- Capture typing: repetition capture wraps element type in `Type::Vector(...)` at the
  `control.rs:3442` hook.
- Totality: repetition arm is non-total → the Phase-2 gate applies.

**Steps & verification.**
6.1 `(p)*` collects into a vector. *Verify:* `35f-repetition.loft` — count + values +
    **length** + **leak** (accumulator frees) on both backends.
6.2 `(p)+` requires ≥1; empty input fails the arm.
6.3 Separator `*(Comma)` consumed not captured. *Verify:* separators absent from the
    result vector; trailing-separator behavior pinned by a probe.

**Exit:** repetition with separators + vector capture, both backends, leak-clean.

---

### Phase 7 — L3.6: iterator inputs (the only phase with new opcodes)

**Goal.** `match some_iter { … }` with backtracking over a non-random-access source.

**Design.** An iterator cursor can't reset an index; it must **buffer pulled items**
while an anchor is live — exactly `Lexer::memory` + the `links` refcount
(`src/lexer.rs:104,106,1371,1385`). Add:
- `OpMatchAnchor` / `OpMatchRevert` opcodes.
- `anchors: Vec<(u32 pos, u32 epoch)>` + a pulled-item memo on `State`
  (`src/state/mod.rs`, modeled on `call_stack`/`coroutines` at `:164,170`), mirrored
  in native (`src/codegen_runtime.rs`, since native ops can't reach `State`).
- A `max_lookahead` arm attribute bounding the buffer (README L3.6), so an infinite
  iterator with an always-matching body can't hang.

**Add-opcode checklist** (from the code map; the *only* new-op work in the plan):
| # | Touch point | File | Action |
|---|---|---|---|
| 1 | Define | `default/01_code.loft` (near `:83-96`) | `fn OpMatchAnchor(...);` + `fn OpMatchRevert(...);` with `#rust"…"` templates. `Op`+uppercase name = auto-registered (`src/data.rs:2847`); op_code auto-assigned at parse (`src/parser/definitions.rs:1286`). |
| 2 | Interp impl | `src/fill.rs` (@generated) | **`make fill`** regenerates from the templates; CI `tests/issues.rs::fill_rs_up_to_date` guards drift. Never hand-edit. |
| 3 | Compile/emit | `src/state/codegen.rs:2986` (generic op path) | Automatic *iff* standard stack effects. These ops touch the anchor shadow stack → add a special-case arm modeled on `OpCoroutineYield` at `codegen.rs:2887`. |
| 4 | Native | `src/generation/ops/mod.rs:148` (`build_registry`) + `src/codegen_runtime.rs` | Templates that touch `State` aren't native-valid → register custom `OpEmitter`s + native runtime helpers (model on `OpFreeRef` at `codegen_runtime.rs:353`). |
| 5 | Native-gate | `src/native_gate.rs` | If the memo can't be native at all, model iterator-match as a denylisted `Value` variant (like `Value::Yield` at `:243`). Prefer keeping it native. |

**Steps & verification.**
7.1 Opcodes + shadow stack, interp first. *Verify:* `35g-iterator-match.loft` matches
    over an `iterator<T>` with backtracking; buffer replays pulled items on revert.
7.2 Native parity. *Verify:* identical result `--native`; `make fill` fresh; clippy
    clean; native gate correct.
7.3 `max_lookahead` bound + termination. *Verify:* an always-matching body over an
    unbounded iterator errors at the bound instead of hanging (`loft --timeout`).
7.4 Side-effecting-iterator + infinite-repetition are documented limitations
    (README "What would not work") — add a `CAVEATS.md` note.

**Exit:** iterator-input matching both backends, bounded, leak-clean; the two opcodes
freshness-checked.

---

## 5. Cross-cutting verification & docs

- **Cross-mode harness.** Every `35*.loft` runs on `--interpret` and `--native`
  under the leak gate (see loft-test skill). Parser diagnostics go in
  `tests/parse_errors.rs`.
- **Rust unit tests** where a helper deserves isolation (e.g. `...rest` sub-slice
  math, capture-type unify).
- **Docs synced per phase:** `LOFT.md` § Match (syntax), `INTERMEDIATE.md` (the two
  L3.6 opcodes only), `CAVEATS.md` (iterator limits), `INCONSISTENCIES.md` #26
  (totality precedent), and this plan's Status table.
- **Freshness:** `make fill` after Phase 7; `make ci` green each phase; the only
  expected non-green is the known-flaky `wasm_debug_relay`.

## 6. Risk register

| Risk | Phase | Mitigation |
|---|---|---|
| D2 wrong (slice backtracking *does* need an op) | 0 | Falsification probe 0.2 **before** any feature code. |
| `parse_pattern` refactor regresses live match | 1 | loft-codegen gate: byte-identical IR/native proof (1.1) before adding cases. |
| Heap captures (`...rest`, repetition vec) leak or diverge across backends | 2,6 | Assert store balance + length on both backends; preserve `Deps::frame1` borrow-dep rewrite (`control.rs:3490-3508`). |
| Repetition loop-IR construction harder than assumed | 6 | Verify the `for`/`while` IR constructor in `collections.rs` at phase start; if costly, land `*`/`+` as a bounded unroll fallback. |
| Iterator memo native parity | 7 | Interp-first, then native; custom `OpEmitter` + `codegen_runtime` helper; native-gate fallback if truly non-native. |

## 7. Open questions (carry from README, decide at the phase that needs them)

1. Explicit PEG commit points (`~`/`!~`) — deferred; revisit if deep-revert errors
   are unhelpful (Phase 4).
2. Longest-partial-match error reporting — design target Phase 4, may slip.
3. Partial-name-overlap unification in multi-pattern arms — P3 requires identical
   name sets; full overlap rule lands with P4's unify helper.
4. Positional tuple-variant sugar — PERMANENTLY out ([C89](../../DESIGN_DECISIONS.md)), never
   promoted: tuple variants force match-to-read + a mitigation-syntax sprawl; loft reads by name.

## 8. See also

- [README.md](README.md) — design/rationale (the "why").
- loft-codegen skill — the IR-identical refactor gate (Phase 1) + add-opcode method (Phase 7).
- `src/parser/control.rs` — the entire pattern surface lives here.
- `src/lexer.rs` §`link`/`revert` — the anchor primitive L3.6 mirrors.
