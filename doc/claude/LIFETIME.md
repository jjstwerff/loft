# Lifetime — Dependency Tracking and Scope-Based Freeing

How the `dep` field on `Type::Text`, `Type::Reference`, `Type::Vector`, and other
heap-owning types interacts with scope exit to decide what gets freed.

---

## The dep field

Every heap-owning type carries a `Vec<u16>` dependency list:

```rust
Type::Text(Vec<u16>)                          // text buffer on the stack frame
Type::Reference(u32, Vec<u16>)                // store-allocated record
Type::Vector(Box<Type>, Vec<u16>)             // dynamic vector
Type::Enum(u32, bool, Vec<u16>)               // struct-enum variant (is_ref=true)
Type::Function(Vec<Type>, Box<Type>, Vec<u16>) // fn-ref with closure record
Type::Sorted(u32, .., Vec<u16>)               // sorted collection
Type::Index(u32, .., Vec<u16>)                // unique index
Type::Hash(u32, .., Vec<u16>)                 // hash table
Type::Spacial(u32, .., Vec<u16>)              // spatial index
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

## Summary: Text vs Reference vs Function freeing

| | Text (`Type::Text(dep)`) | Reference (`Type::Reference(d, dep)`) | Function (`Type::Function(p, r, dep)`) |
|---|---|---|---|
| **Storage** | Stack frame (`Str` = ptr + len) | Store heap (12-byte `DbRef`) | Stack frame (16B: 4B d_nr + 12B closure DbRef) |
| **dep list used for freeing?** | No — always freed | Yes — only freed when `dep.is_empty()` | Not yet — closure leak (see below) |
| **dep list purpose** | Type compatibility / format string tracking | Ownership tracking | Protects closure work var via `tp.depend()` |
| **Free opcode** | `OpFreeText` | `OpFreeRef` | None yet (closure DbRef at offset+4 not freed) |
| **skip_free flag** | Not checked | Checked | N/A |
| **Return-value exemption** | By `ret_var` identity only | By `ret_var` identity AND `tp.depend().contains(&v)` | Via `tp.depend()` on declared return type |

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

### Closure field reads now add `__closure` as dep — `objects.rs:94-97, 169-173`

Both closure field-read paths now add `self.closure_param` as a dependency after
reading the field type, matching the pattern for normal struct field access:

```rust
*code = self.get_field(closure_d_nr, fnr, Value::Var(self.closure_param));
t = self.data.attr_type(closure_d_nr, fnr);
t = t.depending(self.closure_param);   // A5.6-text: add __closure as dep
```

This produces `Type::Text([v___closure])` for a text field read from a closure,
matching the `Type::Text([v_p])` produced by `p.name` for a regular struct.  The
dep keeps the closure record alive while derived text is in use.

### The cross-scope crash: `___clos_N` freed before the fn-ref escapes

The cross-scope test (`closure_capture_text`) demonstrates the issue:

```loft
fn make_greeter(prefix: text) -> fn(text) -> text {
    fn(name: text) -> text { "{prefix} {name}" }
}
make_greeter("Hello")("world")
```

`___clos_1` has `Type::Reference(closure_d_nr, [])` — owned, empty deps.  At
function return, `get_free_vars` uses `tp = data.def(d_nr).returned` (the declared
return type).  Even though the fn-ref block's computed type carries `dep=[___clos_1]`,
the declared return type's deps are what `get_free_vars` sees.

### Implemented fix: deps on `Type::Function`

`Type::Function` now carries a `Vec<u16>` dep field:
```rust
Function(Vec<Type>, Box<Type>, Vec<u16>)  // params, return, deps
```

In `emit_lambda_code` (`src/parser/vectors.rs`), the fn-ref type is built with the
closure work variable as a dep: `Function(params, ret, vec![w])`.  Additionally,
`emit_lambda_code` propagates this dep to the enclosing function's declared return
type so that `free_vars` at the Return statement sees `tp.depend()` containing `w`.

Supporting changes (all implemented):
- `depending()`/`depend()`/`is_equal()` in `data.rs` handle Function deps
- `can_convert` in `mod.rs` accepts Text with different deps and Function with
  compatible params/return (ignoring deps)
- ~60 match sites updated from `Function(_, _)` to `Function(_, _, _)`

### Current status

**Same-scope closures**: all pass.  `___clos_N` and the fn-ref live in the same
function scope; `get_free_vars` doesn't run between closure allocation and fn-ref use.

**Cross-scope closures** (ignored test `closure_capture_text`): the declared return
type now carries the dep `[w]`, but `___clos_1` is still freed at function return.
The `free_vars` Return handler uses `self.scope` (the function body scope) as
`to_scope`, and `___clos_1` is registered at that scope (via work_refs init).  The
dep on the return type should suppress the free, but the variable is collected and
freed before the return type check can protect it.  This requires further
investigation into the scope/free ordering at function return boundaries.

### Remaining issue: closure record cleanup at the caller

The closure DbRef is embedded in the fn-ref's 16-byte slot (bytes 4-16).  At the
caller, the fn-ref variable has `Function(params, ret, [])` (empty deps — call
results are owned).  `get_free_vars` does not match `Type::Function` for freeing.
The 16-byte fn-ref slot is stack memory (reclaimed by `FreeStack`), but the closure
store record at offset+4 is never explicitly freed.

For chained calls (`make_greeter("Hello")("world")`), the leak is negligible.
For stored fn-refs, the closure record leaks until program exit.  Fixing this
requires `get_free_vars` to emit an `OpFreeRef` targeting the DbRef at offset+4
within the fn-ref slot — a separate enhancement.

### Same-scope closures: why they work

The passing tests (`closure_capture_text_return`, `closure_capture_text_loop`) use
same-scope closures where the closure is defined and called within one function:

```loft
greeting = "hello";
f = fn(name: text) -> text { "{greeting}, {name}!" };
f("world")
```

Here `___clos_N` and the fn-ref variable `f` live in the **same** function scope.
`___clos_N` is registered at the function body scope (via work_refs init).  The
fn_ref_with_closure block has its own inner scope, but `___clos_N` is not in that
scope.  At the inner block's scope exit, `___clos_N` is not collected.  At the outer
function scope exit, `___clos_N` is freed after the fn-ref call has completed.

---

## Why Function is different from Reference — and how to unify them

### The asymmetry

A `Type::Reference` variable is a 12-byte `DbRef` on the stack pointing to a store
record.  `get_free_vars` handles it directly: check deps, emit `OpFreeRef`, done.
The variable IS the store pointer — freeing it means freeing what it points to.

A `Type::Function` variable is a 16-byte fn-ref on the stack: `[d_nr (4B)][closure
DbRef (12B)]`.  The variable is **not** a store pointer — it's a compound value that
**contains** a store pointer at offset+4.  `OpFreeRef` reads a 12-byte DbRef from the
stack, so it cannot be pointed at a Function variable directly.

This is why Function is excluded from the `get_free_vars` Reference/Vector/Enum match:
emitting `OpFreeRef(fn_var)` would read 12 bytes starting at the fn-ref's base — the
d_nr (4B) plus the first 8 bytes of the DbRef — producing a garbage DbRef.

### The two-variable design

Currently, a capturing lambda produces two variables:

| Variable | Type | Size | Owns the store record? |
|----------|------|------|----------------------|
| `___clos_N` | `Reference(closure_d_nr, [])` | 12B | Yes — standard owned ref |
| fn-ref var | `Function(params, ret, [___clos_N])` | 16B | No — borrows from `___clos_N` |

The fn-ref embeds a **copy** of the closure DbRef (bytes 4-16 match `___clos_N`'s
DbRef), but the dep `[___clos_N]` marks the fn-ref as a borrower.  Freeing is
delegated to `___clos_N` via the normal Reference path.

This design works for same-scope closures but fails when the fn-ref escapes via return:
`___clos_N` is freed at the defining function's scope exit because the declared return
type's `tp.depend()` check cannot always reach it in time.

### How Function could be treated like Reference

The fundamental issue is that the closure DbRef lives at a non-zero offset within the
fn-ref slot.  Two approaches to unify Function with Reference freeing:

#### Approach A: codegen special case for `OpFreeRef` on Function variables

Add Function to the `get_free_vars` match alongside Reference/Vector/Enum.  In
codegen (`generate_call`), when `OpFreeRef` is called on a `Type::Function` variable,
emit `OpVarRef(var_pos - 4)` instead of the normal `generate_var` which would emit
`OpVarFnRef` (16 bytes):

```rust
// In generate_call, after the skip_free check:
if stack.data.def(op).name == "OpFreeRef"
    && let Some(Value::Var(v)) = parameters.first()
    && matches!(stack.function.tp(*v), Type::Function(_, _, _))
{
    let var_pos = stack.position - stack.function.stack(*v);
    stack.add_op("OpVarRef", self);     // push 12B DbRef
    self.code_add(var_pos - 4u16);      // read from offset+4 (skip d_nr)
    stack.add_op("OpFreeRef", self);    // pop and free the DbRef
    return Type::Void;
}
```

`OpVarRef(var_pos - 4)` reads 12 bytes starting 4 bytes into the fn-ref slot —
exactly the closure DbRef.  For non-capturing lambdas, this reads the
`OpNullRefSentinel` (store_nr = u16::MAX), and `database.free()` is a no-op for null
sentinels.

**Advantage**: minimal change — Function joins the existing free logic, codegen handles
the offset.

**Complication**: `dep.is_empty()` means "owned, should free" for Reference, but for
Function variables assigned from expressions (e.g. `f = fn(...) {...}`), the variable
type often has `dep=[]` even when the fn-ref block's computed type had `dep=[w]`.
Variable types are set at registration time and may not reflect expression deps.  This
caused false frees in testing — all closure tests broke because non-owning fn-ref
variables were treated as owners.

#### Approach B: eliminate `___clos_N` — store closure as a standalone Reference

Instead of two variables (work ref + fn-ref), make the fn-ref variable carry only the
d_nr (4B) and store the closure record in a separate Reference variable that follows
the standard lifetime rules.  At call sites, push both the d_nr and the closure DbRef
from their respective variables.

This would make closures fully standard: the closure is a Reference with normal
dep/free semantics, and the d_nr is a plain integer.  No special-case codegen needed.

**Advantage**: completely uniform — no special offset logic, standard Reference freeing.

**Disadvantage**: changes the fn-ref slot layout (currently 16B compound), call site
codegen, and how `CallRef` reads the closure — a larger refactor.

#### Approach C: make the fn-ref variable a Reference to a record containing both d_nr and closure

Allocate a store record with fields `[d_nr: integer, closure: Reference]`.  The fn-ref
variable becomes a standard `Type::Reference` to this record.  Standard Reference
freeing deallocates the record, which in turn should cascade-free the closure.

**Advantage**: fn-ref becomes a plain Reference — fully uniform freeing.

**Disadvantage**: adds a store allocation per fn-ref (overhead), requires cascade-free
for nested references (not currently implemented), and changes the calling convention.

### Brittleness evaluation

The core tension: opcodes work on fixed sizes (4B int, 12B ref, 16B fn-ref).  The
16-byte fn-ref is the only compound stack slot — two values packed together.  Any
approach that requires special-case offset logic for this compound is inherently
brittle because it breaks the invariant that each stack slot is one typed value.

**Approach A is the most brittle**.  `OpVarRef(var_pos - 4)` hardcodes knowledge that
the DbRef starts 4 bytes into the fn-ref slot.  If the fn-ref layout ever changes
(different d_nr size, additional metadata), every offset calculation breaks silently.
It also requires a codegen special case that no other type needs, making the free
path non-uniform.  The dep propagation issue (variable types not reflecting expression
deps) adds another fragile layer — if any code path creates a Function variable
without proper deps, the free logic misbehaves.

**Approach C (store record containing both d_nr and closure) is also brittle** because
it requires cascade-free for nested references — freeing the wrapper record should
also free the closure record inside it.  Cascade-free is not implemented and is a
significant addition to the store allocator.

**Approach B (separate variables) is the least brittle** because it eliminates the
compound slot entirely.  Each value uses standard-sized opcodes with no offset tricks:

| Value | Type | Size | Freed by |
|-------|------|------|----------|
| d_nr | `Integer` | 4B | Nothing (integer, no heap) |
| closure | `Reference(closure_d_nr, [])` | 12B | Standard `OpFreeRef` |

The closure record is a plain Reference that participates in the normal dep/free
lifecycle.  No special codegen, no offset calculations, no dep propagation worries.

### Approach B — concrete design

**At definition time** (`emit_lambda_code`):
- Allocate closure record into `___clos_N` (Reference) — already happens today
- Build fn-ref as `Value::FnRef(d_nr, ___clos_N)` — already happens today
- Variable type: instead of `Function(params, ret, [w])` (16B compound), store two
  pieces: the d_nr in the Function type itself (it already has the d_nr in the FnRef
  IR node), and the closure in the separate Reference variable

**At the call site** (`fn_call_ref`):
- Currently reads `d_nr = get_var::<i32>(fn_var)` and `closure = get_var::<DbRef>(fn_var - 4)`
  from the compound 16B slot
- With Approach B: read d_nr from one stack position, closure DbRef from another
- `OpCallRef` bytecode already takes `fn_var` (distance to fn-ref) and `arg_size`;
  it could read the closure from the argument list instead of the fn-ref slot

**The key insight from `fn_call_ref`**: it already treats the 16B slot as two separate
reads (`get_var::<i32>` then `get_var::<DbRef>`).  The compound slot is a parsing-time
convenience, not a runtime necessity.  At runtime, `fn_call_ref` can just as easily
read d_nr from one variable and closure from another — or from the argument stack
where the closure is already pushed.

**Freeing**: `___clos_N` is a standard Reference.  For same-scope closures, it's freed
at scope exit by the normal Reference path.  For cross-scope closures (returned fn-ref),
the dep system protects it: the fn-ref expression type carries `dep=[___clos_N]`, and
`tp.depend().contains(___clos_N)` suppresses the free.  At the caller, the closure
travels as a regular function argument — `__closure` is a Reference parameter that the
callee receives and the caller's scope frees when done.

**Non-capturing lambdas**: d_nr is just an integer constant.  No closure variable
exists, no Reference to free.  The fn-ref is `Value::Int(d_nr)` — 4 bytes, same as
today minus the 12-byte null sentinel padding.

### Migration path from the current 16B layout to Approach B

The 16B fn-ref layout is used in:
1. `OpVarFnRef` / `OpPutFnRef` — push/store 16 bytes (`[i32; 4]`)
2. `OpCallRef` / `fn_call_ref` — reads d_nr at offset 0, closure at offset+4
3. `gen_fn_ref_value` — ensures every if-else branch fills a full 16B slot
4. `OpNullRefSentinel` — pads non-capturing lambdas to 16B

With Approach B, all four simplify:
1. fn-ref variable is 4B integer (d_nr) → use `OpVarInt` / `OpPutInt`
2. `OpCallRef` reads d_nr from the fn-ref variable, closure from the argument list
   (already pushed by the codegen as the last hidden arg)
3. No 16B padding needed — if-else branches produce matching 4B integers
4. No null sentinel needed — non-capturing lambdas have no closure argument

The closure DbRef is pushed as a regular argument at the call site, just like it was
before A5.6 embedded it in the fn-ref slot.  The difference: with Approach B, the
closure is a proper Reference variable with standard lifetime, not bytes embedded in
a compound slot.

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
