<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN120 — design

Five arcs. Each states the **mechanism** (verified on this tree, with the probe),
the **invariant** it violates, the **design**, what was **rejected**, and how it is
**verified**. Every mechanism below is VERIFIED unless the row says HYPOTHESIZED.

Reading order: **E** (nothing else matters if the tool cannot load your program) →
**B** → **A** → **D** → **C**.  As of 2026-07-27 everything except **A** has
shipped; A is the one arc left and its design is complete — including the fact
(§ A.1) that falsified the shape A was originally given.

---

## E — Reach: the debugger must run the programs people have

Reported as moros **H13**; reproduced here in full. This is the arc with the
largest gap between what the tool can do and what it can be pointed at.

### E.1 — `--lib` ignored — **SHIPPED 2026-07-27**

**Root cause (VERIFIED).** `run_file_debug` built its session with
`ReplSession::new(stdlib_dir)` — stdlib only. `run_rpc` and `run_serve` both use
`ReplSession::new_with_libs(stdlib_dir, lib_dirs)`, whose own doc-comment warns of
exactly this ("the session is stdlib-only and a library … cannot load"). One
caller of three skipped it. Everything downstream already worked: `collect_lib_dirs`
parses `--lib` before the debug dispatch, and `load_program` deliberately preserves
`parser.lib_dirs` across its reset.

**The sharp consequence, and a workaround for consumers today:** the fault was
**interactive-only**. Same file, same flag, same tree:

| invocation | result |
|---|---|
| `loft --interpret --lib <dir> prog.loft` | ✅ runs |
| `loft debug prog.loft:4 --lib <dir>` | ❌ `Library 'zttext' not found` |
| `loft debug prog.loft --rpc --lib <dir>` | ✅ `launch ok`, breakpoint `verified: true` |

So a consumer blocked on this can drive the **RPC** surface (which the `loft-debug`
skill documents) until the fix ships.

**Fix.** `run_file_debug` takes `lib_dirs` and uses `new_with_libs`; `main.rs`
passes the `lib_dirs` it already collected.

**Guarded by** `tests/repl_session.rs::file_debugger_resolves_a_library_from_lib_dirs`
plus its control `…_without_lib_dirs_cannot_resolve_the_library` — without the
control the first test would pass if the fixture were findable some other way.

**Still worth doing (follow-up).** Make the invariant structural rather than
remembered: give the three entry points one *resolution context* value (stdlib +
lib_dirs + registry) instead of parallel parameter lists, so the next flag cannot
be wired into two paths and forgotten in the third. That is the actual defect
class; the fix above closes this instance of it.

### E.2 — the target argument does not skip flags — **SHIPPED 2026-07-27**

**Root cause (VERIFIED).** The target was `args[pos("debug") + 1]`, unfiltered, so
`loft debug --lib dir f.loft:35` took `--lib` as the target, found no `:`, and
reported **`missing :<line>`** — a complaint about a token the user never offered
as the target.

**Fix, and why the obvious one is not enough.** Copying the `--serve` branch's
`.filter(|a| !a.starts_with('-'))` is insufficient: it then picks `dir`, the *value*
of `--lib`, which is just as wrong and fails just as confusingly. The walk must skip
a value-taking flag **and its value** (`--lib` / `--path` / `--port`). Verified with
both orders plus two controls: a genuinely missing `:<line>` still says so, and no
target at all still prints the usage.

### E.3 — a native call abandoned the session, unnamed — **SHIPPED 2026-07-27**

**Mechanism (VERIFIED).** A resumed run is wrapped in
`catch_unwind` (`src/repl.rs`); on `Err` the session is aborted and the message is
the fixed string *"runtime error in the paused run — debug session abandoned
(session preserved)"*. **The panic payload is discarded** — nothing prints the
`Box<dyn Any>`. So every distinct cause presents identically.

**The boundary is a call CROSSING INTO NATIVE CODE — not the package, not the
import.** moros bisected this; the ✅ rows are the controls that make it a boundary
rather than a guess, and the starred rows were re-run here:

| shape | under `loft debug` |
|---|---|
| `use web;` with **no call** | ✅ runs ★ |
| `use time;` + `from_ymd(2026, 7, 27)` — pure loft *inside* a registry package | ✅ runs ★ |
| `use random;` + `rand_seed(7)` / `rand(1, 10)` | ❌ dies ★ |
| `use web;` + `sleep_ms(5)` | ❌ dies ★ |
| `use server;` + `listen(port)` | ❌ dies |

So a registry package is fine until you call the part of it that is `#native`, and
a registry package whose functions are ordinary loft is fine throughout. Every row
runs correctly under `--interpret`. The bisection was the consumer's; naming the
cause took surfacing the panic payload first — see *What shipped* below.

**Consequence, before the fix:** a loft server with a live websocket could not be
debugged at all — it needs `server` + `web` (E.3) and a local `lib/` (E.1), so the
breakpoint was never reached. That is the shape of program most in need of a
debugger, and it is the one that could not be reached.

**Invariant.** *A failure inside a debug session names its cause.* The debugger
exists to explain failures; one that cannot explain its own is self-defeating.

**What shipped — and the method is the point.** Step 1 (surface the payload) was
done first *because* the cause was unknown, and it named it on the very first run:

```
runtime error in the paused run: native function not loaded: its library's
native cdylib is missing or stale …
```

Root cause (VERIFIED from there): the REPL's execute path ran `compile::byte_code`
— which registers native **stubs** — but never called
`extensions::load_all` + `wire_native_fns`. The CLI run path and `loft test` both
did. So an unwired stub panicked on the first `#native` call, which is exactly why
importing a registry package was harmless and calling the native part of it was
fatal. Fixed by a `wire_natives` helper on the session, called after `byte_code` on
the run path, with the hazard written at the helper so the next `byte_code` site
does not repeat it.

The abandon message now carries the payload and says the session ENDED and that
step/continue no longer apply — the old text left the user at a `loft>` prompt
where `:continue` answers "unknown command", which reads as a typo.

**Verified by** `file_debugger_can_call_into_a_native_library` (the `native_pkg`
fixture, a real cdylib the suite rebuilds) and the `panic_message` unit test.
moros's five-row matrix: all four re-runnable rows green, outputs checked by value
(`r=6`, `web ok 2`), not merely "resumed".

---

## B — No silent lies (breakpoint conditions) — **SHIPPED 2026-07-27**

**Mechanism (VERIFIED).** `ReplSession::frame_holds` is

```rust
matches!(self.eval(&format!("assert({condition}, \"cond\")")), Eval::Ran)
```

so *"the condition could not be evaluated"* and *"the condition is false"* are the
same answer. A condition naming anything not in the frame therefore reports
`verified: true` at `setBreakpoints` and then never fires.

**Probe.** `{"line":5,"condition":"i == 2"}` → `verified:true`, program runs to
completion. Same line, `total == 10` → stops correctly. (`i` is out of frame for
the reason in § A, so § A shrinks this arc's blast radius but does not close it —
a typo'd name has the same shape.)

**Invariant.** *A breakpoint reported as `verified` can fire.* `verified` is the
protocol's promise to the client; a permanently-inert verified breakpoint makes the
field worthless and the tool untrustworthy.

**Design.** Give the condition three outcomes instead of two — `true`, `false`,
`unevaluable(diagnostic)` — and act on the third: report it once per breakpoint
(not per hit; a hot line would flood) and **then break**, so a user who typo'd a
condition is stopped at the line rather than warned into scrollback. Carry the same
distinction on the RPC surface as an `output`/error event, since a scripted client
cannot read a console message.

**Rejected.** *Validating the condition at `setBreakpoints` time.* There is no
frame yet, so a condition over locals cannot be checked then; it would reject valid
conditions and still miss the real ones.

**Rejected.** *Reporting and skipping.* It preserves today's control flow, but the
symptom the user brought us is "my breakpoint does not fire" — a message they may
not see does not answer it.

**What shipped.** The live site turned out to be `resolve_pause`, not
`frame_holds`: `self.debug_eval(cond).as_deref() != Some("true")` folded `None`
(unevaluable) into "false". It is now a three-way `match`; the unevaluable arm
pushes a diagnostic into `trace_output` (so the interactive prompt prints it AND
the RPC surface emits it as an `output` event, from one place) and then stops.
Reported once per breakpoint offset via `cond_unevaluable`.

*Left as-is:* the public `frame_holds` — a post-run hit filter with no callers in
the tree — has the same conflation. Worth folding into the same three-way result
if it ever gains one.

**Verified by** three tests in `tests/repl_session.rs`: unevaluable ⇒ diagnostic
AND stop; false-but-valid ⇒ silent and no stop; true ⇒ stop and no diagnostic. The
middle one is the control — without it a "fix" that simply always breaks passes.

---

## A — Frame liveness

The one arc left, and the only one whose design was still a sketch. Everything
below is measured on this tree; § A.1 is a fact that **changes the arc**, and it
falsified two claims the earlier sketch rested on. Read A.1 before A.2.

### A.0 — Today's model, and the rows it gets wrong (VERIFIED)

`State::capture_frame_at` (`src/state/mod.rs`) builds each variable's `first`/`last`
**bytecode reference positions** by scanning `self.vars`, then shows a non-argument
local only when `first[v] <= pc <= last[v]`. The breakpoint pc is the line's *first*
operation. So a local **read later on the same line** is invisible (its `first` is
past `pc`), and a local whose **last read has passed** is gone though still in scope.

The probe is `for i in 0..4 { step = i * 10; total = total + step; }` (`total` above
the loop, a print below it). Reproduced 2026-07-27 with
`printf ':vars all\n:q\n' | loft debug loop.loft:<line>`:

| break at | today | want |
|---|---|---|
| line 4 `step = i * 10` | `total = 0`, `i = 0` | `total`, `i`, `step = <unset>` |
| line 5 `total = total + step` | `total = 0` — **nothing about the loop at all** | `total`, `step`, and `i` accounted for (see A.1) |
| line 7 after the loop | `total = 60` | `total` ✓ — `i`/`step` are out of scope |

`loft introspect` confirms `i` and `step` are real IR variables with slots
(`i(3)`, `step(5)`): nothing is constant-folded away, the filter is simply asking
the wrong question.

**Correction to the earlier write-up:** it said *"the compiler temp `i#index` spans
the whole loop and is shown"*. It is not. `i#index`'s reads are at pc 22/42/67/77,
all **before** line 4's pc 83, so it fails the same `pc <= last[v]` test and is
absent at both loop lines — `:vars all` at line 5 shows only `total` and
`__work_1`. That matters for **§ D.1**, whose premise was that `i#index` is the sole
surviving signal about loop position: there is no such signal today.

### A.1 — The fact that changes the arc: two locals share one slot (VERIFIED)

From `loft introspect` on the probe — the loop body, one iteration:

```
  80[64]: PutInt(var[48], value: integer)                     ← writes i
  83[56]: [4] VarInt(var[48]) -> integer var=i[48]:integer    ← line 4 reads i
  95[72]: MulInt(v1: integer, v2: integer) -> integer
  96[64]: PutInt(var[48], value: integer)                     ← writes step
  99[56]: [5] VarInt(var[32]) -> integer var=total[32]:integer
 102[64]: VarInt(var[48]) -> integer var=step[48]:integer     ← line 5 reads step
```

**`i` and `step` are the same four bytes.** This is by design, not a bug:
`assign_slots_v2` is documented as the *"scope-blind, kind-aware, aligned
interval-graph greedy allocator"* and `scopes.rs:2187` records it as **the only
allocator** — two locals share a slot whenever their live intervals are disjoint,
**including two locals in the same scope**. Nothing in the conflict rules mentions
scope.

Two consequences, cutting in opposite directions:

- **The target this arc was written to hit is unachievable.** The old table asked
  for `i` at line 5. At line 5 slot 48 holds `i * 10`; `i`'s own value no longer
  exists anywhere in the frame. No table, tier, or filter can produce it.
- **A scope-only fix would print a lie.** At line 4, `step` is in lexical scope and
  not yet assigned — and its slot holds `i`. "Show the locals in lexical scope with
  their slots' contents" prints `step = 0` on the first iteration: `i`'s value under
  `step`'s name. That is arc **B**'s defect (a silent lie) reintroduced inside arc A,
  and for a `text`/`vector` local it is worse than wrong — see A.3 fact 2.

So lexical scope answers *which names belong here*. It does not answer *whether the
frame still holds the value*. The invariant has to say both.

### A.2 — Invariant

> *A paused frame shows every local in lexical scope at that line, and shows each
> one either with its own value or with an explicit reason it has none — never with
> another local's bytes.*

Two reasons, kept distinct because the user's next move differs:

| marker | meaning | example |
|---|---|---|
| `<unset>` | in scope; its assignment has not run yet on this path | `step` at line 4 |
| `<reused by step>` | in scope; its slot now belongs to `step` | `i` at line 5 |

Silence is what today gets wrong — a reader who sees nothing concludes the debugger
is broken. A value that isn't there is a lie. A **named** absence is the only honest
third answer, and it is the same move arc B made for conditions: say what happened
and name the cause. `<reused by step>` also tells the reader what to do (break one
line earlier), which a bare `<optimised out>` does not.

### A.3 — The three facts, and where each comes from

| # | fact | question it answers | source |
|---|---|---|---|
| 1 | **scope spans** — per definition, `(pc_start, pc_end, scope)` for every block | is `v`'s scope open at `pc`? | **new** — recorded by codegen's block walk |
| 2 | **store spans** — per definition, `(pc_start, pc_end, var)` for every assignment | has `v` been written yet at `pc`? | **new** — recorded by codegen's `Set`/`TuplePut` arms |
| 3 | `Variables::scope(v)` → `u16` | which scope does `v` belong to? | **exists** (the IR dump's `name(N)`) |

Visibility is fact 3 joined to fact 1 on the scope number; the frame's *state* per
local comes from fact 2:

```
visible(v, pc)   ⟺  ∃ span (s,e,sc) with sc == scope(v) and s <= pc < e
assigned(v, pc)  ⟺  ∃ store span (s,e,v) with e <= pc          // e = the store has completed
owner(slot, pc)  =   the v whose latest completed store ≤ pc is greatest,
                     among the vars whose slot byte-ranges overlap
state(v, pc)     =   Unset            if !assigned(v, pc)
                     Reused(owner)    if owner(slot(v), pc) != v
                     Held             otherwise
```

**Correction to the earlier A.3.** It concluded that a scope→pc extent needs the
missing scope-**parent** relation. That is true only for *deriving* an extent from
the reference positions of a scope's own variables (the shortcut it correctly
rejected). An extent **recorded by codegen** covers its descendants for free: a
child block's bytecode is emitted *inside* its parent's, so **containment of pc
ranges is the nesting relation.** No parent table and no scope tree are needed —
which also makes fact 1 directly serviceable as a DAP `scopes` response (@I91).

Why each source is trustworthy:

- **Fact 1 is one recording site per block kind.** `generate_block`
  (`ValueType::Block`) and `gen_loop` (`ValueType::Loop`) both already receive an
  `IrBlock`; both emit their children sequentially, so the span is
  `code_pos` before → `code_pos` after. `IrBlock` needs a `scope()` accessor (6
  lines, mirroring `result()`); `Block.scope` is carried by **both** IR backings
  (`ir_store.rs:219` writes it, `ir_read.rs:354` reads it), so the store-IR path
  works unchanged. The **body** block's span must be widened to the definition's
  full `[code_position, +code_length)` so a `set_breakpoint_fn_start` pause — which
  lands on the entry preamble, before the body block's first child — still has the
  body scope open.
- **Each scope is emitted once, contiguously — measured.** Across all 952
  `tests/dumps/*.txt`, exactly 7 functions repeat a block-opener scope number, and
  **every one of them is the `(65535)` sentinel** (an uninstantiated generic
  template, which has no scope numbers at all). No real scope is emitted twice, so
  one span per scope is sufficient. If a future lowering does duplicate a block, the
  span list is a `Vec` — record both and let containment do the work; nothing in the
  query assumes uniqueness.
- **Fact 2 is provably complete at two sites.** `assign_slots_v2` skips any var with
  `first_def == u32::MAX` (it gets no slot at all), and `compute_intervals` sets
  `first_def` only on `Value::Set` and `Value::TuplePut`. Therefore **every var that
  has a slot has a `Set` or a `TuplePut` in the IR** — recording at those two
  codegen arms cannot miss a slotted local. A debug-build audit ("every slotted
  non-argument var in a def with scope spans has ≥ 1 store span") turns any future
  slot-writing lowering from a silent `<unset>` into a test failure.
- **Fact 2 must be recorded at the arm, not inside `generate_set`.** `generate_set`
  has early returns, and its existing `self.vars.insert(self.code_pos, v)` fires at
  the **statement-start** pc — *before* the value expression runs — so it is a lower
  bound on the store and would over-claim "assigned". Bracketing the arm
  (`let s = self.code_pos; …; push((s, self.code_pos, v))`) yields the completed-store
  pc and is immune to the early returns.
- **Why `self.vars` cannot be repaired into fact 2.** It is a `HashMap<u32, u16>`
  keyed by pc alone, and both `generate_set` (statement start) and `generate_var` (a
  read) insert into it. In the probe both fire at pc 83, so `step`'s write record is
  **overwritten by `i`'s read** — which is exactly why `first[step]` is 102 (its
  read) and why `step` vanishes at line 5. The map is structurally lossy for this
  question; a purpose-built list is not a nicety.
- **Fact 3 has one hole, and it fails safe.** Per-var `scope` is populated on the
  fresh-parse path for every executable definition — including instantiated generics
  (`fn t_7integer_pick(v:vector<integer>, at:integer)` → `chosen(1)`, arguments at
  scope 0). It is `u16::MAX` only for **uninstantiated templates**, which never
  execute. Independently, the store-IR `VarSnapshot` carries nine fields and `scope`
  is **not** among them, so any future warm-load of `Data` restores vars with no
  scope. Both cases are handled by one rule: **`scope(v) == u16::MAX` falls back to
  today's reference-span test for that var**, so such a definition degrades to
  today's behaviour rather than to an empty frame. If `Data` warm-loading is ever
  revived, adding `scope` to `VarSnapshot` upgrades it — the fallback means nothing
  breaks in the meantime.

**Cost.** One `Vec<(u32,u32,u16)>` plus one `Vec<(u32,u32,u16)>` per program, both
on `State` beside `line_numbers` (so **not** part of the serialised layout contract,
@PLN97 — nothing to version). Built on every run, like `line_numbers`: one code path,
no "correct only under `--debug`" divergence. The query is O(vars in def + spans in
def) and *replaces* a scan of the whole `self.vars` map per frame — so `break_stack`,
which today rescans the entire program's map once per frame, gets faster.

### A.4 — One query, and the state travels with the entry

The filter is asked for today in **five** places, three of which are silent if they
disagree. This is the arc's real hazard: widening the frame without moving every
consumer converts an information bug into a memory bug.

| # | site | what it does today | what widening the frame does to it |
|---|---|---|---|
| 1 | `capture_frame_at` (`state/mod.rs`) | rescans `self.vars` for `first`/`last` | the frame contents — the visible symptom |
| 2 | `frame_variables` (`state/debug.rs`) | rescans `self.vars` independently | a second, drifting copy of the same filter |
| 3 | `frame_local_is_live` (`state/mod.rs`) | `h.locals` *contains the name* | **silent**: an `<unset>` local is now "live", so `eval_frame_heap` reads a garbage `DbRef` — the OOB that gate exists to prevent |
| 4 | store-UAF detector (`state/debug.rs:512`) | `fv.bc_first > frame_pos` → skip | **silent**: mis-attributes or misses a freed-store read |
| 5 | `set_frame_value` / `set_frame_literal` | `frame_slot(name)`, no gate for ints; the **text** edit gates on liveness because overwriting a text local `Drop`s the old `String` | **silent, and the worst**: editing an `<unset>` text local `Drop`s garbage. The int path has no gate at all today, so `i = 5` at line 5 already writes `step`'s slot — latent now, likely once `i` is shown |

So the design is not "replace the filter in `capture_frame_at`". It is:

```rust
/// Every local in lexical scope at `pc`, each tagged with whether the frame
/// still holds its value.  The single source for the frame's contents, the
/// eval gate, the edit gate, and the slot dump.
fn frame_view(&self, d_nr: u32, pc: u32, data: &Data) -> Vec<FrameEntry>

struct FrameEntry { var_nr: u16, name: String, slot: u16, tp: Type,
                    is_argument: bool, state: LocalState }
enum LocalState { Held, Unset, Reused(String) }
```

`state` is **part of the value**, not something a caller recomputes — a site that
forgets to check it does not compile past the `match`. Sites 1 and 2 become mappers
over `frame_view`; 3 and 5 gate on `Held`; 4 gates on `Held` at its own
`frame_pos`. `BreakHit.locals` keeps its `Vec<(String, String)>` wire shape (the
marker rides in the rendered string, so **PROTOCOL.md and the RPC clients are
unchanged**), with the state carried alongside for the internal gates. Arguments
stay unconditionally `Held` — the caller wrote them.

`:eval` inherits the same fact and stops answering with a category: an unheld local
gets *"`i` is in scope but the frame no longer holds it — its slot was reused by
`step`; break at line 4 to read it"* instead of today's *"couldn't evaluate ... at
the frame"*. That closes **§ D.3**'s surviving consumer report by construction.

### A.5 — The design computed by hand on the probe

Spans read off the bytecode above: scope 1 = the whole function; scope 2 (`For
block`) = [18, 112); scope 3 (`For loop`) = [22, 112); scope 5 (inner block) =
[83, 109). Store spans: `total` [6,18) and [99,109); `i#index` [18,22) and [22,58);
`i` [77,83); `step` [83,99). Slots: `total` 32, `i#index` 40, `i` 48, `step` 48.

| pause | local | visible? | assigned? | owner of its slot | shown |
|---|---|---|---|---|---|
| line 4, pc 83 | `total` | 1 open | [6,18) ends 18 ≤ 83 | 32: only `total` | `0` |
| | `i` | 3 open | [77,83) ends 83 ≤ 83 | 48: `i`(83) vs `step`(99>83) → `i` | `0` |
| | `step` | 5 open | [83,99) ends 99 > 83 | — | `<unset>` |
| | `i#index` | 2 open | ends 58 ≤ 83 | 40: only `i#index` | `0` (temp; § D.1 filters) |
| line 5, pc 99 | `total` | 1 open | 18 ≤ 99 | `total` | `0` |
| | `step` | 5 open | 99 ≤ 99 | 48: `step`(99) > `i`(83) → `step` | `0` **← the value line 5 reads** |
| | `i` | 3 open | 83 ≤ 99 | 48 → `step` | `<reused by step>` |
| line 7, pc 112 | `total` | 1 open | ✓ | `total` | `60` |
| | `i`/`step`/`i#index` | 2/3/5 all closed at 112 | — | — | absent ✓ |

Every cell is derived from the four numbers above, not from running a fix — which is
what makes it a prediction the build can be validated against.

**One documented limitation, chosen deliberately.** State is evaluated in **static
pc order**, so inside a loop body it reads as *"so far in this iteration"*: at the
top of iteration 2, `step`'s store span (ending at 99) is still ahead of the pause,
so `step` shows `<unset>` even though its slot holds iteration 1's value. That is
the honest answer — a loop-body local is logically fresh each iteration and loft
gives no way to name the previous one — and a loop-**carried** local is unaffected
(`total`'s pre-loop store keeps it `Held`, which the table above shows).

### A.6 — Rejected

- **A debug tier that suppresses slot sharing** (`--lean` shows the tier machinery
  exists). The earlier draft rejected this on the grounds that *"the probe proves the
  variables already exist with slots. Nothing needs preserving"* — A.1 falsifies that
  reasoning, so the rejection is re-argued on stronger ground: it makes the debugged
  program's frame layout **different from the shipped one**, which for the subsystem
  that is loft's #1 weakness (store lifetime) is the worst possible property — a
  slot-aliasing heisenbug would vanish under the debugger. It also does not remove the
  need for fact 2 (an unwritten private slot is still garbage). Breaking one line
  earlier gets the user the same value at zero risk.
- **Making the allocator scope-aware again**, so no two simultaneously-in-scope
  locals share a slot. That is what V1 did; @PLAN53 replaced it deliberately, and the
  frame would grow for every same-scope disjoint-interval pair. Trading a
  recently-stabilised, correctness-critical layout for one line of debugger output is
  the wrong direction.
- **Recovering `i` at line 5 from `i#index`** (which holds the same logical value).
  It is an alias that exists for `for`-range loops and nothing else; the general case
  has no such twin, and a debugger that sometimes reconstructs a value from an
  unrelated slot is less trustworthy than one that says it cannot.
- **Deriving scope extents from variable reference positions** — the earlier A.3's
  shortcut, still rejected, still for its own reason: scope 3 holds only `i`, whose
  sole reference is on line 4, so the derived extent is a single point.

### A.7 — Verified by

The A.5 table as a scripted RPC session (`loft debug --rpc`), asserting the **exact**
local set and the **exact** rendering at each of the three lines. Three of the rows
are controls, and none is optional:

1. **line 7 must NOT gain `i`/`step`** — without it, a fix that shows everything
   everywhere passes.
2. **line 4 `step` must read `<unset>`, not `0`** — without it, the scope-only lie of
   A.1 passes. Non-vacuity: with the store-span check reverted, this row asserts the
   value `0`, which is `i`'s — prove the test fails that way before trusting it.
3. **line 5 `step` must read `0` and `i` must read `<reused by step>`** — the first
   half is the user-visible fix, the second is the honesty property.

Plus: a `text` variant of the probe (`msg = "x"` inside the loop) breaking on its
assignment line, asserting `msg = <unset>` **and** that `msg = "hi"` is refused there
— that is site 5's `Drop`-on-garbage guard, and it is the one failure mode of this arc
that corrupts memory rather than merely misinforming. And the debug-build store-span
audit from A.3, which runs over the whole suite for free.

### A.8 — Steps

Each step compiles and is verifiable on its own; the frame does not change until 4.

1. **`IrBlock::scope()`** + record **scope spans** in `generate_block` / `gen_loop`,
   with the body block widened to the definition's full code span. Inert — nothing
   reads them yet. *Gate:* a unit test asserting the probe's four spans.
2. **Record store spans** at the `Set` and `TuplePut` arms, plus the debug-build
   completeness audit. Still inert. *Gate:* the audit over `make test`.
3. **`frame_view` + `LocalState`**, implemented over facts 1–3 with the
   `scope(v) == u16::MAX` per-var fallback to the reference-span test. Not yet wired.
   *Gate:* a unit test over the probe reproducing the A.5 table cell for cell.
4. **Route all five sites** through `frame_view`; render the two markers. *Gate:* the
   A.7 RPC session including its three controls, plus the text-edit refusal.
5. **`:eval` names the reason** for an unheld local (closes § D.3).
6. **§ D.1** — with the frame correct, filter `i#index`/`__work_N` from `:vars` and
   keep `:vars all`. Note A.0's correction: the "loop position is only visible via
   `i#index`" objection to doing this was never true.

---

## D — Cleanup, and the consumer's write-up

- **D.1** — filter compiler-generated names (`__work_N`, `i#index`, `_`-prefixed)
  from `:vars` and from RPC frames. Still **sequenced after A**, but the original
  reason is withdrawn: the claim was that `i#index` is the only signal about loop
  position while `i` is missing, and § A.0 measures that it is not shown either (its
  last read precedes both loop lines). There is no signal to preserve — the reason to
  wait is simply that A is what makes the frame worth filtering. Keep a way to see
  them (`:vars all`) for compiler work.
- **D.2** — fold the zero-trust `doc/DEVELOPMENT.md` § *Debugging tools that
  actually work here* back into our docs. It is a second, independent user guide
  that should not have had to exist; the parts that are ours (driving the prompt
  non-interactively, what `:vars` shows) belong in `DEBUG.md` § C1.
- **D.3** — re-verify their two remaining reports against arc A: *"eval fails on a
  local not live at the exact break point"* is A by construction; *`:undo` reported
  nothing to undo* **does not reproduce** (`total = 99` → `:undo` → `0` works), so
  close it unless they can re-trigger it.

---

## C — Discoverability

The original finding, and the one with the best ratio.

| surface | mentions the debugger? |
|---|---|
| `CLAUDE.md` — always in context | **no** |
| `doc/claude/DEBUG.md` — 1053 lines, 23 §, where `CLAUDE.md` § Debugging policy routes | **no** — zero hits for `loft debug` / breakpoint / DAP |
| `loft --help` | yes |
| `.claude/skills/loft-debug` | yes, well, with a verified RPC example |
| `doc/features/F51.md` | yes |

**Design — one home, pointers to it.** The `loft-debug` skill stays canonical for
the agent RPC surface; nothing is copied.

- **C1** — a `## Interactive debugging` section in `DEBUG.md`: what the tool is,
  the one-line invocation, the piped-stdin form, and a pointer to the skill and to
  PROTOCOL.md. This is the biggest single gap — the doc that owns debugging does
  not mention the debugger.
- **C2** — one line in `CLAUDE.md` § Key commands. It is the only surface always in
  context.
- **C3 — SHIPPED 2026-07-27.** A bare verb shadowed a live local: the paused prompt
  accepts verbs with or without the colon, and `s` `n` `c` `r` `o` `u` `q` are both
  verbs and the commonest loft locals. Typing `c` to read a local **resumed the
  program and ended the session**. Now a live local wins and `:c` is always the
  verb (`handle_paused` + `paused_prompt_tests`, non-vacuity proven by reverting
  the guard).

**Rejected.** *Adding the debugger to the `loft-write` skill* (the question that
opened this plan). `loft-write` is a reference for *writing* loft; a runtime
investigation tool does not belong in it, and a second copy would drift from the
skill that owns it. The reach problem is that `DEBUG.md` and `CLAUDE.md` are
silent, not that the wrong skill is silent — C1 and C2 fix it at the source.

---

## What "better shape" means, concretely

- A two-package program with a registry dependency can be debugged (**E**).
- No breakpoint is both `verified` and inert; no failure inside a session is
  unnamed (**B**, **E.3**).
- A paused frame shows the locals a reader of the source expects, and never shows one
  local's bytes under another's name (**A**).
- An agent who has read only `CLAUDE.md` can find the tool (**C**).

---

## Concrete steps

Ordered. Each step is one commit's worth, names its gate, and can be verified
before the next begins. `[✓]` = shipped 2026-07-27.

### Now (reach — the tool cannot be pointed at real programs)

1. `[✓]` **E1** — `run_file_debug` takes `lib_dirs`, uses `new_with_libs`; `main.rs`
   passes them. *Gate:* the E1 test + its no-lib control.
2. `[✓]` **E2** — the target walk skips a flag and its value. *Gate:* both flag
   orders, plus "missing `:<line>`" and "no target" controls.
3. `[✓]` **E3a — print the discarded panic payload.** At the `catch_unwind` in
   `src/repl.rs`, downcast the `Box<dyn Any>` to `&str` / `String` and include it in
   the abandon message; say the session ENDED and which prompt the user is now at.
   *Do this before diagnosing E3b — it is what names the cause.* *Gate:* a probe
   that triggers the abandon and asserts the message carries a cause, not a
   category.
4. `[✓]` **E3b — fix native dispatch under debug control.** With E3a's message in hand,
   pin why a `#native` call panics in an observing run (hypothesis: the registry
   cdylib's store/registration is not carried on that path). *Gate:* moros's matrix
   — the three ❌ rows go green and the two ✅ rows stay green.

### Next (stop lying)

5. `[✓]` **B — three-valued conditions.** Give `frame_holds` a
   `true | false | unevaluable(diag)` result; report the third once per breakpoint,
   then break. Mirror it on the RPC surface as an event, since a scripted client
   cannot read a console line. *Gate:* the probe pair — unevaluable ⇒ diagnostic AND
   stop; false-but-valid ⇒ silent, no stop (the control that stops a fix from simply
   always breaking).

### Then (the design arc)

6. `[✓]` **A — design complete 2026-07-27.** Scope spans + store spans + `scope(v)`,
   one `frame_view` query, five consumer sites, the hand-computed expectation table
   and the gate: § A.
7. **A — build it.** ← THE ONE ARC LEFT. Six steps, each verifiable alone and the
   first two inert — see § A.8. *Gate:* § A.7 — the A.5 table as a scripted RPC
   session with its three controls (line 7 must NOT gain `i`/`step`; line 4's `step`
   must read `<unset>` and not `0`; line 5's `i` must read `<reused by step>`), plus
   the text-edit refusal and the debug-build store-span audit.
8. `[~]` **D1 — compiler temps.** `__`-prefixed filtered, `:vars all` shows them,
   `i#index` still shown. Finish this step with 7 — noting that the stated reason for
   holding it (loop position is visible only via `i#index`) is false: § A.0 measures
   that `i#index` is absent at both loop lines too.

### Any time (cheapest first, independent)

9. `[✓]` **C1** — `## Interactive debugging` in `DEBUG.md`: what it is, the one-line
   invocation, the piped-stdin form, pointers to the `loft-debug` skill and
   PROTOCOL.md. Biggest single gap.
10. `[✓]` **C2** — one line in `CLAUDE.md` § Key commands.
11. **D2/D3** — fold the consumers' write-ups back; close their `:undo` report
    (does not reproduce) and re-check "eval fails on a local not live here" against
    step 7.
