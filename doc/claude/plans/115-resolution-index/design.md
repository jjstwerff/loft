<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN115 — Design (Phase 0)

Detailed design with concrete code points and small, safe, individually-landable
steps. The guiding constraint: **the parser is loft's #1-weakness subsystem, so
every step must leave the compile path byte-identical when recording is off**, and
recording must be a pure side-append that changes no parse decision.

## Decision 1 — instrument, don't re-resolve

**Gated instrumentation at the parser's existing resolution chokepoints**, not a
separate resolver pass. Rationale: the parser ALREADY computes each binding; a
separate pass would re-implement loft's scope + type resolution — a second source
of truth that drifts (the stability red-flag: two readers of one IR). The cost of
instrumentation is contained because the positions are *already in scope* at every
chokepoint (see below), and the record is a pure append behind a bool gate.

## The chokepoints (positions are already threaded)

Every site below already has the identifier's `&Position` in scope — no new
plumbing to reach the position; only a record call to add.

| Resolution | Code point | Already in scope | Records |
|---|---|---|---|
| **Local / param** (`x`) | `parser/objects.rs::parse_var` — the `Value::Var(v_nr)` sites (`objects.rs:302,305,314,350`) and the create-new-var sites (`455,506`) | `name_pos: &Position` (param), `v_nr`, `self.context` (enclosing-fn def_nr, `mod.rs:213`) | `Local { fn_def: self.context, var_nr }` |
| **Call → fn/method** | `parser/mod.rs::call` → `self.data.find_fn(source, name, dispatch_tp)` (`mod.rs:2939`); entered via `parse_call(…, name_pos)` | `name_pos` (threaded in the fix-b work), the `find_fn` result def_nr | `Global(def_nr)` or `Method { recv_type, method_def }` |
| **Constant / type name** | `parser/objects.rs:191` `parse_constant_value(…, name_pos, …)` | `name_pos`, the resolved def | `Global(def_nr)` |
| **Field / method access** (`expr.f`) | `parser/fields.rs` field/method dispatch (the `get_field` / `find_fn` on `parent_tp` region, ~`fields.rs:187+`) | the field name + its position, `parent_tp` (receiver type) | `Field { type_def, attr }` / `Method { recv_type, method_def }` |

`parse_var` (`objects.rs:65`) is the PRIMARY chokepoint — it is the single dispatch
that decides whether an identifier is a local, a call, a constant, or a `self`
field. Phase 1 hooks it; later phases add fields.rs + the call site.

## Decision 2 — the `Resolution` model

A new `pub mod resolution` (`src/resolution.rs`), so both the parser and
`loft::lsp` can name it:

```rust
pub struct Occurrence {
    pub pos: Position,     // start of the identifier (as the chokepoint already has it)
    pub len: u16,          // char length of the name (for the reference range)
    pub res: Resolution,
}

pub enum Resolution {
    Global(u32),                                 // def_nr — fn/type/struct/enum/const/interface
    Local { fn_def: u32, var_nr: u16 },          // type via def(fn_def).variables().tp(var_nr)
    Field { type_def: u32, attr: u16 },          // expr.field
    Method { recv_type: u32, method_def: u32 },  // expr.method(…)
    Unresolved,                                  // C80 value-undefinedness / a typo
}
```

`Local` stores `(fn_def, var_nr)` — the IDENTITY of a binding — not a name. Two
same-named locals in different functions have different `(fn_def, var_nr)`, so
find-references / rename become exact and shadowing-correct. Type/name are looked up
on demand, not duplicated.

## Decision 3 — storage + the zero-cost-off invariant

On `Parser` (`src/parser/mod.rs` struct):

```rust
record_resolutions: bool,     // gate — DEFAULT false; only the LSP parse sets it
resolutions: Vec<Occurrence>, // cleared per parse (like deferred_unknown)
```

Recording goes through ONE helper so the gate lives in exactly one place:

```rust
fn record(&mut self, pos: &Position, len: u16, res: Resolution) {
    if self.record_resolutions {
        self.resolutions.push(Occurrence { pos: pos.clone(), len, res });
    }
}
```

**Zero-cost-off invariant:** with `record_resolutions == false` (every normal
compile, every existing test), `record` is a single predictable branch that returns.
No allocation, no behavior change. This is the non-negotiable property each step
proves (below).

## The query API

Parser side (after `parse_source`):

```rust
impl Parser { pub fn resolutions(&self) -> &[Occurrence] { &self.resolutions } }
```

`loft::lsp` consumers (new thin wrappers, replacing the lexical ones):

- `resolution_at(occurrences, line, col) -> Option<&Occurrence>` — the record whose
  `[pos, pos+len)` span contains the cursor.
- `occurrences_of(occurrences, binding) -> Vec<Position>` — every occurrence with the
  SAME `Resolution` identity (same `def_nr`, or same `(fn_def, var_nr)`). This is the
  exact, non-lexical find-references / rename set.

## Small, safe steps

Each step is individually landable, gated OFF by default, and its **acceptance gate
includes a byte-identical-IR check** (§ below). No step changes a parse decision.

- **S1 — scaffold, inert. ✅ DONE (`34c428c0`).** Add `pub mod resolution` (the enum + `Occurrence`), the
  `record_resolutions` + `resolutions` fields (default false / empty), the `record`
  helper, `Parser::resolutions()`, and clear `resolutions` where `deferred_unknown`
  is cleared (`parser/mod.rs` `parse_str`). No hook yet — nothing calls `record`.
  *Gate:* builds; full suite green; `loft introspect` corpus byte-identical (trivially,
  no call sites).
- **S2 — one hook: locals in `parse_var`. ✅ DONE (`da8bcca2`).** *Implemented at the
  pass-2 `name_exists` chokepoint* rather than a `resolve_var` wrapper: on pass 2 every
  local occurrence (write target, read, `return`) flows through `name_exists` because
  the var was created on pass 1, so a single `record(name_pos, name.chars().count(),
  Local{fn_def: self.context, var_nr: index_var})` there — gated on `!self.first_pass` +
  `record_resolutions` — captures the full occurrence set. *Gate met:* with the gate OFF,
  byte-identical IR (S1 vs S2 binary, empty `loft introspect` diff); with the gate ON,
  a local's occurrences are recorded and two same-named locals in different fns get
  distinct `(fn_def, var_nr)` (`tests/resolution_index.rs`).
- **S3 — enable the gate for the LSP parse only. ✅ DONE (`b85e84da`).** Factored the
  7 duplicated `Parser::new()+load_stdlib+parse_source` blocks into `parse_lsp_buffer`,
  which sets `record_resolutions = true` *after* `load_stdlib` (only the user buffer
  records — the warm-cache path doesn't re-parse the stdlib). Exposed via
  `lsp::resolutions(text, name, stdlib_dir)`. *Gate met:* the LSP parse carries
  occurrences; the CLI/compiler parse never routes through here → `loft introspect`
  byte-identical to S2; all LSP suites green (behavior-preserving refactor).
- **S4 — first consumer: precise LOCAL references/rename.** Replace F-v1's block-scan
  for a local with `occurrences_of(Local{…})`. *Gate:* renaming a local touches exactly
  its binding's occurrences, shadowing-correct; `tests/lsp_scope.rs` extended.
- **S5 — hook globals + calls.** Record `Global`/`Method` at `mod.rs::call` (the
  `find_fn` result) and constants at `objects.rs:191`. *Gate:* byte-identical off;
  method references (`text.len`) now exclude other types' `len`.
- **S6 — hook field/method access** (`parser/fields.rs`). *Gate:* completion's `expr.`
  receiver + hover resolve via the index.
- **S7 — unblock E (inlayHint).** With reliable binding positions from the index, emit
  inferred-type hints at local bindings (`var_type` → `type_name_str`). *Gate:* the
  E gate that couldn't be met before now passes for every local, not just the first.

S1–S4 are the minimal spine that proves the mechanism + delivers the first precision
win. S5–S7 extend coverage. Stop after any step with value banked.

## The byte-identical-IR gate (every step)

The one procedure that guards the parser (per [CODEGEN_METHOD](../../CODEGEN_METHOD.md)
Mode B): build a one-fn-per-path corpus once, capture `loft introspect corpus.loft >
before.txt` on `main`, and after each step (gate OFF) capture `after.txt` and
`diff before.txt after.txt` — **an empty diff is the proof** the recording changed
nothing emitted, on both backends. A non-empty diff with the gate off is the bug;
bisect by step, don't push through.

## Open questions carried into execution

1. **Which parse pass records?** loft parses twice (`parse_str`); pass 2 has resolved
   types. Record on pass 2 (types known) and clear pass-1's records, mirroring how
   `deferred_unknown` is handled — decide in S2 against the two-pass dedup.
2. **Granularity.** S1–S4 record locals (the ambiguous case); globals (S5) are already
   name-precise, so recording them is for method disambiguation + a uniform API, not
   correctness — keep if the byte-identical gate stays clean, drop if it adds weight.
3. **Storage lifetime.** Per-parse on `Parser` (the LSP re-parses anyway). Do NOT persist
   in `Data`/the stdlib cache bundle (would grow @I86's round-trip for no LSP gain — the
   stdlib's own occurrences are never queried).

## Risk register

| Risk | Mitigation |
|---|---|
| Recording perturbs a parse decision | `record` is append-only behind a bool; the byte-identical-off gate on every step |
| Compile-path cost when off | One predictable branch; no allocation on the off path; `resolutions` stays empty |
| Wrong parse pass → pre-resolution types / dupes | Record on pass 2, clear pass-1 (S2 open question 1) |
| Method dispatch has several `find_fn` sites (`mod.rs:2939,3916,3930,4034`) | Hook the ONE user-call chokepoint (`call`/`parse_call`); leave internal/stdlib re-dispatch unrecorded |
