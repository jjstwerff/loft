<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# loft-dap advanced debug tools — design (@PLN63 LSP.3 follow-ups)

> **Identity:** a design sub-doc of `@PLN63` (loft-lsp), companion to
> [DAP.md](DAP.md) (the D0–D6 DAP MVP, LANDED). This doc designs the four advanced
> debug tools the MVP deferred, each as a **small-step spine** landable behind the
> existing `tests/dap_transport.rs` harness.
> **Status:** DESIGN — no code yet. The MVP was a *pure translation* over the `--rpc`
> engine (`src/rpc.rs`); three of these four need ONE engine step first (the RPC surface
> genuinely lacks the capability), so each spine is split **engine → RPC → DAP**.

## Why this doc exists — the MVP's honest leaves

The D0–D6 adapter surfaces the engine's flat, single-frame, forward-only pause: every
local is a leaf, the stack is one frame, there are no data breakpoints, and there is no
stepping backward (DAP.md § Refusals). Each boundary was an honest capability bit or a
clean error, never a wrong picture. This doc turns each into a working tool.

**These are not adapter oversights — three are engine gaps.** Before designing, each
candidate was probed on the proven `loft debug --rpc` path (the same chokepoint loft-dap
drives). The probes are the grounding — and the reason two "obvious" wirings (reverse
stepping via the existing `undo`; data breakpoints via the existing `setWatch`) would
have shipped a *wrong picture*:

| Tool | Probe | Result | What it means |
|---|---|---|---|
| **VE** variable expansion | `eval` of a struct / vector local at a stop | `{"value":{"x":9,"y":2}}` / `[10,20,30]` (proven by `tests/rpc.rs`) | **No engine change.** `debug_eval_json` already walks values → adapter-only. |
| **SF** multi-frame stack | — (`call_stack: Vec<CallFrame>` exists, `src/state/mod.rs:164`) | the data is present but `frame_field` (`rpc.rs:408`) renders only the top `BreakHit` | Engine step: **surface `call_stack`** on the RPC, then translate. |
| **DB** data breakpoints | `setWatch {expr:"x"}` on a stack local `x` | `"not a watchable scalar region"` | `resolve_watch_region` (`mod.rs:3243`) resolves only **store/heap** DbRefs; a stack scalar isn't an allocation. Engine step: **watch stack scalars**. |
| **RX** reverse execution | `undo` after a `stepOver` | `"nothing to do"`, frame degraded to `replmain_1` | The engine's `undo`/`redo` is @PLN16 **M2 reverse-*edit*** (revert a `setValue` journal, `debugger.rs:107`) — **not** reverse execution. Needs real execution checkpointing. |

**Order of work — by cost, cheapest first (VE ships alone; the rest each stack one engine
step):** VE → SF → DB → RX. VE and SF are the highest value per unit cost (they make the
locals panel and call stack real); DB is medium; RX is the largest (a new engine
capability) and is scoped last.

The invariant that binds all four (inherited from DAP.md): **loft-dap adds no debug
semantics.** Every advanced tool is the engine's own capability surfaced on the RPC and
translated — if the engine can't do it truthfully, the adapter refuses it, never fakes it.

---

## VE — structured variable expansion (adapter-only) — **BUILT**

> **Status: LANDED.** `src/bin/loft-dap.rs` (`build_variables`/`expand_node`/`mint_value`);
> gate `tests/dap_transport.rs::variable_expansion_walks_structs_vectors_and_nesting`
> (VE0–VE3). No engine change, as designed. **Known limit (a `frame_field` issue, not VE):**
> a **bare top-level heap-vector local** appears in the frame under its `__vdb_N` compiler
> backing (or is absent), so it may not show under its source name — VE still expands
> whatever IS shown and evaluable, and a vector **nested inside** an evaluable value (a
> struct field, an element) expands fully via the JSON tree. Making the top-level frame
> locals source-faithful for heap vectors is a separate frame-fidelity follow-up (see SF's
> per-frame locals, SF1).

**Gap.** Every `variables` entry is a leaf (`variablesReference: 0`); a struct or vector
local shows its one-line rendering with no drill-in (DAP.md Decision 3).

**Why it's free.** `debug_eval_json(expr)` (`repl.rs:2261`) evaluates any expression in the
paused frame and returns its value as JSON — a scalar, or a nested object/array — proven
for structs (`rpc_eval_struct_as_json`), bare vectors (`rpc_eval_bare_vector_live`), and
keyed collections (`rpc_eval_in_a_keyed_collection_frame`). The DAP `variablesReference`
tree IS a JSON tree; the adapter walks it.

**Invariant.** A `variablesReference` expands to **exactly** the immediate JSON children of
the value `debug_eval_json` returns for that node — no synthesized structure — and a node
is a leaf **iff** its JSON value is a scalar. *Falsify:* expand a struct → its fields and
only its fields; expand a scalar → no expand arrow (ref 0).

**Chokepoints.** `debug_eval_json` via the RPC `eval` verb (already driven by loft-dap's
`evaluate`); the adapter's per-stop handle table (mint/invalidate, DAP.md Decision 3).

**Steps.**

- **VE0 — expandability at the top level.** In the Locals `variables` handler, for each
  local `name`, drive RPC `eval name`; if the value parses as a non-empty JSON object/array,
  mint a `variablesReference` and register the value; else keep `0`. Display value stays the
  flat-frame rendering (unchanged). *Gate:* at a stop with a struct local `p` and a scalar
  `a`, `p.variablesReference != 0` and `a.variablesReference == 0`.
- **VE1 — one-level expansion.** `variables {ref}` for a registered handle returns the JSON
  value's immediate children: object → `{name: field, value: render(child)}`; array →
  `{name: "[i]", value: render(child)}`. A child that is itself object/array gets its own
  minted handle (VE2 walks it); a scalar child gets `0`. *Gate:* expand `p` → `x=9, y=2`;
  expand `nums` → `[0]=10, [1]=20, [2]=30`.
- **VE2 — arbitrary nesting, no re-eval.** Cache the parsed JSON **tree** at the handle;
  a child handle points at a sub-node of the cached tree, so deeper expansion navigates the
  tree in memory (one `eval` per top-level local, none per drill-down). This sidesteps the
  keyed-collection edge (a hash child's path expression isn't a field access) — children are
  read from the parent's tree, never re-evaluated. *Gate:* expand a `vector<Struct>` →
  element → its fields (two levels), values correct.
- **VE3 — handle lifetime.** Invalidate the whole handle table on every resume and mint a
  fresh generation per stop (reuse the `locals_ref` invalidation VE inherits): a stale child
  reference after a resume returns empty, never a wrong subtree. *Gate:* expand at stop A,
  resume, reuse the handle → empty.

All four steps are loft-dap-local; no engine or RPC change — **all four LANDED.** The gate
drives the reliable path (a struct `sq` shown by name → expand its vector field `members`
and nested struct field `lead` → their leaves, through the cached JSON tree). Note VE2's
tree-cache is load-bearing, not just an optimization: a probe showed `eval sq.members`
returns `null` (a direct vector field-access doesn't evaluate), so children MUST be read
from the parent's cached tree, never re-evaluated by path.

---

## SF — multi-frame stack trace (engine → RPC → DAP)

**Gap.** `stackTrace` returns the current frame only; a call three deep shows one frame
(DAP.md § Risks). The client's call-stack panel is effectively blind.

**Why the data is there.** The paused `State` holds `call_stack: Vec<CallFrame>`
(`mod.rs:164`); each `CallFrame` carries `d_nr` (the called function → its name via
`Data`), `call_pos` + `line` (the call-site source line, TR1.4), and `args_base` +
`args_size` (the frame's argument region). `frame_field` (`rpc.rs:408`) renders only the
top `BreakHit`; TR1.3's `n_stack_trace` snapshot (`mod.rs:653–722`) already walks
`call_stack` for a *runtime* stack-trace value — the same walk, for the debugger pause.

**Invariant.** The DAP stack is the engine's `call_stack`, **one `StackFrame` per
`CallFrame`, innermost first**, each frame's `name`/`line` read from its `CallFrame` — never
a synthesized, reordered, or truncated stack. *Falsify:* a 3-deep call chain
`main → a → b`, breakpoint in `b` → exactly `[b, a, main]` with each frame's call-site line.

**Chokepoints.** `State::call_stack` + `Data` def names; a new engine renderer beside
`paused_frame`; a widened RPC `stackTrace`; loft-dap's `stackTrace` fan-out.

**Steps.**

- **SF0 — engine: render the frame list.** Add `ReplSession::paused_stack() -> Vec<Frame>`
  that walks the paused `State.call_stack` innermost-first, resolving each `d_nr` to a
  function name and each frame's source line (reuse the TR1.3 line-resolution at
  `mod.rs:679–717`). *Gate (unit):* a 3-deep pause returns 3 frames, names + lines correct.
- **SF1 — engine: per-frame locals.** Extend each rendered frame with its locals, reusing
  the `BreakHit` slot-read that renders the top frame today (read the frame's slot region at
  `args_base`); a frame whose slots aren't recoverable yields an empty locals list (honest),
  never wrong bytes. *Gate:* the caller frame's parameter shows its value.
- **SF2 — RPC: widen `stackTrace`.** The `stackTrace` verb (`rpc.rs:320`) returns
  `{"frames":[{function, line, locals}, …]}` (was the single `frame`); keep the old single
  `frame` field too for one release (additive — the compatibility rule). *Gate:* `tests/rpc.rs`
  asserts the multi-frame array for a nested call.
- **SF3 — DAP: fan out the frames.** loft-dap's `stackTrace` emits one `StackFrame` per RPC
  frame with a **distinct `frameId`** (e.g. `1000 + depth`); `scopes`/`variables` key off the
  requested `frameId` so the client can inspect any frame's locals, not just the top. *Gate:*
  `tests/dap_transport.rs` walks a caller frame's variables.
- **SF4 — the current-frame source marker.** The top frame carries the parked line; the
  rest carry their call-site line (so the editor underlines the call in each caller). *Gate:*
  the caller frame's `line` is the call site, not the callee's body.

SF depends on nothing but itself; it is the second increment (highest value after VE).

---

## DB — data breakpoints via watchpoints (engine → RPC → DAP)

**Gap.** DAP data breakpoints ("break when this variable changes") aren't offered. The
engine HAS watchpoints (`add_watchpoint`, `poll_watchpoints`, `mod.rs:3242`+) and a watch
hit already rides the RPC as `stopped{reason:"watch"}` → loft-dap already maps that to
DAP `data breakpoint`. **But** `resolve_watch_region` resolves only store/heap DbRefs, so a
plain stack local `x` returns `"not a watchable scalar region"` (probed) — the common case
fails.

**Invariant.** A data breakpoint fires **iff** the watched scalar's bytes change (stack OR
heap); a target that cannot be watched comes back `verified: false` — never a silent miss,
never a false stop. *Falsify:* watch a local, mutate it a line later → a stop with the old →
new value; watch an unwatchable expression → `verified:false`, no stop.

**Chokepoints.** `resolve_watch_region` + `poll_watchpoints` (`mod.rs:3243`, `3267`); the
`Watchpoint` struct (`debugger.rs`); RPC `setWatch`/`clearWatch` (already present); DAP
`dataBreakpointInfo` + `setDataBreakpoints`.

**Steps.**

- **DB0 — engine: watch a stack scalar.** Give `Watchpoint` a region **variant**: the
  existing store region `(store_nr, rec, off, len)` OR a new **stack region** (the paused
  frame's absolute slot address + width). Extend `resolve_watch_region` to resolve a scalar
  local to its stack slot when it isn't a store DbRef. *Gate (unit):* `add_watchpoint("x")`
  on an integer local returns `true`.
- **DB1 — engine: poll the stack region.** `poll_watchpoints` reads the stack region's bytes
  each stepped op (mirror the store `read_span`), emitting the same `WatchHit{label, old,
  new}`. A watch whose frame has returned (slot gone) is dropped, not read stale — mirror the
  freed-store skip (`mod.rs:3274`). *Gate:* mutating the watched local fires exactly one hit.
- **DB2 — RPC: verified feedback.** `setWatch` already returns ok/err; surface the
  resolvability as a `verified` flag per watch so a dead data breakpoint is reported at set
  time (mirror `setBreakpoints`' `verified`). *Gate:* `tests/rpc.rs` — a watchable local
  verifies, an unwatchable expression does not.
- **DB3 — DAP: `dataBreakpointInfo`.** Answer with `{dataId: <expr>, description, accessTypes:
  ["write"]}` when the expression resolves; `{dataId: null}` otherwise (the client then greys
  out the option). Advertise `supportsDataBreakpoints`. *Gate:* info on a local returns a
  `dataId`; info on a literal returns null.
- **DB4 — DAP: `setDataBreakpoints`.** Replace the watch set: RPC `clearWatch`, then one
  `setWatch` per `dataId`, returning `{breakpoints:[{verified}]}`. A watch hit → the existing
  `stopped{reason:"data breakpoint"}`. *Gate:* `tests/dap_transport.rs` — set a data
  breakpoint on a local, continue, assert the stop + the changed value.

DB is self-contained after DB0–DB1; the DAP half (DB3–DB4) is a thin translation.

---

## RX — reverse execution (the large one: engine checkpointing → RPC → DAP)

**Gap.** No stepping backward. The probe confirms the engine's `undo`/`redo`
(`debugger.rs:107`) is **reverse-*edit*** — it reverts a `setValue` journal, and returns
`"nothing to do"` after a *step* because a step records no edit journal. True reverse
execution needs the engine to remember prior execution **states**, which it does not.

**Design decision — reuse the journal, don't snapshot the world.** loft already journals
store mutations (`crate::database::Journal`, the M2 edit machinery, `mod.rs:2687`+). Reverse
execution generalizes that from *edits* to *steps*: journal the store mutations **and** the
stack delta of each executed step, so a step can be reverted by replaying its journal
backward. This avoids a full-State snapshot per step (unaffordable) and builds on proven
infrastructure. **Open question to settle with a probe first (RX0):** the checkpoint
granularity — per source-line (step) vs per opcode — and the memory ceiling of an unbounded
timeline (likely a bounded ring, "reverse up to N steps", surfaced honestly).

**Invariant.** `stepBack` lands on **exactly** the state before the last forward step —
frame, locals, stack pointer, and store contents byte-identical to never having taken it.
*Falsify:* snapshot the frame + all locals, `next`, `stepBack`, re-snapshot → byte-identical;
then a store-mutating step (`v[0] = 9`), `stepBack` → the store reverts too.

**Chokepoints.** `Journal` + `begin_edit_journal`/`commit`/`debug_undo` (`mod.rs:2687`–`2720`,
`debugger.rs:107`); the step loop `debug_step` (`repl.rs:2158`); RPC `undo`/`redo` (already
wired, `rpc.rs:318`); DAP `stepBack`/`reverseContinue`.

**Steps.**

- **RX0 — the falsification probe FIRST.** Before any engine change, build the byte-identity
  probe above as a `tests/` harness that **fails** today (proving reverse execution is
  absent), and measure journal size per step on a representative program — this sizes the
  timeline and picks the granularity. (The @PLN16 matrix-first rule: earn the fix.)
- **RX1 — engine: journal a step.** Arm the store journal around each `debug_step` (not just
  around an edit), and capture the stack-bytes delta, into a per-step timeline entry. *Gate
  (unit):* one step produces one revertible timeline entry.
- **RX2 — engine: `step_back`.** Revert the top timeline entry (store journal backward +
  restore the stack delta + rewind the bytecode position + refresh the paused frame). Redo
  replays it forward. *Gate:* the RX0 byte-identity probe now PASSES on both backends.
- **RX3 — engine: bounded timeline.** Cap the timeline at N entries (a ring); reverting past
  the cap returns a clean "no earlier state" (not a wrong one). Surface N. *Gate:* N+1 steps
  then N+1 `step_back`s — the last reports the floor, no corruption.
- **RX4 — RPC: reason + frame on `undo`/`redo`.** `undo`/`redo` return the refreshed frame
  (they do today) plus a `reason` so the adapter can label the stop; unchanged wire shape
  otherwise. *Gate:* `tests/rpc.rs` — undo after a step returns the prior frame (was `"nothing
  to do"`).
- **RX5 — DAP: `stepBack` / `reverseContinue`.** Advertise `supportsStepBack`; `stepBack` →
  RPC `undo` → `stopped{reason:"step"}`; `reverseContinue` → `undo` to the timeline floor →
  `stopped`. *Gate:* `tests/dap_transport.rs` — step forward, `stepBack`, assert the line +
  locals returned to the prior stop.

RX is the deepest change (a new engine capability) and is scoped last; RX0's probe is the
gate on whether the journal-reuse approach holds before investing in RX1–RX5.

---

## Not designed here (still honest refusals)

- **`pause` (async interrupt).** Orthogonal to these four — it needs an interrupt/signal
  path into the running interpreter, not a data or history capability. Remains a clean "not
  supported" error until an interrupt mechanism exists (its own plan).
- **Multi-worker `par` threads.** One synthetic thread today; one-per-worker rides SF's
  multi-frame machinery once `par` worker frames are surfaced — a follow-up over SF.

## See also

- [DAP.md](DAP.md) — the D0–D6 MVP these extend (the translation invariant, the envelope,
  the flat frame each tool drills into).
- [@PLN16](../../plans/16-debugger/README.md) — the debug engine (A–F, M1–M5); RX generalizes
  its M2 edit-journal, DB extends its M3 watchpoints, SF surfaces its `call_stack`.
- [16.P PROTOCOL.md](../../plans/16-debugger/PROTOCOL.md) — the RPC contract each spine widens.
- `src/rpc.rs` — `DebugDriver`/`handle` (the chokepoint), `frame_field` (the flat frame SF
  and VE replace), `report` (the stop/event path DB and RX ride).
- [STACKTRACE.md](../../STACKTRACE.md) — TR1.3 `vector<StackFrame>` (the call-stack walk SF0 reuses).
