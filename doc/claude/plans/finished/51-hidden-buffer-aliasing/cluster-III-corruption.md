# Cluster III — Silent data corruption (interpret-only)

**Severity (split by failure mode):**
- **Corruption / panic / hang:** WAS the worst data-integrity class — silent wrong-value reads with no warnings (probe 04 iter 2: `cv_c.data[0] = 1` instead of `2`; probe 28 iter 2: `cv.w = 0` instead of `4`).  CLOSED 2026-05-28 (commit `d710e399`).
- **Leak:** WAS Canvas×5/iter; CLOSED 2026-05-29 via the Cluster II arc (probes 04 + 28 also leaked even after corruption was fixed; the Cluster II `is_hidden_buf_arg` narrowing closed both).

**Status (2026-05-29): 🟢 CLOSED.**

**Affected probes:** 04 (mixed-lit-call), 28 (only-conditional-set) — both graduated to `tests/scripts/143-plan51-cluster3-mixed-lit-call.loft` + `146-plan51-cluster2-conditional-set.loft`.
**Backend asymmetry:** `--interpret` corrupted; `--native` was always clean (Rust borrow semantics keep `&mut` buffer modifications visible to the caller).

The two probes had **different** symptom paths but the same root mechanism: an in-place struct-literal first Set followed by a non-S1 Call Set on the same hidden-buffer var.  Treated as one cluster from 2026-05-28.

## Mechanism — verified

For both probes, the body had:

1. A FIRST initialization of `cv` via struct-literal (direct field-init on cv's slot via `OpDatabase` + `OpSetInt*`).  When `cv` is a hidden-buffer parameter, `parse_object` (at `src/parser/objects.rs:1538-1544`) skips the `v_set(Null)` prelude and emits `OpDatabase` directly on cv's slot.
2. A SECOND assignment via `Set(cv, Call(...))`, where the Call used a fresh `__ref_local` as its hidden buffer (S1 did NOT substitute).

The pre-Set OpFreeRef on cv's slot was suppressed by the `is_hidden_buf_arg` guard at `src/state/codegen.rs:1406` — the guard's rationale was "the callee's OpDatabase will reuse cv's store in-place via `clear+claim`", which is true ONLY when the Call's hidden-buf arg IS `cv` (S1-substituted in-place reuse).  When the Call's hidden buf is a distinct `__ref_local`, the assumption breaks: cv's current store (from the struct-literal init) becomes orphan when the reassignment writes the deep-copied result, AND the callee's local `cv` is updated to point to `__ref_local`'s store while the caller-side buffer still holds the original.

Probe 28 had additional control-flow structure: the second Set was inside an `if p.tag > 0` wrapper.  Same mechanism, just gated.

### Reference probe 01 (CLEAN)

```loft
fn render_p(p: P) -> Canvas { cv = alloc_canvas(4, 5, p.tag); cv }
```

S1 fires (penultimate Set immediately precedes tail Var) → call writes IN-PLACE through cv → no caller-slot-vs-local-cv divergence.

### Problem probe 04 (mixed-lit-call)

```loft
fn render_lit_then_call(p: P) -> Canvas {
  cv = Canvas { data: [], w: 1 };    // struct-lit: OpDatabase(cv) + field-set
  cv = alloc_canvas(4, 5, p.tag);    // Set: S1 doesn't fire (first stmt isn't Set(cv, Call))
  cv
}
```

S1's backward walk through consecutive `Set(cv, Call(_))` halts at the struct-literal-lowered ops (they're `OpDatabase` + `OpSetInt*`, not a `Set` with `Call` RHS) — so the call's hidden buf stays as `__ref_1`.

### Problem probe 28 (only-conditional-set)

```loft
fn render(p: P) -> Canvas {
  cv: Canvas = Canvas { data: [], w: 0 };  // default-init: same OpDatabase pattern
  if p.tag > 0 { cv = alloc_canvas(4, 5, p.tag); }
  cv
}
```

Same shape as probe 04 plus the If wrapper.  S1's penultimate check requires `Set(cv, Call)` directly — `If` wrapping disqualifies.

## The fix (commit `e4fca573`)

Narrowed `is_hidden_buf_arg` at `src/state/codegen.rs:1406` to additionally require `s1_substituted`:

```rust
let is_hidden_buf_arg = s1_substituted
    && stack.function.is_argument(v)
    && { /* attribute hidden check */ };
```

When the second Set is NOT S1-substituted (probe 04 / 28), the narrowed guard is false, so the reassignment path's pre-Set OpFreeRef fires on cv — reclaiming the struct-literal init's store before the deep-copy.  The Cluster II Step 2 post-call free + sentinel reset (commit `db8fd532`) takes care of the placeholder slot.

## What we know vs. don't

| Claim | Status |
|---|---|
| IR shapes for 04 and 28 | ✅ `/tmp/bc_04.txt`, `/tmp/bc_28.txt` |
| Both have struct-lit + non-S1 Call Set sequence | ✅ Verified |
| S1 doesn't substitute because penultimate isn't `Set(cv, Call)` | ✅ Verified — probe 04's penultimate is the struct-lit's `OpSetInt`; probe 28's penultimate is the `If` |
| `is_hidden_buf_arg` over-broad gate was the corruption site | ✅ Verified by fix — narrowing the gate closed both |
| Slot allocation is clean for 04, 28 | ✅ `LOFT_LOG=slots:n_render` — only is_argument SKIPs |
| Native escapes via `&mut` borrow semantics | ✅ Verified by inspection of generated Rust |

## Fix iterations

Single attempt — the narrowed `is_hidden_buf_arg` predicate closed both probes on first try.  See [`cluster-II-latent-leak.md` § Fix iterations](cluster-II-latent-leak.md#fix-iterations) for the iteration journal that established the diagnostic vocabulary; this cluster's fix landed by applying that vocabulary directly.

## Why native escapes

Native lowers `cv = expr` where cv is a `&mut` parameter to a Rust assignment that updates through the borrow.  The caller's slot reflects the update automatically.  Interp's bytecode VM has no equivalent — the codegen has to emit the right opcodes to keep the buffer-slot and the callee-local-cv in sync, and the `is_hidden_buf_arg` skip was breaking that sync.

## Historical: previous attempts (appendix)

Earlier mechanism hypotheses that proved incomplete:

- **"Recursive child-store free in `OpDatabase` REUSE path" (2026-05-28 investigation agent)** — proposed as the Cluster II + III joint fix.  Not needed: the over-broad `is_hidden_buf_arg` gate turned out to be the operative bug, fixed with a single-line predicate change.  The recursive-free approach would have addressed a different (real but currently unreached) shape; reserve for any future probe that doesn't match `caller_hidden_buf` or S1.
- **"S1 doesn't fire because l's layout differs at S1's invocation"** — investigation hypothesis before the IR was actually dumped.  After dumping the IR, the real reason became clear: S1's penultimate check requires `Set(cv, Call)` literally, and both probes 04 + 28 violate that literally.
- **"Make the Set always write through the buffer slot"** — proposed Fix Surface (a).  Implicitly accomplished by the narrowed `is_hidden_buf_arg`: routing through the reassignment path now emits the OpCopyRecord that writes the deep-copy into cv's slot.

## Tools added during this cluster's investigation

None new — investigation reused `LOFT_LOG=static` for IR dumps, `LOFT_STORES=warn` for leak counts, and the LOFT_TRACE_DB/CR tracers Cluster II added.
