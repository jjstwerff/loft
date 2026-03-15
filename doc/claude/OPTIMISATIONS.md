# Runtime Optimisation Opportunities

This document audits the interpreter runtime for concrete performance improvements,
weighing impact against implementation cost and maintainability.

## Contents
- [Completed optimisations](#completed-optimisations)
- [Open opportunities](#open-opportunities)
- [Not worth changing](#not-worth-changing)
- [Open — recommended priority order](#open--recommended-priority-order)

---

## Completed optimisations

| # | Change | File(s) | Notes |
|---|--------|---------|-------|
| 1 | `assert!` → `debug_assert!` in all 3 execute loops | `state.rs` | Eliminates compare+branch on every opcode dispatch in release builds |
| 2 | Remove `.clone()` on iteration (2/3) | `parser.rs`, `state.rs` | `state.rs:1571` clone is necessary — borrow-checker requires `Vec<u32>` collected before `&mut self` re-borrow |
| 3 | `scratch.clear()` at start of `execute()` | `state.rs` | Prevents unbounded growth of temporary `String`s across calls |
| 4 | Hoist `bytecode_len` local before execute loop | `state.rs` | Done together with item 1 |
| 5 | `assert_ne!` → `debug_assert_ne!` in `claim()` | `store.rs` | Consistency with other debug assertions |
| 6 | `sub_text` use `is_char_boundary` | `native.rs` | Replaces manual continuation-byte check; removed now-unused `let b` binding |
| 7 | `Arc<Vec<u8>>` for bytecode/text_code/library in workers | `state.rs`, `parallel.rs`, `database.rs`, `ops.rs` | `WorkerProgram` fields changed to `Arc<Vec<u8>>`/`Arc<Vec<Call>>`; `State` fields likewise; `clone_refs()` replaces `clone_owned()`; `ParallelCtx` field types updated; zero per-thread allocation for read-only bytecode |
| 8 | LLRB free-space tree in `Store` — O(log n) `claim()` | `store.rs` | `free_root: u32`; nodes stored inside free blocks at FL_LEFT/FL_RIGHT/FL_COLOR; `fl_take_ge` fast path; `fl_rebuild()` for open(); `init()` resets tree; `claim_scan` removes last block from tree before `claim_grow` |

---

## Open opportunities

### ~~1. `WorkerProgram`: share bytecode instead of cloning per thread~~ **DONE 2026-03-14**

**Files changed:** `state.rs`, `parallel.rs`, `database.rs`, `ops.rs`

`WorkerProgram` fields are now `Arc<Vec<u8>>`/`Arc<Vec<Call>>`; `State` fields
(`bytecode`, `text_code`, `library`) are likewise `Arc`-wrapped.
`worker_program()` calls `Arc::clone()` instead of deep-cloning.
`clone_refs()` (renamed from `clone_owned`) returns three `Arc` clones — O(1).
`ParallelCtx` raw pointer types updated to `*const Arc<Vec<u8>>`.
Mutation during compilation uses `Arc::make_mut()` (always sole owner,
so no actual cloning occurs).  Per-thread bytecode allocation eliminated.

---

### 2. `Stores::types` and `Stores::names` cloned for every worker

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

### ~~3. `store.claim()` — linear scan through store memory~~ **DONE 2026-03-14**

**Files changed:** `store.rs`

LLRB free-space tree (`free_root: u32`) replaces the O(n) scan.  Tree nodes
are stored inside free blocks at byte offsets FL_LEFT (4), FL_RIGHT (8),
FL_COLOR (12).  Key = `(positive_block_size, position)`.  Only blocks with
size ≥ `MIN_FREE_TREE` (2 words) are tracked; single-word blocks fall through
to the linear scan which is now rarely exercised.

`claim()` fast-path: `fl_take_ge(req_size)` — O(log n).
`delete()`: coalesces adjacent free blocks then `fl_insert()` — O(log n).
`resize()` in-place: `fl_remove()` + optional `fl_insert()` — O(log n).
`fl_rebuild()`: reconstructs the tree from a linear scan after `open()`.
`init()`: resets `free_root = 0` and `claims` so store reuse starts clean.

**Bug fixed during implementation:** `claim_scan` (the slow fallback path)
extended the last free block via `claim_grow()` without first removing it from
the LLRB tree.  The block was then claimed (positive header) while still
reachable as a tree node.  Fixed: `fl_remove(last)` is called before
`claim_grow()` when the last block is free.

**Diagnostics retained:** `fl_validate()` / `fl_validate_node()` (debug-only)
called at START+END of `claim()` and END of `delete()`; `fl_contains()` used
in `fl_remove()` debug assert; `valid()` overflow fix for freed-block headers.

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
| ~~1~~ | ~~`Arc<Vec<u8>>` for bytecode in workers~~ **DONE** | `state.rs`, `parallel.rs` | — | Med (parallel) |
| 2 | `Arc` for `Stores::types` / `names` | `database.rs` | Medium | Low–Med |
| ~~3~~ | ~~Free-list in `store.claim()`~~ **DONE** | `store.rs` | — | High (alloc-heavy) |

---

## See also
- [PLANNING.md](PLANNING.md) — Priority-ordered backlog; T2-1 and T2-2 track these optimisation items
- [INTERNALS.md](INTERNALS.md) — `src/parallel.rs`, `src/store.rs`, `src/state/` implementation details
