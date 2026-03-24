# Test Failures

> **HISTORICAL** — Recorded 2026-03-20. Both bugs documented here are now fixed.
> Bug 1 (slot conflict) was resolved by the two-zone slot assignment redesign (A6.3a/b, A13/A14/A15).
> Bug 2 (`ref_param_append_bug`) was fixed by the S4/binary I/O work in 0.8.2.
> All 23 tests that were failing now pass. See [SLOT_FAILURES.md](SLOT_FAILURES.md) for the slot analysis.

Recorded 2026-03-20. Two distinct root-cause bugs produced all 23 failures.

---

## Bug 1 — Stack-slot conflict after sequential `for` loops (slot assignment)

**Root cause:** The slot-assignment algorithm in `src/variables/` reuses stack slots across sequential `for`-loop bodies.  When a wide type (`long` = 8 bytes, `vec<T>` = 12 bytes, `ref` = 12 bytes) is allocated first and a narrower type is allocated later, the narrower type can be given a starting address inside the still-live range of the wider type, producing an overlap that the post-pass verifier at `src/variables/:1195` detects and panics on.

**Common pattern:** Two or more `for` loops in sequence within the same function, where `r#iter_state : long` (8 bytes) from the first loop overlaps with an integer or index variable from the second loop.

**Verifier error:** `Variables 'X' (slot [...), live [...]) and 'Y' (slot [...), live [...]) share a stack slot while both live`

**Affected tests — `tests/vectors.rs` (8 failures):**

| Test | Function | Conflicting pair | Slots |
|---|---|---|---|
| `append_vector` | `n_test` | (not printed — output empty) | — |
| `growing_vector` | `n_test` | `_vector_2` (vec, 12 B) vs `elm#index` (int, 4 B) | `[104, 116)` vs `[104, 108)` |
| `index_iterator` | `n_test` | `total` (int, 4 B) vs `r#iter_state` (long, 8 B) | `[80, 84)` vs `[80, 88)` |
| `index_key_null_removes_all` | `n_check` | `r#index` (int, 4 B) vs `r#iter_state` (long, 8 B) | `[48, 52)` vs `[48, 56)` |
| `index_loop_remove_large` | `n_test` | `r#index` (int, 4 B) vs `r#iter_state` (long, 8 B) | `[68, 72)` vs `[68, 76)` |
| `index_loop_remove_small` | `n_test` | `cnt` (int, 4 B) vs `r#iter_state` (long, 8 B) | `[52, 56)` vs `[52, 60)` |
| `sorted_filtered_remove_large` | `n_test` | `r#index` (int, 4 B) vs `r#iter_state` (long, 8 B) | `[40, 44)` vs `[40, 48)` |
| `sorted_remove` | `n_test` | `total` (int, 4 B) vs `e#iter_state` (long, 8 B) | `[52, 56)` vs `[52, 60)` |

**Affected tests — `tests/wrap.rs` (14 failures, all loft script tests):**

All 14 fail with the same `variables/:1195` panic. The `collections` script additionally shows:
- Function `n_main`: `idb` (ref, 12 B, slot `[332, 344)`, live `[271, 2090]`) vs `r#index` (int, 4 B, slot `[340, 344)`, live `[474, 497]`)

| Script test | File |
|---|---|
| `collections` | `tests/scripts/10-collections.loft` |
| `control_flow` | `tests/scripts/06-control-flow.loft` |
| `dir` | `tests/scripts/17-dir.loft` |
| `enums` | `tests/scripts/09-enums.loft` |
| `file_debug` | `tests/scripts/16-file-debug.loft` |
| `formatting` | `tests/scripts/05-formatting.loft` |
| `last` | `tests/scripts/20-math-functions.loft` (last script run) |
| `loft_suite` | `tests/scripts/00-loft-suite.loft` |
| `map_filter_reduce` | `tests/scripts/14-map-filter-reduce.loft` |
| `script_threading` | `tests/scripts/15-threading.loft` |
| `stress` | `tests/scripts/12-stress.loft` |
| `text` | `tests/scripts/03-text.loft` |
| `threading` | `tests/scripts/15-threading.loft` |
| `vectors` | `tests/scripts/10-collections.loft` |

**Fix path:** In `src/variables/`, when a slot for a new variable is selected after an existing wide-type variable's live range has ended, ensure the candidate slot does not partially overlap with the wide type's byte range.  The allocator must align new allocations such that `new_start >= wide_var_end` (not merely `new_start >= wide_var_start`).  See `ASSIGNMENT.md` for the full slot-assignment design.

---

## Bug 2 — `ref_param_append_bug`: index out of bounds in `src/keys.rs:143`

**Test:** `ref_param_append_bug` in `tests/issues.rs`

**Test scenario:**
```loft
struct Item { name: text, value: integer }
fn fill(v: &vector<Item>, extra: vector<Item>) { v += extra; }
fn test() {
    buf = [Item { name: "a", value: 1 }];
    fill(buf, [Item { name: "b", value: 2 }]);
    assert(len(buf) == 2, "len after fill: {len(buf)}");
    assert(buf[1].value == 2, "buf[1].value: {buf[1].value}");
}
```

**Error:** `panicked at src/keys.rs:143:10: index out of bounds: the len is 4 but the index is 18752`

**Root cause (from `PROBLEMS.md`):** When `v += extra` is compiled for a `v: &vector<T>` ref-param, codegen emits `OpAppendVector` with the raw ref-param `DbRef` (a stack pointer into the caller's frame via `OpCreateStack`) rather than the actual vector `DbRef`.  `vector_append` calls `store.get_int(v.rec, v.pos)` to read the vector header; `v.rec` is the caller's stack-frame record, which is not present in the current function's store `claims` — the bogus record index (18752) is far out of bounds for the local store.  In release builds the `debug_assert` is elided and the append silently does nothing.

**Fix path (from `PROBLEMS.md`):**
1. Emit `OpGetStackRef` to dereference the ref-param and load the actual vector `DbRef`.
2. Emit `OpAppendVector` with the loaded `DbRef`.
3. Emit `OpSetStackRef` to write back the (possibly reallocated) `DbRef` through the ref.

---

## Summary

| Bug | Root cause file | Tests affected | Severity |
|---|---|---|---|
| Slot-conflict after sequential loops | `src/variables/` | 22 (8 Rust + 14 script) | High — blocks entire `wrap` test suite |
| `&vector<T>` append via ref-param | `src/state/codegen.rs` | 1 | Medium — silently corrupts in release |

---

## See also
- [PROBLEMS.md](PROBLEMS.md) — Canonical open-issue tracker with fix paths and severities
- [SLOTS.md](SLOTS.md) — Two-zone slot design that resolved the slot-conflict root cause
- [ASSIGNMENT.md](ASSIGNMENT.md) — Full history of the A6 slot-assignment redesign
- [SLOT_FAILURES.md](SLOT_FAILURES.md) — Detailed A/B/C bug-category analysis for the slot failures
- [TESTING.md](TESTING.md) — How to reproduce failures and use `LOFT_LOG` for diagnosis
