# Runtime Optimisation Opportunities

This document audits the interpreter runtime for concrete performance improvements,
weighing impact against implementation cost and maintainability.

## Contents
- [Open opportunities](#open-opportunities)
- [Not worth changing](#not-worth-changing)
- [Open — recommended priority order](#open--recommended-priority-order)

Completed optimisations (debug_assert, clone removal, Arc bytecode sharing, LLRB free-list)
are recorded in CHANGELOG.md.

---

## Open opportunities

### 1. `Stores::types` and `Stores::names` cloned for every worker

**File:** `database.rs:1541-1561`

`clone_for_worker` copies:

- `types: self.types.clone()` — `Vec<Type>`, read-only after compilation
- `names: self.names.clone()` — `HashMap<String, u16>`, read-only after compilation

Both are pure metadata that no worker modifies.  Wrapping them in
`Arc<Vec<Type>>` and `Arc<HashMap<String, u16>>` would reduce the per-worker
clone to two atomic-ref-count increments.

For a program with 200 types and a 500-entry name map the savings are small in
absolute bytes, but the pattern becomes significant if the type system grows or
if hundreds of parallel calls are made.

**Impact:** Low-Medium — mainly prevents future scaling problems
**Cost:** Medium — field types change throughout `database.rs`; some methods need `Arc::make_mut` if mutation is ever needed before `clone_for_worker` is called
**Verdict:** Defer until parallel usage grows; note the shape of the fix here

---

## Not worth changing

| Pattern | Reason |
|---|---|
| `State` HashMap fields (`stack`, `vars`, `calls`, `types`, `line_numbers`) | Only accessed in debug/dump functions, not in the hot execute loop |
| `WorkerProgram` channel + batching in `parallel.rs` | `Vec::with_capacity(end-start)` is already exact; no reallocation |
| `calc.rs` BTreeMap for struct layout | Compile-time only; immeasurable runtime effect |
| `library_names: HashMap<String, u16>` | Queried during compilation, not execution; worker states leave it empty |
| Function pointer dispatch table in `fill.rs` | Already optimal for an interpreter; JIT is the next step |

---

## Open — recommended priority order

| # | Change | File(s) | Effort | Impact |
|---|--------|---------|--------|--------|
| 1 | `Arc` for `Stores::types` / `names` | `database.rs` | Medium | Low–Med |

---

## See also
- [PERFORMANCE.md](PERFORMANCE.md) — Benchmark results, root-cause analysis, and detailed designs for O1–O7 (superinstructions, stack pointer cache, native collection emit, purity analysis)
- [PLANNING.md](PLANNING.md) — Priority-ordered backlog
- [INTERNALS.md](INTERNALS.md) — `src/parallel.rs`, `src/store.rs`, `src/state/` implementation details
