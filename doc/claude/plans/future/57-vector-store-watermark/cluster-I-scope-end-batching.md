# Cluster I — named store-backed locals freed at scope-end, not last-use

## Shape

A named vector local holds its store from allocation until the enclosing **function
scope exits**, regardless of when the local is last read. A function with N distinct
store-backed locals therefore holds ~N stores simultaneously near its end, even when most
are dead.

## Severity (two fields)

- **Corruption / panic / hang:** none.
- **Leak (escapes scope):** **none** — every store frees at scope exit; the `loft_suite`
  exit gate (`tests/wrap.rs:276`) passes. This is a *watermark/efficiency* finding only.

## Verified

- ✅ **All-allocs-then-all-frees, zero interleaving.** 11-vectors trace under
  `LOFT_STORES=log` is `aaaa…(44)…ffff…(42)` — no store frees until a single burst at
  function teardown. The last 12 events are all `- free`, `active` counting down to 3.
  *(trace: RESULTS.md § Key traces; reproduce per RESULTS.md § Reproduce.)*
- ✅ **It is per-binding, not per-allocation.** Probe 07 (35 named locals) → watermark 72,
  all freed at scope-end. Probe 02 (20-iteration loop, one local) → watermark 4, the
  store **reused in place** each iteration. A loop body is a scope that exits each
  iteration, so its store is reclaimed; 35 sibling statements in one scope are not.
  *(traces: RESULTS.md probe matrix rows 02, 07.)*
- ✅ **Binding is what pins.** The same value as an *unbound* temp (probe 08, slice in a
  format string) frees/reuses at statement-end → watermark 4 across 35 statements. So the
  pin is a property of named-local lifetime, not of the value being a vector.
  *(trace: RESULTS.md probe matrix row 08.)*
- ✅ **Both backends.** Interp watermark 44, native 42 on 11-vectors. Not a backend
  artifact.

## Hypothesized (Stage B — needs source reading)

- 🤔 **Store-free is anchored to scope exit, not to variable last-use.** loft does
  live-interval analysis (`src/scopes.rs`), but the *store* free for a store-backed local
  appears to be emitted at scope-end rather than at the end of the local's live interval.
  The trace is consistent with this but the emission site is unconfirmed. **Action:** read
  `src/scopes.rs` + the free-emission in `src/state/codegen.rs`; find where `OpFreeRef`
  for a function-local DbRef is anchored.
- 🤔 **This may be deliberate.** [LIFETIME.md](../../../LIFETIME.md) describes scope-based
  freeing as the model. If store-free *intentionally* fires only at scope exit (for
  dep-tracking / aliasing safety — the PLAN51/52 surface), then cluster I is "working as
  designed" and the fix is a heuristic change (raise the `LOFT_STORES=warn` floor), not a
  lifetime change. This is the central Stage-C design question and is the user's call.

## Fix options (Stage C — do not implement without the design decision)

1. **Do nothing (watermark is benign).** Argue that scope-end freeing is correct and the
   only defect is the noisy heuristic → raise/relativise the `LOFT_STORES=warn` threshold.
   Lowest risk; the watermark itself remains O(locals).
2. **Free store-backed locals at last-use.** Emit `OpFreeRef` at the end of a local's live
   interval instead of scope-end. Reduces watermark but adds free-emission complexity and
   interacts with dep-tracking/aliasing (cannot free a local whose store is aliased by a
   still-live binding — exactly the PLAN51/52 hazard). Needs the full aliasing analysis.

## Probes

`02`, `07`, `09` (+ contrast `08`); field repro `tests/scripts/11-vectors.loft`.
