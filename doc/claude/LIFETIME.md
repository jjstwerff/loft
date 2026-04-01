# Lifetime — Dependency Tracking and Scope-Based Freeing

How the `dep` field on `Type::Text`, `Type::Reference`, `Type::Vector`, and other
heap-owning types interacts with scope exit to decide what gets freed.

---

## The dep field

Every heap-owning type carries a `Vec<u16>` dependency list:

```rust
Type::Text(Vec<u16>)              // text buffer on the stack frame
Type::Reference(u32, Vec<u16>)    // store-allocated record
Type::Vector(Box<Type>, Vec<u16>) // dynamic vector
Type::Enum(u32, bool, Vec<u16>)   // struct-enum variant (is_ref=true)
Type::Sorted(u32, .., Vec<u16>)   // sorted collection
Type::Index(u32, .., Vec<u16>)    // unique index
Type::Hash(u32, .., Vec<u16>)     // hash table
Type::Spacial(u32, .., Vec<u16>)  // spatial index
```

Each `u16` in the list is a **variable number** (`v_nr`).  The meaning:

| dep list | Meaning | Free behaviour |
|----------|---------|----------------|
| `[]` (empty) | **Owned** — this variable allocated the value | Freed at scope exit |
| `[v]` | **Borrowed** — derived from variable `v` | NOT freed (owner frees it) |
| `[v, w, …]` | Borrowed from multiple ancestors | NOT freed |

The dep list answers: "who is responsible for freeing this value?"  An empty list
means "I am"; a non-empty list means "someone else is".

---

## How deps are created

### `Type::depending(on: u16)` — `src/data.rs:209`

Adds variable `on` to the front of the dep list, producing a borrowed variant of the
same type.  Called whenever a value is **derived from** another variable:

```rust
// Reading a variable produces a value that depends on that variable.
// src/parser/objects.rs:135
t = self.vars.tp(v_nr).depending(v_nr);

// Accessing a field inherits the parent's deps plus the parent variable.
// src/parser/fields.rs:58-61
for on in dep { t = t.depending(on); }
t = t.depending(*nr);  // nr = the struct variable

// Iterating a vector: element depends on the vector variable.
// src/parser/collections.rs:832
in_type = in_type.depending(vec_var);
```

### Text exception — `src/parser/objects.rs:112-113`

When reading a `Text` variable, the type is cloned **without** adding the self-dep:

```rust
if matches!(self.vars.tp(v_nr), Type::Text(_)) {
    t = self.vars.tp(v_nr).clone();     // keeps existing deps, no self-dep added
} else {
    t = self.vars.tp(v_nr).depending(v_nr);  // adds self-dep
}
```

This is because text lives on the stack frame (as a `Str` pointer+length), not in the
store.  A text value IS the variable — there is no separate heap allocation that the
variable "points to" and could outlive.

### `Function::depend(var_nr, on)` — `src/variables/mod.rs:499`

Mutates a variable's type in-place to add a dep.  Used when the parser discovers
that `var_nr` borrows from `on` (e.g. a vector element assigned to a local).

---

## Scope exit — `get_free_vars` — `src/scopes.rs:564-665`

When a scope ends (block exit, function return, loop iteration boundary), the scope
analysis emits free operations for variables registered in that scope.

### Step 1: Collect variables — `variables(to_scope)` — `src/scopes.rs:503-534`

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

---

## Summary: Text vs Reference freeing

| | Text (`Type::Text(dep)`) | Reference (`Type::Reference(d, dep)`) |
|---|---|---|
| **Storage** | Stack frame (`Str` = ptr + len) | Store heap (12-byte `DbRef`) |
| **dep list used for freeing?** | No — always freed | Yes — only freed when `dep.is_empty()` |
| **dep list purpose** | Type compatibility / format string tracking | Ownership tracking |
| **Free opcode** | `OpFreeText` | `OpFreeRef` |
| **skip_free flag** | Not checked | Checked |
| **Return-value exemption** | By `ret_var` identity only | By `ret_var` identity AND `tp.depend().contains(&v)` |

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

For a text field on a struct variable `p`, this produces `Type::Text([v_p])` — the
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
(`src/parser/objects.rs:112-113`).  This is correct because text lives on the stack
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

### Closure record allocation — `src/parser/vectors.rs:628-700`

- Anonymous struct `__closure_N` with fields matching each captured variable
- Allocated at lambda **definition time** via `OpDatabase`
- Each captured value is **copied** into the record's fields (set_field)
- The record's `DbRef` is embedded in the 16-byte fn-ref slot
- Work variable `__clos_N` has type `Type::Reference(closure_d_nr, [])` — owned

### Inside the lambda: `__closure` is a struct parameter

When parsing the lambda body, the compiler adds a hidden `__closure` parameter:

```rust
// src/parser/vectors.rs:393-399
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

**Path 1** — known closure variable (`src/parser/objects.rs:91-95`):
```rust
let closure_d_nr = self.data.def(self.context).closure_record;
let fnr = self.data.attr(closure_d_nr, name);
*code = self.get_field(closure_d_nr, fnr, Value::Var(self.closure_param));
t = self.data.attr_type(closure_d_nr, fnr);
```

**Path 2** — capture_context variable (`src/parser/objects.rs:168-170`):
```rust
*code = self.get_field(closure_d_nr, fnr, Value::Var(self.closure_param));
t = self.data.attr_type(closure_d_nr, fnr);
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

### Missing dep: closure field reads do not add `__closure` — `objects.rs:94-95, 169-170`

Both closure field-read paths set `t = self.data.attr_type(...)` but do **not** call
`t = t.depending(self.closure_param)`.  The resulting text type has no reference to the
`__closure` variable.

For a regular struct this would mean:

```
p.name  → Text([v_p])     ← normal field access adds v_p
```

But for a closure:

```
__closure.prefix  → Text([])     ← closure field read, v___closure NOT added
```

This missing dep is the reason the closure record's lifetime is not linked to the text
it contains.  Inside the lambda body this is not immediately fatal because `__closure`
is a function argument (never freed).  But it means the **return type** of the lambda
carries `Text([])` instead of `Text([v___closure])`, and this propagates problems
outward.

### The real crash: `___clos_N` is freed before the fn-ref is returned

The runtime trace for the cross-scope test reveals the actual crash site:

```loft
fn make_greeter(prefix: text) -> fn(text) -> text {
    fn(name: text) -> text { "{prefix} {name}" }
}
make_greeter("Hello")("world")
```

The IR for `make_greeter` is:

```
fn n_make_greeter(prefix:text) -> function([text], text) {
  ___clos_1:ref(__closure_0) = null;
  {#fn_ref_with_closure
    OpDatabase(___clos_1, 52);           ← allocate closure record
    OpSetText(___clos_1, 0, prefix);     ← copy "Hello" into closure
    OpFreeRef(___clos_1);                ← FREE the closure record!
    FnRef(511, ___clos_1);               ← try to read freed record → crash
  }
}
```

`___clos_1` has `Type::Reference(closure_d_nr, [])` — owned, empty deps.
The block's return type is `Type::Function(params, ret)` — **no dep field**.

At scope exit, `get_free_vars` checks for `___clos_1`:
1. `dep.is_empty()` → true (owned)
2. `!tp.depend().contains(&___clos_1)` → true (`Function` has no deps → `depend()` returns `[]`)
3. `!function.is_skip_free(___clos_1)` → true

All three conditions pass → `OpFreeRef(___clos_1)` is emitted inside the block,
before the `FnRef` that tries to read the closure's DbRef.

### The root cause: `Type::Function` cannot carry deps

The scope exit logic suppresses `OpFreeRef` when the return type's dep list mentions
the variable (`tp.depend().contains(&v)`).  This works for text returning a struct's
field: `Text([v_p])` protects `p`.

But `Type::Function(Vec<Type>, Box<Type>)` has no `Vec<u16>` dep field.  Even though
the fn-ref embeds the closure DbRef (bytes 4-16 of the 16-byte fn-ref slot), the type
system cannot express "this fn-ref depends on `___clos_1`".

### Proposed fix: add deps to `Type::Function`

Change `Type::Function` from:
```rust
Function(Vec<Type>, Box<Type>)            // params, return
```
to:
```rust
Function(Vec<Type>, Box<Type>, Vec<u16>)  // params, return, deps
```

Then in `emit_lambda_code` (`src/parser/vectors.rs:666`), build the fn_type with the
closure work variable as a dep:

```rust
// Before:
let fn_type = Type::Function(visible_params, Box::new(ret_tp));
// After:
let fn_type = Type::Function(visible_params, Box::new(ret_tp), vec![w]);
```

At scope exit, `tp.depend()` now returns `[___clos_1]`, so condition 2 becomes false
→ `OpFreeRef` is suppressed → the closure record survives the fn-ref block.

### Would this fix the crash?

**Yes** — for the `make_greeter` case.  The dep on the fn-ref type prevents
`___clos_1` from being freed inside `make_greeter`.  The fn-ref carries the closure
DbRef embedded in its 16-byte slot, and it escapes via the return value.  The closure
store record stays alive.

### Remaining issue: who frees the closure record at the caller?

At the call site:
```
___fn_ref_tmp_1:function([text], text) = n_make_greeter("Hello");
fn_ref[1]("world");
```

`___fn_ref_tmp_1` is `Type::Function(params, ret, [])` — assigned from a call result,
which produces owned (empty deps) values.  When `___fn_ref_tmp_1` goes out of scope,
`get_free_vars` only checks `Reference`, `Vector`, and `Enum(_, true, _)`:

```rust
if let Type::Reference(_, dep) | Type::Vector(_, dep) | Type::Enum(_, true, dep) = ...
```

`Type::Function` is not matched.  The 16-byte fn-ref slot is stack memory (reclaimed
by `FreeStack`), but the 12-byte DbRef at offset+4 points to a store-allocated closure
record that is **never explicitly freed**.

**For the immediate test** (`make_greeter("Hello")("world")`), this is a chained call —
the fn-ref is consumed instantly by `CallRef` and the closure record leak is negligible.

**For stored fn-refs** (e.g. `f = make_greeter("Hello"); ... f("world"); ...`), the
closure record leaks until the store is reclaimed at program exit.  Fixing this requires
`get_free_vars` to handle `Type::Function` with non-empty deps by emitting an
`OpFreeRef` targeting the DbRef at offset+4 within the fn-ref slot.  This is a separate
enhancement.

### Additional fixes needed alongside Function deps

1. **`can_convert`** (`src/parser/mod.rs:626-662`) — must accept `Type::Text` with
   different dep lists and `Type::Function` with compatible params/return (ignoring
   deps).

2. **Closure field reads** (`src/parser/objects.rs:94-95, 169-170`) — must add
   `t = t.depending(self.closure_param)` so text read from the closure record
   depends on `__closure`, matching the pattern for normal struct field access.

3. **`depending()`** and **`depend()`** (`src/data.rs`) — must handle
   `Type::Function(params, ret, dep)`.

4. **`is_equal()`** (`src/data.rs`) — must compare Function types ignoring deps.

5. **All match sites** — ~60 occurrences of `Type::Function(_, _)` across the codebase
   must become `Type::Function(_, _, _)`.

### Same-scope closures: why they work today without this fix

The passing tests (`closure_capture_text_return`, `closure_capture_text_loop`) use
same-scope closures where the closure is defined and called within one function:

```loft
greeting = "hello";
f = fn(name: text) -> text { "{greeting}, {name}!" };
f("world")
```

Here `___clos_N` and the fn-ref variable `f` live in the **same** scope.  The block
containing the fn-ref construction is not a separate scope — it's inlined.  So
`get_free_vars` doesn't run between closure allocation and fn-ref use.  The closure
record stays alive until the outer scope exits, by which time the call has completed.

Cross-scope closures (closure returned from a function) break because `make_greeter`
has its own scope exit where `___clos_1` is freed before the fn-ref escapes.

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

## Diagnostic: `LOFT_LOG=scope_debug`

Set `LOFT_LOG=scope_debug` to trace free decisions at compile time:

```
[scope_debug] freeing 'p' (var=3, scope=2)
[scope_debug] NOT freeing 'name' (var=5, scope=2): dep_empty=false in_ret=false skip_free=false
[scope_debug] ORPHANED Reference 'x' (var=7): its scope=4 is not in the chain to to_scope=2
```

The orphan check catches variables whose scope was never entered in the current chain —
a condition that should not occur after the A5.6 block-pre-registration fix.
