# Phase 00 — Survey

**Status:** OPEN

**Kind:** Infrastructure (no code change; produces written
findings that drive phase 02's exact edit)

**Time budget:** 0.5-1 day.

## Goal

Confirm three preliminary findings (from code inspection during
plan authoring) and surface anything they missed:

1. **The OpFreeRef runtime is already safe.**  Inspected
   `src/codegen_runtime.rs:98-122` shows early-returns on
   `store_nr == u16::MAX` (line 100) and out-of-bounds store_nr
   (line 103), plus automatic file-handle close via
   `stores.files[i] = None` (line 117).  Phase 01 collapses to
   "verify + add tests."
2. **File-handle Drop is already in place.**
   `src/database/mod.rs:162` declares `pub files: Vec<Option<std::fs::File>>`.
   Setting a slot to `None` drops the previous `Some(File)`,
   which closes the OS handle.  Phase 03 collapses to "verify +
   document the contract."
3. **The bug lives in the scope-exit gating, not in emission
   shape.**  `src/scopes.rs:1053` decides per-var whether to
   emit OpFreeRef:
   ```rust
   let emit = (dep.is_empty() || is_work_ref) && !in_ret && !function.is_skip_free(v);
   ```
   For P203's file ref, one of the three negative conditions
   evaluates true (likely `dep.is_empty() == false` because the
   dep-tracker incorrectly attaches a dep to the file ref).
   Phase 02's loosening of this gate is the real fix.

## Detailed steps with validation

### Step 0.1 — Verify OpFreeRef runtime safety

**Action**: read `src/codegen_runtime.rs:98-122` and confirm:

```bash
sed -n '98,122p' src/codegen_runtime.rs
```

Confirm three early-return paths exist:
- `db.store_nr == u16::MAX` → return
- `db.store_nr as usize >= stores.allocations.len()` → return
- File-type detection via `stores.names.get("File")` →
  `stores.files[file_ref] = None` (Drop closes OS handle)

If any of these is missing or different from preliminary finding,
phase 01 expands to fill the gap.

**Validation**: written confirmation in "Survey findings" below.

### Step 0.2 — Verify file-handle Drop

**Action**:
```bash
grep -n "files:" src/database/mod.rs
```

Confirm `files: Vec<Option<std::fs::File>>` (line 162) — Drop
fires on slot replacement.

Confirm: when `OpFreeRef` runs `stores.files[i] = None`, the
previous `Some(File)` drops, OS handle closes.

**Validation**: written confirmation.

### Step 0.3 — Trace why OpFreeRef doesn't fire for P203

**Action**: instrument `src/scopes.rs:1054-1063` with a
`scope_debug` print for the file ref `f` in `repro_p203.loft`:

```bash
cd /home/ubuntu/loft
LOFT_LOG=variables cargo run --bin loft --release --quiet -- \
    --native-emit /tmp/p203.rs tests/scripts/repro_p203.loft 2>&1 | \
    grep -E "scope_debug.*'f'|var=.*name=f"
```

Expected output: a `[scope_debug] NOT freeing 'f' …` line that
shows which condition gated the emission:
```
[scope_debug] NOT freeing 'f' (var=N, scope=S, to_scope=T):
    dep_empty=false in_ret=? skip_free=?
```

The gate-state print tells phase 02 exactly which condition to
loosen.  Possibilities:
- `dep_empty=false` → dep tracker attaches non-empty dep to file
  ref.  Phase 02: loosen the `dep.is_empty()` gate for ref-shaped
  locals.
- `in_ret=true` (incorrectly) → return-tracking misclassifies file
  ref.  Phase 02: refine the `in_ret` predicate.
- `skip_free=true` → `is_skip_free` flag is incorrectly set.
  Phase 02: investigate where the flag is set.

**Validation**: a written report of which condition fires for the
P203 reproducer.  This is the **single most important survey
output** — it determines phase 02's exact edit.

### Step 0.4 — Map dep-tracking consumers

**Action**: find all readers of `dep` (the dep list on
`Type::Reference` etc.):

```bash
rg -n "\.depend\(\)|\.dep\b|dep\.is_empty|dep_has_var" \
    src/scopes.rs src/parser/ src/generation/ \
    --type rust > /tmp/p10-survey/dep-readers.txt
wc -l /tmp/p10-survey/dep-readers.txt
```

Classify each reader by purpose: cleanup gating / aliasing
analysis / closure capture / parallel isolation / other.

**Validation**: a written table.  If most readers are cleanup
gating (likely), phase 02 can loosen the gate confidently.  If
many readers are aliasing/closure (less likely), be careful that
phase 02 doesn't break their assumptions.

### Step 0.5 — Identify deliberate suppression cases

**Action**: from existing `is_skip_free` flag, find every place
that sets it:

```bash
rg -n "is_skip_free|set_skip_free|mark_skip_free" \
    src/parser/ src/scopes.rs --type rust > /tmp/p10-survey/skip-free.txt
```

Each entry is a deliberate "do not emit OpFreeRef for this var"
case.  Phase 02 keeps these by leaving `is_skip_free` in the
gate.

**Validation**: written list with rationale per entry.

### Step 0.6 — Estimate phase 02 risk

**Action**: based on findings, classify:
- Risk **low** if step 0.3 surfaces `dep_empty=false` and only a
  small number (≤3) of dep-cleanup readers exist outside scopes.rs.
- Risk **medium** if many dep readers exist or step 0.3 surfaces
  multiple gate conditions failing for the same var.
- Risk **high** if dep tracking is tightly coupled to other
  systems (closure capture, parallel isolation) such that
  loosening the gate would cascade.

**Validation**: documented risk level with rationale.

## Acceptance for phase 00

The four catalogue files exist:
- `/tmp/p10-survey/scope-debug-p203.txt` (step 0.3 output)
- `/tmp/p10-survey/dep-readers.txt`
- `/tmp/p10-survey/skip-free.txt`

This file's "Survey findings" populated.

## Survey findings

### OpFreeRef runtime safety (step 0.1)

_(populate; expected: confirms preliminary finding)_

### File-handle Drop (step 0.2)

_(populate; expected: confirms preliminary finding)_

### Why OpFreeRef doesn't fire for P203 file ref (step 0.3)

_(populate; this drives phase 02's exact edit)_

### Dep-tracking consumers (step 0.4)

_(populate)_

### Deliberate skip_free cases (step 0.5)

_(populate)_

### Phase 02 risk estimate (step 0.6)

_(populate)_

## Problems encountered

_(append per problem)_

## Implementation notes

_(append per non-obvious decision)_
