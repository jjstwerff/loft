# Phase 3 — Spec: the inline-lift + view-vs-owned invariant

Status: **Done**.

## What landed

Added a new section **"Inline-lift safety — the `OpCopyRecord | 0x8000`
invariant"** to `doc/claude/LIFETIME.md`, between the `OpFreeRef` runtime
section and the `LOFT_LOG=scope_debug` diagnostic section.  Covers:

1. **Why the flag exists** (issue #120 leak prevention for owned returns).
2. **Why it's unsafe for borrowed-view returns** (the P181 corruption class).
3. **The gate** at `gen_set_first_ref_call_copy` + `generate_set`
   reassignment, keyed on `!def.returned.depend().is_empty()`.
4. **The dep-merging helpers** (`text_return` / `ref_return`) and which
   parser paths call them (`block_result` for tails, `parse_return` for
   mid-body).  Documents the Vector asymmetry and the reason for it.
5. **The lock bracket** as second-line defence via `n_set_store_lock`.
6. **Known trade-offs**: owned-fallback leak on mixed-return callees, WASM
   feature behaviour, Vector mid-body not merged.
7. **History** pointer back to this initiative directory.

Other doc locations already up to date:

- `doc/claude/PROBLEMS.md` — P181 entry marked Fixed with full Phase 1/1b
  detail.
- `doc/claude/plans/00-inline-lift-safety/` — full initiative record
  (README, per-phase plans, 18 snippet variants).

## Non-goals for Phase 3

- No new language spec syntax (e.g. `-> Hex[m]` hand-annotation).  Option 2
  of Phase 1b remains deferred; the inference-based merge is sufficient for
  all observed cases.
- No change to the ops reference in INTERMEDIATE.md.  `OpCopyRecord`'s raw
  opcode semantics are unchanged; only the codegen choice about setting
  the flag has evolved.

## Conclusion

The inline-lift-safety initiative closes here.  Every phase (0, 1, 1b, 1c
likely-closed, 1d likely-closed, 2, 2a likely-closed, 2b covered by Phase 2,
3) is done or closed-by-subsumption.  All 18 snippet variants pass; the
full workspace suite passes; P181 is marked Fixed in PROBLEMS.md.
