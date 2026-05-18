# Phase 00 — Characterise the gate

**Status:** OPEN — not scheduled.

**Kind:** Survey (no code change; written reference for whoever
picks up phase 01 later).

## Goal

Document, in one place, what `src/scopes.rs:1053`'s cleanup gate
does, what its conditions mean, and what runtime/structural
invariants exist around it.  Save phase 01 from re-discovering
this when it eventually lands.

## The gate, today

`src/scopes.rs:1001-1112` is the heap-ref scope-exit emission
block.  Line 1053 decides per-var whether to emit OpFreeRef:

```rust
let emit = (dep.is_empty() || is_work_ref) && !in_ret && !function.is_skip_free(v);
```

Three positive conditions:

| Condition | Purpose | Source of truth |
|---|---|---|
| `dep.is_empty()` | The var owns its value (no other ref aliases it) | Dep-tracker analysis (parser side) |
| `is_work_ref` | The var is a `__ref_*` / `__rref_*` work-ref | Name-prefix check |
| `!in_ret` | The var doesn't participate in the function return | Computed from return-type deps |
| `!function.is_skip_free(v)` | The var hasn't been explicitly marked skip | Set elsewhere (e.g., `clean_work_refs`) |

The first two are alternatives joined by `||`; the rest are AND.

## What each condition guards against

### `dep.is_empty()`
Without this gate, OpFreeRef would fire on vars that share a record
with another live var.  The shared record's ref-count protects the
runtime from corruption (already-freed slots no-op), so the gate is
an optimisation, not a correctness requirement.

### `is_work_ref`
Work-refs are parser-introduced placeholders (`__ref_N`) used to
thread call results through expressions.  They typically don't have
empty deps (the dep-tracker attaches them to their consuming user
var) but still need cleanup.  The `||` lets them through when
`dep.is_empty()` doesn't.

### `!in_ret`
A var that's part of the function's return value MUST NOT be freed
at scope exit — the caller still owns it.  Detected by checking
whether the var appears in the result type's dep list or the
function's declared return type.

### `!is_skip_free(v)`
Explicit override: the parser marks vars whose cleanup is handled
elsewhere (e.g., callee takes ownership).  See
`src/variables/mod.rs::clean_work_refs` for the bulk-marking case.

## Confirmed runtime invariants

These are the safety properties phase 01 would rely on if the gate
loosens.  Already verified:

### OpFreeRef early-returns (`src/codegen_runtime.rs:100-104`)

```rust
if db.store_nr == u16::MAX {
    return;
}
if (db.store_nr as usize) >= stores.allocations.len() {
    return;
}
```

OpFreeRef is safe to call on already-freed slots and bogus
store_nrs.  Phase 01's "more OpFreeRef calls" cost is bounded by
this fast path.

### File handle Drop (`src/database/mod.rs:162`)

```rust
pub files: Vec<Option<std::fs::File>>,
```

When `OpFreeRef` runs `stores.files[file_ref] = None` (line 117),
the previous `Some(File)` drops, flushing + closing the OS handle.

### Reference counting

`OpFreeRef` only closes the file handle when `ref_count <= 1` —
multiple refs to the same record are handled correctly.

## Suppression catalogue

Spot-check via `rg "skip_free"` (run when phase 01 starts; cache
the list here):

| Site | Why suppressed |
|---|---|
| `src/variables/mod.rs::clean_work_refs:1111` | Bulk-mark `__ref_N` past parser's "no longer needed" point.  Work-refs are transient parser scaffolding whose contents transfer to consuming user-vars. |
| _(populate further sites when phase 01 surveys)_ | _(populate)_ |

## Dep-tracking consumers (cleanup vs other)

Phase 01 will need to confirm which non-cleanup consumers rely on
dep state.  Skipped here — fill in when phase 01 lands.

Search command:
```bash
rg -n "\.depend\(\)|\.dep\b|dep\.is_empty|dep_has_var" \
    src/scopes.rs src/parser/ src/generation/ --type rust
```

## Historical context — @P203 diagnostic

Plan 10 was originally framed as a @P203 fix.  The first phase 00
(2026-05-02) ran a strace-driven diagnostic that:

1. **Confirmed** OpFreeRef runtime safety (early returns +
   file-close path) — listed above.
2. **Confirmed** file-handle Drop semantics
   (`Vec<Option<File>>` drops on slot replacement).
3. **Refuted** the premise that OpFreeRef-doesn't-fire causes
   @P203.  Strace showed:
   ```
   openat("repro_p203.txt", O_WRONLY|O_CREAT|O_TRUNC, 0666) = 3
   write(3, "x", 1) = 1
   close(3) = 0                          ← OpFreeRef closed correctly
   unlink("repro_p203.txt") = 0          ← n_delete() succeeded (1st call)
   write(2, "thread 'main' panicked …")  ← assertion panicked (2nd call returned NotFound)
   ```
4. **Identified @P203's actual root cause**: template double-
   substitution at `default/01_code.loft:705`.  The
   `OpConvIntFromEnum` template substitutes `@v1` twice; for
   `delete(…) == FileResult.Ok` this calls `n_delete()` twice
   (deletes file → file gone → returns NotFound).

The discovery is recorded in PROBLEMS.md under @P203's fix path.
The strace trace + runtime-safety confirmation remain useful
evidence for whoever picks up plan 10 later — they prove the
infrastructure phase 01 builds on is solid.

## What's NOT in this characterisation

- A risk estimate for phase 01 — unknown until phase 01 actually
  starts and audits suppression cases under the loosened gate.
- A LOC delta projection — the simplification is small (~3 lines)
  but the suppression-list audit could surface new cases that
  add LOC elsewhere.
- A concrete trigger to start phase 01 — see the README's
  "Triggers to revisit" section.

## Acceptance for phase 00

This file exists and is populated.  No code change.  Phase 01
starts (when it does) by re-confirming this characterisation
hasn't drifted (the gate condition might evolve under unrelated
work).

## Implementation notes

_(future sessions: append per non-obvious decision encountered
when phase 01 starts)_
