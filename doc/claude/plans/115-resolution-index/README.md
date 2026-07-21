<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN115 — Parse-time resolution index

**Tracker:** [loft-lang/plans#115](https://github.com/loft-lang/plans/issues/115) ·
**Subject:** loft · **Status:** future (design-first)

## Status

**Phase 0 (design) DONE — see [design.md](design.md)** (concrete code points + the
S1–S7 small-safe-step spine + the byte-identical-IR gate).

**Execution: S1 + S2 DONE.**
- **S1** (`34c428c0`) — `pub mod resolution` (`Occurrence` + `Resolution`), the
  `record_resolutions`/`resolutions` fields (default off/empty), the gated `record`
  helper, `Parser::resolutions()`, and `resolutions.clear()` paired with every
  `deferred_unknown.clear()`. Inert — nothing calls `record`.
- **S2** (`da8bcca2`) — first hook: LOCAL occurrences in `parse_var`. At the pass-2
  `name_exists` chokepoint (where every local read/write/return flows, since the var
  was created on pass 1), records `Local { fn_def: self.context, var_nr }` at
  `name_pos`. Keyed on binding IDENTITY — two same-named locals in different fns get
  distinct `(fn_def, var_nr)`. Public `set_record_resolutions` setter added for S3.
  **Gate met:** `loft introspect` byte-identical with the gate off (S1 vs S2 binary,
  empty diff); `tests/resolution_index.rs` proves distinct bindings on + off-by-default.

**Next: S3** — enable the gate on the LSP fresh parse (`loft::lsp`) and expose the
occurrences to the lsp module; the CLI/compiler parse stays gate-off/unchanged.

This is the deferred FOUNDATION under @PLN63 (loft-lsp): every LSP feature that needs to
know *what an identifier occurrence refers to* — not just its spelling — depends on it.
It is filed as its own plan because it touches loft's most delicate subsystem (the
parser) and needs design + rigor before any code.

## Goal

Record, **during parse**, the binding each identifier occurrence resolves to — a map
from a source position to a `Resolution`:

- **Global** — a top-level def (`def_nr`): fn / struct / enum / typedef / constant / interface.
- **Local** — a variable / parameter: its function, `var_nr`, resolved `Type`, and scope.
- **Field** — `expr.field`: the receiver's type + the attribute.
- **Method** — `expr.method(…)`: the receiver's type + the method def.
- **Unresolved** — a name that bound to nothing (a typo, or C80 value-undefinedness).

Expose it on the parser after a parse so tooling can answer *"what is the symbol at
`(line, col)`?"* and *"where are all the occurrences of THIS binding?"* precisely —
replacing @PLN63's lexical, name-based resolution.

## Why (the imprecision it removes)

`loft-lsp` today resolves identifiers **lexically** — by matching names. That is correct
for globally-unique names but wrong or blocked for everything scoped or type-dependent:

| Feature | Today (lexical) | With the index |
|---|---|---|
| find-references / rename of a LOCAL | approximated by enclosing block (F v1); no shadowing | exact binding, block-scoped |
| find-references of a METHOD (`len`) | over-matches every `len` token | only `text.len` (the resolved method) |
| **inlayHint (E)** | **BLOCKED** — variable positions unreliable in the parse path | binding position + `Type` available |
| completion after `expr.` | best-effort (first fn with a var of that name) | scope-precise receiver type |
| semanticTokens | globals + keywords only | locals / params / fields classified too |

All five need the *same* thing: per-occurrence resolution recorded at parse time. Building
it once lifts all of them — the reason it earns a plan rather than five separate hacks.

## The core design tension (Phase 0 decides)

The parser is loft's #1-weakness subsystem (heap / store-lifetime — see
[OWNERSHIP_MODEL](../../OWNERSHIP_MODEL.md), [STABILITY_HOTSPOTS](../../STABILITY_HOTSPOTS.md)).
Two ways to obtain the resolution:

1. **Gated instrumentation** at the parser's resolution chokepoints — where it ALREADY
   computes the binding: variable lookup (`Function::var`/`name_exists` in
   `parser/objects.rs`), field/method access (`parser/fields.rs`), call dispatch
   (`parser/mod.rs::call` / `parser/control.rs`). Append a `(span, Resolution)` record at
   each site. **Pro:** the resolution is right there — no re-derivation. **Con:** touches
   delicate code; must be strictly gated (zero cost when off) and proven behavior-identical.
2. **Separate resolver pass** over the parsed IR + the `Data`/variable tables. **Pro:**
   clean separation, the parser is untouched. **Con:** re-implements loft's scope + type
   resolution — a second source of truth that will drift (the exact anti-pattern the
   stability red-flags chase: two generators reading one IR).

**Leaning:** gated instrumentation (§1) — a single fact recorded where it is already
known beats a parallel resolver. But this is the load-bearing decision; Phase 0 proves it
with a byte-identical-IR gate (below) before committing.

## Sub-arcs / Phases

- **Phase 0 — design — DONE ([design.md](design.md)).** Decided: gated instrumentation
  (not a separate pass); the `Resolution` enum + `Occurrence`; storage on the `Parser`
  behind a `record_resolutions` gate (default off, zero-cost); the query API; and the
  **S1–S7 small-safe-step spine** with a byte-identical-IR gate on every step. The
  chokepoints are named to `file:line` — and the identifier positions are already in
  scope there (`parse_var`'s `name_pos`, threaded in @PLN63's fix-b).
- **Phase 1 — recording infra + one hook.** A `record_resolutions` gate on the parser,
  a `Vec<(Span, Resolution)>` store, and ONE hook — variable resolution. Prove on a probe
  that hovering / referencing a local resolves via the index. **Gate the gate:** with
  recording OFF, `loft introspect` IR is byte-identical to before (the recording must not
  perturb the parse — the [CODEGEN_METHOD](../../CODEGEN_METHOD.md) before/after diff).
- **Phase 2 — extend hooks.** Field access, method dispatch, call resolution, global refs.
  Each hook lands with a probe; the byte-identical-off gate holds throughout.
- **Phase 3 — wire the @PLN63 consumers.** Replace lexical resolution where it matters:
  E (inlayHint) reads binding position + `Type`; F-v2 makes references/rename per-occurrence
  exact for methods + shadowed locals; completion's `expr.` receiver resolves scope-precisely.
- **Phase 4 — validate.** Full suite both backends unaffected with recording off; the
  @PLN63 feature gates sharpened; graduate probes to regressions.

## Open questions

1. **Instrument vs. separate pass** (the §1/§2 tension) — Phase 0.
2. **Which parse pass records?** loft parses twice; pass 2 has resolved types. Record on
   pass 2 (types known), or both (positions on pass 1)? Interacts with two-pass dedup.
3. **Granularity** — all identifiers, or only the ambiguous (local / field / method)?
   Globals are already name-precise, so recording them may be redundant weight.
4. **Storage** — per-parse on the `Parser` (LSP re-parses anyway), or persisted in `Data`
   (reusable, but grows the cached bundle — cf. the @I86 stdlib cache round-trip).
5. **Span vs point** — record the full identifier span, or a start point + re-derive length?

## Risks

- **Perturbing the parse.** The #1 risk. Recording must be a pure side-append that changes
  nothing else; the byte-identical-IR-when-off gate (Phase 1) is non-negotiable.
- **Performance on the compile path.** The gated-OFF path must be provably zero-cost (a
  branch that never runs), or every `cargo run` / build pays for LSP-only machinery.
- **Two-pass subtlety.** Recording in the wrong pass captures pre-resolution types or
  duplicates.

## See also

- **Consumer:** [@PLN63 loft-lsp](../../lib_plans/63-lsp/README.md) — the plan whose E +
  F-v2 steps this unblocks (see its "Build order — remaining features").
- Method: [CODEGEN_METHOD](../../CODEGEN_METHOD.md) (the byte-identical before/after gate),
  [engineering-rigor](../../../../.claude/skills/engineering-rigor) (matrix-first).
- Substrate already in place: `Definition.position`, `Function::{var, name_exists, tp}`,
  `Parser.member_access`, `api_surface::classify` — the resolution index formalizes into a
  positional map what these expose piecemeal.
