<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 86 — sandbox-subset-flag: total-sublanguage sandboxing for user scripts

## Status

**Open — full design (total-sublanguage model), no implementation.** Plan **@PLN86**
([loft-lang/plans#86](https://github.com/loft-lang/plans/issues/86)) · `status:future`
· `subject:loft`. The concrete first slice of [SANDBOX.md](../../SANDBOX.md): a
compiler-enforced flag that runs **designated subsets** of a program (user scripts)
under a *prove-it-safe-at-load* policy, while the surrounding host code runs
unrestricted, in one process, sharing the store at full `DbRef` speed.

**Scope.** v1 is intentionally **fairly restricted** — a small total dialect (bounded
loops, no/structural recursion, total ops) plus a **curated host API**; the
expressiveness comes from the *API* (the safe operations the host hands modders), not
from language permissiveness — restricted-language + rich-API still expresses most game
situations. Admitting *more* complex scripts via deeper termination/safety analysis
("involved code inspection") is **a separate future plan**, out of scope here.

## Goal

Let a host (a game engine, an in-game console, a player playground) embed user scripts
with fast, direct data access **without letting them break the host**. The pivotal
realization: do **not** *catch* bad behaviour at runtime — a runtime abort breaks the
game and a copy-everything rollback fails efficiency. Instead **prevent** it at
admission: only admit scripts that are statically proven safe; an admitted script then
**always runs to completion**.

## The invariant (the one thing every step defends)

> **Every *admitted* script is (a) capability-bounded — reaches only allowed groups;
> (b) *total* — statically proven to terminate on all inputs; (c) built from *total
> operations* — no operation can fault; (d) able to write only through
> invariant-preserving capability operations — no raw mutation — so its writes can
> never corrupt the host.** So an admitted script always runs to completion and only
> ever leaves the world in a *valid* state; nothing that could hang, escape, fault, or
> corrupt is ever admitted. **Rejection happens only at LOAD — never a runtime abort,
> and there is no rollback (if a design needs one, the whole idea is dropped).**

## Why this shape — the design forces

- **No aborted scripts** (normal *or* user-provided): a runtime kill is itself a way
  to break the game, so boundedness must be **proven, not caught**.
- **Rollback-by-copy fails efficiency**: snapshotting the world per run is O(world).
  But with nothing to abort, there is **nothing to roll back** — scripts write straight
  to live state. The expensive copy disappears.
- **Verified substrate:** the interpreter is a **bytecode VM with an explicit
  `call_frames: Vec<CallFrame>` stack** (`src/state/mod.rs`), *not* Rust recursion —
  `r(200000)` returns a clean `error: call stack overflow`, never a segfault. So the
  worst case is always a clean error, never a host crash, and bounded execution is
  enforceable; we aim never to reach that backstop, but it can never crash the engine.

## Design

### 1. The host designates subsets — not the script
Authoritative from the trusted side, so a script can't opt itself out:
- **Manifest / CLI (primary):** `loft.toml` `[sandbox]` maps file/function globs to a
  named profile.
- **Source annotation (secondary):** `#sandbox("mod-script")`, honoured only for
  sources the host already trusts; the manifest always wins.

The compiler tags each `def` with its profile (or "unrestricted") and admits the
**sandbox-reachable set** — the closure over **references** (calls + fn-ref literals +
type uses), so indirect calls can't escape (L4).

### 2. The boundary (sandboxed ↔ unrestricted)
| Direction | Allowed? |
|---|---|
| sandboxed → another sandboxed def | yes |
| sandboxed → an allow-listed host symbol | yes — runs unrestricted as a vetted primitive; the primitive must itself be total/bounded (host contract, L-host) |
| sandboxed → a symbol outside the allowed groups / native FFI / file·net·env | **compile error** |
| unrestricted → a sandboxed def | yes — and the call always returns (the script is total) |

### 3. Capability groups — declared in the code, never a drift-prone list
Each API function/method/type carries a `#cap "<group>"` tag **at its definition**
(beside `#native`/`#rust`); a profile selects **groups**, never names — a function's
capability is a fact that lives with the function (one home per fact,
[STABILITY_REDFLAGS.md](../../STABILITY_REDFLAGS.md)).

```
fn read_file(path: text) -> text;  #native "n_file_read"  #cap "fs.read"
pub fn get(self: vector, i: integer) -> ...;              #cap "collections.read"
pub fn clear(self: vector);                                #cap "collections.write"
```
```toml
[profile.mod-script]
backend = "interpret"; native_ffi = false        # never native/rustc, no cdylib
allow_caps = ["game.read", "game.write", "math", "collections.read", "text"]
# fs.*, net, env, collections.write → tagged outside → denied. Prefix: "game" ⊇ game.*.
```
The group travels with the symbol; an **un-tagged symbol is denied** (forces
classification, L3-cap).

### 4. Totality admission (the core)
A sandboxed def is admitted only if the compiler can prove it **total** — terminates
on every input — from a restricted but useful form:
- **Bounded loops only.** `for x in <finite collection>` / `for i in 0..N`. An
  unbounded `while` is rejected unless it carries a compiler-checked **decreasing
  variant** (a measure that strictly drops each iteration toward a floor).
- **Well-founded / bounded recursion.** Recursion only on a **structurally decreasing**
  argument (a smaller sub-structure), or an acyclic call graph (depth bounded by the
  longest path). Otherwise rejected.
- **Total operations.** Every operation has a defined result on every input — loft
  already does OOB index → `null`; division/modulo by zero, integer overflow, etc. get
  the same total treatment (a defined sentinel/saturating/widening result) or are
  excluded from the dialect. **No operation may fault** (L3).
- **Worst-case complexity is computed, not bounded-by-kill.** Admission derives the
  script's step/depth cost as a function of input sizes (e.g. `O(entities)`); the host
  bounds those inputs (entity counts, structure depth) so a single run can't stall a
  frame. This is a *contract + a reported complexity*, not a runtime fuel-kill.

Anything that can't be proven total is **rejected at load**, with the reason.

### 5. No rollback — restrict the writers instead (the efficiency anchor)
**Rollback is out of scope: if any path needs to undo a script's effects, the whole
idea is dropped.** Instead, make every write a sandboxed script can do **safe by
construction**, so direct writes to live state are always valid and there is nothing
to undo:
- sandboxed code has **no raw store mutation** — it cannot assign struct fields or
  store cells directly. Its only writes are through **invariant-preserving,
  capability-gated host operations** (the `*.write` groups): each validates its inputs
  and leaves the host's invariants intact.
- a profile grants *which* write ops via `allow_caps`; each granted op is vetted safe
  (the L-host contract, extended to writes). **Any sequence of them keeps the world
  valid**, so order/interruption can't matter — and admitted scripts never interrupt
  (they're total).
- writes therefore land **directly on live state** (fast — the performance premise);
  a script's *logic* may produce an unwanted-but-valid state (the modder's
  responsibility), **never an invalid/corrupt one**.
- there is **no runtime resource guard, no abort path, no rollback, no copy, no journal
  on the safety path.** The VM's clean `call stack overflow` / step ceiling is a
  defense-in-depth backstop (a clean error, never a crash), targeted never to fire
  inside the host's input envelope.

### 6. Admission diagnostics — the modder's feedback loop (errors *before* it runs)
Because every safety property is decided at load, the modder iterates against
**admission errors** — and they must be **correct, specific, and actionable**, pointing
at the exact construct and the rule it breaks, *before* the script is ever allowed to
run. Each rejection class names its fix:
- **denied capability** → the symbol + its `#cap` group + the profile's allowed set;
- **totality violation** → the unbounded loop / non-well-founded recursion + how to
  bound it (a range, a decreasing variant, a structural argument);
- **non-total op** → the operation + its total alternative;
- **forbidden write** → the write op + the granted write groups.

A script that compiles clean is then guaranteed safe at runtime — **the admission
errors are the contract.** This diagnostic quality is a first-class deliverable, not an
afterthought: it is the entire developer experience of writing a mod.

## Compile-time vs runtime

| Concern | Where |
|---|---|
| capability groups · FFI/backend · reference-closure · **totality** · total-op check · worst-case complexity | **compile** — reject at load |
| executing a proven-total script + delivering its result | **runtime** — and there is **no abort path** to design fail-safe |

A compile-time rejection is at *load*, outside the frame — intrinsically game-safe.
The runtime does nothing but run a script that was already proven to terminate, fault
nothing, and touch only allowed capabilities.

## Load-bearing claims — and the probe that falsifies each

- **L1 — bounded execution can't crash the host.** *Status:* **VERIFIED** —
  `r(200000)` → clean `call stack overflow` (not segfault); execution is an explicit
  `call_frames` VM stack, not Rust recursion.
- **L2 — totality is statically decidable for the admitted form, and the form is
  expressive enough.** *Risk:* unsound (admits a non-terminating script) or too
  restrictive (modders can't write useful logic). **Probe:** the checker admits
  `for e in entities {…}` + structural recursion, rejects `while true {}` and unbounded
  recursion; a *real game-script corpus* fits the form (extract from moros/dryopea).
- **L3 — every operation can be made total.** *Probe:* enumerate the partial ops
  (div/mod-zero, integer overflow, conversions, …); each gets a defined total result or
  is excluded — none can fault. (OOB→`null` already holds.)
- **L4 — capabilities are complete (indirect calls can't escape).** *Risk:* a fn-ref to
  a denied symbol called indirectly. *Mitigation:* admission closes over **references**
  (`let f = read_file` is a reference → caught) **and** allow-listed host APIs must not
  hand untrusted code an arbitrary fn-ref (host contract). **Probe:**
  `let f = read_file; f("/etc/passwd")` rejected naming `fs.read`.
- **L5 — worst-case complexity is computable and inputs are host-bounded.** *Risk:* a
  total-but-`O(huge)` script stalls a frame. *Mitigation:* admission reports the
  complexity; the host bounds the inputs. **Probe:** admission reports `O(entities)` for
  a per-entity loop; a loop over an unbounded source is flagged for an input bound.
- **L-host — allow-listed host primitives are themselves total/bounded.** A trusted API
  the script may call must not hang or fault (no runtime abort exists to save it).
  *Probe:* the `#cap`-tagging review gate for an API includes "bounded on any script
  input".
- **L-write — every granted write op is invariant-preserving, so direct writes never
  need undo.** *Risk:* a `*.write` op that can leave the host corrupt → rollback would
  be needed → the whole idea is dropped (§5). *Mitigation:* the `#cap`-tagging gate for
  a `*.write` op requires "leaves all host invariants intact on any input"; sandboxed
  code has no raw mutation, so these ops are the *only* write surface. *Probe:* each
  granted write op holds its invariant under adversarial inputs; a raw field-assign in a
  sandboxed def is rejected at admission.

## The verifiable step ladder

Each rung is independently verifiable (RED before, GREEN after). Admission + the parse
guard are compile-time (backend-agnostic); the only runtime work is executing a
proven-total script.

- **P0 — parser nesting bound (load-time).** A nesting-depth guard in the
  recursive-descent parser, active **only while parsing a sandboxed def's body** (needs
  the parse-time "in a sandboxed def" flag from 1.2). *Verify:* 5000-deep nested input
  in a sandboxed def → clean **parse error**, not `rc=139`; trusted source unaffected.
  *(A load-time rejection, not a runtime abort.)*
- **P1 — designation + reachable set + FFI/backend ban.** Tag defs with a profile;
  compute the reference-closure; force interpret, reject `[native]` libs. *Verify:* a
  sandboxed `--native`/native-lib use rejected; the reference-closure puts
  `let f = g`'s `g` in the set (L4).
- **P2 — capability groups + group admission + no-raw-write.** `#cap` annotation + tag
  the stdlib/API surface + group-membership check; **reject raw store/field mutation in
  sandboxed code** — writes only through allow-listed `*.write` ops (§5). *Verify:*
  `read_file`/`Vector.clear` (denied groups) rejected naming the group; a raw field
  assign `e.health = 0` in a sandboxed def rejected (must use a granted write op);
  `Vector.get` passes; the L4 indirect probe rejected. **Every rejection names the
  symbol/group + the allowed set** (§6 — the modder's error contract).
- **P3 — totality admission (the core).** Bounded-loop + well-founded-recursion +
  total-op analysis over the sandbox-reachable set; reject non-total at load; report the
  worst-case complexity. *Verify (L2/L3/L5):* `while true {}` / unbounded recursion
  rejected; `for e in entities {…}` + structural recursion admitted with a reported
  complexity; a div-by-zero in a sandboxed def is total (sentinel), never a fault.
- **P4 — execution + result delivery.** `run_script(src, policy, world) -> Result` runs
  the proven-total script and returns its value; **no guard, no abort path** — the only
  failure surface is a *load-time* admission failure. *Verify:* an admitted script runs
  to completion and writes live state; there is no code path that aborts a running
  admitted script.
- **P5 — performance gate.** *Verify (S6):* a sandboxed script reading/writing N
  entities/frame stays within a frame budget vs a native baseline — with **direct
  writes, no rollback, no copy, no journal** (the efficiency premise — if this needs a
  rollback to be safe, the design is wrong, per §5).

Throughout, **P2/P3 admission errors are a first-class deliverable** (§6): each is
tested to name the exact construct + rule + fix, so a clean compile is the safety
contract.

## Open questions

- **Totality-checker expressiveness (scoped for v1)** — v1 = *bounded loops + no /
  structural recursion* (the simplest decidable form); the expressiveness lever is the
  **API**, not the language. Admitting more via deeper code inspection (general
  termination analysis) is **a separate future plan**, not this one.
- **Total-op treatment** — div/mod-zero → `null` vs a saturating/sentinel value vs an
  error-value the script must handle; overflow → wrap vs saturate vs widen. Each must be
  *defined*, never faulting.
- **Input-bounding contract** — how the host caps collection sizes / structure depth so
  the reported worst-case complexity stays within a frame.
- **VM backstop values** — the `call_frames` ceiling + an optional step ceiling as
  defense-in-depth, sized to never fire inside the envelope.
- **Write-op vetting** — how a `*.write` op proves it is invariant-preserving (a review
  gate, a property test, or an explicit invariant the op upholds).
- **Diagnostic format** — the structured admission-error shape (construct span + rule +
  fix) that keeps the modder's fix-and-readmit loop tight (§6).

## See also
- [SANDBOX.md](../../SANDBOX.md) — invariants S1–S8 + the buildable-now slice.
- Symbol naming (`n_<name>` / `t_<LEN><Type>_<method>` — why method/type granularity is
  free): `CLAUDE.md § conventions`.
