# Cluster II — Latent leak (interpret-only)

**Severity (split by failure mode):**
- **Corruption / panic / hang:** NONE (assertions always PASS).
- **Leak:** WAS one Canvas per iter, linear scaling (probe 21 confirmed at 100 iters).  CLOSED 2026-05-29.

**Status (2026-05-29): 🟢 CLOSED** — all 9 Cluster II probes (02, 03, 04, 07, 11, 21, 25, 26, 28) pass leak-free on `--interpret`.  Three landed commits over two days:

| Commit | Closes | Mechanism |
|---|---|---|
| `ff0b38d4` | 02, 21 | Extends `nrvo_collapse_tail_set` backwards through consecutive `Set(cv, Call(_))` ops |
| `db8fd532` | 03, 07, 11, 25, 26 (partial) | Step 2 refines `is_borrowed_view` + post-call `OpVarRef→OpFreeRef→OpInitRefSentinel` reset for caller-hidden-buf args |
| `e4fca573` | 04, 28 | Narrows `is_hidden_buf_arg` to require S1 substitution — non-S1 reassignment falls through to the reassignment path that emits the pre-Set free |

**Affected probes:** 02, 03, 04, 07, 11, 21, 25, 26, 28 (9 probes — all now graduated to `tests/scripts/14[2-6]-plan51-*.loft`).
**Backend asymmetry:** `--interpret` leaked; `--native` was always clean (Rust's `Drop` recursively frees child stores).

## Mechanism — pinned via IR diff + bytecode trace

### Reference probe 01 (canonical, CLEAN)

```loft
fn render_p(p: P) -> Canvas { cv = alloc_canvas(4, 5, p.tag); cv }
```

Lowered IR (after S1 substitution):

```
fn n_render_p(p:P, cv:Canvas) -> Canvas["cv"] {
  __ref_1(1):ref(Canvas) = null;
  [30] cv(0):ref(Canvas) = n_alloc_canvas(4i32, 5i32, OpGetInt(p(0), 0i32), cv(0));
                                                                            ^^^^^
                                                            S1 SUBSTITUTED
  OpFreeRef(__ref_1(1));
  return cv(0);
}
```

S1's `nrvo_collapse_tail_set` substituted the inner call's hidden-buffer arg position with `cv`.  The inner call writes directly into cv's slot — no intermediate store, no leak.

### Problem probe 02 (double-set, was Canvas×6/iter)

```loft
fn render_double(p: P) -> Canvas {
  cv = alloc_canvas(3, 3, p.tag);       // First Set
  cv = alloc_canvas(4, 5, p.tag + 1);   // Second Set (penultimate)
  cv
}
```

Pre-fix IR — S1 fires only on the SECOND Set; the FIRST Set's call uses `__ref_1`:

```
fn n_render_double(p:P, cv:Canvas) -> Canvas["cv"] {
  __ref_1(1):ref(Canvas) = null;
  [34] cv(0):ref(Canvas) = n_alloc_canvas(3, 3, p.tag, __ref_1(1));
                                                       ^^^^^^^^^^
                                       FIRST Set: __ref_1 (not cv)
  [35] cv(0):ref(Canvas) = n_alloc_canvas(4, 5, p.tag + 1, cv(0));
                                                           ^^^^^
                                       SECOND Set: S1 substituted (cv)
  OpFreeRefIfDistinct(__ref_1(1), cv(0));
  return cv(0);
}
```

### The divergence

Probe 02 leaks because:
1. First call passes `__ref_1` as hidden buf; alloc_canvas allocates store `S_a` at __ref_1's slot.
2. Set assigns `cv = __ref_1` — both alias `S_a`.
3. Second call (S1-substituted, buf = cv = `S_a`): alloc_canvas's `cv = Canvas{…}` invokes `OpDatabase` with `cv.store_nr == S_a` (REUSE path).  `clear()` calls `store.init()` — wipes `S_a`'s allocator metadata.  Fresh Canvas record + vector child claim happen in `S_a`.
4. The OLD record's `data` vector store is orphaned — `clear()` doesn't walk Reference-typed fields.

Native escapes because Rust's `Drop` recursively frees child stores when the slot is reassigned.

### The cross-iter slot dangling (Step 2 corruption mode)

Bytecode trace for probe 02 with Step 2 (`0x8000` source-free) active but no slot reset:

```
iter 0:
  OpDatabase var=24 (p)    db=#3@0,8       → p at #3
  OpDatabase var=12 (cv_a) db=#4@0,8       → cv_a at #4
  OpDatabase var=44 (mlb)  db=#2@0,8       → main_local_buffer at #2
  OpCopyRecord src=#2 dst=#4 free_src=true → wrap: copy + FREE #2
  scope-exit frees #4, #3

iter 1:
  OpDatabase var=24 (p)    db=#2@0,8       → p NOW at #2 (allocator reuse)
  OpDatabase var=12 (cv_a) db=#3@0,8
  OpDatabase var=44 (mlb)  db=#2@0,8       → mlb's slot still has stale #2
                                            → clear(#2) WIPES p's data
                                            → mlb and p now alias the SAME RECORD
```

The cross-iter dangling slot is the root cause: Step 2's `0x8000` free succeeds, but the placeholder's slot DbRef isn't reset.  Next iter the allocator reuses that store_nr for a different var; the placeholder's stale slot DbRef then clobbers it via `OpDatabase`'s `clear+claim`.

## What we know vs. don't

| Claim | Status |
|---|---|
| IR difference between probe 01 and 02 | ✅ `/tmp/bc_01.txt` and `/tmp/bc_02.txt` |
| S1 fires on second Set only in probe 02 | ✅ IR shows `__ref_1` vs `cv` arg |
| Slot allocation clean for Cluster II probes | ✅ `LOFT_LOG=slots:n_render` — no unallocated vars |
| Mechanism is RUNTIME, not parse/codegen | ✅ Slot trace clean → bug is in opcode execution |
| Child-store orphan in `OpDatabase` REUSE path | ✅ `src/database/allocation.rs:420` `clear()` calls `init()`; no Reference-field walk |
| Cross-iter dangling slot caused Step 2 corruption | ✅ Pinned via LOFT_TRACE_DB + LOFT_TRACE_CR (commit a957a365) |
| `OpFreeRef` idempotent for sentinel + freed stores | ✅ `free_named` early-returns on both (`src/database/allocation.rs:169, 175`) |

## Fix surface

| Option | Effort | Shipped? |
|---|---|---|
| (a) Extend S1 to consecutive `Set(cv, Call(_))` ops (commit `ff0b38d4`) | S | ✅ closed probes 02, 21 |
| (b) Step 2: refined `is_borrowed_view` + post-call free + sentinel reset (commit `db8fd532`) | M | ✅ closed probes 03, 07, 11, 25, 26 |
| (c) Narrow `is_hidden_buf_arg` to require S1 (commit `e4fca573`) | XS | ✅ closed probes 04, 28 |
| (d) Recursive child-store free in `OpDatabase` REUSE path | M | Not pursued — (a)+(b)+(c) closed every probe; reserve for future shapes |
| (e) Refcount-based ownership (`project_drop_store_refcount`) | L | Deferred — would subsume the whole class, but no live driver |

## Fix iterations

Three landed attempts.  Each corrected an assumption that the previous attempt's success would have hidden.

### Attempt 1 — extended S1 substitution (commit `ff0b38d4`)

Extend `nrvo_collapse_tail_set` to substitute backwards through CONSECUTIVE `Set(cv, Call(_))` ops.  Closes probes 02 and 21.  Insufficient for 03, 04, 07, 11, 25, 26, 28 — those break the consecutive walk via intervening stmts, struct-literal first Sets, conditional Sets in If branches, or explicit-return wrappers.

An earlier attempt to extend the substitution to ALL `Set(cv, Call(_))` in the body (including descent into If/Block/Return wrappers) regressed `tests/scripts/87-store-leaks.loft` (NaN result).  Rolled back to consecutive-only.

### Attempt 2 — Step 2: refined `is_borrowed_view` + post-call sentinel reset

Refining `is_borrowed_view` at `src/state/codegen.rs:1472, 2061` to require at least one VISIBLE-arg dep means hidden-only deps enable `0x8000` source-free at OpCopyRecord.  Combined with a post-call `OpInitRefSentinel` on the placeholder slot, this should close all remaining shapes.

First sub-attempt — `OpInitRefSentinel` only — closed probes 03/07/11/25/26 but REGRESSED `tests/leak_cases::leak_cases_interp` on `clean/local_var_return_shifted_var_nr`.  Root cause via `LOFT_STORES=log`: when `@P290`'s `protect_store_frees` blocks OpCopyRecord's source-free, the placeholder store is still live; the sentinel reset then orphans it.

Corrected sub-attempt — emit `OpVarRef(slot) → OpFreeRef → OpInitRefSentinel(slot)` (commit `db8fd532`).  `OpFreeRef` is idempotent (handles sentinel + already-freed), so the sequence is safe whether `0x8000` freed the source or `@P290` blocked it.  Closes 03/07/11/25/26 without regressing leak_cases.  Still left 04 + 28 leaking.

Also required: bytecode VM `OpDatabase` sentinel handling at `src/state/io.rs:723` (mirrors native runtime).  Without this, the next iter's `OpDatabase` on a sentinel slot would OOB into `allocations[u16::MAX]`.

### Attempt 3 — narrow `is_hidden_buf_arg` to require S1 substitution (commit `e4fca573`)

Probes 04 + 28 second Sets bypass the reassignment path because `is_hidden_buf_arg` at `src/state/codegen.rs:1406` was suppressing the pre-Set free under the wrong precondition.  The skip is correct when the call's hidden-buf arg IS `cv` (S1-substituted in-place reuse) but wrong when the hidden buf is a distinct `__ref_N` — then `cv`'s current store (from a preceding struct-literal init or default-init) becomes orphan when the reassignment writes the deep-copied result.

Single conjunct addition (`is_hidden_buf_arg = s1_substituted && …`) routes the non-S1 case through the reassignment path with proper free.

## Validation

- `cargo test --release --test leak_cases leak_cases_interp` — passes.
- All 9 Cluster II probes (02, 03, 04, 07, 11, 21, 25, 26, 28) — leak-free on `--interpret`.
- 50 V-class probe runs (`--interpret` + `--native`) — no regressions.
- Full `cargo test --release --no-fail-fast` suite — 0 failures.

## Why native escapes

Native's codegen lowers `Set(cv, expr)` to a Rust statement that uses ownership / `Drop` semantics: the new value's record is computed, the OLD value's record is dropped (Rust's destructor runs recursively, freeing child stores), the slot is updated.  Automatic — Rust's drop handles the child-store recursion.  The interpret backend has no equivalent recursive-drop mechanism; each `OpDatabase` REUSE updates the slot but doesn't recursively free.

## Historical: previous attempts (appendix)

Summaries of approaches tried during the investigation arc.  All retracted; kept here so future investigators don't re-explore the same dead ends.

- **Caller-side `is_borrowed_view` refinement alone (2026-05-28)** — closed probes 03/04/07/11/25/26 but REGRESSED 02/21/28 with corruption.  Cause: with extended S1, the canonical multi-Set callee returns the caller's hidden buffer; OpCopyRecord's src and dst share a store.  The runtime guard at `src/state/io.rs:1230` (added later as commit `172171ee`) makes same-store copy a no-op; this attempt happened before that guard, so `remove_claims` freed nested vec records BEFORE `copy_block` read them.
- **Defensive same-store `OpCopyRecord` no-op (commit `172171ee`, 2026-05-28)** — landed standalone as the hardening that the caller-side refinement attempt above had needed.  Still didn't close Cluster II on its own (the cross-iter slot dangling remained unaddressed).
- **Recursive child-store free in `OpDatabase` REUSE path** — proposed by an earlier investigation agent as fix path (d) above.  Not pursued because Attempts 1+2+3 closed every Cluster II probe without it.  Reserve for any future shape that doesn't fit `caller_hidden_buf` or S1.

## Tools added during this cluster's investigation

| Tool | Status | Used for |
|---|---|---|
| `LOFT_TRACE_DB=1` | New (`src/state/io.rs:717`) | Every `OpDatabase` call (var, type, current DbRef).  Pinned cross-iter slot dangling. |
| `LOFT_TRACE_CR=1` | New (`src/state/io.rs:1237`) | Every `OpCopyRecord` (src+dst with Canvas field reads BEFORE and AFTER copy).  Pinned same-store corruption mechanism. |
| `LOFT_TRACE_COPY=1` | Pre-existing | Native-side OpCopyRecord trace; companion to LOFT_TRACE_CR. |
| `LOFT_STORES=log` | Pre-existing | Per-store alloc/free trace.  Pinned the `local_var_return_shifted_var_nr` regression. |
