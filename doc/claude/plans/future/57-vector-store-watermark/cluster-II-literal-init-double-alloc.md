# Cluster II — `local = [literal]` allocates two scope-pinned stores

## Shape

Initialising a named vector local from a literal — `a = [1, 2]` or
`a: vector<integer> = [1, 2]` — allocates **two** stores: the literal's store and the
local's store. Both are pinned to function scope (cluster I), so the redundant init-temp
roughly **doubles** the function's vector-store watermark.

## Severity (two fields)

- **Corruption / panic / hang:** none.
- **Leak (escapes scope):** **none** — both stores free at scope exit. Watermark concern
  only, but a *constant ×2* multiplier on top of cluster I's O(locals).

## Verified

- ✅ **2 stores per literal-init local.** Probe 07 (35 locals) → 72 allocs ≈ 2×35 + 2
  (the +2 = CONST_STORE #1 and the main locals/schema store). All 72 freed at scope-end.
  *(trace: RESULTS.md probe matrix row 07.)*
- ✅ **Annotation-independent.** Probe 09 (untyped `a = [..]`) → also 72, identical to the
  typed probe 07. So the doubling is **not** caused by the `: vector<integer>` coercion;
  it is the literal-init temp itself. *(traces: RESULTS.md probe matrix rows 07 vs 09;
  README § Reference ↔ problem pairings 09↔07.)*
- ✅ **Consistent with the field repro.** 11-vectors has ~22 named vector locals; observed
  watermark 44 ≈ 2 × 22. The 2×-per-local model predicts the field number.

## Hypothesized (Stage B — needs source reading)

- 🤔 **The literal store is copied into a separate local store rather than becoming it.**
  `a = [literal]` looks like it (1) allocates a store for the literal, (2) allocates the
  local's store, (3) copies elements across, (4) leaves the literal store pinned to scope.
  If the literal store could simply *become* the local's store (move, not copy), the
  watermark would halve with no lifetime-model change. **Action:** read the assignment/
  init codegen for `local = <vector-literal>` in `src/state/codegen.rs` (and the parser's
  materialisation in `src/parser/expressions.rs` / `collections.rs`); find the literal-temp
  alloc and confirm whether a copy-then-pin or a move is emitted.
- 🤔 **This is likely the cheaper, lower-risk half of @P393.** Unlike cluster I (which
  touches last-use lifetime and the aliasing surface), eliminating a redundant init-temp
  is a local codegen change that does not alter *when* the local frees — so it should not
  interact with dep-tracking. If confirmed, fix cluster II first: it halves the watermark
  on its own and de-risks the cluster-I design decision.

## Fix options (Stage C)

1. **Move the literal store into the local** (reuse store #literal as the local's store)
   instead of alloc-local + copy + pin-temp. Halves the watermark; no lifetime change.
   Must confirm no other binding aliases the literal store (it is freshly built from a
   literal, so aliasing is unlikely — but verify against the PLAN51/52 hazard list).
2. **Free the init-temp at statement-end** (emit `OpFreeRef` for the literal temp right
   after the copy). Simpler than the move but leaves the copy cost; reduces watermark
   without reusing the store.

## Probes

`07` (typed, 72), `09` (untyped, 72); contrast `08` (unbound temp, 4) and `02` (loop reuse, 4).
