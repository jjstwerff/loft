# @PLN10 — D (delete `Stores::scratch`) in small verifiable steps

The goal step (delete the field + `Scratch` newtype + sentinel + dead fallbacks).
Earlier this was attempted "delete the field first, fix the fallout" — that creates
ONE large broken-build state where nothing is verifiable until everything is done,
and a mistake (a mis-deleted live function silently misaligning the stack) hides
until the very end.  This is the small-step redesign.

## The invariant each step preserves

> **The field stays alive and the build stays green until the very last step; each
> step removes exactly ONE dead use and is independently `cargo build` + targeted-test
> green before it is committed.**

The order is **inverted**: remove every *use* of `scratch` first (each verifiable
with the field still present), so that deleting the field itself becomes a trivial
removal of an unused declaration — and **the build is then the completeness proof**
(if any use remained, it fails to compile and names the site).

## The deadness pre-condition (the licence to delete)

Every `scratch.push` in the tree is already **dead** — proven by the whole-suite
`LOFT_SCRATCH_TRIP=panic` run reading **zero** (2022/2022, non-wasm; wasm routed by
construction).  That is what makes each removal *behaviour-neutral*: removing dead
code can't change a result.  **Re-run it once as the fresh baseline before starting**
(`LOFT_SCRATCH_TRIP=panic ./scripts/find_problems.sh --bg`, expect zero `scratch.push
hit`).  This is the last time the sentinel is used; it is deleted in the final step.

## The per-step verification gate

Every step ends with, in order:
1. `cargo build --release --lib` (and `cargo check --target wasm32-unknown-unknown
   --no-default-features --features wasm --lib` if the step touched `cfg(wasm)`).
2. The **targeted test** for what the step removed (table below) — green.
3. `git commit` (one logical step per commit → a regression is isolated to one diff,
   inspectable with `git show`; **no `git bisect`** needed).

A step that fails build or test is the culprit *by construction* (nothing else
changed) → fix or revert just that step.

## The ordered steps

| # | Step | Removes | Targeted test gate |
|---|---|---|---|
| **G0** | Re-run the `=panic` baseline | nothing | whole-suite zero `scratch.push hit` |
| **G1** | `emit.rs` × 3: replace each scratch-emitting `else` with `unreachable!()` (a non-`nwb` text return ALWAYS has a work buffer, so the branch is dead) | the 3 generated-code fallback emissions | `native_scripts` + `native_dir`; `--native-emit` over a script → `grep -c 'scratch\.push'` == 0 |
| **G2a** | `extensions.rs` `bridge_text_result` `else` → `debug_assert!(false,…)` + `Str::new("")` | the cdylib-bridge scratch fallback | `multiplayer_v3/v5/v2` + `viewer_markdown` under `=panic` |
| **G2b** | `native.rs` (wasm) `put_owned_text_or_dest` `else` → same graceful form | the wasm scratch fallback | wasm32 `cargo check` (no live wasm harness) |
| **G3** | **CORRECTED — graceful, NOT delete.** The dead non-`_dest` natives are *runtime*-dead (whole-suite `=panic` == 0 — covered positions route to `_dest`), but their **registration is load-bearing**: `tests/doc_hygiene.rs::p54_json_natives_registered_for_every_declaration` scans `native.rs` for the literal `"n_<name>"` of every `default/06_json.loft` declaration, and the codegen general-dispatch fallback resolves the base name.  So deleting the fn/entry breaks `p54` and risks a silent `library_names`-miss → `OpCall` (NOT a loud panic).  Instead: replace each base native's `scratch.push(X)`+`Str::new(scratch…)` output with a shared `put_dead_text(stores, stack, X)` (empty `Str` + dev `debug_assert!`), keeping the fn + registration.  Fns: `n_kind`/`n_to_json`/`n_to_json_pretty`/`n_as_text`/`i_parse_errors`/`n_json_errors`/`struct_to_json_dispatch`/`t_4text_replace`/`t_4text_to_lowercase`/`t_4text_to_uppercase`/`n_source_dir`/`n_store_memory`/`n_ymd_days_ago`/`n_env_variable`/`n_parallel_buf_get_text` | ~15 dead producers → empty stubs | `issues` (JSON) + `wrap` (text) + `196`–`198`; `grep -c scratch.push src/native.rs` == 0; `=panic` re-confirm zero |
| **G4** | `clear_scratch` (`fill.rs`) → **empty no-op body** (do NOT delete the opcode — that shifts every `op_code`, an alignment hazard; leave it a dead no-op). Remove the `codegen.rs` `OpClearScratch` emit-ref if present | the last non-producer scratch use | full `issues` + `wrap` |
| **G5** | **Delete the field**: `Stores::scratch` + `Scratch` + `scratch_trip_mode` + the 4 initialisers + the `allocation.rs` import | the field + sentinel | **build green = the completeness proof** (zero uses remain); then `make ci` |

After G5 the absence of the field is the compile-time guard: any future
`stores.scratch` is a hard error.

## Why each removal is safe to verify locally

- **G1** is generator-Rust text, not a live `scratch` ref — it can't break the loft
  build; the gate is that *generated* code no longer contains `scratch.push`.
- **G2** changes only the dead `None`-case of a function whose live (`Some`-dest)
  path is exercised by the server tests — those tests pin the live path.
- **G3** deletes functions the `=panic` zero proved are never called; a wrong
  deletion surfaces *loudly* (codegen `library_names` miss, or a failing producer
  test), never silently — the targeted test is chosen to exercise exactly that
  producer through its `_dest` variant.
- **G5** cannot leave a dangling use: if one existed the build fails and names it.

## Deferred F-cleanup (the graceful stubs → real deletion)

G3 leaves the ~15 base natives as empty registered stubs (dead but present).  Fully
deleting them is a separate **F** pass that needs design, not mechanics:
1. Update `p54_json_natives_registered_for_every_declaration` to accept the `_dest`
   registration as satisfying a declaration (the hygiene invariant becomes "every
   JSON native has a registered impl OR a registered `_dest` variant").
2. Extend codegen so EVERY call context of an `is_text_dest_native` / cdylib text fn
   routes through `_dest` (today the general-dispatch fallback resolves the base name
   for shapes the chokepoint/`set_var` don't cover — those are runtime-dead but still
   emit the base lib_nr).  Then a base deletion is a clean `library_names` miss.
3. Delete the stub bodies + their FUNCTIONS entries.

## Optional follow-up (NOT part of D's goal)

- **G6**: fully delete the `OpClearScratch` opcode (loft decl in `default/02_files.loft`
  + the `OPERATORS` entry + `clear_scratch` fn) *together*, keeping declaration-order
  ↔ `OPERATORS`-index alignment.  Riskier (op_code shift); the field is already gone
  after G5, so this is pure polish and can wait for its own focused change.
