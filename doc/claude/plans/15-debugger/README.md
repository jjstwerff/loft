<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 15 — Debugger (interpreter-mode; host-agnostic engine, browser its natural home)

> **Identity:** `@PLN15` — [`loft-lang/plans`](https://github.com/loft-lang/plans/issues)
> issue (pending creation). Slug `debugger`.

## Status

In progress — **tracer-bullet slice landed (2026-06-08)**: the riskiest unknown is
now proven *yes*. A breakpoint set on a source line pauses the interpreter there and
reads the live frame's arguments correctly; it fires once per call and is inert when
nothing is registered. `src/debugger.rs` (`Debugger`/`BreakHit`) + the per-op hook
in `State::execute_argv` + `set_breakpoint_fn_line` (function-scoped) /
`set_breakpoint_fn_entry` / `capture_break_frame` / `render_frame_local`;
`tests/debugger.rs` (**17 tests**). **The pipeline works end-to-end:** set a
breakpoint → pause + capture the live frame (args **and** non-arg locals,
liveness-gated, every value type) → **drop into a REPL and evaluate against the
frame** (`ReplSession::seed_frame` / `from_parser` — `n + a == 21`,
`pt.x * pt.y == 12`) → **conditional breakpoints** (`frame_holds`) → **step
into / over / out / continue** (`debug_step`) → **suspend, edit a value in the REPL,
resume — and execution continues with the edited value** (`enable_stepping` /
`set_frame_value` / `value_of`: edit `n` at a break → `calc(5)` returns 990).
D0 + D1 + E + F done. The hot loop carries one
`Option::is_some` branch per op when not debugging (issues suite green — no
regression). **Facts the slices pinned** (matrix-first paid off): a function-*entry*
breakpoint pauses pre-prologue (slots zero), so the read point is a **body line**; a
variable sits at the frame-absolute `stack_cur.pos + args_base + vars.stack(i)` —
*not* `get_var`'s stack-depth-relative operand; a bare line number is *unscoped*
(matches every function), so breakpoints are function-scoped; `first_def`/`last_use`
are *sequence* numbers (wrong unit), but `self.vars`' `code_pos → var_nr` map yields
a usable reference-range liveness (Q6); the **own-format literal is the D1 bridge**
(so read-eval-at-frame needs no @PLN14 store-resident env — exact value-for-value
seeding is the remaining upgrade); and **loft calls are *jumps*** (all execution
state is in `State` fields), so the loop can suspend at a breakpoint and re-enter to
resume — the basis for F's step into/over/out + edit-and-continue. **G1 interactive
pause/step/edit + REPL-at-frame landed (2026-06-08)**: the REPL itself now suspends
at a breakpoint (`(dbg)` prompt), shows the frame, steps (into/over/out/continue),
edits a live **scalar** local (integer/float/single/boolean/character, by literal
or expression) that the resumed call picks up, **and evaluates any expression
against the paused frame** (`n * 3`, `pt.x * pt.y` — every type) — all proven
end-to-end through the binary. **G1-span fixed (2026-06-08):** breakpoints now
resolve against the dense `line_numbers` table, so **every** body line is breakable
(was source-spans = fault-prone arithmetic only, leaving pure-`if`/constant bodies
unbreakable). **B finished (2026-06-09):** B1 (interpret_set) + B2 (`unmark_for_debug`
— un-mark + recompile switches a compiled fn's interpret-set back to interpreted) +
B3-introspection (`break_stack`) shipped; the B3 *switching* facet (on-demand
mid-run, on-stack deopt) deferred to the lavition engine integration. **Text +
simple-enum live edits landed (2026-06-09):** a text **local** (owned `String`,
overwritten without dropping the possibly-uninitialised prior slot) and text
**argument** (16-byte `Str` repointed at a `Debugger`-owned buffer) and a simple
enum (inline discriminant byte) are now editable at a pause and picked up on resume;
the fix also corrected the text-**local** *read* (the renderer was reading every text
slot as a 16-byte `Str`, but a non-arg local is a 24-byte `String` — showed `""`).
**Next**: **heap** live edits (struct / vector / struct-enum — needs a
literal→live-store materialiser; `Stores::clone` empties `allocations`, so a value
built in the REPL store can't be aliased across) · the **G2** browser/DAP surface ·
the file-run (`prog.loft:42`) entry point.

This is the **purpose the REPL work serves**: the REPL is not standalone
dev-tooling, it is the *interactive surface of a breakpoint debugger*. The plan
composes three things loft already has or is building rather than building a
debugger from scratch:

- the **interpreter**, which can already step op-by-op and *suspend a frame* (the
  coroutine `yield` machinery snapshots the live stack frame);
- the **per-function interpret-over-a-compiled-baseline dispatch** (N9/C71
  shared-store dispatch + Goal-D backend parity) — the mechanism that flips one
  routine to the interpreter while the rest stays compiled;
- the **store-resident session environment** (@PLN14) — which, seeded from a paused
  frame instead of from typed bindings, *is* the breakpoint's variable view.

A breakpoint is then: *suspend the interpreter at a source line, expose the paused
frame as a REPL environment.* Everything new is the glue around that.

**Host-agnostic engine, multiple surfaces.** The debug *engine* (sub-arcs A–F) has
no UI and no browser dependency — it is a loft API: set/clear breakpoints, run,
pause, read/mutate the frame, evaluate against it, step, resume. It runs anywhere
loft runs:

- **terminal / CLI** — the **shipped REPL is the headless surface**: hit a
  breakpoint, drop into the terminal REPL at that frame. Nearly free — it is the
  REPL pointed at a paused frame instead of stdin.
- **browser** — its *natural home*: a DAP adapter + web UI (gutter breakpoints,
  variables panel, call stack, REPL console) make it rich and visual.
- **embedded** — a game's own console (the lavition "live interface" node) drives
  the same API in-process.

The browser is where it gets the best ergonomics; it is **not** where it lives. The
engine ships and is useful from a terminal before any browser UI exists.

## Goal

Set a breakpoint in a routine; loft flips that routine **and its live call-chain**
to the interpreter, pauses when the line is reached, and drops into a REPL whose
environment is the **paused frame's locals** — inspect and change live state,
evaluate against the frame, optionally break only when a condition/assertion fires,
then step or resume — driven by a **host-agnostic debug API** that a terminal, the
browser (its natural home), or an embedded console all consume, with the rest of the
program still running compiled.

## Effort + design

- **Effort:** VH — multi-phase execution-control + API + (per-surface) protocol/UI;
  spans the interpreter, the dispatch boundary, @PLN14's env, and a DAP surface.
- **Design:** — needs design (this README is the first cut); each sub-arc earns its
  own design doc as it unpauses.
- **Last touched:** 2026-06-08

## Why this isn't a bolt-on subsystem

The naive debugger is a second execution engine with its own frame model — the
brittle path. loft's is the *composition* above: the invariant is **one frame
model, one environment model, shared by the REPL session and the breakpoint
frame.** A paused frame's locals and a REPL session's bindings are the same
store-resident records (position-independent `DbRef`s); a conditional breakpoint is
just `eval` against that env. Hold that invariant and the debugger is a suspend
hook + a registry + a seed bridge; lose it and it's a parallel interpreter.

## Prior art — this is HotSpot's deoptimization model

The architecture matches how the JVM debugs (HotSpot + JDWP/JVMTI) almost
feature-for-feature — strong validation that the model is sound, not accidental.
The core identity is the same: a **mixed interpreter + compiled runtime where
debugging works by forcing the debugged code back into the interpreter**, because
the interpreter is the only mode with breakpoint hooks and introspectable frames.

| loft | Java / HotSpot |
|---|---|
| interpreted bytecode + compiled cdylib baseline (Goal-D parity) | interpreter + C1/C2 JIT (deopt preserves semantics) |
| **B/B2** — switch a fn to the interpreter to break in it | JVMTI `interp_only` / deoptimization |
| breakpoint = a check in the interpreter loop | JIT code has no hooks; the interpreter does |
| **`break_stack`** — frames + per-frame locals | JDI `StackFrame` + `visibleVariables` |
| **liveness reference-range** (`first_ref ≤ pc ≤ last_ref`) | LocalVariableTable scope start/length (`-g`) |
| **`set_frame_value`** → resume | `StackFrame.setValue` → continue |
| **`debug_step`** (line + call-depth) | JDWP step requests (INTO/OVER/OUT) |
| **`frame_holds`** — eval a condition against the frame | conditional breakpoints (JDI expression eval) |
| edit-and-continue / live editing (lavition) | HotSwap (`RedefineClasses`) |
| shared store across interpreted + compiled | unified heap shared by interpreter + JIT |

**The one capability loft is behind:** HotSpot can **deoptimize a frame already
running compiled, *on the stack*** — it reconstructs interpreter frames for live
JIT frames from debug metadata the JIT emits at safepoints (scope descriptors
mapping registers/stack slots to interpreter locals), so it can introspect and
step *out* through a caller that ran JIT-compiled.  loft today can only switch at
the **call boundary** — interpret a function from its *entry* (B2 re-wires before
it runs), never catch one mid-flight: a cdylib frame's locals live in native
registers with no safepoint metadata to reconstruct them.  The shared store gives
the *data* for free; what's missing is the *control-state* mapping mid-cdylib.
**This is exactly the B3-switching limitation** (and the "indirect caller already
compiled is non-introspectable" gap).

**Why loft's domain rarely needs on-stack deopt: the main loop provides re-entry.**
lavition runs a Rust main loop that calls into loft each frame, so a function is
**re-entered every iteration**.  Set a breakpoint → re-wire that function to
interpret → *resume*: the current (possibly compiled) frame finishes
un-introspected, and the **next loop iteration enters the interpreted path** and
breaks there.  You never reconstruct the live compiled frame — the loop hands you a
fresh interpreted entry ~one frame later.  This is precisely why HotSpot *needs*
on-stack deopt (the JVM can't assume a method runs again) and loft's frame-loop
domain doesn't: re-entry is free.  Edges: it assumes the program **loops** (a
one-shot batch run re-runs instead — cheap via state continuity); the next iteration
is the **evolved** state, not the exact frame (use a conditional breakpoint (E) to
catch a specific invocation); and a within-iteration step-out into an
already-compiled caller still waits one cycle.

**KNOWN — deferred, not dismissed (Q7).** True on-stack deopt — introspecting a
frame *mid-flight* without re-entry (a non-looping program, the *exact* current
invocation, step-out into a compiled caller *now*) — remains a real, unsolved
capability the loop only side-steps.  We may revisit it to get the general case
fully correct; it is tricky (needs cdylib-side safepoint metadata to reconstruct
interpreter locals from native registers — the hard part HotSpot pays for).

## Composition matrix — Stage A

The load-bearing claim: *a breakpoint pauses with the correct frame, and evaluating
/ mutating in the REPL-at-frame is faithful to the live program.* Probe before
building, on `--interpret`, **driving the engine API directly (no UI)**:

- **breakpoint site** — fn entry · mid-fn line · inside a loop body · inside a
  branch not taken (never fires) · last line before return.
- **frame contents** — locals of every type (scalars, text, struct, vector, enum,
  nested) visible + correctly typed in the REPL-at-frame; an unassigned-yet local
  (before its first write) reported as such, not garbage.
- **call-chain** — break N frames deep: the stack above is introspectable (each
  caller's locals + args), step-out lands in the caller's frame.
- **interpret scope** — a breakpoint reachable only through compiled callers still
  fires (the chain switched to interpret); a function with no path to any
  breakpoint stays compiled (no blanket slowdown).
- **condition / test** — a condition referencing frame locals breaks only when true;
  an assertion-style breakpoint fires on violation; a condition with a side effect
  runs once per hit, not per step.
- **mutate + resume** — change a local in the REPL-at-frame, resume, observe the
  program continue with the new value.
- **backend parity** — the *value* observed at a break equals what `--native` would
  compute for the same point (Goal-D parity is what makes interpret-on-break
  invisible).

Done when every cell is green from the **headless API** (so every surface inherits
it) and the probes graduate to a debugger regression suite.

## Sub-arcs

The host-agnostic engine is A–F; the surfaces are G.

| Item | Status |
|---|---|
| **A** — breakpoint registry + source-line → bytecode-offset map (reuse the per-op source positions the bytecode already carries) | **First cut** — **function-scoped** line breakpoints (`set_breakpoint_fn_line`, scoped to `[code_position, +code_length)` — a bare line matches that line in *every* function, stdlib included) + fn-entry, via `source_spans` / `code_position` |
| **B** — interpret-scope analysis: from a breakpoint set, compute the functions whose call-chain must interpret (static call-graph reachability; refine to on-demand frame-entry switching) | **B1 done** — `interpret_set(data, bp_fn)` = the bp fn + its **transitive callers**, a fixpoint over the static call graph. The graph comes from `collect_calls`, an **exhaustive** walk of all 34 `Value` variants (no `_` wildcard, so a new IR construct is a compile error, never a silently-dropped call edge — the under-reach hazard). Pure, no execution. **Limitation (verified):** `CallRef` (fn-ref / indirect calls) is unresolvable statically — `breakpoint_fires_in_indirectly_called_fn` proves the breakpoint still *fires* through a fn-ref (execution is fine), while `b1_does_not_trace_indirect_callers` proves the indirect caller is *absent* from the static `interpret_set`. So an interpreted middle reached via a fn-ref over compiled-deeper functions works at the *execution* layer; only static auto-discovery of its caller-chain fails — which is exactly what **B3 (on-demand, runtime-path-driven)** resolves. **B3 introspection facet done** — `break_stack(data)` walks the **live `call_stack`** and captures every frame, so the full chain to the break is introspectable *including indirectly-reached callers*: `b3_full_stack_includes_indirect_caller` shows the fn-ref caller `apply` appears with its locals (`x==5`), the very frame B1 could not link — the runtime path resolves the static gap. **B2 done** — `unmark_for_debug(data, bp_fn)` clears the default-native mark (`def.native`) on every **body-bearing** fn in `interpret_set(bp_fn)`, so the standard recompile routes their calls to the **interpreted** body. **Mechanism correction:** the compiled-vs-interpret choice is *codegen-time*, keyed on the callee's `def.native` (`codegen.rs:2542` — set → `OpStaticCall` cdylib bridge; empty + a user body → `OpCall` interpreted), so the switch is **un-mark + recompile**, **not** the `replace_static_fn` library swap the original note named — a library dispatcher (`fn(&mut Stores, &mut DbRef)`) has no `State`, so it cannot re-enter the interpreter to run a body.  *Body-bearing only:* a pure-cdylib fn (`code == Null`) stays marked — an absolute boundary.  Tests: `b2_unmark_switches_interpret_set_to_interpreted` (un-marks the bp fn + transitive callers, leaves an unreachable fn compiled, idempotent), `b2_pure_cdylib_fn_stays_an_absolute_boundary`.  **B2 is the engine-facing API** (the per-fn interpret engine calls it before recompiling a debug run); the all-interpreted REPL needs no wiring (user fns are already unmarked — no-op).  Full mixed-execution validation (a dispatched fn actually re-interpreting end-to-end) rides with the engine integration — same trigger as the switching facet below; the forward direction (marked → dispatch) is already proven by `tests/n2_cdylib.rs::f3_body_bearing_marked_fn_dispatch_vs_interpret`. **DEFERRED — B3 switching facet** (compiled→interpret *on-demand, mid-run*): needs a real mixed-execution scenario (cdylib loaded) + on-stack deopt for a frame *already* running compiled (Q7).  The main-loop re-entry side-steps it for lavition's frame-loop domain (re-wire → resume → next iteration enters the interpreted path); trigger = the lavition engine integration. |
| **C** — suspend hook: the bytecode loop pauses at a breakpoint offset and exposes the live frame (reuse the coroutine frame-snapshot machinery) | **First cut** — synchronous in-loop hook (record-and-continue); true suspend/resume for stepping is the enhancement |
| **D0** — frame read: capture the paused frame's variables (name → rendered value) from its slot table | **DONE** — captures **arguments + non-arg locals**, every value type (scalars/text inline, heap struct/vector/struct-enum via `show_loft`, simple enums by discriminant), via `render_frame_local`. Locals are **liveness-gated** by reference-range (`first_ref <= pc <= last_ref`) derived from `self.vars` (the codegen `code_pos → var_nr` map) — *safe* (a var in its read-range is necessarily assigned, never garbage) and it picks the right owner of a reused slot; the only gap is a defined-but-not-yet-read local before its first read (under-shows, never wrong). See Open question 6 (resolved). |
| **D1** — frame → REPL bridge: seed a `ReplSession` from D0's captured frame, so evaluating at a break runs against the live locals | **First cut (the headline, working)** — `ReplSession::seed_frame` binds each captured `(name, own-format literal)` as `name = <literal>`; `ReplSession::from_parser` builds the session over the program's parser so heap values' types are in scope. Eval-at-frame works end-to-end (`n + a == 21`, `pt.x * pt.y == 12`). **No @PLN14 dependency for read-eval** — the own-format literal *is* the bridge; exact value-for-value seeding + **frame mutation / write-back** (edit a local at the break, resume with it) is the @PLN14 upgrade. |
| **E** — conditional / test breakpoints: an expression/assertion evaluated in the frame env decides whether to break | **First cut** — `ReplSession::frame_holds(hit, condition)` seeds the frame (D1) and `assert`s the condition; a caller keeps only the hits where it holds (break when `n > 1`) or where an invariant is violated. Post-run filter on recorded hits; an in-loop *skip* (a non-matching breakpoint never even pauses) needs the condition pre-compiled against the frame — a refinement. |
| **F** — stepping + resume verbs (step over / into / out, continue) driving the bytecode loop — exposed as the **host-agnostic debug API** that all surfaces call | **DONE (first cut) — incl. edit-and-continue (the hard part)**. `enable_stepping` makes a breakpoint *suspend* (the execute loop returns to the driver, mirroring `frame_yield`, with the frame in `paused_frame`). `debug_step(StepMode)` resumes with **into / over / out / continue** by tracking the **source line** + **call depth** per op (*into* = next line at any depth; *over* = run a deeper call to completion, stop at the next line in the same/shallower frame; *out* = run to the frame's return). `set_frame_value` writes an edited value into the **live** frame slot; `resume` / `debug_step` then **picks it up**. Proven: edit `n` at a break → `calc(5)` returns 990; step *into* `inner`, *over* the call (result live), *out* to the caller. Works because loft calls are *jumps* (all execution state is in `State` fields, so the loop is freely re-enterable). `ReplSession::value_of` extracts the edited value (also the result-as-String primitive REPL.T deferred). |
| **G1** — terminal / CLI surface: drop into the **shipped REPL** at a paused frame (the near-free headless front-end) | **Interactive pause/step/edit landed** — the REPL `:break` command (**function-scoped**: `<fn>` body start via `State::set_breakpoint_fn_start`, or `<fn>:<line>`; `:break` lists / `clear` removes) and the **paused sub-mode**: when interactive stepping is on (the REPL driver enables it), the next call that reaches a breakpoint **suspends** — the live `State` is held on the session (`ReplSession::paused`), the prompt becomes `(dbg)`, and the frame is shown. At the pause: `:step`/`:next`/`:finish`/`:continue` (→ `debug_step`/`StepMode`), `name = <int>` edits the live frame (`debug_set` → `set_frame_value` + `refresh_paused_frame`), `:vars` re-shows it. On `:continue` to completion the observing statement prints its (edited) value. **The pause is a full REPL-at-frame:** any expression typed at `(dbg)` is evaluated against the live frame (`ReplSession::debug_eval` — swaps the session body to the frame's literal bindings and runs `value_of`, so it covers every type: `n * 3`, `pt.x * pt.y`, struct/vector reads), reusing the D1 bridge. New `Eval::Paused`; `ReplSession::debug_stepping/is_debugging/paused_frame/debug_set/debug_step/debug_continue/debug_eval/abort_debug`; `handle_paused` in `process_line` (wrapped in `catch_unwind` — a panic in the paused run abandons the debug session, never kills the REPL). Proven end-to-end through the binary (`tests/repl.rs::interactive_breakpoint_edit_continue` → `990`; `interactive_breakpoint_eval_expression`: `n * 3` → `15`) plus `tests/repl_session.rs` (`repl_interactive_break_edit_continue`, `repl_interactive_step_into_and_out`, `repl_interactive_eval_at_frame`). **Edit-and-continue covers every inline scalar** (`integer`/`float`/`single`/`boolean`/`character`) via `State::set_frame_literal` (type-directed parse + write, shared `frame_slot` lookup); `debug_set` evaluates the RHS against the frame first, so `n = n + 1` / `b = !b` work, and a type-mismatched edit is rejected. **Text + simple-enum live edits also land** (`set_frame_literal` `Type::Text` / `Type::Enum(_,false,_)` arms): a text **local** overwrites its owned `String` via `ptr::write` (no drop of the possibly-uninitialised prior slot — `reserve_frame` does not zero), a text **argument** repoints its 16-byte `Str` at a `Debugger::edited_text` buffer (stable for the run), and a simple enum writes its inline discriminant byte; gated on the local being shown at the pause. The same change fixed the text-**local** *read* (the renderer read every text slot as a `Str`, but a non-arg local is a 24-byte `String` — showed `""`), with a guarded read so an uninitialised local slot never crashes. A **heap** edit (struct / vector / struct-enum) is still rejected. Surfaced + fixed a latent round-trip bug: `render_frame_local` rendered a `float` `2.0` as bare `"2"` (re-typing as `integer` when the D1 seed re-binds it) — both it and `repl::float_literal` now share `state::loft_float_literal`. Covered by `tests/repl_session.rs::repl_interactive_edit_scalar_types` + `tests/repl.rs::interactive_breakpoint_edit_boolean`. **Record-and-continue stays** the off-stepping default (`last_hits`) for programmatic / conditional-breakpoint sweeps. **Why fn-scoped:** a bare or file:line number is *not* unique in the REPL — every input parses under the synthetic file `"<repl>"` with line numbers restarting at 1, so only the function is a unique anchor (a bare line is rejected with guidance). **`file:line` is unique only for file-based source** → it lands with the **file-run debugger** (a CLI entry point — break at `prog.loft:42`), a later slice. **Remaining G1:** **heap** live edits (struct / vector / struct-enum — a `DbRef` slot; reconstructing one in the *live* store from a literal needs a literal→store materialiser, since `Stores::clone` empties `allocations` so a value built in the REPL store can't be aliased across) and the file-run (`prog.loft:42`) entry point. **`debug_eval` on a *text* frame value now works** — the pre-existing REPL value-snapshot crash ([#293](https://github.com/loft-lang/loft/issues/293), fixed) double-freed when capturing a text value that borrowed a local `String`; the snapshot now routes text through a store-resident `vector<text>` wrap (`ReplSession::capture_typed`), so text edits accept any expression RHS. |
| **G1-span** — **breakpoint line resolution** (surfaced + **FIXED** during G1) | **DONE.** Breakpoints resolved against the sparse `source_spans` (emitted only at fault-prone arithmetic `+ - * / % << >>`), so a body with no such op (pure `if`/constant, a bare-var return, a plain assignment) had no breakable offset and silently never paused.  **Fix:** resolve against the dense **`line_numbers`** table instead — `code_pos → line`, emitted before the first op of *every* source line and **post-prologue** (lands after any `ReserveFrame`).  `set_breakpoint_fn_line` / `set_breakpoint_fn_start` / `breakable_lines` now use it, and stepping's per-op line uses a new `State::line_at` (was the sparse `source_loc_for`).  `source_spans` stays for runtime-error `file:line:col`.  Every body line is now breakable (`tests/debugger.rs::breakpoint_fires_without_arithmetic`, `breakable_lines_cover_non_arithmetic_lines`; the piped `interactive_breakpoint_edit_boolean` breaks a pure-`if` body).  **Method note:** the earlier "doesn't fire even at `code_position`" reading was a **stale-binary artifact** — `cargo build --release --lib` doesn't rebuild the `loft` *binary*, so the manual REPL probes ran old code; a fresh `--bin loft` build confirmed the fix. |
| **G2** — browser surface: `loft-dap` protocol + web UI (gutter, variables, call stack, REPL console) — the *natural home* | Open |

## Phase ordering

1. **A + C together** — a registry that can pause the interpreter at a line is the
   spine; nothing is observable until execution can stop at a point. *(first cut
   landed — the tracer-bullet slice.)*
2. **D0 (frame read)** — *done*: args + non-arg locals (liveness-gated), every
   value type. Extended `capture_break_frame` / `render_frame_local`; no new
   dependency.
3. **D1 (frame → REPL bridge)** — *first cut done*: a paused frame becomes an
   inspectable REPL via `seed_frame` / `from_parser`, evaluating against the frame's
   own-format literals. The @PLN14 store-resident env is the *upgrade* (exact
   value-for-value seeding + frame mutation/write-back), not a prerequisite for
   read-eval.
4. **F (debug API)** — *done*: suspend (`enable_stepping`) → step into/over/out
   (`debug_step`) → edit (`set_frame_value`, fed by `value_of`) → resume picks up
   the change. **G1 (terminal surface)** wraps A–F as the headless API; the REPL is
   already the front-end, so the engine ships useful before any browser work.
5. **B (interpret-scope)** — for breakpoints reached through *compiled* code (the
   only place it's needed; user code is already interpreted). **B1 done**
   (`interpret_set` reachability), **B2 done** (`unmark_for_debug` = un-mark
   `def.native` on the set + recompile → interpreted), **B3-introspection done**
   (`break_stack`). **Deferred**: B3 on-demand *switching* mid-run (on-stack deopt,
   Q7) → lavition engine integration.
6. **E (conditional/test)** — *first cut done*: `frame_holds` evals a condition
   against the seeded frame to filter recorded hits (thin, as predicted, once D1
   existed). In-loop skip is a refinement.
7. **G2 (DAP + browser UI)** — the protocol adapter (`loft-dap`, scoped under
   @PLN/09-lsp LSP.3) and the web-IDE surface consume the same API as G1.

## Open design questions

1. **Interpret scope — static vs on-demand.** "Everything that calls that routine"
   needs the *dynamic* call-chain interpreted so the whole stack is introspectable.
   Static (all transitive callers) is conservative and can engulf the program;
   on-demand (compiled until a frame that can transitively reach a breakpoint is
   entered, then switch that frame + descendants) is precise but needs a "can-reach
   a breakpoint" call-graph analysis. Lean on-demand; B builds the reachability.
2. **Pause granularity.** Per-source-line (DAP-natural) vs per-bytecode-op. Line is
   the user model; map line → the offset range and pause at the first op of the
   line.
3. **Frame mutation semantics.** Editing a local at a break writes the
   store-resident env; on resume the frame must read the edited value. Does the
   seeded env *alias* the live frame's slots, or is it a copy written back on
   resume? (Aliasing is faithful but couples the REPL env to the live stack; copy
   + write-back is cleaner but must reconcile.) Same store-copy-vs-alias question
   @PLN14 Q2/Q4 raises — resolve once, shared.
4. **Compiled frames above the break.** If a caller is compiled (not yet switched),
   its frame isn't introspectable. Does step-out across a compiled frame degrade
   gracefully (show "compiled frame, no locals") or does B guarantee the whole
   reached chain is interpreted? Lean on the latter (B's job).
5. **Coroutine reuse depth.** The suspend hook should reuse the coroutine
   frame-snapshot/restore path, not a parallel one — confirm a breakpoint pause is
   expressible as the same suspend primitive (it is a yield to the debugger).
6. **Local liveness in bytecode units (the D0 blocker) — RESOLVED.** The variable
   table's `first_def`/`last_use` are *sequence* numbers (IR-walk order), the wrong
   unit for the runtime `code_pos`. But no codegen addition was needed: `State.vars`
   already records `code_pos → var_nr` for every variable reference (`generate_var`
   reads + `generate_set` writes), so a per-var **bytecode reference range** falls
   out by scanning it within the function's `[code_position, +code_length)`. D0
   gates a local by `first_ref <= bp_pc <= last_ref` — *safe* (a var inside its
   read-range is necessarily assigned), and it picks the right owner of a reused
   slot. The map is **read-dominated** (scalar first-assignments don't record a
   write pc), so the only residual is a defined-but-not-yet-read local before its
   first read: it under-shows, never reads garbage. A true def-based gate (record
   the missing scalar-write pcs) is a future refinement only if that residual
   bites.
7. **On-stack deopt — true mid-flight introspection (KNOWN, deferred, hard).**
   Introspecting a frame that already ran *compiled* **without** waiting for loop
   re-entry (a non-looping program, the *exact* current invocation, or step-out into
   a compiled caller *now*). HotSpot does it by reconstructing interpreter locals
   from JIT **safepoint metadata**. loft's compiled path is a **native, optimized
   Rust cdylib**, so for loft this means **inspecting the live stack of a running,
   optimized Rust frame** — the genuinely hard part: optimized native code keeps no
   safepoint→local map, the optimizer elides / coalesces / inlines locals, and only
   DWARF (lossy under `-O`) describes them. The **main loop side-steps it** for
   loft's domain — re-entry is free (see § Prior art) — so revisit only if the
   non-loop / exact-frame case becomes load-bearing. The realistic path is then
   **loft-emitted safepoints + debug metadata in the cdylib at known points** (the
   HotSpot bargain), *not* general optimized-Rust-stack walking.

## Cross-arc dependencies

- **@PLN12** (REPL + introspection, **finished**) — the interactive surface; the
  REPL-at-frame is a `ReplSession` over the paused frame, and the terminal surface
  (G1) is that REPL directly.
- **@PLN14** (store-resident REPL session) — **load-bearing**: the breakpoint frame
  *is* the store-resident environment. @PLN14 now carries the sharpening that its
  env must be seedable from an arbitrary live frame, not only typed bindings (sub-D1
  here consumes it).
- **N9/C71 per-function interpret dispatch + Goal-D parity** — the interpret-scope
  switch (B) and the invisibility of interpret-on-break (parity).
- **Coroutine suspend machinery** — the frame-snapshot primitive C reuses.
- **`loft-dap` / 09-lsp LSP.3** — the browser protocol surface (G2);
  **25-native-debug** is the `--native` GDB/LLDB complement (this plan is the
  interpreter-mode debugger); **lib_plans/07-web-ide** is the browser UI host.

## See also

- [LAVITION.md](../../LAVITION.md) — the Live-interface / State-continuity /
  Fault-containment continuity nodes this plan realizes.
- [@PLN14 store-resident REPL session](../14-store-resident-repl-session/README.md)
  + [CONVERGENCE.md](../14-store-resident-repl-session/CONVERGENCE.md) — the frame
  environment model.
- [lib_plans/future/09-lsp/](../../lib_plans/future/09-lsp/README.md) — `loft-dap`
  (LSP.3) protocol; [plans/future/25-native-debug/](../future/25-native-debug/README.md)
  — the native-mode complement.
- **Tracker:** to be filed as `@PLN15` on `loft-lang/plans`; labels `plan` ·
  `subject:loft` · `status:future`.
