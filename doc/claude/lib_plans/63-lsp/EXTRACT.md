<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Extract-function — design (@PLN63 LSP.2 `refactor.extract`)

> **Identity:** a design sub-doc of `@PLN63` (loft-lsp), the `refactor.extract`
> table row. Slug `extract-function`.
> **Status:** design — written before code, so the data-flow engine (the whole
> cost) is scoped against fixed parser code points and lands in small, safe,
> individually-verifiable steps. No code yet.

## Goal

Turn a SELECTION of whole statements inside one function into a new function, and
replace the selection with a call to it. The protocol side is trivial (a
`CodeAction` carrying a `WorkspaceEdit`); the substance is the **intra-function
data-flow engine** that decides the new function's *signature*:

- locals **read before written** inside the selection → **parameters** (the values
  flow in),
- locals **written inside** and **read after** the selection → **return values**
  (loft tuples cover several; a var that is both → an in-out param+return),
- locals declared *and* dead within the selection → plain locals of the new fn.

The extracted body is the selected SOURCE TEXT verbatim (variable names are
preserved, so params/locals resolve unchanged) plus a synthesized `return (…)`;
the call site is `(outs) = name(ins)`. Text in / text out — a `WorkspaceEdit`,
never an IR re-serialization.

## Decision 1 — analyse the committed IR, don't re-derive

The engine reads the ALREADY-parsed function body + variable table (the parser
computed both); it does **not** re-implement scope/type resolution. Every fact it
needs is in structures the LSP already has after `parse_lsp_buffer`:

- **the body IR** — `Definition.code: Value` (`src/data.rs:2678`), a
  `Value::Block(Block{ operators: Vec<Value>, … })` (`src/data.rs:464`) whose
  `operators` are the statements;
- **reads / writes** — the `Value` enum (`src/data.rs:476`): a READ is
  `Value::Var(v)`, a WRITE is `Value::Set(v, expr)` (a compound `x += 1` lowers to
  `Set(v, Op(Var(v), …))` — a read of `v` in the RHS *before* the write, which the
  data-flow must see); calls `Call`/`CallRef`, control `If`/`Loop`/`Return`;
- **positions** — statements are wrapped `Value::Span(Box<(Position, Value)>)`, so
  a source-line selection maps to a contiguous slice of `operators`;
- **types** — `Definition.variables: Function` (`src/variables/mod.rs`):
  `count()`, `name(v)`, `tp(v)` / `var_type(v)`, `is_argument(v)` give each
  input/output its declared type (rendered by `Data::type_name_str`, the same
  spelling hover / inlayHint use).

**Reuse, don't rebuild the walkers.** `use_analysis.rs` already traverses this IR:
`collect_uses` (`:675`, reads) and `collect_defs` (`:1514`, writes) are the read/
write collectors; `collect_last_set` (`:2971`) finds a var's last write. The
data-flow is these walkers restricted to a statement RANGE + a live-out query over
the tail.

## Decision 2 — the data-flow (upward-exposed uses / liveness)

For a linear statement slice `S = operators[a..=b]` inside function `f`:

- **inputs(S)** = vars with an *upward-exposed use* in `S`: a read of `v` in `S`
  not preceded (within `S`) by a full write to `v`. (Scan `S` top-down keeping a
  `written` set; a `Var(v)` read with `v ∉ written` marks `v` an input; a
  `Set(v,…)` adds `v` to `written` *after* its RHS is scanned.) A parameter of `f`
  read in `S` is an input like any other local.
- **outputs(S)** = vars written in `S` (`collect_defs` over `S`) that are
  **live-out**: read on some path in `operators[b+1..]` (the function tail after
  the selection). Liveness over the tail reuses `collect_uses`.
- **in-out** = `inputs(S) ∩ outputs(S)` — a param that the new fn also returns.
- **selection-local** = written in `S`, declared in `S` (its declaration is in `S`
  — the @PLN115 `Occurrence.declaration` flag, or a `Set` with no earlier read),
  and NOT live-out → stays a plain local, neither param nor return.

`self` (a method's receiver, `is_argument(0)` on a method) read in `S` is an input
named `self` — the new fn is a free function taking the receiver by value/ref.

The @PLN115 **resolution index** (`src/resolution.rs`) is a cross-check, not the
engine: it maps the selection's byte range to the exact bindings and flags
declarations, useful to confirm the IR-derived var set and to place the edit
precisely — but the read/write *nature* comes from the IR (`Var` vs `Set`), which
the index does not carry.

## The chokepoints (code points the engine reads / writes)

| Concern | Code point | Gives |
|---|---|---|
| Enclosing function of the selection | `lsp.rs::enclosing_fn` (top-level fn whose body spans the cursor) | the `f` def_nr whose `code` + `variables` we analyse |
| Statement slice from a line range | `Definition.code` → `Value::Block.operators`, unwrapping `Value::Span` positions (`data.rs:464,476,2678`) | `operators[a..=b]` = the selection |
| Reads / writes over the slice | `use_analysis::collect_uses` (`:675`), `collect_defs` (`:1514`) | the var read/write sets to classify |
| Live-out over the tail | `collect_uses` on `operators[b+1..]` | which outputs escape |
| Input / output types | `Definition.variables` (`variables/mod.rs`: `tp`/`var_type`/`is_argument`) + `Data::type_name_str` | the new signature's types |
| Signature synthesis | `api_surface::signature_of` / `type_name_str` (already `pub`) | one type spelling, matching hover |
| Body text | the open buffer (selected lines, verbatim) | the new fn body + the replaced call |
| Protocol | `loft-lsp.rs::code_actions` + a new `lsp::extract_function` (kind `refactor.extract`) reusing the rename `WorkspaceEdit` builder | the two edits (insert fn, replace selection) |

## Small, safe steps

Each step is independently landable behind a protocol-level + unit gate. The engine
grows analysis-first; nothing emits an edit until the signature it computes is
proven correct.

- **E0 — protocol stub — DONE.** `codeActionProvider` now advertises
  `codeActionKinds: ["quickfix", "refactor.extract"]` (was a bare `true`); the
  `code_actions` handler offers a `CodeAction{kind:"refactor.extract", title:"Extract
  to function"}` with a NO-OP edit on a MULTI-LINE selection (the discriminator that
  leaves single-line quick-fix requests untouched). Refactored the quickfix path into
  `quickfix_from_diagnostic` so the two action sources compose. *Gate met:*
  `lsp_transport::extract_function_offered_on_a_multiline_selection` (real binary — the
  kind is advertised; offered on a 2-line selection, absent on a bare cursor).
- **E1 — selection → statement slice — DONE.** `lsp::extract_range(text, name,
  stdlib_dir, start_line, end_line) -> Option<(fn_def, first_op, last_op)>`: resolves
  the enclosing top-level fn (`enclosing_fn`), then maps the line range to the
  contiguous TOP-LEVEL statement operators via the body block's **`Value::Line(n)`
  markers** (NOT `Span` — a plain `a = 1` carries no Span; only fault-prone
  expressions do. `Line` markers precede every statement, and a nested block's
  markers live INSIDE its nested `Block`, so a selection inside one maps to no
  top-level statement and is refused). Refuses a start that is not a statement
  boundary, a reversed/empty range, a non-block (`#rust`) body, and a nested-block
  interior. *Gate met:* `tests/extract.rs` (a slice maps + widens with more
  statements; the signature line / closing brace / reversed range / past-EOF refuse;
  a line inside a `for` body refuses while the whole `for` maps).
- **E2 — inputs (parameters) — DONE.** `lsp::extract_inputs(…)` runs the ordered
  upward-exposed-use scan over the slice → the input names in first-use order. The
  walk special-cases the var-READ variants `for_each_child` does not surface (`Var`,
  `TupleGet`, `CallRef`, `TuplePut`), reads a `Set`'s RHS BEFORE marking its target
  written (so `x += 1` reads-before-writes), and treats writes inside an `if` branch /
  loop body as NOT definite (conservative — an unnecessary param, never a missed one);
  every other variant recurses via `for_each_child` (safe on all variants). *Gate met:*
  `tests/extract.rs` — `{ y = x + 1; z = y * 2 }` → `[x]`; narrowing to `z = y * 2` →
  `[y]`; `x = x + 1` → `[x]`; two write-first locals → `[]`.
- **E3 — outputs (returns) — DONE.** `lsp::extract_outputs(…)` = writes-in-slice
  (all paths, `collect_writes`) ∩ live-out-over-tail (`collect_reads` on
  `operators[last_op+1..]`), in write order; a var in both inputs and outputs is
  in-out. Conservatism flips from E2 — outputs are a SUPERSET (any write ∩ any tail
  read), so a modification the caller might need is never dropped. Both collectors
  special-case the same var-read/write variants and delegate the rest to
  `for_each_child`. *Gate met:* `tests/extract.rs` — extracting the `for` where
  `total` is read after → `total` is an output AND an in-out param; a dead write is
  excluded; two live-out writes → two outputs (a tuple return).
- **E4 — synthesize + edit.** Build `fn <name>(ins) -> outs { <selected text>
  return (outs) }` (single output → `-> U` / `return o`; none → `-> void`, no return;
  tuple for ≥2), and the call `(outs) = <name>(ins)`; emit a `WorkspaceEdit` (insert
  the fn after the enclosing fn, replace the selection with the call) reusing the
  rename edit builder. *Gate:* `lsp_transport` — extract a simple selection; assert
  the two edits, then RE-PARSE the applied text and assert it is diagnostic-clean and
  runs identically on both backends (the extraction is behaviour-preserving).
- **E5 — ownership / deps correctness.** An input that is a heap value
  (vector / struct / text — `tp(v).heap_dep().is_some()`) must pass with the SAME
  ownership the original had (by-value copy vs `&`-borrow), per
  [OWNERSHIP_MODEL.md](../../OWNERSHIP_MODEL.md); an in-out heap var returns its
  store. v1 REFUSES what it cannot prove safe (a `&`-param captured in the slice, a
  borrowed view whose base is outside the slice) with a clear message, rather than
  emit a subtly-wrong edit. *Gate:* extract a slice over a `vector<T>` local →
  correct ownership on both backends (`LOFT_STORES=warn` / `LOFT_NATIVE_LEAK_CHECK`
  clean); the refused cases return a diagnostic-carrying `CodeAction`-absence.
- **E6 — `self` + refusals.** `self` becomes a leading input; refuse a selection
  that (a) lives in a `#rust` / `#native` body, (b) contains a `Return` / `break` /
  `continue` / `yield` targeting outside the slice (would change control flow), or
  (c) reads a variable declared *after* the selection. Each refusal is a clear
  message, never a broken edit. *Gate:* unit — each refusal case yields `None` +
  reason; a method extraction threads `self`.

E0–E6 complete `refactor.extract` for the common straight-line + single-loop case.
Follow-ups (own steps): extracting an EXPRESSION (not whole statements); a selection
spanning nested blocks; renaming a captured local that collides in the new scope.

## Synthesis shapes (worked)

```loft
// before — extract the two marked lines out of `f`
fn f(w: integer) -> integer {
  total = 0
  for i in 0..w {          //┐ selection
    total = total + i * i  //┘
  }
  return total
}
// after
fn sum_squares(total: integer, w: integer) -> integer {
  for i in 0..w {
    total = total + i * i
  }
  return total
}
fn f(w: integer) -> integer {
  total = 0
  total = sum_squares(total, w)
  return total
}
```

`total` is in-out (read-before-write via `+=` and live-out), `w` is an input, `i`
is selection-local. Multiple outputs return a tuple; the call destructures
`(a, b) = name(…)`.

## Refusals (safety — an extraction never corrupts)

- a selection that is not a whole-statement contiguous slice (E1);
- a `#rust` / `#native` body (E6);
- control flow escaping the slice (`return`/`break`/`continue`/`yield`) (E6);
- an input whose ownership can't be preserved (E5);
- a name the new function's scope would collide with (follow-up).

Each is `None` (no action offered) or an action whose title states why — never a
silent, wrong edit.

## Risks + open questions

| Risk / question | Handling |
|---|---|
| Read/write nature not in the resolution index | Derive it from the IR (`Var` vs `Set`); the index is a position cross-check only |
| Ownership of a heap input (copy vs borrow) | E5 — honour the deps model; REFUSE the unprovable case in v1 (never a wrong edit) |
| Control flow (`return`/`break`) in the slice | E6 — refuse; a follow-up could rewrite `return x` → an output |
| Compound assignment (`x += 1`) read-before-write | The IR lowers it to `Set(v, Op(Var(v),…))`; the RHS-first scan sees the read |
| Where to place the new function | After the enclosing fn (v1); a project style pass can reorder later |
| Naming the new function | v1 uses a placeholder (`extracted`) the user renames; a `rename`-follow-up chains cleanly |

## See also

- [README.md](README.md) — the LSP.2 surface; this is the `refactor.extract` row.
- [@PLN115 resolution index](../../plans/115-resolution-index/README.md) — the
  position→binding + declaration cross-check.
- [OWNERSHIP_MODEL.md](../../OWNERSHIP_MODEL.md) — the deps north-star E5 honours.
- `use_analysis.rs` — the IR read/write walkers the engine reuses.
