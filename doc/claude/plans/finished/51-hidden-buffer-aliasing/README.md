# Plan 51 — Hidden-buffer aliasing under buffer reuse

**Status — DONE 2026-05-29.**

Reference content for the post-plan state moved to:
- [`../../../DEBUG.md`](../../../DEBUG.md) — `LOFT_TRACE_DB`, `LOFT_TRACE_CR`, `LOFT_TRACE_COPY`, `LOFT_TRACE_FINISH`, `LOFT_KEEP_NATIVE_RS` tracer levers (under § "Debugging a Tricky Compiler Bug" levers table).
- `tests/scripts/14[1-9]-plan51-*.loft` + `150-plan51-cluster5c-lambda-with-captures.loft` — 10 graduated regression probes (one per distinct cluster mechanism + two real-library extractions).

This file is a closure record only.  The per-cluster docs + 62 probe files are kept here as the diagnostic archaeology that produced the fixes.

---

## What shipped

12 commits on `p377-fix` over 2026-05-28 → 2026-05-29 closed all 5 clusters:

| Cluster | Closing commits | Net effect |
|---|---|---|
| I (canonical leak + corruption) | `6909177e`, `d7d6ebcf` | S1 parse-time NRVO + S2 narrow codegen gate close the @P377 oracle shape on both backends |
| II (latent leak, 9 probes) | `ff0b38d4`, `db8fd532`, `e4fca573` | Extended S1 + Step 2 (refined `is_borrowed_view` + post-call `OpVarRef→OpFreeRef→OpInitRefSentinel`) + narrowed `is_hidden_buf_arg` to require S1.  See [`cluster-II-latent-leak.md`](cluster-II-latent-leak.md) for the 3-attempt fix-iteration journal |
| III (silent data corruption) | `d710e399`, `e4fca573` | Probes 04 + 28 closed by the cluster-II narrowed-guard fix |
| IV (codegen panic on both backends) | `d630e68b` | `caller_hidden_buf` flag + null-init unblocks the slot allocator for the if-tail / recursion / match-tail family |
| V-a (tuple schema mismatch) | `b69a1707` | `Stores::add_tuple_group` propagates `field_groups` from compile-side to native-runtime database |
| V-b (nested tuple codegen) | `92ebe8dc` | `emit_tuple_set_ops` detects heap-promoted nested-tuple Call returns and emits a single OpCopyRecord instead of mismatched casts |
| V-c (lambda dispatch, native + interp) | `e4cd328d`, `5eb7d90d` | Native: candidate-filter excludes hidden args + per-arm hidden-buf emit.  Interp: `fn_call_ref` introspects callee attributes + pushes hidden DbRefs |
| Probe 39 (moros_map leak) | (incidental) | Closed by cluster-II Step 2 + narrowed-guard; no dedicated fix needed |

Tools added (all env-var-gated, zero cost when unset):
- `LOFT_TRACE_DB` (`src/state/io.rs:717`) — every OpDatabase call.
- `LOFT_TRACE_CR` (`src/state/io.rs:1237`) — every interp OpCopyRecord with src/dst field reads.
- `LOFT_TRACE_COPY` (`src/codegen_runtime.rs`) — native-side OpCopyRecord trace.
- `LOFT_TRACE_FINISH` (tuple types) — finish_type entry/exit.
- `LOFT_KEEP_NATIVE_RS` — preserves the generated Rust at /tmp/loft_native_*.rs.

10 probes graduated to `tests/scripts/` (commit `3a1a6ec8`); see [§ Graduated probes](#graduated-probes) for the substitution rationale (probes 08, 30, 40 not graduated — separate downstream bugs).

---

## Graduated probes

| Cluster | Probe | Graduated file |
|---|---|---|
| I canonical | 01 | `tests/scripts/141-plan51-canonical-immediate.loft` |
| II double-Set | 02 | `tests/scripts/142-plan51-cluster2-double-set.loft` |
| III mixed-lit | 04 | `tests/scripts/143-plan51-cluster3-mixed-lit-call.loft` |
| IV (substituted from 08) | 18 | `tests/scripts/144-plan51-cluster4-match-tail.loft` |
| II many-iters stress | 21 | `tests/scripts/145-plan51-cluster2-many-iters.loft` |
| II conditional set | 28 | `tests/scripts/146-plan51-cluster2-conditional-set.loft` |
| V-a tuple-return | 29 | `tests/scripts/147-plan51-cluster5a-tuple-return.loft` |
| real-lib gridmesh | 38 | `tests/scripts/148-plan51-gridmesh-real-lib.loft` |
| real-lib moros_map | 39 | `tests/scripts/149-plan51-moros-map-real-lib.loft` |
| V-c (substituted from 30) | 53 | `tests/scripts/150-plan51-cluster5c-lambda-with-captures.loft` |

Substitutions: probe 08 SIGSEGVs at process teardown despite `PASSED` printing (separate downstream bug, NOT part of cluster IV's codegen-panic class).  Probes 30 and 40 still leak Canvas×6 / Canvas×24 per iter despite corruption fixes; no clean substitute available for V-b (sole probe), so it isn't graduated.

---

## Investigation deliverables (kept here as archaeology)

| Document | What it contains |
|---|---|
| [`RESULTS.md`](RESULTS.md) | Full 63-probe matrix, cluster definitions, verified-vs-hypothesized findings |
| [`cluster-II-latent-leak.md`](cluster-II-latent-leak.md) | Cross-iter slot dangling mechanism + 3-attempt fix-iteration journal |
| [`cluster-III-corruption.md`](cluster-III-corruption.md) | Over-broad `is_hidden_buf_arg` guard mechanism |
| [`cluster-IV-codegen-panic.md`](cluster-IV-codegen-panic.md) | `unify_if_branches_work_refs` substitution gap |
| [`cluster-V-native-only.md`](cluster-V-native-only.md) | Tuple schema-propagation + lambda dispatch hidden-buf emission |
| `probes/` (62 files) | Each shape explored during the investigation; one comment-header explaining what it's testing |

---

## Closure-record archaeology

PLAN51 was opened 2026-05-27 as the investigation-shape companion to @P377's narrow fix.  The original @P377 was filed against a 2-fn corruption oracle; investigation surfaced four additional clusters (II/III/IV/V) covering distinct failure mechanisms in the same shared codegen surface.

Fix-arc lessons (now codified in [`../../_INVESTIGATION_TEMPLATE.md`](../../_INVESTIGATION_TEMPLATE.md)):

1. **Stage C is often skippable** — when Stage B's mechanism analysis uniquely determines the fix shape (the bug is one gate; the question is the predicate change), go B → D iteratively.  PLAN51 went B → D for all 5 clusters; no Stage C deliverable was needed.
2. **A/B/C probe curation is OPTIONAL** — PLAN51 carried the curation table but implementers worked from cluster docs, not the table.  The flat probe-list is the default; A/B/C earns its overhead only at >15 probes + multiple cold readers.
3. **Promotion gate beyond assertions** — probes 08 (SIGSEGV at teardown) and 22 (infinite loop) passed the assertion check but failed graduation.  Gate requires: assertions PASS + clean process exit + no leak + bounded runtime.
4. **Severity split per cluster** — corruption / panic / hang must be tracked separately from leak.  PLAN51's Cluster III was marked FIXED on corruption-closed while leaks persisted under Cluster II; conflation caused two false-fix moments in the follow-up session.
5. **Fix-iteration journal** — when a fix takes >1 attempt, commit messages capture each landed change in isolation but the SEQUENCE (and why attempt N didn't suffice) is the load-bearing context.  Cluster II took 3 attempts; without the journal, attempt-3's predicate would look arbitrary.

Template revisions landed in commit `516a9683`; PLAN51's own cluster docs retrofitted to the new shape in `1b8fba6f`.

---

## See also

- [`../../../DEBUG.md`](../../../DEBUG.md) — `LOFT_TRACE_*` tracer reference (added during this plan).
- [`../../../LIFETIME.md`](../../../LIFETIME.md) — dep tracking + hidden-buffer-passing semantics that this plan's fixes operate on.
- [`../../../PROBLEMS.md`](../../../PROBLEMS.md) — @P377, @P378(a), @P381, @P382 (all closed; PLAN51's clusters are the breadth-extension).
- [`../../_INVESTIGATION_TEMPLATE.md`](../../_INVESTIGATION_TEMPLATE.md) — investigation-plan template (revised from PLAN51 lessons).
- `project_drop_store_refcount.md` — the documented long-term direction (store-refcount); deferred because PLAN51's targeted fixes closed all clusters without it.
