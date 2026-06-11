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
| F1 | **Two-pass parse determinism** | pass 1 and pass 2 must derive identical facts (def numbering, lambda numbering, attr types, flags) | every `first_pass` branch (≈40 sites in `parser/*`); `data.reset()` keeps definitions but pass 2 re-parses bodies | ▶ lib-closure-factory cell held (`f1.loft`); #313/#314 historical instances fixed; DEFERRED: diagnostics-altering-flow and todo_files-reorder cells |
| F2 | **Ownership of a store** | who frees a store, exactly once | `scopes.rs get_free_vars` (dep-empty = owned, captured-ref exemption, work-refs, in_ret); `scan_set owned_refs` (#316); `state/codegen.rs generate_set` pre-Set free + S1 guard; `check_ref_leaks` (debug assert); `free_named` cascade (`__closure_*` only); `paired_witness`/`witness_buffer` OpFreeRefIfDistinct | ▶ **#330** (see log); `value_reads_var` default arm misses read-bearing variants (`FnRef(_, w, _)` reads w — audit the ~30 default-arm variants when pass 2 centralises the predicate); par cell held (`fpar.loft`); witness-pair cells DEFERRED |
| F3 | **Null representation of a DbRef** | what bytes mean "null pointer" | `OpNullRefSentinel` (store_nr = u16::MAX); zero-default record bytes (store 0, rec 0); `free_named`'s skip pattern (store 0 ∧ rec 0); `OpEqRef` comparisons | ✅ probed, HELD (see log) — comparator normalises; dual encoding persists as latent risk for byte-level consumers |
| F4 | **Interp op ≡ native op** | every opcode's semantics on both backends | `fill.rs` (interp) vs `generation/ops/*` + `codegen_runtime.rs` (native); `cross_mode` harness covers a sample, not the op set | ▶ first finding: **#333** (float `/0.0`: interp aborts, native nulls); int `/0` `%0`, neg div/mod, float assoc AGREE (`f4.loft`); DEFERRED: the op-coverage sentinel (enumerate ops lacking cross_mode cells) |
| F5 | **Stack slot layout** | a value's slot size/alignment as parser, codegen, and runtime each compute it | `variables/mod.rs` size table (fn-ref = 20B …); `state/codegen.rs stack_step`/`size_*`; `state/mod.rs` runtime reads; LOFT_ALIGN duality | ▶ LOFT_ALIGN=1 sample held on DbRef-heavy program; tuple struct-field round-trip held (`ftup.loft`); DEFERRED: full odd-size adjacency matrix |
| F6 | **DB field layout** | a struct field's position/width as the parser reads it vs fill_database wrote it | `parser/mod.rs get_val/set_field_check` width selection (alias forced_size, byte_width, vector_narrow_width); `typedef.rs fill_database` arms; `database/types.rs finish_type` packing; native `generation/mod.rs emit_field` re-derivation | ▶ finding: **#332** (nullable narrow null doesn't round-trip); HELD: u8/i16/i32/limit field values, narrow vectors u8/i16/u16, enum+bool packing, tuple fields, hash insert/update — both backends (`f6a–c`, `ftup`, `fhash`) |
| F7 | **Value vs pointer copy semantics** | what `a = b` / field-assign / capture copies | LOFT.md:570 says struct var assign copies the DbRef; codegen `gen_set_first_ref_var_copy` deep-copies; `OpCopyRecord` field copies; `reference<T>` repoints (#328); captures share by DbRef (P260) | ✅ probed, BROKEN at the spec home: **#331** (see log) |
| F8 | **Deps as liveness vs deps as ownership vs deps as markers** | what `Type::*(…, deps)` means | borrow liveness (scopes); ownership negation (dep-empty = owned); the `u16::MAX` share marker (#328/closure records); `@P302` self-dep = ownership marker for keyed locals; attribute-index deps in returned types (`dep_has_var` resolves attr-index vs var-nr with a FALLBACK comparing raw numbers) | ▶ view-dep basic cell held (`f8a.loft`); #328's self-dep strip is a documented instance of meaning-collision; DEFERRED: crafted attr-index/var-nr collision (needs IR-level var numbering control) — pass-2 should split the four meanings into distinct fields |
| F9 | **The startup/stdlib caches vs live state** | cached parse must equal fresh parse | `startup_cache.rs` manifest (sources, sig); `cache.rs` keys; `ir_store`/`ir_read` round-trip; REPL `rollback_to` | ▶ first cells probed, HELD: #328 marker-dep types and #313 split fn-fields round-trip warm≡cold (`/tmp/claude/sweep/f9*.loft`, 2026-06-11); REPL rollback + lib-path axes todo |
| F10 | **Text ownership** | who frees a String buffer | `free_text` / `OpFreeText`; work-text result buffers; `skip_free` texts; text-returning-fn cell exemption (02d-vii) | ▶ struct-field text churn held leak-free (`f10.loft`); plan-53 historical; DEFERRED: par text buffers, text in returned structs |

## Module work list (every file; sweep top-down by risk)

| Module | Lines | Suspected dual-invariants to hunt | Status |
|---|---|---|---|
| `src/parser/mod.rs` | 6863 | F1, F6, F8; `fn_ref_field_is_split` single home verified (#313); `base_var_of` vs other lvalue walks | ▶ family-covered (#332 cells); base_var_of unification → pass 2 |
| `src/scopes.rs` | 4969 | F2, F8; `value_reads_var` default-arm gap NOTED (FnRef reads w) | ▶ #330; predicate centralisation → pass 2 |
| `src/state/codegen.rs` | 3745 | F2, F5; the S1 top-level-arg predicate IS #330's broken home | ✅ swept → #330 |
| `src/state/mod.rs` | 4616 | F5; `fn_call_ref` 20B slot | ✅ covered by closure/mut_closure matrices (both backends, 44 cells) | |
| `src/parser/control.rs` | 4823 | F1; match/if result-type unification vs codegen's branch layout | ▶ enum/tuple cells held (`f6c`, `ftup`); match-unification DEFERRED |
| `src/parser/vectors.rs` | 4424 | F1 lambda-epilogue asymmetry (#313/#314 fixed); factory cell held (`f1.loft`) | ✅ swept via #313/#314/#323 arcs |
| `src/parser/collections.rs` | 4186 | `towards_set` dual discriminators (shape vs type) — documented in #328 fix comments | ▶ keyed insert/update held (`fhash`); discriminator unification → pass 2 |
| `src/data.rs` | 4304 | `def_nr` convention sentinel grep: only system/type names lack prefixes — convention holds | ✅ swept (static sentinel) |
| `src/database/allocation.rs` | 2061 | F2, F3; free-bitmap vs `store.free` (@P317 tripwire) | ▶ F3 held; bitmap/flag disagreement cells DEFERRED (tripwire already guards) |
| `src/database/types.rs` | 2929 | F6 | ▶ width/packing cells held (#332 is the narrow-null exception); P191 late-mutation DEFERRED (validator exists) |
| `src/store.rs` | 2788 | F3 held; LLRB vs `needs_coalesce`, `claims` vs headers | ▶ DEFERRED (needs store-level fuzz harness — pass-2 candidate instrument) |
| `src/typedef.rs` | ~800 | `has_value_cycle` skip ≡ fill arms — aligned in #328 | ✅ swept |
| `src/fill.rs` | 2168 | F4 | ▶ #333 (float div); arg-width audit → the F4 op sentinel (DEFERRED) |
| `src/generation/mod.rs` | 3142 | F4, F6; emit_field re-derivation (fn-fields single-homed in #313) | ▶ narrow/enum/tuple/hash native cells held; remaining Type arms DEFERRED |
| `src/generation/emit.rs` | 1768 | F4 | ▶ see F4 row (#333; sentinel deferred) |
| `src/generation/ops/*` | ~3000 | F4; post-plan-57 rc remnants | ▶ DEFERRED to pass 3 (deletion candidates by usage sentinel) |
| `src/codegen_runtime.rs` | 4118 | F4, F5 | ▶ covered by cross-backend probes this sweep; helper-duplication inventory → pass 2 |
| `src/main.rs` | 5803 | F9 CLI dispatch | ➖ exercised by every suite mode; no dual-invariant suspect found |
| `src/cache.rs` + `src/startup_cache.rs` | ~900 | F9; manifest vs inputs (#322 fixed) | ▶ warm≡cold held for new shapes; lib-PATH/env axes DEFERRED |
| `src/ir_schema*.rs` + `ir_store.rs` + `ir_read.rs` + `data_store.rs` | ~7000 | F9 codec vs struct | ▶ marker-dep + split-field shapes round-trip held (f9/f9b); field-by-field codec audit DEFERRED (pass-2: derive codec from one schema) |
| `src/parser/expressions.rs` | 2697 | F7; `change_var` merges | ✅ swept: `x = x` corrupts both backends, divergently — logged on #330 (the predicate hole's degenerate cell) |
| `src/parser/operators.rs` | 2173 | `call_to_set_op` GET→SET table vs get_val table | ▶ assignment shapes this sweep all held; table-completeness sentinel DEFERRED |
| `src/parser/objects.rs` | 2058 | construction vs post-hoc assignment parity | ✅ swept: text/narrow/enum/tuple/hash/reference cells held (#328 fixed the reference cell) |
| `src/parser/definitions.rs` | 2236 | F1; sub_type single funnel verified (#318 R3, #328 marker) | ✅ swept |
| `src/parser/fields.rs` + `builtins.rs` | ~1500 | field-access dispatch | ▶ chained/nested reads held across #328 probes; iterator-op dispatch DEFERRED |
| `src/variables/mod.rs` | 1704 | F5 size table vs Type variants | ▶ exercised broadly; add-a-variant drift is a pass-2 chokepoint candidate (exhaustive match) |
| `src/parallel.rs` + `src/state/io.rs` | ~3500 | F2 across threads | ▶ par result cell held (`fpar`, both backends); dispenser/swap-back stress DEFERRED |
| `src/extensions.rs` + `src/native_lib.rs` | ~3000 | #303 marshallability single home | ✅ verified by cdylib suites each run |
| `src/lexer.rs` | 1759 | position/link invariants under `revert` | ➖ low-risk |
| `src/repl.rs` | 3141 | F9 `rollback_to` | ▶ error-recovery cell held; counter/caches-after-rollback cells DEFERRED |
| `src/state/text.rs` + `debug.rs` | ~2500 | F10; dumper deref-safety | ✅ re-probed post-#328: dumper clean on reference-field + walk programs (the crash was the fixed frame corruption) |
| `src/database/format.rs` + `journal.rs` | ~3000 | F9 persistence | ▶ covered by store_durable/persist suites; new-shape persistence cells DEFERRED |
| `src/keys.rs`, `src/log_config.rs`, misc leaf files | ~2000 | — | ➖ low-risk |
| `default/01_code.loft` (+02,03…) | ~3000 | `#rust` twin bodies | ▶ #333 IS this dual for float-div; systematic twin-audit = the F4 sentinel (DEFERRED) |
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
  `not null` field, BOTH backends (`/tmp/p_followups/f2b.loft`, `f2c.loft`);
  degenerate `x = x` corrupts divergently (interp reads the recycled slot,
  native nulls — `fself.loft`).  Nested ref-param call shape held; #328's
  borrow shape held (deps gate).
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

### F6-1 — #332: nullable narrow field null never round-trips (2026-06-11)

- **Invariant**: a nullable field's null state survives write→read→compare,
  whatever the storage width.
- **Homes today**: narrow-width selection in `get_val`/`set_field_check`
  (Parts::Short legacy `+1` vs ShortRaw vs Int sentinels) vs the null write
  encoding vs the `== null` decode — the omitted-field zero default matches
  none of them.
- **Natural home**: the `Parts` variant — each storage width owns its null
  sentinel and BOTH the write and the compare ask it.
- **Probe / damage**: `a: i16` omitted → `== null` false; `a = null` →
  still false; values round-trip fine.  Both backends
  (`/tmp/p_followups/f6d.loft`).
- **Artifacts**: issue #332; ignored test
  `issue_332_nullable_narrow_field_null_roundtrip`.

### F4-1 — #333: float ÷0 — interp aborts, native nulls (2026-06-11)

- **Invariant**: one semantics per operator edge case, asserted by the lint
  text, the interp body, and the native body alike (null per C66/C67).
- **Homes today**: `fill.rs` float-div (traps) vs the generated native body
  (nulls) vs the lint's own promise ("may produce null").
- **Natural home**: the op's single definition — `default/01_code.loft`'s
  `#rust` template and the interp body must be one artifact (the F4 family's
  general cure).
- **Probe / damage**: `1.0 / 0.0` halts interp with exit 1; native prints
  `null` and continues (`/tmp/p_followups/f4.loft`).  Integer edges agree.
- **Artifacts**: issue #333; ignored test
  `issue_333_float_div_by_zero_yields_null`.

### Held-cells log (2026-06-11 sweep day 1)

`f6a/b/c` narrow+enum+limit fields and vectors (both backends) · `f6d`
value cells · `ftup` tuple field repoint+destructure (both) · `fhash` keyed
insert/update (both) · `fpar` par loop (both) · `f8a` view dep · `f1` lib
closure factory · `f10` text-field churn leak-free · `f9/f9b` program-cache
warm≡cold for #328/#313 shapes · LOFT_ALIGN=1 sample · REPL error rollback ·
trace dumper on reference-field programs.

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
