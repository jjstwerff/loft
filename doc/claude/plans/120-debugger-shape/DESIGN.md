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
shipped; A is the one arc left and § A.3 now says what it needs.

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

**Mechanism (VERIFIED).** `State::capture_frame_at` (`src/state/mod.rs`) builds
each variable's `first`/`last` **bytecode reference positions** by scanning
`self.vars`, then shows a non-argument local only when
`first[v] <= pc <= last[v]`. The breakpoint pc is the line's *first* operation. So:

- a local **read later on the same line** is invisible — its `first` is past `pc`;
- a local whose **last read has passed** is gone, though still in scope;
- the compiler temp `i#index` spans the whole loop and *is* shown.

**Probe** (`for i in 0..4 { step = i * 10; total = total + step; }`):

| break at | frame shows | should show |
|---|---|---|
| `step = i * 10` | `total`, `i` | `total`, `i`, `step` (unset) |
| `total = total + step` | `total` | `total`, `i`, `step` |
| after the loop | `total` | `total` ✓ (correct — `i`/`step` are out of scope) |

`loft introspect` confirms `i` and `step` are **real IR variables with slots**
(`i(3)`, `step(5)`) — this is not constant folding, and the values are in the frame;
only the filter hides them.

**Invariant.** *A paused frame shows exactly the locals in lexical scope at that
line.* Not "referenced near this pc" — scope is the thing a reader of the source
can predict, and the last row above shows scope already gives the right answer
where the current model accidentally agrees.

**Design.** Replace the reference-span filter with the **declaring scope**: a local
is in the frame from its declaration to the end of its enclosing block. Two
sub-decisions:

- **A.1 — uninitialised window.** A local in scope but not yet assigned shows as
  `<unset>` rather than being omitted. Omission is precisely today's failure, and a
  reader who sees `step = <unset>` on the line that assigns it learns something
  true; a reader who sees nothing concludes the debugger is broken.
- **A.2 — temps.** With the user's own `i` restored, `i#index` and `__work_1` are
  noise and can be filtered by the existing compiler-generated test — see § D. They
  must not be filtered before A lands.

### A.3 — where the scope fact comes from — **ANSWERED 2026-07-27**

The open question was whether to precompute a per-function scope table or walk the
IR at pause time. **Neither is optional: the fact does not exist yet, so it has to
be built.** What the tree actually has:

| candidate source | verdict |
|---|---|
| `Variables::scope(var_nr)` → `u16` | **exists and is meaningful** — the IR dump's `name(N)` is this number, correctly nested for the probe (`total(1)` ⊃ `i#index(2)` ⊃ `i(3)` ⊃ `step(5)`) |
| a scope → **pc range** | **does not exist** |
| a scope → **parent** relation | **does not exist** (no `scope_parent` / scope tree) |
| `Variables::loop_seq_ranges` | a per-scope `(start, end)` — but **loop scopes only**, and in statement-SEQUENCE units, not bytecode pc |

The tempting shortcut — derive each scope's extent as the union of its own
variables' reference positions — **does not work**, and the probe says why: scope 3
holds only `i`, whose sole reference is on line 4, so its derived extent is a
single point and `i` would still vanish at line 5. A correct extent must cover a
scope's DESCENDANTS, which needs the parent relation that is missing.

**So: candidate (1), a per-function scope table (pc range → visible slots) emitted
at compile time beside the existing line table.** The cost question that was going
to decide between (1) and (2) is moot — (2) needs the same missing tree — and (1)
is independently what a DAP `scopes` request wants (@I91) and what a stack walk
building N frames at once should read.

**Implementation note for whoever takes it:** keep `first[v]` from the existing
scan alongside the new table. Scope answers *"is `v` visible here"*; `first[v] > pc`
answers *"has it been assigned yet"*, which is what A.1 renders as `<unset>` — the
two facts are different and the frame needs both.

**Rejected.** *Suppressing the optimiser under a debug tier so every local
survives.* `--lean` shows the tier machinery exists, so it is available — but the
probe proves the variables already exist with slots. Nothing needs preserving; the
filter is simply asking the wrong question. Reach for a tier only if A.3's
measurement rules both candidates out.

**Verified by.** The probe table above, as a scripted RPC session asserting the
exact local set at each of the three lines — including the third row, which must
NOT gain `i`/`step`. Without that row a fix that shows everything everywhere passes.

---

## D — Cleanup, and the consumer's write-up

- **D.1** — filter compiler-generated names (`__work_N`, `i#index`, `_`-prefixed)
  from `:vars` and from RPC frames. **Blocked on A**: today `i#index` is the only
  signal about loop position while `i` is missing, so filtering first would make
  the frame strictly worse. Keep a way to see them (`:vars all`) for compiler work.
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
- A paused frame shows the locals a reader of the source expects (**A**).
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

6. `[✓]` **A.3 — answered.** No scope→pc range and no scope-parent relation exist, so the table must be built; see § A.3.
7. **A — scope-based frame.** ← THE ONE ARC LEFT. Replace the reference-span filter with declaring
   scope; show an in-scope-but-unassigned local as `<unset>`. *Gate:* the three-row
   probe table in § A, as a scripted RPC session asserting the exact local set —
   including the after-the-loop row, which must NOT gain `i`/`step`.
8. `[~]` **D1 — compiler temps.** `__`-prefixed filtered, `:vars all` shows them, `i#index` deliberately kept until A lands: it is the sole signal about
   loop position while the user's own `i` is missing. Finish this step with 7.

### Any time (cheapest first, independent)

9. `[✓]` **C1** — `## Interactive debugging` in `DEBUG.md`: what it is, the one-line
   invocation, the piped-stdin form, pointers to the `loft-debug` skill and
   PROTOCOL.md. Biggest single gap.
10. `[✓]` **C2** — one line in `CLAUDE.md` § Key commands.
11. **D2/D3** — fold the consumers' write-ups back; close their `:undo` report
    (does not reproduce) and re-check "eval fails on a local not live here" against
    step 7.
