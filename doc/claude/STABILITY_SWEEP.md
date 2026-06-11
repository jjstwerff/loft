<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# STABILITY_SWEEP.md — the brittleness hunt work list

A systematic sweep of the whole body of code for **invariants implemented in
more than one way** — the disease class behind #313 (layout decided in three
places), #314 (capture shape frozen before the decision was final), #323
(ownership transferred on one path, freed on another), and #328 (pointer-ness
erased between parse and layout).  The method is pass 1 of
[STABILITY_METHOD.md](STABILITY_METHOD.md) (sweep → move algorithms to their
data structures → de-duplicate), using the
[engineering-rigor](../../.claude/skills/engineering-rigor/SKILL.md)
instruments for *breakage discovery*:

1. **Find the fact with two homes.**  Grep/read the module for a fact that is
   derived, cached, or re-asserted in a second place (a flag AND a layout; a
   parse-time decision AND a codegen-time re-derivation; an interp op AND its
   native twin; a sentinel AND a zero-default).
2. **Write the probe that makes the homes disagree.**  Throwaway `/tmp`
   probes on `--interpret` first; distinctive collision-resistant values;
   vary ONE axis per probe (the composition axes: construction-path ×
   context × depth × null × cardinality × ordering × backend).
3. **Document — do not fix (sweep rule, user call 2026-06-11).**  For every
   finding, write down in the Findings log: the ONE invariant (a single
   sentence naming the fact and its single rightful home), every home where
   the code re-asserts it today, the probe that shows the homes disagreeing,
   and the observed damage.  Breaks become GitHub issues with the matrix and
   a `/tmp/p_followups/` repro; demonstrating tests land as
   `#[ignore = "stability-sweep: #NNN"]` so the suite stays green.  Fixes are
   a SEPARATE later pass — the sweep's output is the documented invariant
   catalog, not patches (fixing mid-sweep would re-shuffle the very ground
   being surveyed).  **The later pass's shape (user, 2026-06-11): move each
   algorithm toward the data structure it runs on — so the fact and the
   logic that asserts it share one home — and THEN remove the duplications.**
   The catalog should therefore name, per finding, which data structure is
   the invariant's natural home (where the algorithm should eventually live).
4. **A probe that holds is also a result.**  Record it in the row — "probed,
   held" tells the next session what was covered.

## Status legend

`☐ todo` · `▶ in progress` · `✅ swept (findings linked)` · `➖ low-risk
(thin/leaf module, sweep last)`

## Cross-cutting invariant families (highest yield — sweep these THROUGH the modules)

These are single facts asserted in N places across module boundaries.  Each
gets probed wherever its homes can drift apart.

| # | Family | The fact | Known homes | Status / findings |
|---|---|---|---|---|
| F1 | **Two-pass parse determinism** | pass 1 and pass 2 must derive identical facts (def numbering, lambda numbering, attr types, flags) | every `first_pass` branch (≈40 sites in `parser/*`); `data.reset()` keeps definitions but pass 2 re-parses bodies | ☐ todo — #313/#314 were instances; probe: facts set late in pass 1 vs consumed early in pass 2 |
| F2 | **Ownership of a store** | who frees a store, exactly once | `scopes.rs get_free_vars` (dep-empty = owned, captured-ref exemption, work-refs, in_ret); `scan_set owned_refs` (#316); `state/codegen.rs generate_set` pre-Set free + S1 guard; `check_ref_leaks` (debug assert); `free_named` cascade (`__closure_*` only); `paired_witness`/`witness_buffer` OpFreeRefIfDistinct | ▶ first finding: **#330** (see log); remaining pairs todo |
| F3 | **Null representation of a DbRef** | what bytes mean "null pointer" | `OpNullRefSentinel` (store_nr = u16::MAX); zero-default record bytes (store 0, rec 0); `free_named`'s skip pattern (store 0 ∧ rec 0); `OpEqRef` comparisons | ✅ probed, HELD (see log) — comparator normalises; dual encoding persists as latent risk for byte-level consumers |
| F4 | **Interp op ≡ native op** | every opcode's semantics on both backends | `fill.rs` (interp) vs `generation/ops/*` + `codegen_runtime.rs` (native); `cross_mode` harness covers a sample, not the op set | ☐ todo — #323-native was an instance; probe: ops with no cross_mode cell (list them first — a usage-sentinel sweep) |
| F5 | **Stack slot layout** | a value's slot size/alignment as parser, codegen, and runtime each compute it | `variables/mod.rs` size table (fn-ref = 20B …); `state/codegen.rs stack_step`/`size_*`; `state/mod.rs` runtime reads; LOFT_ALIGN duality | ☐ todo — PLAN53 cluster 2 was an instance; probe: odd-sized types (12B DbRef) adjacent to 8B slots |
| F6 | **DB field layout** | a struct field's position/width as the parser reads it vs fill_database wrote it | `parser/mod.rs get_val/set_field_check` width selection (alias forced_size, byte_width, vector_narrow_width); `typedef.rs fill_database` arms; `database/types.rs finish_type` packing; native `generation/mod.rs emit_field` re-derivation | ☐ todo — #313/#328 + the emit_field schema split were instances; probe: every Type arm where get_val's width logic and fill_database's arm could pick differently (narrow ints, enums, tuples, child recs) |
| F7 | **Value vs pointer copy semantics** | what `a = b` / field-assign / capture copies | LOFT.md:570 says struct var assign copies the DbRef; codegen `gen_set_first_ref_var_copy` deep-copies; `OpCopyRecord` field copies; `reference<T>` repoints (#328); captures share by DbRef (P260) | ✅ probed, BROKEN at the spec home: **#331** (see log) |
| F8 | **Deps as liveness vs deps as ownership vs deps as markers** | what `Type::*(…, deps)` means | borrow liveness (scopes); ownership negation (dep-empty = owned); the `u16::MAX` share marker (#328/closure records); `@P302` self-dep = ownership marker for keyed locals; attribute-index deps in returned types (`dep_has_var` resolves attr-index vs var-nr with a FALLBACK comparing raw numbers) | ☐ todo — one Vec<u16> carries FOUR meanings; probe: collisions (a var-nr that equals an attr index; a marker surviving into liveness analysis) |
| F9 | **The startup/stdlib caches vs live state** | cached parse must equal fresh parse | `startup_cache.rs` manifest (sources, sig); `cache.rs` keys; `ir_store`/`ir_read` round-trip; REPL `rollback_to` | ▶ first cells probed, HELD: #328 marker-dep types and #313 split fn-fields round-trip warm≡cold (`/tmp/claude/sweep/f9*.loft`, 2026-06-11); REPL rollback + lib-path axes todo |
| F10 | **Text ownership** | who frees a String buffer | `free_text` / `OpFreeText`; work-text result buffers; `skip_free` texts; text-returning-fn cell exemption (02d-vii) | ☐ todo — plan-53 cluster 5 was an instance |

## Module work list (every file; sweep top-down by risk)

| Module | Lines | Suspected dual-invariants to hunt | Status |
|---|---|---|---|
| `src/parser/mod.rs` | 6863 | F1, F6, F8; `fn_ref_field_is_split` vs `assigned_lambda_d_nr` (one home now — verify no third consumer); `base_var_of` root walk vs other lvalue walks | ☐ todo |
| `src/scopes.rs` | 4969 | F2, F8; `var_mapping` scope copies vs codegen slots; `value_reads_var` coverage vs Value variants (a missed variant = wrong free) | ☐ todo |
| `src/state/codegen.rs` | 3745 | F2, F5; S1-substitution detection (`any arg == Var(v)`) vs scan_set's reads-v test — TWO different "RHS uses v" predicates | ☐ todo |
| `src/state/mod.rs` | 4616 | F5; `fn_call_ref` 20B slot parsing vs parser's fn-ref block shapes | ☐ todo |
| `src/parser/control.rs` | 4823 | F1; match/if result-type unification vs codegen's branch layout | ☐ todo |
| `src/parser/vectors.rs` | 4424 | F1 (lambda epilogues pass-asymmetry — emit_lambda_code `!first_pass`); closure record synthesis pass-1-only attr freeze | ☐ todo |
| `src/parser/collections.rs` | 4186 | `towards_set` lvalue-shape dispatch (OpGetDbRef/OpGetRecord/keyed) vs the type-based dispatch below it — TWO discriminators for one assignment fact (#328 used shape because deps get rewritten) | ☐ todo |
| `src/data.rs` | 4304 | F8; `def_nr("n_*")` naming convention re-asserted at every call site; `type_elm`/`type_def_nr` vs typedef resolution | ☐ todo |
| `src/database/allocation.rs` | 2061 | F2, F3; free-bitmap vs `store.free` flag (the @P317 tripwire exists BECAUSE they can disagree); `max` watermark trim vs bitmap | ☐ todo |
| `src/database/types.rs` | 2929 | F6; `position()` linear scan vs `finish_type` packing; `content()`/`size()` consistency for late-mutated types (@P191) | ☐ todo |
| `src/store.rs` | 2788 | F3; LLRB free-tree vs `needs_coalesce` flag; `claims` set vs actual record headers; `generation` counter consumers | ☐ todo |
| `src/typedef.rs` | ~800 | F1, F6; `has_value_cycle` skip-rules vs fill_database arms (must skip the SAME fields) | ☐ todo |
| `src/fill.rs` | 2168 | F4; op arg-decoding vs codegen's arg-encoding (u16/i32 widths per op) | ☐ todo |
| `src/generation/mod.rs` | 3142 | F4, F6; emit_field schema re-derivation vs registered stores (partially single-homed by #313 — sweep the OTHER Type arms) | ☐ todo |
| `src/generation/emit.rs` | 1768 | F4; per-op Rust templates vs fill.rs semantics | ☐ todo |
| `src/generation/ops/*` | ~3000 | F4; ref_ops/refcount remnants post plan-57 (does dead rc code still emit?) | ☐ todo |
| `src/codegen_runtime.rs` | 4118 | F4, F5; runtime helpers duplicating `state/*` logic for native | ☐ todo |
| `src/main.rs` | 5803 | F9; CLI mode dispatch (interpret/native/html/introspect) re-deciding pipeline stages | ☐ todo |
| `src/cache.rs` + `src/startup_cache.rs` | ~900 | F9; manifest coverage vs actual inputs (#322 fixed sources — what about `--lib` PATHS, env, registry index?) | ☐ todo |
| `src/ir_schema*.rs` + `ir_store.rs` + `ir_read.rs` + `data_store.rs` | ~7000 | F9; serialized Definition fields vs `Definition` struct (a field added to one and not the other = silent cache corruption — `assigned_lambda_d_nr` IS serialized; are marker deps?) | ☐ todo |
| `src/parser/expressions.rs` | 2697 | F7; `change_var` type merges (self-dep strip is #328-narrow — other degenerate merges?) | ☐ todo |
| `src/parser/operators.rs` | 2173 | `call_to_set_op` GET→SET table vs get_val emission table (a get emitted with no set twin = silent no-op assignment) | ☐ todo |
| `src/parser/objects.rs` | 2058 | construction field-init vs post-hoc assignment (#328 showed they diverge; sweep other field types) | ☐ todo |
| `src/parser/definitions.rs` | 2236 | F1; sub_type wrappers; attribute re-creation across passes | ☐ todo |
| `src/parser/fields.rs` + `builtins.rs` | ~1500 | field-access dispatch vs get_val arms | ☐ todo |
| `src/variables/mod.rs` | 1704 | F5; size table vs Type (every new Type variant must be added — usage sentinel?) | ☐ todo |
| `src/parallel.rs` + `src/state/io.rs` | ~3500 | F2 across threads (worker slot dispenser vs free-bitmap; store swap-back) | ☐ todo |
| `src/extensions.rs` + `src/native_lib.rs` | ~3000 | shared-bridge marshallability (#303 unified it — verify single home held) | ☐ todo |
| `src/lexer.rs` | 1759 | position/link invariants under `revert` | ➖ low-risk |
| `src/repl.rs` | 3141 | F9 (`rollback_to` vs caches/lambda counters/fn_lambdas) | ☐ todo |
| `src/state/text.rs` + `debug.rs` | ~2500 | F10; the trace dumper deref-safety (it SIGSEGV'd on #328 records — known instance, file?) | ☐ todo |
| `src/database/format.rs` + `journal.rs` | ~3000 | F9 persistence round-trips | ☐ todo |
| `src/keys.rs`, `src/log_config.rs`, misc leaf files | ~2000 | — | ➖ low-risk |
| `default/01_code.loft` (+02,03…) | ~3000 | `#rust` template bodies vs interp registration (one fn, two bodies) | ☐ todo |
| `tools/indexer`, `lib/*` consumers | — | dogfood surface; run, don't sweep | ➖ low-risk |

## Findings log

### F2-1 — #330: self-reading struct-literal reassignment reads the recycled store (2026-06-11)

- **Invariant**: a reassignment's old store must outlive every read the RHS
  performs; equivalently, exactly one predicate decides "does the RHS read
  `v`" for every free site.
- **Homes today**: `state/codegen.rs generate_set` pre-Set `OpFreeRef` with
  the S1 guard (top-level Call args only; struct-literal RHS explicitly
  bails toward freeing) vs `scopes.rs value_reads_var` (recursive, used only
  by the #316 transition free).
- **Natural home**: ONE RHS-reads-v predicate owned by the IR (`Value`),
  consumed by both free sites; longer term the free belongs to the store
  allocator's reassignment path, not codegen.
- **Probe / damage**: `x = S { v: x.v + 1 }` → `x.v == null` through a
  `not null` field, BOTH backends (`/tmp/p_followups/f2b.loft`, `f2c.loft`).
  Nested ref-param call shape held; #328's borrow shape held (deps gate).
- **Artifacts**: issue #330; `tests/issues.rs::issue_330_self_reading_literal_reassignment`
  (`#[ignore]`, fails-as-demonstration when forced).

### F7-1 — #331: LOFT.md:570 alias claim vs codegen deep copy (2026-06-11)

- **Invariant**: `a = b` for struct vars has ONE defined meaning, asserted
  identically by the spec and the implementation.
- **Homes today**: LOFT.md:570 ("the DbRef is copied — both point to the
  same record") vs `gen_set_first_ref_var_copy` (deep copy, relied on by
  #316's ownership tracking and C38's capture rationale).
- **Natural home**: the spec section + one codegen arm; which semantics wins
  is a pass-2 design decision (alias would need the lifetime model to carry
  it; deep-copy needs a sanctioned borrow idiom for walks).
- **Probe / damage**: `b.v = 42` after `a = b` → `a.v == 1` (probe
  `/tmp/p_followups/f7.loft`); doc misleads; the #316 walk-leak residual is
  a downstream cost of the copy.
- **Artifacts**: issue #331.

### F3-1 — two null encodings, comparator normalises: probed, HELD (2026-06-11)

- **Invariant**: one byte-pattern means "null DbRef" — or every consumer
  must accept both.
- **Homes today**: `OpNullRefSentinel` (store u16::MAX) for explicit null /
  clears; zero-default bytes (0,0) from `set_default_value` for omitted
  fields; `OpEqRef`; `free_named`'s skip patterns (MAX early-return AND the
  (0,0) cascade skip).
- **Probe**: defaults vs explicit null — `== null` true for both, the two
  nulls compare EQUAL to each other, both deref gracefully
  (`/tmp/claude/sweep/f3a.loft`, `f3b.loft`).  Verdict: held — the
  comparison home normalises.  Residual risk: byte-level consumers (codecs,
  `get_u32_raw` walkers, future ops) must keep accepting both; the dual
  encoding itself is a pass-3 dedup candidate (pick one encoding at write
  time).
