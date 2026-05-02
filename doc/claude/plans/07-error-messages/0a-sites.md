<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 0a — Error site inventory

Generated 2026-04-28 by `rg -n
'diagnostic!|specific!|Level::Error|Level::Fatal|panic!|unreachable!|\.expect\('
src/ --type rust > 0a-sites.txt`.

Raw output: [0a-sites.txt](0a-sites.txt) — 815 hits across 52 files.

## Pattern frequency (whole-tree)

| Pattern | Hits | Carries `Position`? |
|---|---:|---|
| `diagnostic!` / `pos_diagnostic!` | 326 | yes — every call passes a `Position` |
| `Level::Error` / `Level::Fatal` (incl. `add_at` / `add` calls) | 354 / 21 | mostly — `add_at` does, `add` does not |
| `specific!` | 6 | yes — wraps `pos_diagnostic!` |
| `panic!` | 109 | **no** — text-only, no source attribution |
| `unreachable!` | 21 | **no** — interpreter-invariant fail-fast |
| `.expect("…")` | 62 | **no** — text-only |

## By category (the phase-1 worklist)

| Category | Files | Sites | Has Position? | Notes |
|---|---|---:|---|---|
| Parser — token / syntax | `src/parser/{mod,definitions,collections,control,expressions,builtins,fields,objects,operators,vectors}.rs`, `src/lexer.rs` | 602 | **yes** — `self.lexer.position` flows into every `diagnostic!` / `specific!` / `Level::Error` add_at | already span-attached; phase 6 rewrites the *messages* |
| Definitions / typedef | `src/parser/definitions.rs`, `src/typedef.rs` | 104 | partial — uses `Definition.position`, not per-token | the definition's start position is attached, but a struct-field-mismatch error in mid-definition only points at the struct opener |
| Scope analysis | `src/scopes.rs`, `src/variables/{validate,mod}.rs` | 13 | **partial** — 7 `panic!` in scope code without `Position`; 4 `diagnostic!` do attach | phase-1 worklist row: convert the 7 panics to `add_at` |
| Codegen | `src/state/codegen.rs` (20), `src/compile.rs` (?) | 20 | **rare** — most are `panic!` / `unreachable!` for interpreter invariants | should stay panics post-phase-4 (see plan rule #5) |
| Runtime fault | `src/fill.rs` | 1 explicit `panic!` (the `panic(text)` builtin at line 1709) | **no** — the rest of fill.rs returns sentinels rather than panicking | the headline gap; phase 3 + phase 4 jointly close it |
| Native codegen | `src/generation/{text,mod,dispatch}.rs` (12), `src/native.rs` (17) | 29 | partial — `crash_report.rs` recovers SIGSEGV pc + op-name but **not** loft source line | phase 3 (pc→source-line table) + the existing crash hook close it |
| Database / store internals | `src/database/*.rs` (~70), `src/store.rs` (12), `src/data.rs` (15) | 97 | partial — most are interpreter-invariant `panic!`; a few (`store.rs::store_addr` etc.) bubble up null-DbRef misuse | phase-4 candidates: null-DbRef deref, OOB store access |
| JSON / I/O | `src/json.rs` (15), `src/state/{io,text}.rs`, `src/database/io.rs` | 39 | partial — JSON returns `Option`/`enum`; many internal `panic!` are interpreter-invariants | phase 4 rewrites user-attributable JSON parse errors as `RuntimeError` |
| Documentation / introspect / wasm | `src/documentation.rs` (13), `src/introspect.rs` (2), `src/wasm.rs` (6), `src/extensions.rs` (10) | 31 | mixed — these are tooling, not user-facing runtime | mostly out-of-scope for phase 1; phase 6 polishes any user-visible CLI message |

## Files ranked by site count

| Rank | File | Sites |
|---:|---|---:|
| 1 | `src/parser/definitions.rs` | 101 |
| 2 | `src/parser/collections.rs` | 96 |
| 3 | `src/parser/control.rs` | 94 |
| 4 | `src/parser/mod.rs` | 78 |
| 5 | `src/parser/expressions.rs` | 42 |
| 6 | `src/parser/builtins.rs` | 35 |
| 7 | `src/parser/fields.rs` | 34 |
| 8 | `src/parser/objects.rs` | 33 |
| 9 | `src/lexer.rs` | 32 |
| 10 | `src/parser/operators.rs` | 31 |
| 11 | `src/parser/vectors.rs` | 26 |
| 12 | `src/state/codegen.rs` | 20 |
| 13 | `src/native.rs` | 17 |
| 14 | `src/database/types.rs` | 17 |
| 15 | `src/json.rs` | 15 |
| 16 | `src/data.rs` | 15 |
| 17 | `src/documentation.rs` | 13 |
| 18 | `src/store.rs` | 12 |
| 19 | `src/state/mod.rs` | 11 |
| 20 | `src/extensions.rs` | 10 |

(Tail: 32 files with ≤ 8 sites each, totalling 90.)

## Headline observations

1. **Parser is already fully span-attached** (~600 sites) — phase 1
   doesn't need to touch parser sites.  Phase 2 (renderer) and
   phase 6 (type-mismatch detail) are the relevant phases for the
   parser bucket.
2. **Runtime fault surface (`src/fill.rs`) has exactly 1 explicit
   panic** — the `panic(text)` builtin at `src/fill.rs:1709`.  Every
   other runtime fault either returns a sentinel or unwraps in a
   helper.  The gap is the **mapping from `pc` back to `.loft`
   source line**, not adding panic sites.  Phase 3 builds that
   mapping; phase 4 picks the small set of user-attributable kinds
   to convert from sentinel to `RuntimeError`.
3. **Scope analysis (`src/scopes.rs` + `src/variables/*`) has 7
   `panic!` calls without spans** — small, contained worklist for
   phase 1.  Convert those to `Diagnostics::add_at(position, …)`.
4. **`src/state/codegen.rs` has 18 `panic!` + 2 `unreachable!`** —
   per plan rule #5, these *should* stay panics after phase 4 (they
   represent interpreter bugs, not user errors).  Phase 3 still
   wraps them with the source-line printer so a panic shows the
   loft line that compiled to the offending op.
5. **Native codegen path (`src/generation/*` + `src/native.rs`)
   has 29 panic-style sites** — `crash_report.rs` already publishes
   `pc` + `op_name` on SIGSEGV; phase 3's pc→source-span table
   converts that into `at file:line:col`.  No new sites need adding.

## Phase-1 worklist size

Sites where phase 1 must attach a `Position` that today is missing:

| Group | Count | Locations |
|---|---:|---|
| `panic!` in scope analysis | 7 | `src/scopes.rs` + `src/variables/*` |
| Free-floating `Level::Fatal` calls without `add_at` | ~10 | tail of the 21 fatals — manual audit needed |
| Database / store user-attributable panics (null DbRef etc.) | ~5 | `src/store.rs`, `src/database/allocation.rs` |

Plus the IR-node side of phase 1: every `Value::Call`, `Value::Set`,
`Value::Iter`, division/index/field operator, struct construction
needs a `Position`.  Today only `Value::Line(u32)` exists
(`src/data.rs:291`); column + file are inferred from the surrounding
`Definition.position`.  This is a `Value`-enum layout decision, not a
site-by-site change — see phase 1 § decision A.

## Hand-off

Phase 1 picks up:
- The IR-layout decision (widen `Value::Line` to `Value::Span` vs.
  side-table).
- The ~22 panic-sites in scope / store that phase 1 converts.
- The pc→span table is phase 3's, not phase 1's.
