<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Cluster IV-Vec-nested-field-push — `field += inner_vec` for `vector<vector<X>>` fields

**Surfaced during PLAN52 cluster IV-Vec probing (probes 21, 36).**  The
"pre-existing vec-of-vec lookup bug" first noted in the README is in fact
a parser-lowering misroute that exists independently of `??`.  Brought back
in-plan 2026-05-30 per investigation-plan policy.

**Severity:**
- **Interpret silent corruption** — every variant of `field += inner_vec`
  produces wrong-length `o.lists` (flattens on primitives) or zeroed
  elements (structs).  No diagnostic.  Real-library impact: any code that
  builds a `vector<vector<X>>` STRUCT FIELD via `+=` ships wrong data.
- **Native silent corruption + secondary panic** — same symptom, plus
  exposes a secondary `copy_claims` panic at `database/allocation.rs:1190`
  when the inner element type is a struct (enum-dispatch out of bounds).

## Mechanism (verified by IR trace 2026-05-29)

`+=` on a **local var** of type `vector<vector<X>>` and `+=` on a
**struct field** of the same type take DIFFERENT lowering branches in
`src/parser/expressions.rs::parse_assign_op`:

### Local var (correct — probe 92 PASSes both backends)

Branch at line 957 (P188) fires:

```rust
// P188 — local-var collection `+= elem`
if op == "+="
    && var_nr != u16::MAX                                // ← LOCAL
    && matches!(f_type, Type::Vector(...) | ...)
{
    let elm_tp = f_type.content();
    if !elm_tp.is_unknown() && elm_tp.is_equal(&s_type) { // ← single-element intent
        let ls = self.new_record(... var_nr ...);         // ← correct: OpNewRecord/CopyRecord/FinishRecord
        ...
    }
}
```

IR emitted:

```
_elm_2 = OpNewRecord(outer, 66=vector<Inner>, 65535);
OpSetInt4(_elm_2, 0, 0);
OpCopyRecord(inner, _elm_2, 66);
OpFinishRecord(outer, _elm_2, 66, 65535);
```

### Struct field (broken — probes 91, 93, 94 FAIL both backends)

`var_nr == u16::MAX` so the P188 branch is skipped.  Execution falls
through to line 1370:

```rust
// `lhs += other_vec` where both sides are vectors: append all elements.
if op == "+="
    && let Type::Vector(elm_tp, _) = &f_type.clone()    // ← LHS is Vector
    && matches!(s_type, Type::Vector(_, _))             // ← RHS is ALSO Vector
{
    *code = Value::Insert(vec![self.cl("OpAppendVector", ...)]);  // ← CONCATENATE
    return Type::Void;
}
```

`OpAppendVector` calls `Database::vector_add` (`src/database/structures.rs:251`)
which reads `length_vector(o_db)` and appends THAT MANY elements
byte-copied from `o_db`'s data area.  The inner element type (`X`)'s
serialized layout overlays into `o.lists`'s element slots
(which expect `vector<X>` headers).  Result:

| Inner type `X` | Symptom |
|---|---|
| `integer` | `len(o.lists) == len(inner)` (flattened); reading `o.lists[0]` interprets first 8 bytes as a vector header → garbage length, garbage elements |
| `text` | Same flatten; first slot's bytes (a 4-byte string-table offset) read as 0 → `len(o.lists[0]) = 1`, `o.lists[0][0] = ""` |
| struct `Inner` | The element-size mismatch produces `len(o.lists) == 1` (struct payload happens to fit) but reading `o.lists[0]` reads zero-payload (`Inner.tag == 0`) and `len(a) == 0` |

The bug is shape-only: the parser branch at line 1370 incorrectly fires
for ALL `vector<V> += V` where V is itself a Vector, treating
single-element push as concatenation.

## Probes

Added 2026-05-30 to extend the probe matrix from probes 21/36:

| Probe | Shape | Cluster sub | --interpret (pre-fix) | --native (pre-fix) | --interpret (post-fix) | --native (post-fix) |
|---|---|---|---|---|---|---|
| 91 | `o.lists += inner` (struct-Inner) | misroute | FAIL `len(a)=0` | FAIL `len(a)=0` | **PASS** | FAIL alloc:1190 panic (second bug) |
| 92 | `outer += inner` (LOCAL var control) | reference | PASS | PASS | PASS | PASS |
| 93 | `o.lists += inner` (integer inner) | misroute | FAIL flat `len=3` | FAIL flat `len=3` | **PASS** | **PASS** |
| 94 | `o.lists += inner` (text inner) | misroute | FAIL flat `len=2` | FAIL flat `len=2` | **PASS** | **PASS** |
| 95 | `o.lists = [inner]` (field= literal) | second-bug | PASS | PANIC alloc:1190 | PASS | PANIC alloc:1190 |
| 96 | `Outer{lists:[inner]}` (ctor literal) | second-bug | PASS | PANIC alloc:1190 | PASS | PANIC alloc:1190 |
| 97 | 3-deep `vector<vector<vector<int>>>` | second-bug | PANIC alloc:1190 | PANIC alloc:1190 | PANIC alloc:1190 | PANIC alloc:1190 |

**Diagnostic shortcut:** diff probe 91 (FAIL) against probe 92 (PASS) —
the ONLY difference is local-var vs struct-field LHS.  Same RHS, same
types, same `+=`.  This pin-points the parser-side branch divergence.

## Fix (parser side) — STRICT RULE landed 2026-05-30

**Site:** `src/parser/expressions.rs::parse_assign_op` lines 957, 1370, 1389.

The fix landed as a STRICTER form than originally proposed.  Vector `+=`
is now unambiguous-by-construction:

1. **Push** requires the explicit `[elem]` form: `vec += [elem]`.
2. **Concat** requires exact type match: `vec += other_vec` only if
   `typeof(other_vec) == typeof(vec)`.
3. **Bare `vec += elem`** (without brackets) is a COMPILE ERROR.
4. **Type-mismatched concat** (`vec<int> += vec<u8>`) is a COMPILE ERROR.

The line-957 P188 branch (local-var push) no longer accepts `Type::Vector`
— only keyed collections (sorted/hash/index/spacial).  A new diagnostic
emits "vector `+= elem` is ambiguous; use `+= [elem]` to push one
element, or `+= other_vec` (typeof must match) to concatenate" when a
vector LHS receives bare element-typed RHS.

The line-1370 concat branch now requires `s_type.is_equal(f_type)` —
mismatched-type concat is rejected with "vector `+= other_vec` requires
equal types (X != Y)".

The line-1389 field-`+= elem` branch still exists but is unreachable for
vectors after the new diagnostic at line 967+ (always fires first for the
strict-rule violation).

### Why this is safe

The strict rule eliminates the ambiguity class entirely:

- `vec<int> += [42]` — explicit push. Allowed. ✅
- `vec<int> += [1, 2, 3]` — explicit concat-via-literal. Allowed. ✅
- `vec<int> += other_vec_int` — concat (typeof match). Allowed. ✅
- `vec<int> += 42` — bare push. **ERROR** (use `+= [42]`).
- `vec<int> += vec_of_u8` — type-mismatch concat. **ERROR**.
- `vec<vec<int>> += inner_vec_int` — bare push of element-type. **ERROR**
  (use `+= [inner_vec_int]`). Was previously the silent-corruption case.

The keyed-collection bare push (`hash += Entry{}`, `sorted += Score{}`,
`index += Item{}`) is UNAFFECTED — keyed collections have no concat
semantics, so the bare form is unambiguous and remains the idiom.

### Verification

Applied to `src/parser/expressions.rs` 2026-05-30.

- Probes 91, 93, 94 all PASS on interpret after the fix.
- Probes 93, 94 PASS on native too (primitive inner types).
- Probe 91 native, probes 95, 96 native, probe 97 both — still FAIL with
  the secondary `database/allocation.rs:1190` panic.  This is a separate
  bug in the `copy_claims` deep-copy path for `vector<vector<Struct>>`
  that PRE-DATES the parser fix (probes 95/96 already panicked pre-fix).
- Set H (`probes/run_set.sh H`) — all 11 baselines PASS unchanged.
- `cargo test --release --test issues` — 681/681 pass, no regression.

## Remaining work — probe 97 only

**LANDED 2026-05-30 (commit d98c32b)**: The field-content `db.vector(...)`
emission for nested-vector fields was rewritten as a chained
`{ let _v0 = db.vector(<innermost>); let _v1 = db.vector(_v0); ... }`
expression that handles any nesting depth.  Closes probes 91, 93, 94,
95, 96 on both backends.

**Probe 97 (3-deep `vector<vector<vector<integer>>>`) SPUN OFF as
[@P384](../../../PROBLEMS.md#open-issues--quick-reference) 2026-05-30** —
distinct sub-bug: `OpCopyRecord(..., LITERAL_TYPE_ID)` is emitted with a
literal type id the PARSER computed via its database, but the RUNTIME
database may have a different mapping at that slot (because intervening
`db.vector(other)` calls in default-lib initialization shift slot
assignments).  Probe 91/95/96 happen to coincide because the parser's
slot 66 also lands at runtime slot 66 for `vector<Inner>`; probe 97's
3-deep case shifts beyond that coincidence.

Probe 97 is no longer in PLAN52 scope.  Tracked under @P384 with two
candidate fix paths (symbolic type-id refs in op-call codegen OR
parser/runtime type-table alignment refactor); both architectural,
1-2 weeks each.  Workaround in the meantime: keep nesting ≤2 deep, or
assign each layer to a separate local var so the working
`OpNewRecord/OpCopyRecord/OpFinishRecord` local-var path fires.

### Earlier symptom (now closed)

After the parser fix, the parser correctly emits
`OpNewRecord/OpCopyRecord/OpFinishRecord` for `field += inner_vec`.  But
when `inner_vec` is `vector<Struct>` (probes 91 native, 95, 96, 97), the
`OpCopyRecord` deep-copy of the inner vec's elements panics in
`src/database/allocation.rs:1190` (Parts::Enum dispatch out-of-bounds).

Symptom: `index out of bounds: the len is 7 but the index is 41` (probes
91/95/96) or `index is 16` (probe 97 3-deep).  This is `copy_claims`
reading a byte from a non-Enum position but routing through the Enum
arm — the type id passed to `copy_claims` is mismatched.

This is a separate code path (database/allocation, not parser/codegen)
and its own fix surface.  Tracking as the next step within this cluster
since the probe-matrix gate doesn't close until ALL of 91-97 PASS on
both backends.

Mechanism hypothesis (unverified): the type id passed to `OpCopyRecord`
for nested vector deep-copy is the OUTER element type
(`vector<Inner> = type 66`), but `copy_claims`'s recursive walk over
that type's contents inadvertently re-enters with the parent struct's
type id at some level, hitting Inner's struct-tag byte at the wrong
offset and interpreting it as an enum tag.

### Investigation step (next)

Add `LOFT_LOG=copy_check` (the @P317 deep-copy debug, see
`src/database/allocation.rs:1200`) under probe 91 native to localise
which `copy_claims` call passes the wrong type id.  Likely fix surface:
`src/database/structures.rs::vector_add` (lines 348-362) deep-copy
loop, OR `Stores::finish_type`'s Vector→Array promotion logic that
sets up the linked-content type tables.

## Status

| | Status |
|---|---|
| Mechanism understood? | ✅ Verified via IR trace (parser branch divergence at line 1370) |
| Fix surface identified? | ✅ Parser: `src/parser/expressions.rs:1370` one-line guard |
| Probe-set coverage | ✅ Probes 91-97 (7 probes covering primitive/text/struct inner + control + workaround variants) |
| Fix applied? | ✅ Parser fix in working tree 2026-05-30 (interp side fully closed; native primitive cases closed) |
| Secondary `copy_claims` panic | 🟡 Still open — separate fix surface in `database/allocation.rs` / `database/structures.rs` |
| Ready to close cluster? | ❌ Pending secondary panic fix (probes 91 native, 95-97) |

## See also

- [`cluster-IV-heap-typed.md`](cluster-IV-heap-typed.md) — sibling cluster
  documenting `??` value-block heap-DbRef predicate emit (different
  surface, complementary fix).
- [`probes/21-vector-coalesce.loft`](probes/21-vector-coalesce.loft) —
  original probe that surfaced this bug while testing `??` over
  `vector<vector<X>>`.
- [`probes/36-iter-over-vec-coalesce.loft`](probes/36-iter-over-vec-coalesce.loft) —
  iteration-consumer companion of probe 21.
- `src/parser/expressions.rs::parse_assign_op` lines 957 (P188 local-var
  branch — the working reference) + 1370 (broken concat-misroute) + 1389
  (field-`+= elem` correct lowering — now reachable after the fix).
- `src/database/structures.rs::vector_add` — the concatenate op
  (called by `OpAppendVector`) which was wrongly used as if it were a
  single-element push.
- `src/database/allocation.rs::copy_claims` line 1188 — the panic site
  for the secondary bug remaining after the parser fix.
