
// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

# String Buffer Allocation and Optimization Opportunities

## Text type duality

Loft has two runtime representations for text:

| Type | Size | Heap? | Where used |
|---|---|---|---|
| `Str` | 16 bytes (ptr + len + pad) | No — borrows existing buffer | Arguments, temporaries on eval stack |
| `String` | 24 bytes (ptr + capacity + len) | Yes — owns heap buffer | Local variables, work texts |

The split is the primary optimization: text arguments are zero-copy
references into the caller's (or constant pool's) memory.

---

## Allocation lifecycle of a local text variable

```
OpText          →  24B String written to stack, zero heap (String::new())
OpAppendText    →  first append allocates heap buffer
OpClearText     →  .clear() — content gone, heap buffer preserved
OpAppendText    →  reuses existing buffer if it fits
OpFreeText      →  .shrink_to(0) — deallocates heap buffer
```

Key insight: `String::new()` is free (no heap allocation).  The real
cost is the **first `OpAppendText`** which triggers a heap allocation.
Subsequent reassignments via `OpClearText` + `OpAppendText` often
reuse the existing buffer.

---

## Where copies actually happen

| Situation | What happens | Heap alloc? |
|---|---|---|
| Text argument passing | 16B Str reference pushed | **No** |
| `OpVarText` (read local) | Create 16B Str view of 24B String | **No** |
| `OpArgText` (read param) | Read existing 16B Str | **No** |
| **`x = "hello"` (first)** | OpText + OpAppendText | **Yes** — one alloc |
| **`x = y` (text copy)** | OpText + OpVarText + OpAppendText | **Yes** — copy into new buffer |
| **`x = y + z` (concat)** | Work text + 2× OpAppendText | **Yes** — work text buffer grows |
| `x = func()` (text return) | Destination passing via RefVar(Text) | **No extra** — writes into x directly |
| `x = "new"` (reassign) | OpClearText + OpAppendText | **Usually no** — reuses buffer |
| Work text reuse | OpClearText | **No** — keeps capacity |

### Destination passing (already optimized)

Text-returning functions use `RefVar(Text)`: the caller's String
buffer is passed as an implicit parameter, and the callee writes
directly into it.  No intermediate copy.

```
fn greet(name: text) -> text {
  "hello " + name       // writes directly into caller's buffer
}
result = greet("world"); // result's String IS the buffer
```

This is implemented in `codegen.rs:gen_text_dest_call` (~line 1858)
and `text_return()` in `control.rs`.

---

## Current efficiency assessment

The design is already quite efficient:

1. **Arguments**: Zero-copy Str references — best possible.
2. **Work texts**: Allocated once per function, reused across
   statements.  `.clear()` preserves capacity.
3. **Destination passing**: Text-returning functions avoid
   intermediate buffers entirely.
4. **Reassignment**: `.clear()` + append reuses the heap buffer.
5. **String::new()**: Zero-cost until first content — no
   speculative allocation.

The remaining overhead is **one heap allocation per mutable text
variable** on first content assignment.  This is inherent to the
owned-buffer design.

---

## Optimization opportunities

### O-S1. `String::clone()` for `x = y` — **Low value**

Currently `x = y` emits OpText (empty String) + OpVarText (read y) +
OpAppendText (copy into x).  This does: allocate empty → reallocate
to fit → copy.

A dedicated `OpCloneText` could do `String::clone()` directly: one
allocation at the correct size, one memcpy.  Saves one reallocation.

**Impact:** Marginal — `String::clone()` vs empty + append is ~10%
difference in microbenchmarks.  Not worth a new opcode.

### O-S2. Pre-sized allocation for known lengths — **Low value**

For `x = "long literal string"`, the compiler knows the length at
compile time.  `String::with_capacity(len)` would avoid the realloc
on first append.

**Impact:** Negligible — short strings (< 16 chars) are the common
case, and the allocator typically over-provisions anyway.

### O-S3. Copy-on-write (Cow) for read-only variables — **Medium value, high complexity**

If a text variable is assigned once and only read thereafter, it
could stay as a borrowed `Str` instead of copying into an owned
`String`.  This requires:
- Mutation analysis in the parser (which variables are never mutated?)
- A third text representation: `Cow<'a, str>` or similar
- Fallback path for variables that are later mutated

This is analogous to the auto-const analysis for struct parameters.
The compiler already knows (via `find_written_vars`) which variables
are mutated.

**Impact:** Eliminates heap allocation for read-only text variables.
Significant for programs that pass text through multiple layers
without modifying it.  But the P115 auto-promotion mechanism shows
that mutation detection is feasible — we could do the inverse: keep
as Str until first mutation, then promote.

**Risk:** Lifetime management.  The borrowed Str points into the
caller's memory.  If the caller's String is freed or reallocated
while the callee still holds a Str, we get use-after-free.  This
is safe today because Str arguments have function-call lifetime.
Extending to local variables requires proving the source outlives
the borrower.

### O-S4. Small-string optimization (SSO) — **High value, high complexity**

Store strings ≤ 22 bytes inline in the 24-byte stack slot instead
of heap-allocating.  This eliminates heap allocation for the vast
majority of strings in typical programs (names, labels, short
messages).

Requires replacing `String` with a custom `SmallString` type that
stores either inline data or a heap pointer.  Every text operation
(`OpAppendText`, `OpClearText`, `OpVarText`, `OpFreeText`) needs
to handle both representations.

**Impact:** High — eliminates ~80% of text heap allocations in
typical programs.  But the implementation cost is substantial.

---

## Recommendation

The current design is already well-optimized for the common cases.
The `Str`/`String` split, destination passing, and work-text reuse
handle the important paths.

**No immediate action needed.**  If profiling reveals text allocation
as a bottleneck, O-S3 (copy-on-write for read-only variables) is the
most impactful optimization that integrates with the existing
architecture.  O-S4 (SSO) delivers the highest raw improvement but
requires a custom string type that touches every text operation.
