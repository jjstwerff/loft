<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN120 — design

Five arcs. Each states the **mechanism** (verified on this tree, with the probe),
the **invariant** it violates, the **design**, what was **rejected**, and how it is
**verified**. Every mechanism below is VERIFIED unless the row says HYPOTHESIZED.

Reading order: **E** (nothing else matters if the tool cannot load your program) →
**B** → **A** → **D** → **C** (independent, cheapest).

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

### E.3 — a native call abandons the session, unnamed

**Mechanism (VERIFIED, cause HYPOTHESIZED).** A resumed run is wrapped in
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
runs correctly under `--interpret`. **Why the native call panics is not yet
pinned** — that is the first task of this arc, and step 1 below is what will name
it. Working hypothesis (HYPOTHESIZED): the debug run executes as an *observing
REPL eval*, and that path does not carry whatever the direct-run path sets up for
native dispatch (the registry cdylib's store/registration).

**Consequence worth stating plainly:** a loft server with a live websocket cannot
be debugged at all today — it needs `server` + `web` (E.3) and a local `lib/`
(E.1), so the breakpoint is never reached. That is the shape of program most in
need of a debugger.

**Invariant.** *A failure inside a debug session names its cause.* The debugger
exists to explain failures; one that cannot explain its own is self-defeating.

**Design, in two independent steps:**

1. **Print the payload** (small, do first, independent of the cause). Downcast to
   `&str` / `String` and include it. This is worth doing even if step 2 removes
   every current trigger, because it converts the whole *class* from silent to
   diagnosed — and it is what will tell us the cause.
2. **Fix the native dispatch** under debug control, once step 1 names it.

**Also:** after `abort_debug()` the prompt silently reverts from `(dbg)` to `loft>`
and `:continue` answers *"unknown command"*, which reads as a typo. Say the session
ended and what state the user is in.

---

## B — No silent lies (breakpoint conditions)

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

**Verified by.** A probe pair: an unevaluable condition must produce a diagnostic
AND a stop; a false-but-valid condition must stay silent and not stop. The second
half is the control — without it the fix could simply break always.

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

### A.3 — where the scope fact comes from

The open question. `Variables` already carries per-variable scope and the IR
carries block structure, but `capture_frame_at` reconstructs from `State::vars`
instead — a bc→var map with no scope in it. Two candidates:

1. **Precomputed per-function scope table** (pc range → visible slots), built once
   at compile time beside the existing line table. Costs a little memory per
   function; makes the frame a lookup.
2. **Walk the IR at pause time** from the function's block tree to the pc. No
   memory cost, but it happens per pause and per frame in a stack walk.

**Recommendation: (1)**, because the same table is what a DAP `scopes` request
wants (@I91) and because a stack walk builds N frames at once. **The measurement
that settles it:** the added table size for the largest function in the stdlib, and
a paused-frame capture timing on a deep stack. Take it before building.

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
3. **E3a — print the discarded panic payload.** At the `catch_unwind` in
   `src/repl.rs`, downcast the `Box<dyn Any>` to `&str` / `String` and include it in
   the abandon message; say the session ENDED and which prompt the user is now at.
   *Do this before diagnosing E3b — it is what names the cause.* *Gate:* a probe
   that triggers the abandon and asserts the message carries a cause, not a
   category.
4. **E3b — fix native dispatch under debug control.** With E3a's message in hand,
   pin why a `#native` call panics in an observing run (hypothesis: the registry
   cdylib's store/registration is not carried on that path). *Gate:* moros's matrix
   — the three ❌ rows go green and the two ✅ rows stay green.

### Next (stop lying)

5. **B — three-valued conditions.** Give `frame_holds` a
   `true | false | unevaluable(diag)` result; report the third once per breakpoint,
   then break. Mirror it on the RPC surface as an event, since a scripted client
   cannot read a console line. *Gate:* the probe pair — unevaluable ⇒ diagnostic AND
   stop; false-but-valid ⇒ silent, no stop (the control that stops a fix from simply
   always breaking).

### Then (the design arc)

6. **A.3 measurement first.** Table size for the largest stdlib function, and
   paused-frame capture timing on a deep stack. Take it *before* choosing between
   the precomputed scope table and the pause-time IR walk; write the numbers into
   this doc.
7. **A — scope-based frame.** Replace the reference-span filter with declaring
   scope; show an in-scope-but-unassigned local as `<unset>`. *Gate:* the three-row
   probe table in § A, as a scripted RPC session asserting the exact local set —
   including the after-the-loop row, which must NOT gain `i`/`step`.
8. **D1 — filter compiler temps** from `:vars` and RPC frames, with `:vars all` to
   see them. Only after 7; before it, `i#index` is the sole signal about loop
   position.

### Any time (cheapest first, independent)

9. **C1** — `## Interactive debugging` in `DEBUG.md`: what it is, the one-line
   invocation, the piped-stdin form, pointers to the `loft-debug` skill and
   PROTOCOL.md. Biggest single gap.
10. **C2** — one line in `CLAUDE.md` § Key commands.
11. **D2/D3** — fold the consumers' write-ups back; close their `:undo` report
    (does not reproduce) and re-check "eval fails on a local not live here" against
    step 7.
