<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 86 — sandbox-subset-flag: total-sublanguage sandboxing for user scripts

## Status

**v1 safety model COMPLETE — built, tested, and CLI-enforced end-to-end.** Plan **@PLN86**
([loft-lang/plans#86](https://github.com/loft-lang/plans/issues/86)) · `status:active`
· `subject:loft`. The concrete first slice of [SANDBOX.md](../../SANDBOX.md): a
compiler-enforced flag that runs **designated subsets** of a program (user scripts)
under a *prove-it-safe-at-load* policy, while the surrounding host code runs
unrestricted, in one process, sharing the store at full `DbRef` speed.  An admitted
sandboxed script reaches only allow-listed capabilities, terminates, never faults, runs
interpret-only, performs no raw writes to host data, and carries a worst-case complexity
budget — all four arcs (capability · termination · data-integrity · backend) checked at
load via `Parser::sandbox_admission_errors`, wired into the CLI (`loft.toml` `[sandbox]`
policy → reject-or-run).  Remaining work is post-v1 expressiveness + hardening — see
[§ Open work](#open-work).

**Implementation progress** (branch `tuxedo-work2`):
- **1.1 ✅** — `src/sandbox.rs`: `SandboxProfile`/`SandboxConfig` + the deny-by-default
  dotted-segment `allows()` cap match **and `allows_lib()` wholesale-library match** +
  the `loft.toml` `[sandbox]`/`[profile.*]` parser (`allow_libs` / `allow_caps`).
  Unit-tested.
- **1.2 ✅** — per-def designation in the parser: `set_sandbox_config`, a
  `def_sandbox` side-map (`def_nr → profile`, off `Definition` so it stays out of
  IR serialization), recorded in `parse_function`. Host-controlled only; e2e test.
- **0.1 ✅** — scoped parser nesting guard: the transient `in_sandbox` flag +
  `expression`→`expression_inner` depth-guarded wrapper; hostile deep nesting in a
  sandboxed def is a clean LOAD-time error (limit 128 ≈ 1.3 MB; ~10 KB/level), trusted
  code unguarded. Deterministic explicit-stack test.
- **1.3 ✅** — `sandbox::reachable_set` (restricted-mode DFS over `Value::walk`) +
  `Parser::sandbox_reachable_set`: descends into sandboxed defs, trusted symbols are
  leaves. **L4 hole found by probing the IR first** — a non-capturing fn-ref is a bare
  `Value::Int(def_nr)` (`apply(target,5)` → `n_apply(599i32,…)`), not a `FnRef` node, so
  it is read as a reference only in a `Function`-typed position (call arg / assignment).
  Both `apply(target,…)` and `f=target; apply(f,…)` covered; the
  returns/fields/collections forms are ALSO covered — recorded at the fn-ref CREATION
  site, so the flow afterward (a returned fn-ref, a vector element, a struct field)
  cannot escape it. **Confirmed by the prevention-#4 escape suite**
  (`admission_escape_suite_rejects_every_breakout`): `v = [secret]; v[0]()`,
  `get() -> fn(){ secret }` then call, and a `Holder { f: secret }` field call are each
  rejected. (The earlier "residual tracked" note was stale.)
- **1.4 ✅ (FFI + backend)** — **FFI:** `sandbox::reachable_ffi_bridges` flags every
  reachable def whose `#native` symbol is owned by an external native package
  (`native_symbol_crates`), distinct from built-in `#rust`/`#native` primitives — dlopen of
  a cdylib is RCE → rejected unless `native_ffi = true`. **Backend:**
  `Parser::sandbox_forces_interpret` (true on ANY designation) + main.rs wiring: a program's
  `[sandbox]` policy loads from `loft.toml` before parse, and a sandboxed program REFUSES
  `--native` / overrides the default native backend to interpret. Interpret-only is
  unconditional, not a profile setting (the old `interpret_only` field + `backend` key
  removed). `cargo build --no-default-features` builds (the cdylib-loading `native-extensions`
  path is removable). Verified end-to-end on the CLI.
- **2.1 ✅** — `#cap "<group>"` annotation: parsed in `parse_rust` beside `#native`
  (any file); `Definition.cap` + `Parser::def_cap_group` read it. `cap` is NOT yet
  IR-round-tripped (re-derived on parse) — persistence lands coupled with the first
  stdlib `#cap` annotation, documented on the field so the gap stays loud. Read/write
  groups read back distinctly; unannotated = None.
- **2.3 ✅ (the convergence) + library-first** — `sandbox::admit_capabilities` +
  `Parser::sandbox_admit`: each sandboxed def is admitted under ITS OWN profile; every
  trusted symbol it references admits if **its library is allow-listed wholesale**
  (`allow_libs` / `def_library`) **or** its `#cap` group is (1.1 `allows`) — or, for an
  external FFI bridge (1.4), `native_ffi` permits it. So a whole vetted library is
  included with NO tags (§3); a ref to another sandboxed def is skipped. Deny-by-default.
  **L4-complete**: indirect fn-refs resolve to their target (1.3). `CapViolation`
  (UngrantedCap{group}/UntaggedSymbol/ExternalFfi{crate}) names each offender for 2.5.
  Verified: wholesale-lib admits untagged fns; granted/ungranted caps gate; L4-indirect
  rejected.
- **2.2 ✅ (lint + persistence; blanket tagging dropped)** — `untagged_public_symbols`
  lists public fns lacking a `#cap` group — now a tool for a HOST tagging its OWN APIs, not
  a stdlib full-coverage gate (library-first made that unnecessary).
  **`cap` IR persistence shipped (2.2-persist):** `Definition.cap` round-trips through the
  store codec, so a `#cap`-tagged stdlib loaded from the `LOFT_STDLIB_CACHE` bundle still
  gates correctly. The DB packs fields by region, so `cap` landed at offset 148 (read off
  the baked-layout probe), pushing the trailing bools (rnn 148→152, pub_visible 149→153,
  stride 150→154); `baked_layout_mirrors_loft_schema` is the offset guard, and
  `cap_annotation_survives_store_round_trip` proves a non-empty group survives.
- **2.2 tagging — fs/env ✅ (the one stdlib split)** — `default/02_files.loft` carries 32
  `#cap` groups: `fs.read` (14), `fs.write` (16), `env` (2). Under **library-first** this
  is the *only* stdlib tagging needed — the built-in `files` read/write split. The pure
  modules (`code`/`text`/`json`) are included wholesale via `allow_libs`, untagged.
  Verified on the REAL stdlib: `mtime` (fs.read) rejected naming the group when only `env`
  is granted; `env_variable` (env) admits; untagged `now()` admits under `allow_libs`.
- **2.5 ✅ (P2 diagnostics)** — `describe_violation` / `Parser::sandbox_admission_errors`
  render each capability rejection as an actionable error: call-site position
  (`reference_position`), the symbol + rule, the profile's allowed set, and BOTH
  library-first fixes (`allow_libs` wholesale / the `allow_caps` cap). The contract a
  modder iterates against.
- **P3 ◐ (totality: loops + recursion + total-ops)** — `admit_totality` →
  `TotalityViolation`: **3.1** rejects an unbounded `while` (parse recorded), admits bounded
  `for`; **3.2** rejects recursion via a colour-DFS cycle check on the sandboxed call graph
  (self + mutual), admits acyclic; **3.3** excludes the explicit-abort ops (`assert` /
  `panic` / `log_fatal`) while arithmetic stays total on the interpreter (div-by-zero →
  null). All render actionable errors through `sandbox_admission_errors`. The capability arc
  (P0–P2 + 2.5) and the termination arc (3.1/3.2/3.3) are both proven.
- **1.4 backend half ✅** — sandboxed code is force-interpreted (`--native` refused), the
  `[sandbox]` policy loads from `loft.toml`, and `--no-default-features` drops the cdylib
  path. This was the prerequisite 3.3's total-op guarantee depends on (native traps).
- **CLI admission-reject ✅** — after the backend force, the CLI runs the full admission
  walk and rejects a violating sandboxed program at LOAD with the actionable errors
  (`tests/sandbox_cli.rs` drives it end-to-end: violation rejected, clean admitted,
  `--native` refused). **The sandbox is now enforced end-to-end through the binary.**
- **3.4 + 3.5 ✅** — `sandbox_complexity_degree`/`_report` give the host the `O(n^d)` budget
  hint (L5); the totality rejections already render actionable, bound-it diagnostics.
- **2.4 ✅** — no-raw-write admission: a sandboxed def may not raw-write heap data
  (`e.health = 0` / `v[i] = 9`); recorded at parse, rejected via `sandbox_admission_errors`.
  The data-integrity arc.
- **All FOUR arcs are DONE and CLI-enforced:** capability (P0–P2 + 2.5), termination
  (3.1–3.5), data-integrity (2.4), backend ban (1.4). An admitted script reaches only
  allow-listed capabilities, terminates, never faults, performs no raw writes, runs
  interpret-only, and carries a complexity budget. **Post-v1 items: [§ Open work](#open-work).**

**Scope.** v1 is intentionally **fairly restricted** — a small total dialect (bounded
loops, no/structural recursion, total ops) plus a **curated host API**; the
expressiveness comes from the *API* (the safe operations the host hands modders), not
from language permissiveness — restricted-language + rich-API still expresses most game
situations. Admitting *more* complex scripts via deeper termination/safety analysis
("involved code inspection") is **a separate future plan**, out of scope here.

## Open work

What is built is listed under [§ Implementation progress](#status); what remains:

### Prioritization — prevention over the catch-net (root cause, not mop-up)

The model's whole premise is **prove-it-safe-at-load**: an admitted script *cannot*
fault.  So a runtime fault is, by definition, a HOLE — in the admission walk or in
the interpreter — and the answer is to **close the hole at load time, not catch the
fault at runtime**.  Catching a panic or killing a hung loop keeps the *host* alive
but leaves the *mod* exactly as broken — same input, same fault, every run; for any
KNOWN fault class the catch-net is a band-aid over a fixable bug, and because the
root cause stays it breaks again immediately.

So each fault class shifts LEFT to a load-time / interpreter-level guarantee.  This
is the load-bearing open work, ranked **above** the catch-net:

1. **Memory-safe interpreter** — the root cause of "interpreter bug".  A UAF /
   double-free caught as a panic is still a UAF; the fix is that it cannot happen.
   This is the @PLN85 store-lifetime / ownership (`deps`) work
   ([STABILITY_REDFLAGS.md](../../STABILITY_REDFLAGS.md) — the hard dependency
   SANDBOX.md names: admission narrows the *language*, this removes the *escape
   hatch* a memory-safety bug opens).  **Highest priority** — it is the whole
   difference between "proven safe" and "proven safe *assuming a correct engine*".
2. **Space budget at admission — ✅ DONE.** The root cause of OOM.  Admission now
   computes a worst-case SPACE degree (`sandbox_space_degree`) the same way as the
   TIME degree (`O(n^d)`, 3.4), so the host bounds `n` for memory the way it already
   does for time — turning OOM from a runtime abort `catch_unwind` cannot even see
   into a load-time concern.  **Model** (`src/sandbox.rs`): the peak-heap degree is
   the deepest loop nesting at which a structure GROWS (`OpAppend*` / `OpPreAlloc*`,
   what `v += x` lowers to) on a var NOT reset in that loop — a transient buffer
   (reset each iteration, e.g. `b = []; b += x`) is O(1), a pure-compute loop is
   O(1), a vector built across nested loops is O(n²).  Reset is detected via
   `Value::reads_var` so a self-referential `Set(v, …v…)` (growth) is not mistaken
   for a reset.  Composes inter-procedurally, but ONLY for a callee that itself
   accumulates (a non-allocating call adds no space — that is where space diverges
   from the time rule).  `complexity_report` now names both axes (`time … / space …`).
   *Known v1 under-models* (future tightening): an explicit concat-reassign
   `v = v + [x]` (lowers via `OpAddVector` on a temp) and a kept growing return.
3. **Total host capabilities — ✅ DONE (loft-bodied surface).** The root cause of
   host-fn faults.  An allow-listed capability is trusted but, unlike sandboxed code
   (3.3), un-analysed; if it aborts on a script-supplied value the fault is past
   admission.  `capability_totality_violations` (`src/sandbox.rs`) is the host-side
   MIRROR of 3.3: over every `#cap`-tagged function it flags those whose call tree
   (transitively, following every callee) reaches an `ABORT_OPS` op (`assert` /
   `panic` / `log_fatal`), with an actionable "make it total — validate + return a
   clean error" message (`describe_cap_totality_violation`).  API:
   `Parser::sandbox_capability_totality_violations`.  **Limitation:** a NATIVE
   capability has no loft body, so its Rust is OPAQUE to this lint — the host vouches
   for native totality separately; this catches the loft-bodied (library) capability
   surface, which grows as libraries expose `#cap` functions.
4. **Close + FUZZ the admission walk — ✅ DONE.** The root cause of admission gaps.
   The adversarial escape suite `admission_escape_suite_rejects_every_breakout`
   (`src/parser/mod.rs`) TRIES to break out across every dimension — capability
   (direct / indirect fn-ref / via a sandboxed helper / a fn-ref in a collection,
   return, or struct field), totality (unbounded `while`, self- + mutual recursion,
   `assert` / `panic` abort ops, non-constant + conditional `while` steps), raw-write
   (field / index / nested field) — and asserts admission rejects each (16 breakouts).
   Positive controls (bounded `for`/`while`, struct construction, local writes, a
   granted capability) prove it is not vacuously rejecting.  Each probe is guarded to
   have PARSED, so a malformed probe can't pass silently.  **Residual closed by proof:**
   the documented L4 returns/fields/collections residual is NOT a hole — fn-refs are
   recorded at the CREATION site, so all three forms are rejected (the README note was
   stale; now corrected).  "No unknown holes" is unprovable, so this is the standing
   adversarial battery confidence rests on; the next deepening is to add forms as the
   library/capability surface grows.

**The catch-net (S7/S8) is demoted to a thin, ALARMED backstop** for the one thing
prevention cannot reach — the unknown-unknown, an interpreter bug nobody has found
yet.  A game engine must survive one bad mod rather than hard-crash, so
`run_script() -> Result` (`catch_unwind` → value) + a journalled store (effects roll
back) earn their keep there.  But its honest role is **host survival + a bug
report**, never "handled": every catch is a prevention failure that must fire an
alarm and become a root-cause fix.  If catches are *routine*, the design has already
failed.  (Hard aborts — OOM, stack-`SIGSEGV` — bypass `catch_unwind` and fall to the
process watchdog `--timeout` / @PLN49; one more reason prevention, not the net, is
the line.)

### A. Safety — completes the v1 admitted-script guarantee
- **2.4 No-raw-write admission — ✅ DONE.** (Was the one open v1 safety step.) A sandboxed
  def may not raw-write heap data (`e.health = 0` / `v[i] = 9`) — recorded at parse
  (`sandbox_raw_writes`, where field-write vs struct-construction is unambiguous) and
  rejected via `sandbox_admission_errors`; mutation only through an allow-listed `*.write`
  op. The v1 safety model is now complete.
- **Runtime fault-isolation (S7/S8 complement) — DEMOTED to the alarmed backstop.**
  `run_script() -> Result` (`catch_unwind` → value) + a journalled store roll-back,
  the runtime side of [SANDBOX.md](../../SANDBOX.md) S7/S8.  **NOT a primary item** —
  per the [§ Prioritization](#prioritization--prevention-over-the-catch-net-root-cause-not-mop-up)
  above, the load-bearing work is *preventing* the fault (memory-safe interpreter,
  space budget, total host capabilities, admission fuzzing), since a caught fault
  recurs on the next run.  This catch-net is only for the unknown-unknown, and every
  catch is a bug report that must become a root-cause fix.

### B. Expressiveness relaxations (post-v1 — admit MORE total programs)
- **3.1b decreasing-variant `while` — ✅ DONE.** `while_is_bounded` admits a `while` with a
  proven decreasing variant: an int counter vs a STABLE bound (int literal or a body-unchanged
  var), stepped by a positive constant every iteration (top-level `c = c ± k`), modified no
  other way — counting up/down or guarded by `… && i < N`. Sound + conservative: flag loops,
  conditional/cancelling/non-constant steps, moving bounds, and non-int/field/**call-bounded**
  variants are rejected (hoist a call bound like `len(v)` to a local first). Tests:
  `bounded_while_is_admitted` / `unprovable_while_is_rejected`.
- **3.2b structural recursion** — admit recursion when a structurally-decreasing argument
  is proven (today the call graph must be acyclic).
- **2.1b type/module `#cap` default + per-method override** — a `#cap` on a type/module
  sets the default for its methods; a method without its own `#cap` inherits it. Today
  `#cap` is per-def only.

### C. Precision / completeness refinements
- **1.3b L4 residual — ✅ DONE.** Every fn-ref is now recorded at its CREATION site
  (`sandbox_fn_refs`, `record_sandbox_fn_ref` at the two `Value::Int(fn_d_nr)` points in
  `objects.rs`), so a fn-ref flowing through a RETURN value, struct field, or collection
  element is caught completely + precisely — `add_recorded_fn_refs` unions it into every
  admission walk (capability, FFI-reach, recursion). Verified: a fn-ref to `mtime` hidden in
  `[mtime]` is rejected naming `fs.read`. The capability arc is now soundly closed.
- **3.3b abort-op robustness** — `ABORT_OPS` (`n_assert`/`n_panic`/`n_log_fatal`) is a
  by-name list; a `#[fault]`-style attribute on the def would be rename-proof, and the set
  should be audited for other process-terminating ops.
- **3.4b complexity precision** — trusted-leaf ops count as O(1)/call (a `sort` is
  O(n log n), `contains` O(n)); and the loop walk treats an `Iter`'s once-run init as
  in-loop (a safe over-count). Both tighten the `O(n^d)` report.
- **2.4b index/vector write ownership** — ownership-aware 2.4 admits a script-owned STRUCT
  mutation but conservatively rejects index writes (`v[i] = …`) and non-struct bases, since a
  vector carries no per-value owner. Needs value provenance (is the vector a local
  construction vs a host-passed param) to admit a mod mutating its own array.

### D. Hardening / integration
- **1.4b feature-gate `--native` codegen** — the dlopen path is already removable
  (`--no-default-features` drops `native-extensions`); the rustc *codegen* path
  (`src/generation/`) is not yet behind a feature, so a deployment can't build with ZERO
  host-codegen RCE surface. Gate it.
- **CLI warm-cache gap — ✅ FIXED.** The program warm-cache (default-on) restored the IR
  without re-parsing, so `def_sandbox` never formed and admission + force-interpret were
  bypassed on warm runs (and, since the cache keys on program content not policy, a tightened
  policy was ignored). Fix: the `[sandbox]` policy loads BEFORE the warm-load decision and
  warm-load is disabled when a policy is active (`Parser::sandbox_is_active`) — a sandboxed
  program always parses fresh. Regression: `warm_program_cache_does_not_bypass_admission`
  (proven to fail without the gate).
- **`loft.toml` discovery** — the CLI loads the `[sandbox]` policy from `loft.toml` *next to
  the program file* only; add parent-dir / `--project`-root discovery.
- **Surface the complexity report** — `sandbox_complexity_report()` is computed but not
  printed by the CLI; show it (informational) on a sandboxed run.

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

### 3. Library-first admission — whole libraries in, tags only to split one
A reachable trusted symbol admits if **either** its **library** is allow-listed
wholesale (`allow_libs`) **or** its `#cap` **group** is allow-listed (`allow_caps`).
A complete, host-vetted library is included as a **unit** — every symbol in it
admits with **no tag**, including its untagged functions and its native bridges. So
the host allows the pure stdlib modules (`code`/`text`/`json`) wholesale and tags
nothing; the 166-fn stdlib never needs blanket tagging.

**Tags are purely the fine-grained layer** — for carving a library in half
("include `files`, but reads only"). The capability is a fact that lives with the
function (`#cap "<group>"` at its definition, beside `#native`/`#rust` — one home per
fact, [STABILITY_REDFLAGS.md](../../STABILITY_REDFLAGS.md)). Only a *partially*-
included library needs them.

- **The stdlib ships its own tags**, only where a built-in split is worth exposing:
  `files`'s `fs.read`/`fs.write` is the one example today. Projects **cannot edit the
  stdlib** — they include its modules wholesale, or use the shipped split.
- **Projects tag their OWN code** — internal APIs + bundled libraries — to gate them
  finely; that is where new `#cap` tags come from, not the stdlib.
- Keeping a few real tags (the `files` split) keeps the **cap path verifiable**
  end-to-end alongside the wholesale path.

```
// the stdlib SHIPS this split (a project cannot add it):
pub fn mtime(path: text) -> integer;  #cap "fs.read"
pub fn write(self: File, v: text);    #cap "fs.write"
```
```toml
[profile.mod-script]
native_ffi = false                            # no vetted cdylib bridge (interpret-only
                                              # is unconditional — not a key)
allow_libs = ["code", "text", "json"]        # whole modules — no tags needed
allow_caps = ["fs.read"]                      # files NOT wholesale → reads only, no writes
```
A symbol in **no** allowed library **and** with **no** allowed cap is denied
(deny-by-default). The library is the source module (stdlib) or package name.

### 4. Totality admission (the core)

**The admission walk** is a DFS over the sandbox-reachable set, and it needs the
**compile-time analog of the restricted/unrestricted stack we dropped from the runtime**
— the same model reappears here, as *analysis* state rather than a resource budget:
- **mode — restricted vs unrestricted.** The totality / capability / no-raw-write rules
  apply only while the walk is inside a **sandboxed** def's body. A call to a **trusted**
  allow-listed symbol is a **leaf**: the walk does *not* descend or re-analyze it — its
  safety is the host contract (L-host). The compiler must therefore know, at every point,
  which mode it is in (the 1.2 flag), and switch to "leaf" at the sandboxed→trusted edge.
- **ancestry — the recursion stack.** The walk carries the chain of sandboxed defs on the
  current path (its *parents*). A call to an **ancestor** is a back-edge = recursion; v1
  **rejects** it (acyclic only), so termination is structural by construction. (Admitting
  a back-edge when a structurally-decreasing argument is proven is the later relaxation;
  general termination analysis is the separate plan.)

A sandboxed def is admitted only if, under that walk, the compiler can prove it
**total** — terminates on every input — from a restricted but useful form:
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
- **L4 — capabilities are complete (indirect calls can't escape).** *Status:*
  **partially verified (1.3).** *Risk:* a fn-ref to a denied symbol called indirectly.
  *IR finding (verified):* a non-capturing fn-ref is emitted as a bare
  `Value::Int(def_nr)` — `apply(target, 5)` → `n_apply(599i32, 5i32)` — **not** a
  `FnRef` node, so it is indistinguishable from an integer literal **except by type
  context**; `reachable_set` reads an `Int`/`Long` as a reference only in a
  `Function`-typed position (call arg / assignment). *Mitigation:* admission closes over
  **references** (the fn-ref `target` → caught) **and** allow-listed host APIs must not
  hand untrusted code an arbitrary fn-ref (host contract). **Probe (passing):**
  `apply(target,…)` and `f = target; apply(f,…)` both put `target` in the set. *Residual
  (tracked):* fn-refs via `Function`-typed return / struct field / collection — close
  before 2.3 relies on the set.
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

### P0 — parser nesting bound (load-time)
- **0.1 Scoped parser depth guard.** *Change:* a nesting-depth counter in the
  operator-precedence parser (counted at precedence-0 entries = source nesting),
  active **only while parsing a sandboxed def's body** (gated on the 1.2 flag); a parse
  *error* past a configurable limit. *Verify:* 5000-deep nested input in a sandboxed def
  → clean parse error (not `rc=139`); the same in trusted source is unaffected; a legal
  sandboxed program below the limit compiles. *(Load-time rejection, not a runtime
  abort.)*

### P1 — designation + reachable set + coarse bans
- **1.1 Policy model + parse.** *Change:* parse `[sandbox]` + `[profile.*]` into a
  `Policy` (allowed groups, backend, native_ffi, input-bound hints). *Verify:* round-trip
  unit test; a malformed policy → a clear error.
- **1.2 Per-def profile tag + the "in sandboxed code" flag.** *Change:* from the manifest
  globs, set `def.profile: Option<ProfileId>`, and a parse-time + runtime
  "currently-untrusted" flag derived from it (annotation honoured only on trusted
  sources). *Verify:* `loft introspect` shows exactly the designated defs tagged; the
  flag is set while parsing/executing a sandboxed def, clear otherwise. *(Unblocks 0.1.)*
- **1.3 Reachable-set over references (restricted-mode DFS).** *Change:* DFS the closure
  over calls + fn-ref literals + type uses from sandboxed entries, **in restricted mode**;
  a trusted allow-listed symbol is a **leaf** (not descended — §4). *Verify (L4):* an
  indirect `let f = g` puts `g` in the set; a trusted call is a leaf; the set equals the
  hand-computed set.
- **1.4 Backend + FFI ban.** ◐ *FFI half shipped* (`reachable_ffi_bridges`). *Change:*
  sandboxed defs forced to interpret; a sandboxed `use` of a `[native]` lib rejected.
  *Verify:* sandboxed `--native` / native-lib use rejected; `cargo build
  --no-default-features` proves the cdylib path is removable. **Done:** external-FFI
  bridges reachable from sandboxed code are detected via `native_symbol_crates`.
  **Remaining:** the backend force-interpret in the execution pipeline + the
  no-default-features cdylib removal.

### P2 — capabilities (groups + no-raw-write) + diagnostics
- **2.1 `#cap "<group>"` annotation.** ✅ *per-def parse + read shipped.* *Change:* parse
  `#cap` onto the def (like `#native`); type/module group with per-method override.
  *Verify:* a def's group is readable; `Vector.get`=`collections.read`,
  `Vector.clear`=`collections.write`. **Done:** per-def `#cap` parse + `def_cap_group`
  (read/write groups read back distinctly). **Remaining:** the type/module default with
  per-method override (resolve a method's effective cap from its type when it has none),
  and IR persistence (coupled with the stdlib annotation in 2.2).
- **2.2 Coverage lint + the one stdlib split.** ✅ *(re-scoped by library-first).* Blanket
  stdlib tagging is **dropped**: pure modules are included wholesale (`allow_libs`), so
  `code`/`text`/`json`/… stay untagged. Shipped: `untagged_public_symbols` (the lint — now
  a tool for a *host* tagging its OWN APIs, not a stdlib gate), `cap` IR persistence
  (baked-layout regen), and the built-in `files` `fs.read`/`fs.write` split. *L3-cap* is no
  longer "every public symbol must be tagged" — it is "every symbol a sandbox reaches is in
  an allowed library or carries an allowed cap," which the admission walk enforces.
- **2.3 Library-first admission.** ✅ *shipped* (`admit_capabilities` / `sandbox_admit`).
  *Change:* every reachable symbol is admitted if its **library ∈ `allow_libs`** OR its
  **group ∈ `allow_caps`** OR it is another sandboxed def, else reject — so whole vetted
  libraries need no tags (§3) and tags only carve a library in half. *Verify:* a
  wholesale-allowed library admits its untagged fns; `mtime` (`fs.read`) rejected naming
  the group when ungranted; the L4 indirect `f=read_file; f(...)` rejected. **Done on the
  REAL stdlib**: untagged `now()` admits under `allow_libs=["files"]`; the fs.read/fs.write
  split gates `files` finely. Library = source module / package (`def_library`).
- **2.4 No-raw-write admission — OWNERSHIP-AWARE.** ✅ *shipped* (`RawWriteViolation` /
  `raw_write_is_host_owned`). *Change:* in a sandboxed def, reject a raw field/index write to
  HOST data, but ALLOW the script to mutate the data it OWNS (the ../crawler dogfood:
  `e.alive = false` on a script-created entity). Host data = the base root is a parameter, or
  its type is a host-library struct (the TYPE catches aliasing `x = player; x.health = …`), a
  vector, or a scalar; a non-parameter local of a script-defined struct (its `def_library ∉
  allow_libs`) is the script's own → mutable. *Verify:* `m = Mob{}; m.hp = 0` admits;
  `fn f(p: Player){ p.hp = 0 }` rejected. **Limitation (post-v1):** index writes + non-struct
  bases stay conservatively rejected — vector-element ownership needs value provenance.
- **2.5 Diagnostic quality (§6).** ✅ *P2 classes shipped* (`describe_violation` /
  `Parser::sandbox_admission_errors`). *Change:* every rejection carries the construct
  span + the rule + the allowed set / fix. *Verify:* the error text for each rejection
  class names the symbol/group/op and points at the fix. **Done:** the three capability
  classes (UngrantedCap / UntaggedSymbol / ExternalFfi) each render with a call-site
  position (`reference_position` tracks the enclosing `Span`), the symbol + rule, the
  profile's allowed set, and BOTH library-first fixes. **Remaining:** the P3 totality /
  no-raw-write classes get their messages when those checks land.

### P3 — totality admission (the core)
- **3.1 Loop boundedness.** ✅ *shipped* (`admit_totality` → `UnboundedLoop`). *Change:*
  admit `for x in <finite collection>` / `for i in 0..N`; reject unbounded `while`.
  *Verify (L2):* `while …` rejected; `for i in 0..10 {…}` admitted. **Done:** `parse_while`
  records the loop when `in_sandbox` (the IR can't tell a `while` `Loop` from a bounded
  comprehension `Loop`, so the parser marks it where it knows). The decreasing-variant
  relaxation is later.
- **3.2 Recursion analysis.** ✅ *shipped* (`recursion_cycles` → `Recursion`). *Change:* a
  colour DFS over the sandboxed call graph (trusted callees not followed — total by
  contract); a back-edge to a node on the current path is a cycle → v1 rejects (acyclic
  only). *Verify (L2):* self-recursion `rec(n+1)` and mutual `a→b→a` rejected naming the
  cycle; an acyclic chain admitted. The structurally-decreasing relaxation is later.
- **3.3 Total-operation check.** ✅ *shipped* (`admit_totality` → `PartialOp`). *Finding
  (both backends probed):* the **interpreter already makes the arithmetic ops total** —
  div/mod-by-zero → `null`, OOB → `null`, integer overflow wraps — so a sandboxed
  expression never faults on them; **native traps** ("divide by zero"), so the guarantee
  rests on interpret-only (the 1.4 backend ban). No rejection is needed for arithmetic.
  *Excluded:* the explicit-abort ops `assert` / `panic` / `log_fatal` (`ABORT_OPS`) — they
  fault the script and cannot be made total → `PartialOp` rejection naming the op + a
  defensive-check fix. *Verify (L3):* `a / b ?? 0` admits as total; a sandboxed `assert`
  is rejected. ✓
- **3.4 Worst-case complexity report.** ✅ *shipped* (`sandbox_complexity_degree` /
  `_report`). *Change:* derive the step cost as a function of input size. *Verify (L5):* a
  per-entity loop reports `O(n)`. **Done:** the degree = max over the body of `loop_nesting
  + degree(callee)`, composed across the acyclic call graph (loop-calling-a-looping-fn →
  O(n²)); rendered `O(1)`/`O(n)`/`O(n^d)` for the host to bound inputs. Reported, not
  rejected.
- **3.5 Totality diagnostics (§6).** ✅ *shipped with 3.1–3.3* — each totality rejection
  (`describe_totality_violation`) names the construct + how to bound it (a bounded `for`, a
  defensive check), folded into `sandbox_admission_errors`.
  *Verify:* the unbounded-loop / unbounded-recursion errors include the actionable fix.

### P4 — execution (no guard, no abort path)
- **4.1 `run_script(src, policy, world) -> Result`.** *Change:* a `src/lib.rs` entry that
  compiles + admits + runs and returns the script's value; the only failure surface is a
  *load-time* admission error. *Verify:* an admitted script runs to completion and writes
  live state directly; **no code path aborts a running admitted script** (grep + a probe
  that an admitted long-but-total script always finishes).
- **4.2 Runtime total-op semantics.** *Change:* the interpreter honours the 3.3 total
  semantics at runtime (OOB→`null`, div-zero→sentinel, …). *Verify:* runtime results
  match admission's promise on both backends for the trusted-call path.

### P5 — performance gate
- **5.1 Benchmark.** *Verify (S6):* a sandboxed script reading/writing N entities/frame
  stays within a frame budget vs a native baseline — with **direct writes, no rollback,
  no copy, no journal** (if this needs a rollback to be safe, the design is wrong, §5).

**Dependency order:** 1.1→1.2 (1.2 unblocks 0.1) → 1.3/1.4 → 2.x → 3.x → 4.x → 5.1.
**P0–P3 are the compile-time core** (reject at load, game-safe); P4 has no abort path to
make fail-safe; P5 proves *fast + safe*. **Admission diagnostics (2.5, 3.5) are
first-class** — a clean compile is the safety contract. A rung graduates its probe to
`tests/scripts/` / `tests/sandbox.rs` when green on both backends where applicable.

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
