
# Lifetime — Dependency Tracking and Scope-Based Freeing

How the `dep` field on `Type::Text`, `Type::Reference`, `Type::Vector`, and other
heap-owning types interacts with scope exit to decide what gets freed.

---

## Dep field and scope exit freeing

The `dep` field on `Type` controls ownership and freeing.  See `src/data.rs`
(Type enum doc) and `src/scopes.rs` (module doc) for the core semantics.

## Scope exit — `get_free_vars`

When a scope ends (block exit, function return, loop iteration boundary), the scope
analysis emits free operations for variables registered in that scope.

### Step 1: Collect variables — `variables(to_scope)` — `src/scopes.rs:503-533`

Walk the scope stack from the current scope back to `to_scope`.  Collect all variables
whose `var_scope` is in this range.  Variables are returned in **reverse insertion
order** (most-recently-created first) to satisfy the LIFO invariant on store freeing.

### Step 2: Skip the return variable

The variable being returned (`ret_var`, found by `returned_var(expr)`) is never freed —
it escapes the scope.

### Step 3: Emit free ops per variable

For each variable `v` in the collected set:

#### Tuples (`Type::Tuple`)
```
→ (T1.3/T1.4: per-element free — not yet fully implemented)
→ skip to next variable
```

#### Text (`Type::Text(_)`)
```rust
if matches!(function.tp(v), Type::Text(_)) {
    ls.push(call("OpFreeText", v, data));   // ALWAYS emitted
}
```

**Text is always freed** at scope exit, regardless of its dep list.  This is because
text occupies stack-frame memory (a `Str` struct = pointer + length).  The stack frame
is about to be reclaimed, so the text buffer must be released.

The dep list on Text is used for **type compatibility checking**, not for free decisions.

#### References, Vectors, Struct-Enums (`Type::Reference(_, dep)` etc.)
```rust
let emit = dep.is_empty()                    // (1) I am the owner
         && !tp.depend().contains(&v)         // (2) not escaping via return type
         && !function.is_skip_free(v);        // (3) not marked skip_free
if emit {
    ls.push(call("OpFreeRef", v, data));
}
```

Three conditions must ALL be true to emit `OpFreeRef`:

1. **`dep.is_empty()`** — the variable owns its store allocation.  If deps are
   non-empty, some other variable owns the underlying store record and will free it.

2. **`!tp.depend().contains(&v)`** — the return type's dep list does not mention this
   variable.  If it does, the value escapes via the return expression and must not be
   freed here.

3. **`!function.is_skip_free(v)`** — the variable is not marked `skip_free`.  This
   flag is set by `clean_work_refs` for work-ref temporaries that are re-purposed
   after use (A14), and by `set_skip_free` for borrowed references like par-loop
   result variables.

#### Function (`Type::Function`) — freed at scope exit (verified 2026-05-12)

`Type::Function` does not appear in the `get_free_vars` match by name, but the
closure DbRef embedded at offset+4 of the 16-byte fn-ref slot is freed at
scope exit by two mechanisms that cover every shipped capture pattern:

- **Local var (D1)**: closure record dies with the stack frame via the
  standard local-cleanup path; the captured payload (text / reference / etc.)
  is freed transitively.
- **Struct field (D3)**: @P213's `Parts::ChildRec` cascade walks the host
  struct's fields at scope exit and frees the closure record co-located in
  the host store.

**Verified clean** by `tests/leak.rs::p15_phase03_closure_text_capture_*_no_leak`
— two 100-iteration tight loops calling capturing closures (text capture in
both D1 and D3 destinations) with `state.check_store_leaks()` asserting zero
leaked stores after.

The legacy "Implementation path" section below describes a `get_free_vars`
`Type::Function` arm that was contemplated but never landed because the
`Parts::ChildRec` + standard-local-cleanup combination already covers the
surface.  The cross-scope closure work it discusses (Steps 1–4) DID land
through @P213/P215/P227 by 2026-05-05 — read it as historical context, not
as current open work.

Plan-15 phase 06 will trim the legacy "Implementation path" section once
the matrix is complete; the C2 cells in `tests/closure_matrix.rs` plus the
leak guards above are the canonical regression record for this surface.

---

## Summary: Text vs Reference vs Function freeing

| | Text (`Type::Text(dep)`) | Reference (`Type::Reference(d, dep)`) | Function (`Type::Function(p, r, dep)`) |
|---|---|---|---|
| **Storage** | Stack frame (`Str` = ptr + len) | Store heap (12-byte `DbRef`) | Stack frame (16B: 4B d_nr + 12B closure DbRef) |
| **dep list used for freeing?** | No — always freed | Yes — only freed when `dep.is_empty()` | Indirectly — `tp.depend()` keeps closure-record dep visible to the standard `Reference` free path |
| **dep list purpose** | Type compatibility / format string tracking | Ownership tracking | Protects closure work var via `tp.depend()` |
| **Free opcode** | `OpFreeText` | `OpFreeRef` | `OpFreeRef` on the closure-record DbRef (D1: stack-frame exit; D3: `Parts::ChildRec` cascade) |
| **skip_free flag** | Not checked | Checked | N/A |
| **Return-value exemption** | By `ret_var` identity only | By `ret_var` identity AND `tp.depend().contains(&v)` | Via `tp.depend()` on declared return type |

---

## View returns — dep merge and materialisation (#306)

A function may return a *view*: a struct read out of a vector (`table[idx]`)
aliases the vector's store rather than owning one.  Two rules keep the caller
from freeing a store it does not own:

1. **View of a parameter → dep merge.**  `ref_return` walks the returned
   local's type deps *transitively* and merges every reachable parameter into
   the declared return deps (`fn pick(t, i) -> M` whose body returns
   `chosen = t[i]; chosen` declares `M["chosen", "t"]`).  The call site maps
   the dep to its argument, so the result is a borrow there and is not freed.
2. **View of a local → materialise.**  When a transitive dep resolves to a
   non-parameter local (`pool = build(); pool[i]`), the owner dies at function
   exit, so deps cannot help the caller.  `block_result` / `parse_return`
   rewrite the return value into a deep copy of the view into a fresh
   work-ref, which NRVO-promotes into the caller-provided buffer
   (`materialize_view_return`, `src/parser/control.rs`).

Var-to-var **re**assignment of same-struct references deep-copies (matching
first assignment, `generate_set` reassignment arm) — the parser strips the
LHS deps on that shape assuming a copy, so aliasing there let a later
`OpFreeRef` release a store the variable never owned.

**Ownership-transition free (#316).**  A var whose latest assignment was
OWNED (`chosen = m_none()`) and which is then reassigned with a BORROW
(`chosen = pool[i] ?? m_none()`) would orphan the owned store: the merged
static type already carries deps, so codegen's dep-empty pre-Set free never
fires.  `scan_set` (`src/scopes.rs`) tracks each Reference var's
latest-assignment ownership through the IR (path-sensitive across `if`
branches, conservative across loop bodies) and emits an explicit
`OpFreeRef(v)` before the borrowing Set.  Only provably-owned shapes are
tracked (a call with no visible-attr return dep; a same-struct var copy);
everything else is Unknown and never freed this way.

**Self-read transition residual (#328).**  When the borrowing RHS reads the
variable itself (`x = x.next` — the linked-list walk), the pre-Set free is
skipped (it would invalidate what the RHS dereferences), so the owned store
survives until scope exit: ONE bounded store per owned→self-read-borrow
transition, also when the reassign sits inside a loop (the depth guard).
Correctness is unaffected.  Walks that start from a borrow (e.g. iterating a
structure owned elsewhere) do not pay it.

Two runtime backstops make any future wrong-free loud instead of corrupting:
`free_named` refuses to free the eval-stack store (slot 0,
`Stores::stack_store_at_zero`), and the slot allocator panics with a
"store table exhausted" diagnosis at 65535 live stores instead of wrapping
the u16 watermark back onto the stack store.

---

## The return-value exemption in detail

When a scope exits with a return expression, two mechanisms prevent premature freeing:

### 1. `ret_var` — identity match

`returned_var(expr)` walks the return expression to find the last `Value::Var(v)`.
That variable is skipped entirely in `get_free_vars`.  Works for both Text and
Reference.

### 2. `tp.depend().contains(&v)` — dependency match

The return **type** (`tp`) carries a dep list.  If variable `v` appears in that list,
the return value borrows from `v`, so `v` must outlive the scope.  This only applies
to References (not Text, which is always freed).

Example:
```loft
fn get_name(p: Point) -> text {
    p.name    // returns text; p (Reference) is in return type's deps
}
```
Here `p` has `Type::Reference(Point_dnr, [])` (owned), but the return type is
`Type::Text([v_p])` where `v_p` is p's variable number.  The check
`tp.depend().contains(&v_p)` prevents `OpFreeRef` for `p`, keeping the store record
alive until the caller reads the text.

---

## Text from structs — deps keep the entire ownership chain alive

When a text field is read from a struct — or from any sub-reference reachable from
that struct — the resulting text type inherits every variable in the access chain as
dependencies.  This is critical: without it, any struct in the chain could be freed
while the text still points into store memory.

### How field access builds the dep chain — `src/parser/fields.rs:130-137`

```rust
// Normal (non-constant) field access:
let dep = t.depend();                    // existing deps on the struct expression
t = self.data.attr_type(dnr, fnr);       // field's declared type (e.g. Text([]))
for on in dep { t = t.depending(on); }  // inherit parent's deps
if let Value::Var(nr) = code {
    t = t.depending(*nr);                // add the struct variable itself
}
```

For a text field on struct variable `p`, this produces `Type::Text([v_p])` — the
text depends on variable `p`.

### Why this matters for freeing

Consider:
```loft
fn get_name(p: Point) -> text {
    p.name    // Type::Text([v_p])
}
```

At scope exit, the free logic processes each variable:

1. **`name` (the text result)**: Text is always freed via `OpFreeText` — the dep list
   is not checked.  But `name` is the `ret_var` (the variable being returned), so it
   is **skipped entirely**.

2. **`p` (the struct)**: `p` has `Type::Reference(Point_dnr, [])` — owned, empty deps.
   Normally this would be freed.  But the return type `tp` is `Type::Text([v_p])`, and
   the check `tp.depend().contains(&v_p)` finds `p` in the return type's dep list.
   So `OpFreeRef` is **suppressed** for `p`.

The struct stays alive long enough for the caller to read the text from the return
value.  The caller's scope then frees both the text and (eventually) the struct.

### The dependency chain protects against premature free

```
p.name → Type::Text([v_p])
                      ↑
                      └── "this text was read from p"
                          → at scope exit, p must not be freed if this text escapes
```

Without the dep, `p` would have `dep.is_empty() == true` and no mention in the
return type's dep list, so `OpFreeRef` would fire — deallocating the store record
while the caller still expects to read the returned text from it.

### Sub-references and intermediate variables extend the chain

The same mechanism applies to any depth of struct nesting.  When field access traverses
sub-references, each step inherits the parent's deps and adds the parent variable, so
the final text carries the entire ownership chain:

```loft
fn get_city(company: Company) -> text {
    addr = company.address;   // Type::Reference(Address_dnr, [v_company])
    addr.city                 // Type::Text([v_addr, v_company])
}
```

The return type `Text([v_addr, v_company])` contains both `v_addr` and `v_company`.
At scope exit:
- `v_addr` is found in `tp.depend()` → `OpFreeRef` suppressed for `addr`
- `v_company` is found in `tp.depend()` → `OpFreeRef` suppressed for `company`

The entire chain from text back to the root struct is kept alive.

### Deeper nesting — every sub-reference is protected

This extends to arbitrary depth.  Each field access step in `parse_field`
(`src/parser/fields.rs:130-137`) inherits deps from the parent expression and adds the
parent variable, so deps accumulate transitively:

```loft
fn get_street(org: Organization) -> text {
    hq = org.headquarters;          // Reference(Office_dnr, [v_org])
    loc = hq.location;              // Reference(Location_dnr, [v_hq, v_org])
    loc.street                      // Text([v_loc, v_hq, v_org])
}
```

At scope exit the return type `Text([v_loc, v_hq, v_org])` protects all three
references from being freed:

```
loc.street → Text([v_loc, v_hq, v_org])
                    ↑      ↑     ↑
                    │      │     └── org must stay alive (root owner)
                    │      └──────── hq must stay alive (intermediate)
                    └─────────────── loc must stay alive (direct parent)
```

Without this transitive dep chain, freeing `org` at scope exit would deallocate the
store record that `hq` points into, and freeing `hq` would deallocate the record that
`loc` points into — both while the returned text still references store memory.

The same principle applies to non-text sub-references too: a borrowed Reference
(`dep` non-empty) is never freed by `get_free_vars`, and its presence in the return
type's dep list prevents its parent from being freed.  Text is simply the most visible
case because text is always freed unless it is the `ret_var` itself — making the dep
chain on the return type the only thing keeping the structs alive.

### Text-to-text: no dep added

When reading a text *variable* (not a struct field), no self-dep is added
(`src/parser/objects.rs:115-119`).  This is correct because text lives on the stack
frame — there is no separate store allocation to protect.  The text variable IS the
value, so the `ret_var` identity check is sufficient.

---

## Closures are structs — the same lifetime rules apply

A closure record is a store-allocated struct.  When a lambda captures variables, the
compiler synthesizes an anonymous struct `__closure_N` with fields matching each
captured variable.  At runtime, the closure record is a `DbRef` pointing to a store
allocation — identical to any other struct.

This means text read from a closure record should follow the exact same dep chain
rules as text read from a regular struct: the text must depend on the closure record
variable, and that dep must keep the closure store allocation alive.

### Closure record allocation — `src/parser/vectors.rs:628-712`

- Anonymous struct `__closure_N` with fields matching each captured variable
- Allocated at lambda **definition time** via `OpDatabase`
- Each captured value is **copied** into the record's fields (set_field)
- The record's `DbRef` is embedded in the 16-byte fn-ref slot
- Work variable `__clos_N` has type `Type::Reference(closure_d_nr, [])` — owned

### Adopted vs borrowed captures — who frees the store behind a capture (#682)

A reference / collection capture is stored as a 12-byte `DbRef`, so exactly one owner
must reclaim it.  `free_named`'s cascade (`src/database/allocation.rs`) frees the
record's `DbRef` fields when the record dies, and `get_free_vars`' `captured_ref`
suppresses the defining frame's own `OpFreeRef` for a captured reference.  Those two
are a PAIR: the suppression is what hands the store over, and it is what makes an
escaping factory closure sound (#323).

The pairing only holds where a frame free existed to suppress.  A captured
**parameter** never enters the scope-exit sweep at all (`variables()`: "never return
function arguments"), and a **projection local** (`ch = w.chunks[1]`, a `for` element)
is `owns == false` — so for both, the cascade was a second free of a live store.  It
destroyed the caller's value, and because a freed-but-unreused store still reads
correctly, the crash surfaced thousands of ops later in whatever function next
touched it.

The two cases are now distinct in the schema.  Both are 12 bytes at align 4, so
nothing about layout, reads or writes changes — only the free decision:

| Marker (`Deps`) | Field storage type | Cascade frees it? |
|---|---|---|
| `share_sentinel()` | `dbref` | yes — the record ADOPTED the store |
| `borrowed_share_sentinel()` | `dbref_borrow` | no — someone else owns it |

**The verdict is not knowable at parse time**, which is why it is not decided in
`synthesize_closure_record`.  `ch = pick(w, 1)` parses as "borrows `w`" from the
callee's declared return, and only `scopes::check`'s call-result rewrite
(`make_independent`, the `!adopts_fresh_store` arm) turns it into OWNED once it knows
the return ABI deep-copies into a fresh store.  Reading the parse-time dep leaked
that copy.  So the decision is `scopes::mark_borrowed_captures`, run after every dep
rewrite has settled; `--native` reads the marked attribute type directly, while the
interpreter's already-registered schema is synced by
`typedef::sync_capture_ownership` from `compile::byte_code_from`.

A `__cell_<T>` capture (plan-22's boxed mutated scalar / text) is ALWAYS adopted: the
cell is minted for that closure alone, so the record is its only possible owner
however the original binding was reached.

Both `dbref` shapes are registered **together**, from either entry point: type
numbers are positional and `--native` replays the registration sequence to rebuild
the schema, so a shape present in only some programs would shift every id after it.

### Inside the lambda: `__closure` is a struct parameter

When parsing the lambda body, the compiler adds a hidden `__closure` parameter:

```rust
// src/parser/vectors.rs:391-399
let closure_tp = Type::Reference(closure_rec, vec![]);
self.data.add_attribute(&mut self.lexer, d_nr, "__closure", closure_tp.clone());
let v_nr = self.create_var("__closure", &closure_tp);
self.vars.become_argument(v_nr);
self.closure_param = v_nr;
```

`__closure` is `Type::Reference(closure_rec, [])` — an owned Reference that is a
function argument (scope 0, never freed by `get_free_vars`).

### Reading captured variables = reading struct fields

When the lambda body references a captured variable like `prefix`, the parser redirects
it to a field read from the closure record.  There are two code paths:

**Path 1** — known closure variable (`src/parser/objects.rs:91-98`):
```rust
let closure_d_nr = self.data.def(self.context).closure_record;
let fnr = self.data.attr(closure_d_nr, name);
*code = self.get_field(closure_d_nr, fnr, Value::Var(self.closure_param));
t = self.data.attr_type(closure_d_nr, fnr);
t = t.depending(self.closure_param);   // A5.6-text: add __closure as dep
```

**Path 2** — capture_context variable (`src/parser/objects.rs:172-175`):
```rust
*code = self.get_field(closure_d_nr, fnr, Value::Var(self.closure_param));
t = self.data.attr_type(closure_d_nr, fnr);
t = t.depending(self.closure_param);   // A5.6-text: add __closure as dep
```

### The analogy to regular struct field access

Compare the closure field read above with normal field access
(`src/parser/fields.rs:130-137`):

```rust
let dep = t.depend();
t = self.data.attr_type(dnr, fnr);       // same — get field's declared type
for on in dep { t = t.depending(on); }  // inherit parent's deps
if let Value::Var(nr) = code {
    t = t.depending(*nr);                // add the struct variable as dep
}
```

Normal field access adds the struct variable (and its deps) to the result type.
For a text field on struct `p`, this produces `Type::Text([v_p])` — the text depends
on `p`, which prevents `p` from being freed while the text escapes.

Both closure field-read paths add `self.closure_param` as a dependency after reading
the field type, matching the pattern for normal struct field access.  This produces
`Type::Text([v___closure])` for a text field read from a closure, matching the
`Type::Text([v_p])` produced by `p.name` for a regular struct.  The dep keeps the
closure record alive while derived text is in use.

---

## The 16-byte fn-ref slot layout

A `Type::Function` variable occupies 16 bytes on the stack:

```
bytes  0.. 4: d_nr       (i32, function definition number)
bytes  4..16: closure     (DbRef, 12 bytes; null sentinel if no closure)
```

### Key codegen paths

| Component | Location | Role |
|-----------|----------|------|
| `gen_set_first_at_tos` | `codegen.rs:843-847` | Delegates to `gen_fn_ref_value` for Function vars |
| `gen_fn_ref_value` | `codegen.rs:466-490` | Ensures every if-else branch produces 16B |
| `OpVarFnRef` | `02_files.loft:350` | Push 16B fn-ref from frame variable |
| `OpPutFnRef` | `02_files.loft:354` | Pop 16B fn-ref into frame variable |
| `OpNullRefSentinel` | `01_code.loft:733` | Pads non-capturing lambdas (4B d_nr → 16B) |
| `fn_call_ref` | `state/mod.rs:221-249` | Reads d_nr at offset 0, closure at offset+4 |

### fn-ref type carries closure dep — `vectors.rs:666-669`

```rust
// A5.6-text: fn-ref depends on closure work var `w` so that
// get_free_vars does not emit OpFreeRef for the closure record
// before the fn-ref escapes the defining scope.
let fn_type = Type::Function(visible_params, Box::new(ret_tp), vec![w]);
```

### Return type dep propagation — `vectors.rs:701-711`

When the enclosing function returns a fn-ref, the closure dep `w` is propagated to
the declared return type so `get_free_vars` at the Return statement sees
`tp.depend()` containing `w`:

```rust
if matches!(self.data.def(self.context).returned, Type::Function(_, _, _)) {
    self.data.definitions[self.context as usize].returned =
        self.data.definitions[self.context as usize].returned.depending(w);
}
```

---

## Current status of closure freeing

### Same-scope closures: WORKING

All same-scope closure tests pass.  `___clos_N` and the fn-ref live in the same
function scope; `get_free_vars` doesn't run between closure allocation and fn-ref use.

**Passing tests** (tests/expressions.rs):
- `closure_capture_integer` (line 317)
- `closure_capture_after_change` (line 323)
- `closure_capture_multiple` (line 335)
- `closure_capture_text_integer_return` (line 355)
- `closure_capture_text_return` (line 364)
- `closure_capture_struct_ref` (line 375)
- `closure_capture_vector_elem` (line 390)
- `closure_capture_text_loop` (line 406)

### Cross-scope closures: WORKING

**`closure_capture_text`** (tests/expressions.rs:343) now passes.

Four bugs were fixed:

1. **Free suppression** — `get_free_vars` used only the block result type (`tp`) for
   the dep check, but the block result type doesn't carry the closure dep that was
   propagated to the function's declared return type.  Fix: also check
   `data.def(self.d_nr).returned.depend()`.

2. **Work-buffer propagation** — the declared `fn(text) -> text` return type didn't
   encode the lambda's work-buffer deps.  `try_fn_ref_call` created zero work buffers,
   so the lambda's `__work_1` parameter received garbage.  Fix: `emit_lambda_code`
   replaces the inner return type with the lambda's actual return type.

3. **fn-ref null pre-init** — `gen_set_first_at_tos` emitted only `NullRefSentinel`
   (12 bytes) for Function variables, but fn-ref slots are 16 bytes.  `PutFnRef`
   overwrote 4 bytes of the next variable.  Fix: emit `ConstInt(i32::MIN)` +
   `NullRefSentinel` for a full 16-byte null slot.

4. **Caller-side closure free** — `get_free_vars` had no `Type::Function` branch.
   The closure DbRef at offset+4 leaked when fn-ref variables went out of scope.
   Fix: add a `Type::Function` arm with a codegen special case that reads the
   closure via `OpVarRef(var_pos - 4)` before `OpFreeRef`.  Same-scope fn-refs
   carry `dep=[w]` (the closure work var), so the free is suppressed — `___clos_N`
   already handles it.

### Caller-side closure free: native codegen path

The interpreter frees closure records via the codegen special case described
above.  Native codegen handles closure-record drop via Rust's RAII at
stack-frame exit (no explicit `OpFreeRef` emission needed); @PLAN15 phase
03–05 leak guards (`tests/leak.rs::p15_phase03_*` / `_phase04_*` /
`_phase05_*`) confirmed both paths produce clean store state under
100-iteration tight loops for text / Reference / nested-closure captures
across D1 + D3.  Cross-mode equivalence (`tests/closure_matrix.rs`)
catches any future native-vs-interp divergence in observable output.

---

## Historical implementation path (closed)

The three bugs that originally motivated `Type::Function` work in
`get_free_vars` — cross-scope closure freeing, caller-side cleanup,
and capturing-into-struct-field — closed through @P213 (struct-field
layout via `Parts::ChildRec`, 2026-05-04), @P215 (nested-closure
name resolution, 2026-05-05), and @P227 (text-returning fn-ref
calls, 2026-05-05).  Plan-15 phases 03–05 (2026-05-12) confirmed
no residual leak via `tests/leak.rs` 100-iteration tight loops
across capture types (text / Reference / nested) and destinations
(local / struct field).

The detailed step-by-step "Implementation path" that previously
lived here described the contemplated `OpFreeClosureRef` opcode +
`get_free_vars` extension — neither shipped because the
`Parts::ChildRec` cascade plus standard local-cleanup already
covers the surface.  Removed during @PLAN15 phase 06 closeout
(2026-05-12); see git history if you need the original analysis.

---

## `OpFreeText` runtime — `src/state/text.rs:270`

```rust
pub fn free_text(&mut self) {
    let pos = *self.code::<u16>();           // stack slot
    let s = self.string_mut(pos);
    s.clear();
    s.shrink_to(0);                          // release heap allocation
}
```

Clears and deallocates the string buffer at the given stack position.  In debug
builds, fills freed memory with `'*'` and checks for double-free.

## `OpFreeRef` runtime — `src/state/io.rs:414-437`

```rust
pub fn free_ref(&mut self) {
    let db = *self.ref_at(pos);              // read DbRef from stack
    self.database.free(&db);                 // return store slot to free list
}
```

Returns the store record to the free list.  The store allocator uses a bitmap
(`free_bits`) for slot reclamation (S29).

---

## Inline-lift safety — the `OpCopyRecord | 0x8000` invariant

Struct-returning calls that appear inline in an expression (format-string
interpolation, chained accessor, assertion, tuple element) are transformed by
scope analysis into `__lift_N = callee(...)` followed by
`OpCopyRecord(src, to=__lift_N, tp)`.  The top bit of `tp` (`0x8000`) is the
**free-source** flag: after copying the returned record into the destination,
the source store is freed.

The flag is necessary for **owned returns** — if the callee freshly allocated
its return (e.g. `fn f() -> T { T { .. } }`), nothing else would free that
store and issue #120 reintroduces.

The flag is **unsafe for borrowed-view returns** — if the callee returned a
view into one of its arguments (e.g. `fn f(c) -> Inner { c.items[0] }`), the
source store is the caller's own data.  Freeing it corrupts the caller.

### The gate

`src/state/codegen.rs` emits the flag only when the callee's declared return
type carries an **empty** `dep` chain (= owned).  If the chain is non-empty
(= view into some arg), the flag is cleared.

Two emission sites:

- `gen_set_first_ref_call_copy` (~`codegen.rs:1284`) — first-assignment from a call
- `generate_set` reassignment path (~`codegen.rs:918`) — re-assignment into an existing ref slot

Both sites test:
```rust
let is_borrowed_view = !stack.data.def(fn_nr).returned.depend().is_empty();
let tp_with_free = if is_borrowed_view {
    i32::from(tp_nr)                 // no free
} else {
    i32::from(tp_nr) | 0x8000        // safe to free (owned)
};
```

### Feeding the gate — dep merging from return expressions

For the gate to work, `def.returned.depend()` must reflect whether the
function's body ever returns a view.  Two parser helpers merge per-return
deps into the declared return type:

| Helper | File | Fires on |
|---|---|---|
| `text_return(ls)` | `parser/control.rs:2264` | `Type::Text` returns, in both `parse_return` (mid-body) and `block_result` (tail) |
| `ref_return(ls)` | `parser/control.rs:2351` | `Type::Reference` / `Type::Enum(_, true, _)` returns, in both `parse_return` (mid-body) and `block_result` (tail) |

The Vector arm of `ref_return` fires only from `block_result` (tail), not
from `parse_return` (mid-body) — promoting mid-body Vector deps would
promote globals and locals to hidden ref args and break callers.

For a mixed-return callee
```loft
fn first_or_empty(c: Container, idx: integer) -> Inner {
    if idx >= 0 && idx < len(c.items) {
        return c.items[idx];   // view
    }
    Inner { n: 0 }             // owned
}
```
the mid-body `return c.items[idx]` carries `Reference(Inner, [c])`.
`parse_return` calls `ref_return([c])`, which merges `c` into
`def.returned`'s dep chain via the `attr_names` idempotency path (since `c`
is already an attribute — no new hidden arg created).  The declared return
becomes `Reference(Inner, [c])`; the gate fires at the call site; `0x8000`
clears; the caller's store is untouched.

### Lock bracket — second line of defence

Both gated emission sites wrap the `OpCopyRecord` in `n_set_store_lock(arg,
true)` / `(arg, false)` for every ref-typed arg to the call.  The runtime
`copy_record` handler at `state/io.rs:1001` skips the source-free when the
source store is locked.  This is a belt-and-suspenders guard for the case
where the dep-chain inference is incomplete.

### Known trade-offs

1. **Owned-fallback leak on mixed-return callees.**  After the dep merge,
   the gate clears `0x8000` for every call to a mixed-return callee.  The
   owned fallback branch's fresh store is no longer freed and leaks.
   Magnitude: one small struct per fallback call; the fallback is typically
   an error path.  Future: promote Reference returns to a caller-provided
   scratch buffer (analogous to the `__ref_1` vector mechanism) to close
   this.

2. **WASM feature unconditionally clears `0x8000`** at
   `gen_set_first_ref_call_copy`.  Safe (no corruption) but leaks
   callee-fresh stores under WASM.  Separate audit.

3. **Vector mid-body returns** are not merged.  If a Vector-returning
   function has `return GLOBAL_CONST;` or `return local_vec;` in a branch,
   `ref_return`'s promotion logic would add hidden ref args that break
   callers.  No Vector SIGSEGV variant has been observed; a future phase
   could filter `ls` to function-parameter vars only.

### History

P181 surfaced the corruption; Phase 1 (2026-04-18) added the gate;
Phase 1b (2026-04-18) added the `parse_return` dep merge; Phase 2
(2026-04-18) audited all `OpCopyRecord` emission sites and confirmed
the invariant holds.  See
`doc/claude/plans/finished/00-inline-lift-safety/` for the full initiative record.

## Diagnostic: `LOFT_LOG=scope_debug`

Set `LOFT_LOG=scope_debug` to trace free decisions at compile time:

```
[scope_debug] freeing 'p' (var=3, scope=2)
[scope_debug] NOT freeing 'name' (var=5, scope=2): dep_empty=false in_ret=false skip_free=false
[scope_debug] ORPHANED Reference 'x' (var=7): its scope=4 is not in the chain to to_scope=2
```

The orphan check catches variables whose scope was never entered in the current chain —
a condition that should not occur after the A5.6 block-pre-registration fix.

## Diagnostic: `introspect --show-ownership` + the free-before-dependent-read overlay

`loft introspect --show-ownership <prog>` renders each binding's resolved ownership
(`Owned` / `Borrowed(base)` / `Join(base)`, with `Owned (backing=…)` for a buffer). This is
the STATIC half of the @PLN103 lifetime inspector (the runtime half is `LOFT_STORES=timeline`).

The verdicts alone are **temporal-agnostic** — they say *who owns what*, not *when a store is
freed relative to its reads* — so a correctly-placed free and a use-after-free render
identically. The overlay closes that gap: `use_analysis::free_before_dependent_read` walks the
committed IR and, along each straight-line path, flags an `OpFreeRef(S)` followed by a
DEREFERENCE of any (transitive) view of `S`:

```
  ⚠ UAF: `arg` is read AFTER `OpFreeRef(__vdb_1)` — `arg` views the freed store
     (backing=__vdb_1); free-before-dependent-read
```

Precision rests on two axes: it follows **transitive** view chains (a nested `match arg {…}`
derefs `arg` through an intermediate that only transitively views the store) and counts only
**dereference** reads — a bare `Var` in a `return`/move is a safe delivery the retbuf/ownership
machinery handles, not a UAF. Blind spot: a bug whose root is a *missing* dep (a dropped borrow)
cannot be seen by a dep-based check. Gate: `tests/introspect.rs`.
