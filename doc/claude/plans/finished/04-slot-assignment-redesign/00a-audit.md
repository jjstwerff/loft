# Phase 0 Audit: Slot Allocator Branches and Dispatch Points

**Generated:** 2026-04-22  
**Scope:** `src/variables/slots.rs`, `src/variables/mod.rs`, `src/variables/intervals.rs`, `src/scopes.rs`, `src/state/codegen.rs`, `src/stack.rs`

---

## Executive Summary

The slot allocator enforces a **two-zone layout** divided by size (≤8B vs >8B, documented inconsistently as ≤16B vs >16B). Zone 1 (small) is pre-claimed per Block with greedy interval colouring; Zone 2 (large) uses sequential IR-walk placement. Loops skip Zone 1 entirely (`var_size=0`), placing all variables sequentially. The algorithm contains **4 distinct dimension branches**: variable size, scope kind (Block vs Loop), set cardinality (block-return vs inner-has-pre-assignments), and Text type (excluded from certain optimizations). Phase 1 must eliminate all dispatch by integrating these into a uniform placement formula.

---

## Table 1: Size-Based Branches

| File:Line | Snippet | Classification | Notes |
|-----------|---------|-----------------|-------|
| `slots.rs:92` | `s > 0 && s <= 8` | placement-dispatch | Zone 1 filter: only small vars eligible for greedy colouring in Blocks |
| `slots.rs:153` | `if v_size != j_size` | placement-dispatch | Greedy colouring: dead slot reuse only when sizes match (avoids displacement) |
| `slots.rs:163` | `zone1_hwm = zone1_hwm.max(candidate + v_size)` | formula | Tracks Zone 1 high-water mark; not a branch |
| `slots.rs:216` | `if v_size > 0 && v_size <= 8 && function.is_loop_scope(scope)` | placement-dispatch | Loop small vars: place sequentially at TOS, not in Zone 1 coloring |
| `slots.rs:230` | `if v_size > 8` | placement-dispatch | Zone 2 entry: large vars use `place_zone2()`, small vars skip this block |
| `slots.rs:312-313` | `let v_size = size(...); let reuse_slot = find_reusable_zone2_slot(...)` | formula | Computes Zone 2 slot; reads size for reuse check but doesn't dispatch placement itself |
| `slots.rs:363` | `size(&v.type_def, &Context::Variable) > 0` | placement-dispatch | Orphan filter: skip zero-size vars (arguments, type-only) |
| `slots.rs:371` | `let v_size = size(...); ... candidate += 1` | formula | Orphan placement loop uses size for end-point check, not dispatch |
| `slots.rs:387` | `size(&jv.type_def, &Context::Variable)` | formula | Orphan conflict check uses size to compute slot ranges |
| `slots.rs:422-423` | `let j_size = size(...); if j_size != v_size || std::mem::discriminant(...) != v_disc` | placement-dispatch | Zone 2 reuse: only Text-to-Text, only matching size |
| `validate.rs:142` | `if left_size == 0 { continue; }` | formula | Overlap validator skips zero-size vars (not placement) |
| `validate.rs:151` | `if right_size == 0 { continue; }` | formula | Overlap validator skips zero-size vars (not placement) |
| `intervals.rs:124` | `if var_size > 0 && v.first_def < seq_start && v.last_use >= seq_start` | formula | Loop-carry detection uses size as guard; doesn't affect placement dispatch |

**Subtotal:** 5 hard placement-dispatch branches, 8 formula-only (read size, don't dispatch).

---

## Table 2: Scope-Kind Branches

| File:Line | Snippet | Classification | Notes |
|-----------|---------|-----------------|-------|
| `slots.rs:73` | `let is_loop = matches!(block_val, Value::Loop(_));` | placement-dispatch | Determines whether to run Zone 1 greedy colouring |
| `slots.rs:74-76` | `let bl_scope = match block_val { Value::Block(bl) \| Value::Loop(bl) => bl.scope, _ => return, }` | placement-dispatch | Extracts scope from either Block or Loop; returns early if neither |
| `slots.rs:82-83` | `let mut small_vars: Vec<usize> = if is_loop { Vec::new() } else { ... }` | placement-dispatch | Loops empty Zone 1 vars list; Blocks populate it for greedy colouring |
| `slots.rs:176-177` | `if let Value::Block(bl) \| Value::Loop(bl) = block_val { bl.var_size = if is_loop { 0 } else { zone1_size }; }` | placement-dispatch | Sets Block's pre-claimed size; Loops always get 0 (no OpReserveFrame) |
| `slots.rs:188` | `let operators = match block_val { Value::Block(bl) \| Value::Loop(bl) => &mut bl.operators, _ => return, };` | placement-dispatch | Extracts operators from either; returns early otherwise |
| `slots.rs:235-236` | `if matches!(inner.as_ref(), Value::Block(_)) && !matches!(function.variables[v].type_def, Type::Text(_))` | placement-dispatch | Block-return pattern: non-Text types use frame-sharing; Text uses separate frame |
| `slots.rs:264` | `Value::Block(_) => { let child_base = *tos; process_scope(...); }` | placement-dispatch | Block child scope: process_scope recursively; does not advance TOS |
| `slots.rs:269` | `Value::Loop(_) => { let child_base = *tos; process_scope(...); }` | placement-dispatch | Loop child scope: process_scope recursively; TOS may advance if loop vars are large |
| `slots.rs:276` | `if matches!(then_val.as_ref(), Value::Block(_))` | placement-dispatch | If-then: Block branches call process_scope; non-Block branches walk recursively |
| `slots.rs:282` | `if matches!(else_val.as_ref(), Value::Block(_))` | placement-dispatch | If-else: Block branches call process_scope; non-Block branches walk recursively |
| `slots.rs:337` | `Value::Block(_) \| Value::Loop(_) => true` | placement-dispatch | `inner_has_pre_assignments`: Block/Loop contain assignments before the outer Set's target |
| `validate.rs:58` | `Value::Block(bl) \| Value::Loop(bl) => { ... }` | formula | Scope-parent builder: recurses both; not affecting allocation |

**Subtotal:** 11 scope-kind placement-dispatch branches.

---

## Table 3: `place_orphaned_vars` Call-Site Audit

**Single call site:** `slots.rs:58`

```rust
place_orphaned_vars(function, local_start);
```

**When orphans exist:**

1. **Scope has no Block/Loop node in IR tree** — Variables defined in a scope whose IR path never encounters a Block or Loop node are missed by `process_scope` and `place_large_and_recurse`.

2. **Common IR shapes causing orphans:**
   - **If-condition without Block:** `Value::If(condition, Block(...), Block(...))` — the condition is an expression (not wrapped in Block), so variables defined in it become orphans. Example: `if count() > 0 { ... }` where `count()` is a Call that returns intermediate variables.
   - **Call arguments:** `Value::Call(fn, [arg1, arg2])` — arguments are expressions that may define temporaries. The arguments themselves are placed via `place_large_and_recurse`, but orphans can occur if those arguments are nested Inside further expressions with no Block wrapper.
   - **Formatted strings (P135):** `Value::Insert([Set(__lift_N, expr), ...])` — the Insert preamble Sets may define variables in a scope that has no Block node, leaving them orphaned.

3. **`local_start` value:** Passed as the floor for candidate-slot search (line 379). Protects orphans from overlapping argument/return-address slots at [0, local_start). Arguments have `stack_pos == u16::MAX` during `assign_slots`, so the per-variable conflict check (line 382–389) can't see them; `local_start` is the only protection.

4. **Variables ending up as orphans:** Any non-argument variable with:
   - `first_def != u32::MAX` (was defined)
   - `stack_pos == u16::MAX` (not assigned by the main walk)
   - `size > 0` (has runtime storage)
   
   Sorted by `first_def` so earlier-defined vars get lower slots, matching codegen's evaluation order.

5. **Why orphans exist:** The IR tree walk (`process_scope` → `place_large_and_recurse`) only encounters variables in Set nodes and child scopes. Variables defined in naked expressions (If conditions, Call args, Insert preambles) that lack an enclosing Block node are invisible to the main walk and must be placed afterwards via linear conflict scanning.

6. **Tests documenting orphan placement:**
   - `insert_preamble_sets_placed_before_target` (approx. line 1550): P135 lift vars are orphans until `place_orphaned_vars` assigns them slots above `local_start`.
   - `parent_var_set_inside_child_scope_operators` (approx. line 1470): A parent-scope variable assigned inside a child scope's operators is orphaned until collected.

---

## Table 4: Zone-1 / Zone-2 Split Callers

| File:Line | Snippet | Boundary / Decision | Reconciliation |
|-----------|---------|-------------------|-----------------|
| `slots.rs:13` | Doc header: `≤16-byte types (int, float, bool, char, DbRef)` | Stated as ≤16B | **INCONSISTENCY:** Actual code uses `s <= 8` (lines 92, 216) |
| `slots.rs:16` | Doc header: `>16-byte types (text=24B String, tuples, etc.)` | Stated as >16B | **INCONSISTENCY:** Actual code uses `v_size > 8` (line 230) |
| `slots.rs:92` | `s > 0 && s <= 8` | **8-byte boundary** | Zone 1 filter for greedy colouring |
| `slots.rs:216` | `if v_size > 0 && v_size <= 8 && function.is_loop_scope(scope)` | **8-byte boundary** | Loop small-var sequential placement |
| `slots.rs:230` | `if v_size > 8` | **8-byte boundary** | Zone 2 entry for non-loop large vars |
| `codegen.rs:1038` | `stack.add_op("OpReserveFrame", self);` | Emitted when `block.var_size > 0` (line 1896) | Block pre-claims its Zone 1 size; Loops never emit (var_size=0) |
| `stack.rs:63` | `Value::Text(_) => size_of::<&str>() as u16,` | Text in arguments: 8B pointer | Formula-only; not affecting zone split |
| `mod.rs:1258` | `Type::Text(_) if context == &Context::Variable => size_of::<String>() as u16,` | Text in variables: 24B String | Zone 2 (> 8B) |

**Key finding:** The **Zone 1 / Zone 2 split is at 8 bytes**, not 16 bytes as documented. Comment header (`slots.rs:13–16`) states ≤16B / >16B but code enforces ≤8 / >8. **Phase 1 must reconcile this discrepancy.**

---

## Algorithm Summary (Code-Observed Behavior)

**Input:** Live-interval data from `compute_intervals` (first_def, last_use per variable) and IR tree structure.

**Process:**

1. **Zone 1 (Blocks only):**
   - Filter variables: `scope == block_scope && size ∈ [1, 8] && first_def != u32::MAX`.
   - Sort by first_def (definition order).
   - For each variable, use greedy interval colouring: find the lowest slot [candidate, candidate + size) that has no spatial+temporal overlap with already-placed Zone 1 vars in the same scope.
   - Reuse rule: dead slot (last_use < first_def of new var) is reusable only if sizes match exactly (avoids displacement errors).
   - Track zone1_hwm (high-water mark).

2. **Zone 2 (IR walk, Blocks and Loops):**
   - Walk IR depth-first, encountering Set nodes and child scopes.
   - For each Set(v, inner) at scope level:
     - **Loop small vars (size ∈ [1, 8]):** Place sequentially at TOS, then recurse inner.
     - **Block large vars (size > 8):** Branch on inner structure:
       - **Block-return pattern** (`Set(v, Block(...))`, non-Text): Place v at TOS, then `process_scope` on child (child starts at v's slot, frame sharing).
       - **Inner has pre-assignments** (`inner_has_pre_assignments(inner)`, non-Text): Recurse inner first (to assign its child scopes), then place v.
       - **Otherwise:** Place v at TOS, then recurse inner.
     - On every placement, call `place_zone2(v, scope, tos)`: try reusable dead slot (Text-to-Text, same size), else allocate fresh at TOS and advance TOS.
   - Handle Block/Loop children: `process_scope` recursively.
   - Handle If branches: process Block children via `process_scope`; non-Block children via `place_large_and_recurse`.

3. **Orphan placement (post-walk):**
   - Collect all variables still at `stack_pos == u16::MAX` (first_def != u32::MAX, size > 0).
   - Sort by first_def.
   - For each orphan, find lowest slot [candidate, candidate + size) starting from `local_start` with no spatial+temporal conflict against already-placed vars.

**Output:** Every variable has `stack_pos` assigned, pre-claimed frame sizes recorded in Block nodes (`var_size`).

---

## SLOTS.md Pattern Mapping

| Pattern (SLOTS.md line) | Actual Rust Implementation |
|-------------------------|---------------------------|
| 126: Many parent refs + child loop index | `slots.rs:216` (loop small vars sequential) + `slots.rs:269` (loop child scope) |
| 127: Call with Block arg (coroutine) | `slots.rs:294–297` (Call args walk), `slots.rs:235–240` (block-return) |
| 128: Insert preamble (P135 lift) | `slots.rs:289–292` (Insert ops walk), `slots.rs:354–403` (orphan collection) |
| 129: Sequential lifted calls | `slots.rs:249–251` (inner_has_pre_assignments branches) |
| 130: Parent var Set inside child scope | `slots.rs:256–260` (scope mismatch logging), `slots.rs:354` (orphans) |
| 131: Text block-return | `slots.rs:235–236` (Text excluded from block-return frame sharing) |
| 132: Sibling scope reuse | `slots.rs:264–287` (If branches, each calls process_scope) |
| 133: Comprehension then literal | `slots.rs:294–297` (Call args), `slots.rs:230–255` (large var placement) |
| 134: Sorted range comprehension | `slots.rs:289–292` (Insert preamble) |
| 135: Par loop with inner for | `slots.rs:269–271` (nested loops), `slots.rs:216` (inner loop sequential) |

---

## Critical Inconsistencies for Phase 1

1. **Size boundary mismatch:** Documentation says ≤16B / >16B; code enforces ≤8B / >8B. **Decision required:** which boundary applies to Text (24B String in Variable context)?

2. **Block-return Text exception:** Text variables assigned to Block results bypass frame-sharing (line 235–236). This is the only type-based exception in Zone 2. **Phase 1 must decide** if Text deserves special treatment or if the boundary should subsume it.

3. **Orphan placement vs. greedy colouring:** Orphans use linear search (O(n²) conflict scanning); Zone 1 uses greedy colouring (O(n) per var). **Phase 1 must unify** the reuse algorithm.

4. **Reuse guards:** Zone 1 requires size match; Zone 2 (Text-only) requires size + type discriminant match. **Phase 1 must clarify** whether dead-slot reuse should be type-sensitive or just size-sensitive.

---

## Dispatch Points Requiring Subsumption

- **Size dispatch (5 points):** `s <= 8` checks in Zone 1 filter, loop sequential placement, Zone 2 entry, orphan loop guard, reuse guards.
- **Scope-kind dispatch (11 points):** Block vs Loop in Zone 1 eligibility, var_size setting, child recursion, If branch handling, inner_has_pre_assignments checks.
- **Text dispatch (1 point):** Frame-sharing exception for non-Text block returns.
- **Set cardinality dispatch (3 points):** Block-return pattern, inner_has_pre_assignments, simple inner placement.

**Total:** 20 branch points. A "uniform placement" algorithm must eliminate all four dimension branches by integrating size, scope kind, text type, and set cardinality into a single deterministic formula.

