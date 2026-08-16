---
render_with_liquid: false
---
# Changelog

All notable changes to the loft language and interpreter.

---

## [Unreleased]

### A coroutine's yielded record is borrowed, not owned (loft#920 partial, 2026-08-16)

`collect_iterator_subject` materialises a `match <iterator<T>>` subject by pulling the
coroutine into a buffer.  The pulled value lands in `stream_x`, typed with the ELEMENT type.
For a **struct-enum** element that makes it a record ref — and the ref points into the
coroutine's own frame, which lives in the STACK store.  The append below deep-copies it
(`OpCopyRecord`) so the buffer owns its copy, but nothing said `stream_x` does not, and the
loop-scope exit emitted `OpFreeRef(_stream_x_1)` on every pull:

```
loop {#stream pull_3
  _stream_x_1(3):ref(Tok) = OpCoroutineNext(_stream_gen_1(2), 12i32);
  …
    OpCopyRecord(_stream_x_1(3), OpGetField(_stream_elm_1(3), 0i32, 78i32), 78i32);
  OpFreeRef(_stream_x_1(3));          <-- whole-store free of a stack-record ref
}
```

Only the store-0 guard kept that from taking every live frame with it; what reached the user
was `BUG (#306)`.  The sibling `stream_elm` was already marked `skip_free` for the identical
reason one line below, with a comment saying so.

`skip_free` is ONE bit for every free kind, so the mark is gated on the record case: a `text`
element's `stream_x` holds a String the caller DOES own (`OpFreeText`), and marking it would
trade the wrong free for a leak of one string per yield.  A scalar emits no free at all.
Verified both directions — the leak detector fires on a known-leaking control and stays
silent here, on both backends.

**The previous attribution was wrong, and the instrument is why.**  loft#920 was recorded
against `tests/scripts/75-native-stub.loft`; the refusals actually came from
`35p-iterator-match.loft`, which reproduces **alone**, on `--interpret`, with no harness and
no `catch_unwind` — so the "mid-execution unwind leaves a live state" theory it was filed
under had nothing to do with it.  The message carried a bare `pc=`, which is a position in
the WHOLE bytecode stream (stdlib included) while `introspect` prints only the user file's,
so no reader could resolve it.  It now names `file:line:col` (via the published span table)
AND the dispatching opcode — the span map is sparse and can point several statements away,
the op is exact, so both are printed.  That pair turned three failed attempts into one
reduction.

**And the gate that let it live.**  `35p-iterator-match.loft` raised this on every suite run
for as long as it existed and scored PASS each time: the refusal keeps the store alive, so
nothing the script asserts can notice, and the report went to stderr where nothing read it.
`Stores::free_named` now counts refusals unconditionally (`keys::stack_free_refusals`) and
`tests/wrap.rs` fails any script that raises one.  Corpus run: 3 refusals before, 0 after,
and no other script trips it.

**Not closed.**  The nightly poison gate's SIGSEGV survives this fix — `OpLengthVector`,
reached during `917-nullable-collection-field-says-so.loft`, which is clean standalone.  That
is latent corruption from an earlier script surfacing there, a second mechanism, and it does
not reproduce under `LOFT_STRICT_STORES=1` (strict mode changes free/reuse and masks it).

### A tuple's record shape does not depend on how its type was reached (loft#943, 2026-08-16)

Two defects, one report, because a vector literal of tuples needs both fixed.

**1. An inferred tuple element type registered no `__tuple<…>` struct.**  A tuple is stored as
that synthetic struct, and every consumer needing its record shape — `type_def_nr`,
`type_elm`, `fill_database` — resolves it BY NAME.  Registration happened only where a tuple
appears in a TYPE position (`sub_type`, `parse_type_full`, the `__retbuf` rewrite), which is
every DECLARED spelling and no inferred one: `v = [(7, 8)]` names no type anywhere.
`type_def_nr` answered `u32::MAX` and `new_record` refused the literal outright with *"cannot
build this record — its type never resolved"*.

`Data::vector_def` now registers from the element type, which is the point that already needs
it — the vector def's own `parent` is `type_def_nr(tp)`, and it was being stored as `u32::MAX`
beside the refusal.  `ensure_tuple_defs` recurses inside-out (a nested tuple's members are
sized from their own defs) and SKIPS a tuple with an `Unknown` member: a forward-declared
member resolves only in pass 2, and registering the pass-1 shape would mint a second
`__tuple<…,unknown>` beside the real one.  `ensure_tuple_defs_for_capture` (P216, the same
walker written for closure captures) now delegates to it rather than keeping a second list.

**The filed scope was the tuple LITERAL; the axis is inference.**  `t = (7, 8); v = [t]`
builds its element from a tuple LOCAL and failed identically — c5 in the guard.

**2. A struct-literal tuple member kept its `Rewritten` marker.**  `Rewritten(Reference(S))`
says a value was built in place (#319) — a signal to the expression that parsed it, not a type
a member can HAVE.  The tuple literal (`vectors.rs:409`) recorded it verbatim in the member
list, so every consumer matching on the type constructor missed it:

| site | what the user saw |
|---|---|
| `set_field` | `Cannot assign to field '_0' of type S` |
| `get_val` | `Field access not supported on type S` |
| `emit_tuple_put_ops` | `internal compiler error — unsupported elem Rewritten(Reference(707, …))` |

That last row needs no vector at all: a bare `t = (S { … }, k)` was an ICE on released
2026.8.0.  The wrapper is now peeled at the producer through the new `Type::unrewritten()`,
which is the same move `parse_vector` and `parse_vector_for` already make for a vector's
ELEMENT type — a tuple member is that fact one level in.  A struct member reached through a
LOCAL or a CALL was always fine, which is what named the literal as the axis.

Guard: `tests/scripts/943-inferred-tuple-element-type.loft`, 9 rows + 4 controls, both
backends.  Out of scope and filed separately: a forward-declared type inside a tuple never
resolves (loft#944) — it fails on the DECLARED path too, so it is stub adoption, not
registration.

### A tuple vector element is not offered the element slot (loft#942, 2026-08-16)

`parse_item` seeds the element expression with `Value::Var(elm)` — the slot `OpNewRecord`
carved out of the container — so a struct element builds straight into it; `parse_object`
takes that in-place path whenever it is handed a `Var` that owns a store.  A TUPLE element is
not built as one record: `emit_tuple_set_ops` writes each member at its own offset.  Offering
the slot let the tuple literal's FIRST member consume it, so `[(S { … }, k)]` wrote S's fields
directly into the element and then handed that valueless statement list to `OpCopyRecord` as
its SOURCE:

```
_elm_1 = OpNewRecord(v, 81, 65535);
OpCopyRecord({ !! INSERT                      <-- a statement list, no value
    OpSetInt(_elm_1, 0, 11); OpSetInt(_elm_1, 8, 22)
  }, OpGetField(_elm_1, 0, 78), 78);
```

Only the FIRST member could fail this way: `(a, b, …)` parses member zero into the caller's
value (`vectors.rs:409`) and every later member into a fresh one, which is the whole reason
`vector<(integer, S)>` was correct while `vector<(S, integer)>` was not.

One defect, four filed symptoms — each construction path corrupts differently: reading an
element back panicked in `allocation.rs` using the tuple's SECOND member value as a record
index (`--native` refused the generated Rust with E0308), a 2+ element literal aborted the
compiler with `Incorrect var _elm_1[56] versus 40`, a single `+=` silently zeroed the struct's
fields, and a second `+=` SIGSEGV'd.

**The guard keys on the `(` token, not on the element type.**  A literal in RETURN position
infers its element type from itself, so the type is still `Unknown` when member zero is seeded
and only resolves by member one — instrumenting the seed showed the guard firing 4× for a
declared local but only 2× in return position.  A type-keyed attempt therefore fixed every
other row and turned the return-position abort into a SILENTLY EMPTY vector, which is why the
regression guard asserts lengths as well as values.  A `(`-leading element that is not a tuple
(`[(S { … })]`) gives up the in-place build and takes the allocate-then-copy path every
non-first member already takes; the unparenthesised `[S { … }]` common case is untouched.

Not fixed, and separate: a vector literal of tuples cannot have its element type INFERRED
(`v = [(7, 8)]` fails with "cannot build this record — its type never resolved" for every
tuple shape including ones with no struct at all, on the released binary too).

### A destructured tuple element is a value the binding owns (loft#941, 2026-08-16)

A tuple return wider than 8B lands in a synthetic `__tuple<…>` record held by a work-ref
belonging to the CALL SITE, so one site reuses one buffer.  Destructuring read each element
straight out of it — `OpGetField(tmp, offset, …)` answers a DbRef sharing `tmp`'s `store_nr`
and `rec` — and made that VIEW the binding.  Reassigning the work-ref frees the store it
named, and reassigning it is exactly what the next turn of a loop does, *before* the call it
feeds:

```
327: VarRef(__ref_2)              ; the tuple store from the previous iteration
330: FreeRef            [store-free]
337: Call(fn=n_passthrough)       ; xs, which VIEWS that store, is passed in here
```

So the binding dangled from the second iteration on.  One use-after-free, two symptoms:
reading a freed store answers its cleared contents, so `(xs, n) = passthrough(xs)` reported
`len` 0 while `--native` — which does not emit that free — answered correctly; and appending
onto a record the arena had recycled panicked in `vector_append`.

P250 had given a `Reference` element a DEPENDENCY on `tmp` so scope analysis would not emit a
second `OpFreeRef` for the binding.  That stops a double free, but a dependency cannot lengthen
the buffer's life past the reassignment — the binding still outlived the record it was read
from.  `materialize_tuple_element` copies instead, the same materialise-the-view move
`return <field>` (#306) and `&out = <field>` (loft#775) already make, which is why those two
directions were safe and this one was not.  A record goes through `OpDatabase` + `OpCopyRecord`,
a collection through `vector_db` + `OpAppendVector`; value-typed elements are read by value and
are untouched.

The filed scope was a third of it.  A plain STRUCT element fails identically, so the axis is any
element read back as a pointer, not `vector<T>`; and the `xs = f(xs)` spelling is not needed —
any binding that names the buffer and is read after the site runs again will do.  What IS
load-bearing is the SITE repeating: two distinct call sites alternating in one loop each own a
buffer and were always correct.

The result is emitted as a flat `Insert`, not as `Set(v, <block ending in v>)`: the allocation
writes through the binding itself, and the native backend renders a first binding as
`let mut var_v = <init>`, which rustc rejects when `var_v` appears inside it.

### `loft test` shares one library parse across the files that `use` it (loft#925, 2026-08-16)

`run_tests` builds one `Parser` per test file, deliberately — a shared one would let one
file's definitions leak into the next.  Each of those parsers loaded the `use`d library from
source, and **twice**: `Parser::parse` runs two passes, `Data::reset` clears `use_names`
between them, and an unnamed library is one `use` re-reads.  A suite therefore paid the
PRODUCT of its file count and its library's size — measured at 0.068 s/file against a
no-`use` control's 0.022 s/file, with the per-file cost proportional to the module count.

Three pieces:

- `Data::preloaded_uses` + `Data::freeze_uses`.  `reset` re-seeds `use_names` from it, so a
  `use` of an already-parsed library takes the `use_exists` branch — a pending import against
  definitions that are already present — instead of `switch_to_dep`.  This is the whole
  mechanism; everything else is plumbing around it.  Empty for every ordinary parse.
- `Parser::parse_as` — `parse` refactored so the entry can be a STRING claiming a filename,
  the two `lexer.switch` sites going through one `load_main_file`.  Not `parse_source`, which
  skips the between-pass promotions (`reserve_late_return_buffers` and friends): a base built
  that way would hand its libraries on in a state no ordinary parse produces.
- `Parser::seed_from` — `data` cloned, `database.install_schema`, plus `use_paths` and the
  native/placed-library registrations a manifest read queued.  The parse-time side maps
  (`complexity`, `field_read_counts`, the sandbox designations) deliberately do NOT travel:
  they drive diagnostics the base already emitted, and copying them would emit each twice.
  The runner carries the base's diagnostic LINES instead — minus the ones positioned at the
  base file itself, which every group member re-emits from its own `use` line.

`test_runner` groups files by `(directory, lib search path, leading use region)` — the region
VERBATIM, so the parser stays the authority on what those lines mean and one key is one
library set by construction.  The base is built when a SECOND file asks for it; a seeded file
skips the stdlib warm load (the base holds it, and decoding the bundle only for `seed_from`
to discard was most of what a seeded file still paid) and takes `start_def` from the base's
recorded stdlib boundary, so the native codegen range and the coverage tally keep counting
the library as part of the program under test.

Refused, falling back to the ordinary parse: under a `[sandbox]` policy (admission reads what
the parse recorded about designated functions), on a base parse that panics or errors, and on
any region that is not plainly an optional `#cwd` plus complete `use` statements.  `#cwd` is
IN the region rather than a reason to give up — all 81 of dryopea's test files open with one,
so refusing it made the change measure perfectly on a synthetic and do nothing for the case
that motivated it.

Measured: 20 files / 25 modules 1.32 s → 0.43 s, 40 files 2.68 s → 0.74 s, 20 files / 50
modules 2.44 s → 0.81 s; dryopea's 81-file, 1161-test suite 238 s → 209 s with byte-identical
output.  `loft test <one-file>` unchanged (0.07 s → 0.06 s).

`LOFT_NO_TEST_BASE=1` is the opt-out and `LOFT_TEST_BASE_REPORT=1` names the shared regions.
`tests/test_base_equivalence.rs` compares a whole run against the opt-out over a package with
four groups across two libraries — proved able to fail by dropping the carried diagnostics
(turns `@EXPECT_WARNING` and `--deny-warnings` green) and by dropping the region from the key
(a file resolves a library it never named).

Also here: the per-function `@EXPECT_ERROR` / `@EXPECT_WARNING` / `@EXPECT_FAIL` maps are
`BTreeMap`s.  They are iterated to REPORT the function names a file satisfied, and hash order
is randomised per process, so the same green run printed that list in a different order every
time — which makes a run's output undiffable, and this change is verified by diffing runs.

### `sizeof` and `type_name` answered null for an undeclared name (loft#933, 2026-08-15)

Both intrinsics read their argument the same way: take the identifier, look it up, and — when
the def exists but is still `DefType::Unknown` after pass 2 — mark the argument *found* and
return.  `*val` keeps its `Null` initialiser, so `sizeof(NoSuchType)` answered `null` and
`type_name(NoSuchType)` rendered `null` as if that were a type's name.  Neither said anything,
and the null flowed on as a value.

A name still unresolved after pass 2 is not a forward reference — those resolve in
`resolve_deferred_unknowns` and take the branch below with a real size (verified: a struct
declared after its use, and a type named only in an earlier signature, both answer correctly).
It is a typo, and it is the likeliest way to reach either intrinsic wrongly.  Both now report
`Undefined type <name> — sizeof/type_name needs a variable or a declared type`, still marked
found so the expression path adds no cascade.

`src/parser/operators.rs`'s bare `Unknown variable` also names its variable now, matching every
sibling site (`Unknown variable 'x' — did you mean 'y'?`).  Half of loft#934; the other half —
an undefined comparison operand whose ONLY report is `missing argument for parameter 'v1' of
`OpLtInt`` — is pinned in `36-parse-errors.loft` and left open.


### `--lib` is part of the program-cache key (loft#930, 2026-08-15)

`program_cache_paths` hashed the entry script's path and nothing else, so one script run
against two library trees shared a cache slot and the second run silently reused the first
tree's build.  Nothing downstream could catch it: the drift manifest re-validates the files
the FIRST run resolved, and those are still unchanged, so the freshness check passes and
hands back the wrong library's code.

That made the tool look responsive while ignoring the flag that selects which library it is
responding to — an in-place edit of the bound library DID rebuild, moving the tree away DID
force re-resolution, and only the `--lib` value itself changed nothing.  A consumer's A/B
harness (the Moros Economy planet-generator port, verifying loft against a compiled C# twin
by running one entry script against two library trees) compared an arm against itself and
read the byte-identical output as the strongest possible pass.

The search path now feeds the key, length-prefixed and in order — order matters because the
same dirs listed differently can resolve a name to a different file.  The cache still hits:
repeated runs against one tree keep one manifest and stay warm (0.04 s cold → 0.01 s), and
two trees now keep two.

`loft` built in a Cargo tree disables the program cache by default, which is why this
reproduces on an installed `loft` and needs `LOFT_PROGRAM_CACHE=1` to show up in a dev
build — worth knowing before concluding a cache defect is fixed.


### 56 of 167 `@EXPECT_ERROR` annotations never fired, and the suite reported nothing (loft#929, 2026-08-15)

`check_diagnostics` failed a file on *unexpected* errors and on unmatched `// #warn`
patterns.  It collected unmatched `@EXPECT_ERROR` and `@EXPECT_WARNING` substrings and then
DROPPED them, so an expectation whose diagnostic had been reworded, narrowed or removed kept
passing.  A third of that guard family was inert.

**The dominant cause was not message drift.**  `Parser::parse` runs pass 2 only when pass 1
finished clean, and a large share of loft's diagnostics are emitted by `!first_pass` code —
`Unknown variable`, the const/`&` checks, match exhaustiveness, the @PLN25 N-Store family, the
type-mismatch messages.  One pass-1 error therefore silences every pass-2 diagnostic in the
same file, and an annotation for one of those can never match however correct its wording.
Two files held 52 of the 56 for exactly this reason.  Error fixtures are now split by pass —
`102`/`102b`, `36`/`36b`, `35`/`35b` — and TESTING.md records the rule.

The rest triaged into reword (the argument/return/branch messages became `expected X, got Y
on …`; the `&`-vector concat refusal was rephrased), tier (the whole N-Store family reports at
`warning`, not error — `@EXPECT_WARNING` now), over-count (three `Undefined type V` signatures
earn ONE diagnostic), delete (a tuple in a struct field is supported since Plan-06, so its
refusal is gone), and blocked-by-a-neighbour (three tuple fixtures sourced their nullable from
`s as integer`, which now errors `text-parse-may-fail` on the line before, so the N-Store check
was never reached — they source it from division instead).

Two guards had gone inert without any annotation being wrong:

* `389-narrow-sentinel-rejected.loft` asserted the pre-@PLN25-F2 rule.  F2 made a plain narrow
  integer non-null and full-range, so `nullable_sentinel_hint` returns early and there was no
  error left to expect.  It now pins the rule from the value side, plus the half F2 left open
  (a NULLABLE narrow still spends its top value on the sentinel: `U8Q { x: 255 }` reads back
  null, silently).
* `894-lost-write-through-returned-struct.loft` expected a diagnostic **the harness could not
  produce**: `warn_lost_temp_writes` and its two neighbours ran only in `src/main.rs`.  They now
  run in `run_test` in the same window, so the suite can both confirm one of their warnings and
  catch a false positive from one.

Both holes are closed: unmatched expectations are fatal, and the check runs even when a file
produced NO diagnostics — the second way an expectation went unlooked-at.  `loft test`
(`src/test_runner.rs`) had the weaker form of the same hole, where any single matching error
satisfied every `@EXPECT_ERROR` in the file; each substring must now match one, the bar
`@EXPECT_WARNING` already held.

Three defects surfaced that the inert guards had been hiding, filed with repros: an `i32`
struct field silently truncates a 64-bit integer (loft#931 — `i32` is the one narrow alias
declared without a `limit(…)`, so the range-containment narrowing test cannot see it), the
reserved-`key` hash guard covers a struct field but not a local (loft#932), and
`sizeof(<undeclared name>)` answers null with no diagnostic (loft#933).  A fourth, an
unresolved comparison operand cascading into `missing argument for parameter 'v1' of
`OpLtInt``, is pinned in the fixture and filed as loft#934.

### A struct that contains itself says so, even when the program uses it (loft#929, 2026-08-15)

`Data::has_value_cycle` skipped recursing into a child struct that `def_referenced` marked.
That flag records that a struct has been CONSTRUCTED somewhere (`build_object_ops` and the
object literals set it) — it says nothing about whether the FIELD is a reference.  So the cycle
report fired only for a cyclic type nothing instantiates, and every cyclic type a real program
writes fell through to the layout validator instead:

```
Error: type layout: PENode: field 'next' has no position (u16::MAX)
```

in place of *"Struct 'PENode' contains itself (directly or indirectly) — use reference<PENode>
to break the cycle"*.  The field's own deps are what say "reference" (the `u16::MAX` share
marker), and that test was already there; the extra condition only suppressed the good message.
Verified across the shapes the rule separates: `reference<Self>`, mutual A/B, mutual broken by a
`reference`, plain nesting, `vector<Self>`, and a cyclic type never constructed.  The
`@EXPECT_ERROR: contains itself` fixture that should have caught this had itself gone inert.


### A `for` loop binds its own variable, so two loops may reuse a name (loft#915, 2026-08-15)

A loop variable was an ordinary function-scoped local: `add_variable` resolved it by name, so
a second `for` over the same name was handed the FIRST loop's slot — old var, old type, old
dep. That is why two loops in one function could not reuse a name at different element types,
and it is the mechanism behind loft#690's corruption (the second body read B's records through
A's layout, `m=8589934636` for a sum of 3), which had been answered with a diagnostic.

Each loop now binds its own variable. `Function::loop_binding` names them: the first loop to
use a name binds the name itself — so a program with no repeat spells every loop variable
exactly as it did before, dumps and debugger frames included — and a second binds `i#1`, a
third `i#2`. The suffix is on the NAME and not merely on a lookup key because the native
backend names a local `var_<name>` and two locals spelling one name declare it twice, the same
constraint loft#928 hit for a generator's fields.

**The cross-pass identity key is separate from the name** (`Function::loop_variable`, key
`<name>#bind`). A loop variable cannot key on its own name: the name is re-pointed at each loop
that binds it, so `names["i"]` ends pass 1 holding the LAST loop's slot and pass 2's FIRST loop
would be handed it — a text binding reusing an integer one, which is the shape the split exists
to stop. The occurrence counter reads no type and consults no table, so pass 2 regenerates the
same sequence and every slot number holds.

`i` after the loop still reads what the last loop left, so nothing that read it before changes.
The companions (`#index`, `#next`, `#count`, `#iter_state`) are keyed off the binding rather
than the spelling, and `iter_op` derives that base from the variable the name resolves to, so
`i#index` in the second loop finds the second loop's counter. `loop_nr` — which `#break` and
`#continue` jump on — matches the loop by BINDING instead of by name, since comparing names
would walk past a loop whose variable is `i#1` and answer the chain length.

**Two diagnostics folded into one.** loft#690's *"loop variable 'i' has type text but was
previously used as integer"* is gone: the corruption it reported is unreachable by
construction, and the local collision it also covered no longer needs a type comparison to
state — any non-loop binding of the name is the shadow, whatever its type. The C61 shadow
diagnostic now owns that case and fires on PASS 1 for every type pairing, where the type
diagnostic reached the differing-kind case only on pass 2.

Still rejected: a loop variable landing on a plain function local, and nested same-name loops
(the inner binding would take over the name for the rest of the outer body).

Guard: `tests/scripts/915-loop-variable-per-loop.loft` — 13 hand-computed cells on both
backends, covering the filed shape, the after-loop read, `#index` / `#count` / `#first` /
`#break`, loft#690's two-struct shape, three loops under one name, text loops, comprehensions,
`_`, nesting, and per-function independence.


### A loop that writes no store reads its vectors through one derived header (loft#885, 2026-08-15)

`v[i]` re-derives three facts per element on `--native`: which store holds the vector, which
record its elements live in, and how long it is. All three are loop-invariant in a loop that
writes no store, and `rustc` cannot lift any of them — every store load is guarded
(`if rec != 0 && valid(..)`) and LLVM will not speculate a conditional load out of a loop.
So the emitter lifts them: `let __vh_N = vector::vec_header(…)` lands before the loop and each
read becomes a bounds test plus address arithmetic. **~2× on the issue's kernel**, taking
`vector<single>` indexed reads from ~15× hand-written Rust to ~6.5×.

A scalar read of a hoisted element then fuses into ONE load (`vector::get_elem_hoisted`):
the bounds test, then the value, with no element `DbRef` built, no `rec == 0` test, no second
store resolution and no `rec != 0 && valid(..)` re-check between them — the bounds test
decided all of it. Covers `OpGetInt` / `OpGetSingle` / `OpGetFloat`, the getters whose `Store`
bodies are a plain guarded load; the masking / re-basing / decoding ones stay unfused. That
also meant teaching the pre-eval collector about the fusion: `OpGetVector*` is on
`op_uses_stores`, so it was being hoisted into a `let _pre_N` that the fused emission
ignores — which would have run the read twice. `hoist::fused_element_read` is the one
definition of the shape, and the emitter and the collector both ask it.

Only an index in range for the hoisted length takes the fast path — a negative index, an
out-of-range one, `i64::MIN`, a null or an empty vector all fall back into `get_vector` /
`vec_get_or_raise_runtime`, so those answers and the `IndexOutOfBounds` / `NegativeIndex`
raise keep one definition. The interpreter is untouched.

The gate (`src/generation/hoist.rs`) is an **allow-list**, deliberately: PERFORMANCE.md
§ Design: P8 catalogues five hand-maintained deny-lists of "which op mutates" that have
already drifted, and an omission in one of those would be a silent wrong read. Here an op
missing from the list costs the optimisation instead. An op qualifies by being named as a
reader/constant, or by declaring at least one parameter with **every parameter a plain
runtime scalar** — where `const` disqualifies, because a `const` parameter is a slot number
or type id, and that is the channel `OpDatabase`, `OpCoroutineNext` and `OpFreeText` reach
state through despite scalar signatures. The walk follows calls, so `for i in 0..len(v)`
still hoists; `CallRef`, `par` and `yield` decline.

Two switches, both read at generation time: `LOFT_HOIST_VERIFY=1` emits the checking form of
every hoisted read (re-derives the header, panics on a mismatch) and `LOFT_NO_VECTOR_HOIST=1`
emits the pre-885 form. Guards: `tests/hoist_gate.rs`, `tests/scripts/885-vector-hoist.loft`.

### A `τ?` struct field is representable as absent (loft#896, 2026-08-14)

A field declared `maybe: Inner?` was stored as a dense `Inner`, byte-identical to a
non-nullable one. `OpGetField` therefore handed back a `DbRef` into the PARENT record, whose
`rec` is never 0, so every reader saw a present-but-zeroed value: `??` never reached its
default, `== null` was always false, and `h.maybe = null` found nothing to clear. Both
backends, silently.

**The representation was already built.** The synthetic `__nullable<S>` enum (discriminant 0 =
absent) has shipped default-on for vector ELEMENTS for some time, and `vector<Inner?>` answers
every cell of loft#896 correctly — which is what made it the oracle. What was wrong is the
rewrite that assigns that type to a FIELD (`typedef.rs::synth_nullable_struct_fields`): it
matched a bare `Type::Reference`, and `Inner?` reaches the type table as
`Optional(Reference(Inner))`. A field written `Inner` — one that *cannot* be absent — is the
bare `Reference`. So the arm selected the exact COMPLEMENT of its intended set, rewriting every
dense field and never once firing for the `S?` it was written for.

That inversion is also why it sat behind `LOFT_E2_FIELDS`: the gate's stated justification was
that flipping fields tree-wide breaks stdlib field reads, which is what rewriting *dense* fields
does. The symptom was read as the representation being immature rather than as the selector
being backwards, and the read/construct glue the gate's comment called missing turns out to
work — a bare `h.maybe.z` auto-unwraps.

Fix: select on `Optional(Reference(S))`, drop the gate, and add the literal-`null` construction
path (`H { maybe: null }`) — assignment already had one, so both now route through a single
`Parser::build_nullable_set_null` and the two spellings of "absent" cannot drift.

Also fixed by the type change: a struct literal that merely OMITS such a field did not compile
under `--native`. The omitted default was `Value::Null`, which the interpreter tolerates as a
no-op `OpCopyRecord` of a null source and native lowers to `OpCopyRecord(cell, (), …)` —
`()` where a `DbRef` is expected. It reproduced on the released `loft 2026.8.0`.

**Costs.** A `S?` field carries a discriminant, so it grows 8 bytes;
`tests/multilib/fwd797_layout.loft`'s hand-computed sizes moved 44→52 and 52→60. A
`vector<T>?` FIELD is `Optional(Vector)`, a different payload shape, and is unchanged — still
wrong, and genuinely separate work.

Guard: `tests/issue_896_nullable_field.rs`, 16 cells on both backends, including a dense-field
control that fails if the selector ever again keys on anything but the `?`.
`tests/plan25_e2_layout.rs`'s fixture declared `item: Row` and asserted the rewrite fired — it
passed by encoding the inversion, and now declares `item: Row?` with a dense sibling beside it.

### A partial struct literal names the field it left out (loft#914, 2026-08-14)

New `advice[omitted-field-zero]`, default on, `LOFT_NO_OMITTED_FIELD` opts out. A literal that
names SOME fields and leaves another out gives the omitted one its type's zero, and nothing
distinguished that from an author writing the zero deliberately. It bites where zero is a
meaningful value of the field's domain — dryopea's palette index wanted `-1` for "nothing
selected", got `0`, and `0` is the entry that erases; the project carried a two-field workaround
and a CLAUDE.md rule for it, because the cure (a declared field default) was undiscoverable.

`advice`, not `warning`, per the two-tier rule: the zero is documented behaviour
(`tests/scripts/06-structs.loft` locks it), so ignoring it cannot produce a result the language
did not promise — and a warning would fail every library's own `LOFT_DENY_WARNINGS=1` CI on a
common idiom, which is the trap the tiers exist to avoid.

Quiet where the code already says what it means, or where the cure would be a no-op: a declared
default; a nullable field; `reference<T>` and fn-ref fields (their omitted default is a null
sentinel, and a fn-ref has no other default to declare); collections and `text` (their zero is
the identity, and `= []` / `= ""` IS that zero); and a bare `S {}`, which asks for the whole
default record and reads that way. Only the PARTIAL literal is ambiguous.

The last three exemptions came from the suite rather than from reasoning —
`issue_328_reference_field_pointer_semantics` and `p213_struct_field_default_init` each name a
field whose absence is already the declaration's promise. Swept before it spoke: 25 hits across
7 of ~400 corpus files, and one golden baseline moved
(`tests/error_messages/cases/33_struct_missing_field.loft`, which produced no output at all).

### `loft test` no longer reports a green for a file it did not run (loft#916, 2026-08-14)

`loft test <a> <b>` silently discarded everything after the first target. It ran `<a>`, printed
`test result: ok. 1 passed; 1 file`, and exited **0** — even when `<b>` held a failing test. The
file count was the only place it showed, and that reads as correct unless you already knew how many
you asked for. Naming two files is the natural move when a change touches two suites and the whole
run is slow, which is exactly when nobody re-reads the count: it cost a sabotage sweep whose second
half never executed and was reported green.

A second target is now an error naming both, not a drop. One target per run is kept deliberately —
the summary line is a single verdict over one scope, and looping would print a partial one per file,
which misleads in a new way rather than fixing this one. Only the CONSECUTIVE leading positionals
are examined, so a later flag's value (`--lib <dir>`) is never mistaken for a second target; that
row is in the guard, because a rule written as "no bare token after the target" would have broken
it. Guards: `tests/test_command_targets.rs`, asserting the EXIT CODE as well as the text — the exit
code is what a CI job reads, and it was the half that made this dangerous rather than merely
confusing.

### A module shadowed by a dependency's same-named file now says so (loft#912, 2026-08-14)

A module's basename is global across a consumer's whole dependency graph. Only the first file to
claim a name is loaded, so adding `src/catalogue.loft` to a package whose dependency already had
one made the LOSER's functions simply absent — reported as `Unknown function part_list` at a line
inside a package the consumer never edited and cannot fix. Nothing in the output mentioned a
collision, so the search went looking for a missing `pub`, a typo, or a version skew.

New `advice[module-name-shadowed]` names the collision and BOTH files: *"module 'catalogue' is
declared by two files — '…/pkg_top/src/catalogue.loft' and '…/pkg_dep/src/catalogue.loft' — … this
`use` binds the second one"*. Both load orders are covered, so the report does not move when a
`use` is reordered.

**The resolution itself is unchanged, and that is deliberate — decided against a measurement rather
than a preference.** The obvious fix, refusing the clash, was implemented first and then measured:
it breaks code that builds today. `graphics` ≤ 0.4.2 and `mesh3d` both ship `math` / `mesh` /
`scene`, and this repo's own `tests/fixtures/libs/graphics` depends on the registry `mesh3d` while
carrying its own copies of all three — three test binaries went red. A first sweep over
`~/.loft/registry` alone had said the clash was extinct; it missed the fixtures, which is the axis
that was held fixed.

**Scoping module names to their package is the fix this advice is a signpost for.** Two things
block it, both worth recording: `Data::use_add` derives a new source id from the SIZE of the name
map (so a second key per module happens to keep the counter right, but nothing says it must), and
`qualified_type_name` derives a DATABASE type key from a module's short name — a package-qualified
key has to stay machine-independent, so it cannot carry the package's path. Guards:
`tests/module_name_clash.rs`, which asserts the advice fires in both directions AND that the
`Unknown function` symptom still follows, so a test cannot silently claim the fix that has not
landed. It also pins the three neighbouring shapes that must stay silent: a distinct name, one
module used from two files of one package, and a file named like a declared dependency (which the
existing dep-shadowing guard already resolves).

### `loft doc <library>` documented nothing, into the directory you were standing in (loft#911, 2026-08-14)

`loft doc` reads as — and is used as — `loft doc <library>`, but its argument was a PATH only. A
library name is not a directory, so `loft doc graphics` took the empty-manifest branch: it created
`./graphics/doc/` wherever the user happened to stand, found no `src/` to read, and reported
"0 API sections" for a package with 119 documented `pub fn`s. The path it printed was relative, so
the stray tree looked like part of the project — one was swept into an unrelated repository by a
later `git add -A`.

The argument now resolves as a directory first and an installed package second, and a name that
resolves to NEITHER is an error that creates nothing. An installed library's docs go to
`~/.loft/doc/<name>-<version>`, because the registry copy is shared immutable cache content and the
working directory is not loft's to write to; `-o <dir>` overrides, and the reported path is
absolute. `loft doc graphics` now reports 1 guide and 19 API sections. Guards:
`tests/doc_command.rs`.

### A non-empty collection literal on a nullable field aborted the compiler (loft#909, 2026-08-14)

`struct S { m: vector<integer>?, t: integer }` with `S { m: [5], t: 3 }` aborted with
`Incorrect var _vec_1[65535]`, on both backends and on the published release. Whether a field
carries a record-pointer HEADER is a question about its storage, and `Optional(τ)` shares τ's
storage exactly — but `parse_object_field` asked it by matching the declared type against the
collection formers without peeling the marker. The field was therefore not recognised as a
collection: the literal built through a standalone temp instead of in place, and that temp, minted
with a dep on the struct it sits in, is exactly the case where `build_vector_list` skips
`vector_db`. Nothing assigned it, so it reached codegen with no live interval and no stack slot.

Both halves were needed — a non-nullable field took the in-place path, and an empty literal
returned before the temp was minted — which is why each of them rescued the program. A nullable
KEYED field failed one step earlier still, refusing the literal outright with "Cannot assign
vector<R> to field of type optional(hash(…))".

The peel is applied at the three sites that classify a field by its layout — `parse_object_field`'s
header prime, `handle_field`'s deep-copy dispatch, and `get_type`, whose sibling resolvers
(`type_def_nr`, `type_elm`, `rust_type`, `element_stack_size`) all peeled already while it answered
`u16::MAX`, the "no such type" sentinel, for every `Optional`. The nullability checks keep the
unpeeled type: those ARE about nullability. Guard:
`tests/scripts/909-nullable-collection-field-literal.loft`.

### A bare `if` statement swallowed the `[` of the line below it (loft#910, 2026-08-14)

`[` postfix-indexes a value, and a `Void` expression produced none. `if` is an expression in loft,
so a bare `if c { … }` STATEMENT reached the postfix chain that handles `.`, `[…]` and `(…)` — and
that chain consumed the bracket opening the next line. A function whose tail expression was a
vector literal had that literal read as an index on the `if` above it, and indexing a `Void` fell
to the catch-all: *"Indexing a non vector — keyed collections have no generic-constructor
expression"*, naming a feature the program does not use and a line that is correct as written.

The filed scope was a comprehension in tail-return position reading a local a bare `if` had
mutated. None of those three is the trigger: a plain `[1, 2]` fails identically, `else` makes no
difference, and the mutation is irrelevant. `for` and `while` never had it — they are statements
and never reach the chain. The guard reads the subject's TYPE, so an `if` that yields a value keeps
its index: `if c { [1,2] } else { [3,4] }[0]` is unaffected. An indexed `Void` CALL
(`voidcall()[0]`) is still an error, so no wrong program became a silently accepted one.

**A native-only defect surfaced while pinning that last row**, present on the published release
too: `if c { [1,2] } else { [3,4] }[0]` ran correctly under `--interpret` and failed native codegen
with "expected expression, found `let` statement". A pre-eval binding is emitted as
`let _pre_N = <text>;`, so the text has to be one Rust expression — and an `if` that pre-declares
its branch variables emits `let mut var__vec_1: DbRef = …; if …`. The wrap that made a statement
sequence an expression had been written into ONE of the two lowerings that produce that prefix, so
the other never got it. It is now enforced on the artifact instead: whatever lands in a `let`
binding is braced if it is a statement sequence, which no producer can drift away from. Guard:
`tests/scripts/910-statement-if-does-not-index-the-next-line.loft`, asserted on both backends.

### `loft test` refused the path it prints (loft#913, 2026-08-14)

`loft test` reports its files as `tests/<name>.loft` and rejected that exact string: the argument
was joined onto `tests/` unconditionally, so pasting a failing line back asked for
`tests/tests/<name>.loft`. Copying the path out of the output is the obvious way to iterate on one
file, and the tool not accepting its own output is re-discovered by every new user rather than
learned once.

Measuring every spelling before fixing turned up a second break the report did not mention:
`loft test draw::test_foo` — the selector form the code's own comment documents — resolved to
`tests/draw::test_foo`, whose path half has no extension and matches no file. The `.loft` was
appended to the whole argument rather than to the PATH half, so the documented form only worked
if the caller also wrote `.loft`.

`resolve_test_target` now splits the `::selector` off first, supplies the extension on the path
half, and joins `tests/` only when the path is not already under it (nor absolute, nor reaching
out with `..`). The doubled path could never exist, so every spelling that worked before resolves
to the same file; four spellings that used to be errors now work. Unit tests in `src/main.rs`.

**And the accidental guard it removed is replaced by a real one.** `loft test good::test_missing`
used to fail — for the wrong reason, on the mangled path — while the correctly-spelled
`loft test good.loft::test_missing` reported `ok. 0 passed; 0 files` and exited 0, on the published
release too. A filter that matches nothing left every file empty and each was skipped silently. A
selector naming no test function is now an error: it is the shape a CI job reads as "the tests I
asked for passed". Only an explicit selector is checked — a directory with no tests is a different,
legitimate zero. (A brace list with SOME matches still runs those and reports them; only a
completely unmatched selector fails.)

### An empty-text assignment pushed a value nothing consumed (loft#908, 2026-08-14)

Reported as "a function that reads a MISSING file and returns a struct double-frees and SIGABRTs
the interpreter" — `free(): invalid pointer`, `last op: OpFreeText`, on `--interpret` only, which is
the worst direction: a consumer's gates run interpreted and the shipped native build is correct.

Neither the file nor the struct is the defect. Appending the EMPTY literal to a text variable is a
no-op — the variable has just been cleared — so `set_var`'s put dispatch skipped `OpAppendText` for
it. It skipped only the OP, after `self.generate(value, …)` had already pushed the 16-byte const:
the value stayed on the eval stack with nothing to take it off, and `stack.position` ran high for
the rest of the statement. **A value is pushed if and only if an op consumes it**, and the two
decisions sat in different places.

Harmless until the statement is one ARM of an `if`/`else` — which `?? ""` over a CALL always is, the
nullable result going into a work-ref that the presence test branches on. The arms then disagreed in
height and `gen_if`'s arm-height equaliser (@PLN85 P2) "corrected" the taller one with an
`OpFreeStack` whose discard walked past the frame's eval base into the LOCALS, overwriting a live
text descriptor; freeing that at scope exit aborted. The aggregate return is what puts a live local
under the over-discard (the hidden `__retbuf` shifts the frame), which is why the reporter's matrix
found it needed a struct return — and why `-> integer` merely mis-tracked the stack silently.

Four axes had to meet: the nullable text from a CALL, the call answering null, an EMPTY default, and
an aggregate return. Moving any one made it correct, which is what kept it hidden.

The guard now returns BEFORE the push, so push and consume are one decision;
`gen_set_first_text` already skipped the whole `set_var` for this value, so the reassignment path
now agrees with the first-assignment path rather than diverging from it.

Guard: `tests/scripts/908-empty-text-default-does-not-strand-a-const.loft`, one axis per row and
every row asserting a VALUE — `--native` was always correct, so a row that merely ran would read as
a pass there. Verified to abort on the pre-fix build before shipping.

### `--native` linked a `#native` symbol by NAME, not by what implements it (loft#907, 2026-08-14)

`#native "sym"` is an API id, not the name of the Rust fn behind it. A library registers its
implementations by loft symbol — `loft_register_bridges! { "sym" => other__loft_bridge }` — and
that table is free to name an `other` different from `sym`. `--interpret` reads the table.
`--native` put the `#native` string straight into a `#[link_name]`, so it bound whatever else the
cdylib happened to export under that name, and a C-ABI link matches on name alone: no error, no
warning, a call marshalled into the wrong function.

In the published `graphics` that hit **ten** functions — every store-aware one. Each has loft's
`(LoftStore, LoftRef)` entry point at `n_<x>` and an older raw `(ptr, count)` fn under the
`#native` name, so the arguments arrived shifted by a register. `save_png` returned `false` and
wrote nothing under `--native` while returning `true` under `--interpret` (the reported symptom);
`gl_upload_vertices`, `gl_upload_canvas`, `gl_upload_indices`, `gl_upload_instance_buffer`,
`gl_update_buffer`, `gl_set_mat4`, `gl_texture_subimage`, `rasterize_text_into` and
`audio_play_raw` were mis-marshalled the same way and had no reporter because the WebGL
consumers run in the browser, whose `--html` host imports take the raw pair by design.

**One source for the answer, read by both backends.** `extensions::resolve_native_impl_symbols`
asks the loaded cdylibs' own registration which fn implements each symbol (`dladdr` on the
registered bridge names it; `X__loft_bridge` sits beside `X`), and records only the entries where
the two names differ, in `Data::native_impl_symbols`. `Data::link_symbol` is what codegen emits
through, on both the C-ABI `#[link_name]` and the rlib `krate::sym` path. A clean binding — what
`loft-ffi-build`'s generator produces, and the only shape it CAN produce — maps to itself and is
untouched.

Residual: a library whose cdylib is absent or predates the bridge registry cannot be resolved and
keeps the literal name. That is not a silent wrong answer — the interpreter reports it at load
(loft#886) and calling it panics rather than answering.

Guards: `tests/lib/native_remap_pkg` is a `[native] crate` fixture in exactly this shape, exporting
a DECOY under each `#native` name (-1000 / -2000) so a regression answers rather than fails to
link, and the answer names which resolution path was taken;
`native::remapped_native_symbol_resolves_to_its_implementation_on_both_backends` runs it on both.
`native_scalar_pkg` is the clean-binding control.

### Removing one entry of a linked collection group had no owner (loft#900, 2026-08-14)

loft#898 gave the CLEAR an owner for a linked group's shared records; removal never got
one, and was wrong in both directions. Through a VIEW it freed the record the primary
still held (the vector kept the entry and its key, the text read back `null`); through the
PRIMARY it never reached the views, which reported their old length over a freed record.
Both backends, and the published 2026.8.0.

**A removal spelled through any member removes it from the group** — the same verdict
loft#898 reached for the clear, and for the same reason: `h.view += [e]` has appended to
every member since loft#843, so an operation spelled through a view acts on the group. The
alternative has no coherent successor state — `h.by_k[1] = null` then
`h.by_k[1] = E{k:1,…}` would remove one index entry and then add to the whole group,
leaving the primary holding two records under one key with nothing able to repair it.

The ORDER is the mechanism. Every unlink reads the record's key out of the record, so the
free must come last and the record must stay reachable until then. The parser emits the
lookup ONCE into a work-ref temporary (marked `inline_ref`, since the record is the
collection's, not the temporary's), then one `OpHashRemove` per other member carrying the
`CLEAR_KEYED_VIEW` bit — the same `0x8000` convention `OpClearKeyed` and `OpSetKeyed`
already use on their `tp`, so arity and both emitters are unchanged — and finally the
ordinary removal on the member the source named, which frees. The temporary is also what
keeps the key expression evaluated once (@PLN102 F2); repeating `OpGetRecord` per member
would have re-run it.

The field site is resolved by walking the `OpGetField` chain (`keyed_field_site` /
`holder_type`) rather than by reading the base variable's type, so a group one level down
resolves too — reading only the base var is what left loft#898's nested case on the unsafe
path until its guard row a7 caught it.

Two supporting facts had to be repaired, both pinned by guard rows:

* `Stores::remove`'s `Parts::Array` arm computed its slot with BY-VALUE arithmetic
  (`(rec.pos - 8) / size`), which is 0 for every element of a record-backed container —
  the loft#719 defect, fixed then for `Ordered` and left for `Array`. The documented
  `vector<T>` + `hash<T[k]>` group has an `array` primary, so every unlink through it went
  to slot 0.
* `remove_owned` sent a grouped hash to `hash::free_entry`, which correctly declines to
  free a record a stride-0 table only borrows. Declining is right only while somebody else
  frees; when the removal is spelled through that member it IS the free, so the record and
  everything it claimed leaked. `Stores::hash_owns_entries` is the table's own answer to
  which case it is.

Matrix: 45 cells × both backends — every (primary, view, spelled-member) triple over the
four member kinds, three-member groups, an absent key, drain-and-refill, first/middle/last
of three, and ungrouped controls per kind.

Two PRE-EXISTING defects the matrix separated out and did not fix, filed with repros:
loft#902 (two `index` members share their red-black links, which live in fields of the
element record — the fill "works" because both fields then describe ONE tree, and the first
removal rebalances it into a panic) and loft#903 (`e#remove` in a loop maintains no
sibling, and over an `array<T>` removes two elements — no group involved).

### A `sorted` emptied by removal published the wrong slot on the next append (2026-08-14)

`sorted_new` hands the constructor a scratch slot and `sorted_finish` / `ordered_finish`
read the new record back out of it — at `length + 1`, except at length 0 where they take
the "first record needs no reordering" path and read slot 0. `sorted_new`'s existing-record
branch always answered `length + 1`, so the two disagreed at length 0.

Only one thing reaches that state: a collection EMPTIED entry-by-entry
(`coll[key] = null`), which keeps its allocation. `coll = []` drops the record, so the
next append takes the fresh-claim branch and lands in slot 0 as expected. The append
therefore wrote into slot 1 while `sorted_finish` published slot 0 — the bytes of the last
element removed, with its text already freed — so `s.a += [E{k:9,…}]` read back as
`2:null` and the new element was simply lost. `ordered_finish` inherits the slot from the
same call, and had the same failure with the rec-id.

Pre-existing on the published 2026.8.0, both backends, `sorted` and `ordered` only —
`hash` and `index` are unaffected. Found by the loft#900 matrix's drain-and-refill cell;
guard row b3 of `tests/scripts/900-linked-group-remove.loft`.

### A linked collection group's second route was silently under-populated (loft#901, 2026-08-14)

Filling one member of a linked group fills every member (loft#843). For three pair shapes
the second route never got the elements, with no diagnostic: `hash` + `index` kept ONE
element however many went in, `sorted` + `sorted` and `vector` + `sorted` stayed empty,
and — not in the filed scope — `hash` + `hash` built the right NUMBER of entries with
every one naming the first record. The filed table counted `len` only, which is exactly
what that last case does not disturb. Both backends and the published 2026.8.0.

**One fact explains all of them.** Every member names its elements by a 4-byte record id:
a hash slot encodes `rec.rec` (`hash::SLOT_RECORD`), an `array` / `ordered` slot stores it
raw and reads it back at a hard-coded payload start, and an `index` keeps its red-black
links in FIELDS of the record. None can express a position INSIDE a record. Two shapes
handed the siblings elements that do not own one:

* a hash **packs its entries into a shared chunk arena** (@PLN135 arc H), so an
  instrumented `record_finish` showed the two elements of `hash` + `index` arriving as
  `rec=(2,15,8)` and `rec=(2,15,32)` — one record, two positions. The index's b-tree links
  then collided in that record and it kept the first; a sibling hash encoded both slots as
  record 15 and read both back as its payload start.
* a `sorted` **stores its elements inline**, so as a view it has no record to name at all:
  `insert_record`'s `Parts::Sorted` arm never receives `rec` and sorts the view's own empty
  buffer.

Both disappear once the group's element type is record-backed. `record_new` already
refuses the arena for an element type flagged `linked`, with a comment describing this
exact failure, and `finish_type` already promotes `vector` → `array` and `sorted` →
`ordered` for one. The flag was only ever **set as a side effect of that promotion**, so a
group whose members are all keyed never set it. `Stores::finish` now seeds it from group
membership directly — a field with a non-empty `other_indexes` — which is the same
predicate `types.rs` used to form the group, so the two cannot drift.

This also removes an action-at-a-distance: whether a `sorted<T[k]>` was record-backed used
to depend on an `index<T[..]>` declared anywhere else in the program (the loft#719 /
loft#891 conversion), so the same source line lowered differently per file. That is what
made loft#898's `vector` + `sorted` matrix cell vacuous rather than correct, and it is why
`tests/scripts/901-linked-group-fill.loft` gives **every row its own element type** —
written over a shared `E` the guard printed `901 ok` on the unfixed published build.

Scope held: a collection that is not in a group is untouched, so a lone hash keeps its
arena (guard row c3 — a fix that made every hash allocate one record per entry would pass
every other row and silently give back @PLN135 arc H's win).

Matrix: 70 cells × both backends, covering all 16 primary/view pairs in isolation, `=` vs
`+=`, the fill spelled through the view, three-member groups, the contaminated-file
confounder, key lookup through the view, element counts 0/1/3, `trie` in both declaration
orders, and a clear after the fill. Gate: 3974/3974 curated + 57/57 on the four excluded
binaries a schema change can reach, fmt + clippy clean.

### A linked collection group had no owner for its records (loft#898, 2026-08-14)

Two or more keyed collections over one element type in one struct are auto-linked into
several routes to a SINGLE record set (`Field.other_indexes`, loft#843). Nothing said which
of them OWNED that set, so `remove_claims` freed the element records through whichever was
cleared and left the others naming freed memory — a length that still read 2 over bytes
answering `4294967296:null`.

The filed scope was wrong in three ways, all measured on a 12-cell matrix against the
published 2026.8.0:

* **`vector<T>` + `hash<T[k]>` is affected and was not in it** — the pairing DATABASE.md
  documents by name, and the one with an unambiguous owner.
* **Both directions are broken, one was filed.** Clearing a VIEW leaves the primary over
  freed records; clearing the PRIMARY never resets the views, which keep their old length
  over the same freed records. A fix for one does nothing for the other.
* **`Parts::Array | Ordered` is not the producer the report named.** `vector` + `sorted`
  is not a counter-example either: it never links at all, so that cell is VACUOUS rather
  than correct, and it is recorded as such rather than counted as coverage.

The ownership fact already existed in the schema and had exactly one reader. `types.rs`
marks every member after the first with a leading `u16::MAX` on `other_indexes`; only the
JSON default-init asked. Three pieces make it load-bearing:

1. `Stores::borrowed_spine` — what a VIEW owns, per kind: the hash table record, the
   `Ordered` slot list, and for `index` nothing at all (a b-tree's nodes ARE the element
   records, so zeroing the root is the whole teardown). It rides the SAME per-`Parts`
   match as `for_each_owned_child` rather than sitting beside it, because the spine a view
   drops is the `container_rec`/`extra_recs` that walk already names — a layout change
   cannot move one and miss the other. `OwnedChild` gained a `borrowed` flag so the
   struct-teardown arm can mark a view field from the schema.
2. A `0x8000` bit on `OpClearKeyed`'s `tp`, the convention `OpSetKeyed`/`OpReplaceKeyed`
   already use, so the op's arity and both emitters are unchanged. Both backends decode it
   in ONE place — `Stores::remove_claims_keyed` — so the interpreter's `#rust` template and
   `codegen_runtime::OpClearKeyed` cannot drift.
3. `Parser::keyed_group_clear`, emitted by the KEYED assign and the VECTOR assign alike:
   the documented `vector<T>` + `hash<T[k]>` shape has the vector as record holder, so a
   fix living only in the keyed branch would have closed half the matrix.
   `clear_group_primary` picks the op the owner's kind needs (`OpClearVector` for a plain
   vector, `OpClearKeyed` otherwise), because a clear may be reached from either member.

**The semantics question the report left open**, and what settled it: a clear spelled
through ANY member empties the group. Not a preference — `h.view += [e]` already appends
to every member (loft#843), so an operation spelled through a view acts on the group, and
`=` must match or `h.view = []` followed by `h.view += [x]` is incoherent. The filed
report asked for view-only emptying, which cannot be made coherent for a NON-EMPTY
literal: the elements still enter the group, so `h.view = [e]` would leave the view
holding `e` and the primary holding `e` plus everything it had. A model that works only
for the empty literal is not a model, and its output — an index silently not indexing its
records — has no repair operation. Rows d1/d2 of the guard pin the `+=` fact the model
rests on, so a future change to it fails here rather than silently invalidating the clear.

The parent struct type comes from `lhs_parent_tp`, which the assign already holds. Reading
it back off the base EXPRESSION only resolved a bare `Value::Var`, so a group one level
down (`o.inner.by_k`) read as "not a group" and kept the unsafe clear — the cell that
caught it is a7 in the guard.

loft#895's exclusion is gone with it: the multi-index field was kept on the append
specifically to avoid this use-after-free, so `=` now replaces on a group like every other
keyed field. `895-keyed-assign-replaces.loft` row c15 pinned that append deliberately and
is updated rather than left to flip silently.

**Not fixed, filed:** removal (`coll[key] = null`) has the same two directions and neither
is right (loft#900) — `Stores::remove_owned` takes no `secondary` flag, unlike the sibling
`dedup_keyed` that already makes exactly this distinction. And a group's view is silently
under-populated for three pair shapes (loft#901), which is why the `vector` + `sorted` cell
above is vacuous.

### A field store had no type check, and two of them corrupted the heap (loft#893, 2026-08-13)

A field store is the one assignment form with no variable to re-type, so
`change_var_type`'s rejection — the one that refuses `v = make()` for a local — never saw
it. The checks that DID cover fields are further down `parse_assign_op`, behind an early
return that a `text` or collection target takes first, so the class went unreported.

Three symptoms, one missing assertion:

* `h.v = make()` on a `vector<float>` field stored nothing and leaked the source store;
* `h.s = 3` on a `text` field carried the integer into `OpSetText` as a text handle and
  took SIGSEGV;
* `h.v += make()` reached the same op pair and panicked writing into the read-only
  `CONST_STORE`.

So the hole was memory safety, not only a dropped write.

Enforced at the point every store form still reaches (`parse_assign_op`, where `s_type`
settles, before any of the early returns) — which is why one check closes all three. The
predicate is `convert`, the same one the constructor path (`handle_field`) and the
scalar-target check already ask, plus one named carve-out: a keyed collection BUILT from a
vector of its elements (`h.m = [E{…}]` for `hash<E[k]>`) is the supported idiom, is
deliberately not `is_equal`, and has no `convert` arm.

`convert` is a `&mut self` emitter, so it is asked in the shape-only form it already
understands — a `Value::Null` expression, which every rewriting arm guards against and no
verdict depends on — and `conv_owned_result` is saved and restored around the call, since
a cast arm sets it to mark an allocating conversion and the next real conversion `take()`s
it. A probe that left it set would hand its answer to an unrelated expression. Adding the
diagnostic therefore cannot move codegen.

**Method note.** The predicate was run as a silent probe over all 2188 `.loft` files in
the tree before it was allowed to speak. Exactly one file hit, and it was a true positive:
`tests/docs/13-file.loft` read a sized `f#read` straight into a `vector<single>` field,
which LOFT.md's conversion table documents as needing an explicit `as`. It had been
storing an EMPTY field, and the example asserted nothing about the result, so nothing
caught it — the doc stated a rule the code never ran. Fixed to the documented spelling
and given the two assertions that would have caught it.

Known and NOT fixed here: the documented `as vector<single>` cure leaks its store when
consumed directly by a field store or call argument (loft#897), so the doc example binds
it to a local first.

### A write through a returned struct is now reported (loft#894, 2026-08-13)

`hurt(first(s), 10.0)` writes into a temporary discarded one instruction later, while
`hurt(s.es[0] ?? E {}, 10.0)` writes through — same types, no diagnostic on either. This
is the shape `lost-write` exists to catch and it was silent, so the analysis now covers
its second shape. Semantics unchanged.

Two facts must meet, and requiring both is what separates a LOST write from a merely
pointless one:

* the callee WRITES THROUGH the parameter — read off its own body with
  `find_field_written_vars`, the same walk `check_ref_mutations` uses to decide whether a
  `&` parameter was really mutated, so the two cannot disagree about what such a write is;
* the argument COPIES A PLACE THE CALLER CAN STILL REACH — read off the return type's
  deps, since `first(s)` returns `E["s"]` while a value built from nothing is dep-free.

The second condition is the one that matters. Without it the lint fires on
`hurt(fresh(), …)`, where a write into a freshly built value loses nothing that existed
before the call, and on the write-then-return builder idiom, where the write is delivered
through the return value. A dep is believed only when it names a parameter the call site
filled with a REAL variable: a function building into a caller-supplied return buffer
carries a dep too (`alloc_canvas(w, h, fill)` returns `Canvas["cv"]`), and that copy is
nobody's data — the `_`-prefix test tells them apart, the same convention
`warn_dead_stores` uses.

Both exclusions were found by sweeping all 2188 `.loft` files with the lint as a probe
before letting it speak: the first cut had two hits, both the builder idiom, and the final
one has zero. Runs from `main` beside `warn_dead_stores` / `warn_double_move`, reusing the
`lost-write` code rather than minting one (same fact about the same C86 copy);
`LOFT_NO_LOST_TEMP_WRITE` opts out.

Deliberately an under-approximation, per the two-tier rule: binding the result to a local
first stays silent, because that copy is still readable and belongs to `warn_copies`.

### `=` to a keyed collection appended, because only the EMPTY literal cleared (loft#895, 2026-08-13)

A collection literal lowers to element-construction ops that APPEND, so the assignment has
to put a clear in front of them. `parse_assign_op`'s vector-field arm does. The keyed arm
did it only for `Value::Insert(ls) if ls.is_empty()` — `s.h = []`, the @P307 clear — and
said so in place: *"Non-empty / non-literal keyed-field reassignment is a separate (harder)
case left to its current path."* So `s.h = [a, b]` added to what the field held, and `=`
meant `+=`.

The filed scope was a struct with two keyed fields, where the second assignment read length
4 for two elements. The matrix says that is not the boundary. Assignment ORDER is
irrelevant — the row filed as correct fails too, 4 the other way round — and a SINGLE keyed
field assigned twice is equally wrong, as is a keyed LOCAL, which has no struct at all. The
pair is just the loudest witness, because `Field.other_indexes` makes two keyed fields over
one element type two views of one record set (loft#843), so filling either fills both.

Two arms now carry the clear: the field one prefixes any literal with `OpClearKeyed`, and
the local one prefixes `Set(v, Null)` — the lowering `s = []` already takes (P193
`create_keyed`), which codegen turns into the `OpDatabase` store reset, and which also
gives the slot its init when a literal is the local's first assignment.

A MULTI-INDEXED field is excluded and keeps the append (loft#898). `OpClearKeyed` →
`remove_claims` frees the element RECORDS, not just this route to them: `Parts::Array |
Ordered` hands every slot back with `owning_elem: Some(elm)` unconditionally, and a
borrowing `Parts::Hash` does the same whenever `owns_entries` is false. So both members of
a group free the shared records and whichever is cleared first takes the other's elements
down with it — `h.ordered = []` leaves `h.keyed` reporting length 2 over freed memory. That
is a use-after-free, and emitting the clear there would trade #895's wrong length for it.
`allocation.rs:2921` already carried the marker: `// TODO prevent removing records twice via
secondary structures`. The exclusion reuses `keyed_field_is_linked`, the same predicate
@P305 uses to route `coll[key] = value` away from the group for the same reason.

The empty literal keeps its unconditional clear either way — making that one conditional
would restore the silent no-op @P307 fixed, so the change is strictly additive.

### A field-store RHS temp was typed as the destination, so it never owned anything (loft#897, 2026-08-13)

`s.v = <expr>` lowers to `Set(tmp, expr); Clear(s.v); Append(s.v, tmp)`. `tmp` was built
with `f_type` — the destination FIELD's type, deps included. scopes.rs frees a var only
when its deps are EMPTY (*"`dep` empty → the variable owns the value → emit `OpFreeRef`"*),
so a temp carrying the field's dep read as a borrow of the struct and no free was ever
emitted. Any allocating RHS then leaked for the life of the program.

Nothing about the `as` cast was involved, which is what the filed scope named. A local was
clean only because a user local carries no such dep — `LOFT_VAR_TABLE` shows both temps
marked `OWNS`, and the difference is entirely whether something BINDS the value. The
borrowed-Var arm two branches up already builds its temp from a dep-free
`Type::Vector(elm, Deps::none())` for the #320 aliasing reason; this is that same choice on
the general arm, which is the one an allocating RHS reaches.

The other half of the filed scope — the same expression consumed with NO binding — is not
an ownership question and is loft#899. `#reading file`'s temp DECLARATION is lifted into an
expression slot there (`{ !! INSERT _read_1(5):vector<single> = null … }`), which
`--interpret` evaluates against the wrong header (`len` answers 1; `for e in` yields the
second element alone) and `--native` emits as a `let` inside an expression, so rustc
rejects it. The emitted `OpReadFile(…, db_tp=78)` is byte-identical between the working and
broken programs, so the read op is not what differs. It is also order-sensitive: an
unrelated `vector<single>` local elsewhere in the file flips the answer, which is what a
type-registration side effect looks like. Fixing the leak on a path whose value is wrong
would have been polishing, so this fix stops at the field store.

### An unbound `f#read(n) as vector<T>` failed three ways, from two causes (loft#899, 2026-08-13)

The order-sensitivity was the tell, and it named the first cause. `gen_set_first_vector_null`
resolves its store type by NAME — `data.name_type("main_vector<single>")` — and the read's
temp is the one vector local that never reaches an assignment, so nothing registered the
wrapper: every other vector local gets it from `Parser::change_var_type`, and the
`typedef.rs` sweep that catches the remaining producers reads struct and enum-value FIELDS
only. The lookup returned `u16::MAX`, and the emitted `OpDatabase(var, db_tp=65535)` created
the store with no type at all. Wrong header width, so `len` answered 1 and the data started
one element in — and any OTHER `vector<single>` in the file registered the wrapper as a side
effect and made the same read correct, which is why a line elsewhere changed the answer.
`objects.rs` now calls `data.vector_def` for a vector read type, the same call its
`OpCastVectorFromText` sibling makes 800 lines down.

The `debug_assert_ne!` guarding exactly this `u16::MAX` sat one line below the lookup and
has never run: `[profile.dev.package.loft] debug-assertions = false` strips it from the
library in both profiles. An env-gated `eprintln` in its place, swept over all 2190 corpus
`.loft` files, found this temp to be the ONLY producer — and found no corpus file that
covers it, which is how it shipped.

The other two failures are one mechanism. The `Value::Block` arm returns a value block that
yields an owned temp as `Insert([Set(v, Null), block])`, and `scan_args` hoists that `Set`
into the enclosing statement list (`is_a56_hoisted`). But `scan`'s `Value::Span` arm rewraps
the scanned argument, and its unwrap predicate recognised only the `Set(__lift_N, …)`
preamble — so a span-wrapped null-init preamble never reached the `if let Value::Insert`
that would hoist it, and the declaration stayed inside the argument expression. Native
emitted it there literally: `expected expression, found let statement`, plus an E0425 for
the `var__read_1` that no longer scoped. That arm's own comment already gave the reason the
lift shape is unwrapped — *"the native backend would emit `Set(__lift_N, …)` inside an
enclosing expression and fail to compile"* — for a sibling shape it did not cover. The two
sites now share one predicate, `is_null_init_preamble`, so they cannot drift again.

Hoisting alone left the store unfreed, because the hoist MOVES the owner: the declaration
now stands in the enclosing statement list, and an argument is only read, never adopted the
way `v = <block>` adopts. `scan_args` re-registers the temp at the current scope for
`get_free_vars` and runs `mark_lift_handoff` on it, so an argument the callee MOVES from
(`OpCopyRecord` with the `0x8000` flag) still does not drop twice. `return f#read(…)` is the
other side of that and must NOT be freed; it transfers, and the guard's c8 row pins it.

Element type is an axis here, not a detail. `main_vector<integer>` is registered by the
stdlib whatever the program does, so an integer-element probe sees the leak and the native
failure but never the wrong value. The same masking bites the regression guard itself: any
control row that binds the read to a local registers the wrapper and disarms the very cell
it pins, so the `vector<single>` case needs a file that declares no other vector at all
(`899-unbound-file-read-only-vector.loft`, deliberately minimal for that reason) while the
main guard carries the remaining seven shapes plus a `vector<P>` row.

### The last local-gate flake: a well-known port on a shared machine (2026-08-13)

`engine_host_udp::probe_server_poses_ride_the_fastest_path_per_client` connected to a
hardcoded **18084**, because the fixture it drives —
`tools/audience-demo-50/probe_server_kernel.loft` — binds that constant. Its own comment
said so, and named the fix: *"this one test can still collide with a concurrent
sibling-checkout run; fixing that needs a port-arg on the fixture."*

On this machine 18084 was held by **five** long-lived processes from other checkouts
(`planet_server-a`, `planet_server-e`, `loft_native_bin`), so the test failed for someone
else's run — every run, not intermittently.

The fixture now honours `LOFT_PROBE_PORT`, defaulting to `PORT` when unset
(`env_variable` answers `""`), so the documented demo invocation is byte-identical. The
test passes a port from a new `free_port()` helper.

`free_port()` checks **TCP and UDP**: the kernel listens on both for one number, and the
OS picks a TCP port knowing nothing about the UDP table — a TCP-only probe would hand
back a port whose UDP half is taken, and the fast-lane assertions would then fail for a
reason unrelated to the code under test. `SO_REUSEADDR` is deliberately not set, since a
port that only looks free because of address reuse is not free.

Candidates come from **20000–29999 keyed on the pid**, not from `bind(":0")`. The first
attempt did use `bind(":0")` and a full-suite run then failed
`engine_host_placed::the_engine_host_serves_the_same_client_from_either_placement`, which
passes 6/6 in isolation on both this tree and the preceding commit. `bind(":0")` draws
from the OS ephemeral range (32768–60999 on Linux) — the same pool every other test's
port probe draws from, including that one's TCP-only `free_port` — so it traded a
collision with a *well-known* port for a collision with a *sibling test*, which is harder
to recognise when it bites. A pid-keyed number in a quiet range separates concurrent
checkouts and stays out of that pool.

`engine_host_placed` still has its own TCP-only probe. It is latently exposed to the same
UDP half-taken hazard; left alone here because nothing has been measured failing on it
once the ephemeral contention is removed, and a speculative rewrite of a second test's
networking is churn.

Verified by binding: with `LOFT_PROBE_PORT=19731` the fixture listens on 19731 for both
UDP and TCP. The test's own timing is corroboration — it now connects to the port
`free_port()` chose, so a fixture that ignored the variable would sit on 18084 and
`ws_connect` would spin to its 15 s deadline; it completes in ~0.12 s instead.

That closes the third and last of the session's flakes, all one shape — **a fixed-name
shared resource plus parallelism**: a process-global overwritten per compile, one temp
path for four callers, and a well-known port. Local gate: **4079 passed, 0 failed**.

### A test helper's temp file was keyed on the pid, so its four callers shared one path (2026-08-13)

`tests/introspect.rs::resolution` wrote its program to
`temp_dir()/loft_res_<pid>.loft`, ran `loft introspect` on it, and deleted it. The pid
is the same for every test in the binary, so all **four** call sites shared one path —
and `why_reports_where_a_name_is_defined_and_reachable_from` alone calls it twice. On 8
threads one call's `remove_file` landed while another's subprocess was still opening the
file; the subprocess printed nothing and `section()` panicked with ``no `=== resolution
===` in:``.

Measured at 4/6 failing runs of the binary, and 3/6 on the preceding commit — pre-existing,
and the second of the two flakes behind the local gate's "4077 passed, 2 failed". It passes
100 % in isolation, because the race needs a second caller in flight.

Fixed by making the path per-CALL (an `AtomicUsize` counter alongside the pid) rather than
per-process; the pid still separates concurrent `cargo test` invocations. 8/8 clean after.

The wider pattern is worth knowing when writing a test helper here: **a fixed-name shared
resource plus parallel tests**, the same family as the hardcoded ports in
`engine_host_udp.rs` (18084) and `multiplayer` (18099). 70 test files call
`std::process::id()`; most already add a per-test discriminator (`{name}`, `{tag}`,
`{port}`), and a pid-only name is only safe where exactly one caller exists.

### The `#native` stub set was a process-global that every compile overwrote (2026-08-13)

`compile::byte_code` recorded which `#native` symbols it registered a panic stub for —
the set `wire_native_fns` consults to know which stubs it may replace with an
auto-marshalled wrapper — by *overwriting* a `static STUB_SYMBOLS`:

```rust
pub fn set_stub_symbols(syms: HashSet<String>) {
    *STUB_SYMBOLS.lock()… = Some(syms);   // wholesale, on every compile
}
```

In any process that compiles more than one program — a test binary, the REPL loading a
second file, an embedder — a sibling compile landing between one program's compile and
its wiring replaced the set. `wire_native_fns` then hit `!stubs.contains(sym) → continue`
in **both** phases, skipped resolution for its own symbols, and left the panic stub in
place. The failure surfaces much later, at the first call, as *"native function not
loaded: its library's native cdylib is missing or stale"* — a message that sends the
reader after a build problem that does not exist. Diagnosing this one burned time on
exactly that: rebuilding cdylibs, and checking `nm -D` for undefined `libloft` symbols
to rule out staleness (there are none — the fixture links only `loft-ffi`).

**Fix: the set lives on `State`.** It describes the program that was just compiled, and
`register_native_stubs` already had the `State` in hand (`state.static_fn(sym, stub)` on
the line above). `STUB_SYMBOLS` and `set_stub_symbols` are deleted rather than kept
beside the new field, so there is one home for the fact. A lock around the global would
have been the wrong fix — it serialises the writes and still lets the last writer win.

Cost measured before the fix, by worktree A/B against the preceding commit, alternating
single RUNS with both arms pre-built (binaries *and* test binaries) so no build sat
between them:

| | `repl_session` not-pass |
|---|---|
| A — preceding commit | **5 / 10** |
| B — same tree + the vector-leak and panic-hook commits | **5 / 10** |

Identical rate, identical failure mode: `file_debugger_can_call_into_a_native_library`
fails about half of all full-binary runs and passes 100 % in isolation, because it needs
~52 sibling compiles in the process to lose the race. That is almost certainly the single
failure behind earlier "3972/3973 curated" gate reports.

`tests/native_loader.rs` already carried a `TEST_LOCK` whose comment named
`STUB_SYMBOLS` as shared global state — a workaround that could only ever cover tests in
that one file, and `repl_session` is a different binary.

Guard: `native_loader.rs::a_sibling_compile_does_not_take_over_this_program_s_stub_set`
reproduces the interleaving **deterministically** — compile B, compile a sibling
declaring a *different* `#native` symbol, then wire and run B — so it fails outright on
the old code instead of depending on thread scheduling.

### A buffer-bound vector fn delivered only its TAIL when the tail borrowed an argument (2026-08-13)

`dispatch_vector_delivery` is the one place that decides how a vector-returning function's
result reaches the caller's `__retbuf`. `Delivery::Rename` routes through `ref_return`, which
delivers the tail AND rewrites every mid-body `return <fresh local>` into the buffer.
`Delivery::CopyBorrow` routes through `copy_borrow_tail_into_retbuf` — a **tail-only** funnel,
by design and by its doc comment. So a function with an early `return <fresh local>` and a tail
that borrows an argument delivered the tail and left the early return handing back a store of
its own: the caller adopts the buffer, the fresh store orphans. One leaked store per
undelivered return, every value correct.

The invariant, now asserted in both arms: **a buffer-bound vector fn delivers EVERY return site
into the buffer, not only its tail.** The `CopyBorrow` arm calls `deliver_mid_vector_returns`
before the tail copy; `copy_borrow_tail_into_retbuf` keeps its narrower tail-only contract. The
walk is idempotent by construction (it rewrites `Return(Var(v))` only for `v != buf_var`, and
its own rewrite yields `Return(Var(buf_var))`), so the existing fallback — which delivers again
via `ref_return` when the work-var allocation fails — cannot double-deliver.

Boundary, mapped on a 14-cell matrix before the fix (values hand-computed, each cell asserting
value + length + leak, both backends):

| tail shape | mid-body payload | pre-fix |
|---|---|---|
| borrows a whole vector argument | fresh local / inline literal | **leak** |
| borrows a struct FIELD of an argument (#415) | fresh local | **leak** |
| a call / a fresh local / the same var | fresh local | clean |
| branch arms (`Delivery::Materialize`) | fresh local | clean |
| borrows an argument | a param / a call result | clean |

Both leaking rows are the two sub-shapes the funnel's own comment says route to it, so the
boundary is the `CopyBorrow` arm exactly. The leak count scales with the number of undelivered
returns — a two-early-return function leaked ×2 — which is what pinned the mechanism rather
than merely correlating with it. `Delivery::ForwardCopy` needs a `#native` heap-returning
callee and is **not** covered by the matrix; it shares the tail-only shape and is the place to
look if this recurs.

Guard: `tests/scripts/midbody-return-into-borrow-tail-retbuf.loft` — both sub-shapes, a
two-early-return function (guards the COUNT), a loop-nested return, and a caller loop proving
the delivery clears before filling. Proven to fail on the released binary: ×9 leaked stores
while still printing `ok`, so only `wrap.rs`'s exit-time gate catches it.

Found while probing whether `OpReplaceVector`'s absence from `find_written_vars` /
`find_field_written_vars` could give a wrong answer. It cannot today — all 9 of its occurrences
across the stdlib and the dump corpus are masked by an `OpClearVector` or a `Value::Set` on the
same target — but the masking is incidental, so the op is now listed in both walkers as
hardening. That is a no-op on current behaviour, kept because the two lists are hand-maintained
twins and nothing compares them (PERFORMANCE.md § Design: P8).

### Two nightly gates that measured the environment, not the diff (loft#888, 2026-08-13)

**The leak gate went red on our own fix.** loft#876 gave a field's declared default a home on
the schema `Field`, and a TEXT default has to intern its spelling: `Content::Str` is a raw
`{ptr, len}` with no owning variant, so `fold_declared_default` reaches for the same
intentional `Box::leak` that `ir_read` / `ir_schema` / `snapshot` already use for the same
type. That leak is bounded by the SOURCE — one allocation per field that declares a text
default, decided once at type registration, never one per read — so it belongs in
`.github/lsan_suppressions.txt` beside its three siblings.

What kept it out was purely a symbolization detail: a suppression matches by FRAME NAME, and
the function inlined into `typedef::fill_database`, so the only name on the stack was that
one. Suppressing `fill_database` would have blinded the gate to every allocation in the whole
type-registration path — a real loss, since that is where schema construction allocates. So
`fold_declared_default` now carries `#[inline(never)]` FOR the suppression, and the two must
be kept in step: drop the attribute and the suppression silently stops matching.

Both halves are measured rather than argued. Without the suppression the frame is now named
(`#2 loft::typedef::fold_declared_default`, `#3 fill_database`); with it, the run is clean and
LSan reports the template it used. The per-file scan over the whole corpus is **0 leaking files
of 721**. And the gate is still live where it matters: a deliberate `Box::leak` injected into
`fill_database` itself is still reported, with the suppression file active, owner
`loft::typedef::fill_database` — so the new line suppresses exactly one deliberate interner
and nothing else.

**The toolchain matrix failed before running a loft op.**
`a_private_scope_end_hook_in_a_library_runs` spawns loft to BUILD a library cdylib, which links
`libloft.rlib`. The `Suite under <toolchain>` job only ever runs `cargo test`, which builds the
lib into `deps/` for the test binaries and never produces the rlib
`native_lib::find_loft_rlib` looks for — so the spawned build died on "libloft.rlib not found
for this build". That is an environment result, and this matrix exists to detect toolchain
drift in loft's own code.

The obvious repair does not work, which is why it is recorded here rather than tried again:
adding `cargo build --release --lib` clears "not found" and then fails `E0463: can't find crate
for libloading`, because `cargo test` and `cargo build --lib` unify features differently, so
the uplifted rlib's dependency set is not the one sitting in `deps/`. Both cells were run in an
isolated target dir; cell A reproduces the CI message verbatim. The test is therefore skipped
in that job, which is the exclusion the asan and asan-leak jobs already carry for it and for
the same reason (loft#855). Its sibling `a_delegating_producer_binds_its_companion_cleanly`
passes there and is deliberately NOT skipped.

The third leg needed no change: the `LOFT_POISON` gate was red on
`877-index-a-call-result-in-return-position.loft`, which is loft#882 / loft#889 / loft#890, and
the poison sweep now runs **1870/1870** on this branch.

### Two stores freed at the wrong time (loft#889, loft#890, 2026-08-13)

**loft#890 — a lift freed what its consuming op had already released.** `br = mk_hash(n)`
lowers to `__lift_1 = mk_hash(n); OpReplaceKeyed(__lift_1, br, tp | 0x8000)`. The bit means
"nobody else owns this store", which is true of a bare call result and false the moment
`scan_args` lifts it into a temp the scope sweep frees. `free_named` is a no-op only while
the slot is still free, so the second free steals whatever store the allocator handed that
slot in between — and the record return allocates its buffer in exactly that window, which
is why the filed shape needed a call, a keyed container AND a record return together. With
an integer return nothing is allocated there and the double free is invisible. The
interpreter was right for no better reason than not reusing the slot.

`scan_args` already carried the lift-site half of this rule for `OpCopyRecord` (@PLN139
stage C), but only for the DROP — the store was "left to the ordinary sweep, which is
null-tolerant either way", which is the part that is not true. `Scopes::mark_lift_handoff`
now records the FREE hand-off too, and `get_free_vars` consults it. Only `OpReplaceKeyed`
answers `moved_source_arg`: it is the one `0x8000` op whose source is a whole store a lift
can own. Answering for `OpCopyRecord` took the free away from @PLN85's Join-return
machinery — 3 of 54 fuzz cells SIGSEGV'd and `elem_accumulate` doubled its own source
vector. The marker is a `Scopes` set rather than a `skip_free` stamp for the same class of
reason: that flag is read at ALLOCATION time too, so stamping it made the lift borrow
instead of own.

**loft#889 — a collection reached through a field of a call's result.** `mk_bag(n).b_vec[0]`
reads an element living in the bag's store, and the bag is an inline call result with no
name, so the element typed as OWNED and nothing copied it out before `OpFreeRef`.
`keyed_container_dep` (loft#882) is now `container_dep` and reaches THROUGH field
projections via `projection_root_mut` to bind the ROOT call — the bag, not the `b_vec`
projection, because that is whose store the element is in. `parse_index`'s VECTOR arm asks
too; it had relied on the container type's own deps, which a fresh call has none of.

The SUBSCRIPT asks, not the field read. `return make().rows` returns the field itself,
which the delivery machinery already copies out (loft#877 / zt12), so binding a container
there only adds a holder nothing releases — five of them in that one file.

The binding now happens on BOTH parser passes. That is load-bearing: this dep is what tells
`ref_return` the binding borrows, and a verdict that differs between passes is worse than
none. Skipping pass 1 read the binding as owned and renamed it ONTO the return buffer; pass
2 then saw the view and materialised into the buffer the binding now WAS, so
`materialize_return_into` emitted `OpDatabase(e); OpCopyRecord(e, e)` — a copy from the
record it had just re-minted. `e = mk_hash(n)[k] ?? d; e` answered an empty record for that
reason before this issue existed, so loft#882's own shape had the hole one bind away.

`return_projects_into_local` gained `projection_base`, which peels the binding block to its
var: a base that is neither a var nor a call read as "rooted at nothing", and the field was
delivered as if it owned what it points at.

Guarded by `tests/store_lifetime_890_889.rs` over `tests/scripts/{889,890}-*.loft`: value,
`LOFT_POISON=1` and `LOFT_NATIVE_LEAK_CHECK` on both backends, plus a harness control. The
poison oracle because freed bytes are usually still intact; the leak oracle because "never
free it" ends both use-after-frees while passing every value cell. loft#888's nightly
poison gate was red on `877-index-a-call-result-in-return-position.loft::i877_field_of_call`
— loft#889's cell, recorded there and invisible because the suite does not run that file
under poison — and is green with this.

`723-ncc-loop-element-bind.loft`'s leak check now measures round-over-round inside ONE
frame: a container work-ref is one slot per SITE for the life of its frame, so two snapshots
taken at different sites differ by that constant and say nothing about the round.

### A sorted collection dropped every insert once an index existed elsewhere (2026-08-13)

`s[k] = v` on a `sorted<T[k]>` local inserted nothing — `len(s)` answered 0 and every lookup
its fallback — whenever ANY struct in the program declared an `index<T[…]>` field over the
same element type. The struct is never constructed; declaring it is the whole input. Both
backends.

A `sorted<T[k]>` becomes an `ORDERED<T[k]>` — the by-reference twin — under exactly that
condition, so the same source line lowers differently because of a declaration somewhere
else entirely. `towards_set`'s insert arm listed Hash, Sorted, Index, Radix and Trie, so an
`Ordered` collection fell past `OpSetKeyed` to the update-only `OpCopyRecord`, which copies
into the lookup's result and therefore no-ops when the key is absent.

This is loft#719's omission one function over: that issue gave `Ordered` to the REMOVAL arm
(`towards_set_hash_remove`, directly above), where its absence had made `coll[key] = null` a
silent no-op interpreted and a compile failure natively. Nothing compares the two lists.
`Stores::set_keyed` has always handled `Ordered`; only the routing to it was missing.

Found while building loft#889's boundary matrix: a five-collection bag promoted its own
`sorted` field, and that one cell answered the fallback on both backends while every
neighbouring kind was right — a lopsided matrix that is evidence of a missing arm rather
than of the bug under investigation. Guarded by
`tests/scripts/891-sorted-promoted-to-ordered.loft`, which fails at its first assertion on
the previous commit.

### A keyed element read never said it borrows its container (loft#882, 2026-08-13)

`v[i]` on a vector types its result with a dep naming the container, and that dep is the
whole reason the vector shape is safe: `return_views_local` sees a borrow from a local and
`materialize_view_return` copies the element into the return buffer before the container is
freed. Every keyed read — hash, index, sorted, trie, any key arity — carried none, so
`return make_hash()[k]` handed back a pointer into a store the same function freed on the
way out.

`parse_index` propagates the container TYPE's deps (`for on in t.depend()`), and a freshly
built collection has none to propagate. `Parser::container_dep`
(`src/parser/fields.rs`, then named `keyed_container_dep`) now names the container at the one place keyed element reads are
typed: a local, parameter or field is depended on directly; an inline call that MINTS a
container is bound to a pass-2 work-ref first, because `scopes.rs` lifts it into a
`__lift_N` long after the materialisation decision has been made. A parameter's dep
resolves to a function attribute, so the element correctly stays a borrow.

The two backends disagreed, which is why it survived: an EMPTY dep list reads as OWNED by
`--native`'s assignment lowering, so it inserted a defensive `OpCopyRecord` and the program
was right, while the interpreter aliased and read freed bytes. Under `LOFT_POISON=1` the
boundary matrix scored `--interpret` 6/17 and `--native` 14/17; both are 16/17 now.

The filed cause (`parse_key`'s no-prelude branch) was not the boundary: the prelude branch
attaches `dep.clone()` — the container type's deps, which are empty — so it named the
container no more than the other branch did, and BOTH spellings were broken. The two cells
still red are older and separate: loft#889 (a collection reached through a field of a call's
result) and loft#890 (a bound keyed container on `--native` when the function returns a
record — the workaround the issue was filed with).

Guarded by `tests/keyed_element_borrow.rs`, which runs under `LOFT_POISON=1` on both
backends plus a static oracle (the container must be NAMED and the return MATERIALISED), a
leak check and a harness control. It needs its own binary because freed bytes are usually
still intact — the ordinary suite was green over this.

### A registered native with no bridge was only found by calling it (loft#886, 2026-08-13)

A cdylib can export a `#native` symbol and register no marshal bridge for it. The symbol
resolves, wiring succeeds, and `native_auto_dispatch` panics — but only when something
calls it, so a library can ship, pass its own suite, and carry a function that is dead for
every consumer exercising a path its tests do not.

`wire_native_fns` now collects those symbols and reports them at load
(`report_bridgeless_natives`), separately from `report_unresolved_natives` because the fix
differs: the library is not stale, its registration is incomplete. The message names the
library and each dead function and points at
`loft_ffi_build::generate_register_from_loft_with_bridges`, which derives both the register
list and the bridge list from the `#native` annotations and cannot drift — a hand-written
`loft_register_bridges!` lives in a different file from the declarations and nothing
compares the two.

The issue's stated cause — a non-`pub` `#native` taking a vector gets no bridge — does not
reproduce: a 9-cell package varying visibility against parameter kind, call site and symbol
binding is correct in every cell on both backends, and `parse_register_symbols_from_loft`
strips an optional `pub ` and never looks at it again.

### A repeat literal walked off the store on a negative count, and lost its text (2026-08-13)

Two further defects in `[x; n]`, found while reading the bulk-fill path before routing a
constant comprehension into it (loft#884).

A NEGATIVE count cast `as u32`, so `[7; -1]` became 4 294 967 295 `copy_block`s that walked
off the store until glibc aborted — the same failure `n == 0` had. A count is a TOTAL and a
negative total is no vector at all, so it now answers empty. `--native` already clamped with
`count.max(0)` and the interpreter did not: a heap-corrupting input on which the twins
disagreed.

The claim copy took the VECTOR HANDLE as its source instead of the template element, so a
`text` element re-interned whatever the handle's four bytes decoded to: `["abc"; 4]` gave
"abc" at index 0 and junk at 1, 2 and 3. Structs and nested vectors carry claims too and
were wrong the same way. Length and element 0 were both correct, which is what made it
invisible.

The twins are also back in step on the record re-read: growing the vector can move its
backing record and both ends of the copy live inside it — `--native` re-read it for the
destination only, the interpreter not at all.

### `[x; n]` built n+1 elements, and n=0 corrupted the heap (2026-08-12)

`OpAppendCopy` receives the TOTAL a repeat literal asks for, and the template element is
already appended by the time it runs — so it needs `n - 1` more. It added `n`:
`vector_set_size(&data, multiply, size)` grew the vector one past the request while the
copy loop wrote only `multiply - 1` slots, leaving the last one never initialised. `[7; 3]`
read back as **length 4 with garbage in the last element** — a wrong length and an
uninitialised read, silently, on both backends.

`n == 0` is the same off-by-one taken to its end: `for i in 0..(multiply - 1)` on a `u32`
wrapped to 4 294 967 295 and walked `copy_block` off the end of the store until glibc
aborted the process (`Fatal glibc error: malloc.c:2599 (sysmalloc): assertion failed`).
The template also has to be dropped, or a zero-length request answers length 1.

The op's contract is now "the vector ends with exactly `count` copies of its last element",
which is what the literal means, and it is total: 0 removes the template, 1 is already the
answer, n adds `n - 1`. Fixed in BOTH twins — `State::append_copy` (`src/state/io.rs`) and
`codegen_runtime::OpAppendCopy` — which carry separate copies of the loop.

Both halves reproduce on the published `2026.8.0`. Found while measuring loft#884: the
repeat literal is the bulk-fill path a constant comprehension would be lowered into, so it
was read before being built on. Guarded in `tests/scripts/886-repeat-literal-count.loft`,
including a RUNTIME count and a runtime zero — the rows no const-fold can reach — and a
`float` element so a stride error shows as a wrong sum rather than a wrong length.

### A declared field default now reaches a cast, when it is a constant (2026-08-12)

`height: float = 1.5` was honoured by a struct literal and ignored by `text as Struct`,
which wrote the type's zero — the same field with two absent values depending on how the
record was made. Invisible before loft#870, because the cast answered `null` for all
three cases and the value was wrong in a louder way.

The default lives parser-side as a `Value` IR node and the JSON walker sits below the
parser with no evaluator, which is what made this `needs-design`. Of the three possible
answers, the one taken is folding a CONSTANT default into the schema `Field`
(`typedef::fold_declared_default`) — it needs no evaluator, and it comes with a contract
to state rather than a hole:

* a LITERAL default (`= 1.5`, `= 7`, `= "hi"`, `= true`) is part of the type: it answers
  a missing key, an explicit `null`, and a struct literal alike;
* any other default is computed, is already lowered parser-side into a function the
  CONSTRUCTION site calls, and keeps exactly its previous reach — the constructor, not a
  cast. Documented in LOFT.md § struct fields, and pinned by a probe cell rather than
  left implicit.

Deposited at the one parse-time site that knows (`typedef::fill_database`), beside the
`nullable` deposit and for the same reason. Three details carry the weight:

* **The value goes in through `walk_parsed_into`** — the same writer the cast uses for a
  key the document DID carry — so a default lands exactly as if the JSON had spelled it,
  and every field encoding (ranged `u8`/`u16`, text interning, the `Parts` dispatch) is
  handled in one place instead of restated. A literal that does not fit its field writes
  nothing and the previous absent value stands.
* **It is written only for `Absent::Final`.** A `Prefill` is overwritten by whatever
  follows, so honouring a default there would pay back the per-record cost loft#875 split
  the enum apart to avoid.
* **It is carried, never RENDERED.** A default changes no width and no offset, so
  `layout_dump` must not see it — otherwise `height: float = 1.5` becomes a different
  layout from `height: float` and adding a default would refuse an existing store. The
  dump's default branch is removed; it never fired, because every field held the `Str("")`
  placeholder. Same call `nullable` made (@PLN127 arc D).

`Field::default` becomes `Option<Content>` (it was `Content`, set to `Str("")` at every
site and read by nothing). The snapshot and IR-store formats are unchanged: `None` is
written as `Str("")` and read back as `None`, so an existing schema round-trips
byte-identically. `--native` needed its own half — the generated `init()` replays the
schema, so `emit_field` now emits `set_field_default`, folded by the same function, which
is why the two backends cannot disagree about which defaults are constant.

Matrix: 9 cells on both backends, 7 failing on the published `2026.8.0`; the negative
controls (a present key beating the default, a field with no default keeping loft#870's
answers, a literal override) pass on both. Guarded in
`tests/scripts/876-declared-field-default-in-cast.loft`.

### An optional return was a shape the lift never recognised (2026-08-12)

`inline_struct_return` (`src/scopes.rs`) is the one predicate that answers "does this
call hand back a store the caller must own?", and every arm matched the callee's
return type UNPEELED. `Optional(τ)` is a compile-time wrapper over τ's own runtime
layout (@PLN25), so `-> C?` allocates and delivers exactly what `-> C` does — but it
read as "not liftable", and the result got a bare stack-pop (`FreeStack`) instead of a
`__lift_N` temp with a scope-exit `OpFreeRef`. One leaked record per call, unbounded in
a loop, interpreter-only (native frees through its own drop path).

Filed as loft#879, a `??` bug. The `??` is incidental: a discarded `pick(1);` leaks the
same store with no `??` anywhere, `takeopt(pick(1))` leaks it as an argument, and an
optional VECTOR return leaks too. The deciding axis is the optional aggregate return
whose result stays a temporary — not the spelling that produced it.

The `??` half is a second arm. A null-coalesce lowers to an `ncc` value-block that
assigns the subject to a `__ncc_N` temp and yields either that temp or the default arm's
`__ref_N`. The temp is `skip_free` — the block's result ALIASES it, so freeing at the
block would dangle the value the consumer reads — which leaves the subject owned by
nothing when the block is used inline. Text ncc temps were already covered by the
@PLN85 skip_free-orphan pass and vectors by their own delivery path; only the
`Reference` result leaked, and only that arm was added.

Both halves emit what the hand-correct bound form has always emitted: `x = pick(1)`
binds an `optional(reference(C))` local and frees it at scope exit. The lift rewrites
the inline spelling into that bound form, so the fix adds no new delivery path — which
is also the soundness argument for the borrowed case (`fn kid(h) -> Cell? { h.child }`),
since binding one has always been clean.

Boundary matrix (11 cells, both backends, `scripts/probe-matrix`): 7 cells fail on the
published `2026.8.0` and pass here; the 4 negative controls — the default arm, a
borrowed optional view, a non-optional return, and store-free optional scalars — pass
on both, so the matrix is not green by having stopped checking. Guarded in
`tests/scripts/174-inline-temp-free.loft`, the file that already owns this class.

### One call emitter re-derived the Rust fn identifier (2026-08-12)

Emitted Rust is one flat namespace, so two same-named fns from different files get a
file-hash suffix on the DEFINITION (`disambiguated_fn_ident`, #305). `Output::fn_ident`
is the chokepoint, and its doc says every site writing a definition OR a call must go
through it. `dispatch.rs`'s adopt-or-copy bind — `{ let _dst = …; let _src =
<callee>(cell, …)` — wrote `callee.name()` instead, so the call named a `fn` that had
been emitted under another: `error[E0425]: cannot find function n_defaulted in this
scope`, on a package whose interpreter suite was green.

The trigger is narrow enough that both the reporter's minimisation and my first one came
out GREEN, which is what the guard's comment records. A FIRST bind of a call result goes
through `calls.rs`, which was always right; it takes the adopt path — the callee returning
a LOCAL bound from another call — to reach this emitter at all. Reproduced by
reconstructing the consumer's pre-fix state from its current source (the failing state was
never committed), then reduced to a 20-line package.

Swept the siblings: `dispatch.rs:665` delegates to `output_code_inner`, and the other
`def_fn.name()` reads are dispatch predicates on `Op*` builtins, which are never mangled.
loft#878.

### A work-ref mint landed on the return-buffer ARGUMENT (2026-08-12)

`ref_return` promotes a body work-ref to the function's hidden return-buffer argument on
pass 1, and the variable tables persist across passes BY NAME while the counter restarts —
so on pass 2 `Function::work_refs` re-minted `__ref_N`, found the argument, and
`set_type`'d it to whatever the new site asked for. `work_refs`'s own doc already stated
the rule ("a name that resolves to an argument is STEPPED OVER rather than reused") and
the body never implemented it.

The site asking was a CALL's out-param buffer: the callee was handed the caller's record
buffer as its `vector<T>` destination, cleared it as a vector and built into it. The value
came back empty and the write that followed landed out of bounds — silently on the
interpreter, as `store_nr == 65535` reaching `allocations[…]` on native.

The step-over is keyed on the TYPE, not on argument-ness alone: pass 2 re-minting the same
name for the same ROLE is how the return buffer is re-found, and a blanket step-over grew a
lambda a second return-buffer attribute on pass 2 ("grew a pass-2-only attribute", the H5
two-pass contract). `Function::retypes_argument` = argument AND `without_deps()` differs, so
only a mint that would RE-TYPE the buffer steps on. `LOFT_NO_WORKREF_STEPOVER` opts out
(`keys::work_ref_stepover_enabled`).

It subsumes half of @PLN90 W1's A1b collapse: with `LOFT_NO_A1B=1` alone the known-wrong
plan is now correct, so `oracle_flags_the_a1b_wrong_plan` disables both gates to still have
a defect to catch. loft#872.

### A container in `ls` was renamed onto a RECORD return buffer (2026-08-12)

`classify_reference_delivery`'s fallback renames the return's dep candidates onto
`__retbuf`. A tail that indexes a call — `make(n)[0] ?? d` — leaves the indexed CONTAINER
in `ls`, and that container has no further deps of its own, so `return_views_local` reads
it as owned and the rename fires: a `vector<Cell>` became the `Cell`-shaped buffer the
caller allocated. Same promotion `moros H12` hit through a field projection, reached here
through an index, which is why the guard is on the buffer's SHAPE (`ls_can_be_record_buffer`)
rather than a fourth tail walker. Both delivery arms carry it — the block tail and the
`RetSite::MidReturn` explicit `return`, which fails identically.

Only a COLLECTION blocks the rename, and that narrowness is load-bearing: a first, wider
spelling refused every non-record candidate, so a `-> Dialect?` whose value came from a
call was MATERIALISED — and materialising a null-valued record fabricates an empty one.
`registry_pure.loft`'s "a refusal speaks no dialect" caught it. loft#877.

### `ShowDb::write_hash` walked a layout that had moved (2026-08-12)

Formatting a struct with a `hash<…>` field SIGSEGV'd the interpreter in `OpFormatDatabase`
and exited 1 with no output on native; `to_json()` on the same record shared the walker and
so shared the crash. `write_hash` carried a bucket loop of its own that read each slot as a
bare record number at `pos: 8` — the layout before @PLN135 arc H moved entries into an
arena, where several share one record and `(rec, pos)` identifies an entry.

Nothing caught the drift because nothing reached it: a BARE hash is refused at compile time
(`append_data`: "Cannot format type hash<…>"), so a hash FIELD of a struct is the only way
in, and the method was marked `#[allow(dead_code)]`. It is `hash::records_sorted` now — the
module that owns the layout — which also gives the render a stated order (key order, like
`index`/`sorted`) instead of bucket order. The `max_elements` cap the debugger's glance
relies on came along with it. loft#873.

### A not-found key field was used as an attribute INDEX (2026-08-12)

`Data::attr` answers `usize::MAX` for a name it cannot find, and `set_mutable` /
`set_mutable_directed` (`src/typedef.rs`) handed that straight to
`definitions[..].attributes[a_nr]`. Any key field a keyed collection names that its
ELEMENT type does not have therefore panicked — "index out of bounds: the len is 6 but
the index is 18446744073709551615" — with a Rust source location and a caret on
whatever line the layout was reached from, which is correct as written.

Wider than the report, which arrived as the two-argument `hash<integer, At>` spelling.
The same sentinel is reached by an ordinary MISSPELLING (`hash<At[ca_kye]>`), which is
both well-formed and far more likely, and by all five keyed kinds — `hash`, `index`,
`sorted`, `spatial`/radix and `trie` — since every one of them routes through these two
helpers. Sweep of the five spellings: all ICE'd before, none does now.

The name is recorded (`Data::record_unknown_key_field`) rather than reported in place
because `fill_database` has no lexer; `fill_all` has one and drains it
(`report_unknown_key_fields`) — the same record-here / report-there split
`actual_types_deferred`'s `defer_unknown` uses. The caret lands on the FIELD that
declared the collection, which is where the name was written.

The message corrects the MODEL rather than just naming the symptom, because the way in
is a user who believes `hash<K, V>`: it says the key must be a field of the element and
shows the spelling. A did-you-mean rides `suggest_similar_capped`; failing that it lists
the element's fields, except when the element is not a struct at all (`hash<integer,
At>` puts the key in the element slot) — there it says so, because listing what
`integer` answers to would offer its METHODS as candidate keys. loft#874.


### A struct field's absent value is the FIELD's question, not the type's (2026-08-12)

`integer`(0), `long`(1), `single`(2) and `float`(3) spell absence with a SENTINEL and
share one content type between their `T` and `T?` spellings, so
`Stores::set_default_value` — which sees only `tp` — had to pick one, and picked the
sentinel. Writing that into a field declared plain put a null in a slot DN1 says cannot
hold one: the reader answered `null`, the declared type said otherwise, and
`redundant-coalesce` then advised deleting the `?? 0.0` doing the work. A ranged field
was always right (`Parts::Byte`/`Short`/`ShortRaw`/`Int` carry `nullable` in the Part),
which is what made the defect read as a float/integer oddity rather than a rule about
FIELDS.

`Field::nullable` (@PLN127 arc D) already existed and already documented why it must be
DEPOSITED rather than derived — "`text?` and `integer?` share their non-null type and
spell absence with a SENTINEL, so nothing in the store implies this". It simply was not
being asked. Three sites default a struct field and all three dropped it:
`set_default_value`'s `Parts::Struct | EnumValue` arm (recursed on `f.content` alone),
`walk_parsed_struct`'s missing-key loop, and `walk_parsed_into`'s `Parsed::Null` arm.
All three now route through `set_default_value_nullable`, which differs from the
type-only answer in exactly four arms; `field_declared_nullable` resolves `(rec_tp,
field)` and answers `true` — today's behaviour — wherever the question does not apply
(`field == u16::MAX` for a top-level or array-element target, a non-struct `rec_tp`), so
every non-struct path is byte-identical.

Wider than filed on three counts. A key the JSON OMITS is the same question as one
written `null` and had the same wrong answer. So did a FAILED parse — a syntax error or
a leaf type mismatch abandons the record at its pre-defaults, and those were sentinels
too, which is why `tests/docs/24-json.loft` and `tests/scripts/57-json.loft` both
asserted `== null` after a bad parse. And a non-null field at DEPTH follows its own
declaration, not its parent's.

Consequence worth knowing: `ShowDb::write_fields` skips a field iff `is_null`, so fields
that were wrongly null were invisible in a dump and are now printed. Parse-then-show
NORMALISES rather than echoes, and the normalised form round-trips to itself —
`tests/data_structures.rs::record` now asserts that second parse directly instead of
asserting `show(parse(x)) == x`, which only held because the omitted fields were null.

`tests/scripts/298-multi-return-site-ref-buffer.loft` reached its third return site
through `result.v == null` on a plain `integer`; `v` is `integer?` now, because the
@PLAN59 site under test would otherwise have gone unexercised while every assertion
still passed.

Still open, filed: a DECLARED default (`= 1.5`) is ignored by the walker (loft#876)
because it lives parser-side as a `Value` IR node, not in `Field::default`. The fix
attaches at the same missing-key loop below and needs an answer for a non-constant
default first — fold a constant one into the schema, refuse a computed one on a
JSON-castable struct, or give the walker an evaluator. loft#870.

### The `text` half: what an absent text field costs (2026-08-12)

`text` was left out of the fix above because a text handle of 0 IS null (`Store::get_str`),
so its empty value has to be INTERNED — and `set_default_value`'s struct arm runs per
record on the allocation path. Interning there measured +78 % wall and +91 % peak heap over
400 000 rows with three text fields, every one of them overwritten by the literal that
followed.

So the call was split by what the value is FOR (`structures::Absent`): `Prefill` — a fresh
record, whose fields the literal or the walker will write — keeps the cheap 0, and `Final`
— the value a reader actually sees — interns. Three sites are `Final`: `walk_parsed_into`'s
`Parsed::Null` arm, `walk_parsed_struct`'s missing-key loop, and `db_from_text`'s pre-parse
fill (`set_final_default_value`), that last one because a parse that FAILS reaches neither
walker and leaves the record exactly as the fill left it.

A wide first spelling — intern in every arm — also leaked: `set_text` claims a fresh record
and overwrites the handle without freeing the old one, so a prefilled empty leaked once per
text field per record (`removal-frees-what-the-element-owned.loft`: 323 → 773).

The literal keeps @PLN25's base-zero rule for a NULLABLE field (`text? → ""`), which is a
decision, not a divergence: a constructor omitting a field asks for the zero, while a
document omitting a key did not say anything. `875-json-absent-text-field.loft` pins both.

Three assertions in the suite were the demonstration and are now updated: `57-json.loft:65`,
`tests/docs/24-json.loft` and `tests/docs/23-safety.loft` each asserted `!x` on a plain
`text` — passing only because of this defect, on a line where `redundant-null-negation` said
it never could. loft#875.

### A narrow vector element got the wide store op from the comprehension (2026-08-12)

`narrow_elm_set` (`src/parser/vectors.rs`) picks the store op for an element's own width,
and its contract is that every site BUILDING a vector routes through it — its own doc
names the failure mode: "a site that misses it emits the wide 8-byte `OpSetInt` into a
1-byte slot, so one write covers eight element slots" (the slice half of #624).
`build_comprehension_code` was a third such site and went straight to `set_field`, which
dispatches on the element DEF. A narrow integer is an ALIAS of `integer`, so a 4-byte
slot got the 8-byte op.

Each element overwrote its successor; once the writes passed the initial allocation they
reached the vector's own bookkeeping and `vector_add` stopped terminating. Hence a
BOUNDARY rather than a slowdown — `[for i in 0..13 { i as i32 }]` hung where `0..12`
returned instantly, that being where the overrun first reaches the header. Measured
boundaries: i8/u8 at 17, i16/u16/i32/u32 at 13, `integer` never (8 into 8 is correct).

Two things narrowed the filed scope. `+=` was already routed through the helper, so the
append loop was clean to n=5000 and the defect read as comprehension-specific rather than
width-specific. And `r.len()` cannot see the damage BELOW the boundary: a store that
clobbers its neighbours leaves the count right and the values wrong, so the guard
(`tests/scripts/869-narrow-vector-comprehension.loft`) checks elements and a
hand-computed sum. loft#869.

### A text→heap cast was typed as a view of its source (2026-08-12)

`OpCastVectorFromText` (`State::db_from_text`) interns text into a store of its OWN, but
the `as` handler (`src/parser/operators.rs`) grafted the source's deps onto the result.
@PLN99 arc C had already established the rule for `convert`'s allocating conversions —
"the result is not a view of the source, so grafting would mark it a borrow" — and `cast`,
which has the same property, never reported it. `Parser::cast_allocates` now answers it
from the two TYPES (text source, heap target), which is what makes the verdict
pass-stable: the return buffer is chosen on PASS 1 and freezes the signature, so a
pass-2-only correction lands after the text local has already become a parameter.

A freshly allocated record therefore read as a borrow of the text it parsed, and the
return-buffer machinery delivered it as one. The symptom was decided entirely by what the
source expression was:

| cast source | interpreter | native |
|---|---|---|
| text LOCAL (incl. a literal) | renamed onto `__retbuf` — one slot both `text` and record buffer: #306 guard, then SIGSEGV | 4 × rustc E0308 (`"".to_string()` into a `DbRef`) |
| LIFTED call temp | correct | bound as the buffer, cast emitted as a bare STATEMENT, untouched buffer returned — an EMPTY vector, silently |
| PARAMETER | correct | correct |

`file()` was never part of the trigger: a text literal reproduces it, which is what says
the defect is the cast. rustc had been reporting the vector half from the other side all
along ("unused return value of `db_from_text`").

With the deps corrected, the struct target still answered null on native:
`classify_vector_delivery` has a #409 forward-copy leg for a `#rust` callee that delivers
its own store, and `classify_reference_delivery` had none — it classified `AsIs`, which
claims the tail already wrote the buffer. It gets the record twin
(`emit_forward_copy_ref_409`), and "does this tail forward its own store"
(`tail_forwards_own_store`) now has ONE home instead of two that disagreed.

Matrix: 13 probes over {vector, struct} × {tail, non-tail, bound-local, inline, no-return}
× {parameter, text local, literal, lifted call, const}, every cell value-checked by hand on
both backends plus `LOFT_STORES=warn` / `LOFT_NATIVE_LEAK_CHECK`. The spurious `File×1`
leak those shapes reported went away with the borrow that caused it.

Residue: the interpreter still prints the #306 guard twice for
`t = <text>; m = t as Struct; m` — value correct on both backends, native clean. Two
mechanisms were tried and reverted (rerouting the delivery to `MaterializeView` returned
a null; a `SkipReassigned` rung in `classify_ret_promotion` changed nothing), so the
rename is reached from a site neither covers. loft#866, loft#867.

### A member access recovered only over an identifier (2026-08-12)

`Parser::field`'s Unknown/Never recovery skips the member token so parsing can continue
past a receiver with no type yet. It consumed an IDENTIFIER only, so a numeric tuple
index stayed in the stream and the statement parser tripped on it as `Expect token ;` —
on PASS 1, and a pass-1 error means pass 2 never runs.

Two failures from that one gap. A forward reference to a tuple-returning function could
not be tuple-accessed AT ALL (`v = later("x"); a = v.0;` with `later` declared below is
legal, and Unknown on pass 1 by design). And an unresolved call was never reported:
"Unknown function" is a pass-2 diagnostic — pass 1 cannot tell a typo from a forward
reference — so the aborting `Expect token ;` left the real cause unmentioned. The
named-field and `[i]` spellings always recovered, each consuming its own member token;
only the tuple form had no consumer.

Indexing a `Never`-poisoned receiver is now silent too, matching the @P376 recovery in
`field()`: the receiver's own error is already on screen, so "Indexing a non vector" only
named a second correct line. loft#868.

### The op-sets are a property of the program, not of each question (2026-08-12)

`use_analysis` consults four op-number sets — `projection_ops`, `value_reader_ops`, the
arg-0 writers (`is_first_arg_write_name`) and the collection `len` methods
(`is_length_op_name`). Each is a pure function of the definition NAME table, so each is
constant for a given program. Each was rebuilt per question, and the walks
(`dead_store_accesses`, `collect_uses`, `Ownership::new`, `classifies_structurally`) ask
once per FUNCTION — two of the four being a full scan of every definition doing string
prefix matches. O(functions × definitions).

Measured on a `println`-sized program: **9 000 rebuilds over 708 definitions**. `perf`
put `first_arg_write_ops` at **22.7 %** of a warm-cache run — the hottest symbol in the
process — and with the `HashSet<u32>` inserts, rehashes and sip-hashing it drives, ~40 %
of startup. Warm 18.3 → 9.0 ms/run, cold 53.7 → 30.0 ms/run, against a HEAD build of the
same tree.

Cached on `Data` as `OpSetCache`, **keyed by definition count**. The keying is the whole
correctness story, and the first attempt (a plain `OnceLock`) was a measured no-op: the
first question arrives while the definition table is still growing, so pinning the answer
to that moment made every later question a miss — **7.5 M rebuilds** across the
`tests/scripts` corpus, i.e. the cache never once answered. A `debug_assert` would not
have caught it either: `[profile.dev.package.loft]` sets `debug-assertions = false`, so
such a guard is compiled out of the library in every standard build. Hence a checked key
rather than an asserted invariant.

Shape and clones-empty rationale follow `LazyDriverCache`, the existing precedent
directly above it. Deliberately NOT the loft#854 shape: those facts derive from
`Definition::code`, which `scopes.rs` rewrites, so a `Data`-lived cache would answer from
a body that no longer exists; these derive from def names, which never change once a
definition exists. `rebuild_indices` (which can REMOVE definitions) drops the cache.

Behaviour-preservation gate: `loft introspect` over all **672** `tests/scripts` programs
byte-identical before and after, IR and bytecode.

Closes loft#864 (per-invocation floor). The issue proposed a warm session or a `loft
serve` daemon; the measurement said the floor did not need one. Two facts settled it.
An INSTALLED loft was already at ~20 ms, not the 80–220 ms reported: the whole-program
cache is default-on but deliberately disabled for a dev build
(`cache::running_a_dev_build` — any `debug`/`release` path component), and the report
measured a from-source binary, which is the one configuration where it is off. And the
floor that remained was over a third pure waste, which is what this removes. A
persistent-session daemon stays available as future work; it is no longer the answer to
this issue.

Two unrelated reds cleared alongside:

- `cargo build --no-default-features` did not compile (already red on HEAD): `loft_home`
  sat behind the `registry` feature while `cache_areas` — the unconditional `loft cache`
  command — resolves the build cache through it. `dirs` is an unconditional dependency,
  so the gate was simply wrong.
- `AtomicU64::fetch_update` is deprecated; now `update`, not `try_update` — the
  saturating subtraction cannot fail, so there is no `None` case to report.

### The heap ledger is per-linkage-unit, and a store outlives the one that made it (loft#862) (2026-08-12)

`make ci` aborted at `exit_codes::moros_glb_cli_end_to_end` with `store_budget.rs:219:
attempt to add with overflow` — an *add* overflowing a `u64`, which can only happen if
`TOTAL` is already near `u64::MAX`, which can only happen if a `fetch_sub` already went
below zero and wrapped. So the reported site was the victim; the cause was a release.

The instrument said the rest in one run. Asserting `bytes <= TOTAL` inside `release`
rather than waiting for the next allocation gave:

```
PROBE release underflow: kt=125 bytes=4344 born_at=99218 TOTAL=0
  store_budget::release ← codegen_runtime::OpDatabase ← extensions::shared_store_dispatch
```

`TOTAL=0` with 4344 bytes being released: that `OpDatabase` runs INSIDE the library's
auto-native cdylib, which links its **own** copy of libloft and therefore its own
`store_budget` statics. The store was counted by the host's ledger and released against
the cdylib's, which starts empty.

`TOTAL` now saturates at zero, because below zero is not a quantity of heap — it is the
ledger being asked about bytes belonging to another one. What that costs is stated rather
than hidden: the HOST still counts those bytes as live, since its own ledger never sees
the release either, so the ceiling reads high for a program that frees inside a library.
That direction is the safe one (it can refuse early, never late) and it is what the
behaviour already was; making the two ledgers one needs a `loft_ffi` hand-off and is not
this change. Guarded by `releasing_more_than_was_added_stops_at_zero`, which asserts the
VALUE — a saturating floor and a wrap both survive a debug `fetch_sub`, and only the
number tells them apart.

Worth noting where it hid: `binary(exit_codes)` is outside `find_problems.sh`'s curated
selection, so a green `--wait` never covered it, and a release build wraps silently rather
than aborting. Both were true of `main` as well — verified on a clean worktree at
`00db858b`, not inferred.

### A refusal that returned `u32::MAX` became a syntax error (loft#863) (2026-08-12)

`fn sum(v: vector<integer>) -> integer { … }` collides with the stdlib's
`pub fn sum <T: Addable>` and is correctly refused — but it answered with a bare
`Cannot redefine 'sum'` and then `Syntax error: unexpected '->'` against a signature that
is perfectly well formed. Reporting the collision returned `u32::MAX`, `parse_function`
reads that as "this was not a function" and returns `false`, and the top-level loop then
resumed with the lexer parked between the parameter list and the `->`.

The sibling branch ten lines above — a free function shadowed by a METHOD on its first
argument's type, which `len` takes — never had the second message, because it reports and
falls THROUGH to the registration. This is the same fall-through: the rejected definition
registers under a `#dup` name no call can spell, so the rest of it parses, the real error
stands alone, and the winner keeps the real name. The position is printed too, which is
what says `sum` is the stdlib's rather than a duplicate of the reader's own.

### A tuple element read was a cursor typed as an owner (loft#857, loft#858) (2026-08-12)

Two issues, one line. `v[i]` on a `vector<(…)>` unboxes the element through a work-ref
that holds a DbRef into the vector's store, and `unbox_tuple_from_dbref` minted it with
`Deps::none()` — which in this codebase means OWNS, not "unknown".

Read as an owner, the cursor was freed at scope exit. Reading out of a `vector<(…)>`
**parameter** therefore destroyed the CALLER's vector store on return; the slot was
recycled, and the next call's `+=` appended through a handle that named another record
entirely (loft#857 — `vector_append: field 1.8 lies outside its own record, which claims
-99 words`). The filed scope was three axes too wide: the hash parameter, the outer loop
and the call count are all irrelevant, and one indexed read plus two calls reproduces it.
A DEAD read does too, so it is the read and not the dataflow.

Read as an owner, the cursor also had to hold its own store, so `__ref_1 = <foreign
DbRef>` lowered to `OpDatabase` + `OpCopyRecord`: every `v[i]` **allocated a store,
deep-copied the element into it and freed the previous one**, then read the elements back
out of the copy (loft#858). That is the ~14× against `vector<struct>`, whose cursor has
carried a dep on the vector all along and just copies a pointer. The reporter's verbatim
benchmark goes **379 ms → 12 ms**, against 11 ms for the struct — 14.6× to 1.09×.

So the cursor names the vector in its deps when the receiver is a variable, which is what
the struct path always did. Two measurements say the earlier readings of #858 were wrong,
and both are worth recording: the `??` join is **not** the cost (`v[i]?` measured the
same, 159 ms vs 155 ms), and it is **not** per-element unboxing either (arity 2 cost
139 ms against arity 3's 155 ms — the allocate/free dominates, so the gap barely moves
with arity). A `vector<float>` indexed read is only 2.2× its iteration, which is the
raising `OpGetVector` (a length read on top of `get_vector`) and is a separate, small
thing left alone.

The free still needs suppressing separately: the scope-exit sweep frees a `__ref_N` on
its NAME ahead of asking whether it owns anything — a rule written for the work-refs that
back ref-returning calls, which do own what they hold. And the borrow verdict is per-DbRef,
not per-call-site: the same helper unboxes heap-carrying tuple RETURNS, which ARE owned,
and a blanket skip leaked the four `pair(…)` return buffers in
`822-vector-tuple-spellings.loft`. A vector element read is the positive, checkable case;
everything unrecognised keeps the owning treatment. The mark is pass-2 only — the variable
table persists across passes by name while the `__ref_N` counter restarts, so a pass-1
mark could land on whatever temp pass 2 gives that name (loft#848).

`857-vector-tuple-element-read-borrow.loft` pins both, including the cells a borrow newly
has to survive: a read and an append in one statement, and 40 rounds of grow-then-read so
a cursor left pointing at a moved allocation shows up as wrong values rather than a crash.

### `τ` → `τ?` was refused with advice that could not work (loft#859) (2026-08-12)

Storing `x / g` back into a non-null `g` is refused correctly — a variable divisor can be
zero, so the quotient is `τ?` (C80). But the message offered `as` and a new variable name,
and on this shape **both fail**: the cast checker refuses `as τ` for the very reason the
store was refused, `as τ?` lands back on this same error — a closed loop between two
diagnostics — and a fresh name still has to store the value somewhere. The cures that
work, `?` and `?? <default>`, were named only by the cast checker.

`reject_retype` now picks the advice by which property changed. Only the ADVICE half
differs: the diagnosis, "cannot change type from τ to τ?", is right as it stands and is
kept word for word, so the two messages remain one diagnostic to anyone reading, grepping
or testing for it. A genuine type change (`integer` → `text`) keeps the `as` advice, which
is what it was written for.

`bench/06_newton_sqrt/bench.loft` was the reason @PLN140's oracle corpus had no row for
it — it did not run on either backend. It discharges at the division now and runs.

### The runtime rebuild retried against the state that motivated it (loft#855) (2026-08-12)

`--native`'s post-compile heal rebuilds loft's runtime rlib and retries the compile with
the SAME `Command` — whose `--extern loft=` / `-L dependency=` args were chosen from the
rlib that was on disk when the command was built. With no rlib at all they were never
added, so the retry re-ran a rustc that still named no crate: the heal reported its own
success and an identical `E0463: can't find crate for loft` in one breath, which reads as
a broken toolchain rather than a missed refresh. The runtime args are re-asked after a
successful rebuild.

The nightly ASan legs were red for a different reason and no leak: the per-file scan
reported **0 leaking files of 701**. Two tests spawn loft on both backends, and the
`--native` leg makes the spawned run BUILD native artifacts, which cannot resolve loft's
proc-macro deps under `-Zsanitizer` + `--target`. The ASan sweep already excluded them;
the leak gate did not, so it now does — a gate that reds on its own toolchain teaches
readers to ignore it.

### The ownership oracle is asked once per function, not once per question (loft#854) (2026-08-12)

`use_analysis::ownership_of` is documented as *"the ONE fact every own-vs-borrow
chokepoint READS instead of re-deriving"*. It re-derived it on every read:

```rust
pub fn ownership_of(data: &Data, d_nr: u32, value: &Value) -> Own {
    let def = data.def(d_nr);
    collect_defs(&def.code, &FillOps::of(own.data), &mut defs);  // the WHOLE function
    own.classify(value, &def.variables, &defs)                   // …to answer about ONE value
}
```

`scopes::scan_set` asks once per assignment, and a vector literal is one assignment per
element — so an n-element literal walked the function n times, **cloning every defining
right-hand side** on each walk. Quadratic: 2 000 elements 0.68 s, 4 000 2.34 s, 8 000
9.42 s, a clean 4× per doubling. crawler's generated terrain file (one 86 400-element
`vector<integer>`) took **over 13 minutes** at 99 % CPU with no output, which reads as a
hang; five of its `make` targets imported that module, so none of them were ever run.
It is now **0.99 s**.

**The fix is a memo, and where it lives is the whole design.** `function_defs` is split
out of `ownership_of` and memoised on `Scopes` — created per scan phase, holding the
`d_nr` being scanned. Correct *by construction* rather than by convention: `data` is
borrowed `&Data` for the entire traversal and `run_scan_phase` installs the rewritten body
only after `scan` returns, so the borrow checker is what keeps the memo from going stale.
It is the same value the old code recomputed — `data.def(d_nr).code` — computed once.

**Not cached on `Data`**, which is where it would naturally go. `Data` must stay `Sync`
(tests park it in a process-wide `OnceLock` and parallel workers read `&Data` across
threads), and the existing precedent there — `caller_index` — is a write-once `OnceLock`
that is never invalidated, which is sound only because the caller graph is stable after
parse. `Defs` is not: `scopes.rs` rewrites `definitions[d_nr].code` at four points, so a
`Data`-lived cache would answer from a body that no longer exists — silently, and in the
direction that mis-classifies ownership. The other 14 `ownership_of` call sites are
unchanged and keep recomputing; none of them is hot.

**Verified behaviour-preserving, not just green:** `loft introspect` over 60
`tests/scripts/*.loft` programs is **byte-identical** before and after, IR and bytecode.
Guarded by `tests/compile_scaling.rs`, a new home for complexity bounds, whose margin is
measured in both directions — 69.9 s against the reverted fix, 0.33 s with it.

Two things the filed report had wrong, worth recording because both would misdirect a
reader: it is **not parsing** (parse is linear: 12 → 17 → 34 ms while `scopes::check` went
705 → 2 271 → 9 323 ms), and the suggested **chunking workaround does not work** — eight
1 000-element literals in one function cost the same as one of 8 000, because the axis is
per-FUNCTION, not per-literal. Splitting across functions is what helped.

### A method lookup asks the type's OWN source only to replace a foreign candidate (loft#850 follow-up) (2026-08-11)

loft#850 taught `find_fn` that the mangled key `t_<len><Name>_<fn>` spells a type's NAME,
not a type: two packages may each declare a `Thing`, both register `t_5Thing_go`, and the
caller's name table holds whichever import landed first. The fix checks each candidate
against the receiver it declares and, when the candidate is foreign, re-asks in the type's
OWN source — where the right package's method lives.

The re-ask was written as a plain second search source:

```rust
for from in [source, own_source] {
    let d_nr = self.source_nr(from, &key);
    if self.method_receives(d_nr, type_nr) { return d_nr; }
}
```

**Every type has an own source, and for a builtin it is the stdlib.** So this did not just
resolve collisions — it added the whole stdlib method surface as a fallback for any call
whose first argument is a builtin type, ahead of the free-function lookup below it. A
library's free `split(pattern: text, input: text)` lost its own qualified call to the
stdlib's `split(self: text, separator: character)`: `regex::split("[,;]", "a,b;c")` stopped
compiling with *expected character, got text on argument 2*. The published `regex` package
had shipped that call since 0.2.0, and a language change retro-breaking a shipped library is
what the freeze forbids — `revalidate-libs` is the gate that caught it, green on `main` and
red on the branch.

The re-ask is now what it was meant to be: a REPLACEMENT for a candidate this scope answered
with and that proved foreign, never a second place to find a method the caller's scope does
not have. No candidate under the key means nothing to disambiguate, so the search falls
through to the free function exactly as it did before loft#850.

Pinned by `issue853_a_library_free_fn_outranks_a_stdlib_method_of_the_same_name`
(`tests/imports.rs`, both backends), whose control line asserts the stdlib's own free
functions and text methods still resolve from the same scope — a fix that reached the free
function by losing the methods would satisfy the subject and break the language.

### A binding position mints a local, whatever else carries that name (loft#852, loft#756) (2026-08-11)

A library's public function occupied the CONSUMER's variable namespace: with
`use engine_host;` in scope, `turn = 0` was a compile error anywhere in the consuming
program. So one public verb added to a library broke every consumer already using that
word as a local — crawler's gate went red across 109 rows, on a commit crawler did not
make. [C97](DESIGN_DECISIONS.md)/[C98](DESIGN_DECISIONS.md) make this impossible for
the *stdlib*; this is the same hazard one level up, where nothing weighs each new name
and nothing announces which words a release claims.

**The refusal was one code path, not a rule.** It fired in three binding forms —
`name = …`, the typed local `name: T = …`, and a tuple-destructuring element — all
three routing through the bare-function-reference fallback in `parse_var`
(`src/parser/objects.rs`). The other three never refused, for the stdlib either: a
parameter, a `for` variable and a struct field could all be called `chr` while
`chr(65)` in the same scope answered `"A"`. So loft already keeps values and functions
in **separate namespaces**, and the parentheses already pick between them.

**The fix** is that the function-ref fallback yields at a binding position, exactly as
the type/enum branch beside it already did (`def_nr(name) != MAX && !at_binding_name()`);
the name then takes the ordinary new-local path. The pass-2 rescue that re-resolves an
untyped pass-1 placeholder to a forward-declared function takes the same guard — without
it a binding whose type pass 1 could not infer (`turn = a_forward_fn();`) would hand
`parse_assign` a function-ref where its target belongs.

What the refusal was really reporting was a **recovery** problem, not a namespace one:
@P335/@P392 found that the function-ref left the `:` or `=` unconsumed and the author
saw a confusing `Expect token ;`. Binding supplies the `Value::Var` that recovery
wanted, so those forms parse rather than diagnose. `tests/scripts/repro_p392.loft`
changes from an `@EXPECT_ERROR` case to asserting the typed local binds and `now()`
still answers — the mis-parse it exists for fails it either way.

**Pinned by** `tests/scripts/852-local-shadows-a-function-name.loft`, which carries the
parameter / loop / field cells as CONTROLS so a change that frees the three refusing
forms by breaking the three that already worked fails there, and
`pln102_c98_a_local_may_shadow_a_library_function` (`tests/imports.rs`) for the library
half on both backends. Register entry: [C112](DESIGN_DECISIONS.md).

**Residual, unchanged by this:** a bare `use lib;` still wildcard-imports every public
name into the unqualified namespace (`src/parser/mod.rs`,
`None => Some(ImportSpec::Wildcard)`), so `turn(3)` is callable unqualified after
`use mylib;`. C98 rules it must bind only the `lib` handle. That is a breaking
resolution change needing a pre-freeze migration (`use lib;` → `use lib::*;`), so it is
owner-timed rather than folded in here — and C112 is forward-compatible with it.

### `--html` binds a filesystem, over raw `loft_io` imports (loft#851) (2026-08-11)

`--html` bound no filesystem. The loft-side file calls compiled anyway — the
wasm-bindgen feature that routes them to a JS host is off for this target, so each
one took its inert branch and answered "absent" — and the build reported success.
A page could draw and could not save, and the consumer discovered it by grepping
the emitted bundle.

**The transport.** `--html` cannot reuse the `loftHost.fs_*` bridges: those are
`js_sys::Reflect` lookups needing wasm-bindgen, and this target builds its rlib
`--no-default-features --features random` and refuses a page whose wasm imports
anything beyond `loft_gl` and `loft_io`. So the file functions are raw `loft_io`
imports declared in `src/lib.rs`, and reads use the `len`-then-`copy` shape
`loft_host_input_len`/`copy` already proves, sharing one host stash — safe because
each is a synchronous loft call, so the next read cannot begin before this one has
copied. `usize::MAX` is absent, which is not a length of 0.

**One chokepoint.** `src/wasm.rs::host_fs_*` is the only place that picks a
transport (`js_sys` under the `wasm` feature, raw imports otherwise). Every call
site asks one question instead — the new `host_fs` cfg from `build.rs`, set for the
wasm-bindgen bundle and for `--html`, and deliberately NOT for `wasm32-wasip2`,
which has a real WASI filesystem. That replaced 40-odd hand-written
`feature = "wasm"` gates across `state/io.rs`, `database/io.rs` and
`codegen_runtime.rs`, and the two `--html`-only inert stubs they had grown beside
them. `codegen_runtime.rs` is the one that mattered: `--html` runs generated code,
and its browser arms were stubs returning nothing while the interpreter's were real
bridges — so the two backends had different filesystems and only one of them was
documented.

**The cursor.** A page has no OS file handle, so the host keeps the read/write
position per path. This is where the first working version was still wrong, and the
matrix is what caught it: every whole-file cell passed while `read_bytes` answered
zero bytes for a file that had just been written. `fs_read_bytes` / `fs_write_bytes`
in `database/io.rs` are WHOLE-file operations and were calling the cursor-relative
bridges — so a preceding write left the cursor at the end and the read started
there. They call `read_binary` / `write_binary` now, which also fixes the same wrong
result in the wasm-bindgen build, where it had been latent.

**The host half** is `doc/loft-fs.js`: an immutable base tree the page supplies
(`globalThis.loftBaseFS`) plus a delta holding every write, persisted to
`localStorage` — the `LayeredFS` shape, which is what a page needs. Both page
shells bind it, so a program that only stores still gets the minimal engine-less
page. Every node harness that instantiates an `--html` wasm imports the same module
rather than restubbing it, because a stub answering 0 means "an empty file that
exists" where the contract says absent.

**Guards.** `tests/html_wasm.rs` drives a real page through the whole surface and
through the cursor, with every expected value taken from what `--interpret` and
`--native` print for the same program; `tools/loft_fs_unit.mjs` (run from the same
test binary) covers the base tree and the reload, which a node-hosted page cannot
reach. The build-time warning added earlier for this issue is gone — its premise
was that the target binds no filesystem.

Also here: the wasm32 rlib would not compile at all. An entry inserted into
`src/native.rs`'s natives table landed between a `#[cfg(not(target_arch =
"wasm32"))]` and the item it guarded, so `n_kernel_listen` lost its gate; `--html`
reported "install the target" — blaming the environment for a source error — and
linked the previous rlib.

### `store_release` — a working-set hint, and the per-record shape it replaces is measured dead (@PLN126) (2026-08-11)

@PLN126 opened on a measurement rather than an API: *does ordered insertion leave a
finished record contiguous?* `src/database/spans.rs` answers it by painting every word
of a built arena with the record that owns it, through `for_each_owned_child` — the same
ownership walk `remove_claims` frees by, so the measurement cannot disagree with the
runtime about who owns what. On `routing`'s generator shape (`hash<TTile[tkey]>`, two
grown-by-append vectors):

| | span/live mean | exclusive 4 KB pages | droppable below the finish frontier |
|---|---|---|---|
| strict key order (W=1) | **356×** | **0.0%** | **98.7%** |
| 16 records open (W=16) | 391× | 0.0% | 93.0% |
| 64 open | 540× | 0.1% | 86.9% |

**Contiguity is false by two to three orders of magnitude, and the cause is not vector
reallocation.** The outer `hash` keeps entries in a chunked arena claimed early, while a
record's vectors are claimed at the frontier much later — so a record's own bytes sit
either side of the whole store. With ONE record in the store and nothing else alive, its
5-word slot and its 28 words of vectors are separated by 318 words of the collection's
own spine.

That kills the per-record release outright (0.0% exclusive pages is the granularity
problem in a number) and, contrary to the plan's reasoning, does **not** kill the
frontier release: that one needs the region below the mark not to be written again,
which is a property of the allocator on an append-only workload, and it holds. So the
plan was re-scoped onto the claim that measured true, and built.

`store_release(collection) -> integer` (`Store::release_resident` → `Stores::release_store`)
does `msync(MS_ASYNC)` + `madvise(MADV_DONTNEED)` over the whole pages below the mark
that have not been released. Peak RSS 44.3 MB → **2.2 MB** on an 89 MB build, at 1.0×
wall, one call per record. Content, references and file length are all untouched: the
mapping is `MAP_SHARED`, so a released record re-reads from the file one fault later.

Three costs the design could not have predicted, each found by an instrument:

* **Deriving the frontier cost the whole point.** `Store::usage` walks the block chain,
  one header per block, touching every page — so asking it what to drop faulted the
  entire store back in first, and peak RSS became the whole file (80.9 MB against 44.3
  for making no call at all). `Store::claimed_end` now carries the mark forward at
  `claim_block`. It is a monotone UPPER bound on `live_end_words` — the safe direction
  here, the wrong one for `shrink_to` / `reclaim_tail` / `bind_path`, which still read
  the chain because truncating to an upper bound cuts live data.
* **Each call must be bounded to what is new** (`Store::released_bytes`). Flushing from
  zero every time re-syncs a region that grows with the run: 208× wall.
* **`MS_ASYNC`, never `MS_SYNC`** — same resident set, and waiting costs ~1.5 ms a call.

**It pays only for an ordered build** (1.01× at W=16), and the attribution is the free
block count rather than the layout: 10 free blocks at W=1 against 3 691 at W=16 for the
same data. The LLRB free-space tree's nodes live inside the freed blocks, so a scattered
arena gives the allocator its own scattered working set — exactly the region the release
just dropped.

Gates: `tests/scripts/126-store-release-keeps-everything.loft` +
`store_release_keeps_every_record_and_reference_both_backends` (a reference held across a
release is checked by VALUE, because a re-faulted page returns a plausible number either
way), `database::spans::one_tile_footprint_is_the_blocks_it_owns`, and the two `#[ignore]`
measurements. Full workings: `doc/claude/plans/126-record-frontier.md`.

### A hash's entries move into a chunked arena, and the lookup win it was built for does not exist (@PLN135 arc H, #809) (2026-08-10)

`hash<T[k]>` stops claiming one store record per entry. Entries are slots at a fixed
stride in a chunked arena (`src/arena.rs`) whose bookkeeping lives in the bucket table,
and a bucket slot holds a 1-based arena INDEX rather than a record number.
`placement::HASH` bumps 1 → 2, so a store written before this REFUSES instead of being
misread — the reason Q2 shipped ahead of H.

Measured against the installed `v2026.8.0` as before-oracle, alternating A/B on a quiet
box, 1M `integer` keys, `--native-release`:

| | before | after | |
|---|---|---|---|
| insert (reserved) | 330 ms | **258 ms** | **1.28x** |
| store bytes / entry | 27.67 B | **18.6 B** | **−33%** |
| claimed records, 2000 entries | ~2000 | **9** | table + directory + 6 chunks |
| random lookup | 184 ns | 183 ns | **unchanged** |

**@PLN135 predicted 2.3x on lookup and that is wrong, from correct measurements.** Q1's
ablation (the record read is 82% of a random lookup) and Q5's shapes (a dense
`vector<Entry>` at 80 ns against the hash's 200) are both sound; the inference is not.
80 ns is ONE random read and a hash lookup makes TWO — bucket, then entry — so packing
the entries moves where the second miss lands without removing it. Density pays for
locality and a lookup has none. What the arena removes is the per-entry `Store::claim`,
which is what #809's title names, and that lands on insert and on bytes.

Three things the build turned up that the design did not:

* **A hash has TWO kinds of entry.** A secondary index (a sibling field's
  `other_indexes`) reaches records the PRIMARY collection owns and must neither move nor
  free them. The discriminator is the recorded stride — a table that allocated its
  entries knows their width, one that borrows records has none — so `stride == 0` means
  borrowed, and every decode, free and teardown reads it.
* **Chunk sizes must stop doubling** (`arena::CAP_CHUNK`). Uncapped, the tail waste is
  proportional and measured 27.33 B/entry — the whole saving — and it made a store's size
  depend on construction order. Capping bounds it at one partly-filled chunk: the
  difference between −1% and −33%.
* **Creation and freeing are one change.** `record_new`'s keyed arm allocates from the
  arena; `for_each_owned_child` stops returning each entry as an `owning_elem` for
  `Store::delete` and returns the chunks and directory through a new
  `OwnedWalk::extra_recs`. Half of that alone hands interior arena bytes to the free tree.

**A latent store-lifetime bug came with it, and it is the sharper half.**
`free_iteration_scratch` decided what to release by reading its scratch header's
fields, guarded only by `Store::is_claimed_record` — which says the BLOCK is live, not
that it is still ours. A released scratch leaves its block on the free list and the
next claim takes it; every field read after that is somebody else's bytes. One of
those reads decides *"the elements live in another store, so free the whole store"*,
and it fired: the arena's first chunk is exactly the claim that lands on a released
2-word header, so a hash captured by a closure lost the entire store it lived in
between one invocation and the next — every entry gone, no error anywhere. It surfaced
as `multiplayer_v2::v2_two_clients_with_spectator_routing` hanging: the tictactoe
server keeps its client table in a struct its `server::run` closure captures, so every
lookup after the first iteration missed and `handle_click` returned at its first guard,
leaving both clients to wait out their budget. The scratch header now carries a marker
(`vector::scratch_tag`, and the same word holds the element width), and the free path
refuses a record that does not present it — refusing costs at most the scratch's own
two blocks once, acting on a foreign record cost the store. Guard:
`data_structures::a_released_iteration_scratch_is_not_acted_on_twice`, which re-claims
the released header's block deliberately and asserts on `Store::is_free` rather than on
a value read back, because a freed store keeps its buffer until the slot is reused.

Four `store_persist_loft` tests changed with it, none by loosening a bound. Two fixtures
(`store_compact_slack_730`, `store_load_density_729`) were building their vectors in a
LOCAL and copying the finished thing into the entry, which acquires none of the in-place
growth slack they are named for — the entry's copy is claimed at its exact length. What
they actually acquired was the blocks the local abandoned on the way up, i.e. free space
BETWEEN records, and the arena made the allocator pack those better (same digest, file
1 729 032 → 1 091 112 B). Both now append through `h[i].data`, which grows the record the
collection HOLDS. `persisted_size_tracks_content_not_construction` now compares the two
construction orders AFTER a rebind, which is the comparison loft#710 is about — as built
they differ by interior free space that only compaction reclaims, as that test's own
header says. `reclaim_and_compaction_refuse_a_sealed_store…` grows its control 10x
instead of 2x, because 3000 extra entries are now ~48 KB of slots rather than 3000 claims
and no longer push the store buffer past its bound size.

### Three answers that were derived from a proxy instead of the fact (#829, #830, #831) (2026-08-09)

Three consumer-filed defects with one shape: a decision read a stand-in for the
fact it needed, and the stand-in was true when the fact was not.

- **#829 — `content()` answered `""` for bytes it could not decode.** `""` is
  what an empty file says, so a caller could not tell the two apart, and a
  round-trip gate over binary data passed vacuously (`0 == 0 * 2`). `content()`
  is `text?` and already answered null for a missing file (@PLN102 H4); it now
  does the same for non-UTF-8 bytes and for a directory, with `""` reserved for a
  file that really is empty. The decision sits in the loft-level `content()` —
  one home, so both backends get it from the same place. The read beneath it also
  had two homes (`State::get_file_text` and `codegen_runtime::OpGetFileText`) and
  the stderr warning lived in only one of them, so `--native` read binary in
  silence; both now call `read_file_text_into`. Guards:
  `tests/binary_io_matrix.rs::c829_*` (four `cross_mode!` cells, including the
  empty-file cell that keeps null and `""` apart) and a both-backend
  `p166_content_on_binary_file_warns`.

- **#830 — `loft update` resolved the lockfile, not the project.** A dependency
  declared in `loft.toml` and absent from `loft.lock` was never looked up, and
  the summary counted lock entries, so the omission printed `all N packages
  up-to-date`. The work list is now `lockfile::update_worklist(lock, declared)` —
  the union, as a pure function with unit tests, so "which packages" has one
  testable home. A declared-but-unresolvable package is named and turns
  `--check` red (that check asks whether the lock describes the manifest);
  `loft update <pkg>` on a non-dependency refuses instead of claiming it is
  up-to-date; a project with declared deps and no lockfile gets one written.

- **#831 — a cdylib that built was assumed to be one this process can use.**
  Marking a function for cdylib dispatch makes `byte_code` emit `OpStaticCall`,
  so an unwirable symbol reaches the `compile.rs` panic stub and kills the run.
  Marking was gated on the BUILD succeeding; an artifact can build and still not
  load (different `libloft.rlib`, missing system library, replaced by a
  concurrent `loft`), and an artifact declaring no layout is adopted outright
  because that is what a hand-written cdylib looks like.
  `native_lib::probe_and_mark_exports` now `dlopen`s the artifact and `dlsym`s
  each bridge before marking, marks only what resolves — partial is a valid
  outcome — and KEEPS the handle, so a later prune or rebuild cannot invalidate
  the decision. Unresolved functions interpret, which is what the auto-native
  model always promised. This is why crawler's suite lost a different test on
  each parallel run: processes share `<pkg>/native-auto/`, and the loser got the
  panic stub instead of the interpreter. Guards:
  `tests/n3_use_native.rs::an_unwirable_cdylib_interprets_instead_of_panicking`
  and `::a_partially_exporting_cdylib_marks_only_what_resolves`, both driven by a
  real artifact replaced with a cdylib that loads and exports no bridge — the
  shape every freshness check accepts. `--help` now names `LOFT_NO_NATIVE_LIBS`
  and `LOFT_REQUIRE_NATIVE`, which the report searched for and could not find.

  **Residual half, found by the same suite:** `prune_artifacts` bounded
  `native-auto/` by sweeping every `.so` in it by age, and the directory is not
  exclusively its own — a `[c] shim` cdylib lives there too, content-keyed and
  built ONCE, hence permanently the oldest file and the sweep's first victim.
  That does not cost a rebuild, it deletes the only definition of the package's
  `#c` symbols; the run then dies at `c_call.rs` with *"symbol not found … or
  check the spelling"*, and nothing can interpret in its place because a `#c`
  binding IS the implementation. Reproduced deterministically against
  `tests/fixtures/sqldb/sqlite` (saturate, run once, shim gone, exit 101) — it
  had been living in the suite as the "known flaky"
  `native::a_lazy_read_gives_one_answer_down_rust_and_down_loft`. The sweep now
  takes only the `loft_auto_<pkg>_` family it built; guard
  `::a_foreign_library_in_native_auto_survives_pruning`.
### The block-tail `expected` push learns a third shape: the interpolation target (#837) (2026-08-10)

@PLN124's target is read off the one `⇐` channel, and `parse_block` pushed the block's
result type into that channel only when the result was an **enum** (@PLN22 phase 1) or a
**collection** (@PLN90 W8). A struct is neither, so `fn q(name: text) -> Query { "hi
{name}" }` parsed its tail with `expected = Unknown`, took the ordinary text path, and
failed the tail conversion — *"expected Query, got text on return from block"*. The gate
now also fires when `interpolation_target(result)` resolves, which is a pure lookup
(`Type::Reference` → `DefType::Struct` → defines `t_<len><name>_lit`), so the cost is one
def-table probe on block tails whose result is a struct.

One gate covers all three reported spellings — block tail, explicit `return`, and an `if`
tail threading into both branches — because they share `parse_block`'s tail.

The issue asked which of doc and code was wrong, on the reading that the **call-argument**
position had been closed deliberately by #776. It has not: the argument position builds
correctly on both backends on current `main` (verified `parts == ["hi "]`, `values ==
["ada"]`, not merely that it type-checks), so the doc's list was accurate except for the
return. #776's narrowing was of the HOLE channel, not the argument channel, and that gate
still holds — `q: Query = "{"seed"}"` passes `"seed"` to `hole_text` as a value rather
than building a second accumulator.

Guards: `tests/scripts/interpolation-hook.loft` grows `built_as_tail` / `built_as_return`
/ `built_in_branch` beside the existing `seq_of` argument-position case, asserting the
call SEQUENCE (`lit(t)>int>lit(u)`) rather than the result — a target that only checked
the final string could not tell the hook from ordinary formatting. Both backends.

### A tuple match arm that consumes nothing is a parse that never ends (#832) (2026-08-10)

`parse_tuple_match`'s arm loop could iterate without consuming a token, and then it
never stopped. `(first, ..)` reached the element loop's literal branch, where
`expression` took the `..` and left the `)` unclaimed; `expect_match_arm_arrow` then
found no `=>` and called `recover_to(&[",", "}", ";"])`, which **resynchronises** and
returns WITHOUT consuming when the cursor already sits on a stop token or an unmatched
closer. The arm re-parsed the same token forever — 2.1 million iterations in four
seconds — and silently, because first-pass diagnostics are suppressed. loft is
unbounded by default, so nothing bounded it.

The filed scope was `..`; the matrix widened it. **An over-arity pattern hangs
identically** (`(a, b, c, d)` on a three-element tuple): the element loop stops at the
subject's arity and leaves the cursor on the surplus `,`. Junk arm heads (`1`, `"x"`,
`[1,2,3]`, `{ }`) recover fine, because `recover_to` scans forward from them and does
consume — which is what made the boundary look narrower than it was.

Three changes, one invariant — *every arm-loop iteration consumes at least one token*:

- `..` / `..=` is refused **by name** in an element position, with the supported form in
  the message, then skipped to the closing `)`. Arity is fixed by design (TUPLES.md
  § "What is NOT supported"), so a rest has nothing to stand for.
- A pattern longer than the subject reports the subject's **arity** rather than a bare
  "expected ')'", and the surplus is skipped so the arm reaches its `=>`.
- A `bad_pattern` flag keeps a refused arm from being classified as a **wildcard**. A
  refusal binds nothing and tests nothing, which reads exactly like `(_, _, _)` — and a
  wildcard arm ends the arm loop, so a rejected FIRST arm swallowed every arm after it
  and reported a missing `}` instead of the refusal. This is why `(.., last)` and `(..)`
  behaved differently from `(first, ..)`, which binds and so escaped the misclassification.
- The element loop's missing-comma `break` is no longer gated on `!first_pass`. Both
  passes must walk an arm the same way, or the first wanders into positions the second
  never visits.
- A backstop compares `lexer.at()` across the whole iteration and breaks if nothing moved,
  so an unknown shape ends in a diagnostic rather than a stuck build.

Two adjacent defects found by the first tuple-element-pattern coverage the corpus has
ever had (`28-tuples.loft` carries no `match`, which is why the hang shipped), both
filed rather than fixed here:

- **#839** — an `if` guard never parses on a vector or tuple arm: those two loops call
  `has_keyword("if")`, which matches only `LexItem::Identifier`, while `if` lexes as a
  token; the three working match kinds use `has_token`. Swapping it in was tried and
  reverted: the guard then parses and the arm silently does not match, because captures
  are assigned in the arm BODY and the condition runs first, so the guard reads an
  unassigned variable. A clean refusal beats a silent wrong answer; both call sites
  record why.
- **#840** — a tuple **parameter** with a `text` element fails rustc on `--native` when
  it is the match subject: the `match_tuple` temp is spelled with the owned type
  (`String`) and initialised from the borrowed parameter (`&str`).

Guards: `tests/scripts/832-tuple-pattern-refused.loft` (every `..` position plus
over-arity, asserting the REJECTION — a timeout-only test would pass for the wrong
reason) and `832-tuple-pattern-elements.loft` (the positive twin; a fix that rejected
every tuple pattern would satisfy the first alone). Both backends.

### One SQL boundary closes: a table loft made and a table loft found are the same value (@PLN133) (2026-08-08)

@PLN133's gate passes, on **four database backends and both loft backends, with
byte-identical output in all eight cells**. Write a struct graph through the derived
`INSERT`, bind a collection lazily to the SAME connection string, traverse it, and
get back the values, the identity across two paths, and the trip counts laziness
predicts. **Run twice** — once into an empty database where loft writes the schema,
once into a table made by hand with a different column order, the float kept in a
`VARCHAR`, and an extra column loft knows nothing about. Only the second run proves
requirement 3; the first passes even against a `reconcile` that always agrees.

- **S11 + S12 are ONE call.** `ensure(d, dial, want)` is the whole absent-or-present
  decision, because it is one decision — splitting it puts the test in every caller,
  which is where two callers eventually disagree about what absence means. Absence is
  decided by ASKING THE CATALOGUE, never by an `IF NOT EXISTS`: the rule *loft never
  touches a table it did not find missing* belongs in loft's code where it can be read,
  not in an engine's tolerance for a repeated `CREATE`. After creating, it reads the
  table BACK and reconciles against what the engine actually stored — mariadb turns
  `BOOLEAN` into `tinyint`, so reconciling against the derivation would assert the
  round trip instead of testing it.
- **`introspect` now reads all four catalogues**, which is what S12 needed and what S6
  had deferred with a scope statement. The columns half unifies on
  `information_schema.columns` (scoped by an expression the `Dialect` carries); the
  INDEX half does not and is not pretended to — `information_schema` has no index view,
  so PostgreSQL answers from `pg_index`, mariadb from `information_schema.statistics`,
  and duckdb hands back the `CREATE INDEX` TEXT rather than a row per column. Every
  query was RUN against a live server of its engine before it was written down.
  Two things a guess would have got wrong: PostgreSQL's `indkey`/`indoption` are
  **0-based** `int2vector`s, so `indoption[ord-1]` reads every direction as NULL; and
  the type mapping is a WHITELIST rather than sqlite's substring test, because sqlite
  has affinity — a rule the engine itself applies — while PostgreSQL's `point`
  merely contains `INT`.
- **S13's statement is derived, its values are not.** `insert_row` renders the writer's
  `INSERT` from the same `TableDef` the reader's `SELECT` comes from, so they cannot
  drift. The generic walk from an arbitrary struct's fields to those values is NOT
  built and cannot be here: loft's reflection reports types, not values.
- **S10's deletion is REFUSED, and that is the finding.** Deleting core's Rust sqlite
  path makes a driver mandatory for `sqlite:`, a driver names a concrete element type
  so it cannot be generic, and `store_bind_lazy(c, "sqlite:x.db")` needing no user code
  is a shipped promise. The alternative — core synthesising a driver that calls the loft
  library — makes a fixture a dependency of core. So S9's precedence rule IS the answer:
  a demotion, not a deletion. What it buys is not deletion but a stopped clock, which
  was the plan's actual complaint: N=4 backends now and **+1 forever**. The +1 is gone.

Found on the way and filed rather than absorbed: **[loft#813](https://github.com/loft-lang/loft/issues/813)** —
a value whose static type is a struct-enum VARIANT (`x = AsA { … }` rather than
`x: Any = AsA { … }`) is accepted where a bounded generic wants the ENUM and then
answers the type's empty value. Silent on `--interpret`, a `todo!()` panic on
`--native`, a SIGSEGV with two generic hops.

### A buffer that is already a reference is handed over, not wrapped (loft#806) (2026-08-08)

`return t.m(i) ?? "x"` SIGSEGV'd the interpreter while `--native` answered correctly.

`OpCreateStack(v)` is how a variable that OWNS its text hands out a reference to it.
The call site that fills a callee's hidden `&text` return buffer picks that buffer BY
NAME (`__work_cN`, so the two passes agree on it), which means it never looked at the
variable's TYPE — and the name can already belong to a `&text` PARAMETER of the calling
function, because `text_return` promotes a text local the return value depends on into a
caller-allocated buffer (loft#662). Wrapping it again built a DbRef pointing at the
reference SLOT; the callee's single deref then read that slot as text. `--native` passes
the reference by the Rust ABI and is immune, which is why the two backends disagreed
instead of both crashing.

This is the rule #266 already states for non-text references at the argument-coercion
site. That site compares TYPES, so a variable already holding the wanted reference never
reaches its conversion at all — which is why only this one, keyed on a name, could reach
the double wrap.

**The filed boundary was a tenth of the defect.** A 20-cell matrix over the composition
axes put 8 cells on the crash, and two of the three conditions the report listed as
required are not:

| axis | filed | measured |
|---|---|---|
| enclosing function returns `text` | required | `text?` crashes too |
| fallback is a non-empty literal | implied | `?? ""` crashes too — the buffer-append is skipped, so the wrap alone is the fault |
| receiver is a plain variable | implied | a field read (`h.inner.m(i)`) crashes too |

The passing cells are not incidental: a free function, an intermediate local, an extra
text parameter and interpolation all avoid it, and each does so by changing which
variable the promotion picks. An attribution pass over the IR — `OpCreateStack` applied
to a var the same function declares `&text`, keyed on the (name, scope) PAIR because
`n_main`'s plain local and a callee's promoted parameter routinely share a `__work_cN`
name — flags exactly the 8 crashing cells and none of the 12 passing ones, before and
after.

The fix hands over the BARE variable. Both backends already forward a `RefVar` argument
into a `RefVar` parameter with no deref (`codegen.rs` `OpVarRef`; `generation/calls.rs`
`var_x`), and both recognise that shape only as a literal `Value::Var` — wrapped in a
block or an `Insert` the generic path runs instead and re-derefs, which is a second wrong
read one layer down rather than a fix. The per-call clear is not lost: a promoted buffer
is cleared by the function preamble once per invocation, and promotion only happens when
the RETURN VALUE depends on the buffer, which puts its call in tail position. Restricted
to the no-default case, so a `&text` parameter carrying a `= "…"` default keeps its
existing lowering rather than trading this crash for a dropped default.

`tests/scripts/issue-806-retbuf-double-reference.loft` carries the axes as value
assertions — a wrong read here is as likely to answer `""` as to fault, so "did not
crash" is not the bar — plus the forwarded-buffer control that says the fix did not
simply delete the wrap everywhere.

### A method's return was adopted by one half of the compiler and freed by the other (loft#810) (2026-08-08)

Filed as a SIGSEGV needing a library, a `vector` local, a foreign package's record type
and a loop-body local — six ingredients, drop any one and it ran. None of them is the
defect. It is **one word in a predicate**, it needs no library at all, and its ordinary
outcome is a WRONG VALUE rather than a crash.

Binding a call's result to a heap local asks one question: may the caller ADOPT the
returned store, or must it COPY into a store the binding owns? Cluster A already collapsed
the ANSWER into one carried fact (`Def::return_adopts_fresh_store`). What had drifted was
the GATE on it — which callees the question is even asked about:

| site | decides | accepted |
|---|---|---|
| `scopes.rs` (`scan_set`) | strip the binding's deps → emit its scope-exit `OpFreeRef` | `n_` **and** `t_` |
| `state/codegen.rs` (first-Set) | deep-copy, or adopt | `n_` only |
| `state/codegen.rs` (reassign) | deep-copy, or adopt | `n_` only |

So a `t_` METHOD returning through the caller's hidden `__ref_N` buffer — the shape every
`q: Acc = Acc { }; …; return q` compiles to, dep `["q"]` — fell past both copy arms to the
plain-adopt fallthrough, and was then freed at scope exit as if the binding owned it. The
buffer's store went back to the pool while the caller still named it. Next iteration the
callee's own work-ref drew that slot, the retbuf `OpDatabase` re-`claim`ed it at the
original name, and one record had two owners.

`Def::is_loft_defined()` is now the single home for that gate, next to the two facts it
guards, and the six spelled-out copies of it (scopes ×3, codegen ×2, parser ×1) read it.

**What the boundary actually is**, measured one axis at a time
(`tests/scripts/810-method-return-buffer.loft`):

- No library, no foreign package, no vector: a single file reproduces it.
- The callee needs ONE competing allocation between the free and the next iteration's
  re-adoption — otherwise the freed slot is handed straight back and nothing shows.
- Whether it crashes at all depends on what lands in the recycled slot. The reported case
  hit a record header and died in `memcpy`; the plain shape silently loses a vector and
  answers a plausible count. **The value cells are the test**, not "did not crash".
- Three cells, not one: first-Set in a loop body, reassignment of an outer local, and
  assignment inside a nested block — the last two go through the OTHER codegen site, so
  fixing only the first leaves two thirds of the defect standing.

`--native` was never affected and needs no change, for a reason worth writing down rather
than trusting: its Reference-typed `__ref_N` is a `DbRef::NULL` sentinel passed BY VALUE,
so the callee always allocates fresh and the caller's copy stays null. The alias the
interpreter formed cannot form there. Its vector retbuf (`__vdb_N`) IS caller-allocated,
and that path was already right — pinned as a control.

The store guards that came out of the first pass at this stay, and earn their keep: a
non-positive size word makes every whole-payload walk compute `size * 8 - 4`, which wraps
to ~18 exabytes and dies inside `memcpy`, naming the copy — which is innocent. `assert!`
and not `debug_assert!` on purpose: a debug build already catches the underflow, and it is
the RELEASE build, where the wrap is silent, that needed one. `Store::copy` /
`Store::zero_fill` route through one `payload_bytes` helper; `vector_append` asks first
whether the field is inside its own record at all, and now says what that means — a slot
with two owners, with `LOFT_NO_SLOT_REUSE=1` as the one-run test — rather than the layout
fault it first read as. `Store::valid` bounds a field with `fld <= size * 8`, admitting a
read that starts exactly AT the record's end, and is a `debug_assert!` compiled out of the
profile the loft library builds under; that is why nothing caught this earlier.

`LOFT_TRACE_DB` now prints from the native runtime too. It had existed only in the
bytecode VM, so it went silent exactly where a call crossed into a package's shared
library — which is where the slot adoption it exists to show was happening. Both backends
read one cached key.

### A persisted trie is laid out so it can be paged (@PLN134) (2026-08-08)

`trie<T[k]>` ships whole-image only: a prefix query over `routing`'s 220 032-word
vocabulary is 5.9 MB gzipped, downloaded once. @PLN134 asked whether a paged reader could
answer it in a few range reads instead, and opened on the measurement that decides it —
**pages touched, not nodes**.

The first answer killed the cheap design. A PATRICIA descent reads ~330 bytes of nodes and
spreads them over **27 pages of 64 KB**, because node ids are handed out in INSERTION
order and a root→leaf path visits nodes created at wildly different times. Renumbering
breadth-first halves it and stops there. 1.7 MB to answer a keystroke is worse than the
download by the fourth one.

The second answer is the plan's own declined branch, reached on evidence: it is not "page
a trie", it is **lay a trie out so it can be paged**. Same tree, same walk, same touch
sets, five numberings:

| node order | pages @ 64 KB | @ 4 KB |
|---|---|---|
| as built | 27.1 | 36.4 |
| breadth-first | 15.4 | 26.0 |
| key order | 8.7 | 14.5 |
| depth-first pre-order | 4.2 | 7.2 |
| **van Emde Boas** | **2.8** | **3.8** |

The 4 KB column identifies the mechanism rather than the number: vEB barely moves where
every other order inflates by half. That is what cache-*oblivious* means, and it matters
here because the page size is not ours to pick — a local file, an HTTP range read and a
browser cache disagree about it.

The records matter more than the nodes, and step 1 had not measured them at all: the 20
records a query RETURNS sit on ~20 distinct pages when claimed in insertion order, and on
**1** when written in trie key order. Together a cold query is ~3.8 pages / 250 KB against
the 5.9 MB image, and the second keystroke of a session costs ONE page.

- **`radix_tree::rtree_relayout`** renumbers a tree van Emde Boas and compacts the free
  list. Node ids are internal, so nothing observable moves — `r11` holds it to the same
  walk and the same record for every key, which is the gate that matters: rewriting the
  array in place produces a structurally valid PATRICIA tree holding the wrong records,
  and `rtree_validate` alone passes that. Idempotent, and it refuses a tree whose walk
  does not account for `n-1` nodes over `n` records.
- **`store_persist_bind` runs it before writing the image** (`Stores::relayout_tries`),
  because that image is what a reader pages. Stores whose SCHEMA cannot hold a trie skip
  the data walk entirely, so no other kind pays for it.
- The measurement lives on as `trie_db::pages` (`#[ignore]`, three tests: the layouts, the
  record placement, the warm session) and `r10` asserts every candidate order is a
  permutation of the live nodes — not decoration, since a duplicate-emitting order reports
  a BETTER page count.

Paging a trie is still unwired: `store_bind_lazy` refuses one, and `store_load_key_text`
reads a `hash`. The layout is the prerequisite that made those worth building.

### sqlite down the loft path, measured against the Rust one (@PLN133 S9, 2026-08-08)

Core drives sqlite in Rust (913 lines across `sql_source.rs` and `sql_query.rs`)
and the loft library drives four backends behind one `SqlDb` interface. The step
is *"switch sqlite to the loft path"*, and taken literally it cannot preserve what
it must: every @PLN129 test binds `sqlite:` with NO user code, and
`store_bind_lazy(persons, "sqlite:people.db")` needing no loading step is a
shipped promise.

So it is an opt-in with a measurement:

- **A declared driver WINS**, including over a source core drives in Rust. A
  program moves its sqlite reads onto loft one element type at a time; every type
  with no driver keeps the Rust source. `Stores::lazy_loft_source` now takes the
  caller's answer to *"is there a driver for this element type"*, because the two
  backends learn it differently — the interpreter asks `Data`, `--native` asks the
  table generated `init()` filled — and neither is reachable from `Stores`.
- **The two paths are proven indistinguishable.**
  `tests/fixtures/sqldb/s9_two_paths.loft` puts two element types of one shape
  over two identical tables in ONE program bound to ONE connection string. Same
  values, same float, same identity, same residency counts, same absence handling
  — and the trip count, which is the only thing a value check cannot see: three
  lookups reach the driver and the repeat of a resident key reaches none. Both
  backends, byte-identical.
- **`select_by_key`** — the `select(TableDef, key)` the design table always listed
  — derives the statement from the same `TableDef` a writer would `render` into
  `CREATE TABLE`, wrapping a float column in the dialect's read expression. The
  driver names no column.
- **`Data::lazy_fetch_drivers` is cached** per definition count. It walks every
  definition and sits on the MISS path, which is the one place @PLN129 measures in
  queries per lookup. Keyed on the count rather than answered once, because the
  REPL parses fresh sources into a live `Data` and a driver can appear after a
  lookup has already asked.

**The cost, attributed rather than assumed.** A loft driver has nowhere to keep a
connection — loft has no process-level state a library can hold — so it connects
per missed row where core caches a handle per target. Release build, 400 fetches
each: **67 µs** per fetch through Rust, **140 µs** through loft. ~2.1×, because a
local sqlite file reopens cheaply. What that does NOT cover is the case that
matters most: for a client-server backend the same shape is a TCP connect and an
auth per row, and those are precisely the backends core has no Rust driver for.

**S10 is not unblocked by this.** Deleting the Rust path makes a driver
mandatory, and a driver names a concrete element type so it cannot come from a
library — a program binding `sqlite:` with no user code would stop working. That
needs a generated driver (making the sqldb library a dependency of core) or a
demotion rather than a deletion, and it is a decision about what loft's
distribution contains.

**Filed on the way past:** [loft#810](https://github.com/loft-lang/loft/issues/810)
— a library function that both holds a `vector` local and returns a record of
another package's type SIGSEGVs on the second call when the caller binds the
result to a loop-body local. `Store::copy` computes `size * 8 - 4` from a record
whose size word reads `0`. Six axes were moved one at a time to find the
boundary; the driver takes the passing cell (a fresh `derive` per fetch).

### A lazy driver serves ONE element type (@PLN133 S9 prerequisite, 2026-08-08)

S8 let a program declare one `lazy_fetch`, which reads as a limit on how many
collections may be lazily bound. It was not only that: **nothing checked that the
driver a miss reached was declared for THAT collection.** S8's shape check was
about the driver's signature and never about its subject, so a program with two
lazily-bound element types ran the first type's driver against the second
collection — measured on both backends, inserting a `TdcPerson` into a
`hash<TdcOrder[id]>` and reading `.what` back as `person-9-postgres://db/people`.
One type's field through another type's offset: a plausible value, which is the
class @PLN129 arc C exists to keep out.

One mechanism does both jobs — the driver is looked up by the collection's
ELEMENT TYPE, so several drivers become possible and reaching the wrong one
becomes impossible.

- **`Data::lazy_fetch_drivers`** answers `(element type name, def_nr)` per driver
  and is the single home both backends ask. What a driver serves is read off its
  declared collection parameter, never guessed from its name.
- **The key is a NAME**, because the two sides count types in different spaces (a
  parse-time `Definition`, a runtime `Stores::types` entry) and a name is the one
  key both hold without a mapping to keep in step — `LOFT_STRICT_SCHEMA_IDS`
  exists because that kind of mapping drifts.
- **Membership needs more than the name.** `lazy_fetch` exactly is THE driver
  name, so a wrong shape there is named; `lazy_fetch_<anything>` additionally
  requires a keyed collection as its first parameter. The first version of this
  rule keyed on the name alone, and a plausible helper (`lazy_fetch_row`) was then
  read as a malformed driver and poisoned every lookup in the program, including
  the working driver beside it.
- **Two drivers for one element type are refused, naming both.**
- **`--native` installs one pointer per driver** under the same key, and every
  driver is a reachability ROOT — a driver left out of the walk is S8's quiet
  failure arriving once per type instead of once per program.

**A backend divergence had to be closed to gate the refusals, and it was S8's.**
The interpreter asks `Data` at every miss and reports the sentence it wrote;
`--native` cannot ask, registered nothing, and said *"needs a loft driver"* — the
same program naming a different mistake depending on which backend ran it, and
the one naming the real mistake was the one you did not get if you compiled. The
refusal now travels as data (`register_lazy_fetch_refusal`) and the no-driver
sentence has one home (`database::lazy::no_lazy_driver`).

**The emission diff is one line.** `loft introspect` over the two-driver corpus
before and after differs only in the registration — one
`register_lazy_fetch(n_lazy_fetch)` becoming two keyed calls — with nothing else
in the IR, the bytecode or the generated Rust moved. Corpus and both captures:
`doc/claude/plans/133-sql-one-boundary/bytecode-comparisons/two-drivers-*`.

Gated by `tests/fixtures/133-lazy-driver-dispatch.loft` (three element types over
`hash` and `index`, a fourth bound with no driver, a prefix-sharing helper,
absent-vs-unreachable) plus two refusal programs, through
`tests/lazy_sql_source.rs`, both backends with the whole output compared. The
`orphan` cell asserts a driver-call COUNT rather than a value: a collection whose
type no driver serves must reach none, and a value check alone would pass on a
driver that happened to answer nothing.

### One connection string, four C libraries (@PLN133 S7, 2026-08-08)

Requirement 1 is *one configuration string switches every SQL consumer in the
process*. S5 delivered the parser; this is the half that hands back a connection,
and it had no obvious spelling because **loft interfaces are static dispatch** —
`SqlDb` is satisfied by four unrelated types and no function can return "one of
them".

- **`tests/fixtures/sqldb/registry/`** — `AnyDb`, a struct-enum over the four
  backends plus a refusal variant, satisfying `SqlDb` itself. `connect(spec)`
  parses the string, asks whether that backend's library is on this machine,
  opens it, and runs the dialect's session setup. Shape (1) of the three the plan
  named; the decision and the two it was chosen over are recorded in the file's
  header, because (2) is cheaper and the difference is visible to every consumer.
- **The method must be on the ENUM, not the variant.** A per-variant method
  dispatches correctly and does not satisfy an interface for the enum — the
  compiler says so: *"'AnyDb' does not satisfy interface 'SqlDb': missing
  db_exec"*. Fifteen `match self` forwarders, none of which decides anything.
- **A refusal is the fifth variant, not a null** — every operation false, every
  column null (not `""`, which is a value), `db_last_error` saying why. The idiom
  `TableDef`, `Binding`, `Conn` and `SqlText` already use in that package.
- **The connection string is not one string.** What a driver's own `db_open`
  wants is a fact about the driver: sqlite and duckdb take a path, libpq reads a
  URI itself (so it must arrive WITH its scheme), and mariadb's client takes
  keywords, so a `mysql://…` URL is translated. A PORT in that URL is **refused
  rather than dropped** — the driver connects on 3306 and reads no port, so
  honouring the string would reach a different server than it names.
- **The session setup finally has somewhere to run.** `Dialect.setup` has carried
  PostgreSQL's `SET extra_float_digits = 3` since S3 with no caller. @PLN133 P3
  measured 1887 of 2000 random doubles inexact without it and 0 of 2000 with it,
  and it is a SESSION setting — so the connect is the only place that can make a
  float read back exactly, and nothing downstream can see that it did not.

Gated by `tests/fixtures/sqldb/registry_pure.loft` (unconditional — it opens no
library, so it cannot skip into a green that asserted nothing) and
`registry_live.loft`, through `tests/native.rs`, both loft backends with the whole
output compared.

### `τ?` is one type however it was handed back (2026-08-08)

`Type::is_same` compared `Optional(τ)` with derived `==`, which reaches the inner
`Deps`. Every dep-ignoring rule below that comparison — a text's deps, an
integer's range, a vector's element buffer — was therefore unreachable for a
nullable, so two `text?` differing only in which local they came through read as
different types. It presents as a refusal quoting the same name twice:

```
error: cannot unify: text? and text?
```

Peeled on BOTH sides only, so a `τ?` and a bare `τ` stay different kinds — that
distinction is the whole of DN1. Found by @PLN133 S7, where a `match` forwards
`db_col` to four backends and one of them returns through a local; the same
comparison also gates interface satisfaction and the @P344 loop-variable reuse
check, both of which wanted the dep-insensitive answer all along. Guarded by
`tests/scripts/pln133-optional-unify.loft`.

### A returned text is owned by the return, not borrowed from a buffer (2026-08-08)

A branch in tail position delivers each arm into the return accumulator. An arm
whose text is not a bare variable is first built into a work buffer and handed
back as `OpCreateStack(buf)` — a REFERENCE — and `push_text_arms_into` wrapped
that reference in the delivery, while the enclosing scope frees the buffer on the
next statement.

- **The interpreter answered `""`** — a wrong value, silently, exit 0.
- **`--native` emitted `*var_acc = ().to_string()`**, which is not Rust.

The shape is ordinary: `return x ?? "fallback"`. Binding to a local first
(`y = x ?? "fallback"; return y`) avoided it, which is what made the failure look
like it was about `??` rather than about delivery. The leaf rewrite now delivers
the BUFFER, so the accumulator copies the bytes it is about to own. Guarded by
`tests/scripts/pln133-text-tail-delivery.loft`, whose every cell has a
deliver-through-a-local twin.

Found not by the registry — which does not contain that shape — but by the
regression test written for the `Optional` fix above, whose helper happened to
spell `return got ?? "<null>"`.

**Still open:** [loft#806](https://github.com/loft-lang/loft/issues/806) — a
METHOD call coalesced in return position (`return t.m(i) ?? "x"`) SIGSEGVs the
interpreter while `--native` is correct. The caller-retbuf promotion makes the
callee's work buffer a `&text` PARAMETER and the `#default ref` site then wraps an
already-borrowed variable in `OpCreateStack`, building a reference to a reference.
Workaround: one intermediate local.

### `#c` on a wasm target, and under the sandbox (@PLN24 arcs E–F, 2026-08-08)

Closes @PLN24. Both remaining arcs plus the plan's last open design question.

**Arc E — the two wasm targets get a defined answer, and it is a refusal.** The
plan had recorded wasm as having "no C ABI to bind to at all". It has one:
`wasm32-wasip2` links a libc, so a `#c` binding to `strlen` resolved, LINKED with
a `rust-lld` warning, and then TRAPPED at the call — `signature_mismatch: strlen`,
`(i32) -> i64` against the sysroot's `(i32) -> i32`, because wasm32 is a third
data model (ILP32) while the extern carried the host's widths from
`CTarget::host()`. That is this plan's counted `N × silence` risk arriving at a
re-assertion site nobody listed: one of the targets is not the host.

Two further cells, both measured on one tree: a symbol the sysroot does NOT export
gave a raw `rust-lld: undefined symbol` naming neither package nor library, and a
package declaring `[c] optional-libs` gave `E0433: cannot find c_call in loft`
once per symbol — **for bindings the program never called**, because the lazy
resolver is emitted per declaration rather than per call.

- **Nothing `#c` is emitted on a wasm target** — no `extern "C"` block, no lazy
  resolver. `Output::no_c_abi()` is the single reader of the two target flags, so
  the three sites that consult it cannot drift into different answers.
- **The refusal sits at the CALL** (`output_c_direct_call`), which scopes it to
  reachability for free: an unused `#c` declaration still builds for wasm, the
  rule `#native` already follows for a routeless browser symbol (@PLN26 / P269).
  It names the loft function, the C symbol, the declaring package and the target.
  The PACKAGE, not the library: a `#c` annotation never names the library it came
  from (arc G), and one of a package's `[c]` entries is the shim loft built itself.
- **`__C_LIBS` / `__C_LIB_SYMS` moved OUT of the target gate.** They were emitted
  only on non-browser targets, so `c_library_available` — the query a library is
  told to ask before calling into an optional backend — failed to compile under
  `--html` with `E0425`. A refusal that names a cure has to leave the cure
  reachable. It now compiles on both wasm shapes and answers `false`, which is the
  true answer rather than a stub.
- The static-`clang --target=wasm32-wasi` route stays unbuilt and is recorded as
  such: no C cross-compiler was available to prove one cell, and it reaches only a
  pure-computation shim. `@PLN119` (out-of-process) is the route the message names.

**Arc F** was already closed by @PLN23 S1 (`libmariadb.so.3` through a versioned
soname, both backends identical, zero rustc); the plan's status table said
otherwise.

**Open question 3 — a `#c` binding is gated by `native_ffi`, not by `#cap`.** The
question asked whether an effect declaration could make `#c` admissible under the
sandbox. Measured first: a sandboxed script reaching a `#c` binding tagged
`db#read`, under a profile granting `db#read` with `native_ffi` at its default
false, was **admitted and ran the C**. Both the external-FFI ban and
`reachable_ffi_bridges` key on `def.native()`, which arc A leaves EMPTY on a `#c`
definition on purpose so the Rust dispatch path cannot claim one — the inverse of
arc D's three defects, where paths matching on *body-less* wrongly CLAIMED a `#c`
def. `CapViolation::CBinding` is the new arm; `allow_libs` still admits it, which
is the host vetting the library as a unit exactly as for `#native`.

Guards: `a_c_binding_is_refused_by_name_on_a_wasm_target` (emission-level, so it
runs without a wasm toolchain, and it calibrates against the host emission so a
refusal that fired everywhere could not read as a pass),
`pln24_a_reachable_c_binding_is_refused_end_to_end_on_wasm` (both shapes, asserts
exactly ONE message and that loft's own feature gates never reach the author),
`pln24_html_c_library_available_compiles_and_answers_false`, and
`a_c_binding_is_gated_by_native_ffi_not_by_a_capability_grant` (three cells:
granted cap rejects, `native_ffi = true` admits and calls, `allow_libs` admits).

### A module may name the entry's type in an EXPRESSION (loft#801) (2026-08-07)

Companion to loft#797, which fixed the LAYOUT half of the same load-order story. This is
the resolution half.

A forward reference resolves through a **stub**, not a lookup: an unresolvable type name
becomes an `add_def(name, …, DefType::Unknown)`, `use` imports it into the importer along
with the module's other names, and the importer's own `struct` / `enum` / `type`
declaration upgrades it IN PLACE so both files share one def. `Data::def_nr` is keyed on
`(name, source)` with only a source-0 fallback, so there is no cross-source lookup at all —
adoption is the entire mechanism. Documented in COMPILER.md § How a forward reference
actually resolves, because nothing said so.

The consequence was that only a spelling which LEAVES a stub could be forward-referenced.
Written types go through `parse_type`, which leaves one; expressions did not. So
`r: Roofs = Roofs { … }` compiled and the identical `r = Roofs { … }` did not — the same
name, the same file, decided by whether an annotation happened to be written.

- **Two sites in `parse_var` now leave the same stub** — the `Name { … }` construction
  branch and the bare-name branch. Both pass 1 only. They are tracked in
  `speculative_type_refs` so `resolve_deferred_unknowns` stays quiet about an unadopted
  one and the construction site still reports in pass 2 with the author's own spelling and
  its suggestion — reporting both is the one-typo-two-errors cascade #376 removed.
- The bare-name branch also stops creating a placeholder VARIABLE for such a name. A
  function's variable table survives into pass 2, so the pass-1 placeholder was still
  there when pass 2 looked the name up, and it shadowed the type the declaration had
  meanwhile produced.
- Its name test is deliberately NOT `is_camel`, which answers "not lower_case and no
  underscore" and so accepts `FOO`, `N`, `X`. Treating those as types took the placeholder
  variable away from every misspelled constant — the `upper-case-local` advice and
  `Unknown variable 'N'` are written against it. A type name carries a lowercase letter.
- **`parse_typedef` adopts a stub**, which `parse_struct` and `parse_enum` already did. It
  was reporting the waiting stub as a name clash, so a typedef was the one declaration
  kind a module could not forward-reference.
- **`parse_file` drains `todo_files` on a plain Error**, stopping only on Fatal. That list
  holds the files SUSPENDED at a `use` — the importer, waiting for the module it pulled in
  — and they had not been parsed at all, so abandoning them did not avoid a cascade, it
  invented one: the definitions they carry never registered, and one error in a module
  produced a second saying a type was undefined while it was declared two lines away in
  the importer.

Fixed spellings (both backends): a local built by construction alone, a vector literal,
iterating that vector, the type as a value argument (`sizeof(T)`), and a typedef.

**Not fixed, and deliberately excluded: a bare name qualifying a VALUE (`Colour.Green`).**
It fails in a single file too, so it is not a module problem, and it is loft#803. Leaving
the stub there makes the program COMPILE and evaluate to `unknown` for every variant — a
wrong answer where there had been an error. That issue records two further attempts (an
enum-aware `layout_blocked`; registering enum stragglers from `fill_all`) and exactly what
each broke. Read it before patching.

Guards: `module_names_the_entry_type_in_an_expression` (`tests/issues.rs`) over the
`fwd801` fixture, and golden case `47_module_error_keeps_importer`, whose baseline pins the
ABSENCE of the invented second error.

### A field whose type another module declares gets a slot (loft#797) (2026-08-07)

A package entry that `use`s a module before declaring the types that module names
suspends itself at the `use`. The module was then parsed to completion — layout
included — while every such type was still a `DefType::Unknown` stub. `fill_database`
skips a field whose `type_elm` is `u32::MAX`, so the field got no slot; the stub was
upgraded in place moments later when the entry resumed, so the DECLARATION ended up
correct and the LAYOUT kept the hole, and nothing revisits a registered type. Only
the load ORDER decided it.

`fill_all` now defers a layout until every field's type is known, and re-asks on each
call. Three sites had to agree:

- **`layout_blocked`** (was `has_nameless_unknown_attr`) — covers `Unknown(stub)` as
  well as `Unknown(0)`, and is TRANSITIVE: an inline field stores its content's bytes,
  so a host whose field type is waiting cannot be laid out either. The loop is keyed on
  `known_type == u16::MAX`, so this defers rather than drops.
- **A sweep at the top of `fill_all`** re-runs `copy_unknown_fields` over everything
  still unlaid. Without it the deferral never ends — `actual_types_deferred` sweeps only
  the file it is finishing, and nothing was asking again.
- **`Type::Optional` is peeled, not matched.** `S?` and `S` name the same forward
  reference, and three places that peel `Vector` had all forgotten the `?`:
  `copy_unknown_fields`, `Data::rewrite_type_opt`, and the native `init()` generator's
  field-hoist match. The last one emitted `db.field(t_host, "f", t_content)` ahead of
  `let t_content` — the generated crate did not compile, so a nullable forward field
  broke the library build where a plain one worked.

Also from the same matrix, both now diagnostics rather than panics: a keyed collection
whose content was a stub indexed `attributes[usize::MAX]` in `set_mutable`, and a vector
literal of a stub element tripped `new_record`'s `assert_ne!` as an internal compiler
error. The first is gone with the deferral; the second reports `type 'X' is not defined
here — use the module that declares it`.

Not closed, and out of scope: a type named in a function BODY rather than a field
DECLARATION is not deferred, so a module naming a type it cannot see still fails with
`unknown type 'X'`. That is resolution, not layout — filed as loft#801, together with the
cascade that makes it expensive (`parse_file` returns on error before draining
`todo_files`, so the suspended parent is never re-parsed and the type it declares is then
reported undefined).

Guard: `forward_module_type_gets_a_slot` (`tests/issues.rs`) over the `fwd797` fixture,
asserting sizes as well as values — a read follows whatever offsets the layout ended up
with, so reading a field back cannot by itself prove the field has storage.
`tests/field_without_storage.rs` (loft#796's guard) changes with it: the hole it used as
a trigger no longer exists, so both its tests now assert the ANSWER.

### Lazy stores — the fault is the collection's MISS path, and a SQL source drives it (2026-08-06)

`store_bind_lazy(c, source)` binds a collection to a store image or to
`sqlite:<path>`; a lookup that misses consults the source, materialises the record
and retries. Both backends. The model, the derivation and what is refused are in
[LAZY_STORES.md](LAZY_STORES.md); what matters here is where the hook went and why.

**Not at `Store::addr`, which counted better.** All 14 typed getters funnel through
it behind `valid()`, and the native `#rust` bodies call the same accessors, so one
site would have served both backends. But `valid()` is unconditionally `true` in
release — every check inside it is a `debug_assert!` — so the "one site" does not
exist yet, and creating it puts a branch on the hottest path in the language, paid
by every program. The hook is `Stores::find`'s miss path instead, which already
spells a miss as `rec: 0` and already has exactly two call sites (`State::get_record`
and `codegen_runtime`'s lookup), both holding `&mut Stores`.

**Residency needs no representation.** It is absence from the collection. No third
block state, no cost in `valid()`, and the resident set doubles as the cache — which
is what makes identity fall out of the ordinary lookup instead of a `(type, key) → rec`
map that could diverge from the store.

New modules: `database/sql_query.rs` (the derivation + `Mapping`),
`database/sql_source.rs` (the connection, through `c_call::resolve` with typed
`extern "C"` pointers — no rustc, no loft frame, no re-entrancy),
`database/lazy.rs` (the `LazySource` seam and the materialiser, which reuses
`record_new` + `record_finish` so a SQL arrival and `coll += [x]` end in the same
place).

Three findings worth carrying:

- **The derivation quotes every identifier**, because `from` is an ordinary loft
  field name and a reserved word everywhere. Quoting removes the class rather than
  one word of it.
- **SQLite reads an unresolvable double-quoted name as a STRING LITERAL** —
  `SELECT "naam" FROM "person"` returns the text `naam` once per row, so a renamed
  column would have been materialised into the record. The connection disables it
  (`SQLITE_DBCONFIG_DQS_DML`/`_DDL`); versions before 3.29 do not know the option,
  which is why the schema check is a requirement rather than a guard.
- **An `index` element carries its own red-black links** (`#left_1`, `#right_1`,
  `#color_1`), and `#color_1` is an ordinary boolean — so a column filter written on
  field TYPE named a column no table has. `LayoutField::is_data` now has one home,
  shared with `read_via_descriptor` and the browser delivery.

The tests need `libsqlite3` at runtime and self-skip without it, which is a skip
that reads as a pass. CI now installs it and sets `LOFT_REQUIRE_SQLITE=1`, which
turns the skip into a failure; elsewhere the skip is recorded in the
environmental-skip ledger and surfaced as an annotation. `tests/lazy_sql_source.rs`
also serialises on one mutex: `c_call::register` REPLACES the declared-library list
with the running program's own, so a test that merely runs a loft script was wiping
its neighbour's sqlite declaration.

### The IR store holds a block BY REFERENCE — `Node` shrinks 48 → 28 bytes (2026-08-04)

`NdBlock` / `NdLoop` inlined a whole `Block`, and `NdParFor` a whole
`ParForBody`, so a `Node` record was as wide as its largest variant: 48 bytes,
paid by every node in the image including a 12-byte `NdVar`. They hold a
**box-of-one vector** now — the idiom the schema already uses for `Block.result`
and `DbField.default` — and the stride is 28.

A box is a 4-byte handle. `reference<Block>`, which `ir.loft` had drifted to,
generates a 12-byte `Parts::DbRef`: the same indirection for three times the
width, with no other reader of one in this store and no existing helpers. The box
reuses `field_recvec` / `push` / `get` unchanged.

What moved together, because a half-done version of this is a store that reads
its own records at the wrong offsets:

- `ir.loft` → regenerated `ir_schema_gen.rs` (the field is a vector handle now).
- `data_store.rs`: `NDBLOCK_BLOCK` / `NDPARFOR_BODY` are the HANDLE offsets, and
  the sub-struct constants became the sub-record's own — `PARFOR_X_VAR` is 0, not
  `body_base + 0`. New `Node::block_rec` / `Node::par_for_rec` reach the record,
  and `write_block` / `write_loop` / `write_par_for` push the box and hand it
  back so a caller fills it without a second lookup.
- `ir_store` / `ir_read` / `ir_node` read and write through those records.
- `CACHE_FORMAT_VERSION` → 3: every offset in a `Node` moved.

The layout guard in `data_store.rs` is what made this safe to do — it asserts
each baked constant against the registered schema, so the migration was a
conversation with a failing assertion rather than a hunt for silent corruption.

### `ir_schema_gen.rs` regenerates byte-identically again (2026-08-04)

The IR store-schema generator had been unusable, so schema edits were HAND-ADDED
to the generated file — which is how it drifted out of sync with `ir.loft`
without anyone seeing. Two independent defects and one wrong declaration:

- **`tN` labels were absolute type ids.** `generated.rs` numbers types after the
  whole stdlib and `extract.py` copied those names verbatim, so adding ONE stdlib
  type renumbered every label and a fresh regen differed in ~1300 lines. They are
  only Rust locals, so the extractor now relabels ours in declaration order from
  `t7` (after the `t0..t6` base prelude). Proven: a binary WITHOUT
  `default/07_reflect.loft` and one with it now produce byte-identical output.
- **Named locals were dropped.** The keep-rule listed `byte_enum` and `vec_*`
  only, so a field whose storage local was `dbref_*` referenced a name nothing
  bound and the regenerated file did not compile — which is what forced the
  hand-adds. Every `let <name> = db.…` is kept now.
- **`ir.loft` described `src/data.rs` instead of the STORE.** It had drifted to
  `NdBlock { block: reference<Block> }` because `data.rs` boxes it; the store
  INLINES the block and the hand layer reads it that way
  (`NDBLOCK_BLOCK + BLOCK_SCOPE`). Regenerating from that produced a schema
  nothing could read — SIGSEGV in five IR round-trip tests. `ir.loft` says
  `Block` again, with the reason written beside it: making that field
  by-reference is a real store migration (schema, `ir_store`, `ir_read`, the
  baked offsets, `CACHE_FORMAT_VERSION`), not a transcription change.

The committed schema's CONTENT is unchanged — the regenerated file matches the
previous one registration for registration. What changed is that it is
reproducible, so the next schema edit is a regen rather than a hand-edit.

### @PLN127 arc D: reflection reports field nullability (2026-08-04)

`FieldInfo.nullable` — and the line it draws is the contract decision the plan
asked for: **reflection reports what a VALUE can be, not what CODE may do to it.**
Nullability is the first kind; `const` is the second and stays out.

Neither was a storage fact. `text?` and `text` share a content type and spell
absence with a SENTINEL, so nothing in the stored bytes implies either. (A NARROW
int is the exception — it registers a distinct content type per nullability, which
is why the descriptor reported nullable for those and only those.) The fact
therefore had to be DEPOSITED: `Field.nullable`, set at the one parse-time site
that knows (`typedef.rs`, where `Optional(τ)` is peeled before layout), carried by
`LayoutField`, and read back by `reflect_type_into`.

Nullability is deliberately **not RENDERED**. `layout_algo_hash` hashes
`layout_dump`, and `LayoutDesc::layout_hash` hashes `render_dump` — neither
mentions it, so the @PLN97 layout identity is untouched. That is measured, not
argued: a store written by the pre-arc-D binary loads under the arc-D binary
through both the whole-image and keyed paths with `ok=true`, and the same gate
still REFUSES a genuinely reshaped layout, so the check is not vacuous.

**`--native` reported `nullable=false` for every field** until the generator
emitted the deposit too — it rebuilds the schema by REPLAYING `init()`, so a fact
the parser deposits and the generator does not emit is simply absent there. The
parity probe caught it. The emission wraps `emit_field` rather than sitting in
either caller, because there are two call sites and the one that mattered was not
the obvious one.

The setter is keyed by NAME rather than field index for the same reason: the
generated `init()` writes it beside the `db.field` it belongs to, and one spelling
for both backends is what stops them disagreeing.

It also had to reach the @PLN11 IR-store round trip (`ir_store` / `ir_read` /
`DbField` in `tools/ir_schema/ir.loft`), or a schema read back from a store
answered "not nullable" for every field — caught by
`read_stdlib_schema_round_trips`. That grew `DbField` by a byte (stride 28 → 29),
which needed **`CACHE_FORMAT_VERSION` bumped to 2**: the stdlib cache key does not
fold in the binary's mtime the way a program bundle does (`BUILD_ID` is the git
HEAD hash, unchanged across uncommitted edits), so a cache written at the old
stride was read at the new one and panicked in `ir_read` on a shifted
discriminant — 25 LSP tests, all one cause. A layout change is exactly what that
byte is for. The registration is HAND-ADDED to
`src/ir_schema_gen.rs` beside the existing `ty_optional` hand-add rather than
regenerated: a clean `extract.py` run reorders that whole file today, so a regen
would fold an unrelated drift into this change. `ir.loft` carries the field, so
the source of truth is right and the regen cleanup stays its own task.

Arc E's generator is what made this concrete — written before arc D it was
complete, correct, and could not emit `NOT NULL`, which does not make a DDL less
detailed, it makes it accept rows the loft type would refuse.

### @PLN127 arcs C + E: `type_named`, and the consumer that used the API as a gate (2026-08-04)

`type_named(name) -> TypeInfo?` is reflection with no value in hand — the shape an
ORM needs when the type name arrives from a config file or a catalogue. No parser
intercept, because the name is a RUNTIME value; it works on `--native` because the
generated `init()` replays the type registrations, names included, and
`Stores::name` is a TOTAL lookup that answers absent rather than minting a type
for a typo. Both entry points reach ONE filler, so they cannot disagree.

**That is the plan's Q1 answered rather than worked around.** It expected a
runtime name→id lookup to be impossible under `--native`'s replayed type table;
the replay includes the names.

Arc E is the dogfood gate: `tests/scripts/pln127-reflect-consumer.loft` generates
`CREATE TABLE` from a loft struct through the API only, with the table name as a
runtime value. It passes on both backends, and used as a gate it found two limits:

- **It cannot emit `NOT NULL`.** Nullability is not in the answer because it is
  not in the STORE — `Field` carries a name, a content type-id, a position and a
  default. A narrow scalar records a nullable flag; `text` and a record reference
  spell absent with a SENTINEL instead (`text?` is stored as `"\0"`, the fact arc
  A repaired in the JSON writer). `const` is the same.
- **It had to be a schema generator, not a serialiser.** Reflection describes a
  TYPE; a value's field cannot be read by name, and a serialiser needs both.

So arc D's question changed shape: not "grow the descriptor by two fields" but
"does reflection report facts that exist only in the SOURCE?". One measurement
bears on the cost — `layout_hash` hashes `render_dump()`, so a fact the descriptor
CARRIES but does not RENDER leaves the @PLN97 layout identity untouched. The
carrying is cheap; the depositing is the decision.

### @PLN127 arc B: `type_of(x)` — the declared shape of a type, as data (2026-08-04)

loft had VALUE reflection (`{x:j}`, `Type.parse`) and FRAME reflection
(`stack_trace`) reachable from loft code, and a SCHEMA level only Rust and a
foreign JavaScript reader could see. `default/07_reflect.loft` brings the third
one across: `TypeInfo` / `FieldInfo` / `VariantInfo` / `TypeKind`, filled from
@PLN105's `LayoutDesc` — the descriptor the browser bridge already reads, pinned
byte-for-byte against the @PLN97 layout dump. Reading THAT rather than walking
`Parts` afresh is what stops reflection becoming a second, drifting description
of the same layout.

`type_of(x)` is intercepted in `parse_call_extra` and lowered to
`n_reflect_type(<type-id>)`, so the id is a parse-time constant — the mechanism
`to_json` already uses. One filler (`native::reflect_type_into`) serves both
backends: the interpreter through `src/native.rs`, `--native` through a
`codegen_runtime` wrapper onto the SAME function.

**The argument is not evaluated.** Nothing about the answer depends on the value,
and evaluating it would mean discarding a result — the operation loft's ownership
model gets wrong most easily (loft#771). The contract is C's `sizeof`, and the
doc comment says so.

Three things the build settled:

- **The plan's Q1 dissolves for `type_of`.** `--native` REPLAYS the type table
  rather than minting it, which is why a runtime name lookup was the plan's one
  load-bearing question — but a parse-time id is replayed with the table. Q1 is
  arc C's question, not arc B's.
- **Q3 answers itself for exactly two scalars.** `get_type`, the one existing
  storage derivation, reports `integer` for a `character` (which is how it is
  stored) and has no entry at all for a `boolean` (`#65535`). Those two are named
  directly; everything else keeps the single derivation, because a second one is
  a second thing to drift. Narrow ints still report storage, and `size` shows it.
- **Reflection inside a generic is not reachable this way.** A generic body is
  parsed ONCE against its type variable, so `type_of(v)` there answers
  `__typevar_T` — the same mechanism that makes `"{v:j}"` in a generic body render
  `{}`. Stated in the doc comment rather than left to be discovered.

Arc A was a prerequisite in fact, not just in order: a `TypeInfo` holds an enum in
a struct field, the exact shape that made `json_parse` reject a whole document, so
`"{t:j}"` on a `TypeInfo` renders complete JSON only because arc A landed.

`tests/scripts/pln127-reflect.loft`, both backends: a record with hand-checked
byte offsets, an enum whose tags start at 1 (0 is how the store spells ABSENT), a
struct-enum variant, a nested record, a vector's element, all five scalars, and
the `TypeInfo` itself serialising.

### @PLN127 arc A: the JSON form is the only field enumeration loft has, and two shapes broke it (loft#768, loft#769) (2026-08-04)

`{x:j}` + `json_parse` is what a generic serialiser, an ORM or a schema walk
reaches for, and both defects were WHOLE-DOCUMENT failures rather than a wrong
field — a struct holding either shape could not be read back at all.

**An enum-typed field wrote its tag bare** (`{"kind":Circle {"r":2}}`), an
unquoted token in value position, so the text was not JSON and `json_parse`
returned null for everything. Two writers render an enum and only one knew about
JSON: `Parts::EnumValue` (an enum-VALUE position, the bare `Circle{…}` form)
wrapped as `{"Circle":{…}}`, while `Parts::Enum` (an enum-TYPED position — a
struct field, a vector element) did not. The typed position now wraps the same
way, and `walk_parsed_into` already accepted that shape as a tagged variant, so
writer and reader name one shape between them rather than two. A fieldless
variant gets a body (`{"Dot":{}}`) so every variant reads back through one path;
an absent discriminant and one naming no variant this schema has are both `null`,
which is what the reader already degrades an unknown tag to.

**An absent `text?` is stored as the sentinel `"\0"`, not as a null pointer**, so
it reached the JSON escaper and came back as the one-character string
`" "` — a present, corrupt value where the program meant nothing. It is the
same absence the null-pointer branch beside it already rendered as `null`, so it
renders the same way. That is the distinction the type exists to carry: SQL NULL
and `''` stay different answers across a round trip instead of collapsing.

The debug form (`{x}`) is deliberately unchanged — it shows the representation,
and only the `json`/`loft` re-parseable forms make a claim about round-tripping.

Seven cells in `tests/scripts/57-json.loft`, both backends, each proven able to
fail first: the field form, its round trip through `.parse`, a fieldless variant,
an enum inside a vector, the bare form unchanged, null-text as JSON null, and
absent-versus-empty surviving as themselves.

### @PLN124 H6/H7: an interpolation hole may be a value of a NAMED type (2026-08-04)

`format_hole` read a hole's kind off the value's type and accepted six scalars; a
struct or enum was a compile error. It now derives the kind from the type's own
NAME in the case a loft method is spelled in — `SqlIdent` asks for
`hole_sql_ident`, `Level` for `hole_level`, and an acronym run breaks at the last
capital (`SQLIdent` → `sql_ident`). Derived rather than chosen, so a target and
the parser cannot disagree about what a type's hole is called and the diagnostic
names the exact method to add. The refusals are unchanged: a kind the target does
not define, and a spec on any hole, are both errors.

That is what lets a target hold something apart from BOTH a literal and a bound
value. The motivating case is @PLN23 H6: a SQL table name is genuinely syntax, so
`SqlText` puts it in inline — and the safety rests on the TYPE, because nothing
builds a `SqlIdent` but its validating constructor.

**A second leak of the expected-type channel, into the HOLE.** A hole is not the
destination, so a string literal inside one must not inherit the destination's
type; without that, `q: SqlText = "{"seed"}"` checked the inner literal against
`SqlText` and it took the BUILD path. The same leak the arc closed per call
argument, one level in, and found only once a consumer wrote a text hole inside a
built statement.

The fix is narrower than the call-argument one deliberately. Clearing `expected`
for the whole hole broke `store_load_layout_gate` on `--native`: the hole
`"{(h[42] ?? Tile { … }).name}"` is a KEYED LOOKUP, and a keyed lookup resolves
its record type through that same channel, so blanking it silently changed the
schema the generated `init()` replays. Only the TARGET derivation is gated now
(`in_format_expr` in `constant`), and only its `expected` source — `var_tp` still
applies, since a declaration written inside a hole does name a destination. Cost:
a format string in argument position inside a hole is plain text, which is a
visible type error at the call rather than a silent difference.

Inertness re-proved after both changes: the 104-site corpus is byte-identical in
IR and in generated Rust.

@PLN23 H6/H7 rest on it — `SqlIdent`, and procedures as named parameterised
statements (`CREATE OR REPLACE PROCEDURE` + `CALL` on postgres/mariadb, a shared
process-side registry for sqlite/duckdb). Two findings from that build:

- **Identifier quoting is chosen at ASSEMBLY time**, not when the hole is filled:
  mariadb reads `"loft_p"` as a string literal and wants a backtick, measured as
  a syntax error when given the ANSI quote the other three use.
- **A procedural body is refused on all four backends**, not just the two with no
  procedural language. mariadb writes them in SQL/PSM and postgres in plpgsql or
  `BEGIN ATOMIC`, and neither reads the other's, so there is no such body a
  uniform API could carry. One statement per procedure, refused where the author
  can see it.

### `chr(cp)` names the code-point constructor that already worked (loft#748) (2026-08-03)

`cp as character` already produced the right character and interpolating it
produced the right text, so what was missing was the ENTRY POINT, not the
capability — the same shape as this issue's byte half, where `text_from_bytes`
had existed for two releases and was reported missing because the generated
reference filed it under Environment.

`chr` is a loft-level definition in `default/03_text.loft` beside
`text_from_bytes`, not a new `#rust` native: the mechanism is already proven on
both backends, and loft covering it is exactly when not to add a dependency.

The refusal set is `""`, never a crash (C80): a surrogate, past `U+10FFFF`, a
negative number — and `0`, which is the one that needed deciding. `character`
uses 0 as its null, and text ITERATION STOPS at an embedded NUL (measured: a
3-byte `"A\0B"` reports `len` 3 and slices at 3, and yields ONE character to
`for`), so a NUL built by `chr` could not be read back by the loop `chr` is the
inverse of. The byte route still carries one. That iteration/`len` disagreement
is filed separately.

`doc/claude/STDLIB.md` gained a **Bytes and code points** section: it listed
neither `byte_at` nor `text_from_bytes` either, so #748's discoverability defect
was live in the agent-facing doc as well as the generated one.

### A tail expression that is a place read no longer reaches the Return as null (loft#754) (2026-08-03)

A function body ending in `w.items[i].bytes` returned an EMPTY vector under
`--native` and the right one under `--interpret`; putting `return` in front of
the identical expression fixed it.

A tail with pending scope frees is hoisted to a `__ret_N` temp so the frees run
between the read and the `Return` (`scopes.rs`, the B5-L3 rule). That rule named
only the SCALAR return types and a later branch only text, so a `vector` / record
/ struct-enum tail fell through to a fabricated `Return(Null)` with the
expression left as a discarded statement. The interpreter read the value off
eval-stack top; native emitted `let _ = expr; …; return DbRef::NULL`.

The hoist now covers the heap return types, bounded to a PLACE READ
(`Value::is_place_read` — a bare accessor chain over a variable, now the one home
for the question `Parser::is_addressable` already asked). The bound is
load-bearing in both directions: only a place leaves its value on the eval stack
alone, so only a place can be dropped; and `Set(tmp, place)` is a bare `DbRef`
copy, so the hoist adds no ownership. A CALL tail already delivers through its
hidden buffer, and hoisting one engaged the store-transfer machinery
(`protect_store_frees` + `CopyRefOrNull`) around a borrowed argument and
over-froze the caller's store.

### A vector's element WIDTH is part of its type at a call (loft#751) (2026-08-03)

`Type::is_equal`'s vector arm compared elements with the scalar-integer rule —
kind only, ignoring width — so a `vector<integer>` was accepted wherever a
`vector<u8>` was declared and its 8-byte elements were re-read as bytes. Silent:
the element COUNT is stored, so `len()` still agreed and every length-based check
passed while only the bytes were wrong. The same mismatch written as a literal
was already refused, so the two spellings of one mistake disagreed.

In a register an `integer` and a `u8` are one type; in a vector the element width
IS the stride. `Type::same_element_storage` states that, keyed on the canonical
`IntegerSpec::byte_width` (so `integer(0,100)` and `u8` are correctly the same
layout) plus the sign of the lower bound (so `i8` and `u8` are not). Refuses at
the call, the assignment, the field init and the return — all four went through
`is_equal`. The suite needed no change, so nothing in tree relied on it.

### Compilation is reproducible again (loft#750) (2026-08-03)

`store_confinement` answered a `HashMap`, and its caller relocates each confined
`__vdb`'s null-init; a relocation that cannot reach its block puts the init back
at body position 0, so visiting several confined stores in Rust's per-process
hash order PERMUTED the null-inits at the head of the body and moved the slots
under them. Compiling one file twice with one binary produced different bytecode
and different slots. It is a `BTreeMap` now, so the visit order is declaration
order. Over the 645-file script corpus, self-differing files: 3 → 0.

Program output was never affected. The cost was that a `--native` artifact could
not be bit-reproducible (#711), and that "prove this change emits byte-identical
IR" — the standing gate for every inert-first plan step — could not tell "my
change did nothing" from "the hash seed moved".

### The whole File surface works through a `&File` parameter (loft#753) (2026-08-03)

`&File` was accepted and then nothing you can do with a `File` worked through it:
`f#read` reported "Unknown loop attribute '#f'", `f += v` reported "No matching
operator 'Add' on '&File' and '&File'". A `&File` is `RefVar(Reference(File))` and
`is_file_var` / `is_file_var_type` both matched a bare `Reference(File)`, so every
File path fell through to the generic operator / attribute code and reported what
THAT code saw. Codegen had always dereferenced such a slot (`OpVarRef` +
`OpGetStackRef`). The peel now lives in the one predicate both callers share — the
loft#740 shape, two guards deciding one question with one of them peeling.

### Releasing a bound store hands its file tail back (loft#752) (2026-08-03)

loft#710 decided a persisted store's size must follow its content and fixed the
IMAGE-write path. A store bound with `store_persist_bind` FIRST never goes through
it: its file IS the live arena, which grows by 7/3 and never shrinks by itself, so
the file left behind was a rung on a ladder — up to 57% above its content, with
40 000 and 60 000 features writing a byte-identical file.

`free_named` now calls `reclaim_tail` on a file-backed store before marking the
slot free. That placement is the fix: `usage()` early-returns on a freed store, so
after the flag the chain walk reports mark 0 and `shrink_to` (rightly) refuses.
It does not contradict @PLN123 A3's "the program says when" — that rule is about
the middle of a run, where a reclaim is paid back at 7/3 by the next claim; at
release there is no next claim, which is the one moment the runtime can tell a
permanent drop from a lull. `shrink_to` still declines a read-only store, an
incomplete walk, and a durable `.dmeta` sidecar.

### A bytecode dump survives a partial schema (2026-08-03)

`introspect` aborted mid-dump on a file that failed to compile: a position still
carried a type id the (partial) schema did not have, and the dump indexed the
type table raw. It prints the bare id now — a dump must never panic.

### @PLN124: a format expression hands its parts to the type being built (2026-08-03)

`parse_string` gains a target: when the type a format string is assigned to
defines `lit`, the string lowers to `lit` / `hole_<kind>` method calls on an
accumulator of that type instead of appending into a text work buffer. The
literal/hole boundary already existed at parse time and was erased only because
every branch appended into the same buffer.

`Parser::interpolation_target` is a fifth SHAPE read off the one `⇐` expected-type
channel, beside `lambda_hint` / `enum_hint` / `vector_hint` / `read_target_type` —
not a sixth side-channel. `var_tp` supplies the target for a typed local, a typed
reassignment and a field init; `expected` for a call argument (free function and
method) and a return body.

Three constraints the build settled:

- **The mint must be pass-stable.** Taking the branch mints an accumulator, and
  the variable tables persist across passes BY NAME (loft#662), so the branch
  keys on method defs (collected on both passes, as the `to_text` hook already
  relies on) and the accumulator draws from its own `__fmt_N` counter
  (`Function::work_format`) rather than sharing `__work_N` or `__ref_N`.
- **The expected-type channel leaked across a nested call.** Each hint SET it when
  it applied and none CLEARED it when it did not, so in `take(build_one("arg"))`
  the literal — `build_one`'s `text` parameter — was checked against `take`'s
  parameter type. Latent while only the enum / collection / function shapes read
  the channel; immediate once a `text` parameter could be shadowed by a struct
  target. Now cleared per argument at both call sites.
- **Nullability is not a kind.** `format_hole` peels `Optional`, so a `text?` hole
  is a text hole whose value may be absent and the target's own parameter type
  decides whether it takes one.

An unsupported hole kind and a format spec on a hole are both compile errors —
never a silent fall back to rendering, which would put a value back on the text
path.

Inertness is the gate:
`doc/claude/plans/124-interpolation-hook/bytecode-comparisons/format-corpus.loft`
covers 104 format sites (specs, `text?` holes, the `OpTagFault` path, an inner
fault that must not tag, JSON/pretty, a custom `to_text` spec, three `for` forms,
backtick, escaped braces, `+=`, argument position) and `loft introspect`
before/after is byte-identical.

### Format hook: a boolean cannot carry a count of hidden work buffers (2026-08-03)

`to_text(self, spec)` with a conditional early `return` was an internal compiler
error — *"Too few parameters on t_5Money_to_text (got 3, need 4)"* — on both
backends, for every interpolation of the type, and on the released binary.

Each formatted `return` promotes its own hidden text work buffer, so two of them
make the hook `(self, spec, __work_1, __work_2)`.  `try_bound_to_text_call`
recorded the buffers in a BOOLEAN, which cannot carry "two": it appended one work
argument and `generate_call`'s arity assert fired.

#533 hardened this same site by classifying parameters by TYPE rather than by
count — correct, and still not enough, because it left the COUNT unrepresented. A
one-buffer body worked, and that working member hid the omission.

The fix fills one argument per attribute, walking the definition's own order, and
refuses to emit a call whose argument list does not fit the definition — so a
signature this hook cannot spell falls back to the generic field dump, which is a
defined answer where a short call is a crash. Guard:
`tests/scripts/format-hook-early-return.loft`.

### `LOFT_UAF_GEN`: a stamp keyed by offset cannot say which store it is about (2026-08-01)

Detector (c) reported a use-after-free on 25 of the 548 corpus scripts, all of them
clean. Any loop calling a struct-returning function drew one — an ordinary shape, so
the noise landed on exactly the programs the detector exists to clear.

The shadow stamped each pushed DbRef's slot generation, keyed by eval-stack byte
offset. `put_stack` is the only writer that keeps that shadow in step, and it is not
the only writer OF the eval stack: `copy_result` slides a return value down with a raw
`copy_block`. The destination offset kept whatever stamp its previous occupant left,
and the next pop compared a returned DbRef against a generation belonging to some
earlier value. A trace showed a push of store 2 at offset 648 answered by a pop of
store 3 at the same offset, checked against store 2's stamp.

Two changes, because either alone leaves the class open. `copy_result` moves the stamp
with the bytes — the source stamp is the real one, written by the callee's `put_stack`
— which covers the SAME-store case no identity check can reach (the slot is recycled
between iterations, so the stale stamp and the fresh ref name the same store at the
same rec and pos). And the shadow now carries the STORE alongside the gen, so a
leftover from any bypassing writer, found or not, is inert rather than evidence.

Ground truth for calling the reports false: `LOFT_NO_SLOT_REUSE=1` with
`LOFT_POISON=1`. With no slot reuse a genuine stale read must land on poisoned bytes,
and every reporting script stayed clean and correct.

`LOFT_UAF_GEN_INJECT=1` is the other half. Silencing a detector and fixing one are
indistinguishable from a test that asserts only "no reports", so the injection ages
every ref just after its push stamps it: 471 of 548 scripts report under it against 0
without. `tests/uaf_gen_detector.rs` pins both directions. Recorded because it changes
what the tool's silence means — and note the detector never actually caught #723: its
report on the broken binary was this same false positive. Detector (c) sees only the
window between a push and its pop, and #723's genuine stale read is a ref going stale
in a FRAME slot.

### #722 / #723: an ownership proxy and the carried fact disagreed at the pre-Set free (2026-08-01)

`x = f().items[0] ?? Fallback {}` bound a dangling reference into the temporary `f()`
returned — correct on the first read, zeroes once the store was reused. The `??` is the
whole difference: it lowers to a value-producing BLOCK, and the temp holding `f()`'s
result was registered at that block's scope and freed on the way out while `x`, bound
in the enclosing scope, still pointed into it. A lift temp minted inside a value block
is now re-registered at the ENCLOSING scope with its block-exit free dropped, and only
the temps the RESULT points into move — hoisting every lift in a value block instead
made frees go missing (27 leaked `File` stores), so `borrow_root` walks the getter
chain to decide.

The loop form was a second fault with a different mechanism. `generate_set`'s
`owned_ref` decides whether a re-assignment must free the store it drops, and asks the
TYPE: empty deps means owned, by loft's convention. That is a PROXY, and it reads
"owned" for a borrow whose dep list was never populated — exactly what a `??` subject
of Reference type is, since the parser materialises it into `__ncc_N` and marks it
`skip_free` while leaving deps empty. Outside a loop the variable is assigned once, so
the free ran on a null slot; inside one it ran on the previous iteration's borrow, by
then dangling into a store freed at the body exit whose slot the next call had already
recycled — so it released a store that had just been allocated and was about to be
read. The @PLN118 sentinel reset cannot cover it: that hangs off a block-exit free,
which a `skip_free` var by definition does not have.

The fix is a subtraction — `skip_free` vetoes the proxy — and not a new rule:
`generate_call` already suppresses any IR-level `OpFreeRef` naming a `skip_free` var
(the S34 guard). `generate_set` emits its pre-Set free as raw ops and bypassed that
chokepoint. A sentinel over the remaining raw emission sites found 0 further `skip_free`
frees corpus-wide against a proven-live control of 1831 hits, so the class is closed.
Interpreter-only: the generated Rust is byte-identical, and `--native` derives the
borrow correctly in `generation/dispatch.rs`.

### #687: a mutated text capture's STORAGE is decided per binding, not per function (2026-07-30)

#685 fixed mutated scalar PARAMETER captures for every type but one and refused the
remainder by name: a `text` parameter inside a function that itself returns `text`. Plan-22
phase 02d-vii skipped text boxing whenever the parent returned text, so mutable text
travelled as a hidden `&text` out-parameter that pass 2 cannot add without growing the
signature (the H5 two-pass contract catches it).

**`parent_returns_text` was a proxy for one real case, and wrong in both directions.** The
case it protected is a text local that is the function's RETURN SOURCE, which the return
machinery has already given its own hidden `&text` out-parameter — that binding cannot
*also* be a shared cell, and does not need to be, since the record stores the value inline
and the existing per-call write-back propagates the closure's writes. As a proxy it was too
WIDE (it also skipped a text local the function does not return, which boxes cleanly) and
no help at all for a PARAMETER, which has no indirection of its own to reuse. A boundary
sweep separated all four combinations, and the discriminator turned out to be a fact
already used elsewhere in the same function: **`RefVar` means "this binding already has an
indirection"**.

The two halves that must agree are the record's attribute type
(`box_captured_names_for_outer_scalars`, at the LAMBDA's epilogue) and the binding's own
type (`flip_scalars_to_box_types`, pass 2) — and measurement showed the epilogue simply
cannot be right: a to-be-returned text local is still a plain `Text` there and only becomes
`RefVar(Text)` later in the body, which is exactly why the two disagreed. So the epilogue
boxes PROVISIONALLY (the common case, and it must write something because pass 1 freezes
the record's storage — leaving the raw scalar lays the field out 8B inline instead of a 12B
shared DbRef), and `Parser::finalize_capture_storage` corrects it at the parent's **pass-1
body end**: the first moment the fact is final, still before `fill_all` lays the record out,
and the same hook `reject_shared_mutable_scalar_captures` already uses for the same reason.
It runs first — the rejection consumes the parent's lambda list.

Net effect is a SUBTRACTION: both `parent_returns_text` guards and #685's refusal are gone,
replaced by asking the binding. One text-returning function can now need both answers at
once (`keep` returned → inline, `side` not → cell), which is what no per-function condition
could ever get right.

Two measurements worth recording, because both contradicted the obvious reading:

- Removing only ONE of the two guards leaves them disagreeing and SIGSEGVs — they are one
  fact re-derived twice, so they move together.
- `box_captured_names_for_outer_scalars` and `synthesize_closure_record` run in **pass 1
  only**: in pass 2 the lambda records zero captures (its pass-1 placeholder vars are
  restored into its own table, so the capture branch is never reached). The record's
  attribute types are a pass-1 decision, full stop — which is why the correction has to be
  a pass-1 hook and not a pass-2 repair like #686's.

Guards: `tests/scripts/687-mutated-text-param-capture.loft` (values, both backends — the
capture's source, whether it is the returned value, the parent's return type, cardinality,
by-value, repeat calls) and `issue_687_mutated_text_capture_storage_is_per_binding`, which
asserts the STORAGE for three bindings whose only difference is what claims them. Both
verified RED with `finalize_capture_storage` disabled. #685's
`issue_685_text_param_in_text_returning_fn_is_refused` is replaced by it — that test pinned
the refusal, which was always a placeholder for this issue.

### #686: a capture of a FORWARD-declared type was mis-typed, then mis-positioned (2026-07-30)

A lambda capturing a local whose type came from a struct declared LATER in the file read
that capture as `text` — `Unknown field text.cells`, on a program with no `text` in it.
Two faults composed, and the first hid the second.

**Fault 1 — a sentinel read as a def number.** A capture's type is the type of an
EXPRESSION (`ch = w.chunks[1]`), so with the struct not yet declared it is `Unknown(0)`:
the codebase-wide "no type known" marker, which names nothing. `copy_unknown_fields` read
the `0` as a forward-reference stub and set the field to `data.def(0).returned` — whatever
the first definition in the program happens to return, in practice `text`. The `Vector`
arm of that same function already guarded `was != 0`; the bare arm did not. This is a
LYING fact, not a missing one: the field looked resolved, so nothing downstream
questioned it. Guarded now, which turns the symptom honest (`unknown`, not `text`) and is
what exposed the second fault.

**Fault 2 — a struct laid out while a field was unsized.** `fill_database`'s field loop
SKIPS an attribute whose type it cannot size (deliberately — so the user sees the parser's
diagnostic rather than a panic), but the struct is still registered and `finish` still
sizes it, leaving the field at `position == u16::MAX` forever: `finish_type` never
revisits a sized type. The closure then read and wrote its capture at **offset 65535** —
an INTERMITTENT crash, which is what made the readings during investigation worthless
until a repeat-run harness replaced them (two byte-identical probe files disagreed;
single runs had been "confirming" three different stories).

The invariant: **a struct is laid out only once its fields are sized.** Enforced at the
one place that lays them out — `fill_all` skips a def carrying a NAMELESS unknown
attribute (`has_nameless_unknown_attr`). Narrow by construction: `Unknown(stub)` names a
type and `copy_unknown_fields` resolves it before the loop, so only `Unknown(0)` — an
expression-typed field, and the closure record is the sole producer of those — can reach
the layout unsized. The loop is keyed on `known_type == u16::MAX`, so deferring costs
nothing; a field that never resolves leaves the struct unregistered, which is harmless
because the parser has already reported the error.

Pass 2 then re-types the attribute from `capture_context` (`resolve_forward_captures`, at
lambda entry — NOT at the record-synthesis epilogue, which runs after the body) and lays
that one record out on demand via `Stores::lay_out_record`. The on-demand layout is
required, not a shortcut: the body bakes field offsets into its IR as it parses, so the
end-of-pass `finish()` is too late, and a full `finish()` mid-parse re-appends keyed-index
bookkeeping. `lay_out_record` is the sibling of `lay_out_synth`, which solved exactly this
for a forward-referenced synth enum — same deferral, same reason, same empty `linked` set
(a closure record holds scalars and 12-byte DbRefs, never an inline keyed collection).

The pass-1 storage-encoding `match` moved into `closure_attr_type` so the synthesis and
the repair cannot drift on which captures store as a shared DbRef.

Guards: `tests/scripts/686-forward-declared-capture.loft` (values, both backends — field /
element / whole-vector / scalar projections, cardinality, and repeat calls) plus
`issue_686_forward_declared_capture_is_typed_and_positioned` and
`issue_686_nameless_unknown_is_not_resolved_against_def_zero`. The two facts are asserted
separately because they fail separately, and each half of the fix was verified to break
its own: with the sentinel guard off both fail; with only the layout deferral off, the
type is right and the POSITION is still 65535. A value test alone cannot see that — a
positionless field only crashes when the bytes at offset 65535 happen to be fatal.

### #685: a mutated scalar capture sourced from a PARAMETER corrupted the frame (2026-07-30)

Two producers of one fact disagreed. `box_captured_names_for_outer_scalars` gave the
closure record a 12-byte `__cell_<T>` `DbRef` field for a mutated scalar capture, while
`flip_scalars_to_box_types` skipped arguments outright — so the parameter stayed an
8-byte stack scalar and `emit_lambda_code`'s `OpSetDbRef` read 12 bytes out of an
8-byte slot, corrupting the 16-byte fn-ref being built beside it. The interpreter then
dispatched a garbage `d_nr` (`fn_call_ref: … out of range`) or SIGSEGV'd; `--native`
emitted field access on a bare `i64` and would not compile.

**The filed scope was one cell of each axis; the boundary is a single fact.** The
trigger is only "the mutated capture's source is a parameter": every boxable type
fails (integer / float / boolean / character, and text — the last as a SIGSEGV, via a
different lowering), the closure need not be CALLED, the enclosing function need not
read the value back, the closure may sit in a nested block, and a set before the
lambda does not help. **That last cell falsified the filed hypothesis** ("the cell is
never allocated because allocation happens on first set"), which is why the fix does
not touch allocation timing. Two or more captures in one frame crash at a *different*
site (`allocation.rs`) — the same corruption seen through slot reuse.

The argument-skip could not simply be dropped: flipping a parameter's own type to a
12-byte cell reference changes the call ABI. The fix promotes it instead —
`promote_boxed_scalar_arg` mints a shadow local of the same type, `set_promoted_from`
+ `remap_name` point the name at it before the body parses, and the existing
promoted-argument preamble in `parse_code` seeds it at function entry. Every read,
write and capture then routes through the shadow and the emitted IR is byte-identical
to the LOCAL case that already worked — which is why all five types are covered with
no per-type work. It is the hand-written workaround (`acc = n;`) done by the compiler,
and it follows the mutated-text-argument promotion the codebase already had.

Reusing the local path exactly is also what preserves by-value semantics: the
parameter slot is untouched, so the caller cannot see the closure's writes.

Supporting changes:

- `boxed_cell_alloc_and_set` extracted as the ONE home for "a boxed scalar comes into
  existence" — the first assignment to a boxed local and the parameter's entry seed
  need the identical `Set(v,Null)` + `OpDatabase` + `OpSet<T>` trio, and the seed has
  no assignment of its own to hang it on. The shadow is marked `defined` at creation
  so the body's first write does not prepend a SECOND allocation, which would replace
  the seeded cell and lose the argument's value.
- `Type::Boolean => "OpSetBoolean"` added to the cell-write table. It had been
  deferred on the premise that a boolean cell needs a 4-arg `OpSetByte`; the premise
  was wrong — the working boxed-boolean lowering emits the 3-arg `OpSetBoolean`. The
  unit test that pinned the fall-through now pins the write.
- A value-const parameter mutated through a closure is now rejected at the promotion
  site. The closure-side write never reaches `validate_write`'s guard (inside the
  lambda the name is a capture, not a binding carrying the flag), so without this the
  fix would have quietly handed the closure a writable cell for a read-only parameter
  — a silently accepted contract violation in place of a loud crash.
- `RefVar` arguments are excluded from both branches: a user `&T` out-parameter's
  writes must reach the caller, and a mutable text local the compiler already promoted
  to a hidden `&text` out-parameter is itself the working path. The first attempt
  omitted this and refused `local = n; … local = local + k;` — code that worked.

**Residual, refused by name rather than corrupted (#687):** a mutated `text` parameter
inside a text-returning function. There `flip_scalars_to_box_types` skips text boxing
(plan-22 02d-vii) and mutable text travels as a hidden `&text` out-parameter instead,
which cannot be added from pass 2 without growing the signature after pass 1 fixed it.
The **H5 two-pass contract caught that attempt** — the assert doing exactly the job
#662 showed it had been blind to. The diagnostic names the working alternative.

Guards, all four verified RED with the promotion disabled:
`tests/scripts/685-mutated-scalar-param-capture.loft` (values, both backends — every
type, both non-trigger axes, cardinality, and the by-value edge) plus
`issue_685_mutated_scalar_param_is_boxed_like_a_local` (the invariant: the record's
field type and the frame's binding for that name are the same cell, and the arity is
unchanged), `issue_685_text_param_in_text_returning_fn_is_refused`, and
`issue_685_const_scalar_param_mutated_by_closure_is_rejected`.

### #682: the closure-record cascade freed captures the record never owned (2026-07-30)

A reference / collection capture is stored in `__closure_N` as a 12-byte `DbRef`
(P260), and `free_named`'s cascade freed every one of them when the record died.
That is correct for a store the defining frame OWNED and handed over — `get_free_vars`
suppresses the frame's own `OpFreeRef` for a captured reference, and the cascade being
the sole free is exactly what lets an escaping factory closure outlive its frame
(#323). The pairing only holds where a frame free existed to suppress, and for two
common capture sources it never did: a **parameter** is excluded from the scope-exit
sweep entirely (`variables()`: "never return function arguments"), and a **projection
local** (`ch = w.chunks[1]`, a `for` element) is `owns == false`. Both were cascaded
anyway, so the caller's store was freed under it.

**The filed scope was a lambda handed to a library as `fn(float,float)->float`;
neither the hand-off nor the call is a trigger.** The minimal cell captures a struct
parameter and never invokes the lambda. The axes that matter are the ones deciding who
owns the capture — capture SOURCE (parameter / projection / for-element / owned local)
and KIND (struct reference / vector / hash / boxed `__cell_`) — and the class covers
every store-backed capture of a borrowed binding, not just the reported struct.
The symptom was three steps removed: a freed-but-unreused store still reads correctly,
so the fault surfaced when the next allocation recycled the slot, in an unrelated
function ~900 lines from the closure.

One dep marker had to carry two facts, which is the encoding bug behind it:
`Deps::share_sentinel()` meant both "store a 12-byte DbRef" and "the record owns the
target". It is now a pair — `share_sentinel()` (adopted, `dbref`) and
`borrowed_share_sentinel()` (borrowed, `dbref_borrow`) — two type-table entries of the
same 12-byte / align-4 shape, so no position, size, read or write path moves; only
`free_named`'s filter (`Stores::dbref_is_adopted`) reads the difference.

**The verdict cannot be computed at record synthesis, which is why it is not.** A
capture's ownership is not final at parse time: `ch = pick(w, 1)` parses as "borrows
`w`" from the callee's declared return, and only `scopes::check`'s call-result rewrite
(`make_independent`, the `!adopts_fresh_store` arm) turns it into OWNED once it knows
the return ABI deep-copies into a fresh store. A first attempt read the parse-time dep
and leaked that copy. The decision is therefore `scopes::mark_borrowed_captures`, run
after every dep rewrite has settled, reusing `get_free_vars`' own `owns` test so the
two cannot drift. It reaches each record through the defining frame's `___clos_N`
LOCAL; the lambda's hidden `__closure` PARAMETER has the same type, and reading it too
flipped verdicts by definition order.

`--native` picks the marker up from the attribute type directly (its schema is emitted
after scope analysis); the interpreter's schema is laid out during parse, so
`typedef::sync_capture_ownership` re-points the field from `compile::byte_code_from` —
the one funnel every `byte_code*` entry point passes through. A `__cell_<T>` capture
(plan-22's boxed mutated scalar / text) is always adopted: the cell is minted for that
closure alone, so the record is its only possible owner however the binding was
reached — including from a parameter.

Both `dbref` shapes register **together** from either entry point. Type numbers are
positional and `--native` replays the registration sequence to rebuild its schema, so
a shape appearing in only some programs shifted every id after it — `505-collection-
capture` failed native with `Cannot add to none-structure 'State'` until the pair
became unconditional.

Guards: `tests/scripts/682-closure-capture-borrow.loft` (values, both backends — every
borrow cell called twice so the recycled slot shows, every adopt cell three times so a
wrongly-borrowed verdict shows up as a leak) and
`tests/issues.rs::issue_682_closure_capture_ownership_marker` (the marker itself, both
directions). Both verified to go RED with the pass disabled. Reproduced and fixed
against the consumer's real `hex_world::World`, which the pre-fix binary corrupted from
the second tick on.

### #654: jump displacements were 16-bit — a body past 32 KB jumped somewhere arbitrary (2026-07-28)

`OpGotoWord` / `OpGotoFalseWord` carried a `const i16` displacement, computed with an
unchecked `as i16` at every emission site. Past ~32 KB of emitted body the value wrapped
and the jump landed at an arbitrary address; for a `while true` that meant the body ran
ONCE and control fell out of the loop, with `main` returning 0 and no diagnostic.

**The filed scope was the backward `while` jump; a boundary matrix showed the real one.**
All five jump classes truncate, because they share the encoding: `while` and `for`
(backward, `gen_loop` / `gen_continue`), `break` (forward, patched in `Stack::end_loop`),
and the forward skips of `if` and `else` (`gen_if`). `--native` was correct in every cell
— it emits real Rust control flow and never reads these operands — which made it the
positive control the matrix was read against.

Both ops now carry `const i32`, which covers the whole `code_pos` (`u32`) space, so the
threshold is removed rather than moved. A fixed-width slot also means the forward-jump
patch sites need no branch relaxation: `code_put` writes an `i32` into a slot whose size
was already reserved.

Getting a 4-byte constant emitted required two places to stop deriving an integer's width
from its RANGE and start reading its declared `size(N)`:

- `variables::size(_, Context::Constant)` — a 1 / 2 / 8 ladder with no rung for 4.
- `Data::rust_type` — the same ladder, deciding what the generated reader in `fill.rs`
  reads. Left alone it would have READ an `i64` while codegen WROTE 4 bytes.

Both are inert for every integer alias that predates the change (`u8` / `i8` force 1 and
range to 1; `u16` / `i16` force 2 and range to 2; plain `integer` forces nothing), proven
by a byte-identical `loft introspect` over a corpus exercising all of them before and
after the `variables::size` change alone.

Displacement arithmetic that measured from after-the-operand moved from `- 2` to `- 4`
(`gen_loop`, `gen_continue`, `Stack::end_loop`); the disassembler's jump-target scan
(`compile.rs`) and both renderers in `state/debug.rs` follow the same width. `tests/dumps`
is gitignored, so no golden output churned.

Guard: `tests/issues.rs::issue_654_jumps_survive_a_body_past_the_16_bit_displacement`, one
case per jump class, each asserting an accumulated VALUE rather than mere completion —
the failure mode was silent fall-through, which a runs-to-completion check would have
called a pass. Verified non-vacuous against the installed pre-change 2026.7.2 binary: all
five cases produce NO output there (execution falls past the asserts) and pass here.


### @PLN108 "Share read-only parent stores across par workers" — interpreter core (2026-07-17)

- A par worker whose captured parent state is read-only (@PLN102 C93) now **BORROWS** the parent
  stores read-only instead of `clone_for_worker`'s per-worker byte-copy, for `run_parallel_discard`
  and `run_parallel_queue`. Copy-elision, no semantic change.
- **Auto-selected by heap size:** the borrow engages only when the copy it would save is ≥ 2 MB
  (`Stores::active_clone_bytes`), so small/frequent pars keep the cheap rayon-pool clone (no
  regression) while a par over a large read-only structure goes flat instead of copying the session
  heap per worker (measured ~53× on a 122 MB shape). `LOFT_PAR_SHARE=0`/`=1` force off/on.
- Safety is compiler-carried (the dispatcher's `&Stores` signature proves parent-unwritten) +
  the `read_only` write-panic; **ASan + TSan clean** on the flag-ON path (positive control fires).
  `--native` par still copies (a native analogue is deferred). Decision recorded as C99.

### `loft fmt` — parser-driven formatter, written in loft

- New `loft fmt [--check|--write] <file…>` (`-` = stdin): a canonical formatter (`tools/fmt/whole.loft`)
  driven through the new host-call API. Default prints; `--write` rewrites in place; `--check` is a
  CI gate (non-zero if unformatted). One canonical style — struct/enum/interface defs + fn bodies
  expand; struct-literal/control-flow VALUES stay inline; number vectors pack, object vectors break;
  trailing comments stay at end of line. Coexists with the older Rust `--format`.

### `loft::host` — Rust→loft call API

- `Program::from_source(src)` → `prog.call("fn", &[Value::…])` → `Result<Value, LoftError>`: call any
  loft `pub fn` by name with typed args, typed return, errors as a `Result`. Routes through the
  interpreter's stack ABI (`State::execute_host`). Phase 1 supports text / integer / single / boolean
  / void; struct/vector returns are a clean `Unsupported`. Consumed by `loft fmt`.

### @PLN28 "Better error messages" — closeout (2026-07-07)

Phases 5, 6, 1, and 7 landed, completing the plan (0/2/3/4 shipped earlier).
Commits `a77afaec` (P5+P6), `6e9b6c5f` (P1 resolution), `<this>` (P7).

- **P5 suggestions** (`src/diagnostics.rs`, `parser/objects.rs`): all seven
  candidate-scoped `did-you-mean` sites live. Relaxed `suggest_similar_capped`
  from `min(2, n/4)` to distance-2 for names ≥ 4 chars (catches transpositions
  like `naem`→`name`); wired the struct-literal unknown-type site; a qualified
  `Enum::Typo` now reports + suggests instead of silently nulling (exit 0 → 1).
- **P6 type-mismatch** (`parser/mod.rs`, `parser/control.rs`, `parser/objects.rs`):
  call-arg mismatch names the argument index; a `match` pattern whose type can
  never match the subject now errors instead of compiling to a silent dead arm;
  struct extra-field recovers past the orphaned value (6-error cascade → 1).
  The spec's phrasing-only rewrites were skipped (messages already name both
  sides + the operation); missing-field and format-spec-on-wrong-type left as
  designed behaviour (zero-default fields / freeform specs).
- **P1 spans**: verified the 5 "remaining" wraps (Set / Iter / Return / struct-
  lit / narrow-cast) unnecessary — their diagnostics already capture positions
  via `diagnostic_at!` and none faults at runtime, so wrapping would attach a
  position no consumer reads while risking `unspan()` churn. Status → done.
- **P7 closeout**: COMPILER.md § Diagnostics rewritten (spans → runtime C66 →
  renderers); user-facing CHANGELOG entry; CLAUDE.md `LOFT_ERRORS` + diagnostic
  toggles; CAVEATS native-no-source-map entry. No opcode changes; no runtime
  path touched (bench flat).

Golden `error_messages` baselines 06-10, 30, 34 regenerated + locked; full suite
green on both backends. Deferred (non-blocking polish, tracked in the phase docs):
phase-4 `4e.3 slice 2` (finer format-null tokens) and the `= note:` secondary-line
renderer.

### #497: reassignment-path adopt-vs-copy — a borrowed call return freed the lender's store (2026-07-04)

A struct-returning call REASSIGNED into an owned Reference local took the
plain-adopt path whenever the callee had no visible Reference/struct-Enum
param — the old `has_ref_params`-style proxy missed a callee borrowing from a
visible VECTOR param (`return cs[i]`). The local then aliased the borrowed
element, and its owned pre-Set free whole-store-freed the LENDER's backing
store: crawler's `build_walls` lost `cs` mid-function — silent wrong data
first (writebacks vanished; the #496 face), SIGSEGV once store recycling
reused the number (the #497 face; the scale-dependence and heisenbug were
pure visibility artifacts — `LOFT_LOG=poison_free` reproduces it in the small
hand-built level deterministically). The one-axis trigger: the call-assignment
sitting inside a nested `if` (its first-set is the hoisted init, so the Set is
a REASSIGNMENT; the first-Set path already read the carried fact and was
correct).

- Fix at the A.3 chokepoint (OWNERSHIP_MODEL row 102/270): the reassignment
  gate in `state/codegen.rs` now reads the ONE carried adopt-vs-copy fact,
  `return_adopts_fresh_store()`, exactly like the first-Set path — and gains
  the #429 struct-Enum parity the first-Set path already had.
- The raw path is preserved behind `LOFT_NO_REASSIGN_COPY` (the
  `LOFT_NO_JOIN_OWN` preservation pattern) so the ownership fuzz gate's
  crash-channel positive control stays non-vacuous; the self-test's buggy
  config now disables both gates (control re-pinned).
- Guards: `tests/scripts/497-reassign-borrowed-elem-copy.loft` (the minimal
  if-wrapped shape + the two-captures build_walls composition, both backends);
  the 54-cell fuzz map is 0/54 flagged on the default gate.

### Nightly registry validation — published packages vs loft@main (2026-07-04)

New `registry-validation.yml` workflow (04:30 UTC + `workflow_dispatch`): one
matrix leg per non-yanked registry package, each installed from the LIVE
registry exactly as a user gets it and validated against loft built from main
on the runner's current stable rustc — `loft install` (tarball + sha256 +
deps), `cargo build` of the shipped `native/` crate, and the package's own
tests on both backends via the new `scripts/registry_validate.sh` (also
runnable locally). Closes the gap where a released tarball rots unnoticed
after loft moves (the loft-libs-core#14 class); the first live sample caught
cbor 0.1.0 (DN1 type error) and crypto 0.3.4 (machine-local `path =` deps in
the published `native/Cargo.toml`). See PKG_REGISTRY.md § Nightly toolchain
validation.

### `#native` boundary: nullable scalars marshal, C-ABI externs are i64 (2026-07-04)

Found via loft-libs-core#14 (`random.rand` declaring the honest `-> integer?`
contract under the @PLN25 null/dense model). Two related fixes, one invariant:
*marshal/ABI classification is layout-based, and `Optional(τ)` shares τ's
sentinel layout — every judgment classifies the peeled type* (`Type::base()`).

- **`Optional(τ)` in `#native` signatures now wires and dispatches.** The
  marshal classifiers (`extensions::compute_sig` / `compute_shared_sig` /
  `marshal_arg_t`, `native_gate::is_scalar_type` / `is_bridge_type` /
  `classify_bridge_attr` / `shared_store_dispatchable`, the shared-bridge
  read/write emitters, and the `--native` direct-call + extern emitters) all
  fell through to their unmarshallable/default arms on `Optional`, so a
  `#native` fn declared `-> integer?` was never wired — the interpreter call
  hit the stale-cdylib panic stub, and `--native` mis-emitted. All sites now
  peel via `Type::base()`, extending the @PLN25 slice-(b) pattern already used
  by `rust_type`.
- **The C-ABI extern block now declares i64 for plain `integer`.** The emitter
  decided width by `IntegerSpec::is_wide()` (value range — false for a
  declaration's template spec) while the interpreter marshal and the package
  cdylibs use the @P370 judgment (plain `integer` = i64; only `forced_size`
  narrows). The `i32` extern against an i64 cdylib silently truncated i64
  traffic — the null sentinel (`i64::MIN`) arrived as `0`, and beyond-i32 /
  negative values corrupted. Both judgments now key on `forced_size`.
- Regression guards: `native_loader::wires_optional_integer_return_and_wide_values`
  (end-to-end null + 2^40 round-trip through the fixture cdylib) and
  `n2_cdylib::cabi_extern_declares_i64_for_plain_and_optional_integers`
  (emit-shape); fixture patterns 10/11 (`ext_maybe`, `ext_echo`).

### @PLN22 — enum-scoped variants, prelude shadowing, `use … as …` aliasing (2026-06-14)

All four phases of the namespaces initiative (`loft-lang/plans#22`), built
chokepoint-first and verified on both backends.

- **P1 — enum-scoped variant definitions.** Variants are resolved through one
  `Data::variant_of(enum, name)` chokepoint (plus `variant_in_source` /
  `enums_with_variant`) instead of a global bare key, so two enums may share a
  variant name. A bare variant used as a *value* resolves only via context
  (match subject, typed decl, typed reassignment / `rec.field`, parameter,
  return incl. an `if`-branch tail, struct-field type & default, `==` LHS,
  `Enum::`/`Enum.` qualifier); defining a new untyped variable from a bare
  variant (`x = Red`) is a hard error even when the name is unique. Mixed-enum
  unit-variant field defaults no longer clobber a sibling field. The variant
  name stays usable as a TYPE / constructor (`Circle { … }`, `s: Circle`).
- **P2 — prelude shadowing.** `STD_SOURCE = 0` (stdlib + global synthetic
  wrappers) and `MAIN_SOURCE = 1` (user main) are named explicitly; the user
  main file gets its own source so a user `enum E` / `struct File` / `pub PI`
  shadows the stdlib name in bare lookup while `std::Name` still reaches the
  prelude. Built-in type-keywords (`integer`, `vector`, …) stay sacred —
  non-shadowable — via the `DefType::Type @ STD_SOURCE` guard.
- **P3 — `use … as …` aliasing** for libraries (`use lib as m`), types
  (`use lib::Type as T`), and functions (`use lib::fn as f`).
- **P4 — grouped selective imports** `use lib::(a as x, b, c)`; the flat comma
  list `use lib::a, b` is dropped (hard error). Design decision C76.
- **Reserved-keyword hardening (commit `c383a25c`).** `struct iterator` was
  silently adopted (the struct adopt-branch swallowed the `type iterator;`
  forward-decl); `enum hash` / `type sorted` panicked in `complete_definition`.
  Guarded the adopt branch on `DefType::Unknown`, gated the enum/typedef
  completion calls on `!conflict`, and forward-declared `type radix;` /
  `type spacial;`. All builtin type-keywords now reject cleanly across
  struct/enum/type. Regression: `tests/scripts/102-expected-errors.loft`.

Regressions: `tests/scripts/369-pln22-shared-enum-variants.loft`,
`370-pln22-prelude-shadowing.loft`, `tests/imports.rs` (phase 3/4 aliasing +
grouped + flat-list-rejected). Resolves INC#34.

### engine_host: `run_local` — the standalone windowed host (#343) (2026-06-12)

A windowed program with no server could not run on the @PLN18 kernel: `run`
(listener) never returns and has no frame yield; `run_client` bails without a
connection.  `run_local(tick_interval_us, on_event, on_tick)` is the connector
loop with **no transport** — drift-free ticks (one tick = one frame for a GL
host), the per-turn frame yield, the loop's own idle backoff (kills the
consumer's busy-spin), swap machinery (08-S5 `LOFT_SWAP_READY` included) and
the debug control endpoint, all without a peer.

Mechanics: `ClientKernel.conn` became `Option<TcpStream>` (the two `Some`
sites — frame read, masked write — are behavior-identical; `None` reads
nothing and `send` reports false).  `kernel_local(tick_interval_us)` landed on
all three calling conventions: bytecode native, browser (`ws:-1`, guarded
pump/send), and the `--native` typed twin (`CODEGEN_RUNTIME_FNS`).  The loft
side factors `run_client`'s body into one shared `client_loop`; `run_local` is
local boot + the same loop (no third copy).  Going online later means swapping
`run_local` for `run_client` — handlers never change.  Regression (both
backends): `tests/engine_host_kernel.rs::run_local_ticks_and_stops_without_a_server`.
Driven by the crawler consumer (#343); design note: plan-18 ENGINE_HOST.md
§ Update 2026-06-12.

### engine_host: `post`, listener `stop()`, listener frame yield (the crawler K2 trio) (2026-06-12)

The three flow-back asks from the crawler consumer's K2 (observer slice):

- **`post(msg) -> boolean`** — enqueue a LOCAL event on the running kernel
  (any role): window input becomes an ordinary events-class message with
  `cid: -1` (local origin).  The connector loop previously hardcoded `cid: 0`
  when constructing events (the server was the only source); the new
  `kernel_client_event_cid` accessor carries the real origin.  Registered on
  all three calling conventions; surface fns with a `#native "sym"` alias
  register their DEF name in `CODEGEN_RUNTIME_FNS` (`n_post`, like `n_send`).
- **Listener `stop()`** — `Kernel.alive` + `kernel_stop`/`kernel_alive`;
  `run` loops on `kernel_alive()` and returns after a handler calls
  `engine_host::stop()` (the windowed listener's window-close exit, mirror of
  `client_stop`).
- **`kernel_frame()`** — the per-turn yield in `run`'s loop (no-op native,
  frame-yield browser; twin of `kernel_client_frame`), so a windowed
  LISTENER frames correctly.

Regression (both roles × both backends):
`tests/engine_host_kernel.rs::post_and_stop_in_both_roles`.

### rpc debugger: `verified` flag on setBreakpoints + string-form tracepoint log (#342) (2026-06-12)

Two silent-failure footguns in `loft debug --rpc`, found while verifying the
loft-debug skill against the implementation:

- `setBreakpoints` answered `{ok:true}` with no per-breakpoint feedback, so a
  breakpoint that can never fire (no breakable code on the line, or a file the
  program doesn't use — matching is by **basename**) just never stopped.  The
  response now carries the PROTOCOL.md-documented `breakpoints:[{line,
  verified}]`, resolved eagerly via `breakable_lines_in_file`.
- A tracepoint's `"log"` given as a plain string was silently ignored (only
  the array form worked).  A string is now sugar for a one-element array;
  entries are expressions rendered `expr = value`.

PROTOCOL.md's request table is corrected to match the implementation: `launch`
LOADS only; the previously undocumented `run` request starts execution (set
breakpoints between them).  Liveness note for clients: conditions and trace
expressions see only the locals live ON that line — an out-of-scope name
evaluates null and a condition on it silently never matches.  Regressions:
`tests/rpc.rs::rpc_set_breakpoints_reports_verified`,
`rpc_tracepoint_log_accepts_plain_string`.

### Multiple materialised par loops no longer corrupt each other (#282) (2026-06-06)

Several **materialised** par loops (range / `iterator<T>` / text inputs) in one
function, with **different element types**, silently corrupted an earlier loop:
its materialised input (`__par_mat`) was read at the wrong stride (e.g. an
`integer` range loop's input came back as `vector<character>`), so a worker saw
garbage elements.

Root cause (var-table / scoping level, not IR-structure): `materialise_iter_for_par`
builds its body **pass-2-only**, so naming its temps via the global `create_unique`
counter advanced that counter only on pass 2 — desyncing two-pass numbering for
sibling materialise loops, whose `__par_mat` vars then **collided on one name**.
`add_variable` merges by name, so the merged var took one element type; the other
loop read its store at that type's stride. (Same family as the result-var
two-pass fix.) Keyed materialise was immune only because all its loops share one
element type.

Fix: name the materialise temps by the stable `loop_nr` (`_par_mat_l<loop_nr>` …)
via `add_variable` — unique per loop and identical across both passes, so no
collision and no counter advance. Verified on both backends
(`tests/scripts/22e-par-many-materialise.loft`).

### `for … par(…)` accepts every iterable source; hash skips its sort (#270) (2026-06-06)

The parallel for-clause now runs over **any iterable**, not just a flat vector.

- **Parser hang fixed (#270).** `for i in 0..3 par(r = i, 2) { … }` infinite-looped
  the parser: `skip_to_parallel_body` (the par-clause error-recovery drain) had no
  comma consumption and no forward-progress guard, so it spun on the `,`.  Added a
  no-progress guard mirroring `consume_call_args`; recovery can no longer hang.
- **Range / `iterator<T>` / text sources now work.** A non-vector, non-keyed source
  is materialised into a flat `vector<T>` (via `materialise_iter_for_par`, reusing
  `build_comprehension_code` for correct per-kind element append) before the queue
  dispatcher partitions it.  Keyed collections (hash/sorted/index/spacial) keep their
  existing `materialise_keyed_for_par` path.
- **Hash skips the sort for par.** `for x in h par(…)` builds its iteration scratch
  from `hash::records()` (raw bucket walk via the new `hash_unsorted` / `n_hash_unsorted`)
  instead of the key-sorting `hash_sorted` — the parallel queue has no use for a hash's
  order.  Sequential `for x in h` stays key-ordered; only the par form differs.
- **Two pre-existing native-codegen par bugs fixed (surfaced here, untested before —
  no keyed/range/primitive-vector par script reached `--native`):**
  - keyed/range materialise wrapped its temp var in a `v_block`, which native lowers
    to a Rust `{ }` scope, so `__par_mat` died before the dispatch used it (E0425).
    Now spliced as `Value::Insert` (inline), like the vector path.
  - a by-value scalar worker (`fn(x: integer)` over `vector<integer>`/range) got the
    element `DbRef` instead of the read-out value (E0308 `expected i64, found DbRef`).
    `tuple_arg_prep` now reads scalar element types out of the record, the 1-element
    case of the existing tuple-worker path.

  Verified on both backends across range/vector/hash/sorted/index sources and
  integer/float/boolean/single worker returns (`tests/scripts/22c-par-sources.loft`).
- **Interpreter text-return par fixed.** A text-*returning* par worker over a
  non-`DbRef` element (a `vector<integer>` / range → primitive input; a
  `vector<text>` → text input) produced garbage or a SIGSEGV: `run_parallel_text`
  always pushed the element as a 12-byte `DbRef`, unlike the integer path's
  input ladder.  `execute_at_text` now takes a `WorkerArg` (Ref / Primitive /
  Text) and `run_parallel_text` selects it by the worker's first-arg kind —
  the same ladder `run_parallel_queue` applies.
- **Interpreter ref-return par over a primitive input fixed.** A struct/
  reference-*returning* par worker over a `vector<integer>` / range fed the
  worker the element `DbRef` instead of the primitive value (`run_parallel_queue_ref`
  → `execute_at_ref` had no input ladder) → garbage results.  `execute_at_ref`
  now takes the same `WorkerArg`; `run_parallel_queue_ref` reads a primitive
  element by value.  Text / struct inputs keep the `DbRef` path (already correct).
- **Par result-var two-pass instability fixed.** The fused-par result var was
  named `_<name>_<global-counter>` via `create_unique`.  Across many par loops
  with mixed result types the `create_unique` count diverged between parser
  pass 1 and pass 2, so the pass-2 `b_var` failed to reuse its pass-1 entry —
  the user name then aliased to a wrong-typed var (`r.len()` on a `text` result
  rejected as `integer`).  The `b_var` is now keyed on the stable `loop_nr`
  (`_<name>_par<loop_nr>`), identical across both passes.  Guarded by the
  intentional `r`-reuse in `tests/scripts/22c-par-sources.loft`.
- **Native text-input par fixed.** A par worker with a `text` parameter (over a
  `vector<text>` source) failed `--native` compilation: the worker closure passed
  the element `DbRef` where the worker wants `&str` (E0308).  `tuple_arg_prep` now
  emits `loft::codegen_runtime::par_read_text_input(cell, elm)` (reads the row's
  text into an owned `String`) for a text first-arg — the text-input sibling of
  the scalar-element read.
- **Native literal-returning text-return par fixed (#273).** A par worker that
  returns text via literals (the @P205 no-work-buffer / owned-`String` shape) has
  no `&mut String` work-buffer param, but the worker closure unconditionally
  passed one → `E0061`.  The Text closure now branches on
  `generation::returns_owned_string(worker_def)`: owned-`String` workers are
  called `worker(cell, arg)` (no buffer); buffer-building workers keep the
  `let mut _w = String::new(); worker(cell, arg, &mut _w); _w` form.  Both par
  emitters (For + Queue) updated; verified on both backends over range / vector /
  text inputs (`tests/scripts/22c-par-sources.loft`).
- **Native fn-ref-returning par implemented (#281).** A par worker returning a
  function reference had no native lowering — the emitter fell through to a
  wrong-arity call to the interpreter stub (`E0061`).  Added the `QueueStitch::Fn`
  native path: `ClosureShape::Fn` (closure returns the native fn-ref tuple
  `(u32, DbRef)` verbatim) → `n_parallel_queue_fn_native` +
  `n_parallel_buf_get_fn_native` / `_drop_fn_native`, buffering one `(u32, DbRef)`
  per row in the new typed `Stores::par_fn_native_buffer_stack`.  Non-capturing
  fn-ref returns now compile + run on `--native`, matching `--interpret`.
- **Capturing closure from a par worker → clear diagnostic.** A par worker that
  returns a *capturing* closure used to hit a raw `index out of bounds` panic on
  both backends (the worker-local captured store is dropped at join, leaving the
  fn-ref dangling).  It is now rejected at parse time: "a parallel worker cannot
  return a capturing closure …".  The check (`worker_returns_capturing_closure`)
  flags only `FnRef` with a set closure-var in return/tail position, so a
  non-capturing `return add5;` and closures used only internally are never
  rejected.  Supporting capture would mean copying each captured environment
  across the thread boundary — deliberately not done.
- **Native narrow-integer-vector par fixed.** par over a `vector<u8>` / `vector<i32>`
  (or any narrow-Integer element) read garbage on `--native`: the worker-element
  closure used `get_int` (8 bytes) regardless of the element's 1/4-byte stride,
  over-reading across rows.  `tuple_arg_prep` now reads a scalar `Integer` element
  at the vector's stride (`element_size`, threaded in) — `get_byte` / `get_i32_raw`
  zero-extended to `i64`, matching the interpreter's `read_primitive_at`.  Other
  scalar kinds keep their fixed-width reads.  Verified both backends
  (`tests/scripts/22d-par-narrow.loft`).

### Program-relative asset loading — relative paths resolve against the program (#255 / @PLN9) (2026-06-04)

Relative file paths now resolve against the **program's own directory** (source
dir under `--interpret`, exe dir under `--native`, host cwd under wasm) instead of
the launch cwd — so bundled assets (fonts, images) load regardless of where the
program is started, unblocking the native games.  `Stores::resolve_path()` is the
single resolution home; absolute paths untouched.  Opt back into cwd-relative with
the `#cwd` file directive (the repo-root tools that operate on the working dir —
the doc generators, the tracker indexer/scanner, the branch-review viewer, the GLB
exporters — declare it); override globally with `LOFT_PATHS=program|cwd`.  Shipped
both backends + a 13-file corpus migration; the wasip2 print path stays gated on
#268.  (PR #269.)

### Coroutine native yield codec — per-shape spray → one layout-driven flatten-walk (@PLAN16) (2026-06-04)

The `--native` coroutine value channel had a per-shape codec: a hand-written
producer+consumer template per yield shape, gated on a runtime tag.  New composite
shapes fell through to the wrong arm — `(integer,float)`, `(integer,boolean)`,
`(vector,integer)` failed to compile.  Replaced with one **flatten-walk derived
from `T`'s slot kinds** (`src/coroutine_layout.rs`): each scalar slot inline as an
`i64`, each reference slot as a full `DbRef`; the per-slot kind list rides as extra
`OpCoroutineNext` args so producer (from `T`) and consumer (from the transmitted
kinds) agree by construction — no runtime shape tag.  Three previously-broken
composite shapes now compile + run via the single walk, zero per-shape code;
`coroutine_matrix` 18/18 green on both backends.  `(text, …)` tuples remain the one
excluded cell (a text element's `&str` repr needs a store intern).  @PLAN16 closed
→ `finished/`; the build was the *with-arm* that graduated DESIGN_VERIFICATION C1
into [Design Protocol 1](DESIGN_PROTOCOL.md).  (PR #269.)

### @PLN11 G2 Track 1 — program cache default-on + binary-mtime invalidation (2026-06-04)

`src/cache.rs`.  `cache::program_cache_enabled()` now returns **true by
default** for real (non-Cargo) invocations — the whole-program startup cache
(~3–3.6× warm-run speedup) is no longer hidden behind `LOFT_PROGRAM_CACHE`.

Precedence order (first match wins):
1. `LOFT_NO_CACHE` set → off.
2. `LOFT_PROGRAM_CACHE` set → on (explicit force; cache tests use it).
3. `CARGO_MANIFEST_DIR` present → off (inside `cargo run` / `cargo test`).
4. else → **on**.

`build_signature()` now folds `binary_signature_tag()` — the running exe's
mtime — so an uncommitted compiler rebuild invalidates bundles.  `BUILD_ID`
(git HEAD) alone did not change across uncommitted edits.

`cache::prune_program_cache()` evicts the oldest `(.store + .manifest)` pairs
after each cold save to keep the cache dir under `LOFT_CACHE_MAX_MB` (default
512 MiB).

Full design + E1/E2/E3 arc: `doc/claude/plans/11-data-as-store/README.md`.
Benchmark detail: `doc/claude/PERFORMANCE.md § Startup cache`.  Commit `77da481`.

### @PLN11 G2 — `read_data` breakdown: allocation-bound, E2 is the only lever (2026-06-04)

`src/ir_read.rs`.  `bench_read_data_breakdown` (`#[ignore]`; run with
`cargo test --release --lib bench_read_data_breakdown -- --ignored --nocapture`)
profiles `read_data` on the real stdlib bundle.

Results:

| Component | Time | Share |
|---|---|---|
| def-fields (attribute + return-type `Type` trees) | 453 µs | 65% |
| body trees | 142 µs | 20% |
| variable tables | 98 µs | 14% |
| **total** | **693 µs** | — |

Variable-table cost is **~0.39 µs/variable** — linear in allocation count, not
in variable count alone.  The hot work is native-graph materialisation (each
variable = a `String` + a boxed `Type`; each def = its attribute/return `Type`
trees).  No targeted `read_function` optimisation can beat this: the cost IS
the allocation.  E2 (zero-copy store-backed reads) is the only structural lever.

Corrects the earlier "~2.2 ms variable tables" figure, which measured a
whole-program bundle (~5–6 k vars) rather than the stdlib slice (~251 vars).
E2 startup prize sized at **~0.7 ms on the stdlib** (scales with def + var
count).  Commit `41835b2`.

### @PLN11 G2 M1 — `Definition` read-accessor seam completed codebase-wide (2026-06-04)

`src/data.rs`, `src/state/`, `src/generation/`, `src/parser/`, `src/compile.rs`.
All immutable `Definition` field reads across the four subsystems now go through
accessor methods (`name()`, `native()`, `source()`, `position()`, `attributes()`,
`code()`, `returned()`, `op_code()`, `known_type()`, `variables()`, `def_type()`,
`rust()`, `parent()`, `closure_record()`, `mutated_captures()`,
`scalars_to_box()`, `synthetic()`) instead of touching public fields directly.

The three milestones:
- M1a — `state/` — landed earlier in the arc.
- M1b — `generation/` — commit `c2741e2`.
- M1c — `parser/` + `compile.rs` — commit `69f0c6e`.

Derived fields (`attr_names`, `const_ref`, `code_position`, `code_length`)
stay as direct reads — they are cheap computed values, not layout-sensitive.

Pure refactor; no behaviour change, no test delta.  The seam is the
precondition for swapping the `Definition` backing representation to
store-based reads per subsystem (E2 arc in @PLN11).

### Nested-vector layout — four corruption/crash clusters fixed (plan-58) (2026-06-03)

Closed the `vector<vector<…>>` stability class across depth × element-type ×
context (plan-58, now `finished/`).  `vector<vector<…>>` is load-bearing (map
tiles, matrices, adjacency lists, comprehension results); a stride/type-id
investigation found four independent defects beyond the one filed crash:

- **Single sentinel (#262, `tests/scripts/183`).**  A freshly-created
  vector-of-vectors element is a 4-byte rec-id HANDLE, but `OpNewRecord` default-
  inits it with the inner scalar's null sentinel.  For `single` the NaN
  (`0x7FC00000`) is a non-zero garbage rec-id → SIGSEGV.  Generalized the `@P380`
  `OpSetInt4`-zero from the copy path to every construction path.
- **Narrow-int literal (`184`).**  `vector<vector<i32>> = [[1,2]]` typed the
  inner literal wide (`integer`, stride 8) while the read used `i32` (stride 4) →
  silent corruption.  `parse_item` now propagates the declared element type into
  the inner literal.  Width-general (i32/i16/u8).
- **Boolean handle stride (`185`).**  The outer vector strided handles by the
  inner scalar size — fine for ≥4-byte scalars, but a 1-byte `boolean` made the
  4-byte handles overlap (corrupt→crash).  Parse-time fix: pass the outer vector
  type as `OpNewRecord`'s type when the inner content is <4 bytes (so
  `record_new` strides by the handle), plus a read-stride clamp to ≥4 — no type
  classification change.
- **Nested comprehension (`186`).**  `[for i { [..] }]` wrote a 12-byte handle
  via the scalar `OpSetInt4` path → eval-stack skew → CONST_STORE write panic;
  and its `known` over-wrapped one level vs `+=` (off-by-one).  Deep-copy
  (`OpCopyRecord`) + element-type `known` (`vector_of`).  Distinct from #248.

Adjacent fix: **`vector<character>` element reads** (`v[0]` / `for c in v`)
errored "Field access not supported on type character" — `get_val` had no
`Type::Character` arm (only the write side did).  Added the symmetric
`OpGetCharacter` read (`tests/scripts/187`).

All fixes verified on `--interpret` and `--native`.  The temporary `--vec4`
investigation lever was added then retired (−109 lines).  Remaining
nested-vector matrix red cell is `#263` (call-returned fn-ref into any
collection — a general closures bug, out of scope).  Benign residual:
≥4-byte inner scalars still over-reserve the outer slot stride (no
corruption/leak) — accepted; a future stride guard (sanitizer) is noted.

### `loft-libs-core` first all-green chunk under strict CI (2026-05-30)

Landed `loft-lang/loft-libs-core` PR #2 (omnibus): canonical
`library-ci.yml` refresh, `cargo build --release --lib --bin loft`
infra fix (closes the `mmap_storage` blocker that broke every
package's native step), Phase 6r random re-clean (bare `#native`
+ source-scan `build.rs`), `arguments` warning sweep (zero
warnings under `LOFT_DENY_WARNINGS=1`, no `.allow_warnings`
opt-out).  All three packages — `arguments`, `crypto`, `random`
— now green on interpret + native under strict warnings.  Pre-
landed @P385 (parser type-inference asymmetry) + @P386 (native
codegen `Str/&str` mismatch) via #231 — both bugs surfaced
during the warning sweep.  Established three warning-clean
idioms now documented in `.claude/skills/loft-write/SKILL.md`:
`not null` on safe-to-default-`[]` vector fields,
capture-into-local before indexing (skip-pattern 5 needs bare-Var
vec), capture-and-null-check.  Plan-12 README gained a
"Bringing a chunk to all-green CI" checklist; REFERENCE.md
records the per-symbol re-clean decision rule (redundant vs
genuine override).  Remaining chunks: `loft-libs-net`,
`loft-libs-graphics`, plus the registry `pr-validate.yml`.

### @P321c native dimension closed + 8 harvested fixes (2026-05-26)

Dogfood pass against the `../personal/training` Loft port surfaced and fixed a
batch of native-codegen, interpreter, tooling, and library bugs.

**@P321c `imaging` native direct-call ABI — FIXED, commit `8095f4ba`.**
`src/generation/mod.rs::output_native_direct_call` now forwards a `LoftStore`
(built from the struct `Reference` arg's own `store_nr`, not the null store) and
marshals each `Reference` arg as a `LoftRef` (`to_loft_ref` + `transmute_copy`,
no `loft_ffi` type named → no dual-crate StableCrateId collision), so a
store-MUTATING package `#native` fn like `load_png(path, image)` gets its full
4-arg ABI.  Return-conversion (`from_loft_ref`) split from the store-handle need
(`returns_loft_ref` vs `needs_loft_store`).  `loft generate` (`src/main.rs`) now
reads field offsets from the canonical schema (`Stores::position`/`size`) instead
of a separate layout calc that treated plain `integer` as 4 bytes (real layout:
`width@0`/`height@8`/`name@16`/`data@20`); `lib/imaging/native` corrected to those
offsets + `set_long`/`get_long`.  imaging un-skipped from `LIB_PKGS_NATIVE_SKIP`;
`native_library_suite` 53/53.  Only the browser-WASM half of @P321c remains.

**@P347 text ordering compare — FIXED, commit `a3e2e269`.**
`< <= > >=` between a `vector<text>` element (`&str`) and another text (`&String`)
failed `--native` compile (`PartialOrd` has no cross-type impl; `==` worked via
`PartialEq`).  `OpLtText`/`OpLeText` (`default/01_code.loft`) now route through
`ops::op_lt_text`/`op_le_text` (`AsRef<str>`), coercing both to `&str`.  `make
fill` regenerated.  Regression `tests/scripts/repro_p347.loft`.

**@P338 vector-index `&mut stores` double-borrow — FIXED, commit `a3e2e269`.**
`v[n / 2]` (checked-div guard `raise_runtime` + vec-get receiver) → E0499.  The
`OpGetVector`/`OpVectorRef` templates now bind `@index` to a local after `@r`.
Regression `tests/scripts/repro_p338.loft`.

**@P346 empty-text `Set` to a `RefVar(Text)` — FIXED, commit `ed47892c`.**
A string interpolation used as an if-branch result over a vector-indexed value
in a loop accumulated text on the interpreter (`[2.5][2.58][2.581]`).
`State::set_var` (`src/state/codegen.rs`) treated `Set(refvar_text, "")` as a
no-op; the buffer kept the prior iteration's content and `OpFormatStack*`
appended.  Now emits `OpClearStackText` (deref-clear), matching native.
Regression `tests/scripts/repro_p346.loft`.

**@P339 `lib/graphics` text kerning — FIXED, commit `29315f20`.**
`measure_text`/`rasterize_text` (`lib/graphics/native/src/text.rs`) summed bare
advance widths.  Both now apply fontdue `horizontal_kern` (rasterize via a float
pen).  `gl_measure_text("AV",40)` = 59.20 < `A+V` 61.91.  Regression
`lib/graphics/tests/kerning.loft`.

**@P341 native-test cache key — FIXED, commit `a3e2e269`.**
`native_cache_key` (`src/native_utils.rs`) now folds each native-package rlib's
mtime, so rebuilding a lib cdylib invalidates the cached `_bin`.

**@P345 typed loop-var diagnostic — FIXED, commit `a3e2e269`.**
`for i: T in …` now emits one clear "loop variable is type-inferred — remove the
annotation" message + recovery (`src/parser/collections.rs::parse_for`), not a
3-error cascade.  (Syntax intentionally unsupported.)

**@P342 `loft generate` method-as-field — FIXED, commit `a3e2e269`.**
The `u16::MAX` schema-position skip is the correct field/method discriminator;
generated `*_fields` no longer emit bogus constants for methods.

Also filed (open): @P343 (vector<fn-ref> for-loop mis-dispatch — partial
diagnosis recorded, P214-class).

### Open-bug design pass — 4 fixes + 5 grounded designs (2026-05-26)

A focused pass over the remaining open P-issues: each was carried to a
code-grounded fix design, then implemented + verified where the dev
environment allowed.

**@P348 GL golden HiDPI — FIXED.** `tests/graphics_gold.rs::crystal_editor_gl_matches_gold`
degraded the exact-dimension `assert_eq!` to a graceful skip when the captured
framebuffer differs from the gold (a HiDPI/display-scaled environment can hand
a scaled framebuffer even under `xvfb-run`).  CI + `make test-gl-golden`
(controlled size) still compare pixels.

**@P332 Windows install → 0 installed — FIXED.** Root cause: the install/extract
home resolves via `dirs::home_dir()`, which reads `$HOME` on Unix but
`USERPROFILE` on Windows — so the e2e test's `HOME=<tmpdir>` isolation leaked
into the real profile and cross-run caching routed everything to
`skipped_cached`.  `registry_index::cache_dir()` now honours a cross-platform
`LOFT_HOME` env var first (`HomeGuard` sets it); both `#[cfg(not(windows))]`
gates removed; `registry_e2e` 5/5.  Production unchanged (var unset →
`dirs::home_dir()`).

**@P333 Windows `/tmp/` fixtures — FIXED.** `moros_render/geometry.loft` +
`moros_sim/persistence.loft` ported to cwd-relative filenames + `delete()`
(the `scene_glb.loft` convention); Windows skips removed from `wrap.rs` +
`native.rs`.  moros_sim 137/137 + moros_render 155/155, no artifacts left.

**@P340 baseline metric — PARTIAL FIX.** New `gl_font_ascent(font, size) -> float`
(fontdue `horizontal_line_metrics`) lets callers baseline-align mixed-size
text; additive, so the text golden is untouched.  Needed a new
`(I32,F64)->F64` auto-marshal arm in `src/extensions.rs`.  `lib/graphics/tests/font_ascent.loft`,
66/66 both backends.  The `size*1.2`/`size*0.8` rasterization constants are
deliberately unchanged (switching them needs a `gold-text.png` regen).

**Designs recorded, implementation deferred (blocker noted in each PROBLEMS.md row):**
@P334 (`lib/world` wasm trap —
needs `wasmtime`, not installed here), @P343 (all three interp layers now
precisely located incl. the termination-test third layer; native E0600 half
separate), @P344 (doc-fix recommended; skill-checklist edit permission-blocked;
per-loop-scoping rejected as a core-model change for a Low bug), @P331 (cdylib
i64→i32 truncation site found; fix is an M-effort ABI-width alignment touching
the 53-cdylib gate — not blind-patched).

### @P349 — browser WASM playground: refresh bundle + JSON + file I/O (2026-05-26)

Refreshing the `doc/pkg` browser bundle (stale since 2026-05-18) against the
`../personal/training` port's `.field()` routine syntax surfaced a chain of
three gaps that left the gallery/playground unable to run file-reading or JSON
programs.  All fixed:

1. **Stale bundle.** `make wasm` rebuilt `doc/pkg/{loft.js,loft_bg.wasm}` from
   current source (`loft_bg.wasm` 2211894→2260122→2262xxx bytes across the
   three rebuilds).  The in-browser parser now accepts the JsonValue method
   syntax it rejected before (`Expect token ;`).
2. **`06_json.loft` not bundled.** `DEFAULT_FILES` (`src/wasm.rs`) embedded
   `01_code`..`05_coroutine` but not `06_json.loft`, so `json_parse` was an
   `Unknown function` in-browser (native JSON fns were already compiled in —
   no wasm cfg-gate).  Added the embed.
3. **Runtime `file()` ignored VIRT_FS.** `State::get_file_text`'s
   `#[cfg(feature="wasm")]` branch (`src/state/io.rs`) read only via the JS
   `host_fs_read_text` bridge (absent in the playground) → `file().content()`
   returned `""`, so `json_parse(file(...).content())` → `JNull` → `NaN`.  Now
   consults `wasm::virt_fs_get` first (where `compile_and_run` puts passed
   files), falling back to the host bridge — live-FS hosts unaffected.

Verified under Node (`initSync`+`compile_and_run`): `file().content()` →
`HELLO123`; `json_parse(file).field("activities").item(0).field("duration_s").as_number()`
→ `3600`, matching native.  Remaining minor caveat (in the @P349 PROBLEMS.md
row): `run_pipeline` picks `main` as the alphabetically-first user file
(`.min()`), so a data file sorting before the program is mis-compiled as main.

`doc/brick-buster.html` is a self-contained `--html` bundle (base64-embedded
wasm) — independent of `doc/pkg`.  (Earlier note here said its embedded wasm
"runs `loft_start: OK` under `tools/wasm_repro.mjs`, no @P337 trap" — that was
a FALSE NEGATIVE: the stub harness's `loft_gl_create_window` returns 0, so the
program bails before drawing and never reaches the render path.  See the @P337
correction below and @P351.)

### @P337 — Brick Buster browser bundle: corrected diagnosis + pipeline hardening (2026-05-26)

@P337 ("Brick Buster broken on the site / page times out") had been recorded as
a `vector<float>` length divergence on wasm32 (`panic_bounds_check` in
`build_mvp_2d`).  **That diagnosis is DISPROVEN.**  A minimal repro (16-elem
`vector<float>` literal in a struct field, index `[15]`) AND a faithful copy of
`build_mvp_2d` (computed-expression projection passed as `const vector<float>`,
indexing `proj[0..15]` + building a new 16-float vector) BOTH read back
`len==16` and `[15]` correctly on the wasm32-unknown-unknown `--html` build —
identical to interpreter + `--native` — even after `wasm-opt -O1 --asyncify`.
The committed `doc/brick-buster.html` renders cleanly in real headless Chromium
(WebGL via SwiftShader), rAF ~60fps.

**Actual root cause — a build-pipeline / toolchain hazard, not a runtime bug.**
`make wasm` (wasm-pack, `feature=wasm`) and `loft --html` write the SAME
`target/wasm32-unknown-unknown/release/libloft.rlib` with incompatible feature
sets (the Makefile has long warned of this).  Two independent break modes, both
passing the old size/DOCTYPE sanity check:

1. **rlib STOMP** — after `make wasm` the rlib carries `feature=wasm` →
   `wasm-bindgen`/`js_sys`, so `--html` emits a wasm importing
   `__wbindgen_placeholder__` (35+) that the embedded `loft-gl-wasm.js` glue
   (raw `loft_gl`/`loft_io` externs only) does not provide → the page fails to
   instantiate.  A correct `--html` bundle imports ONLY `loft_gl` + `loft_io`.
2. **MISSING `wasm-opt`** — without binaryen the `--asyncify` pass never runs,
   so there is no frame-yield; the HTML driver runs `loft_start()`
   synchronously and brick-buster's `for _ in 0..10000000` render loop blocks
   the browser main thread forever ("page times out").

Today's doc/pkg `make wasm` stomped the rlib; the working-tree
`doc/brick-buster.html` had separately been rebuilt without `wasm-opt`.

**Landed (toolchain + hardening — diagnosis-only on the runtime, no codegen
change):**
- `tools/check_html_bundle.mjs` — static gate: fails on non-`loft_gl`/`loft_io`
  imports (stomp) or a missing `asyncify_start_unwind` export (no frame-yield).
  Wired into `make game` step 6 so a broken bundle fails the build.
- `loft --html` (`src/main.rs`) — now warns LOUDLY, in plain language, when
  `wasm-opt` is absent (the page will hang, not merely be larger).
- `scripts/doctor.sh` + `make doctor` — full wasm/native toolchain report with
  plain-language consequences and package-manager-specific install commands;
  finds cargo/wasmtime-installed tools regardless of shell PATH.
- `doc/claude/WASM.md` — new "Build Toolchain Dependencies" section + the
  rlib-stomp build-order rule.
- `doc/brick-buster.html` regenerated via `make game` (correct rlib + asyncify),
  verified in headless Chromium.

**Follow-ups filed:** @P350 (a DIRECT `loft --html` after `make wasm` still
ships a broken bundle silently — the gate is only in `make game`; needs an
isolated rlib `--target-dir` or `--html` self-validation), @P351 (the
`tests/html_wasm.rs` Node gate cannot exercise the GL/render path — the exact
coverage gap that let this ship + be misdiagnosed).

### Native codegen — eliminated the `output_call_inner` match (2026-05-22)

`src/generation/dispatch.rs::output_call_inner` no longer contains a monolithic
`match` of per-Op emission arms — it is now just a registry-first guard
(`emit_op`) plus the template/user-fn fallback.  The 14 remaining arms were
relocated VERBATIM into `OpEmitter`s: the text/format/buffer family into one
`ops::text_ops::TextDispatchEmitter` (reproducing the @P283 refvar→`Stack`
rewrite internally), and the pass-throughs (`OpConvRefFromNull` / `OpGetTextSub`
/ `OpDatabase` / `OpStep` / `OpRemove`) into `ops::misc_ops`.  No `#rust`
template changed, so `src/fill.rs` (the interpreter) is byte-identical and
native emission matches the deleted arms byte-for-byte.  The
`dispatch_op_arm_budget` test is repurposed as a 0-ratchet that fails if a
`"Op…" =>` match arm is ever re-introduced.

### @P321 native library gate — 16/17 packages green (2026-05-23)

Seven native-codegen root causes (@P321a–g) that blocked `tests/native.rs::native_library_suite` from reaching
full green.  Splits were fixed and un-skipped independently; the gate now covers 16/17 packages.  Only `imaging`
remains skipped (`LIB_PKGS_NATIVE_SKIP`) pending @P321c (design-level, M+).

**@P321d `moros_map/serial` — FIXED 2026-05-23, commit `93a43051`**

`default/01_code.loft` / `src/fill.rs`.
Nested vector index `m.a[0].b[2]` emitted two live `&mut stores` borrows (E0499).
`vec_get_or_raise_runtime` is `&mut self` (may call `raise_runtime` on OOB); the outer
`stores.vec_get_or_raise_runtime(&<inner>, …)` held its receiver borrow across argument
evaluation while `<inner>` expanded to a second such call — Rust E0499.
Fix: the `OpGetVector` / `OpVectorRef` `#rust` templates in `default/01_code.loft` bind
`@r` to a local first (`{{let __vr = @r; s.vec_get_or_raise(&__vr, …)}}`), so the inner
borrow is fully evaluated (owned `DbRef`) before the outer call starts.  `src/fill.rs`
regenerated via `make fill`; the interpreter gets the same harmless local binding — single
source of truth for both backends.
Regression: `tests/scripts/repro_p321d.loft` (both backends).

**@P321e `moros_editor` — FIXED 2026-05-23, commit `da75dc67`**

`src/generation/emit.rs`.
A text-returning fn whose body is a `match` of format strings `.to_string()`'d the match
result into a `__ret_N` LOCAL `String`, then returned `Str::new(&local)` — a borrow of a
fn-local that drops at return → runtime `ptr::copy` panic in the caller's `.to_string()`.
A `RefVar<Text>` work-buffer arg existed but `output_set`'s P205 scratch guard fires only
when the returned value is a `RefVar` — the fn was returning a DIFFERENT local.
Fix: the text-`Return` path in `output_block` also routes through `stores.scratch` when the
returned value is a text LOCAL var (not already a `RefVar<Text>` work buffer).  moros_editor
5/5 files native + interp.

**@P321g `moros_ui` — FIXED 2026-05-23, commit `69f4ec3b`**

`src/generation/dispatch.rs`.
A `&`-ref-param call on an assignment RHS (`ec_hit = route_click(p, st.es_tools, …)`)
emitted `let` in expression position — `error: expected expression, found 'let' statement`.
The `&`-ref arg `st.es_tools` (an addressable field) materialises a
`Set(__ref_N, OpGetField…)` statement ahead of the call, so the RHS is
`Insert([Set(__ref_N, …), Call])` wrapped in `Value::Span` for source-position tracking.
`output_set`'s S35 hoist — which lifts all-but-last Insert ops to statements — matched only
a *bare* `Insert`, so `Span(Insert)` fell through to the brace-less `Insert` arm of
`output_code_inner`, producing `let x = let __ref_N = …; call()`.
Fix: S35 unspans `to` before the Insert check.  moros_ui 4/4 files native + interp.
Regression: `tests/scripts/repro_p321g.loft`.

**@P321c `imaging` — DIAGNOSED, needs design, NOT fixed** *(status as of
this 2026-05-23 entry — closed three days later; see the 2026-05-26
"@P321c native dimension closed" entry above, commit `8095f4ba`)*

`output_native_direct_call` (`src/generation/mod.rs:2181`) cannot express a
store-MUTATING `#native` fn: `load_png` decodes a PNG, allocates the pixel vector, and
writes `name`/`width`/`height`/`data` into the `Image` struct.  The ABI only marshals
text → `(ptr,len)`, vector → `(*const ELEM, count)`, and scalars; no `LoftStore` path
and no struct-ref marshalling → emits a 3-arg call to a 4-arg fn (E0061).
Recommended route: `codegen_runtime + Abi::Cell` (the crypto pattern, with store access)
reusing `src/png_store.rs::read`, with new `(text, struct-ref)` call-marshalling, a dual
interpreter(cdylib)/native(codegen_runtime) split, and `png`-feature gating.
Full diagnosis in PROBLEMS.md @P321c.  `imaging` stays in `LIB_PKGS_NATIVE_SKIP`.

---

### @P274 closed 2026-05-14 — heap-typed tail return + text-concat type-dispatch

Two coordinated codegen + parser fixes for native-only crashes
that surfaced when @PLN42 viewer added the
`render_md_table_row` / `parse_md_row` / `find_table_headers`
helper trio (commit 89fd2767).

**Bug A — `OpFreeRef` hoisted before tail-call argument use**
(`src/generation/pre_eval.rs`).  `patch_hoisted_returns` Pass 2
collapsed `[Call(parse_row, …, var___ref_1), OpFreeRef(var___ref_1),
Return(Null)]` (emitted by `scopes::free_vars`'s else-branch for
heap-typed tails — Vector / Reference / Enum-ref bypass
`is_value_return_type`'s primitive-only check) into
`[OpFreeRef(var___ref_1), Return(Call(parse_row, …, var___ref_1))]`,
giving native code `OpFreeRef(var); var.store_nr = u16::MAX;
return n_parse_row(…, var)` — callee derefs `stores[65535]` and
panics at `src/keys.rs:249`.  Two-part fix: (1) Pass 2 now
detects when `expr` references any var that an intervening free
op will free, and skips the hoist; (2) `detect_ref_tail_capture`
now accepts `Type::Never` blocks and looks up the enclosing
function's return type — so the original `[Call, free,
Return(Null)]` pattern in early-return arms gets the
`let __native_tail_ret = call(…); free; return __native_tail_ret;`
wrap that orders the use BEFORE the free.

**Bug B — `text + integer` concat misrouted** (`src/parser/vectors.rs`).
`parse_append_text` only dispatched parts on Text / Character;
integer / float / boolean / vector / enum etc. fell through to
`OpAppendText` with the wrong operand type → SIGSEGV in interp,
rustc E0614 "type i64 cannot be dereferenced" in native (the
`+= &*(...)` deref).  Fix routes non-text/non-character parts
through `append_data` (the same dispatch the `"…{x}…"` format-
string interpolation path uses), unwrapping `RefVar(inner)` so
`&text` parameters keep the existing OpAppendStackText / OpAppendText
fast path.

Pinned by `tests/scripts/100-p274-tail-return-with-cleanup.loft`
(walked through both backends by `tests/native.rs::native_scripts`
and `tests/wrap.rs`).  `make view` reverted to `--interpret`
default in 5dae80cc, restored to `--native` once @P274 closed.

### Plan-35 (branch-review viewer) closed 2026-05-14

Plan-35 ran 2026-05-13 → 2026-05-14.  Goal: a browser-accessible
doc + code review surface for the current loft branch, served by a
loft-script binary against the host loft binary as a frozen pair.

**Per-phase summary** (all shipped 2026-05-13 unless noted):

- **00** Skeleton + binary build.  `tools/viewer/` package layout,
  `make view-build` + `make view` + `make view-refresh` Makefile
  targets, `BUILD_NOTES.md` records the loft commit the viewer was
  built against.
- **01** HTTP routes.  Server skeleton via `lib/server`, `/`, `/tree/<path>`,
  `/raw/<path>`, `/static/style.css`, 404 fallback.  Originally
  blocked from `--native` by @P262 + @P263; fixed in the seven-bug arc.
- **02** Code-file rendering.  `/file/<path>` renders any text file as
  line-numbered HTML with `<a id="L42">` anchors for fragment scrolling.
  HTML escape + tab-to-4-spaces + binary-extension skip-list.
- **03** Markdown subset (later extracted to `lib/markdown`).  Headings
  with GH-slug ids, paragraphs, fenced code blocks, inline formatting
  (bold/italic/code/strikethrough), links with relative-path resolution,
  images, autolinks, blockquotes, lists with continuation merging,
  GFM tables with alignment, task lists, setext headings, hard line
  breaks, backslash escapes, HTML escaping.  Extracted as standalone
  `lib/markdown/` library + `lib/markdown/tests/01-render.loft` (~30
  in-library assertions, one per construct).  Two follow-up extensions
  shipped 2026-05-14: `extract_headings(source)` returning
  `vector<Heading>` for TOC building; `tag_url_prefix` and
  `image_url_prefix` parameters wiring `@P-id`/`@PLAN-id` autolinks
  + relative image rewriting (caller chooses prefixes).
- **04** Git state via wrapper script.  `tools/viewer/refresh.sh` dumps
  branch + changed-files + recent-commits + uncommitted state to
  `tools/viewer/state/*.json` (uses `git` + `jq`).  Viewer reads JSON
  via the (now fully-wired) JSON natives from P54 sprint completed
  the same day.
- **05** Diff + commit views.  `/diff/<path>` and `/commit/<sha>` use a
  shared `render_diff()` helper that classifies each line and wraps it
  in `.diff-add` / `.diff-del` / `.diff-hunk` / `.diff-head` / `.diff-meta`
  / `.diff-ctx` / `.diff-noeol` spans.  Top-right `[Rendered ¦ Diff vs main]`
  toggle on every `/file/` page; the diff link hides when no per-file
  diff exists.  `breadcrumbs()` fix: parent dirs always link to
  `/tree/<dir>`; only the leaf segment uses the page's kind, so
  `/diff/<path>` doesn't generate broken `/diff/<dir>` parent links.
- **06** Full GFM tables — alignment + headers + body + nested formatting
  in cells (via `render_inline`) shipped via the `lib/markdown` table
  renderer.  Multi-line cells + escaped pipes deferred (rare in loft
  docs; promote when a downstream consumer needs them).
- **07** Closeout (this entry) — DEBUG.md § Branch review viewer +
  CHANGELOG.md user entry + this technical retrospective + plan moved
  to `plans/finished/35-branch-review-viewer/`.

**Loft drivers — features matured by building this**:

- `lib/server` proven well beyond test fixtures (lib's first big
  consumer outside the test suite).
- The seven-bug native arc @P262→@P269 (closed 2026-05-13) was
  surfaced by trying to compile the viewer to `--native`.  Each bug
  was a real loft-codegen issue that was invisible until a real
  consumer walked the path — `lib/web` + `lib/server` integration,
  text-returning fn inline calls, fn-ref dispatcher work-buffers,
  cross-crate native fn routing, JSON parser UTF-8, JSON natives
  todo-stubs.  Closed via DESIGN_DECISIONS.md § C67 ("fail at startup,
  not at runtime — internal-bug runtime panics caught at compile time").
- P54 (JsonValue ecosystem) native side completed via @P268 + the
  16-fn follow-up wiring all 23 JSON natives in
  `src/codegen_runtime.rs`.
- New `lib/markdown` library spun out as a reusable single-file loft
  module, ~720 lines, with comprehensive in-library tests; first
  pure-loft library born from a real consumer.
- Surfaced gaps not blocking the ship: subprocess primitive
  (workaround: `refresh.sh`), regex (workaround: char-by-char
  parsing), HTML escape lib (workaround: `html_escape` in `lib/markdown`
  exposed publicly).

**Plan moved to `plans/finished/35-branch-review-viewer/`.**

---

### Plan-37 phase 04b — viewer per-doc sidebar shipped 2026-05-14

`tools/viewer/src/main.loft` gains two sections at the
bottom of every `/file/<path>` page:

- **Referenced by** — reads `index/tags.json`'s `links`
  bucket (phase 09 backlinks).  Lists every doc that links
  inbound to the current file, with file:line + context.
- **Tags on this page** — walks the tag buckets, surfaces
  any tag whose ref list contains the current file.  Each
  tag is a clickable chip → `/tag/<bare>` (phase 04a's tag
  detail page).

Both render to empty strings when `index/tags.json` is
missing or the file has no associated entries — pages
degrade gracefully on a fresh checkout that hasn't run
`make index` yet.

CSS additions: `section.sidebar` for the section wrapper,
`ul.tag-list` for the chip-style tag pills (flex-wrap, dark
mode covered).

Only the welcome-landing half of phase 04b remains open;
that one depends on @PLAN35 phase 08 which is unstarted.

Verified end-to-end 2026-05-14:
`target/release/loft --interpret --lib lib/ tools/viewer/src/main.loft`
starts the viewer; `curl localhost:8765/file/doc/claude/PROBLEMS.md`
returns 295 KB of HTML containing both sidebar sections
("Referenced by 59" + "Tags on this page" with @P198…@P204
chips, all populated from `index/tags.json`).  An earlier
report of a `--interpret` extension-loading panic (filed as
@P273) turned out to be a stale `target/release/loft`
binary predating the cdylib's last build — once rebuilt
fresh, the `apply_manifest_side_effects` path picks up the
dep cdylibs correctly via `auto_build_native`.  @P273
closed as no-bug; lesson recorded for the next "missing
native" symptom.

---

### Plan-37 phase 09 follow-up — broken-link cleanup shipped 2026-05-14

The 61 broken markdown links surfaced by phase 09's
`broken_links` bucket are all cleaned up.  CI gate enabled
(`tests/index_hygiene.rs::index_hygiene_clean` checks both
`.broken` and `.broken_links` are empty).

**What landed:**

- `tools/indexer/fix_broken_links.py` — auto-fix script:
  for each broken target, tries the `try_extra_dotdot`
  heuristic (pop intermediate path segments and check
  whether the result exists).  Catches the dominant
  off-by-one `../` case where a plan README in
  `plans/<dir>/` cites a top-level doc as `../X.md` but
  needs `../../X.md`.  Manual override map for the
  plan-22 / plan-35 closeout drift.
- Scanner tightened in `tools/indexer/scan.sh` — link
  extraction now uses per-file awk that tracks fenced
  code-block state; example links inside `\`\`\`markdown`
  blocks no longer count as real refs.  Cost: ~1.5 sec
  added to the scan (was 1.5s, now 3s; still under the
  5-sec budget).  Without this fix, the auto-fixer
  rewrote example links to bogus paths in several files
  (caught + reverted via the validate-after-fix step).
- 106 of the 61 distinct broken-target refs auto-fixed
  (61 distinct targets, but 106 individual ref sites).
  Remaining ~20 manually patched: missing-doc citations
  (`DX.md`/`LSP.md`/`WEB_SERVER_LIB.md`) redirected to
  the corresponding `lib_plans/future/` dirs;
  `BYTECODE_CACHE.md` and similar not-yet-written sibling
  docs converted to plain-text mentions; intentional
  template / test-fixture / inline-backtick examples got
  `<!--noindex-->` markers.
- `tests/index_hygiene.rs` rewritten as a single
  `index_hygiene_clean` test (was two parallel tests
  racing on `make index`; corruption surfaced as
  intermittent failures).

**Numbers:**

- Before: 61 broken markdown links, 309 link targets, 1297
  inbound links.
- After fence-aware scanner: 19 broken (42 false positives
  from fenced examples removed), 264 targets, 1277 links.
- After cleanup: 0 broken, 245 targets, 1267 links.

The phase 09 follow-up section in
`plans/42-tracker-index/09-backlinks.md` is updated to
mark this closed.

---

### Plan-37 phase 06 — retroactive `@`-tagging shipped 2026-05-14

`tools/indexer/migrate.py` rewrites bare-name tracker
references to `@`-prefixed form across `doc/claude/**/*.md`:

- `P259` → `@P259` when 259 is a valid PROBLEMS.md row ID.
- `plan-22` → `@PLAN22` when 22 is a valid plan dir number.

The migration is **conservative on purpose** — false
positives in 154 files would be expensive to clean up by
hand:

- Skip `P\d+-R\d+` (phase-N risk-M notation in COROUTINE.md
  / CHANGELOG_TECHNICAL.md / SAFE.md).
- Skip single-digit `P[1-9]` — overloaded with PERFORMANCE.md
  design IDs ("Design: P1") and plan-N phase-M shorthand
  ("P5.2").  Two-digit `P54` and longer are unambiguous
  enough.
- Skip refs preceded by `/` (`/tag/P259` URL routes
  shouldn't break).
- Skip refs inside fenced code blocks.
- Skip refs inside same-line backtick spans (so `` `P259` ``
  examples explaining the convention survive).
- Skip lines containing `<!--noindex-->`.

Multi-line backtick spans (rare but present in CLAUDE.md's
"## Tracker tags" section) need explicit `<!--noindex-->`
markers per line.

**Numbers**:

- 1500+ refs migrated across ~150 `.md` files.
- Legacy bucket dropped from 2643 → 1917 refs (the residue
  is single-digit `P1`-`P9` skipped on purpose, refs to
  closed P-issues no longer in PROBLEMS.md, and code-file
  refs which don't migrate).
- New form: 783 `@P-id` refs + 753 `@PLAN-id` refs.
- `tests/index_hygiene.rs::no_broken_tracker_tags` still
  green.

Phase 06 was originally framed as "retroactive tagging +
closeout" — the closeout half is DEFERRED to after phases
7+8 (loft-native scanner + multi-project deploy).

---

### Plan-37 phase 09 — backlinks (links bucket) shipped 2026-05-14

Indexer now extracts every `[text](path.md)` markdown link <!--noindex-->
across the repo, resolves the target relative to the source
file's directory, and groups inbound refs under a top-level
`links: {target: [{file, line, anchor, context}]}` bucket
in `index/tags.json`.  Path resolution handles `..`, `./`,
repo-root `/...` paths, anchors, and skips http(s)/mailto
schemes.

Two new CLI surfaces on `./scripts/idx`:

- `incoming:<path>` — inverse of `file:`; lists everything
  that links TO the given path.  Trailing `/` resolves to
  `/README.md`; bare basename matches against any key
  ending in `/<name>` (returns `{ambiguous: [...]}` when
  multiple paths match).  `incoming:@PLAN35` delegates to
  the existing `tag:` lookup so the same query shape works
  for both file paths and `@`-tags.
- `broken-links` — sibling of `broken`; lists links
  pointing at non-existent files.  Initial scan surfaced
  62 stale references on the loft tree (mostly off-by-one
  `..` counts in `doc/claude/plans/<dir>/README.md` after
  files were moved to `finished/`).  No CI gate yet — the
  cleanup is a follow-up before tightening
  `tests/index_hygiene.rs` to fail on broken links.

**Bug fixes during the work:**

- **awk match() clobber** — the link-extraction inner
  loop's `resolve(base, target)` helper called `match()`
  internally, clobbering `RSTART`/`RLENGTH` for the outer
  loop's substr-advance step.  Effect: every link emitted
  twice (the second emit re-walked the same content from
  a stale offset).  Fixed by capturing `RSTART`/`RLENGTH`
  into local `rs`/`rl` before any helper call.
- **awk single-quote in shell-quoted block** — a comment
  containing `loop's` broke out of the bash single-quoted
  awk script, surfacing as "syntax error near token `('"
  at the apparent line of the next awk statement.  Comment
  rephrased to drop the apostrophe.
- **jq `--argjson` ARG_MAX overflow** — the assembled
  `LINKS_JSON` (~150 KB on the loft tree) exceeded the OS
  argv limit.  Switched the merge step to `--slurpfile`
  reading from a temp file (no argv pressure).

**Performance:** scanner runs in 1.5 sec on 953 files (was
1.0 sec without the link pass).  No CI gate impact.

`tests/index_hygiene.rs` continues to enforce zero broken
`@`-tag refs (phase 03 contract).  The `broken_links`
bucket is detection-only for now.

---

### Plan-22 (mutable closures) closed 2026-05-13

Plan-22 ran 2026-05-10 → 2026-05-13.  Goal: make closures whose
bodies mutate captures work intuitively without user-visible
annotation — implicit-by-body classification into cases A/B/C.

**Per-phase summary** (all SHIPPED 2026-05-13):

- **00** Matrix freeze + harness wiring.  `tests/mut_closure_matrix.rs`
  scaffolded (44 cells cross-mode); Case A baseline cells green.
- **01** Mutated-captures detection.  `walk_for_mutations` walker marks
  captures as `mutated: bool`; no behaviour change.  Known gap: first-
  pass GetField in `src/parser/vectors.rs:2498-2512` is non-load-bearing
  post-@P260 (cells handle both sides).
- **02** Case B (co-scoped mutating) + all sub-phases:
  - 02b: auto-Reference attribute emission in `synthesize_closure_record`.
  - 02c: Reference-type capture routing in `typedef.rs::fill_database`.
  - 02d-iii.a: `scalars_to_box` type-flip helper (outer local → cell).
  - 02d-iii.b: read auto-deref hook in `parse_var`.
  - 02d-iii.c: boxed-scalar assign-rewrite helper + `change_var_type` guard.
  - 02d-iii.d: `cell_alloc_prepend` helper for first-set rewrites.
  - 02d-vii: text-return crash fix (cell encoding + return routing).
- **03** Case C (factory / escaped closure).  Liveness check + @P259 fix
  (4 commits — OpIncRc + cascade-free cell ownership for multi-factory
  pattern).
- **04** DECOMMISSIONED 2026-05-13.  The cell + auto-Reference from
  phases 02-03 already gives Case D correct shared-state semantics;
  outer + closure share the same cell automatically.  No rejection code
  shipped.  See `04-case-d.md § Major finding`.
- **05** DEFER-BY-DEFAULT.  `Mutable<T>` helper unnecessary: the cell IS
  the shared-ownership mechanism.  Revisit only if a concrete use case
  surfaces that cells can't handle.
- **06** Doc closeout (this entry).  DESIGN_DECISIONS.md C38 updated;
  CAVEATS.md C38 cross-reference updated; ROADMAP.md @PLAN22 row
  removed; plan moved to `finished/`.

**Bug yield — P-issues filed during @PLAN22:**

- **@P256** — vector-capture into closure crashed both backends (no clean
  rejection).  Closed 2026-05-12 with parse-time rejection in
  `src/parser/objects.rs::resolve_name`.  Pinned by
  `tests/parse_errors.rs::p257_vector_capture_in_closure_rejected`.
  *(Filed as part of @PLAN15 closeout probing, attributed to @PLAN22's
  scope — collection capture is a closure-record layout issue.)*
- **@P257** — same as @P256 (duplicate tracking number; see PROBLEMS.md).
- **@P258** — native + interp layout divergence for cell-encoded scalars.
  Closed in phase 02d-iii.b.
- **@P259** — multi-factory cell ownership crash (OpIncRc missing +
  cascade-free teardown).  Closed 2026-05-13 via 4 commits
  `9f00afec` / `29ee04fd` / `cfb65e8b` / `0711973b`.
- **@P260** — closures captured `Type::Reference` by deep-copy; mutations
  silently no-opped.  Closed 2026-05-13 via `cfad6274` (one-line
  architectural fix: drop `is_mutated` gate in `synthesize_closure_record`).
  6 new cross-mode cells in `tests/mut_closure_matrix.rs`.
- **@P261** — vector-field literal-assign appended instead of replacing.
  Closed 2026-05-13 via `a1cf258a` (prepend `OpClearVector` in
  `towards_set`'s vector-literal path).  Pinned by
  `e_d3_struct_vector_assign_in_closure`.

**Final test surface**: 44 `mut_closure_matrix` cells + 22
`closure_matrix` (@PLAN15 regression net) + 633 issues suite + 26
leak guards — all green, interp/native byte-identical.

Active plans remaining after close: 1 (07-error-messages).
Plan moved to `plans/finished/22-mutable-closures/`.

### Plan-15 (closure validation matrix) closed 2026-05-12

Plan-15 ran in one session 2026-05-12 — promoted to current,
shipped all 6 phases, and closed.  Final matrix: 22 cells in
`tests/closure_matrix.rs` across 6 capture shapes (C0
non-capturing, C1 single-int, C2 text, C3 Reference, C5 multi-
basic, C6 nested) and 4 destinations (D1 local, D2 direct
stack, D3 struct field, D4 vector element — D4 only for C0).
Plus 5 leak guards in `tests/leak.rs::p15_phase0[345]_*_no_leak`
covering text / Reference / nested-closure capture surfaces
under 100-iteration tight loops.

**Bug yield: 0** new P-issues filed.  All gaps the plan was
designed to surface (closure-DbRef leak, "move-vs-copy
semantics gap analogous to T1.8c") turned out to be non-
issues — the underlying support landed earlier through @P213
(2026-05-04 — `Parts::ChildRec` layout-widening for struct-
field captures), @P214 (2026-05-05 — vector-of-non-capturing
closures), @P215 (2026-05-05 — nested closure name resolution),
and @P227 (2026-05-05 — text-returning fn-ref calls).

**Per-phase summary** (all SHIPPED 2026-05-12):

- **00** Harness wiring + smoke (3 cells).
- **01** C0 (non-capturing) × D1/D2/D3/D4 (5 cells, pins @P214).
- **02** C1 + C5 (basic-type captures) × D1/D2/D3 (6 cells,
  pins @P213 + @P215).
- **03** C2 (text capture) × D1/D2/D3 (3 cells + 2 leak
  guards) — disposed LIFETIME.md "Type::Function NOT YET
  HANDLED" annotation as documentation drift.
- **04** C3 (Reference capture) × D1/D2/D3 (3 cells + 2 leak
  guards) — no read-after-free, no DbRef-in-closure-record
  leak.
- **05** C6 (nested closures) × D1/D2/D3 (3 cells + 1 leak
  guard) — D3 included (matrix's "deferred" was conservative).
- **06** Doc closeout — LIFETIME.md "Implementation path"
  trimmed; ROADMAP.md / USER_FACING.md / plans/future/36-
  audience-generative-art cross-refs updated; plan moved to
  `plans/finished/15-closure-validation/`.

Active plans now: 2 (07-error-messages + 22-mutable-closures).

### Plan-14 (tuple validation matrix) closed 2026-05-11

Plan-14 ran 2026-04-30 → 2026-05-11.  Final matrix: 40 cells
across 5 element types (E1 scalars, E1n integer-not-null, E2
text, E3 nested, E4 closure, E5 struct reference) and 3
destinations (D1 local, D2 direct stack, D3 struct field), all
cross-mode-validated under `tests/tuple_matrix.rs`'s
interp/native byte-identical assertion.  Plus E6 (struct value)
folded into E5 by C65 design decision and E7 (collections in
tuples) closed-by-non-goal.

**Phases shipped (00, 01, 02, 03, 04, 05, 07):**
- 00: cross-mode harness in `tests/common/cross_mode.rs`;
  `find_loft_rlib`, `compile_native_job`, `run_native_job`
  exposed `pub(crate)` from `tests/native.rs`.
- 01: 12 E1/E2 cells (D1 + D2: local, arg, return, inline,
  match-subj, if-arm).  T1.8a closed via the lighter "rust_type
  Context::Result recursion" fix instead of new opcodes.
- 02: 5 E3 cells.  Closed @P247 (nested-tuple text move in
  format-string interpolation) + @P248 (element-of-element
  assignment `t.0.1 = 99`).
- 03: 5 E4 cells (closure-typed tuple elements).  Closed @P249
  (20-byte fn-ref layout extended into 6 tuple codegen sites +
  `__fn_ref_tmp` postfix-call temp marked skip_free).
- 04: 6 E5 cells (struct references).  Decision: MOVE
  semantics (recorded as C64).  Loop-iteration aliasing bug
  parked as @P250.
- 05: 6 D3 cells (tuples in struct fields).  Decision: LIFT
  was already shipped by Plan-06 phase 4d; phase 05 was a
  verification pass.  E4_d3 (closure-element tuple as struct
  field) parked behind @P251.
- 07: @P234 runtime — lifetime-bearing tuple returns route
  through `Reference(__tuple<…>)` synthetic struct.

**Phase deferred:**
- 08: @P234 runtime extended to LOCAL tuple-with-lifetime-concern
  variables — friction with P189b's vector-of-tuple index access
  meant the rewrite needed broader changes than the original
  Phase 08 scope.  Phase is a uniformity refactor, not a bug
  fix; juice not worth the squeeze.

**Bugs filed during validation:**
- @P247, @P248 — closed in phase 02.
- @P249 — closed in phase 03.
- @P250 — open: ref-tuple loop-iteration aliasing.
- @P251 — open: native projection for fn-ref-tuple-in-struct-field.

**Design decisions recorded in DESIGN_DECISIONS.md:**
- C64: Tuple struct-ref elements use MOVE semantics.
- C65: E6 (struct value) folded into E5 — no inline value-struct
  type in current loft.

Reference content moved to TUPLES.md (Known limitations + Non-
goals + Deferred work updated).  Plan moved to
`finished/14-tuple-validation/`.

### Plan-06 (typed-par redesign) closed 2026-05-09

Plan-06 ARC ran from 2026-04-30 to 2026-05-09.  All 11 sub-steps
shipped or formally deferred with rationale.

**Shipped (A1–A7 + A5b + A8.b + A9 superseded by A4 + A11):**
- A1: parallel workers — extra args + text/ref returns under one
  fused-for-par codegen.
- A2: per-thread slot cap stress test + structural fix
  (`worker_slot_dispenser` atomic counter replaces 8d.3's fixed
  16-slot cap).
- A3: Queue dispatch for narrow-primitive returns (Boolean,
  Character, Enum-no-payload, narrow Integer, Single, Float).
- A4: retired the light Concat path entirely
  (`n_parallel_for_light` and `n_parallel_for` panic if invoked).
- A5: Stitch::Reduce runtime + `par_fold(items, init, fn fold,
  threads)` parser builtin (interp + native).
- A5b: `par_fold` native runtime mirror.
- A6: closed 4 fn-ref / vector / keyed-collection canaries
  (`par_struct_to_vector_t4`, `par_struct_to_fn_t4`,
  `par_vec_of_fns_input_t4`, `par_struct_to_keyed_collection_t4`).
- A7: closed the par-tuple canary surface — A7.1 (size-based
  gate widen + work-ref unification — closes
  `par_tuple_return_int_int` / `_three_arity` / `_nested`),
  A7.2 (@P235 par half — synthesized wrapper-worker — closes
  `par_tuple_destructure_in_for`), A7.3 (@P234 lexer + runtime
  for tuple-of-struct member access).  Companion fix @P236
  (heap-owned reference returns from if/else native data
  corruption — broader than tuples) landed alongside A7.1.
- A8.b: stitch_id consolidation in `src/native.rs` — 5
  `n_parallel_queue*` fns collapsed to thin wrappers around
  `parallel_queue_dispatch(stores, stack, QueueStitch)`.
  Saves ~150 LOC.  Targets the interp-bridge layer (different
  from A8's `src/parallel.rs` target which deferred for sound
  reasons).  Codegen-runtime mirrors stay separate (closure
  types differ per stitch).
- A9: superseded by A4 (light path retired entirely; no `.loft`
  file uses `par_light`).
- A11: this entry + ARC.md status header DONE + acceptance-
  criteria final tally + THREADING.md dispatcher inventory
  section.

**Deferred with rationale:**
- A8 (Queue dispatcher trait collapse in `src/parallel.rs`):
  divergence is structural, not boilerplate.  The 4-5 dispatchers
  differ in `&Stores` vs `&mut Stores` access, worker primitive
  (`parallel_workers` vs raw rayon), per-row execute call,
  per-thread state, and merge step.  A unifying trait would
  relocate complexity rather than remove it.  Full audit in
  ARC.md A8 deferral section.  Codegen side is already collapsed
  (`ParallelQueueEmitter`); buffer stacks per-type are
  intentional (perf).  Commit `ada917d`.  A8.b stitch_id retry
  delivered consolidation at a different layer (see Shipped).
- A10 (browser parallel via wasm-bindgen-rayon): out-of-scope
  for @PLAN06 closure.  S2 strategic showcase; ships as its own
  multi-session arc when scheduled.

**Acceptance criteria — final tally:**
- #1 (≤ 3 dispatchers in `src/parallel.rs`): revised to ≤ 5;
  consolidation delivered instead at native.rs layer via A8.b.
  Documented in ARC.md acceptance section.
- #2 (par_light removed from user surface): MET by A4.
- #3 (zero ignored par canaries): 8 → 1 over the arc.  Final
  remaining ignore is heterogeneous-vec-of-fn (D11a row 8),
  outside @PLAN06 scope (different surface — vector
  construction, not par).

Three closure commits land 2026-05-09: `f974770` (closeout
docs + A8 deferral marker + A9 superseded), `15a7aab` (@P235
par half via wrapper synthesis), and the A8.b commit (this
change).

### @PLAN09 phase 07: close @P205 — bounded-generic text return scratch routing

Closes @P205 (1 of 4 native sub-failures retired).  The bug:
bounded-generic dispatch `fn f<T: Trait>(x: T) -> text` produced
native code that emitted `Str::new(&local_String)` whose pointer
referenced a stack-local that dropped at function return,
dangling the returned `Str` into freed memory.

Fix in `src/generation/emit.rs` at TWO emit sites (the dangle
isn't tied to a single Op — it's emit.rs's `Str::new(...)` wrap
choice for text-returning functions):

- **`Value::Return(val)` text-wrap path** (line 188+): detects
  "function returns Type::Text but has no
  `Type::RefVar(Type::Text(_))` attribute" and routes the value
  through `stores.scratch`.
- **Block-tail `wrap_result` path** (line 887+): same detection,
  same routing.

Detection logic:
```rust
let needs_p205_scratch = wrap_text && {
    let def = self.data.def(self.def_nr);
    matches!(def.returned, Type::Text(_))
        && !def.attributes.iter().any(|a| {
            matches!(a.typedef, Type::RefVar(ref t) if matches!(**t, Type::Text(_)))
        })
};
if needs_p205_scratch {
    write!(w, "{{ stores.scratch.push((")?;
    self.output_code_inner(w, val)?;
    write!(w, ").to_string()); Str::new(stores.scratch.last().unwrap()) }}")?;
}
```

`stores.scratch` is the same Vec<String> that
`n_parallel_buf_get_text_native` uses — lifetime stable for the
caller's use of the returned `Str`.

Detection nuance: `text_return` doesn't set `hidden=true` on the
attributes it adds (only `ref_return` does, at
`parser/control.rs:2452`).  Initial detection used
`a.hidden && Type::RefVar(...)` and filtered out every
text-returning function.  Fix dropped the `hidden` filter.

Probe finding (Outcome B from phase 07's diagnostic step):
removing the `DefType::Generic` skip at `parser/control.rs:375`
makes `text_return` run for generic specialisations but doesn't
help — text_return promotes locals to hidden RefVar(Text)
parameters, but bounded-generic specialisations have no local
text vars to promote.  The fix had to move from parser-side to
codegen-side.

Verified:
- `repro_p205.loft` exits 0 under native (was: panic on assert)
- `86_interfaces` no longer in run failures
- `native_scripts`: 89/93 → 90/93
- threading 43/43, threading_chars 35/35, issues 540/540 unchanged
- p09 fast gate: byte-identical (baseline refreshed for
  25-generics)
- fmt + clippy `-D warnings` clean

Regression tests in `tests/codegen_emitter.rs`:
- `p205_repro_passes_under_native` — runs the reproducer under
  native, asserts exit 0.
- `p205_no_str_new_of_local_in_corpus` — greps the doc-test
  baseline for `Str::new(&var___ret_*)` and fails if reintroduced.

Commit `6151231`.

### @PLAN09 phase 06: close @P202 — n_parallel_queue family in native

Closes @P202 (3 of 6 native sub-failures retired: 19_threading,
22_threading, 40_par_ref_return).  Native compilation of
`for ... par(...)` for-loops now works.

Three components ship together:

1. **Runtime fns** in `src/codegen_runtime.rs`:
   - `n_parallel_queue_native` / `_text_native` / `_ref_native`
     — queue dispatch (closure-based, mirroring
     `n_parallel_for_*_native`).  The ref variant adopts a
     single result store (simpler than the interpreter's
     per-worker dispenser); buf_drop_ref frees it.
   - `n_parallel_buf_get_native` / `_text_native` / `_ref_native`
     — per-row reads from the active par buffer.
   - `n_parallel_buf_drop_native` / `_text_native` / `_ref_native`
     — end-of-loop cleanup.
   - All 9 take `&UnsafeCell<Stores>` per phase 01's ABI;
     registered in `CODEGEN_RUNTIME_FNS` with `Abi::Cell`.
   - `# Panics` sections added to satisfy
     `clippy::missing_panics_doc`.

2. **Emitters** in `src/generation/ops/parallel.rs`:
   - `ParallelQueueEmitter` mirrors phase 03's
     `ParallelForEmitter` but routes calls to
     `n_parallel_queue_*_native` (returning i64 row count
     instead of DbRef).  Reuses `closure_shape` /
     `queue_helper_name` / extra-arg let-binding scaffolding.
   - `ParallelBufRenameEmitter` is a pass-through name
     rewriter for the 6 buf-get / buf-drop names — no closure
     transformation, just appends `_native` to the call site.
   - All 9 names registered in
     `src/generation/ops/mod.rs::build_registry`.

3. **Reachability fix** in `src/generation/mod.rs::collect_calls`:
   - The worker-fn-via-d_nr-arg detection (originally for
     `n_parallel_for*`) extended to the queue family.  Without
     this, the emitter's closure refers to a worker fn that
     never gets emitted (E0425 "cannot find function").  Caught
     during validation, not pre-flight — the lesson is captured
     in `feedback_forwarding_first_recipe.md` (extend reachability
     when emitter synthesises calls by name).

Trait-reuse decision (per phase 06 doc § Implementation notes):
Phase 09's plan-doc projected 3 thin wrappers around a
`ParQueueShape` trait following the for-par pattern.
Implementation revealed each variant pushes to a different
`par_*_buffer_stack` field with a different value type — a
trait would have ~80% conditional branching.  Kept queue
runtime fns flat (~90 LOC vs ~120 with a trait).  Codified in
`feedback_phase_doc_trait_drafts.md` ("if 3 impls share <50%
of method bodies, prefer flat over trait").

Verified:
- native_dir: 29/30 → **30/30** (19_threading compile fix)
- native_scripts: 87/93 → **89/93** (22_threading +
  40_par_ref_return compile fixes)
- threading 43/43, threading_chars 35/35, issues 540/540 unchanged
- p09 fast gate: 9/9 byte-identical (baseline refreshed for
  19-threading + 22-threading — emission shape changed
  intentionally as queue calls now route through emitter)
- fmt + clippy `-D warnings` clean

Regression tests in `tests/codegen_emitter.rs`:
- `p202_parallel_queue_runtime_fns_registered` — pins the 9
  runtime-fn names with `Abi::Cell`.
- `p202_parallel_queue_emitter_registered` — pins the 9
  build_registry entries.

Commit `8cf0676`.

### @PLAN09 phase 00a: introspection findings + downstream updates

Fired late (after phases 00, 01, 03, 04, 09 all shipped) so the
introspection retroactively covers the simplification cluster.

Findings populate `doc/claude/plans/finished/09-native-runtime-rewrite/00a-introspect.md`:

- **Effort vs estimate**: phase 00 landed in 6 commits (under
  7-9 budget) + 4 follow-on infrastructure commits (smoke test,
  evaluation gates, CI cleanup, fmt followup).  Future plans
  should budget post-step infrastructure explicitly.
- **EmitCtx surface area**: 3 planned helpers (`w` / `def_fn` /
  `output`) + 2 surfaced during phase 03 (`emit(v)`,
  `emit_i32_slot(v)`).  Phase 05's int-width helpers not added
  yet — phase 05 itself is misaligned (see below).
- **dispatch.rs op match arms**: 26 → 24 (phase 03 retired
  `n_parallel_for`; phase 04 retired `OpGetRecord` + `OpIterate`).
  Wart-budget gate `dispatch_op_arm_budget_not_exceeded` enforces
  shrink-only.
- **`Value::RawExpr` wart**: accepted with budget gate; future
  codegen synthesis must avoid extending it.
- **Byte-identical contract**: held across all 6 hoist commits +
  every subsequent simplification phase.
- **Two surprises caught and fixed**:
  - Forwarding-first recipe trap (initial 16-Op forwarding list
    included Ops in dispatch.rs special-case match) — caught by
    fast gate, list pruned to 9, recipe documented in NATIVE.md
    + phase 00 findings.  Phase 03/04 used the recipe as planned
    pre-flight.
  - Phase 09's `ParShape` sketch was too narrow (single `WorkerOut`
    type would have erased text's per-worker slot mechanism and
    ref's cross-store deep-copy capability) — implementation
    extended trait with `Self::Batches` second associated type +
    `store_results` method.

Hidden assumptions surfaced (drove downstream doc updates):

1. **Phase 05's WRITE-side scope was wrong** — actual @P200 bug is
   READ-side block-tail comparison-emission (`(_var as u8) ==
   (0_i64)` E0308), not the `f += val` template.  Plan rewrite
   required.  Documented in 05-file.md § Diagnosis findings;
   PROBLEMS.md @P200 entry updated with the surveyed shape.
2. **Phase 02 demoted from "@P200 prerequisite" to "optional
   simplification"** — phase 02 splits `narrow_int_cast`'s
   param-narrowing role (role #2); phase 05's actual bug is the
   block-tail role (role #1).  02-param-adapter.md now carries a
   "Status reassessment" block.
3. **Pre-existing CI breakage accreted** across phases 00-04 — 1
   fmt drift (hand-aligned `RuntimeFn` table from phase 01 step
   1.7) + 5 clippy errors (`map().unwrap_or()`, `borrow as raw
   pointer` ×3, lifetime elision, `let _stub = …` patterns,
   unused `Write` import).  Closed in commits f4d288a +
   97d17cc.  CI gate memory updated to "before each commit on
   hot-path edits."

Decision: **Continue with updated plans** (per the introspection's
decision criteria table — "2-3 surprises that updated downstream
phases").

Memory entries saved (durable beyond @PLAN09):

- `feedback_forwarding_first_recipe.md` — pre-flight pattern.
- `feedback_phase_doc_trait_drafts.md` — trait sketches in plan
  docs are drafts; expect to extend on first contact.
- `feedback_actual_error_survey.md` — bug-fix phases need
  `--native-emit` survey BEFORE writing implementation steps.

### @PLAN09 CI cleanup: fmt + clippy + no-default-features green

Pre-existing breakage that accumulated across phases 00-04:

- `cargo fmt --check` rejected the hand-aligned `RuntimeFn` table
  in `src/codegen_runtime.rs` introduced by phase 01 step 1.7
  (commit `2005f6e`).  Resolution: `#[rustfmt::skip]` on the
  const preserves the alignment.  Rest of the file (and 9 other
  files) reformatted to `cargo fmt` defaults.
- `cargo clippy --tests --release -- -D warnings` failed with 5
  errors (lib) / 9 (lib + tests).  All fixed:
  - `src/generation/ops/mod.rs:178` unused `Write` import — removed.
  - `src/generation/ops/mod.rs:54` explicit lifetimes on
    `EmitCtx<'a, 'b>` — elided to `EmitCtx<'_, '_>`.
  - `src/generation/ops/mod.rs:216-218` `let _stub = StubEmitter`
    pattern (binding to `_`-prefixed without side-effect) —
    rewritten as `let _: &dyn OpEmitter = &StubEmitter` (also
    strengthens the trait-impl-able assertion).
  - `src/codegen_runtime.rs:88` `.map(...).unwrap_or(...)` —
    collapsed to `.map_or(default, fn)`.
  - `src/codegen_runtime.rs:2049/2120/2209` `&mut ws.stores as *mut`
    borrow-as-raw-pointer — rewritten as `&raw mut ws.stores`
    (modern reference-to-raw idiom; clearer + clippy-clean).

Verified: fmt + clippy + no-default-features all clean; behavioural
baselines preserved (codegen_emitter 10/10, threading 43/43,
threading_chars 35/35, issues 540/540).

### @PLAN09 phase 09: parallel runtime consolidation

`src/codegen_runtime.rs`'s three `n_parallel_for_*_native` public
fns (scalar / text / heap-ref) collapsed to thin wrappers around a
generic `n_parallel_for_native_core<S: ParShape, F>(...)` core.

Mechanism:

- New `ParShape` trait with `WorkerOut: Send` + `Batches`
  associated types, `return_sz()`, `run_workers(...)` (static),
  `store_results(&self, ...)`.  Three impls: `ScalarShape`
  (carries `return_size`), `TextShape` (unit), `RefShape`
  (carries `struct_size` + `known_type`).
- The shared core sequences allocate (`alloc_par_result`) → run
  workers → store results → finalise (`finalize_par_result`).
- The three existing `run_native_workers_*` free fns stay as
  internal worker dispatchers; each `ParShape` impl's
  `run_workers` calls the appropriate one.
- Public fn bodies shrank from 36 / 24 / 39 lines to 20 / 13 / 24
  lines (full pub-fn span including signature; body is ~3 lines
  for each).  Pinned by `parallel_runtime_consolidated` test in
  `tests/codegen_emitter.rs` (≤ 15 body lines + must call
  `n_parallel_for_native_core`).
- Phase 06 (@P202 — adds `n_parallel_queue_*_native` queue variants)
  will add 3 thin wrappers (~10 lines) instead of 3 full ~80-line
  fns; cumulative saving ~240 lines.

Emission stays byte-identical (codegen calls the same public fn
names with the same ABI).  Behavioural baselines unchanged:
threading 43/43, threading_chars 35/35, issues 540/540, native
29/30 + 87/93 — same pre-existing failures (85_yield_resume,
86_interfaces, 87_store_leaks; compile failures in
19_threading / 20_binary / 22_threading / 40_par_ref_return).

### ARC.md A2: unbounded per-thread slot dispenser (8d.3 cap retired)

Replaces the spine-8d.3 fixed `SLOTS_PER_THREAD = 16` per-worker
reservation with a shared `Arc<AtomicU16>` dispenser.  Workers that
allocated more than 16 fresh stores per batch hit a hard `assert!`
in `database_named` ("worker exhausted reserved slot range
[N, N+16) at local_count=16"); the new design grows unbounded.

Mechanism:

- `Stores::worker_slot_dispenser: Option<Arc<AtomicU16>>` carries the
  shared counter into each worker's clone.  Initialised at
  `parent.allocations.len() + 1` by `Stores::make_worker_slot_dispenser`
  (the `+1` skips the parent-namespace index where each worker's
  `prog.new_state(ws)` push-at-ends a 1000-byte stack store, so the
  dispenser never collides with a worker's own stack-store slot).
- `Stores::worker_allocated_indices: Vec<u16>` per-worker list of
  parent-namespace indices the dispenser yielded.  After
  `run_parallel_queue_ref` joins, each entry triggers a
  `mem::swap` between parent and the worker's clone at that index.
- `database_named` in worker context now: pulls a fresh index via
  `dispenser.fetch_add(1, Relaxed)`, pushes
  `Store::new(100)` placeholders into the worker's own `allocations`
  Vec to fill any skipped indices owned by other workers, then
  initialises at the yielded index.  The placeholders stay
  `free=true` and are never swapped to parent (each worker only
  swaps its OWN allocated indices).
- 3 fields removed (`worker_slot_offset`, `worker_slot_limit`,
  `worker_slot_local_count`); 2 added (above).
  `reserve_worker_slots` / `release_worker_slots` removed.
- `database_named`'s `if slot == self.max { self.max += 1 }` widened
  to `if slot >= self.max { self.max = slot + 1 }` — the dispenser
  yields indices that can be > current max (it skips ahead in
  parent-namespace), so the strict-equality check missed cases and
  left max stale.

A2.3 invariant: an always-on `assert!` in `database_named` fires if
a worker has a dispenser attached but `disable_slot_reuse` was
cleared mid-call (the bypass would push to a parent-namespace index
unrelated to the dispenser, silently corrupting the swap-back at
thread join).  Always-on rather than `debug_assert!` because the
loft library compiles with `debug-assertions = false` in the test
profile per `[profile.dev.package.loft]` — a `debug_assert!` would
be a silent no-op in `cargo test`.

Tests:

- `tests/threading.rs::par_queue_ref_unbounded_allocations_per_element`
  exercises the allocator directly (bypassing the bytecode
  pipeline's `execute_at_ref` calling-convention mismatch with
  inline-struct-return functions): performs 50 named allocations
  via the dispenser, asserts strictly increasing parent-namespace
  indices and dispenser high-water = `parent_len + 1 + N_ALLOCS`.
- `tests/threading.rs::par_queue_ref_dispenser_bypass_assertion_fires`
  pins A2.3's invariant: a synthetic worker with dispenser attached
  but `disable_slot_reuse=false` panics with the documented message.

Bench-11 ±5%: ~101ms median post-A2 (vs ~98ms `main`, ~101ms
post-A1) — within gate.  All 37 `tests/threading.rs` tests + 31
`tests/threading_chars.rs` tests stay green.

ARC.md A2 status flipped to DONE.

### P196: tuple-of-fn-ref native codegen — `(u32, DbRef) as i32`

Fixes E0605 (`non-primitive cast`) + E0308 in native codegen when a
struct field of type `(fn(...) -> ..., int)` is assigned from a Var
or function-call source rather than a literal `(name, n)` tuple.

The bug: `set_field_check::Type::Tuple` non-literal path stashes the
RHS into a work-ref local, then for each element `i` emits
`OpSet*(ref, pos, TupleGet(tmp, i))`.  For a fn-ref element, native
codegen substitutes the template body's `@val` with `var_tmp.0` —
which has Rust type `(u32, DbRef)` (the fn-ref runtime
representation).  `OpSetInt4`'s template wraps `@val` with both
`@val == i64::MIN` (E0308 — comparing tuple to i64) and `@val as
i32` (E0605 — non-primitive cast on tuple type).

Fix in `src/generation/calls.rs::output_call_template`: when the
template parameter is `Type::Integer` and the IR value is a
`Value::TupleGet(var, idx)` whose tuple element type is
`Type::Function`, wrap the substituted expression with
`(i64::from(({with}).0))` — projecting the `u32` d_nr from the
fn-ref tuple's first element and widening to i64.  The template's
null-check (`== i64::MIN`) becomes tautologically false (a u32
can't equal i64::MIN) but compiles cleanly, and the `as i32` cast
narrows from i64.

The literal-tuple path was unaffected — `set_field_check::Type::Function`
already reduces `Value::FnRef(d_nr, _, _)` to `Value::Int(d_nr)`,
sidestepping the tuple shape entirely.

Tests:

- `tests/issues.rs::p4d_tuple_field_with_fn_ref` covers the literal
  case end-to-end through the interpreter (same shape as
  `p4d_fn_ref_as_struct_field`, but with the fn-ref nested inside a
  tuple field).
- `tests/exit_codes.rs::p196_native_codegen_projects_fn_ref_d_nr`
  pins the codegen-text invariant: a script with a non-literal
  fn-ref tuple source must emit `i64::from((var___ref_1.0).0)` and
  must NOT contain the buggy `(var___ref_1.0) == i64::MIN` shape.

PROBLEMS.md P196 entry retired.  ARC.md A6.c no longer gates on
P196 — closes independently of the 4d.C closure-storage redesign.

### P195: chained tuple-index lex (`n.v.0.0`)

Fixes the lexer's greedy `<digit>.<digit>` → float read when the
previous emitted token was `.` (field access).  Before: `n.v.0.0`
lexed as `n`, `.`, `v`, `.`, `Float(0.0)` — the parser then saw a
type mismatch on assignment and a stray `.` it could not place.
After: lexes as 7 tokens — `n`, `.`, `v`, `.`, `Integer(0)`, `.`,
`Integer(0)` — which is the correct chained tuple-index access.

Mechanism (`src/lexer.rs::number`):

- At entry, capture `prev_was_field_dot = self.peek.has ==
  Token(".")`.  `self.peek` holds the previously-emitted token at
  this point (the parser flow uses `cont()` which sets `peek =
  next()`-result; inside `number()`, `peek` is still the
  before-the-current-number token).
- After consuming a `.` and confirming it is **not** the start of a
  `..` range token, peek the next char in `iter`.  If
  `prev_was_field_dot && next.is_ascii_digit()`, push `Token(".")`
  onto `memory` (so the next `cont()` returns it) and return
  `Integer(val)` immediately — the trailing digit is then re-lexed
  as a fresh number on the call after that.
- The `..` range branch is unchanged: `0..5` still lexes as range,
  not tuple index.  Stand-alone floats like `0.0`, `1.5e3`, and
  expression-position floats like `x = 0.0` are unaffected because
  their preceding token is not `.`.

Test: `src/lexer.rs::test::p195_chained_tuple_index_does_not_glue_into_float`
exercises 5 cases — chained tuple index, stand-alone float,
expression-position float, mixed expression, range — using a new
`cont_array` test helper that drives the lexer through the same
`cont()` API the parser uses (the existing `array()` helper bypasses
`cont()` and would not catch context-aware lexing).

### `--show-types --trace` per-expression type tape

Adds a per-expression tape to the `--show-types` introspection
section.  Where the existing variable-level table catches dep loss
in *stored* values (the function's args, locals, return type), the
trace catches dep loss in *intermediate* sub-expressions of a
chained access.  Specifically, for a nested expression like
`a.v.0`, the tape shows the type at each step:

```
4:7        ref(A)["a"]              <- a
4:9        (text["a"], text["a"])   <- a.v
5:2        text["a"]                <- a.v.0
```

If P197 had been a regression today, line 4:9 would have rendered
`(text, text)` (no host dep) and the bug would have been visible
without reading any code.

Mechanism:
- `Parser::trace_types: bool` flag enables recording.
- `Parser::trace_types_lines: Vec<String>` accumulates entries
  formatted `<fn>\t<line>:<col>\t<type>`.
- `parse_part` calls `record_type_trace(&t)` after the initial
  `parse_single` and after each chaining step.
- Only fires on the second pass (first-pass types are placeholders
  that would emit thousands of meaningless lines).
- `main.rs` enables the flag for the user's file (not for the
  `default/*` stdlib parsed by `parse_dir`).
- `emit_types` in `introspect.rs` reads `opts.trace_lines`,
  filters to the current function (matching the user-typed name,
  i.e. without the `n_` prefix), and renders one section per fn.

Tangential fix discovered while testing on dev profile:
`emit_tuple_set_ops` had a `base_pos + offsets[i]` u16 overflow
when `base_pos` was the `database.position` u16::MAX sentinel
during first-pass placeholder resolution.  Release silently
wrapped; dev profile (with overflow checks) panics.  Switched
both arithmetic sites to `saturating_add` — first-pass IR is
regenerated in pass 2, so a saturated placeholder is safe.

Tests: `introspect_show_types_trace_renders_per_expression` in
`tests/exit_codes.rs`.

### Native-codegen source map + introspection `--diff`

Two developer-velocity wins, both targeting the long tail of
debugging time:

- **`// loft:<file>:<line>` comments in generated Rust.** Every
  function header and every statement boundary in `output_native`
  output now carries a comment mapping back to the originating
  loft source.  Lets `rustc` errors on `/tmp/loft_native.rs` be
  traced to the .loft line in seconds rather than by manually
  reading the generated code.  Cost: ~10 LOC; comments are
  cheap (one per source line).
- **`--introspect --diff <baseline>`.** Captures the requested
  sections to a buffer and runs `diff -u baseline tmp`.  Exits 0
  identical, 1 differs.  Lets devs answer "did this parser tweak
  change anything?" with one command.

Tests: `native_emit_includes_loft_source_map`,
`introspect_diff_against_baseline` in `tests/exit_codes.rs`.

### P194 — tuple-typed struct field reassignment

`p.v = (1, 2)` (where `v` is a tuple-typed struct field) used to
fail with `Tuple destructuring requires plain variable names`.
Root cause: `get_val::Type::Tuple` returns `Value::Tuple([reads])`
for a tuple field read, and the parser's destructuring branch
matched any `Value::Tuple` LHS unconditionally.  Fix: detect
"tuple of OpGet*-style reads (not all `Value::Var`) on a
`Type::Tuple` LHS" in `parse_assign` and route through
`emit_tuple_set_ops` instead of the destructuring branch.

- New helper `leaf_tuple_lhs` walks the leftmost leaf of the
  tuple-of-reads to recover `(host_ref, base_position)`; nested
  tuple elements recurse cleanly.
- `emit_tuple_set_ops` lifted to `pub(crate)` so the new branch in
  `parse_assign` can call it.

Tests: `p194_tuple_field_reassign`,
`p194_tuple_field_reassign_twice` in `tests/issues.rs`.

### P197 — returning `text` from tuple struct field corrupts memory

Surfaced while regression-testing P194.  Returning a `text` element
extracted from a tuple struct field returned garbage characters
(`.0`) or hard-crashed (`.1`, `.2`) with `ptr::copy_nonoverlapping
requires that both pointer arguments are aligned and non-null`.
Construction + read-via-print worked; only the function-return
path failed.

Root cause was two-part — both fixed in the same commit:

1. **`Type::Tuple` had no dep field**, so calling
   `.depending(host)` on a struct field's tuple type fell into the
   `_ => self.clone()` arm at `data.rs:580` and lost the host
   dependency entirely.  Fix: `depending` now recurses into tuple
   elements (each text/reference inside the tuple gets the host
   dep), and `depend()` returns the union of element deps.
2. **Native codegen materialised the tuple into a `(String,
   String)` work-var temp**, then borrowed `&temp.0` past its
   drop — `rustc` rejected with "borrowed value does not live long
   enough".  Fix: when `code` is already a literal `Value::Tuple`,
   `parse_part`'s tuple-index branch returns the indexed read
   directly instead of allocating the work-var temp.

Tests: `p197_text_returned_from_tuple_field`,
`p197_text_returned_from_tuple_field_index_one`,
`p197_text_returned_from_mixed_tuple_field` in `tests/issues.rs`.

### Plan-06 phase 4d.C step 2 — `Parts::DbRef` storage shape + new opcodes

Foundation pieces for closure storage in fn-ref struct fields and
tuple elements.  No user-visible behaviour change yet — the
parser still emits the truncated 4-byte `OpSetInt4` path, which
phase 4d.C step 4 will replace with the new opcodes.

**Database:**

- New `Parts::DbRef` variant in `src/database/mod.rs` — 12-byte
  raw `DbRef` storage cell (`u32` store_nr + `u32` rec + `u32`
  pos).  Match arms wired through `database/io.rs`,
  `database/structures.rs`, `database/format.rs`, and
  `database/search.rs`.  Non-collection operations panic;
  debug-format renders as `DbRef(s,r,p)` or `null` (rec == 0).
- `Stores::dbref()` registers a primitive type named `"dbref"`
  with `Parts::DbRef` and size 12 (idempotent).

**Opcodes (`default/01_code.loft`):**

- `OpSetDbRef(v1: reference, fld: const u16, val: reference)` —
  writes 3 × `set_u32_raw` words at `v1.pos + fld`.
- `OpGetDbRef(v1: reference, fld: const u16) -> reference` —
  reads 3 × `get_u32_raw` words and assembles a `DbRef`.
- OPERATORS array in `src/fill.rs` grown 243 → 245.  Interpreter
  dispatch fns `set_db_ref` / `get_db_ref` regenerated via
  `cargo test regen_fill_rs -- --ignored`.

### Slot allocator & frame layout (plans 04 + 05)

Two companion plans closed together; user-visible only as the
absence of a recurring heap-corruption class (P178 / P185).

**Runtime / codegen changes:**

- Single function-entry `OpReserveFrame(frame_hwm)` replaces the
  per-block `OpReserveFrame(block.var_size)` + `OpFreeStack`
  bookkeeping.  The whole frame is owned by the function and
  released on return.
- Positional init opcodes: `OpInitText(pos)`, `OpInitRef(pos)`,
  `OpInitRefSentinel(pos)`, `OpInitCreateStack(pos, dep_pos)`.
  Every first-assignment writes directly to the allocator-chosen
  slot; slot-move + gap-fill in `gen_set_first_at_tos` is gone.
- `OpText` deleted (−1 opcode).  The three compound ops
  `OpConvRefFromNull` / `OpNullRefSentinel` / `OpCreateStack`
  remain as dictionary-only entries for parser back-compat; their
  runtime bodies are dead code.
- `place_orphaned_vars` deleted (~150 LOC retired).  `process_scope`
  + `place_large_and_recurse` now reach every local: Insert-rooted
  function bodies, cross-scope `Set` in child operator lists, and
  the `BreakWith / Iter / Tuple / TuplePut / Yield / Parallel`
  IR shapes are all handled in the main walk.
- P185 fixed (`p185_slot_alias_on_late_local_in_nested_for` +
  `p185_late_local_after_inner_loop` un-ignored).

**Diagnostics:**

- Invariant **I7 — scope-frame consistency** in
  `src/variables/validate.rs`: each variable's `stack_pos` lies
  within its declared scope's frame region.  Converts the
  `Incorrect var X[slot] versus TOS` runtime panic into a
  compile-time `[I7]` diagnostic.
- V2 allocator (`src/variables/slots_v2.rs`) remains as a shadow
  validator invoked via `LOFT_SLOT_V2=validate`; I1–I6 green on
  the corpus as a correctness gate for future V1 edits.

**Retracted from the original @PLAN04 scope:**

- Single-pass V2 allocator driving codegen (both the
  codegen-is-allocator pivot and the direct V2-drive attempt hit
  the same failure mode on variables declared at outer scope but
  first-Set in inner scope).  V1 continues to drive codegen.

### Integer → i64 migration (Phase 2c)

`integer` is now 8 bytes end-to-end — on the stack, in struct
fields, in runtime arithmetic — across the interpreter, native
codegen, and WASM backends.  Arithmetic that used to silently
wrap at `i32::MIN / MAX` now traps (Phase 1 `?` / `??` dispatch
from `925ee36`) or round-trips correctly on i64.

**What users see:**

- `integer` literals beyond `i32::MAX` (e.g. `9_876_543_210`)
  type-check without any suffix.
- The `long` type keyword and the `l` literal suffix (`33l`,
  `0xFFl`) are **gone**.  Writing `long` in a type position now
  fails with `"Undefined type long"`; writing `33l` fails at the
  lexer.  Use `integer` and plain `33` instead.  Both were
  deprecation-warned in 0.9.0-early and fully removed in
  0.9.0-final (commits `3e976b3`..`0c46abb`).
- Narrow integer aliases — `u8`, `u16`, `i8`, `i16`, and `i32`
  — keep their compact field storage (`Parts::{Byte, Short,
  Int}`), with narrow↔wide conversion at read/write.  Pack
  density is preserved for image buffers, pixel arrays, and
  other bit-bounded data.
- File I/O for binary formats now **requires an explicit
  width cast** on scalar integer writes, e.g.
  `f += 2 as i32;` (4-byte GLB version), `f += 0 as u8;`
  (1-byte pixel).  Pre-2c `f += 2` wrote 4 bytes; post-2c
  writes 8 — silent regressions in existing binary writers
  are the most common footgun of this migration.

**Migration aid:** no external users of pre-0.9.0 loft exist,
so no migration path is needed in practice.  The internal
`loft --migrate-long <path>` CLI is retained as a utility
that rewrites `long` → `integer` and strips `l` suffixes, in
case an external user surfaces later.

**Downsides recorded** (`doc/claude/CAVEATS.md`): memory
footprint of integer-heavy data structures roughly doubles;
cross-crate cdylib packages keep 4-byte `vector<integer>`
element storage (narrow→wide conversion at the FFI boundary).
The bytecode opcode table was reduced from 268 to 234 after
the `Op*Long` family dedup (34 opcodes reclaimed across rounds
10b.1–10b.4 and 10d).

### JSON support

Loft now has built-in JSON parsing and generation.

**Parsing** — `json_parse(text)` turns a JSON string into a typed
`JsonValue` that you can inspect and navigate:

```loft
v = json_parse("{{\"name\":\"Alice\",\"age\":30}}");
println(v.field("name").as_text());   // Alice
println(v.kind());                     // JObject
println(v.to_json_pretty());           // formatted output
```

Bad input returns `JNull` instead of crashing; call `json_errors()`
to see what went wrong (with line numbers and context).

**Reading values** — `field("key")`, `item(index)`, `len()`,
`has_field("key")`, `keys()`, `fields()`, `kind()`.
Type extractors: `as_text()`, `as_number()`, `as_long()`, `as_bool()`.

**Writing JSON** — `to_json()` for compact output,
`to_json_pretty()` for readable indented output.

**Building values from code** — `json_null()`, `json_bool(v)`,
`json_number(v)`, `json_string(v)`, `json_array(items)`,
`json_object(fields)`.

**Struct integration** — `MyStruct.parse(json_value)` populates
a struct from a JsonValue. Type mismatches are reported via
`json_errors()`.

### Plan-06 phases 4c + 4d.A — typed parallel-for dispatch

Two coupled phases of @PLAN06 ("simple typed `par`: everything is a
store") landed.  User-visible only as one extra par canary closing
(`tests/threading_chars.rs::par_tuple_input_int_int`); structurally
this lays the foundation for the remaining phase 4 work (4d.B
keyed-collection input materialisation, 4e caller-supplied
destinations).

**Phase 4c (DESIGN.md D1b):** `Stitch::ConcatLegacy { elem_size,
ret_size }` retired in favour of payload-free `Stitch::Concat`.
`parallel_execute_and_collect` now takes `dispatch_mode:
DispatchMode` and routes via the `Text / Ref / Primitive` arms
keyed on the caller-supplied dispatch mode.  `grep ConcatLegacy
src/` returns zero (spec acceptance).  Opcode payload shrinks 2
bytes per call.

**Phase 4d.A:** typed worker-input dispatch via `InputKind` enum
(`Ref / Text / Primitive { size: u8 }`) with a 64-byte cap on the
`Primitive` slot.  New `read_primitive_at_wide` (stack-allocated
`[u8; 64]` reader) and `execute_at_raw_primitive_input_wide`
(byte-chunk push) handle 9..=64 byte first-arg slots — tuples,
fn-refs, and any inline-typed first arg whose stack representation
exceeds 8 bytes.  Both `run_parallel_direct` and
`run_parallel_light` got matching `prim_in > 8` arms.  Retires
the sentinel-encoded `primitive_first_arg_slot_size` channel.

### Local-var keyed collection iteration (P190)

`for x in <local sorted/hash/index>` used to panic at
`src/state/codegen.rs:1689` with "Too few parameters on
OpIterate (got 2, need 6)".  P188 enabled local-var keyed
collections but the iteration codegen path's
`src/parser/vectors.rs::get_type` only resolved the database
type-name for fields registered via `fill_database` — local-var
keyed collections never reached that registration path, so the
lookup returned `u16::MAX`, `fill_iter` exited early, and
`OpIterate` got 2 args instead of the 6 it needed.

Fix: register the type on demand in `get_type` when the name
lookup misses, mirroring `fill_database`'s `database.sorted` /
`database.hash` / `database.index` calls.  Idempotent — same
content+keys → same type id.  Regression test
`tests/issues.rs::p190_local_var_sorted_iteration`.

Note: this unblocked the iteration codepath; @PLAN06 phase
4d.B for sorted then closed by the parser-side desugar (see
the next entry).

### Plan-06 phase 4d.B sorted — par-over-keyed-collection materialise

`for s in sorted_items par(...)` now compiles end-to-end and
closes the `par_sorted_input_t4` canary (1 more canary
green; 11 ignored, was 12).

When parse_for sees a par() clause with a sorted/hash/index/
spacial input, the new `materialise_keyed_for_par` helper
allocates a temporary `vector<reference<T>>`, walks the
source via the existing `OpIterate`/`OpStep` machinery (the
same helpers `for x in sorted_items` uses), and appends each
element-DbRef.  The par dispatch then runs over the
materialised vector — workers receive the same 12-byte
DbRef as the closed `par_vec_of_refs_input_t4` canary.

The IR shape mirrors the parser's emission for the manual
workaround `refs += [s]`: `OpPreAllocVector` +
`OpNewRecord` + `OpCopyRecord` + `OpFinishRecord` per loop
iteration.  An earlier attempt missed `OpPreAllocVector`
and produced uninitialised slots; this commit lands the full
sequence.

Cost contract: O(N) materialisation + 12-byte temporary
vector + the par work itself.  Documented as known cost;
users can opt out by materialising explicitly into
`vector<reference<T>>` first.

`par_hash_input_t4` and `par_index_input_t4` stay
`#[ignore]`d on **P191** (filed in PROBLEMS.md) —
sequential local-var iteration over hash/index produces
wrong elements (0 for index, 195 instead of 30 for hash).
After P191 closes, both canaries should pass via the same
4d.B desugar.

Regression test:
`tests/issues.rs::p4d_b_par_over_sorted_via_materialise`.

### P191 — `index<T[key]>` bookkeeping field size mismatch

`database.index` appended `#left_N` / `#right_N` bookkeeping
fields declared as 8-byte `integer`, but `tree::add` writes
those tree pointers via `set_i32_raw` at hardcoded offsets
`[pos, pos+4, pos+8]` (RB_LEFT=0 / RB_RIGHT=4 / RB_FLAG=8).
Alignment-aware packing placed the 8-byte fields 8 bytes
apart, so tree pointers landed in the wrong record bytes.
Iteration only returned the root element (e.g., a struct-
field index iteration that should sum 60 returned 10).

Fix: switch bookkeeping to 4-byte `int<0,false>` so the
layout matches `tree::add`'s offsets.  Side benefit: indexed
records shrink by 8 bytes each.  `tree.rs` already operates
exclusively on i32 via `set_i32_raw` / `get_i32_raw`; no
other code changes.

Same commit also adds new `validate_layout` /
`validate_all_layouts` / `debug_layout` / `layout_summary`
helpers in `src/database/types.rs`, wired into the parser
flow after `database.finish()` so future regressions surface
as build-time errors.  16 unit tests cover overlap detection,
beyond-size, bookkeeping-offset mismatch, enum-variant
overlap-within-variant, and the layout-summary format.

Regression test:
`tests/issues.rs::p191_struct_field_index_iteration_after_layout_fix`.

### P192 — `len()` for `hash<T[key]>` and `index<T[key]>`

Only `vector` and `sorted` had `len()` overloads.  Added
two new runtime helpers — `hash::count` (walks the bucket
array, O(room)) and `tree::count` (walks via `first` +
`next`, O(n)) — exposed via `OpLengthHash` (normal stdlib
overload) and `OpLengthIndex` (parser hook in `call()` to
inject the bookkeeping-offset const).

Regression tests:
`p192_len_hash_struct_field`,
`p192_len_index_struct_field`.

### P188 follow-up — `field += elem` for keyed-collection fields

Two distinct bugs broke `db.x += Foo{...}` for hash / sorted /
index / spacial fields and local-vars; vector-literal init
(`db = Db { x: [...] }`) worked because its codegen built
records directly.  Both surfaced once P192's `len()` made
the broken state observable.

**Bug 1 — struct-literal RHS retarget.**  `Score{name: "a",
value: 10}` parses with the LHS field as its target, so the
field-init steps wrote into the field's storage —
overwriting the hash/index root pointer with stray bytes of
the score record.  Struct-field hash with `+=` reported
`len = 11` after one add then SIGSEGV on the next.

Fix: extend the `field += elem` branch in `expressions.rs` to
also match keyed-collection fields with struct-literal RHS,
allocate a fresh element via `new_record_field_op`, and walk
the parsed steps with a new `substitute_value` helper that
replaces the LHS field expression with `Var(elm)` so each
field write lands in the new record.  Gated on
`elm_tp.is_equal(&s_type)` so vector field `+= [1, 2, 3]`
(multi-element append) keeps its existing OpAppendVector path.

**Bug 2 — local-var dispatch via wrong db type.**  `new_record`
local-var branch looked up the keyed-collection's known_type
via `data.def(type_def_nr(lhs_tp)).known_type`, but
`type_def_nr` returns the GENERIC alias (`hash` / `index`),
not the specific `hash<Score[name]>` instantiation.  The
alias's known_type pointed at a Vector type, so
`record_finish` dispatched through `Parts::Vector` and
appended raw bytes — `hash::add` / `tree::add` never fired.
Local-var hash with 3 adds showed 6 records (vector_finish
appends without dedup); local-var index with 2 adds showed 1
(tree::add was bypassed entirely).

Fix: register the specific keyed-collection db type directly
(`database.hash(c, key)` / `index(c, key)` / etc.) —
idempotent with the gen_set_first_keyed_null and typedef-
walker registrations.

4 new P188 regression tests cover struct-field and local-var
hash and index `+=` (each asserts both `len` and the
iteration sum).

### Plan-06 phase 4d.A.2 — partial fix: parser hang eliminated, clean diagnostic emitted

A 2026-04-27 spike landed two contained changes that flip the
canary's failure mode from "infinite-loop in parser, requires
`pkill`" to "fast clean diagnostic, 0.02 s test failure".

**Root cause (parser hang)**: `src/parser/definitions.rs::sub_type`
had no `fn` keyword arm.  When the parser saw
`vector<fn(integer) -> integer>`, sub_type's identifier-only check
rejected `fn`, the lexer reverted past `<`, and the caller's
annotation parser (`expressions.rs::parse_assign:1009`) entered a
tight retry loop on the unconsumed `<`.  The loft binary's `--dump`
flag and `cargo test` both hung at 100% CPU during pass 1 of the
2-pass parser.

**Fix #1 — parser sub_type**: new `fn` arm in `sub_type` that
consumes the `fn(...) -> ...` declaration via `parse_fn_type`,
then emits a clean diagnostic and returns `Type::Unknown(0)` until
full storage support lands.  The parser advances cleanly instead
of looping.

**Fix #2 — vector literal new_record**: `parser/vectors.rs::new_record`
checks for `Type::Function` element type at entry and emits a
clean diagnostic with a workaround suggestion ("wrap the fn-ref in
a struct") instead of hitting the cryptic
`assert_ne!(ed_nr, u32::MAX)` assertion downstream.

**Tests pass**: `threading_chars` 31/0/8, `threading` 16/0/0,
`issues` 522/0/4 (the +3 ignored are diagnostic regression guards
documenting V1/V2/V3 reduced cases).  `cargo clippy` clean.

**Canary remains `#[ignore]`d** — full closure of the canary needs
real storage support for `vector<fn-ref>`, which is its own
@PLAN06 phase 4d.A.2 work (M effort, 2-3 days).  See
`/home/jurjen/.claude/plans/serialized-churning-journal.md` for
the full design (Steps A–E).

### Plan-06 phase 4d.A.2 — partial fix: parser hang eliminated, runtime cascade exposed

A 2026-04-27 spike landed three contained changes that flip the
canary's failure mode from "infinite-loop in parser, requires
`pkill`" to "fast SIGSEGV in runtime, 0.02 s test failure".

**Root cause (parser hang)**: `src/parser/definitions.rs::sub_type`
had no `fn` keyword arm.  When the parser saw
`vector<fn(integer) -> integer>`, sub_type's identifier-only check
rejected `fn`, the lexer reverted past `<`, and the caller's
annotation parser (`expressions.rs::parse_assign:1009`) entered a
tight retry loop on the unconsumed `<`.  The loft binary's `--dump`
flag and `cargo test` both hung at 100% CPU during pass 1 of the
2-pass parser.

**Fix**: new `fn` arm in sub_type that calls `parse_fn_type` and
registers a synthetic `__fn_ref` global struct via the new
`Data::fn_ref_def` helper.  Mirrors the tuple_def pattern (P189):
one global struct shared across all fn-ref shapes, since all
fn-refs have the same vector-storage shape (4-byte i32 d_nr).
`type_def_nr` and `type_elm` get matching `Type::Function` arms
returning the `__fn_ref` def's number.

**Generated-code diagnosis**: with parsing fixed, the test
framework wrote `tests/generated/threading_chars_par_vec_of_fns_input_t4.rs`
for the first time.  Reading it reveals 3 remaining bugs:

1. **`n_apply` empty match** — native codegen specialises
   `OpCallRef` to `match var_f.0 { ... }` over statically-known
   d_nrs.  For `apply(f)` where `f` flows from a generic vector,
   no analysis populates the arms — only `_ => unreachable!()`
   remains.
2. **Vector literal as struct-records** — my `__fn_ref` synthetic
   struct routed `[dbl, triple, quad]` through the
   `OpNewRecord/OpCopyRecord/OpFinishRecord` STRUCT-element
   vector path.  Each fn-ref becomes a heap record with a `d_nr`
   field; vector stride is the record size, not 4.  The
   interpreter SIGSEGVs reading back struct-DbRefs into a worker
   slot expecting flat 4-byte d_nr bytes.
3. **Par dispatch closure type-mismatch** —
   `|stores, elm: DbRef| { n_apply(stores, elm) as i64 }` but
   `n_apply` takes `(u32, DbRef)`.  Dispatcher needs a
   `Type::Function` worker-input arm that reads the 4-byte d_nr
   and constructs the tuple.

**Remaining work to fully close 4d.A.2** (effort: M, 2-3 days):

- Re-design `__fn_ref` as a primitive 4-byte alias (drop struct).
- Vector element-write flat-byte arm in `parse_append_vector`.
- Vector read-back unbox in `parser/fields.rs` (P189b-style).
- Par dispatcher worker-closure `(u32, DbRef)` wrap in
  `src/generation/dispatch.rs:792-870`.
- Native codegen — populate match arms or fallback to interpreter.

Tracked in `/home/jurjen/.claude/plans/serialized-churning-journal.md`.

The canary remains `#[ignore]`d but with an updated message naming
the new failure mode (SIGSEGV instead of hang).

### Plan-06 phase 4d.A.2 — investigation: vec-of-fn-refs is bigger than estimated

A 2026-04-27 spike attempted to close `par_vec_of_fns_input_t4`
by un-ignoring the canary and observing the failure.  Result:
**the worker infinite-loops** rather than failing cleanly.

The README's planned fix ("per-row synthesis of the 12-byte
null closure DbRef") turns out to only address half the gap:

- In-vector storage: 4 bytes per row (just the d_nr stored as i32 —
  `data::element_size(Type::Function) = 4`).
- Worker arg slot: 20 bytes — 8B i64 d_nr + 12B closure DbRef
  (`variables::size(.., Context::Argument) = 20`).

The current wide-input dispatcher (`read_primitive_at_wide`)
reads `element_size = 4` bytes into a 64-byte zero-initialised
buffer, then `execute_at_raw_primitive_input_wide` slices to
`prim_in = 20` bytes.  Slot bytes 4-7 are zero (high 32 bits of
i64 d_nr — fine for any practical d_nr) and bytes 8-19 are zero
(null closure DbRef).

The resulting fn-ref **runs** but `apply(f) → f(10)` loops
indefinitely, suggesting the call-dispatch path (likely
`OpCallRef`) doesn't tolerate a null closure DbRef in this
context — possibly because it interprets `store_nr=0, rec=0`
as a back-pointer to itself, or because the worker's stack
state after `OpCallRef` is wrong without a real closure.

Closing 4d.A.2 needs:

1. A `read_fn_ref_at_wide` helper that explicitly handles the
   i32→i64 d_nr widening (rather than relying on flat memcpy
   into a zeroed buffer).
2. A runtime fix to the `OpCallRef`-on-null-closure path so
   workers don't loop when the closure DbRef is null.
3. A new wide-input plumbing channel similar to
   `tuple_input_types: Option<Vec<Type>>` from P189d — likely
   generalised to `WideInputLayout::{Tuple, FnRef, Plain}`.

Effort revised: **S–M** (was S).  Test-side guard added: the
canary's `#[ignore]` message now warns "DO NOT un-ignore
without fixing — the test infinite-loops and needs `pkill` to
terminate."

### Plan-06 phase 3b.1 — extract shared `merge_batches` helper

Five sites across `src/parallel.rs` and `src/codegen_runtime.rs`
inlined the same 5-line loop after every `parallel_workers`
call: pre-fill a `Vec<R>` with a default value, then walk each
`(start, batch)` pair and write each element into
`results[start + offset]`.

Extracted to `parallel::merge_batches<R: Clone>(batches, n_rows,
default) -> Vec<R>` and applied at:

- `parallel::run_parallel_raw` (Vec<u64>, default `0u64`)
- `parallel::run_parallel_text` (Vec<String>, default `String::new()`)
- `parallel::run_parallel_int` (Vec<i64>, default `i64::MIN` — null sentinel)
- `codegen_runtime::run_native_workers_primitive` (Vec<i64>, `0i64`)
- `codegen_runtime::run_native_workers_text` (Vec<String>, `String::new()`)

Net retire ~25 lines.  The helper accepts the default as a
parameter rather than `R: Default` so the int variant can keep
its `i64::MIN` null sentinel and the text variant can document
the empty-String seed explicitly.

### Plan-06 phase 3b.1 — extract shared par result store helpers

Three native par fns (`n_parallel_for_native`,
`n_parallel_for_text_native`, `n_parallel_for_ref_native`)
shared two identical 7- and 10-line boilerplate blocks for
allocating + finalising the result store.

Extracted to two helpers in `src/codegen_runtime.rs`:

- `alloc_par_result(stores, n, elem_size) -> (DbRef, u32, u32)`
  — allocates the result store, claims the vector body
  (`n * elem_size` bytes) and the 1-word header record, returns
  (result_db, vec_rec, header_rec).
- `finalize_par_result(stores, result_db, n, vec_rec, header_rec) -> DbRef`
  — writes the vector length into `vec_rec[4]`, points the
  header record at the vector, returns the canonical
  `DbRef { …, pos: 4 }` every par caller expects.

Each native par variant now opens with one helper call and
closes with another instead of inlining the boilerplate.  Net
removal: ~30 lines.  No API change — all 30 generated test
fixtures in `tests/generated/threading_chars_par_*.rs` still
match.  Sets up phase 3b.2 (true unification with a `Stitch`
trait).

### Plan-06 phase 1 — clippy gate restored on threading build

`cargo clippy --release --all-targets` was failing on the
default (threading) build with two `not_unsafe_ptr_arg_deref`
errors:

- `state::execute_at_raw_to(dst: *mut u8)` (added by
  @PLAN06 phase 1 G4 / 4d.A in commit 6973b182) was a public
  function that called `ptr::copy_nonoverlapping` without an
  `unsafe` signature.  Now `pub unsafe fn` with a `# Safety`
  doc-comment block; the single caller in
  `parallel.rs::run_parallel_direct` wraps the call in
  `unsafe { … }` with a SAFETY comment naming the slot
  pre-allocation invariant.
- `parallel::run_parallel_direct(out_ptr: *mut u8)` (added by
  4b90d89a) had a `cfg_attr(not(feature = "threading"), allow(
  not_unsafe_ptr_arg_deref))` that suppressed the lint only on
  the WASM-style build.  The attached comment explained the
  reasoning ("making the public function `unsafe` would cascade
  across every par(...) call site and the QUALITY 6a native-
  codegen path") — applied to both builds, so the allow now
  hoists out of the `cfg_attr` and the `cfg_attr` keeps only the
  feature-specific `needless_pass_by_value` + `dead_code`.

`make ci`'s clippy step is green again on the default build.

### P189b / P189d — `vector<(T1, T2, …)>` access closed end-to-end

Two follow-ups to P189 / P189c that closed the remaining
read-side gaps for tuple-element vectors.

**P189b — index-access + for-loop iteration unbox.**

`pairs[0]` returns a `DbRef` into vector storage; the existing
`OpTupleGet(slot, byte_offset)` reads from a *local slot*, so it
decoded the DbRef bytes (`store_nr | (rec << 32)`) as if they
were the tuple's first element.  For-loop iteration hit a
matching shape mismatch and reported "Field access not supported
on type tuple([…])".

Fix: when the tuple value lives in vector storage, the parser
unboxes via the synthetic `__tuple<…>` struct.

- `parser/fields.rs::unbox_tuple_from_dbref` — for `p = pairs[i]`,
  emits per-element loads (`OpGetInt`, `OpGetText`, …) against
  the DbRef and packs the results into a `Value::Tuple` so the
  assignment target receives the inline-on-stack representation.
- `parser/control.rs` for-loop iteration — re-types the loop
  variable as `Reference(__tuple<…>)`, so `p.0` / `p.1` route
  through `parse_part`'s new `__tuple<` arm, which calls
  `get_val(elem, …, offset, …)` (struct-field-style access)
  instead of the stack-tuple `OpTupleGet`.
- `parser/collections.rs::for_iter` — keeps the iterator's
  block-result type aligned with the loop variable's `RefVar(Tuple)`,
  so the next-expression yields the 12-byte DbRef the body expects.

**P189d — text-element worker arg inflation.**

After P189c made `(int, int)` tuple-input workers wide-input
correct, `(int, text)` workers still saw `len(p.1) == 0`.  The
in-vector tuple stores text as a 4-byte interned-pointer; the
worker's argument slot expects the full 16-byte `Str` (8B ptr +
8B len).  `read_primitive_at_wide`'s flat memcpy left the upper
12 bytes of the `Str` slot zero.

Fix: per-element reader.

- `parallel.rs::read_tuple_at_wide(stores, row_ref, elem_types)`
  — walks the tuple element types, copies primitives by memcpy
  and inflates `Text` fields by reading the heap pointer and
  reconstructing a `Str` via `store.get_str(...)`.
- `native.rs::tuple_first_arg_types(def)` — extracts
  `Some(elems)` when the worker's first argument is a tuple,
  else `None`.  Threaded through both `n_parallel_for` (heavy
  path) and `n_parallel_for_light`, then through
  `parallel_execute_and_collect` /
  `parallel_light_execute_and_collect` to the underlying
  `run_parallel_direct` / `run_parallel_light` calls.
- `parallel.rs::run_parallel_direct` and `run_parallel_light` —
  new `tuple_input_types: Option<Vec<Type>>` parameter.  When
  `Some`, the wide-input branch routes through
  `read_tuple_at_wide` instead of `read_primitive_at_wide`; both
  the threaded and sequential branches.  The parameter is
  Arc-wrapped per-call so worker threads share a cheap clone.

**Native codegen header.**

`generation/mod.rs` now emits `use loft::hash;` and
`use loft::tree;` alongside the existing `loft::ops` /
`loft::vector` imports.  P192's `OpLengthHash` (`hash::count`)
and `OpLengthIndex` (`tree::count`) `#rust` templates referenced
the bare module names — without the imports, any program that
reaches `len(h)` / `len(ix)` failed native compilation with
`error[E0433]: cannot find module or crate "hash"`.

**Tests:** `par_tuple_input_int_text` un-`#[ignore]`d;
`p189b_vector_tuple_for_loop_int_int` and
`p189b_vector_tuple_for_loop_int_text` added to `tests/issues.rs`
(the existing index-access tests already cover P189b's first
half).

### P193 — eager init for `local: keyed_collection<T> = []`

`gen_set_first_keyed_null` (P188's local-var alloc) fired
lazily on first WRITE.  When that first write was inside a
`for` loop body, the OpInitRef + OpDatabase init bytecode
landed inside the loop body — every iteration zeroed the
collection's root pointer.  Symptom:
`for i in 0..N { ix += ... }` over a local-var keyed
collection left `len(ix) == 1` (only the last add) and leaked
N stores.  Reading the collection BEFORE any write also
panicked with `Incorrect var ix[65535] versus N`.

Two fixes in concert:

1. **Eager init via parser rewrite** (`parser/operators.rs::create_keyed`).
   When the parser sees `Set(v, Insert(empty))` for a
   keyed-collection local, rewrite to `Set(v, Null)` so
   codegen's existing `gen_set_first_keyed_null` arm fires at
   the declaration's statement position (outside any
   enclosing loop body).

2. **Scope-exit free** (`data.rs::heap_dep` and
   `scopes.rs::get_free_vars`).  Recognise Sorted / Hash /
   Index / Spacial as heap-owned (they each get a fresh
   OpDatabase store on init), so the scope-exit OpFreeRef
   pass emits cleanup for them.  Without this the store
   leaked on program exit ("Stores not freed at program exit:
   N(bc:M)").

3 new P193 regression tests cover loop-form add (index +
hash) and read-before-write.

### Plan-06 phase 4d.B hash + index — closed by P191/P192/P188

`par_hash_input_t4` and `par_index_input_t4` un-`#[ignore]`d
and pass once the underlying P191/P188 fixes landed: the same
4d.B materialise-then-route desugar that closed
`par_sorted_input_t4` extends to hash and index automatically
once the local-var keyed-collection iteration and `+=` paths
are correct.  Phase 4 partial → 4d.B fully done; remaining
phase 4 work: 4a (typed-arity declaration), 4b (5-arg form
retirement), 4e (caller-supplied destination via ref_return).

### Vector-of-tuple support (P189 / P189c)

`vector<(T1, T2, …)>` now parses, constructs, and serves its
elements correctly via the par worker path.  Previously the type
was rejected at parse, then panicked at construction, then read
garbage — three layers fixed jointly:

- `src/parser/definitions.rs::sub_type` accepts `(...)` as the
  inner type of `vector<T>` / `iterator<T>`.
- `src/data.rs::tuple_def(lexer, types) -> u32` registers a
  synthetic struct (`__tuple<T1,T2,…>`) with attributes `_0, _1,
  …` typed per the tuple element.  Idempotent — same shape →
  same def_nr.  `Type::Tuple` arms in `type_def_nr` and
  `type_elm` look up the registered struct.
- `src/parser/vectors.rs::new_record` got a `Value::Tuple(values)`
  arm that emits per-attribute `set_field(tuple_struct_d_nr, i, 0,
  elm, values[i])` calls, mirroring the struct-literal path's
  per-field writes (which are pre-emitted via Value::Insert).

**Open follow-ups documented in PROBLEMS.md:**
- P189b: `pairs[0].0` (DbRef-aware tuple field access) reads the
  DbRef bytes as inline tuple — needs heap-tuple unboxing opcodes.
- P189d: `vector<(integer, text)>` text element reads as
  zero-length — text has different in-vector (4-byte pointer) vs
  on-stack (16-byte Str) representation; read path needs to
  inflate.

### Local-var keyed collections (P188)

`sorted<T[key]>`, `hash<T[key]>`, `index<T[key]>`, and
`spacial<T[key]>` now work as locals; previously they were only
usable as struct fields.  Patterns like

```loft
fn build() -> sorted<Tag[id]> {
    out: sorted<Tag[id]> = [];
    out += Tag { id: 1, label: "v1" };
    out
}
```

used to crash at runtime with an out-of-bounds `mut_store`
because the slot allocator gave `out` a position but neither the
bytecode codegen nor the native generator emitted the
`OpDatabase` init that allocates the backing store and zeroes
the root pointer.  Both paths now allocate the backing record on
first assignment, and subsequent `+= T {...}` operations grow the
collection in place via `record_new`'s
`Parts::Sorted/Hash/Index/Spacial` dispatch.

### Crash fixes

Three crashes that affected programs using `match` on complex types
are now fixed:

- **Character interpolation** — returning `"{c}"` from a function
  no longer crashes. The generated code now correctly handles
  writing to the caller's text buffer.
- **Recursive match on struct-enums** — `match` arms with different
  amounts of local variables (e.g. a simple `Leaf` arm vs. a
  complex `Node` arm with a for-loop) no longer corrupt the return
  address. Both arms now exit at the same stack level.
- **Memory leaks on chained calls** — `json_parse(t).field("x")`
  and similar chains no longer leak memory. The compiler now tracks
  which native functions create new values vs. which ones borrow
  from their input.

### New CLI flag: `--dump`

`loft --dump file.loft` compiles your program and prints the
internal bytecode to stderr — without running it. Useful for
debugging compiler issues. Combine with `LOFT_LOG` for extra
detail:

```bash
LOFT_LOG=variables loft --dump file.loft   # include variable table
```

### WASM / browser improvements

- The `--html` export now correctly compiles programs that call
  text-returning methods (like `kind()`, `to_json()`, `as_text()`).
  Previously this produced a type error during WASM compilation.
- The WASM build is now a release-blocking gate — if the browser
  path breaks, the release is held.

### Brick Buster game

The built-in arcade game got a polish pass: heart-shaped lives,
hand-designed levels 1-5, three original chiptune music tracks,
balloon powerups, screen shake effects, fire-ball trails, high
score persistence, and faster ball/paddle speed.

### Other improvements

- **Crash reporter** — when the interpreter hits a fatal error, it
  now prints which function and instruction caused the crash before
  exiting. Makes bug reports much more useful.
- **Parallel blocks** — `parallel { }` now uses real OS threads.
- **WebGL gallery** — 24 graphics demos running in the browser.
- **HTTP server/client** — blocking HTTP in the `web` package.
- **Playground** — better syntax highlighting, categorized examples,
  assert results shown with checkmarks.
- **Test runner** — `scripts/find_problems.sh --bg` runs the full
  test suite in the background; `--peek` to check progress,
  `--wait` for the summary. Stale caches are cleaned automatically.

### Native Moros editor

A native OpenGL editor for the Moros hex RPG now ships as a standalone
application, independent of the browser shell:

- **Entry point:** `lib/graphics/examples/moros_editor.loft` — run with
  `loft --native --path . lib/graphics/examples/moros_editor.loft`.
- **Fullscreen support:** new `gl_create_fullscreen_window` API; F11
  toggles fullscreen at runtime.
- **Input:** scroll-wheel events + expanded key codes (Home, End,
  PageUp/Down, F1–F12, arrow modifiers) now reach loft programs.
- **Panel UI overlay:** 2D panel drawn after the 3D scene pass;
  `editor_click` routes mouse events to the correct panel or 3D pick.
- **Standalone packaging:** `make editor-dist` produces a self-contained
  `dist/moros-editor/` directory; the binary runs on a machine without
  `loft` installed.
- **Native codegen fix:** functions that reconstruct constants
  (const_refs) now compile correctly under `loft --native`.  This was
  the sole native-codegen regression surfaced during Phase 3b.

All seven phases of the initiative landed on 2026-04-22.  Deferred polish
items (FPS counter, resize handling, avatar, hex-pick highlight) roll into
follow-up work and are not blockers.

### Brick Buster 0.8.4 polish pass

Gameplay feel:

- **Cel-shaded sprites** — every icon and the ball have dark outlines
  over flat-shaded bodies; the ball is a real round sprite with a
  four-frame squash animation that stretches along its velocity
  direction, so diagonal bounces look like bounces instead of flat
  horizontal/vertical squishes.
- **Paddle break** split from 3 rigid pieces to a **12-slot system**.
  On ball-lost the pieces fly out as three 4-piece planks held together
  by 1-pixel overlaps; on `SP_EXPLODE` powerup only 7 of 12 slots are
  active (pairs hidden pseudo-randomly) so some sections look like they
  held together.
- **Balloon powerup** is a rising on-screen projectile with a two-part
  hitbox.  Top half bounces the ball up and shoves the balloon down;
  bottom half mirrors.  The ball's horizontal velocity nudges the
  balloon sideways so the player can herd a loose balloon, pops on
  brick contact and triggers screen shake.
- **Screen shake** implemented as projection-matrix translation so one
  offset shakes the whole world — HUD stays fixed.  Used by balloon
  pops and the `SP_EXPLODE` paddle break.
- **Fire-ball after-images** — ring buffer of past ball positions
  renders a desaturating orange→grey trail that shrinks and fades as
  each entry ages.

Content:

- **Hand-designed levels 1–5** via a `level_brick(lv, r, c)` dispatcher:
  solid 3-row intro → first powerups in row 1 → shoulder-gap pyramid →
  downward-arrow shaft with an explode tip → smile-face pattern of
  specials.  Levels 6+ fall back to the procedural generator with
  progressively denser specials (8/50 at level 5 → +1/50 per level,
  capped at 20/50).
- **Start-row count reduced** from 5 to 3 so early sparse-powerup
  boards aren't a wall of single-colour bricks.
- **Ball and paddle both ~40 % faster** (`BALL_SPEED_BASE` 300→420 px/s,
  `PADDLE_SPEED` 500→620 px/s) — the earlier pace felt sluggish.

HUD & UX:

- **Heart-shaped lives** replace the red squares, rendered from a new
  `S_HEART` atlas cell (point-down after the canvas Y-flip).
- **Roman-numeral level caption** in the top middle (compact 28-pt
  texture per level).
- **High-score persistence** — `.loft/brickbuster_score.txt` loaded at
  boot, written on game-over when beaten, shown below the live score
  as a grey "HI <n>" line.
- **+1 heart on level clear** (soft-capped at 7).
- **Atlas diagnostic overlay** — press **I** during play to toggle a
  labelled 4×5 grid of every sprite index, useful for debugging any
  future atlas remapping.

Audio:

- **Three original chiptune tracks** (C-major "Heroic", A-minor
  "Determined", F-major "Calm Bridge") rotate through each level in
  a random order with 3–8 s silences between.  Queue resets on level
  change; once the three songs have played the sequencer is silent
  until the next level.

Infrastructure:

- `make play` target — prerequisite-checking launcher for the native
  OpenGL build with auto-recovery from stale incremental `rand_core`
  mismatches.
- `loft --html` switched to `wasm-opt -O1` — `-Oz --asyncify` was
  stripping all host imports.  Brick Buster now actually runs on
  Pages.
- Sibling-package `loft.toml` registration and `pub use audio::*` so
  `--native` resolves every `#native` symbol without stubs.
- `tests/scripts/test_gl_snapshots.sh --update` documented in
  `doc/claude/GAME_TESTING.md` as the canonical way to regenerate
  golden PNGs after a visual change.

### WebGL graphics gallery

- **GL6.1** — Graphics library .loft files embedded in WASM binary; `use graphics;`
  resolves under WASM without a native cdylib.
- **GL6.2–GL6.3** — WebGL2 bridge (`wasm_gl.rs`): 43 native gl_* functions read
  interpreter stack arguments and forward to JavaScript via `host_call`.
  `State::replace_native()` swaps panic stubs with real implementations.
- **GL6.5** — Shader version patching: GLSL `#version 330 core` automatically
  converted to `#version 300 es` with precision qualifiers for WebGL2.
- **GAL.2** — Graphics gallery page (`doc/gallery.html`) with WebGL2 canvas,
  example selector, source viewer, and complete JavaScript GL implementation.

### Playground improvements

- Assert results rendered with checkmarks/crosses and pass/fail summary.
- Examples split into categorized groups (Getting Started, Basics, Collections,
  Types & Patterns, Advanced, System, Performance) with `<optgroup>`.
- FizzBuzz default example added; 4 performance benchmarks (Fibonacci, Sieve,
  Mandelbrot, Collatz).
- Syntax highlighting fix: parentheses and punctuation now visible.
- Success status shows "Ok" instead of "error []".
- Diagnostics Display outputs clean newline-separated text instead of debug format.

### Game protocol (Sprint 17)

- **SRV.P** — `game_protocol` package: `MsgType` enum, `WsMessage`,
  `GameEnvelope` structs, and message constructors (`msg_ping`, `msg_pong`,
  `msg_chat`, `msg_input`, `msg_state`, `msg_error`).

### Parallel threading

- **A15** — `parallel {}` now uses real OS threads via `std::thread::scope`.
  Each arm runs in its own thread with a cloned `WorkerStores` snapshot.
  Validated: loft HTTP server + client running concurrently in `parallel {}`.

### HTTP server (Sprint 16)

- **SRV.1** — Blocking HTTP server with polling model. Loft controls the
  request loop via I13 iterator protocol (`for req in srv`). Native cdylib
  handles TCP accept/parse/respond using `std::net` only — no tokio/hyper.
  Functions: `listen`, `next` (iterator), `respond`, `close`.

### Graphics native (Sprint 15)

- **GL5.1** — Window creation + event loop via `glutin` + `winit` with
  `pump_app_events` polling model. Thread-local state via `RefCell`.
- **GL5.2** — Shader compilation and linking (vertex + fragment GLSL).
- **GL5.3** — VBO/VAO upload from packed vertex data (position + normal + color).
- **GL5.4** — Draw calls + render loop with `gl_draw`, `gl_clear`, `gl_swap_buffers`.
- **GL5.5** — Texture upload, binding, and deletion via `glTexImage2D`.
- **GL3** — Font loading (`fontdue`), text width measurement, and alpha bitmap
  rasterization. All in the `lib/graphics/native/` cdylib — no font dependency
  in the interpreter.

### HTTP client (Sprints 13–14)

- **H4.1** — `HttpResponse` struct with `status: integer`, `body: text`, and
  `ok()` method in the `web` package (`lib/web/`).
- **H4.2** — `http_get`, `http_post`, `http_put`, `http_delete` via native
  cdylib using `ureq`.  The `ureq` crate is only in the cdylib — the
  interpreter has no HTTP dependency.
- **H4.3** — Header support: `http_get_h`, `http_post_h`, `http_put_h`,
  `http_delete_h` accept `vector<text>` of `"Key: Value"` headers.
- **loft_register_v1** — unified native extension registration protocol.
  Each cdylib exports one C-ABI function that registers all symbols via a
  callback.  Generic `HashMap<String, FnPtr>` replaces per-function statics.
  All native cdylibs (imaging, random, web) use the new protocol.

### Native codegen for packages (Sprint 11)

- **PKG.4** — Native codegen `--extern`: packages with `[native.functions]` in
  `loft.toml` now emit direct Rust calls in `--native` mode.  The build pipeline
  passes `--extern` flags for pre-compiled native rlibs.
- **PKG.5** — WASM codegen linking: `--native-wasm` resolves package WASM rlibs
  from `prebuilt/wasm32-wasip2/` or `native/target/wasm32-wasip2/release/`.

### Language ergonomics (Sprint 10)

- **C55** — Type aliases: `type Handler = fn(Request) -> Response` — compile-time
  substitution for function and tuple types in `type` declarations.
- **C56** — Null-coalesce with early return: `x ?? return err` desugars to a
  null-check with immediate function return, collapsing two-line null guards
  into one expression.
- **A15** — `parallel { }` structured concurrency block: runs each arm
  sequentially (threading deferred). Three new opcodes replace six dead
  superinstruction slots, freeing three net opcode slots.
- **I13** — Iterator protocol: any type with `fn next(self: T) -> Item?` can be
  used in a `for x in val` loop. Null return from `next` terminates the loop.

### Graphics library (pure-loft package)

- **GL0** — Package scaffolding: `lib/graphics/` with `loft.toml` manifest.
- **GL1** — `Canvas` struct with `canvas()`, `get_pixel()`, `set_pixel()`, `clear()`,
  `blend()`, `blend_pixel()`.
- **GL2.1** — Drawing primitives: `fill_rect()`, `hline()`, `vline()`, `draw_rect()`.
- **GL2.2** — `draw_line()`: Bresenham algorithm for all octants.
- **GL2.3** — `draw_circle()`, `fill_circle()`, `draw_ellipse()`: midpoint algorithms
  with octant/quadrant symmetry.
- **GL2.4** — `draw_bezier()`: cubic Bezier with adaptive de Casteljau subdivision.
- **GL2.5** — `fill_triangle()`: scanline fill with vertex sorting.
- **GL2.6** — `draw_aa_line()`: Xiaolin Wu anti-aliased line with alpha blending.
- `fill_ellipse()`: solid filled ellipse via midpoint algorithm.
- **GL4.1** — `math.loft`: `Vec2`, `Vec3`, `Vec4`, `Mat4` types with vector ops
  (`add3`, `sub3`, `scale3`, `dot3`, `cross`, `normalize3`, `length3`) and matrix
  ops (`mat4_identity`, `mat4_translate`, `mat4_scale`, `mat4_mul`, `mat4_transform`).
- **GL4.2** — `mesh.loft`: `Vertex`, `Triangle`, `Mesh` types with builders
  (`add_vertex`, `add_triangle`, `add_quad`, `cube()`).
- **GL4.3** — `scene.loft`: `Material`, `Node`, `Camera`, `Scene` types with
  PBR material support and scene graph builder.
- **GL5** — `glb.loft`: `save_glb(mesh, path)` exports a single `Mesh` as a
  GLB 2.0 file (POSITION, NORMAL, TEXCOORD_0, u32 indices).  5 binary tests.
- **GL6** — `glb.loft`: `save_scene_glb(scene, path)` exports a full `Scene`
  with multiple meshes, PBR materials, and nodes into one GLB BIN chunk.
  9 tests including JSON content verification and multi-mesh BIN size.
- **GL7** — `scene.loft`: `node_at(name, mesh, mat, transform)` constructor.
  glTF 2.0 compliance: material reference moved to mesh primitive; node
  transform outputs `"matrix"` field only when non-identity.
- RGBA color packing via `rgba()`/`rgb()` using long arithmetic to avoid i32::MIN
  sentinel collision.
- 30 canvas tests covering all primitives.

### Bug fixes

- **C54** — `**` exponentiation operator now works, mapped to `pow()`.
- **P104** — Test runner no longer picks up library functions as tests;
  only functions defined in the test file are executed.
- **P107** — `++` (not a valid operator) now produces a clear error instead
  of crashing in codegen with a confusing type mismatch.

### Package registry (Sprint 9)

- **REG.1** — `src/registry.rs`: registry file parser with version resolution,
  package classification (yanked/deprecated/outdated/current/unknown), and
  installed package scanner.
- **REG.2** — `loft install <name>[@version]`: download and install packages
  from the registry.  Detects already-installed versions, warns on yanked packages.
- **REG.3** — `loft registry sync`: download the latest registry from the
  source URL (`# source:` header, `LOFT_REGISTRY_URL` env, or compiled-in default).
- **REG.4** — `loft registry check`: compare installed packages against the
  registry, report yanked/deprecated/outdated status, exit 1 on security issues.
- `loft registry list [--installed]`: browse all registry packages with
  installed status.

### Package infrastructure

- **PKG.1** — Native stub registration: `#native` annotations generate stubs replaced
  at load time by real shared-library implementations.
- **PKG.2** — `loft install` command for local package installation to `~/.loft/lib/`.
- **PKG.3** — Transitive dependency resolution: packages with `[dependencies]`
  in `loft.toml` automatically discover sibling packages.
- **`loft doc`** — New subcommand generates HTML documentation for packages:
  API reference from `src/*.loft` and guide pages from `docs/*.loft`.
- **PKG.6** — `loft test` subcommand discovers and runs `tests/*.loft` in packages.
- **PKG.3** — `[dependencies]` section in `loft.toml` manifest parsing.
- Manifest parser: `name`, `version`, `loft` version constraint, `native` stem fields.

---

## [0.8.3] — 2026-04-03

### Bug fixes

- **P58** — Variables with unknown type (typos like `y = unknown_thing`) now
  produce a compile-time error instead of silently creating garbage values.
- **P99** — Empty struct comprehension (`[for x in 0..0 { Struct{} }]`) with
  multiple hash types no longer crashes the compiler.
- **P100** — Format left-align (`:<`) and center-align (`:^`) now work for
  integers, longs, and floats.
- **P101** — Float format `:.0` (zero precision) now correctly rounds to zero
  decimal places.
- **P102** — `rev(vector)` now works — plain vectors can be iterated in reverse.
- **P98** — Index range queries with descending primary key now return correct
  results, in both interpreter and native codegen.
- **P91** — Circular `= expr` field defaults (e.g. `a: integer = $.b, b: integer = $.a`)
  are now detected at compile time.
- **C54** — `file.lines()` now returns content after the last newline (or content
  with no newlines at all).
- **P103** (mitigated) — Compile-time warning when vector concatenation appears
  inline in an expression that could corrupt the stack.
- **Windows native codegen** — Backslashes in file paths are now escaped in
  generated Rust string literals.

### Test infrastructure

- `tests/wrap.rs` now discovers and runs all `fn test_*()` entry points, not
  just `main()`.  Supports `@EXPECT_FAIL`, `@EXPECT_ERROR`, `@EXPECT_WARNING`
  annotations per function with `catch_unwind` isolation.
- 12 new test scripts (61–74) covering vector sort/reverse, index range queries,
  format edge cases, hash edge cases, known-issue reproducers (caveats/problems),
  and constant vector initialisation.
- `SUITE_SKIP` emptied — `15-lexer.loft` and `16-parser.loft` now pass.
- Branch protection enabled on `main` — PRs required with all 5 CI checks.

### Optimisations

- **`const_eval`** module — compile-time constant folder for arithmetic, casts,
  comparisons, and boolean ops across all numeric types.
- **`OpPreAllocVector`** — pre-allocates vector capacity for known-size literals,
  eliminating all `store.resize()` calls.
- **Constant comprehension unrolling** — `[for i in 0..N { expr(i) }]` is unrolled
  at compile time when bounds and body are const-evaluable (10,000-element limit).

### Documentation

- New **PACKAGES.md** — unified package format design (native Rust + WASM,
  dependencies, test suites, OpenGL case study).
- New **CONST_DATA.md** — constant data initialisation design with safety analysis.
- New doc pages: **29-match.loft** (pattern matching) and **30-formatting.loft**
  (format string reference).
- Expanded: 14-image (pixel scanning), 19-threading (worker rules), 25-generics
  (bounded generics with interfaces).
- Regenerated 137-page PDF reference.
- ROADMAP split: all H/MH items into S/M testable sub-steps with sprint ordering.
- PLANNING pruned: 473 lines of completed items removed.

### Closures

- **Cross-scope text-capturing closures** (A5.6-text) — Functions that return
  closures (`fn make_greeter(prefix: text) -> fn(text) -> text`) now work correctly.
  Four interrelated bugs fixed: premature closure free at function return, missing
  work buffer for text-returning fn-ref calls, 12-byte fn-ref pre-init (should be
  16 bytes), and closure record leak at caller scope exit.  Test:
  `closure_capture_text`.

### Native codegen

- **Fn-ref `(u32, DbRef)` tuple type** (C39) — Fn-ref variables in native-compiled
  code are now `(u32, DbRef)` tuples instead of plain `u32`.  Closure records are
  correctly freed via `.1` destructuring when fn-ref variables go out of scope.
  Non-capturing lambdas use the null sentinel and are safely skipped.

### Closures — native parity

- **Native cross-scope closures** (C47) — Functions that return closures now
  work in `--native` mode.  Five fixes: FnRef emits closure DbRef, CallRef
  passes `.1` as `__closure`, scope analysis skips cross-function deps,
  `last_closure_work_var` reset after function body, FnRef added to reachable
  set.  Doc test `26-closures.loft` now includes cross-scope `make_adder`.

- **Capturing closures with map/filter** (C48) — `map(v, fn(x) { x * factor })`
  and `filter(v, fn(x) { x > threshold })` now work with capturing lambdas.
  The collections parser accepts fn-ref variables and emits CallRef in the
  desugared loop body.

### Slot assignment

- **Text slot reuse** (C43) — Sequential text variables with non-overlapping lifetimes
  now share the same 24-byte zone-2 slot.  Uses a full conflict scan
  (`find_reusable_zone2_slot`) restricted to Text-only reuse at the top-of-stack
  position.  Tests: `assign_slots_sequential_text_reuse`, `text_slot_reuse_sequential`.

### Bug fixes (continued)

- **C41** — Struct-enum local variable leak (Problem #85) confirmed fixed; regression
  test `struct_enum_local_freed` added.
- **C42** — Undefined variable diagnostic confirmed working; test
  `unknown_variable_error` added.
- **C40** — Debug logger fn-ref opcode guard documented with WARNING comments in
  `02_images.loft` to prevent accidental removal.

### Parallel execution

- **`par_light` runtime foundation** (A14.1–A14.4):
  - A14.1: `Store::borrow_locked_for_light_worker` — O(1) read-only view sharing the
    original's buffer pointer. `borrowed` field prevents double-free on Drop.
  - A14.2: `WorkerPool` — pre-allocates `n_workers × M` stores, reused across invocations.
  - A14.3: `Stores::clone_for_light_worker` — assembles worker view with shallow borrows
    of main stores + fresh pool stores. Zero large buffer copies.
  - A14.4: `run_parallel_light` — drop-in for `run_parallel_direct` using the pool.
  - A14.5: `check_light_eligible` — DFS call-graph analysis validates no recursive store
    allocation. Returns `M` (pool stores per worker) for eligible workers.
  - A14.6: `build_parallel_for_ir` automatically selects `n_parallel_for_light` when
    the worker qualifies (primitive return, no recursive allocation). No new syntax —
    `par(...)` is transparently optimized.
  - A14.7: `n_parallel_for_light` native function registered. Allocates result vector,
    creates `WorkerPool`, dispatches via `run_parallel_light`.
  Auto-selection is fully enabled: eligible `par()` workers (primitive return,
  no recursive store allocation) transparently use the light path.  Three bugs
  fixed in the enablement: stack pop order in the native function, result DbRef
  `pos` field (4 not 8), and store borrow range (all stores, not just `[..max]`).

### Sorted collection slicing (A8)

- **Partial-key match iterator** (A8.3): `idx[k1]` on a multi-key index now iterates
  all elements matching the first key. Parser detects `nr < key_types.len()` and emits
  an inclusive range with `from = till = [k1]`. The existing `key_compare` zip-based
  comparison treats partial prefixes as unconstrained on remaining fields.

### WASM parallel infrastructure (W1.18)

- **WASM Worker Thread infrastructure** (W1.18-1 through W1.18-5):
  - W1.18-1: `#[cfg(all(feature = "wasm", feature = "threading"))]` branch in
    `run_parallel_direct` dispatches to JS host via `parallel_run()`.
  - W1.18-2: `worker_entry(fn_index, start, end)` exported via `#[wasm_bindgen]`.
  - W1.18-3: `tests/wasm/worker.mjs` — Worker Thread park/wake loop.
  - W1.18-4: `tests/wasm/parallel.mjs` — `LoftThreadPool` class.
  - W1.18-5: `tests/wasm/harness.mjs` — `initThreaded()` for shared-memory WASM.
  W1.18-6 (test enablement) deferred until wasm-threads build is available.

### Debugging infrastructure

- **Debug boundary checks for DbRef, record fields, and stack pops** —
  Three `debug_assert!` additions (zero cost in release builds):
  - `keys::store()` / `keys::mut_store()`: assert `store_nr < allocations.len()` with
    clear message showing both values.
  - `Store::addr()` / `Store::addr_mut()`: validate field offset against the record's
    claimed size (first word of record header). Fires for `rec > 1, fld > 0`.
  - `Stores::get<T>()`: assert `stack.pos >= size_of::<T>()` before decrement, catching
    stack underflow from wrong native-function pop order.

### Safety fixes

- **Coroutine store-mutation guard promoted to always-on** (CO1.9) — The generation
  counter in `Store` and the `saved_store_generations` snapshot in `CoroutineFrame`
  were previously compiled only under `#[cfg(debug_assertions)]`.  All `#[cfg]` gates
  have been removed so the guard fires in release builds too.  `debug_assert!` in
  `coroutine_next` is replaced with `assert!`, meaning a mutated-store violation now
  panics with a clear diagnostic in any build profile:
  `"stale DbRef: store N was mutated between coroutine yields (generation at yield: X,
  now: Y) — DbRef locals held by the generator may point to freed or reallocated records"`.
  The affected sites in `store.rs` are `claim`, `resize`, `delete`, and the two
  `clone_locked*` constructors.  New test `coroutine_stale_store_guard_all_builds`
  (no `#[cfg(debug_assertions)]` gate) confirms the panic fires unconditionally.

### Language features

- **`interface` keyword and first-pass parser** (I1, I2, I3) — The first three steps
  of the interface subsystem are implemented:
  - I1 (`src/lexer.rs`): `"interface"` is now a reserved keyword.
  - I2 (`src/data.rs`): `DefType::Interface` added to the definition-type enum;
    `Definition.bounds: Vec<u32>` added to hold interface constraints for bounded
    generic functions (`<T: A + B>`); initialised to `vec![]` in `add_def`.
  - I3 (`src/parser/definitions.rs`, `src/parser/mod.rs`): new `parse_interface()`
    method parses `interface Name { fn method(params) -> type }` declarations.
    `Self` is temporarily registered as a type placeholder so `parse_type_full`
    resolves it during method signature parsing.  Duplicate interface names emit
    "Redefined interface Name".  `parse_interface` is added to the `parse_file`
    top-level dispatch chain alongside `parse_struct`, `parse_enum`, etc.
  Tests: `interface_empty_parses`, `interface_with_method_parses`,
  `interface_duplicate_name_rejected`.

- **Interface subsystem — op-sugar, bound syntax, factory-method guard, gendoc skip** (I3.1, I4, I5, I11):
  - I3.1 (`src/parser/definitions.rs`): `op <token> (params) -> type` in interface bodies
    is syntactic sugar for an `OpCamelCase` method stub. E.g. `op < (self: Self, other: Self) -> boolean`
    registers a method named `OpLt`. The `rename()` helper in `mod.rs` is now `pub(crate)` and
    covers `>` and `>=` in addition to its previous set.
    Tests: `interface_op_sugar_lt_parses`, `interface_op_sugar_multi_parses`.
  - I4 (`src/parser/definitions.rs`): `<T: A + B>` bound syntax in generic function declarations.
    Bound names are collected during parsing and resolved in the second pass to `DefType::Interface`
    def_nrs stored in `Definition.bounds` (introduced in I2). Unknown names emit
    `"'Name' is not a known interface"`; non-interface names emit
    `"'Name' is not an interface — bounds must be interface names"`.
    Tests: `generic_fn_with_bound_parses`, `generic_fn_unknown_bound_errors`,
    `generic_fn_struct_as_bound_errors`.
  - I5 (`src/parser/definitions.rs`): phase-1 factory-method restriction in interface bodies.
    A method that returns `Self` without a leading `self: Self` parameter emits
    `"factory methods not yet supported: 'name' returns Self without a 'self: Self' parameter"`.
    Test: `interface_factory_method_rejected`.
  - I11 (`src/gendoc.rs`): `sig_kind` now returns `"interface"` for `pub interface` / `interface`
    declarations (previously `"const"`). `generate_stdlib_section` skips interface items gracefully.
    Unit test: `sig_kind_interface_returns_interface`.

- **Interface subsystem — satisfaction checking, bounded method/operator calls** (I6, I7, I8.1, I10):
  - I6 (`src/parser/mod.rs`): `check_satisfaction` verifies that a concrete type implements
    every method declared in a bounded generic's interface constraints. Called from
    `try_generic_instantiation` — emits `"'Type' does not satisfy interface 'Name': missing Method"`.
    Tests: `satisfaction_check_passes_with_implementing_type`,
    `satisfaction_check_fails_missing_method`.
  - I7 (`src/parser/fields.rs`, `src/parser/definitions.rs`): T-parameterized method stubs
    (e.g. `t_1T_label`) are created during second-pass bounds resolution. `field()` looks up
    the T-stub via `find_fn` before reporting "field access requires a concrete type", enabling
    `v.method()` inside generic bodies. `re_resolve_call` substitutes the concrete implementation
    at specialization time.
    Test: `bounded_method_call_in_generic_body`.
  - I8.1 (`src/parser/mod.rs`): `call_op` looks up T-stubs for operators (e.g. `t_1T_OpLt`)
    before erroring, enabling `a < b` inside bounded generic bodies. First-pass operator calls
    on T now return `Type::Void` instead of erroring, allowing the second pass to proceed.
    Test: `bounded_operator_in_generic_body`.
  - I10: satisfaction diagnostics share the I6 implementation above.
  - Supporting changes: `Data::children_of` iterates definitions by parent;
    `field()` returns `Type::Unknown(0)` in the first pass for unknown-type field access
    (previously errored); user-defined operator functions (e.g. `fn OpLt(self: Score, ...)`)
    are now allowed in user code without a lowercase name error.

- **Interface operator variants and stdlib `Ordered`** (I8.2, I8.3, I8.4, I9):
  - I8.2: Return-type propagation from interface signature — verified: T-stubs correctly
    substitute `Self` → `T` in both parameter types and the return type.
    Test: `bounded_operator_self_return_type`.
  - I8.3: Mixed-type binary operators (`T op concrete`, e.g. `T * integer`) — verified:
    `call_op`'s T-stub lookup and `call_nr`'s argument matching handle mixed-type parameters.
    Test: `bounded_mixed_type_operator`.
  - I8.4: Unary operators on `T` (e.g. `op -`) — verified: single-operand dispatch uses the
    same `call_op` → T-stub path as binary operators.
    Test: `bounded_unary_operator`.
  - I9: `pub interface Ordered { op < }` added to `default/01_code.loft`. User types satisfy
    `Ordered` by defining `fn OpLt(self: T, other: T) -> boolean`. Existing tests updated to
    use the stdlib interface instead of local redefinitions.
    Test: `stdlib_ordered_interface`.

- **Built-in type satisfaction and stdlib Equatable/Addable** (I9-prim, I9-Eq, I9-Add, I9.1):
  - I9-prim: `find_fn` now falls back to the `possible` operator map when the method-style
    name (`t_7integer_OpLt`) is not found. This lets built-in types (integer, float, etc.)
    satisfy interfaces since their operators use the `add_op` convention (`OpLtInt`).
    `call_op` skips the main operator loop when an operand is a generic type variable,
    preventing false matches via `OpEqRef` / `OpEqBool` implicit conversions.
    `check_satisfaction` delegates to `find_fn` for both naming conventions.
    Tests: `builtin_integer_satisfies_ordered`, `builtin_float_satisfies_ordered`.
  - I9-Eq: `pub interface Equatable { op == }` added to `default/01_code.loft`.
    Test: `stdlib_equatable_interface`.
  - I9-Add: `pub interface Addable { op + }` added to `default/01_code.loft`.
    Test: `stdlib_addable_interface`.
  - I9.1: bounded generics with Addable work on integer and float types.
    Tests: `generic_sum_pair_on_integers`, `generic_sum_pair_on_floats`.

- **Vector<T> element access fix and Numeric interface** (I9-vec, I9.1, I9.2, I9+):
  - I9-vec: fix vector element access in generic specialization. `substitute_type_in_value`
    detects `OpGetVector` calls with baked-in `elm_size=0` (from type variable elements),
    recomputes the correct size from the concrete type, and adds the value-extraction wrapper
    (`OpGetInt`/`OpGetFloat`/etc.).  First-pass `call_op` for generic types now returns the
    type variable type (not `Type::Void`) to prevent "cannot change type" errors.
    Test: `generic_vector_element_access`.
  - I9.1: bounded-generic comparison on vector elements using `Ordered` bound.
    Test: `generic_min_of_vector_elements`.
  - I9.2: bounded-generic sum of vector elements using `Addable` bound.
    Test: `generic_sum_on_integer_vector`.
  - I9+: `pub interface Numeric { op * ; op - }` added to `default/01_code.loft`.
    Test: `stdlib_numeric_interface`.

- **Generic accumulator fix, Scalable interface** (I9-var, I9.1, I9.2, I9-Sc):
  - I9-var: skip `ref_return`/`text_return` for generic templates (`DefType::Generic`).
    The return type `T = Reference(tv_nr)` triggered `ref_return` which promoted local
    variables to hidden parameters.  After specialization to Integer/Float, the hidden
    params caused a codegen crash.  This enables for-loop accumulator patterns inside
    generic bodies.
    Tests: `generic_intermediate_variable`, `generic_for_loop_accumulator`.
  - I9.1: generic `find_max` on integer vectors using `Ordered` for-loop accumulator.
    Test: `generic_max_on_integer_vector`.
  - I9.2: generic `vec_sum` with caller-supplied identity using `Addable` for-loop.
    Test: `generic_sum_with_identity`.
  - I9-Sc: `pub interface Scalable { fn scale(self, factor: integer) -> integer }` in
    `default/01_code.loft`.  Uses a method (not `op *`) to avoid stub-name collision
    with `Numeric`.
    Test: `stdlib_scalable_interface`.

- **Interface stub collision fix, generic min_of/max_of/sum** (I9-stub, I9.1, I9.2):
  - I9-stub: interface method stubs now use `__iface_{d_nr}_{method}` naming instead of
    `t_4Self_{method}`. Multiple interfaces can now declare the same operator without
    collision. `has_bound_for_method` prevents T-stubs from leaking into unbound generics.
  - I9.1: `min_of` and `max_of` replaced with bounded-generic versions using `Ordered`.
    Now work on integer, float, and any user type satisfying `Ordered`. Unused helper
    functions (`__min_int`, `__min_float`, `__max_int`, `__max_float`) removed.
    Tests: `stdlib_min_of_generic`, `stdlib_max_of_generic`, `stdlib_min_of_float`,
    `stdlib_max_of_float`.
  - I9.2: `pub fn sum<T: Addable>(v: vector<T>, init: T) -> T` added. The caller supplies
    the identity element. Integer-specific `sum_of(v)` kept for backward compatibility.
    Test: `stdlib_sum_generic`.

- **Text-returning interface methods, Printable, coroutine yield-from-loop** (I9-text, I9-Pr, CO1.7):
  - I9-text: T-stub creation adds hidden `__work_1: RefVar(Text)` parameter for
    text-returning interface methods. Matches the hidden param from `text_return` so
    `re_resolve_call` finds the correct argument count.
    Test: `generic_text_returning_method`.
  - I9-Pr: `pub interface Printable { fn to_text(self: Self) -> text }` added to stdlib.
    Test: `stdlib_printable_interface`.
  - CO1.7 (partial): coroutine yield from range-based and vector for-loops verified.
    Tests: `coroutine_yield_from_range_loop`, `coroutine_yield_from_vector_loop`.

- **CO1.7 complete: coroutine yield from all for-loop types** —
  Fixed character null sentinel bug: `push_null_value(4)` uses `i32::MIN` as the
  sentinel for all 4-byte values, but `op_conv_bool_from_character` only checked for
  `char::from(0)`. The `i32::MIN` sentinel (0x80000000) looked like a valid character,
  causing for-loops over character iterators to infinite-loop. Also fixed UB in
  `var_character` (fill.rs): reading `i32::MIN` directly as `char` is not a valid
  Unicode scalar — now reads as `u32` and converts via `char::from_u32`.
  Tests: `coroutine_yield_from_text_loop`, `coroutine_character_iterator_exhausts`,
  `coroutine_yield_from_struct_vector_loop`, `coroutine_yield_from_field_text_loop`.

- **CO1.8 complete: multi-text coroutine safety** — Verified all three CO1.8 sub-items
  pass without code changes: (a) multiple text parameters serialised correctly,
  (b) text locals after first yield survive resume, (c) text locals in nested blocks
  freed correctly. Tests: `coroutine_multi_text_params`, `coroutine_text_local_after_yield`,
  `coroutine_text_local_nested_block`.

- **fix-tvscope: clear diagnostic for type variable name clash** — Defining `struct T`
  when `T` is a generic type variable (from stdlib generics) now produces
  `"'T' is reserved as a generic type variable"` instead of a confusing
  "Redefined struct" message or a runtime crash.

### Sorted collection slicing (A8) (continued)

- **Open-ended bounds, range iteration, comprehensions** (A8.1, A8.2, A8.4, A8.6):
  - A8.1: `col[lo..]`, `col[..hi]`, and `col[..]` now work on sorted collections.
    Parser detects `..` before the first expression (open-start) and missing expression
    after `..` (open-end). Runtime handles empty from/till arrays in OpIterate.
    Tests: `sorted_open_end_range`, `sorted_open_start_range`.
  - A8.2: `sorted[lo..hi]` range iteration verified working. Test: `sorted_range_iteration`.
  - A8.4: `[for e in sorted[lo..hi] { expr }]` comprehensions verified.
    Test: `sorted_range_comprehension`.
  - A8.6: nullable lookup `if !col[k]` verified. Test: `sorted_nullable_lookup`.
  - A8.1-idx: open-ended bounds also work on index collections. Test: `index_open_end_range`.
  - A8.5: `rev(col[lo..hi])` reverse range iteration on sorted collections. Parser sets
    `reverse_iterator` flag before the inner subscript expression so `fill_iter` picks it up.
    Test: `sorted_reverse_range`.

### Coroutine safety documentation

- **Coroutine text arg `Str` serialised at create; pointer-patched on resume** (S25.1, S25.2) —
  `State::coroutine_create` now calls `serialise_text_args` after copying the raw
  argument bytes.  For each text (`Str`) argument that points into a dynamic heap
  allocation (not a static literal in `text_code`), the function clones the string
  data into an owned `String` stored in `frame.text_owned`, then overwrites the
  `Str` bytes in `stack_bytes` to point to the owned buffer.  The owned `String`
  outlives any `OpFreeText` the caller may emit after the create; the `Str` pointer
  is therefore never dangling on the first or any subsequent resume (P2-R1, critical
  use-after-free).
  At `coroutine_next`, each owned String's current buffer address is patched back
  into the cloned `stack_bytes` before the bytes are copied to the live stack
  (M6-b pointer-patch step).
  At `coroutine_return`, the existing `frame.text_owned.clear()` now properly drains
  the owned Strings that were populated by S25.1, freeing their heap allocations via
  Rust RAII instead of leaking them (P2-R2, high memory leak).
  Two new tests `coroutine_text_arg_dynamic_serialised` and
  `coroutine_text_arg_freed_at_return` in `tests/expressions.rs` exercise the create
  → resume → exhaust cycle with a dynamically formatted text argument.

- **`const` parameter writes now panic in release builds** (S22) — The
  `#[cfg(debug_assertions)]` guard on auto-lock insertion has been removed from
  `src/parser/expressions.rs`.  `store.claim()` and `store.delete()` now use
  `assert!` instead of `debug_assert!`, so writes to `const` Reference or Vector
  parameters produce a panic in both debug and release builds.  Previously, release
  builds silently discarded the write into a dummy buffer, causing `par()` workers
  to continue with stale data.  Tests `claim_on_locked_store_panics` and
  `delete_on_locked_store_panics` in `tests/expressions.rs` verify the runtime
  enforcement.

- **`e#remove` on a generator iterator: defense-in-depth runtime guard** (S24) —
  Calling `e#remove` inside a generator `for` loop was already rejected at compile
  time (CO1.5c).  A matching runtime guard has been added to `state/io.rs::remove()`
  and `codegen_runtime.rs::OpRemove()`: if `store_nr == u16::MAX` (the coroutine
  sentinel), a `debug_assert!` fires and the call returns early, preventing
  release-build store corruption even if the compiler check is somehow bypassed.

- **Generator functions rejected as `par()` workers at compile time** (S23) — The
  parser now detects when a `par()` worker function has return type `iterator<T>` and
  emits a clear diagnostic instead of allowing the call to proceed.  At runtime,
  worker threads have their own (empty) coroutine table; passing a generator DbRef
  across thread boundaries would either panic with an out-of-bounds index or silently
  advance the wrong generator.  A runtime bounds guard in `coroutine_next` provides
  defence-in-depth.  Test `par_worker_returns_generator` in `tests/parse_errors.rs`
  covers the compile-time path.

- **Abandoned coroutine frame freed on early `for` loop exit** (S37) — When a `for`
  loop breaks before a generator exhausts, `OpFreeRef` calls `free_ref` on the
  coroutine DbRef.  `database.free()` is a no-op for `COROUTINE_STORE`
  (store_nr == u16::MAX), so `text_owned` buffers, `stack_bytes`, and `call_frames`
  in the `CoroutineFrame` were silently leaked on every early-break path.
  Fix: `free_ref` now checks `db.store_nr == COROUTINE_STORE` and calls
  `free_coroutine(db.rec)` explicitly before returning.  Test
  `coroutine_early_break_frame_freed` in `tests/expressions.rs` exercises the
  early-break path and verifies the correct first-yield value is returned.

- **Exhausted coroutine slots freed immediately** (S26) — `coroutine_return` now sets
  the slot to `None` after marking it `Exhausted`, so the `State::coroutines` Vec does
  not grow without bound across repeated `for n in gen() { }` loops.  A guard in
  `coroutine_next` handles the `None` case (push null, return) so existing code that
  re-iterates is unaffected.  Test `coroutine_frame_freed_after_exhaustion` in
  `tests/expressions.rs` runs 1 000 loops to confirm no slot leak.

- **Coroutine `text_positions` save/restore across yield (debug builds)** (S27) —
  In debug builds, `coroutine_yield` now saves the suspended frame's
  `text_positions` entries and removes them from the live set; `coroutine_next`
  restores them on resume.  This prevents false double-free warnings and
  mask-missing-free bugs in `TextStore` ownership tracking when a generator is
  interleaved with text operations in the caller.  Test
  `coroutine_text_positions_save_restore` in `tests/expressions.rs`.

- **`WorkerStores` newtype for compile-time worker-store isolation** (S30) —
  `clone_for_worker` now returns `WorkerStores` instead of plain `Stores`.
  `WorkerStores` is `Send` but not `Sync` (via `PhantomData<*mut ()>`), giving a
  compile-time guarantee that worker-thread store snapshots are passed exclusively to
  `State::new_worker` and cannot be aliased across threads.  A `Deref<Target = Stores>`
  impl allows existing test code to inspect fields without change.

- **Debug generation counter for stale-DbRef detection in coroutines** (S28) —
  `Store` now carries a `generation: u32` field (debug builds only), incremented on
  every `claim`, `delete`, and `resize` call.  `coroutine_yield` snapshots the
  generation of every live, unlocked store; `coroutine_next` asserts that no snapshot
  store changed between yield and resume.  This catches the stale-DbRef hazard — where
  a struct record held by a suspended generator is freed or reallocated by the caller —
  as an early `debug_assert!` panic rather than silent corruption.  Test
  `coroutine_stale_store_guard` in `tests/expressions.rs`.

- **Parallel worker stores use `thread::scope` and skip `claims` clone** (S29) —
  `run_parallel_direct` in `src/parallel.rs` now uses `thread::scope` instead of
  `thread::spawn` + manual join loop, giving lifetime-bounded joining with no `Vec`
  of handles.  `Store::clone_locked_for_worker` skips cloning the `claims` `HashSet`
  (workers never call `validate()`) and `store.valid()` skips the claims check for
  locked stores, removing a spurious "Unknown record" panic that appeared in debug
  builds when workers accessed struct fields.

- **Store allocator uses free-bitmap; non-LIFO slot reuse now correct** (S29 P1-R4) —
  `database_named` previously always allocated from `self.max` and only reclaimed the
  top slot on `free_named`.  Native `OpFreeRef` legitimately frees slots in non-LIFO
  order, leaving freed slots permanently wasted and `max` growing without bound.  A
  `free_bits: Vec<u64>` bitmap was added to `Stores`; `set_free_bit`/`clear_free_bit`
  helpers update it on every free/alloc, and `find_free_slot` scans for the lowest set
  bit below `max`.  `clone_for_worker` propagates the bitmap to worker stores.
  Test `store_non_lifo_free_reclaims_slot` in `tests/threading.rs` verifies that a
  freed non-top slot is reused by the next `database()` call and `max` does not grow.

### Language features (continued)

- **Tuple destructuring in `match`** (T1.9) — `match` now dispatches on `Type::Tuple`
  subjects.  New `parse_tuple_match` in `src/parser/control.rs` parses comma- or
  semicolon-separated arms with wildcard (`_`), binding-variable, and literal patterns.
  Logical AND for multi-element conditions is built as `v_if(a, b, false)` (there is no
  `OpAnd`).  Tests: `tuple_match_wildcard`, `tuple_match_literal`, `tuple_match_binding`.

- **Homogeneous-type tuple coverage** (T1.10) — Three new tests confirm that same-element-type
  tuples work across common data sources: `tuple_homogeneous_text` (`(text, text)` pair
  from function parameters), `tuple_store_text_fields` (text fields extracted from two
  struct records), and `tuple_from_vector_elements` (`(integer, integer)` from indexed
  vector reads).  `tuple_struct_refs` (two `(Point, Point)` DbRefs) remains ignored
  pending T1.8 lifetime tracking for DbRef tuple slots.

- **Tuple type constraint diagnostics** (T1.11) — Two new compile-time guards:
  (a) `struct Foo { pair: (integer, integer) }` now emits "struct field cannot have a
  tuple type — tuples are stack-only values" at parse time (`parse_field` in
  `definitions.rs` detects `(` via `parse_type_full` before `fill_all` is reached);
  (b) `(a, b) += expr` now emits "compound assignment is not supported for tuple
  destructuring — use (a, b) = expr instead" (`parse_assign` in `expressions.rs` returns
  early in both passes, consuming the operator and RHS to keep the parser state clean).

### Coroutine safety documentation (continued)

- **Store-backed `Str` debug guard in `coroutine_yield`** (P2-R5 M10-a) — In
  `#[cfg(debug_assertions)]` builds on 64-bit targets, `coroutine_yield` now
  scans every tracked text local in the generator's `locals_bytes` and warns
  (`eprintln!("[P2-R5] ...")`) if the first 8 bytes (the `Str.ptr` field) fall
  within any live non-stack store allocation.  A store-backed Str in a suspended
  generator dangles if the consumer frees or reuses the backing record before
  the next resume.  The check is a heuristic (cannot cover full pointer
  provenance) but catches the common case of a recently-read text field local.
  No change to correct-program behaviour; the warning is diagnostic only.
  See `COROUTINE.md` CL-2b and `SAFE.md` § P2-R5.

- **Yielded `Str` ownership rule documented** (P2-R10) — `COROUTINE.md` CL-7 records
  the ownership invariant for `text` values produced by `yield`: the value is a
  zero-copy reference into the generator's frame (or `text_owned` buffer once CO1.3d
  lands) and is valid only for the current loop-body iteration.  Consumers that need
  to keep the text beyond one iteration must copy it (`stored = "{value}"`) or pass
  it to a function that calls `set_str`.  No runtime change; documentation only.

- **Text locals survive yield/resume in coroutines** (P2-R3 CO1.3d) — Text
  variables in generator functions are `String` objects (24 B) on the live stack.
  The bitwise copy of the locals region at yield is safe: `String` owns its heap
  buffer and no external code can free that buffer while the generator is suspended.
  The M8-b `debug_assert!` that fired for any text local at yield time has been
  removed; the S27 `text_positions` save/restore is preserved for correctness.
  Additionally, `coroutine_return` and `push_null_value` now push
  `Str::new(STRING_NULL)` (not 16 zero bytes) when an exhausted `iterator<text>`
  generator returns its null sentinel — the zero-pointer `Str` caused a panic in
  `append_text` via `slice::from_raw_parts(0, 0)`.  Test
  `coroutine_text_local_survives_yield` in `tests/expressions.rs` is now active and
  passing.

### Native store safety

- **Locked store cleared on free; `40-par-ref-return.loft` fixed** (S36) —
  `free_named` in `src/database/allocation.rs` now calls `unlock()` on the store
  before marking it free in the bitmap.  The parser auto-inserts
  `n_set_store_lock(stores, param, true)` at the start of functions with `const`
  reference parameters but does not emit the matching unlock before return.  When
  the store was freed while still locked, `find_free_slot` selected the freed slot
  for reuse and `database_named` called `init()` on a locked store, triggering:
  "Write to locked store at rec=1 fld=0".  The bug was invisible in the interpreter
  because `test_runner.rs` creates a fresh `Stores` per test function; in native
  mode all `test_*` functions share one `Stores`, so the leaked lock carried over
  from `test_par_struct_simple` into `test_par_struct_return_single_thread`.
  `40-par-ref-return.loft` now passes in `native_scripts` with 45/45.

### Interpreter fixes

- **`20-binary.loft` double-free fixed** (S34) — When `adjust_first_assignment_slot`
  cannot move a work variable downward (same-scope siblings block the move) and
  Option A fires — forcing the variable to the current TOS, aliasing it with the
  outer `rv` — the variable is now marked `skip_free` at that point.
  `generate_call` suppresses the `OpFreeRef` bytecode for any `skip_free` variable,
  preventing the "Double free store" panic caused by both `rv` and `_read_34` each
  trying to free the same database record at slot 820.  `skip_free` flags set during
  codegen are propagated back to `data.definitions[def_nr].variables` before
  `validate_slots` runs, which now skips slot-overlap pairs where either variable is
  `skip_free`.  The `binary` test (`tests/scripts/20-binary.loft`) no longer has
  `#[ignore]`; `"20-binary.loft"` removed from `ignored_scripts()` in `tests/wrap.rs`.

### WASM / native codegen fixes

- **Native codegen: Insert-return pattern fixed** (S35) — `output_set` in
  `src/generation/dispatch.rs` now detects `Value::Insert` as the RHS of an
  assignment and hoists all-but-last ops as standalone statements before the
  declaration line, emitting only the final expression as the assignment value.
  Previously the inner `Set` ops were emitted inline inside an expression context,
  producing malformed Rust (`let mut var_rv: DbRef = let mut var__read_34: DbRef = …`).
  The same function now also suppresses `OpFreeRef` for variables marked `skip_free`,
  matching the bytecode interpreter fix (S34) and preventing a double-free in the
  native binary.  `"20-binary.loft"` removed from `SCRIPTS_NATIVE_SKIP` in
  `tests/native.rs`; `native_binary_script` test passes without `#[ignore]`.

- **WASM random bridge wired; `rand_indices` shuffles via host bridge** (W1.19) —
  `codegen_runtime::n_rand` previously returned `i32::MIN` (null) when compiled
  without `feature = "random"`, making all `rand(lo, hi)` calls return null in WASM.
  It now delegates to `ops::rand_int`, which already had a WASM fallback calling
  `host_random_int` from `src/wasm.rs`.  A matching WASM `shuffle_ints` fallback
  (feature="wasm", not feature="random") was added to `src/ops.rs`, performing a
  Fisher-Yates shuffle via repeated `host_random_int(0, i)` calls; `n_rand_indices`
  in `codegen_runtime.rs` now enables the shuffle for both the PCG and WASM code
  paths.  `"21-random.loft"` removed from `WASM_SKIP` in `tests/wrap.rs`; the WASM
  compilation test now exercises `rand()`, `rand_seed()`, and `rand_indices()`.

- **WASM time bridge wired to `std::time::SystemTime`** (W1.20) — `host_time_now()`
  and `host_time_ticks()` in `src/wasm.rs` previously returned hard-coded `0`.
  They now call `std::time::SystemTime::now()` via the WASI clock interface (available
  in `wasm32-wasip2` through Rust's std).  `host_time_ticks()` delegates to
  `host_time_now()` (millisecond wall-clock); `n_ticks` computes elapsed microseconds
  as `(host_time_ticks() - start_time_ms) * 1000`, which is sufficient for benchmark
  timing.  `"22-time.loft"` removed from `WASM_SKIP` in `tests/wrap.rs`; the WASM
  compilation test now exercises `now()` and `ticks()` end-to-end.


- **WASM suite subprocess isolation; run-one.mjs helper** (W1.13) — Each test in
  `tests/wasm/suite.mjs` now runs in its own Node.js subprocess via `spawnSync` +
  `tests/wasm/run-one.mjs`.  Previously, a WASM crash (`RuntimeError: unreachable`
  or `memory access out of bounds`) in one test corrupted the shared module's linear
  memory, causing all subsequent tests in the same process to also fail.  `run-one.mjs`
  loads a fresh `pkg/loft.js` module and VirtFS default tree per invocation and writes
  the JSON result to stdout.  `suite.mjs` no longer imports `createHost` /
  `buildDefaultTree` / `withFiles`; the subprocess helper owns that setup.

- **`wasm_compile_and_run_smoke` converted to real integration test** (W1.9) — The
  hollow `#[ignore]` placeholder in `tests/wasm_entry.rs` has been replaced by an
  integration test that runs `node tests/wasm/bridge.test.mjs` as a subprocess.
  The test skips gracefully when the WASM package is not built or Node.js is absent,
  and fails with a clear message when the bridge tests report a non-zero exit code.

- **`13-file.loft` removed from `WASM_SKIP`** — File I/O operations (`OpDelete`,
  `OpMoveFile`, `OpMkdir`, `OpMkdirAll`) now route through `codegen_runtime::fs_*`
  functions that compile cleanly for the `wasm32-wasip2` target.  The wasm32-wasip2
  compilation test (`wasm_dir`) no longer skips `tests/docs/13-file.loft`; `#74`
  is fully resolved.


- **WASM file I/O wired to VirtFS host bridge** (W1.16) — All file operations
  (`read_text`, `write_text`, `read_bytes`, `write_bytes`, `seek`, `file_size`,
  `truncate`, `is_file`, `is_dir`, `list_dir`, `delete`, `move`, `mkdir`,
  `mkdir_all`) now call `globalThis.loftHost.*` via `js_sys::Reflect` under the
  `wasm` feature.  Helpers `assemble_write_data` and `dispatch_read_data` extracted
  from `state/io.rs` to share assembly logic between WASM and native paths and
  satisfy clippy `too_many_lines`.  `tests/wasm/bridge.test.mjs` gains three binary
  I/O tests (BigEndian write/read, seek + partial read, truncate); `doc/claude/ROADMAP.md`
  updated to mark W1.16 as done.

- **WASM skip for lock functions removed** (W1.17) — `n_get_store_lock` and
  `n_set_store_lock` are resolved from `loft::codegen_runtime` (listed in
  `CODEGEN_RUNTIME_FNS` in `generation/mod.rs`), so no `todo!()` stub is emitted.
  `18-locks.loft` removed from `WASM_SKIP`; the WASM compilation test now exercises
  `#lock` attribute syntax and `get_store_lock()` / `set_store_lock()`.

- **WASM skip for function references removed** (W1.15) — `output_call_ref` in
  `emit.rs` generates a `match` dispatch over all reachable definitions with a
  matching signature, implementing fn-ref calls (`f(args)` where `f: fn(T) -> R`)
  in native/WASM output.  `06-function.loft` removed from `WASM_SKIP`; the WASM
  compilation test now exercises function references, lambdas, and higher-order
  functions (`map`, `filter`, `reduce`).

### Native test harness fixes

- **`any`, `all`, `count_if` now work in native code generation; `47-predicates.loft` and `46-caveats.loft` unskipped** (N8a.4) —
  `predicate_loop_scaffold` in `src/parser/collections.rs` previously wrapped
  `[for_next, break_if_done]` in a `v_block`, which in native codegen became a
  Rust `{ ... }` block.  The loop variable (`any_elm`, `all_elm`, `cntif_elm`) was
  declared inside that block, making it invisible to the `short_circuit` or
  `count_step` expression that followed outside the block.  The fix inlines
  `for_next` and `break_if_done` directly in the loop body (the scaffold now returns
  a 4-tuple instead of 3), eliminating the nested block.  Both `47-predicates.loft`
  and `46-caveats.loft` (which uses `any`/`all` internally) removed from
  `SCRIPTS_NATIVE_SKIP`.

- **Native coroutine `yield from` delegation** (N8b.3) — `yield from sub_gen()`
  now works in native-compiled generators.  The sub-generator is stored as
  `Option<Box<dyn LoftCoroutine>>` directly in the outer struct, avoiding the
  `NATIVE_COROUTINES` `RefCell` that would cause a "RefCell already borrowed" panic
  when the outer `next_i64` tries to advance the inner generator.  The outer
  `next_i64` body is wrapped in a `loop {}` when yield-from segments are present;
  exhausted sub-generators set the next state and `continue` immediately.  Factory
  functions for sub-generators are called directly (not via `alloc_coroutine`) so
  sub-generators are never registered in the shared table.  CO1.4 test in
  `51-coroutines.loft` (`outer_with_from` producing 1+10+20+2 = 33) now passes.

- **Native coroutine state-machine code generation** (N8b.1, N8b.2) — Generator
  functions (`fn foo() -> iterator<integer>`) are now supported by the `--native`
  Rust backend.  Each generator is translated into a hand-written Rust state-machine
  struct (e.g. `NCountGen { state: u32, … }`) implementing the new `LoftCoroutine`
  trait (`fn next_i64(&mut self, stores: &mut Stores) -> i64`).  The coroutine body
  is split at `yield` nodes into match arms; a catch-all `_ =>` arm returns
  `COROUTINE_EXHAUSTED` (= `i32::MIN as i64`).  Three new pieces land in
  `src/codegen_runtime.rs`: the `LoftCoroutine` trait, a thread-local
  `NATIVE_COROUTINES` table (avoiding changes to `Stores`), `alloc_coroutine`,
  `coroutine_next_i64`, and `coroutine_is_exhausted`.  Call sites emit
  `loft::codegen_runtime::alloc_coroutine(foo(stores, args))` via a new
  `src/generation/coroutine.rs` module.  `OpCoroutineNext` and `OpCoroutineExhausted`
  are dispatched in `src/generation/dispatch.rs`.  `collect_calls` in
  `src/generation/mod.rs` now walks `Value::Yield` nodes so helper functions called
  from yield expressions are included in the reachable set.  `51-coroutines.loft`
  removed from `SCRIPTS_NATIVE_SKIP`; `native_scripts` passes all 4 generator tests.

- **`45-field-iter.loft` stale skip removed from native test harness** (N8a.5) —
  The `// A10` skip entry for `45-field-iter.loft` in `SCRIPTS_NATIVE_SKIP` was
  stale: the field-iteration native backend already worked correctly after the A10
  implementation.  The entry has been removed; `45-field-iter.loft` now runs in the
  `native_scripts` test alongside all other unblocked scripts.

- **Tuple types now supported in native code generation; `50-tuples.loft` unskipped** (N8a) —
  Three complementary fixes enable tuple types in the `--native` backend:
  (N8a.1) `rust_type(Type::Tuple)` now emits the correct Rust type `(T0, T1, …)`
  instead of `()`, and `default_native_value` returns `String` so tuple zero-values
  `(0, 0)` are built dynamically.
  (N8a.2) `Value::TupleGet` in `emit.rs` now uses the variable's declared name instead
  of its internal index number; `Value::TuplePut` emits the actual element assignment
  `var_x.i = …` rather than a stub.  `TuplePut` added to `is_void_value` in
  `pre_eval.rs` so the block emitter treats it as a statement, not a return expression.
  (N8a.3) Tuple-returning functions `make_pair`/`swap_pair` added to
  `tests/scripts/50-tuples.loft` (with LHS destructuring); the script removed from
  `SCRIPTS_NATIVE_SKIP`.  Both interpreter and native backends pass all tuple assertions.

- **Slot conflict in `20-binary.loft` fixed; removed from native skip list** (S32) —
  `adjust_first_assignment_slot` in `src/state/codegen.rs` now checks for same-scope
  sibling overlap (`has_sibling_overlap`) before moving a variable down to TOS, mirroring
  the existing `has_child_overlap` guard for child-scope variables.  This prevented `rv`
  and `_read_34` in `n_main` from being assigned the same slot range `[820, 832)` despite
  overlapping live intervals.  `20-binary.loft` removed from `SCRIPTS_NATIVE_SKIP`.

- **Generic instantiation confirmed working in native backend; `48-generics.loft` unskipped** (N8c) —
  Audit (N8c.1) showed that monomorphised generic functions already emit correct native
  code.  `48-generics.loft` removed from `SCRIPTS_NATIVE_SKIP`.

- **Optional feature dependencies now passed to standalone `rustc`** (S31) — The
  native test harness now calls `collect_extra_externs()`, which scans all `.rlib`
  files in the current test binary's `deps/` directory and passes each as
  `--extern crate_name=path`.  This unblocks scripts that use `rand`, `rand_seed`,
  or `rand_indices`: `tests/scripts/15-random.loft` and `tests/docs/21-random.loft`
  have been removed from the native skip lists.

- **Native rlib lookup now uses the current test binary's profile** (S33) — The
  previous `find_loft_rlib()` compared modification times across `release/` and
  `debug/` deps directories and could select the wrong profile's rlib (e.g. a
  newer no-features rlib from a `--no-default-features` CI step).  The function
  now uses `current_exe().parent()` — always the current test binary's own `deps/`
  directory — so the selected rlib always matches the features the test was compiled
  with.  `tests/docs/14-image.loft` has been removed from `NATIVE_SKIP`.

### Test coverage

- **`single` (f32) type fully covered** — New `tests/scripts/52-single.loft` covers
  all previously zero-coverage `single` operations: arithmetic (sub, mul, div, rem),
  all six comparison operators, NaN null semantics and propagation, null coalescing,
  positive/negative infinity (non-null), conversions (`as single` from integer/float/text;
  `single as` float/integer/long/text), format specifiers, and NaN-producing casts.
  The test is registered in `tests/wrap.rs` as `single_type`.

### Closure improvements

- **Spurious closure diagnostics suppressed** (A5.6d) — The "closure record '…' created"
  diagnostic is now `Level::Debug` (invisible in normal output and tests).  Captured outer
  variables are now marked as read at the call site via `var_usages`, eliminating false-positive
  "Variable X is never read" and "Dead assignment" warnings for validly captured variables.
  Tests `closure_capture_integer`, `closure_capture_after_change`, `closure_capture_multiple`,
  `closure_capture_text_integer_return`, and `closure_capture_text_return` no longer assert
  spurious warnings.

- **Closure capture coverage tests added** (A5.6e) — Four new tests in `tests/expressions.rs`
  verify closures across data-source scenarios: `closure_capture_struct_ref` (12-byte DbRef
  capture), `closure_capture_vector_elem` (vector element capture), and the existing
  `closure_capture_text_return` / `closure_capture_text_integer_return` tests cover text captures.

- **Work buffer cleared before each closure call** (A5.6f) — The hidden work-buffer `String`
  is now cleared (`v_set(wv, "")`) before each `OpCreateStack` injection at call sites.  Without
  this fix, calling a text-returning lambda inside a loop accumulated text from previous iterations
  (e.g. `"hello, world!"` became `"hello, world!hello, world!"` on the second call).  New test
  `closure_capture_text_loop` in `tests/expressions.rs` verifies the fix.

- **`fn`-ref conditional assignment no longer SIGSEGVs** (A5.6h) —
  `f = if flag { inc } else { dec }` caused a SIGSEGV at the `CallRef` opcode.
  Root cause: a fn-ref slot is 16 bytes (`[d_nr 4B][closure DbRef 12B]`), but
  each branch of an if-else expression generated only 4 bytes (the d_nr via
  `OpConstInt`), because `generate_block` (called for each branch) was setting
  `stack.position = to + size(Function) = to + 16` without emitting any instruction
  to push the 12-byte sentinel.  This phantom advance caused the codegen stack
  tracker to skip `OpNullRefSentinel` and left `CallRef` reading from the wrong
  stack position (the frame header, containing d_nr=0, which dispatched to
  `i_parse_errors()` and then SIGSEGVed in `dump_stack` on a garbage text pointer).
  Fix: `generate_block` now emits `OpNullRefSentinel` when the block result type is
  `Type::Function` and the block's content pushed fewer than 16 bytes.  A defensive
  `gen_fn_ref_value` helper in `generate_set` handles non-Block fn-ref values.
  Additionally, three native-codegen regressions introduced in A5.6g were resolved:
  (1) `visible_attr_count` (not `def.attributes.len()`) is now used in the candidate
  filter for closure-capturing lambdas; (2) the closure work-variable is injected at
  call sites for closure-capturing dispatch; (3) `Value::FnRef(d_nr, …)` is added to
  `collect_int_fn_refs` and emits `{d_nr}_u32` in native output so closure lambda
  functions appear in the reachable set and are compiled.  Test: `fn_ref_conditional_call`
  in `tests/issues.rs`; all 8 closure interpreter tests and the full native suite pass.

- **Definition-time capture semantics and multi-call closure injection** (A5.6g) —
  Closures now capture variable values at definition time (when the lambda is written),
  not at call time (when it is first invoked).  `emit_lambda_code` allocates and
  populates the closure record inside the `fn_ref_with_closure` block — the block is
  the `*code` assigned to the fn-ref variable, so it runs exactly once at definition
  time.  A `closure_vars` fallback was restored in `src/parser/control.rs` (both
  `try_fn_ref_call` and `parse_call` paths): when `last_closure_alloc` has already
  been consumed by a first call site, subsequent call sites to the same fn-ref variable
  look up the closure work variable via `self.closure_vars.get(&v_nr)` and inject it
  as the hidden `__closure` arg.  This fixes `closure_capture_struct_ref` and
  `closure_capture_vector_elem`, which each call the lambda twice (condition + format
  string).  Native codegen was also fixed: `OpVarFnRef`/`OpStoreClosure` declarations
  were removed from `default/02_images.loft` (they would have overflowed the 254-entry
  OPERATORS array); the `output_call_ref` dispatch in `src/generation/emit.rs` now
  compares total attribute count (including `__closure`) against total args (since
  the closure is injected explicitly at the call site, not by `fn_call_ref`); the
  `OpGetClosure` injection was removed.  The block result type was changed to a
  full-range integer to prevent native codegen from emitting a truncating `as u8`
  cast that corrupted the d_nr dispatch value.  All 8 closure tests pass (1 ignored
  for cross-scope closures, a known limitation in CAVEATS.md C1);
  `tests/docs/26-closures.loft` updated to reflect definition-time semantics.

### New features

- **Mutable closure capture works** (A5.6a) — `count += x` inside a lambda now
  compiles and executes correctly.  The `+=` operator on a captured integer variable
  routes through `call_to_set_op` → `OpSetInt`, bypassing the `generate_set`
  self-reference guard that previously caused a codegen panic.  Test `capture_detected`
  in `tests/parse_errors.rs` passes without `#[ignore]`.  Text capture remains
  blocked by two runtime bugs (see CAVEATS.md C1).

- **Lambda function type no longer includes text work variables** (A5.6a fix) —
  `parse_lambda` previously built the `Function(params, ret)` type from
  `data.attributes(d_nr)`, which also includes internal text work variables
  registered by `text_return()`.  This caused spurious "expects N argument(s),
  got M" errors when calling text-returning lambdas via function references.  The
  type is now built directly from the declared `arguments` list, which is always
  correct regardless of how many work variables are registered.

- **Closure capture works in debug builds** (A5.6) — The debug-mode store leak
  where closure record variables (`___clos_N`) were never freed has been fixed.
  `scopes.rs` now pre-registers block-result Reference variables at the enclosing
  outer scope so `get_free_vars` emits `OpFreeRef` at function exit.  A compile-time
  checker (`check_arg_ref_allocs`) panics in debug builds if any `Set(ref, Null)`
  initialisation is still nested inside a call argument, catching this class of
  scope-registration bug early.  Tests `closure_capture_integer`,
  `closure_capture_multiple`, and `closure_capture_after_change` all pass without
  `#[ignore]` in both debug and release builds.  Text capture and mutable capture
  remain deferred (A5.6 in ROADMAP.md).

- **Mutable closure captures write back to outer scope after each call** (A5.6c)
  — Void-return lambda calls now emit a write-back sequence after the `CallRef`
  instruction: for each field of the closure record, `OpGetInt` (or the
  field-type equivalent) reads the updated value back and stores it to the
  corresponding outer-scope variable.  Two root-cause bugs were fixed along the
  way: (1) `closure_vars.insert` was executing before the RHS lambda was parsed
  (because the insert check ran before `parse_assign_op`, which is where the
  lambda tokens are consumed); (2) the write-back used `Value::Block` (which
  creates a new scope), causing `scopes.rs` to emit `OpFreeRef` for the closure
  variable at the inner scope exit — leaving a dangling DbRef for the second
  call.  The fix uses `Value::Insert` instead, keeping the closure record alive
  across all calls in the outer scope.
  Test `p1_1_lambda_void_body` in `tests/issues.rs` passes without `#[ignore]`.

- **Text capture via `CallRef` no longer produces garbage DbRef** (A5.6b.1) —
  In `generate_call_ref`, the `__closure` argument (a `DbRef`) was being pushed
  onto the wrong stack frame: it was placed at the stack position of `x`
  (the first explicit argument), not at the position expected by the lambda
  body.  Two separate code paths were fixed: (1) for zero-param fn-refs the
  fast path now injects the closure arg; (2) `text_return` no longer adds
  captured RefVar(Text) variables as spurious extra args to the lambda's
  parameter type, which previously caused arity-mismatch failures.

- **`generate_call_ref` pre-allocates text work buffers for closures** (A5.6b.2)
  — A spurious `debug_assert!(work_vars.is_empty())` in `generate_call_ref`
  fired when a capturing lambda returned text, because the closure record
  contains a RefVar work buffer.  The assert has been removed; the existing
  logic already handles non-empty `work_vars` correctly.  Test
  `closure_capture_text_integer_return` passes without `#[ignore]`.

- **`yield` inside `par(...)` body now produces a compile-time error** (P2-R6
  M11-a) — The parser sets an `in_par_body` flag while parsing the body block
  of a `for … par(…)` loop.  When `yield` is encountered with `in_par_body`
  true, an Error diagnostic is emitted: "yield is not allowed inside a
  par(...) parallel body".  The yield expression is still consumed (to keep the
  lexer in sync) but no coroutine IR is generated, so scope analysis does not
  see orphaned reference variables.  The `in_par_body` flag is saved and
  restored for nested par() bodies.  Test
  `p2_r6_yield_inside_par_body_rejected` in `tests/issues.rs` passes without
  `#[ignore]`.  The existing runtime out-of-bounds guard (S23 / M11-b) in
  `coroutine_next` remains as defence-in-depth.

- **`yield from` slot-assignment regression fixed** (CO1.4-fix) — `yield from
  inner()` inside a coroutine with local variables before the delegation now
  produces correct results.  The two-zone slot redesign (S17/S18) already
  eliminated the overlap between the `__yf_sub` handle and inner loop
  temporaries; no additional IR restructuring was required.  Test
  `coroutine_yield_from` passes without `#[ignore]`.

- **`stack_trace()` works in parallel workers** (S21, fix #92) — Calling
  `stack_trace()` inside a `par(...)` loop body or any `run_parallel_*` worker
  now returns the actual call frames instead of an empty vector.  Two changes
  enable this: (1) `WorkerProgram` now carries `stack_trace_lib_nr` so the
  resolved index of `n_stack_trace` travels from the main state into each
  worker state; (2) `static_call` takes the call-stack snapshot when
  `stack_trace_lib_nr` matches even when `data_ptr` is null, using a
  `"<worker>"` placeholder for frames that lack `Data` context.  Worker states
  created via both `n_parallel_for_int` (bytecode path) and the direct
  `run_parallel_*` Rust API now report correct frame counts.  Test
  `parallel_stack_trace_non_empty` passes without `#[ignore]`.

- **`init(expr)` circular dependency detection** (S20) — Struct fields that
  form a mutual initialisation cycle (`a: integer init($.b), b: integer init($.a)`)
  now produce a compile error naming the cycle (e.g.
  `circular init dependency: a -> b -> a`).  A DFS cycle check runs after all
  struct fields are parsed; `$.field` reads inside `init(...)` are tracked by
  the parser and checked for cycles per root field.

- **`stack_trace()` vector fields zeroed + call-site line numbers** (S19) —
  `stack_trace()` now returns correct call-site line numbers (`StackFrame.line`)
  for every frame.  Three fixes: `n_stack_trace` explicitly zeroes the
  `arguments` and `variables` fields of each `StackFrame` element so that
  reused store blocks don't leave garbage data; `execute_log_steps` now
  pushes the same synthetic entry `CallFrame` as `execute_argv` (Fix #88
  parity); `fn_call` now resolves call-site lines with a BTreeMap backward
  range search, recovering the correct source line even when `code_pos` has
  advanced past the `line_numbers` entry.
  Tests `stack_trace_returns_frames`, `stack_trace_function_names`, and
  `call_frame_has_line` all pass without `#[ignore]`.

- **Tuple text elements** (T1.8b) — Functions returning `(integer, text)` (or any
  tuple containing a `text` element) now compile and execute correctly.  Text elements
  are stored as `Str` (16B borrowed reference) in tuple slots via the new `OpPutText`
  opcode, consistent with loft's text-argument convention.  Four codegen sites were
  updated: null-init now emits `OpConvTextFromNull`; slot stores use `OpPutText` instead
  of `OpAppendText`; tuple element reads use `OpArgText` instead of `OpVarText`.

- **Tuple function return + destructuring** (T1.8a) — Functions declared `-> (T1, T2)`
  now work end-to-end: the return value is materialised on the caller's stack, element
  access (`pair(3,7).0`) compiles and executes correctly, and LHS tuple destructuring
  (`(a, b) = pair(5)`) is fully supported.  Two fixes enabled this: the two-zone slot
  allocator now emits a no-op for zone-1 Tuple null-inits (space pre-reserved by
  `OpReserveFrame`) and a per-element push for zone-2 Tuple null-inits; the parser
  now marks destructuring targets as defined and types them on both passes so
  `known_var_or_type` does not fire a false "Unknown variable" on the second pass.

- **`size(t)` character count** — `size("héllo")` returns 5 (Unicode code points),
  complementing `len()` which returns byte length. Backed by a new `OpSizeText` opcode.

- **`FileResult` enum** — Filesystem-mutating operations (`delete`, `move`, `mkdir`,
  `mkdir_all`, `set_file_size`) now return a `FileResult` enum (`Ok`, `NotFound`,
  `PermissionDenied`, `IsDirectory`, `NotDirectory`, `Other`) instead of `boolean`.
  Use `.ok()` for a simple success check.

- **Vector aggregates** — `sum_of`, `min_of`, `max_of` for `vector<integer>`, implemented
  as `reduce` wrappers with internal helper functions. Predicate aggregates `any(vec, pred)`,
  `all(vec, pred)`, `count_if(vec, pred)` with short-circuit evaluation and lambda support.

- **Nested match patterns** — Field positions in struct match arms support sub-patterns:
  `Order { status: Paid, amount } => charge(amount)`. Supports enum variants, scalar
  literals, wildcards, and or-patterns (`Paid | Refunded`).

- **Field iteration** — `for f in s#fields` iterates over a struct's primitive fields
  at compile time. Each iteration provides `f.name` (field name) and `f.value` (a
  `FieldValue` enum wrapping the typed value). Works for uniform and mixed-type structs.

- **Generic functions** — `fn name<T>(x: T) -> T { ... }` declares a generic function.
  T must appear in the first parameter (directly or as `vector<T>`). The compiler creates
  specialised copies per concrete type at each call site (P5.2). Disallowed operations on
  T (arithmetic, field access, methods) produce clear compile-time errors (P5.3).
  Documentation test and LOFT.md section added (P5.4).

- **Shadow call-frame vector** (TR1.1) — The interpreter now tracks a shadow call stack
  with function identity and argument layout on each call/return.  The OpCall bytecode
  format encodes the definition number and argument size.  Foundation for `stack_trace()`.

- **Stack trace types** (TR1.2) — `ArgValue`, `ArgInfo`, `VarInfo`, and `StackFrame` types
  declared in `default/04_stacktrace.loft`.  These will be materialised by `stack_trace()`
  in TR1.3.

- **Closure capture analysis** (A5.1) — Lambdas that reference variables from an enclosing
  scope now produce a clear error: "lambda captures variable 'name' — closure capture is
  not yet supported, pass it as a parameter".  Previously this silently created a broken
  local variable.

- **Closure record layout** (A5.2) — For each capturing lambda, the parser now synthesizes
  an anonymous struct type (`__closure_N`) whose fields match the captured variables'
  names and types.  The record def_nr is stored on the lambda's Definition.

- **`stack_trace()` function** (TR1.3) — Returns `vector<StackFrame>` with function name,
  file, and call-site line for each active call frame.  Arguments/variables vectors are
  left empty (full population is future work).  Implemented as a native function with
  call-stack snapshot bridging State to Stores.

- **Call-site line numbers** (TR1.4) — `CallFrame` now stores the source line directly,
  resolved from `line_numbers` at call time.  Eliminates the per-frame HashMap lookup
  during stack trace materialisation.

- **Coroutine types** (CO1.1) — `CoroutineStatus` enum (Created, Suspended, Running,
  Exhausted) declared in `default/05_coroutine.loft`.  `CoroutineFrame` struct and
  coroutine storage infrastructure added to State.

- **`init(expr)` field initialiser** (L7) — `init(expr)` field modifier evaluates once
  at record creation (with `$` access), stores the result, and allows mutation afterward.
  Complements `computed(expr)` (read-only, recomputed on every access).

- **Tuple type system** (T1.1) — `Type::Tuple(Vec<Type>)` variant added to the type
  enum.  Helper functions `element_size`, `element_offsets`, and `owned_elements`
  provide reusable layout calculations for tuples and closure records.

- **Tuple parser** (T1.2) — Tuple type notation `(T1, T2)` is recognized in all type
  positions.  Tuple literals `(expr, expr)`, element access `t.0`, and LHS
  destructuring `(a, b) = expr` are parsed.  `Value::Tuple` IR variant added.

- **Tuple scope analysis** (T1.3) — Scope analysis recognizes `Type::Tuple` variables
  and identifies owned elements for reverse-order cleanup on scope exit.

- **Closure capture diagnostic** (A5.3) — The closure capture error message now
  indicates that closure body reads (A5.4) are the remaining blocker.  The closure
  record struct from A5.2 is still synthesized.

- **Tuple bytecode codegen** (T1.4) — `Value::TupleGet(var, idx)` IR variant for
  element reads.  Codegen emits `OpVar*` at the element's stack offset.  Tuple
  literals, element access, type annotations, and parameters now work end-to-end.

- **Closure body reads** (A5.4) — Captured variable reads inside lambdas now redirect
  to field loads from a hidden `__closure` parameter backed by the A5.2 closure record
  struct.  Read-only captures work; mutable captures are pending.

- **Coroutine opcodes** (CO1.2) — `OpCoroutineCreate` and `OpCoroutineNext` opcodes
  implemented.  Create copies arguments into a `CoroutineFrame` without entering the
  body.  Next restores the frame's stack and resumes execution.

- **`OpCoroutineReturn`** (CO1.3a) — Opcode to exhaust a running coroutine: clears
  frame state, pushes null, returns to consumer.

- **`OpCoroutineYield`** (CO1.3b) — Opcode to suspend a generator: serialises the
  live stack to `stack_bytes`, saves call frames, slides the yielded value to the
  frame base, and returns to the consumer.  Integer-only path; text serialisation
  pending (CO1.3d).

- **`yield` keyword** (CO1.3c) — Parser recognises `yield expr` in generator
  functions (return type `iterator<T>`).  Codegen emits `OpCoroutineCreate` for
  generator calls, `OpCoroutineYield` for yield statements, and `OpCoroutineReturn`
  at generator body end.  `iterator<T>` single-parameter syntax now accepted.

- **Generator type fixes** (CO1.3c-fix) — Generator body return-type check
  suppressed.  `next(gen)` and `exhausted(gen)` wired as special dispatch calls.
  Coroutine iterators no longer materialised into vectors.  `Type::Iterator` sized
  as DbRef.  `coroutine_create_basic` and `coroutine_next_sequence` tests pass.

- **Closure lifetime** (A5.5) — Closure record work variable is already freed by
  existing `OpFreeRef` scope-exit logic.  No new code needed.

- **`exhausted()` stdlib** (CO1.6) — `OpCoroutineExhausted` opcode and `pub fn
  exhausted(gen) -> boolean` declared in `05_coroutine.loft`.

- **`next()` stack tracking fix** (CO1.6a) — `OpCoroutineNext` and
  `OpCoroutineExhausted` now bypass the operator codegen path.  Stack position
  manually adjusted for DbRef consumption and value push.

- **Null sentinel on exhaustion** (CO1.6c) — `coroutine_next` pushes `i32::MIN`
  (integer null) when the generator is exhausted, not uninitialized bytes.

- **For-loop over generators** (CO1.5a+b) — `for n in gen() { ... }` works.
  The iterator protocol detects generator calls, stores the DbRef in a `__gen`
  variable, and uses `OpCoroutineNext` as the advance step with null-check
  termination.  All 6 coroutine tests pass.

- **`e#remove` rejection** (CO1.5c) — `#remove` on a generator for-loop variable
  produces a compile error (existing guard; coroutine loops never call `set_loop`).

- **Nested yield verified** (CO1.3e) — Generator calling a helper function between
  yields correctly saves/restores call frames across yield/resume.

- **`yield from` parsing** (CO1.4) — `yield from sub_gen` desugars to a loop that
  advances the sub-generator and forwards each value via `yield`.  Test `#[ignore]`
  pending slot-assignment fix.

- **Closure call-site allocation** (A5.3) — Capturing lambdas now allocate the
  closure record on the heap, populate fields from captured variables, and inject
  the record as a hidden argument at call sites.  Multi-capture variable redirect
  fixed (pre-has_var check).  Blocked by slot-assignment issue at codegen time.

- **Tuple element assignment** (T1.4) — `t.0 = expr` now works via `Value::TuplePut`
  IR variant.  Parser detects `TupleGet` on the LHS of `=` and routes through
  element-write codegen.

- **Reference-tuple parameters** (T1.5) — A `RefVar(Tuple)` parameter can now have
  its elements read and written using `.0`, `.1` … notation.  Codegen emits
  `OpVarRef` plus element `OpGet*`/`OpSet*` at the correct byte offset.

- **Unused-mutation guard for tuple refs** (T1.6) — Passing a tuple by reference to
  a function that never writes its elements now produces a WARNING (not an error),
  consistent with the existing scalar-ref mutation guard.

- **`integer not null` annotation** (T1.7) — `Type::Integer` gains a third boolean
  field (`not_null`).  The parser accepts the `not null` suffix on integer type names.
  Assigning a nullable value to a `not null` element in a tuple literal is a
  compile-time error.

- **Text parameter survives coroutine yield** (CO1.3d) — Two root causes for SIGSEGV
  in generators that hold a `text` parameter across `yield`:
  (1) `coroutine_create` now appends the 4-byte return-address slot to `stack_bytes`
  so that `get_var` offsets match the codegen-time layout on every resume;
  (2) `Value::Yield` codegen now decrements `stack.position` by the yielded value's
  size after emitting `OpCoroutineYield`, so subsequent variable accesses in the same
  generator use correct offsets on the second and later resumes.

### Bug fixes (continued)

- **Fix #87** — `static_call` no longer snapshots the call stack on every native
  function call; the snapshot now only runs when `n_stack_trace` is dispatched.

- **Fix #88** — `stack_trace()` now includes the entry function (main/test) as the
  outermost frame.

- **Null-coalescing fix** — `f() ?? default` no longer calls `f()` twice; non-trivial
  LHS expressions are materialised into a temporary before the null check.

- **Format specifier warnings** — Compile-time warnings for format specifiers that
  have no effect: hex/binary/octal on text or boolean, zero-padding on text.

- **Slot bug S17: text below TOS in nested scopes** — The two-zone slot redesign
  (0.8.3) fixed the `[generate_set]` panic for text variables pre-assigned below
  the actual TOS in deeply nested scopes.  `text_below_tos_nested_loops` passes;
  `#[ignore]` removed.  CAVEATS.md C4 closed.

- **Slot bug S18: sequential file blocks conflict** — Same two-zone redesign fixed
  the `validate_slots` panic from ref-variable slot override in sequential file
  blocks.  `sequential_file_blocks_read_conflict` passes; `#[ignore]` removed.
  CAVEATS.md C5 closed.

- **`while` loop** (L10) — `while cond { body }` is now a first-class keyword.
  Desugars to a loop with an `if !cond { break }` guard at the top, identical to
  the `for + break` workaround but with familiar syntax.  C11 closed.

### Language changes

- **Format specifier mismatches are now errors** (L9) — Using a radix specifier
  (`:x`, `:b`, `:o`) on a `text` or `boolean` value, or zero-padding (`:05`) on a
  `text` value, is now a compile error rather than a silent no-op.  C14 closed.

### Bug fixes (continued)

- **S15: match arm binding type reuse** — When multiple struct-enum match arms bind the
  same field name with different types, each arm now gets its own variable. Previously
  the second arm reused the first arm's type, causing garbled values.

- **S14: stdlib struct-enum field positions** — Struct-enum types defined in the default
  library (`FieldValue`, etc.) no longer panic with "Fld N is outside of record". Fixed
  two issues in `typedef.rs`: loop range for `fill_all()` and lazy byte-type registration.

---

## [0.8.3] — 2026-03-27

### New features

- **WASM output capture** (W1.2) — `output_push` / `output_take` helpers buffer `println`
  output in a thread-local string.  Used by `compile_and_run()` to collect program output
  without touching the filesystem.

- **WASM `compile_and_run()` entry point** (W1.9) — A `compile_and_run(files_json) -> String`
  function accepts a JSON array of `{name, content}` objects, runs the loft pipeline entirely
  in memory, and returns `{output, diagnostics, success}` JSON.  Exported via `wasm_bindgen`
  when built with `--features wasm`.  Default standard library files are embedded with
  `include_str!()`.  A virtual filesystem (`VIRT_FS`) routes `use` imports to the supplied
  in-memory files.

- **`#native "symbol"` annotation** (A7.1) — Functions declared in loft can carry a
  `#native "symbol_name"` annotation.  When the compiler resolves such a function it emits
  an `OpStaticCall` pointing to `symbol_name` in the native registry instead of the loft
  function name.  This decouples the loft identifier from the Rust symbol.

- **Native extension loader** (A7.2) — The `native-extensions` Cargo feature enables
  loading cdylib shared libraries at runtime via `libloading`.  `extensions::load_all()`
  is called between byte-code generation and execution; each library must export a
  C-ABI `loft_register_v1(*mut LoftPluginCtx)` entry point.

- **`LoftPluginCtx` public ABI** (A7.3) — `LoftPluginCtx` is a stable `repr(C)` struct
  published from `loft::extensions` and mirrored in the standalone `loft-plugin-api` crate.
  Plugin crates call `ctx.register_fn(name, fn_ptr)` once per exported function.

- **Format-string buffer pre-allocation** (O7) — The native/WASM code generator now emits
  `String::with_capacity(N × 8)` instead of `"".to_string()` at the start of format strings
  with ≥ 2 segments.  This avoids repeated `String` reallocations during format-string
  assembly, reducing the wasm/native performance gap on string-heavy workloads.

- **VirtFS JavaScript class** (W1.10) — `tests/wasm/virt-fs.mjs` provides a full in-memory
  virtual filesystem for WASM Node.js tests.  Features: tree-based JSON representation
  (`$type`/`$content` conventions), base64 binary support, path normalisation (`.`/`..`/`//`),
  `snapshot()`/`restore()` for test isolation, binary cursors (`seek`/`readBytes`/`writeBytes`),
  `toJSON()`/`fromJSON()` serialisation, and a minimal test harness (`harness.mjs`).
  13 unit tests in `virt-fs.test.mjs` cover all operations.  Runs via
  `node tests/wasm/virt-fs.test.mjs` when Node.js is available.

- **WASM test suite runner** (W1.13) — `tests/wasm/suite.mjs` discovers all loft programs
  in `tests/scripts/` and `tests/docs/`, runs each through the WASM module with a
  pre-populated VirtFS, and compares output against the native `cargo run` interpreter.
  Skips non-deterministic tests (time, unseeded random, images); verifies WASM success only
  for those.  Run via `node tests/wasm/suite.mjs` after building with `wasm-pack`.
  This is the main confidence gate for the WASM port.

- **LayeredFS class** (W1.12) — `tests/wasm/layered-fs.mjs` implements a two-layer virtual
  filesystem: an immutable base tree (bundled examples/docs/stdlib) plus a mutable delta
  overlay (user edits, persisted to localStorage).  Reads check delta first then fall through
  to base; writes always go to delta, leaving the base untouched.  Supports
  `getDelta()`/`setDelta()`/`saveDelta()`/`resetToBase()`/`isModified()`/`isDeleted()`.
  `ide/scripts/build-base-fs.js` reads `tests/docs/*.loft`, `doc/*.html`, and
  `default/*.loft` to emit `ide/assets/base-fs.json`.  20 unit tests in
  `layered-fs.test.mjs` cover all operations including delta serialisation and snapshot
  isolation.

- **loftHost factory** (W1.11) — `tests/wasm/host.mjs` exports `createHost(tree, options)`
  which wires a `VirtFS` instance to the full `loftHost` bridge API.  Uses a deterministic
  xoshiro128** PRNG for reproducible `rand()` / `rand_seed()` behaviour in tests.  Supports
  configurable `fakeTime`, `fakeTicks`, `env`, and `args` overrides.  Comes with:
  `bridge.test.mjs` (7 WASM integration tests; skips gracefully when `pkg/` not built),
  `file-io.test.mjs` (14 host-level edge-case tests, no WASM required),
  `random.test.mjs` (host PRNG tests + optional WASM-level determinism tests),
  and three fixtures in `tests/wasm/fixtures/`.

---

## [0.8.2] — 2026-03-24

### New features

- **Lambda expressions** — Write inline functions with `fn(x: integer) -> integer { x * 2 }`
  or the short form `|x| { x * 2 }`. Parameter and return types are inferred when the
  context makes them clear (e.g. inside `map`, `filter`, `reduce`). Lambdas cannot capture
  variables from the surrounding scope yet — pass needed values as arguments.

- **Named arguments and defaults** — Functions can declare default values
  (`fn connect(host: text, port: integer = 80, tls: boolean = true)`). Callers can skip
  middle parameters by name: `connect("localhost", tls: false)`.

- **Native compilation** — `loft --native file.loft` compiles your program to a native
  binary via `rustc` and runs it. `loft --native-emit out.rs` saves the generated Rust
  source. `loft --native-wasm out.wasm` compiles to WebAssembly.

- **JSON support** — Serialise any struct to JSON with `"{value:j}"`. Parse JSON into a
  struct with `Type.parse(json_text)` or into an array with `vector<T>.parse(json_text)`.
  Check for parse errors with `value#errors`.

- **Computed fields** — Struct fields marked `computed(expr)` are recalculated on every
  read and take no storage: `area: float computed(PI * $.r * $.r)`.

- **Field constraints** — Struct fields can declare runtime validation:
  `lo: integer assert($.lo <= $.hi)`. Constraints fire on every field write.

- **Parallel workers now support text and enum returns** — `par(...)` workers can return
  `text` and inline enum values in addition to the existing `integer`, `long`, `float`,
  and `boolean`. Workers can also receive extra context arguments beyond the loop element.

### Language changes

- **Function references drop the `fn` prefix** — Write `apply(double, 7)` instead of
  `apply(fn double, 7)`. Using `fn name` as a value is now a compile error.

- **Short-form lambdas infer types** — `|x| { x * 2 }` infers parameter and return
  types from the call site. Use the long form `fn(x: integer) -> integer { ... }` when
  you need explicit types.

- **Private by default** — Definitions without `pub` are no longer visible to `use`
  imports from other files.

### Better error messages

- Using `string` as a type now suggests `text` instead of a generic error.
- Match exhaustiveness errors now point at the `match` keyword, not the closing brace.
- Six common errors now include fix suggestions (e.g. "use a new variable name or
  cast with 'as'" for type-change errors).
- Three errors that previously stopped all parsing now let the compiler continue and
  report additional issues.
- Several places that crashed the compiler on unusual input now produce a proper error.

### Bug fixes

- `c + d` where both are characters no longer crashes. The result is text concatenation.
- PNG image loading now reports correct `width` and `height` values.
- Passing an empty vector `[]` directly as a function argument no longer crashes.
- `v += other_vec` on vectors containing text fields no longer corrupts the original.
- `&vector` parameters correctly propagate appends back to the caller.
- Vector slices assigned to a variable (`s = v[1..3]`) are now independent copies.
- `map`, `filter`, and `reduce` no longer cause internal slot conflicts.

---

## [0.8.0] — 2026-03-17

### New features

- **Match expressions** — Pattern match on enums, structs, and scalar values:
  ```loft
  match shape {
      Circle { r } => PI * pow(r, 2.0),
      Rect { w, h } => w * h,
  }
  ```
  The compiler checks that all variants are handled. Supports or-patterns
  (`North | South =>`), guard clauses (`if r > 0.0`), range patterns (`1..=9`),
  null patterns, character patterns, and block bodies.

- **Code formatter** — `loft --format file.loft` formats a file in-place.
  `loft --format-check file.loft` exits with an error if the file is not formatted.

- **Wildcard and selective imports** — `use mylib::*` imports everything;
  `use mylib::Point, add` imports only specific names. Local definitions take priority
  over imports.

- **Callable function references** — Store a function in a variable and call it:
  `f = fn double; f(5)`. Function-typed parameters also work.

- **`map`, `filter`, `reduce`** — Higher-order collection functions that accept
  function references: `map(numbers, fn double)`.

- **Test runner improvements** — `loft --tests file.loft::test_name` runs a single test.
  `loft --tests 'file.loft::{a,b}'` runs multiple. `loft --tests --native` compiles
  tests to native code first.

- **`now()` and `ticks()`** — `now()` returns milliseconds since the Unix epoch.
  `ticks()` returns microseconds since program start (monotonic timer).

- **`mkdir(path)` and `mkdir_all(path)`** — Create directories from loft code.

- **`vector.clear()`** — Remove all elements from a vector.

- **External library packages** — `use mylib;` can now resolve packaged library
  directories with a `loft.toml` manifest file.

### Diagnostics

- Warning for division or modulo by constant zero.
- Warning for unused loop variables (suppress with `_` prefix: `for _i in ...`).
- Warning for unreachable code after `return`, `break`, or `continue`.
- Warning for redundant null checks on `not null` fields.
- Warning when not all code paths return a value in a `not null` function.

### Bug fixes

- `x << 0` and `x >> 0` now correctly return `x` instead of null.
- `NaN != x` now returns `true` (was incorrectly `false`).
- `??` (null coalescing) on float values works correctly.
- Using `if` as a value expression without `else` is now a compile error instead of
  silently producing null.
- Assigning `null` to a struct field no longer causes a runtime crash.
- Functions with multiple owned struct variables no longer crash on cleanup.
- `sorted[key] = null` and `hash[key] = null` removal works again (was broken by a
  null-handling fix).
- `v += other_vec` on vectors with text fields no longer corrupts data.
- `index<T>` fields inside structs can now be copied and reassigned.
- Sorted filtered loop-remove, index key-null removal, and index loop-remove all fixed.
- `??` null coalescing, non-zero exit on errors, reverse iteration on `sorted<T>`,
  CLI args in `fn main`, format specifier sign order, XOR/OR/AND with null values,
  and `for c in enum_vector` infinite loop — all fixed.

---

## [0.1.0] — 2026-03-15

First release.

### Language

- **Static types with inference** — Types are checked at compile time. No annotations
  needed; the type is inferred from the first assignment.
- **Null safety** — Every value is nullable unless declared `not null`. Null propagates
  through arithmetic. Use `?? default` to provide a fallback value.
- **Primitive types** — `boolean`, `integer`, `long`, `float`, `single`, `character`, `text`.
- **Structs** — Named records with fields: `Point { x: 1.0, y: 2.0 }`.
- **Enums** — Plain enums (named values) and struct-enums (variants with different fields
  and per-variant method dispatch).
- **Control flow** — `if`/`else`, `for`/`in`, `break`, `continue`, `return`.
- **For-loop extras** — Inline filter (`for x in v if x > 0`), loop attributes
  (`x#first`, `x#count`, `x#index`), in-loop removal (`v#remove`).
- **Vector comprehensions** — `[for x in v { expr }]`.
- **String interpolation** — `"Hello {name}, score: {score:.2}"` with format specifiers.
- **Parallel execution** — `for a in items par(b=worker(a), 4) { ... }` runs work across
  CPU cores.
- **Collections** — `vector<T>` (dynamic array), `sorted<T>` (ordered tree),
  `index<T>` (multi-key tree), `hash<T>` (hash table).
- **File I/O** — Read, write, seek, directory listing, PNG image support.
- **Logging** — `log_info`, `log_warn`, `log_error` with source location and rate limiting.
- **Libraries** — `use mylib;` imports from `.loft` files.

---

[0.8.3]: https://github.com/loft-lang/loft/compare/v0.8.2...v0.8.3
[0.8.2]: https://github.com/loft-lang/loft/compare/v0.8.0...v0.8.2
[0.8.0]: https://github.com/loft-lang/loft/compare/v0.1.0...v0.8.0
[0.1.0]: https://github.com/loft-lang/loft/releases/tag/v0.1.0
