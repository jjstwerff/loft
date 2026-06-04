---
render_with_liquid: false
---
# @PLAN21 — Retiring `stores.scratch`

**Status: ACTIVE 2026-06-04 — `@PLN10` ([loft-lang/plans#10](https://github.com/loft-lang/plans/issues/10)).**
Promoted from `future/`.  Material update since this doc was written: **@P227**
(the §5.2 Phase C blocker — text-returning fn-ref crashes) **closed 2026-05-05**,
and its fix shipped exactly the `RefVar(Text)` work-buffer threading that Phase C's
fn-ref piece needs.  So Phase C is far more unblocked than §5 states; the blocker
analysis wants a re-audit.  **First slice: Phase A** (mechanical, ~95% of scratch
traffic by call volume, independently shippable, removes the in-statement-growth
+ re-entrancy hazards).

Design note for retiring the `Vec<String>` lifetime-extension buffer that
backs `Str` returns from native and code-generated text producers.

## TL;DR

`stores.scratch` is a per-`Stores` `Vec<String>` (`src/database/mod.rs:182`)
that exists solely to hold owned `String`s long enough for the caller to
read the `Str { ptr, len }` view it published.  It is drained by
`OpClearScratch` at every statement boundary (`src/state/codegen.rs:295`
emits the clear into every `Value::Line`).

The buffer was a pragmatic stop-gap when text-returning natives were
introduced.  The lifetime model has matured since then: `text_return` /
`ref_return` thread caller-owned work buffers as hidden `RefVar(_)`
arguments (`src/parser/control.rs:2405`, `2492`), and the partial
destination-passing pattern (`*_dest` natives, dispatched by
`is_text_dest_native` in `src/state/codegen.rs:47`) bypasses scratch
entirely for the `OpAppendText` shape.

The remaining producers are now the only reason `scratch` exists.  They
can be migrated incrementally to the work-buffer / destination-passing
machinery that already covers user-defined text-returning functions, at
which point `scratch` and `OpClearScratch` can be deleted.

This document is the design.

---

## 1. What `stores.scratch` is today

### Field

```rust
// src/database/mod.rs:180
/// Temporary strings produced by text-returning native functions.
/// Cleared by `OpClearScratch` at statement boundaries.
pub scratch: Vec<String>,
```

### How a value reaches scratch and how it lives

Producers follow one of two shapes:

```rust
// "always-clear-then-push" — single-slot pattern, e.g. n_kind, i_parse_errors,
// n_json_errors, n_as_text.  Whoever wrote this assumed exactly one live
// scratch reader at a time.
stores.scratch.clear();
stores.scratch.push(s_owned);
stores.put(stack, Str::new(&stores.scratch[0]));
```

```rust
// "push-and-trust-the-line-clear" — additive, e.g. t_4text_replace,
// t_4text_to_lowercase, t_4text_to_uppercase, n_to_json, n_to_json_pretty,
// n_sha256, n_hmac_sha256, n_source_dir, n_parallel_buf_get_text,
// n_parallel_buf_get_text_native, and the codegen wrap sites.
stores.scratch.push(s_owned);
let s = Str::new(stores.scratch.last().unwrap());
stores.put(stack, s);
```

`Str::new(v)` (`src/keys.rs:29`) captures `v.as_ptr()` — the pointer to
the *heap buffer* of the underlying `String`, not into the `Vec`'s slot
array.  Vec reallocation moves the `String` struct (24 bytes) but not its
heap buffer, so `Str` survives `Vec::push` reallocation.  But:

- `scratch.clear()` drops every `String`, freeing all heap buffers.
  Any `Str` still pointing into scratch dangles after this point.
- A `String` that calls `push_str` and reallocates moves *its* heap
  buffer; the always-clear-then-push pattern relies on never doing this
  to the live element.

### When scratch is cleared

`OpClearScratch` is emitted into every `Value::Line` IR node:

```rust
// src/state/codegen.rs:293
Value::Line(line) => {
    self.line_numbers.insert(self.code_pos, *line);
    if let Some(&lib_nr) = self.library_names.get("OpClearScratch") {
        stack.add_op("OpStaticCall", self);
        self.code_add(lib_nr);
    }
    Type::Void
}
```

Bytecode body in `src/fill.rs:1897`:

```rust
fn clear_scratch(s: &mut State) {
    s.database.scratch.clear();
}
```

So the lifetime contract is: **a `Str` whose backing `String` lives in
`scratch` is valid only until the next `Value::Line`.**

### Producer inventory (current)

In `src/native.rs`:

| Function | Pattern | Call shape |
|---|---|---|
| `t_4text_replace` | push-and-trust | `s.replace(a, b)` |
| `t_4text_to_lowercase` | push-and-trust | `s.to_lowercase()` |
| `t_4text_to_uppercase` | push-and-trust | `s.to_uppercase()` |
| `n_to_json` | push-and-trust | `j.to_json()` |
| `n_to_json_pretty` | push-and-trust | `j.to_json_pretty()` |
| `n_kind` | clear-then-push | `j.kind` |
| `n_as_text` | clear-then-push | `j.as_text` |
| `n_source_dir` | push-and-trust | `source_dir()` |
| `n_sha256` | push-and-trust | `sha256(b)` |
| `n_hmac_sha256` | push-and-trust | `hmac_sha256(k, m)` |
| `n_json_errors` | clear-then-push | `json_errors()` |
| `i_parse_errors` | clear-then-push | `Type#errors` |
| `n_parallel_buf_get_text` | push-and-trust | parallel text-buf get |

In `src/codegen_runtime.rs` (native backend):

| Function | Pattern |
|---|---|
| `i_parse_errors` (`:353`) | clear-then-push |
| `i_json_errors` (`:371`) | clear-then-push |
| `n_parallel_buf_get_text_native` (`:2515`) | push-and-trust |

In `src/generation/emit.rs` (codegen wrap sites):

| Site | Why scratch |
|---|---|
| `Value::Return` text-wrap (`:228`) | @P205 — bounded-generic specialisation has no `RefVar(Text)` work buffer; emit wraps in `stores.scratch.push((expr).to_string()); Str::new(...)` |
| Block-tail `wrap_result` (`:1034`) | Same detection, same routing |

In `src/extensions.rs` (cdylib LoftStr → loft `Str` bridge):

```rust
// src/extensions.rs:680
fn push_loft_str(stores, stack, s: LoftStr) {
    if !s.ptr.is_null() && s.len > 0 {
        let text = ...;
        stores.scratch.clear();
        stores.scratch.push(text.to_string());
        stores.put(stack, Str::new(&stores.scratch[0]));
    } ...
}
```

Reverse uses (consumers that read scratch and clone out, then are done) —
none.  Every consumer reads via `Str` and the `Str` outlives the read by
construction.

---

## 2. Why this is a long-running-program hazard

### 2.1  In-statement unbounded growth

`OpClearScratch` fires at line boundaries.  Within a single statement,
every text-returning native call appends and never pops.  Worst-case
shapes that already exist in real loft code:

- A comprehension that builds many texts: each per-element call goes
  through scratch until the comprehension's containing line ends.
- A long format chain or fold whose inner expression `to_lowercase`s,
  `replace`s, or `to_json`s on each step.
- A fn-ref dispatch table whose arms each return text via scratch and
  whose call sites are inside one expression.

Capacity of `Vec<String>` only grows, never shrinks (`clear` keeps
capacity).  In a server / event loop the high-water mark of the worst
single statement the program ever runs is the steady-state RSS cost of
`scratch` for the rest of the process.

This is a *bounded* leak (bounded by the worst statement), but the
bound is set by the most pathological line, not by the steady-state
working set, and is invisible to the programmer.

### 2.2  Cross-statement escape (@P227 family)

The clear-on-line contract assumes nothing reads a scratch-backed `Str`
across a line boundary.  This breaks whenever a value derived from a
scratch `Str` survives the next `Value::Line`:

- **Text-returning fn-ref calls** (`PROBLEMS.md` @P227, open at S1).  The
  dispatch wrapper synthesises a `Str` whose backing `String` is in
  scratch (or a stack-local), and the `Str` outlives the dispatch arm.
  Native panics inside `<Str as Display>::fmt`'s `String::push_str`;
  interpreter SIGSEGVs.
- **Closures** that capture text by `Str`.  Today the work-around is to
  copy text into store records before capture; without that, the
  captured `Str` dangles on the next line.
- **Server callbacks / event-loop handlers** registered with a `Str`
  argument — the registration outlives every line, so any scratch-backed
  text passed in is freed on the next iteration.
- **Coroutines** holding text across a yield (see
  `COROUTINE.md` SC-CO-8 and the dynamic-string side-table notes).

Each new deferred-execution feature multiplies these surfaces.  The
`scratch` model has no way to extend a single value's lifetime; the
only granularity is "clear the whole thing."

### 2.3  Steady-state bloat is invisible

`scratch.clear()` keeps `Vec<String>` capacity.  After capacity grows,
the slot array is permanent.  The `String` heap buffers are dropped on
clear, so the bloat is only the slot array (24 bytes per slot), but it
is invisible in any RSS metric the user looks at.  In practice this is
small; the real concern is 2.1 and 2.2.

### 2.4  WASM and threading

`scratch` lives on `Stores`.  Each parallel worker has its own `Stores`,
so worker-side scratch is fine.  The hazard is at the parent: when a
worker text result is read via `n_parallel_buf_get_text_native`
(`codegen_runtime.rs:2515`), it `clone`s out of the per-call buffer and
pushes into the parent's scratch.  The `Str` returned to the loft caller
is then clear-on-next-line.  Holding the value past the loop body's
last line is the same hazard as 2.2.

### 2.5  Re-entrancy under always-clear-then-push

The clear-then-push producers (`n_kind`, `n_as_text`, `n_json_errors`,
`i_parse_errors`) clear *all* scratch entries before pushing their
own.  Any `Str` from an earlier push-and-trust producer in the same
expression dangles immediately.

Concrete shape that breaks:
```loft
let s = format!("{} {}", val.kind, other.to_lowercase())
```
If `to_lowercase` ran first via push-and-trust, then `val.kind` ran
clear-then-push, the lowercase `Str` is invalidated mid-expression.
This has not been observed in the wild (the format-call evaluation
order keeps these separate in current programs) but the invariant is
fragile and there is no static check.

---

## 3. Existing mechanisms that bypass scratch

The retirement plan reuses three mechanisms that already work for
user-defined text-returning functions and a subset of natives.

### 3.1  Destination-passing for natives — `*_dest` variants

`src/state/codegen.rs:46`:

```rust
fn is_text_dest_native(name: &str) -> bool {
    matches!(
        name,
        "t_4text_replace" | "t_4text_to_lowercase" | "t_4text_to_uppercase"
    )
}
```

For each of these the codegen has a `_dest` companion in `native.rs`
(`t_4text_replace_dest:448`, `t_4text_to_lowercase_dest:468`,
`t_4text_to_uppercase_dest:486`):

```rust
fn t_4text_to_lowercase_dest(stores: &mut Stores, stack: &mut DbRef) {
    let dest = *stores.get::<DbRef>(stack);
    let v_self = *stores.get::<Str>(stack);
    let new_value = v_self.str().to_lowercase();
    stores
        .store_mut(&dest)
        .addr_mut::<String>(dest.rec, dest.pos)
        .push_str(&new_value);
}
```

The `_dest` form is dispatched by `try_text_dest_pass` in
`src/state/codegen.rs:1845`, which only fires inside an
`OpAppendText` (i.e. `out += x.to_lowercase()`).  Outside that pattern,
the non-`_dest` form runs and pushes into scratch.

**Generalising**: every text-producing native gets a `_dest` variant;
codegen routes to it whenever a destination buffer is reachable
(text-typed assignment target, struct-field write, RefVar(Text)
work-buffer in scope).

### 3.2  `text_return` — hidden RefVar(Text) work buffer for user fns

`src/parser/control.rs:2405`:

```rust
pub(crate) fn text_return(&mut self, ls: &[u16]) {
    if let Type::Text(cur) = &self.data.definitions[self.context as usize].returned {
        ...
        if matches!(tp, Type::Text(_)) {
            // create a new attribute with this name
            let a = self.data.add_attribute(
                &mut self.lexer,
                self.context,
                n,
                Type::RefVar(Box::new(Type::Text(Vec::new()))),
            );
            self.vars.become_argument(*v);
            ...
```

Effect: a user-fn `fn label(x: Foo) -> text { ... }` whose body returns
a local `text` variable becomes
`fn label(x: Foo, __work_1: &mut text)`.  The caller pre-allocates the
`__work_1` slot in its own store; the callee writes via `OpAppendText`
into `__work_1`; the returned `Str` views that caller-owned slot.  The
work buffer's lifetime is the caller's stack frame, which is exactly
what the borrow needs.

Generic specialisations don't hit this path
(`PROBLEMS.md` @P205 closed by routing through scratch instead — `:896`).
That fallback is one of the targets of the retirement plan.

### 3.3  `ref_return` — same shape for struct/vector returns

`src/parser/control.rs:2492` does the same thing for `Reference(_, _)`
and vector returns: hidden `RefVar(_)` arg, caller-allocated, callee
writes into it.  No scratch involvement; this is included only for
context — it shows the pattern generalises beyond text.

---

## 4. Path to retirement

Three phases, each independently shippable and individually a net
reduction in scratch traffic.

### Phase A — generalise destination-passing to every text-producing native

**Targets**: every entry in §1's producer inventory under `src/native.rs`
and `src/codegen_runtime.rs`.  Excludes the `extensions.rs` cdylib
bridge (Phase A.5) and the `emit.rs` codegen wrap sites (Phase B).

**Steps per native**:

1. Add `<fn>_dest(stores, stack, dest_ref)` variant.  Body: same as
   today, but `push_str` into `*dest_ref` instead of allocating a
   `String` and pushing it.  Mirror the existing
   `t_4text_to_lowercase_dest` shape at `src/native.rs:468`.
2. Register it in the native table next to the non-`_dest` form.
3. Extend `is_text_dest_native` (`src/state/codegen.rs:47`) to include
   the new name.

**Codegen extension**: today `try_text_dest_pass`
(`src/state/codegen.rs:1845`) only fires inside `OpAppendText`.  Extend
the call shape to:

- `let s = native(...)` — synthesise the destination from `s`'s store
  slot (declare it as `text` in a per-statement temp record if `s`
  isn't already a text-typed local).  This needs the same machinery
  `text_return` uses for user fns: at parse time, when the assignment
  RHS is a text-returning native call and the LHS is text-typed,
  retarget the call's destination to LHS's RefVar(Text).
- `field = native(...)` — similar, but with the field's RefVar(Text)
  as destination.
- `print(native(...))` / argument position — synthesise a hidden
  per-call temp `RefVar(Text)` slot in the current scope, dropped on
  scope exit by the existing `OpFreeText` machinery.  This is the
  fallback for "no obvious destination."

Once Phase A lands, scratch traffic for `t_4text_*`, `n_to_json*`,
`n_sha256`, `n_hmac_sha256`, `n_source_dir`, the parallel-buf-get
producers, and the JSON / parse-error introspection producers all
disappears.

**Risk**: the temp-slot synthesis is the new code.  It is structurally
the same as `text_return`'s hidden-arg machinery — same store, same
`OpFreeText` on scope exit, same `RefVar(Text)` typing.  The novelty
is that the producer is a native call rather than a user-fn return.

**Estimated coverage after Phase A**: ~95% of scratch pushes by call
volume.  Remaining producers: `extensions.rs` LoftStr bridge
(Phase A.5) and the two `emit.rs` wrap sites (Phase B).

### Phase A.5 — cdylib LoftStr bridge

`src/extensions.rs:682`'s `push_loft_str` helper needs a destination
parameter.  Same shape as Phase A.  This is small but separable
because it sits on the foreign-function boundary; the marshalling
generator (`extensions.rs`'s LoftStr decode path) needs the
destination threaded through.

### Phase B — codegen wrap sites

`src/generation/emit.rs:228` and `:1034` wrap text returns from
generic-specialisation paths in scratch.  The fix is to thread a
RefVar(Text) work buffer through the generic specialisation, the same
way `text_return` does for non-generic user fns.

Investigation in @P205 (closed, `PROBLEMS.md:896`) found that
`text_return` skips generics at `src/parser/control.rs:375`
(`DefType::Generic` skip).  Outcome B in that probe found that
removing the skip alone doesn't help — generic specialisations have
no local text variables to promote.  The work in Phase B is to:

1. Detect the generic-specialisation case at codegen time (the
   `needs_p205_scratch` predicate already does this — see
   `CHANGELOG_TECHNICAL.md:33`).
2. Allocate a caller-side RefVar(Text) at the call site for the
   specialised generic, and emit the inner `Str::new(&work_buf)`
   instead of the scratch wrap.
3. Free the work buf on scope exit via existing `OpFreeText`.

The `p205_no_str_new_of_local_in_corpus` regression test
(`tests/codegen_emitter.rs`) needs to be updated: it currently asserts
the absence of `Str::new(&var___ret_*)`.  After Phase B the inverse
holds — `Str::new(&work_buf_*)` is the *correct* shape.

@P208 (closed via wrap suppression, `PROBLEMS.md:42`) is in the same
emit path; same Phase B fix.

### Phase C — closure / fn-ref text return + parallel buf direct read

This is the prerequisite for @P227 closure (S1, open).  Two pieces:

1. **Fn-ref text return**: the dispatcher today produces a `Str`
   whose backing String is scratch or a stack local.  Fix is the
   same RefVar(Text) work buffer threading, but for fn-ref dispatch
   tables: each candidate gets a `__work_*` parameter, the dispatch
   wrapper passes the caller's work buffer through.  `output_call_ref`
   in `src/generation/emit.rs` already filters `__work_*`/`__closure`
   attrs (per the partial @P227 progress note); the missing piece is
   making the work buffer actually flow.
2. **Parallel buf direct read**: `n_parallel_buf_get_text_native`
   (`codegen_runtime.rs:2515`) currently clones into scratch.  The
   per-call `Vec<String>` already lives on `par_text_buffer_stack`
   and stays alive until `n_parallel_buf_drop_text_native` fires.
   Have the get function return `Str::new(&buf[idx])` directly — no
   clone, no scratch — and rely on the drop fence to bound the
   lifetime.  This shrinks the work to ABI-only changes and removes
   the last per-row text allocation.

After Phase C the producer set is empty.  Delete:

- `Stores::scratch` field (`src/database/mod.rs:182`)
- `OpClearScratch` opcode and its emission (`src/state/codegen.rs:295`,
  `src/fill.rs:1897`, `default/01_code.loft` declaration)
- `is_text_dest_native` becomes obsolete — or rather, becomes "every
  text-producing native" by default (the dispatch is unconditional).

---

## 5. What blocks doing it now

### 5.1  Parser machinery

Phase A's destination synthesis for arbitrary text-returning native
expressions is new code.  It is structurally analogous to
`text_return` but applies at the *call expression* rather than the
*function definition*.  Concretely the parser must:

- At `let s = native(...)` parse time, detect text return and rewrite
  the call to pass `s`'s slot as a RefVar(Text) hidden first arg.
- At inner positions (`print(native(...))`, `format!("...{native()}...")`,
  `if cond { native_a() } else { native_b() }`), synthesise a per-call
  temp slot in the current scope and pass it.
- For fn-call-of-fn-call chains (`a.to_lowercase().replace(x, y)`),
  thread the outer call's destination through to the inner one only
  when safe (no aliasing through the source).

The `text_return` work-buffer logic already handles the outer
shape (for user fns); the call-expression shape is the same problem
shifted one level.

### 5.2  @P227 must close in parallel

Phases A and B remove most producers but leave the dispatch / closure
text path as the last consumer of scratch.  Without @P227's fn-ref text
fix in Phase C, retiring scratch in Phases A/B alone shifts the
dangling-pointer hazard from "scratch-backed Str across a line" to
"stack-local-backed Str across a dispatch return."  Net safety
improves (most cases fixed) but the residual hazard is sharper.

### 5.3  No ABI churn at the loft-stack level

The 16-byte `Str` on the loft stack stays.  Native functions still
write `Str { ptr, len }` into stack slots; what changes is *where*
the backing `String` lives — caller-owned RefVar(Text) instead of
`stores.scratch`.  This means no cross-cutting stack-layout
re-engineering is required.  The migration is per-producer and
per-call-site.

### 5.4  Test corpus

Phase A introduces new natives (`*_dest` variants).  The existing
gold-output tests
(`tests/codegen_emitter.rs::p205_no_str_new_of_local_in_corpus`,
`tests/dumps/`) baseline the codegen output and will diff on every
phase.  Plan to refresh the baselines once per phase, not per
individual native rewrite, to keep the diff reviewable.

---

## 6. Sequencing recommendation

Phase A is the right first step.  It is:

- Mechanical (one new fn per producer, one extension to
  `is_text_dest_native`, one extension to `try_text_dest_pass`'s
  detection).
- Fully back-compat (the non-`_dest` natives stay registered for the
  fallback path until Phase C deletes scratch).
- Independent of @P227 (no closure / fn-ref work).
- Removes the in-statement unbounded growth hazard (§2.1) and the
  re-entrancy hazard (§2.5) for ~95% of producers.

Phase B is the next step because it removes the codegen wrap sites
that currently stand in for the missing generic-specialisation
work-buffer.  This is the same fix shape as Phase A's parser
extension, applied to the generic path.

Phase C is the last step and depends on @P227.  Until @P227 closes the
fn-ref dispatch text path, retiring scratch globally would regress
those programs.  Plan-level work: Phase C's two pieces (fn-ref text
work buffer, parallel buf direct read) bundle naturally with the @P227
closure work; do them together.

After Phase C the field, the opcode, and the `Value::Line` injection
all delete.  The diagnostic message in `PROBLEMS.md:896` ("routes
through `stores.scratch`") becomes a closed-by-design entry.

---

## 7. Reading order for implementing

1. `src/database/mod.rs:175-225` — the field and the surrounding
   buffer-stack machinery (parallel buffers are sibling stacks; same
   shape, different lifetimes).
2. `src/keys.rs:21-67` — `Str` semantics; understand why the heap
   buffer is stable across `Vec` reallocation but dies on `clear()`.
3. `src/state/codegen.rs:46-52` and `:1843-1884` —
   `is_text_dest_native` and `try_text_dest_pass`, the existing
   destination-passing path.  Phase A extends both.
4. `src/parser/control.rs:2405-2446` — `text_return`, the user-fn
   work-buffer machinery Phase A adapts.
5. `src/native.rs:438-484` — the `replace / to_lowercase / to_uppercase`
   pair-of-variants pattern Phase A replicates for every other text
   producer.
6. `src/generation/emit.rs:215-245` and `:1020-1050` — the codegen
   wrap sites Phase B replaces with work-buffer threading.
7. `doc/claude/PROBLEMS.md` @P205 (`:896`) and @P227 (`:61`) —
   constraints on Phase B and Phase C respectively.
8. `doc/claude/CHANGELOG_TECHNICAL.md:12-80` — the @P205 close note,
   which explains the `needs_p205_scratch` predicate Phase B reuses.

---

## 8. Out of scope

- The non-text uses of `OpClearScratch`-adjacent buffers
  (`par_buffer_stack`, `par_text_buffer_stack`, `par_ref_buffer_stack`
  on `Stores`).  Those have explicit push/drop fences; they are not
  affected by `scratch` retirement.
- `last_parse_errors` and `last_json_errors` (`Stores` fields used by
  `i_parse_errors` / `n_json_errors`).  Phase A migrates the *Str
  return path* of these introspectors to destination-passing; the
  underlying error vectors and their `clear()`-on-success semantics
  stay.
- Coroutine yield serialisation (`COROUTINE.md` SC-CO-8) — uses a
  separate side table.  No interaction with `scratch`.
