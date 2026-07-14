<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN104 — Option (b) scope: third pass re-finalizes the signature

Scope for closing the interpreter `fill::append_text` leak (loft-lang/loft#568) via
**option (b)**: make the post-oracle third pass produce a def whose promoted `&text`
retbuf survives to codegen as a real signature parameter — byte-identical to the
proven-clean `r = f(x); r` workaround. Grounded in the RECHECK findings
(`bytecode-comparisons/README.md` § RECHECK); read that first.

## IMPLEMENTATION FINDINGS (2026-07-13) — the delivery works; the blocker is `a == v`

Attempted implementation this session. Two hard lessons, both from runtime ground-truth
(the introspect **lies** here — see the calibration warning below):

1. **The delivery half is solved.** The `block_result` frame-dep fix (`control.rs`,
   gated on `force_tret`) makes the third-pass def emit the clean retbuf delivery —
   `AppendStackText(retbuf)` into the caller's buffer, `GetStackText` deref before
   `Return` — byte-identical to the workaround's `run_t`. Confirmed at the bytecode level.

2. **The leak survives because `a != v` (attribute index ≠ variable index).** The
   returned-type dep is consumed in BOTH spaces: attribute-space (the def/signature
   render → `text["#3"]` when out of range) AND variable-space (the block dep + delivery
   op). With the post-hoc `do_tret_bind` retbuf, `a = 2` but `v = 3` (minted after the
   tail's `__work_1@2`), so **no single dep value satisfies both** consumers. Pushing the
   attribute index (`a`) mis-resolves to `__work_1`; pushing the var index (`*v`) makes an
   out-of-range attribute (`#3`). BOTH leak.

   **Runtime proof** (`LOFT_PTR_TRACE`, reverted): min.loft `run_t` executes
   `OpAppendText` into a fresh OWNED String (never freed → leak); min_wa `run_t` executes
   `OpAppendStackText` into the caller's buffer (freed by the caller). The owned append is
   the orphan.

   **CALIBRATION TRAP (cost me hours):** with the ambiguous `#3` dep, `loft introspect`
   dumped `AppendStackText` while `--interpret` EXECUTED `AppendText` — the dump and the
   run disagreed. **Trust the runtime trace / ASan, not the introspect, once the returned
   dep is ambiguous.** (engineering-rigor § calibration: a stale/ambiguous artifact.)

**So the invariant is not "make the dep variable-space" — it is `a == v`.** The retbuf's
variable index must EQUAL its attribute index, so the dep is unambiguous in both spaces.
Pass-1 promotion of a user local (`r = f(x); r`) gets this free (`r` is the first local,
created before `__work_1`); `do_tret_bind`'s post-hoc mint does not.

## Reaching `a == v` — (b1) is EMPIRICALLY BLOCKED; (b2) swap is the path

### (b1) Declaration-time injection — BLOCKED (probed 2026-07-13)

Idea: for `force_tret` defs on the third pass, declare the `&text` retbuf param in
`parse_function` BEFORE the body parses, so it takes the variable index right after the
declared params (= its attribute index). **A probe at the injection point disproves it:**
for `run_t`, `n_attrs(before) = 2` but **`n_vars(before) = 3`**. The third pass inherits
pass-2's persisted variables — including the body local `__work_1` at variable 2 — via
`self.vars.append(def.variables)` (definitions.rs ~1130), BEFORE the injection point. So
the injected retbuf lands at variable **3**, attribute **2** → `a != v`, the exact failure
`do_tret_bind` already has. The only way to get the retbuf a low variable index is to
create it on **pass 1** (before the body local exists) — but the fn-ref/index tail isn't
classifiable until pass 2, and injecting a retbuf for EVERY text return on pass 1 is the
`@P387` change that was REVERTED for breaking non-buffer text call sites (par workers #273,
markdown viewer). So (b1) cannot achieve `a == v` on the third pass, full stop.

### (b2) Swap after promotion — ATTEMPTED; blocked by TYPE-dep renumber (2026-07-13)

Idea: the variable ORDER is fixed by pass-1/2 inheritance, so the retbuf is always minted
high. Accept that and **renumber** — after `text_return` promotes the retbuf (variable `tv`,
attribute ordinal `a = arguments().position(tv)`, `a < tv`), swap variables `a` and `tv` so
the retbuf sits at variable `a` (== its attribute index). Implemented in `block_result`'s
`Type::Text` arm (holds the body op list `l`), gated on `force_tret`: three-way
`remap_var_deep` of `l` (`a → TMP → tv → a`) + a `Function::swap_variables` helper +
`tp = Deps::frame1(a)`.

**Result: still leaks (now `append_stack_text`).** The bytecode header dep flipped to the
correct `text["___tret_1"]` and the delivery became `AppendStackText`, but the IR desynced:
the retbuf got a stray `= ""` init and the `cref_work_buf` dep mislabeled to `["___tret_1"]`.
**Root cause: `remap_var_deep` renumbers var references in the `Value` IR tree, but NOT the
deps embedded in `Type` annotations** (`Block.result`, the `cref_work_buf` buffer type, the
returned type). After the variable-table swap, those type deps still point at the old
indices and now resolve to the wrong names — the cref buffer orphans. There is no
type-dep-aware deep renumber utility; `remap_var_deep` covers only `Value`.

**So (b2) needs a prerequisite: a renumber that walks BOTH the `Value` IR AND every `Deps`
inside every embedded `Type`** (block result types, buffer types, the def's returned type),
applied atomically with the variable-table swap. That is real new infrastructure — build
and unit-test it FIRST (round-trip a swap on a captured IR and assert byte-identity to a
hand-renumbered reference), then the `block_result` swap becomes a safe one-liner over it.

### (b2) COMPLETED with the type-dep-aware renumber — works, but oracle non-determinism blocks (2026-07-13)

Built the missing renumber and wired the swap through it:

- `Deps::renumber_frame` (data.rs) — frame-var entries only, skips `u16::MAX`, debug-asserts
  not attr-space.
- `Type::renumber_frame_deps` (data.rs) — recurses every dep-carrying / nested `Type` variant.
  **5 unit tests** (nested Vector/Function, `u16::MAX` marker, 3-way swap) — all green.
- `Parser::renumber_frame_var` (collections.rs) — the `Value`-tree walker `remap_var_deep`
  lacked: also renumbers `Block.result` + `FnRef` type deps (the desync that broke the first swap).
- `Function::swap_variables` + `renumber_frame_in_types` (variables/mod.rs) — swaps ALL
  var-indexed tables (`names`, `work_texts`, `work_refs`, `arm_consumed`, `inline_ref_vars`,
  `annotated`, `closure_var_map`, `rebind_orig`); leaves scope-indexed tables alone.
- The swap in `block_result`'s `Type::Text` arm (control.rs), gated on `force_tret`.

**Result: WHEN the promotion fires, the fix is byte-exact and leak-free.** `min.loft`
+`LOFT_TRET_FIX` `run_t` becomes byte-identical to min_wa (bytecode AND IR); the runtime text
trace (`LOFT_PTR_TRACE`) shows every buffer allocated-and-freed, identical to clean min_wa
(no orphan). The renumber is correct.

**BLOCKER: `force_tret` is non-deterministic.** Across runs the oracle
(`report_tret_promotions` → `use_analysis::return_ownership`, committed P2/@PLN94 — NOT the
renumber) flags run_t inconsistently: `force_tret={629}` (promote → clean) some runs,
`force_tret={}` (skip → leak) others. So end-to-end the fix is flaky. This is a PRE-EXISTING
non-determinism the renumber merely surfaced — likely a `HashMap`-iteration-order dependence
in the ownership analysis or `do_tret_bind`'s pass-1 firing (`Defs.rhs` is a `HashMap`;
`report_tret_promotions` loops defs deterministically, so the variance is inside
`return_ownership` or the parse that feeds it). **Next: make the promotion decision
deterministic** (find the order-dependent iteration; a `BTreeMap`/sorted pass), then the
whole fix is stable.

**Measurement caveat (cost hours):** the ASan `ir_read` suppression is stack-substring-based
and UNRELIABLE for the third-pass case — `malloc_context_size=64` yields FALSE POSITIVES
(truncated stacks hide the `ir_read` frame → miscounted as `append_text`), `=200` yields
FALSE NEGATIVES (a real leak's deep stack passes through `ir_read` → wrongly suppressed). The
reliable signal is the runtime **text trace** (`LOFT_PTR_TRACE`: every `append`/`free` ptr must
balance), not `realleak.py`.

### RESOLVED (2026-07-13): the "non-determinism" was the STARTUP CACHE, not the oracle

The apparent `force_tret` flakiness was the **@PLN11 whole-program startup cache**
(`cache::program_cache_enabled`, default-ON except under `LOFT_NO_CACHE` / `cargo`). It mmaps
the entire parsed program keyed by **content**, NOT by the env-gated `LOFT_TRET_FIX`. So a warm
run restores cached IR and skips parsing entirely — `report_tret_promotions` never runs, the
flag is ignored, and you get whatever was last cached (promoted or not). Repeated measurements
were reading stale cache state. **The oracle is deterministic; there is no oracle bug.**

**Test the fix with `LOFT_NO_CACHE=1`** (or make it default-on so the cache is built WITH it).
Under `LOFT_NO_CACHE`, verified fully deterministic: +FIX 10/10 `AppendStackText` (clean),
baseline 5/5 `AppendText` (leak); `run_t` byte-identical to min_wa.

### Fix coverage (corpus, `LOFT_NO_CACHE` + `LOFT_TRET_FIX`)

| fn | tail | result |
|---|---|---|
| `ret_fnref` | fn-ref call `f(x)` (the min.loft/#568 primary) | **STACK — FIXED** |
| `ret_local` / `ret_interp` | already clean | STACK — unchanged ✓ |
| `ret_borrow` | borrows an arg | not promoted ✓ (correct) |
| `ret_index` | local-index view `v[0]` | promoted (retbuf present) but STILL `DIRECT` |

`ret_index` is a **distinct, harder sub-case**: its return is a VIEW of a local vector element,
so the returned type carries a SECOND dep `text["___tret_1", "v"]` (retbuf + the vector local),
and the delivery stays `AppendText` — the view must be MATERIALIZED (copied) into the retbuf,
which the owned-value swap does not do. The owned-value leaker class (fn-ref call — the main
#568 shape) is fixed; the view-of-local class and method-call tails remain. All outputs are
correct on both backends.

### To ship

1. Make the promotion **default-on** (drop the `LOFT_TRET_FIX` env gate) so the startup cache is
   built with it (no cache confound) and real programs benefit.
2. Handle the `ret_index` view-of-local class (materialize the view into the retbuf before the
   local frees — the @P329/#306 `materialize_view_return` pattern), and method-call tails.
3. Verify the six real nightly leakers + full suite, both backends, under a rebuilt cache.

### Default-on trial (2026-07-13) — REVERTED to opt-in; 7 suite regressions

Flipped the promotion default-on (`report_tret_promotions` populates `force_tret`
unconditionally). Full suite under `cargo` (cache off): **2883 passed, 7 failed** — all 7
pass again with the opt-out (isolated to this change). The regressions span four modes, so
default-on is NOT ready:

1. **Spurious third-pass diagnostics — FIXED (2026-07-13).** The third pass re-parses the
   whole file and re-emitted/duplicated warnings pass 2 already handled (min-repro
   `textheavy.loft`: 1 → 3 warnings; corpus dead-assignment warnings). **Fix implemented:**
   `Diagnostics::truncate_to` + `Lexer::truncate_diagnostics`; `parse` snapshots
   `diagnostics().entries().len()` before the third pass and truncates back after (the third
   pass is a re-lowering, not a fresh analysis). Verified: textheavy 3 → 1 (matches opt-out),
   corpus dead-assignment 2 → 0. Additive + third-pass-gated, so default-off is untouched.
2. **Runtime shape changes** — `s5_native_swap_under_running_world` / `s7_debugger_loop_end_to_end`
   assert on record IDs (`a#44` vs `a#1`), which shift when the promotion adds retbuf
   attributes/vars. Needs either stable numbering or the promotion to not perturb unrelated defs.
3. **Codegen panic — FIXED (2026-07-13).** `text_return_analysis_matches_corpus` failed
   because the @PLN85 framework corpus **panicked**: `generate_call_ref: variable is not
   Type::Function` (codegen.rs:3192). **Root cause:** `renumber_frame_var` grouped
   `Value::CallRef(_, xs)` with `Value::Call` and skipped the first field — but `Call`'s is a
   def_nr (correct to skip) while **`CallRef`'s is the fn-ref VARIABLE** (must be remapped).
   When the swap moved a variable that was a local fn-ref's slot (a LOCAL fn-ref at the
   attribute-ordinal index), the `CallRef` v_nr desynced → codegen read a non-`Function` slot.
   Fix: split `CallRef` out and remap its v_nr (collections.rs). Minimal repro
   `g = mk(); g(x)`; verified fixed on BOTH backends + the framework corpus + s5/s7/wrap now
   pass under the promotion.
4. **Native `var__vec_N` E0425 — PARTIALLY fixed (2026-07-13); root is third-pass
   NON-IDEMPOTENCY.** `pre_alloc_vector(&var__vec_1)` was emitted with the work-ref decl
   MISSING/mis-ordered. Root: `data.reset()` only clears `use_names` (not defs), so the third
   pass re-parses the WHOLE file and re-lowers already-refined defs on top of their pass-2
   state — NON-idempotent for vector literals (pass 2: `OpDatabase; _vec=OpGetField(vdb);
   OpPreAllocVector`; third pass: `OpPreAllocVector` FIRST, `_vec=null`, no `OpGetField`). And
   crucially it breaks **unpromoted** collateral defs (`source_roots` returns a vector — not
   force_tret). **Partial fix:** snapshot pass-2 defs; after the third pass, restore every def
   that is NOT force_tret AND not a caller (`Call` of a force_tret def / any `CallRef`) — only
   promoted defs + their callers need the third-pass form (`parser/mod.rs`). **Result: 4 → 2
   native_scripts failures; `index_hygiene` + `scan.loft` now pass.** The remaining 2
   (`tail-capture-lifted-arg.loft`) are vector functions that ALSO need the third-pass form
   (caller/CallRef) → kept, still broken. **Complete fix needs third-pass IDEMPOTENCY for
   vector-literal lowering** — the def re-lowers from pass-2's already-delivered state, not a
   clean pass-1 state. This reinforces that the full-re-parse third pass is the wrong long-term
   vehicle: the clean fix is a TARGETED promotion (patch force_tret signatures + caller
   retbuf-arg pushes, no whole-file re-parse — the deferred "forward-ref pre-pass").

**Net:** the promotion is correct for the fn-ref-call tail in isolation but not yet safe to apply
to EVERY text-returning tail. Reverted to opt-in (`LOFT_TRET_FIX`); suite green. Shipping needs,
in priority order:
- (a) ~~third-pass diagnostic suppression~~ **DONE**
- (b) ~~`generate_call_ref` codegen panic (CallRef v_nr renumber)~~ **DONE** — fixes
  text_return_analysis + s5/s7 + wrap under the promotion
- (c) the native `var__vec_1` E0425 — **PARTIAL**: snapshot/restore non-caller defs fixed
  `index_hygiene` + `scan.loft` and cut `native_scripts` 4 → 2; the remaining 2 need third-pass
  IDEMPOTENCY for vector-literal lowering (or the targeted-promotion redesign below)
- (d) the `ret_index` view-of-local + method-call promotion (still leaks; interp)
- (e) shape-stability for s5/s7 under full-suite parallelism (may be pre-existing flake)
- (f) re-run the six real leakers + full suite green default-on

### Where this leaves the fix

Both parser-side paths hit the same wall — the multi-pass variable model fixes indices on
pass 1/2, and the retbuf's need is only known on pass 3, so it always lands mis-indexed and
correcting it means a deep renumber (var refs + type deps). Two ways forward, in order of
preference:

1. **Build the type-dep-aware renumber, then finish (b2).** Contained, proven target
   (min_wa), flag-gated blast radius. The renumber is the only missing piece.
2. **Abandon the retbuf approach; fix at the leak SITE instead.** The orphan is the
   `skip_free` `__ret_N` in `scopes.rs` B5-L3-text (line ~3485) — deliberately leaked "for
   the duration of the caller's read." A runtime fix that has the CALLER free that buffer
   after its `AppendText` copy (or an interpreter Return-of-owned-text that transfers
   ownership) sidesteps the whole variable-ordering problem. Different subsystem
   (`state`/`scopes`), no parser renumber, but needs its own careful design (the UAF risk
   the `skip_free` was avoiding).

## The original mechanism (why the current P3 still leaks)

The third pass DOES promote the retbuf into the IR — `min.loft` + `LOFT_TRET_FIX`:

```
IR:       fn n_run_t(f, x, ___tret_1:&text) -> text["___tret_1"]  {#block: text["___tret_1"]}
bytecode: n_run_t(f, x, ___tret_1:&text[28]) -> text["__work_1"]        ← header dep WRONG
          ...
          AppendStackText(var[32], v1)     ← delivery into the retbuf (CORRECT, matches workaround)
```

The delivery is already correct (the `block_result` frame-dep fix this session got
`AppendStackText` into the retbuf). What still leaks is the **function's returned-type
dep**, which resolves to the freed local `__work_1` instead of the retbuf `___tret_1`.

**Root cause — attribute-index vs variable-index misalignment (var order).** The slots
are identical between fix and workaround (retbuf at stack slot 32, `__work_1` at 56 in
both). The divergence is only the returned dep `u16`:

| | attributes | variables | returned dep | `.show()` resolves to |
|---|---|---|---|---|
| workaround `r` | f=0, x=1, **r=2** | f=0, x=1, **r=2**, __work_1=3 | attr **2** | var 2 = **r** ✓ |
| fix `___tret_1` | f=0, x=1, **___tret_1=2** | f=0, x=1, **__work_1=2**, ___tret_1=3 | attr **2** | var 2 = **__work_1** ✗ |

`text_return` stores the returned dep as an **attribute index** (`dep.push(a)`); codegen
and `dump_fn_signature` resolve it against the **variable table**. This is only sound
when a param's attribute index equals its variable index — true when params are minted
before locals (pass-1 promotion of `r`), false for `do_tret_bind`'s post-hoc `__tret`,
which is minted as a *variable* AFTER the fn-ref's `__work_1` intermediate (variable 3,
not 2). So the retbuf's attribute-2 dep mis-resolves to variable-2 = the freed
`__work_1`, and the return is codegen'd against a freed buffer → the owned String in the
retbuf orphans on the interpreter (native RAII is unaffected).

## The invariant to restore

> **A promoted retbuf parameter's variable index must equal its attribute index** — i.e.
> parameters occupy the lowest variable indices, before any body local. Equivalently: the
> returned-type dep (attribute space) and the delivery var (variable space) must name the
> same slot.

Pass-1 promotion satisfies this for free (params are declared before the body parses).
`do_tret_bind` breaks it because it promotes a body-local into a param *after* the body
(and its `__work_1`) already claimed the low variable indices.

## The approach — inject the retbuf at DECLARATION on the third pass

Do not spill post-hoc (`do_tret_bind`) on the third pass. Instead, for every def in
`force_tret`, add the `&text` retbuf as a real parameter **when the signature is parsed**
(`parse_function` / `definitions.rs`), before the body — exactly as if the source read

```
fn run_t(f, x, __ret: &text) -> text["__ret"] { __ret = f(x); __ret }
```

Then the whole body parses with `__ret` at the correct variable index (right after the
declared params, before `__work_1`), the tail delivers into it, and every caller
re-lowers against the finalized signature. This IS "re-finalize the signature": the
third pass declares the promoted ABI up front rather than growing it mid-body.

### Concrete touch points

1. **`parse_function` (or the definition parse in `definitions.rs`)** — when
   `self.force_tret.contains(&this_def)` on the third pass, append a hidden `&text`
   retbuf attribute + variable to the signature BEFORE parsing the body, so it takes the
   next variable index after the declared params. Mirror `definitions.rs:1196`
   (`__retbuf` for reference returns) and the workaround's `r` promotion shape.
2. **Tail delivery** — the existing `block_result` `Type::Text` arm + `text_return`
   already deliver a var tail into a `&text` retbuf (that is how the workaround works).
   With the retbuf present as a param from declaration, `do_tret_bind`'s post-hoc spill
   is unnecessary for `force_tret` defs — gate it off for them and let the normal var-tail
   `text_return` path deliver, so attribute index == variable index by construction.
3. **Remove the stopgaps** — once the retbuf is declared in-order, the
   `force_tret`-gated frame-dep patch in `block_result` (this session, `control.rs`) and
   the `force_tret` branch of the `do_tret_bind` gate become redundant; fold them out so
   the third pass produces the same code path as pass-1 promotion.

### The target (verification oracle)

`min_wa.loft`'s `run_t` bytecode is the byte-exact goal — capture it and diff:

```
n_run_t(f, x, r:&text[28]) -> text["r"]        ← header dep = the retbuf, NOT a local
   ...
   AppendStackText(var[32], v1)
   FreeText(var[56])            ← __work_1 freed
   VarRef(var[32]) -> ref r
   GetStackText(r) -> text      ← deref the retbuf before Return (fix currently MISSING this)
   Return(...)
```

The current fix is missing the `GetStackText` deref and has the wrong header dep — both
fall out of the mis-resolved returned dep and should disappear once the retbuf is a
proper in-order param.

## Failure paths to probe (design-protocol)

- **H5 two-pass contract.** The retbuf is added only on the third pass, so the third
  pass's attribute counts differ from pass 2's. `assert_pass2_def_attr_stable` runs
  between pass 2 and the third pass, not after — confirm the third pass is not itself
  asserted, and that no later stage re-checks pass-2 stability against the third-pass def.
- **Callers (forward AND backward ref).** Every call site must re-lower against the
  retbuf signature. The third pass re-parses the whole file with `force_tret` known up
  front, so declaration-time injection should reach all callers — but probe a
  forward-ref caller (caller defined before callee) for the `#551` "Too few parameters"
  crash class.
- **Native backend.** The workaround is native-clean; confirm the injected retbuf emits
  the same native Rust as the workaround (`introspect` carries the native source) — no
  double free (RAII + explicit) and no `Str::new(&local)` dangle.
- **The `-> text?` / optional variant.** `text_return` re-applies `?`; confirm the
  optional retbuf path (`@PLN25` slice c) still promotes in-order.
- **Non-fn-ref force_tret members.** The oracle also flags local-index (`v[0]`) and
  method-call tails. Confirm each lowers to the same in-order retbuf, not just the
  fn-ref case (`corpus.loft` `ret_fnref` + `ret_index`).
- **Over-reach guard.** Scope the declaration-time injection to `force_tret` defs only;
  it must be a no-op for every def the oracle did not flag (the whole 2-pass flow stays
  byte-identical — diff `introspect` on a text-heavy corpus before/after).

## Verification plan

1. `min.loft` + `LOFT_TRET_FIX`: bytecode byte-identical to `min_wa.loft`'s `run_t`
   (header dep = retbuf, `GetStackText` present); leak **0** (ASan `-Cforce-frame-
   pointers=yes` + slow-unwind + `realleak.py`, backgrounded — see RECHECK for the method).
2. `corpus.loft`: `ret_fnref` + `ret_index` leak → 0; `ret_borrow`/`ret_local`/
   `ret_interp` stay byte-identical (no spurious promotion), both backends.
3. The six real nightly text-return-tail leakers (`387`, `85-poison-return-tail-uaf`,
   `85-ncc-container-text-return`, `552`, `553`, `557`): leak → 0, both backends.
4. `#549` guards: no crash, no double free (`-C debug-assertions=on`).
5. Full suite green, both backends; then make the promotion default-on (drop the
   `LOFT_TRET_FIX` gate) once the leak gate confirms it.
6. `35n`/`35p` remain OUT of scope (the match field-projection class — a separate bug;
   the oracle already partitions them out).

## Current state (what this session leaves in place)

- `control.rs` `block_result` `Type::Text` arm: `force_tret`-gated frame-dep preservation
  (`tp = t.clone()`) — got the delivery to `AppendStackText`; folds out under this scope.
- The oracle (`report_tret_promotions`), `force_tret`, and the third pass (`mod.rs`) are
  committed on the plan branch; option (b) reshapes step 2 (declaration-time injection)
  and removes the post-hoc `do_tret_bind` spill for flagged defs.
