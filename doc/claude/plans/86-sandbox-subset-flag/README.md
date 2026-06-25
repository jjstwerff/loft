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

### Now: handed to the crawler agent for dogfood

Admission (v1 #422) + the prevention hardening (#2 space budget, #3 total host capabilities,
#4 escape suite — all DONE) are shipped, so the feature is in the **two-agent dogfood** phase
([CLAUDE.md § The consumer runs in its OWN agent](../../../../CLAUDE.md)): **crawler** is the
dedicated consumer agent. It switches on a `[sandbox]` policy over its content/script surface,
feels out whether the restrictions are **too tight** to express its mods, and **adversarially
tries to break out** via its own codebase. This stream does NOT do that consuming — it ships +
documents the feature and *responds* to what crawler reports (a blocking restriction or an escape
routes back here as language work). The remaining post-v1 pieces (transactional world S7,
`run_script` boundary S8) wait on the memory-safe-interpreter dependency (#1, the `../loft`
store-lifetime stream), so the next sandbox work is **driven by the crawler dogfood findings**,
not pushed from here.

### A. Safety — completes the v1 admitted-script guarantee
- **Per-member access — read / update / append (P6 6.4–6.7, §7) — ✅ LANDED (F3–F6).** The
  all-or-nothing no-raw-write arc is now independent **read / update / append** rights per
  struct field, linked via `group#right` to declared `capability` groups; so append-only
  (grow a log, never alter it) is expressible and a `#read`-marked field is private. Pure
  compile-time (parse-site recording + `field_{read,update,append}_violations`), generalising
  the coarse 2.4 below. Remaining model surface: **6.9** parameter `#default` locks + **6.8**
  group-existence validation + IR persistence. *(Construction stays unrestricted — position 1
  — so there is no enum-variant gate.)*
- **Data envelope (P7, §8) — 🟡 DESIGNED.** Turns the §4 complexity *degree* into a hard
  load-time limit: prove `coeff · max_input_n^degree ≤ data_budget` or reject. Closes OOM —
  the one fault `catch_unwind` cannot see — at admission. Pure compile-time, no @PLN85 dep.
- **`sandbox-check` verdict + RED/GREEN access corpus (P8) — 🟡 DESIGNED.** A no-run
  "will this be allowed?" surface (CLI + lib) and the committed compile-only test battery.
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

### 3. Library-first admission — whole libraries in, capabilities only to split one
A reachable trusted symbol admits if **either** its **library** is allow-listed
wholesale (`allow_libs`) **or** the profile grants its `group#right` capability link
(`allow`). A complete, host-vetted library is included as a **unit** — every symbol in it
admits with **no link**, including its unlinked functions and its native bridges. So the
host allows the pure stdlib modules (`code`/`text`/`json`) wholesale and links nothing; the
166-fn stdlib never needs blanket tagging.

**Capabilities are purely the fine-grained layer** — for carving a library in half
("include `files`, but reads only"). The capability is the **same mechanism functions and
data members share** (§7): a declared, namespaced `capability` referenced by a validated
`group#right` link at the definition, beside `#native`/`#rust` — one home per fact,
[STABILITY_REDFLAGS.md](../../STABILITY_REDFLAGS.md). Only a *partially*-included library
needs them. *(v1 shipped the function link as an unchecked `#cap "fs.read"` string; P6.2
migrates it to the declared-`capability` + `group#right` form below — one model, validated.)*

- **The stdlib ships its own capabilities**, only where a built-in split is worth exposing:
  `files`'s `fs#read`/`fs#update` is the one example today. Projects **cannot edit the
  stdlib** — they include its modules wholesale, or use the shipped split.
- **Projects declare + link their OWN code** — internal APIs + bundled libraries — to gate
  them finely; that is where new `capability` declarations come from, not the stdlib.
- Keeping a few real links (the `files` split) keeps the **cap path verifiable** end-to-end
  alongside the wholesale path.

```loft
// the stdlib DECLARES + SHIPS this split (a project cannot add it):
capability fs
pub fn mtime(path: text) -> integer fs#read;     // call gate in the signature (§7.1)
pub fn write(self: File, v: text)   fs#update;   // void → the link goes after the params
```
```toml
[profile.mod-script]
native_ffi = false                            # no vetted cdylib bridge (interpret-only
                                              # is unconditional — not a key)
allow_libs = ["code", "text", "json"]        # whole modules — no links needed
allow      = ["fs#read"]                       # files NOT wholesale → reads only, no writes
```
A symbol in **no** allowed library **and** with **no** granted `group#right` is denied
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
- sandboxed code performs **no _unauthorised_ mutation** — it may directly write only a
  **member the host granted an `update`/`append` right** (§7), each of which the host has
  vouched is invariant-safe for ANY value (the **L-member** contract), and otherwise mutates
  coupled state only through **invariant-preserving, capability-gated host operations**. It
  can never raw-write a member it was not granted (deny-by-default), so no write can corrupt.
- a profile grants *which* members/ops via the `allow` `group#right` links; each granted right
  (and each write op) is vetted safe (the L-host / L-member contract). **Any sequence of them
  keeps the world valid**, so order/interruption can't matter — and admitted scripts never
  interrupt (they're total).
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

### 7. Capabilities — what a restricted caller may do

A capability is a permission the **host/library** requires of a **restricted caller** (a
sandboxed modder).  The host annotates *its own* surface — functions, their parameters, its
struct fields — with `group#right` links; a modder's profile is granted a set of
capabilities; admission checks every point where the modder's code touches the host
surface.  **The modder's own functions and data carry no links and are never restricted** —
only what they reach *into the host* is gated.  This generalizes the all-or-nothing §5
no-raw-write rule into a fine-grained, caller-facing permission system.

**Declaring a capability** — a namespaced top-level symbol; an undeclared group in a link
is a load error (validated at admission, so forward + cross-file references resolve):

```loft
capability fs
capability world
capability bag
```

`capability` is deliberately verbose: declarations are rare, and a design with *many* of
them is a smell, so the cost falls in the right place.

**The three rights** a link may carry:
- **read** — observe a value.
- **update** — change an existing value in place.
- **append** — add to a **structure** (a collection).  Append exists *only* on a collection
  — there is no append for a scalar.

#### 7.1 Calling a function — the call gate (in the signature)
The host puts the call gate at the end of the signature (after the return type, or after the
parameters when there is no return).  The right is the effect the call has.  `#cap` is gone
— the link is a first-class part of the contract, beside the parameters and the return type,
*not* lumped with the `#native` / `#impure` / `#wasm` implementation plumbing.

```loft
fn mtime(path: text) -> int  fs#read;     // calling this reads the filesystem
fn remove(path: text)        fs#update;   // void → the link goes after the params
```

A modder granted `fs#read` may call `mtime`; not granted → the call is rejected at load.
**Passing arguments is part of the call** — if you may call a function, you may pass any
argument you like, with no extra grant.

#### 7.2 Locking a parameter to its default
For when the host lets a modder *call* a function but not steer a particular argument.  Tag
only the parameter you want to pin; it is then forced to its default unless the modder holds
the lock.

```loft
fn spawn(kind:  text,
         count: int = 1  spawn.count#default) -> Entity   world#append;
```

- Granted `world#append`: may call `spawn`, may pass any `kind` (untagged → free), but
  `count` is forced to `1`.
- Also granted `spawn.count#default`: may write `spawn(count: 5)`.

Untagged parameters are always free — **set is inherited from the call** — so the host tags
only what it locks; there is no per-parameter upkeep.  `#default` is the opt-out limitation,
allow-by-default, the inverse polarity of the deny-by-default field rights below.

#### 7.3 Reading and writing a field
A field of a host struct carries its own access links.  A scalar field has read + update; a
**collection** field also has append.

```loft
struct Entity {
    id:     int                                // untagged → readable, never writable
    health: int    health#read health#update   // scalar: read + update
    bag:    [Item]  bag#read bag#append         // collection: read + APPEND only (no update)
}
```

- `e.health` — readable by anyone (read is free), updatable only with `health#update`.
- `e.bag += item` — needs `bag#append`; since `bag#update` is *not* granted, the modder may
  grow the bag but never overwrite an existing slot.  **Append-only is exactly this.**
- `e.id` — no write link → read-only.

`append` appears on `bag` because `bag` is a structure; it would be meaningless on a scalar
like `health`.

#### 7.4 Defaults, and the modder's side
| Operation | When the host adds no link |
|---|---|
| read a field | **free** |
| update / append a field | **denied** (the host must grant) |
| override a parameter | **free** (inherited from the call) |
| call a function | needs its call-gate link (or a wholesale-allowed library, §3) |

The modder writes unrestricted code; gating happens only at the host boundary:

```loft
// no links anywhere — a modder never tags their own code:
fn my_strategy(e: Entity) -> int {
    let h = e.health        // read a host field — free
    damage(e, 10)           // call a host fn — needs whatever damage() requires
    e.bag += Item.Potion    // append to a host field — needs bag#append
    spawn("goblin")         // call — needs world#append; count is pinned to 1
}
```
```toml
[profile.mod]
allow_libs = ["math", "text"]
allow      = ["world#append", "bag#append", "health#read"]
```

A grant resolves through **normal namespacing**: `allow = ["game#read"]` covers every
capability beneath `game` (e.g. `game.entity#read`) with the same right — the existing
`cap_prefix_match` on the group part, exact on the right.  The **§2.4 ownership split is
unchanged**: links gate **host** data only — a script's own structures (types it defines,
values it constructs locally) stay freely read/update/append.

Every check is at **admission** — no runtime cost, no rollback: a disallowed call, parameter
override, or field access is a **load error**, never a caught fault.

**Deliberately *not* in this model** (the over-reach this section drops): no append on a
scalar; no implementer-side mutability/borrow notion (the links authorize the *caller*, not
the function body); and enum-variant **construction** gating is a separate question — it is
**not** folded into read/update/append.  Closures (a modder-authored callable handed across
the boundary, or captured host state) are the one genuinely subtle case and ride on the
existing L4 fn-ref handling — designed in their own pass, not here.

### 8. Data envelope — a compile-time footprint bound
§4 reports a worst-case complexity *degree* (`O(n^d)`) and asks the host to "bound n." The
envelope makes that a **hard, checked limit at load**: the host declares a `data_budget`
(words of peak live heap) plus the input bounds the degree is expressed in (`max_input_n`,
`max_depth`, `max_string_len`), and admission proves the footprint fits — or **rejects**.

Because the totality arc (§4) already bounds every loop and makes recursion acyclic, peak
heap is a closed form: `Σ over accumulating allocation sites (record_size × max_input_n^nesting)`.
Every factor is known — the trip count from `max_input_n`, the record size **exactly** from
the type's stride. The space analysis already computes the *degree*; the envelope adds the
*coefficient* (`Σ record_size`) and compares `coeff · max_input_n^degree` against
`data_budget`. An allocation whose size cannot be tied to a declared bound (an uncapped
dynamic string, a host-value-sized allocation) is **rejected** — deny-by-default, the same
trade as an unbounded loop, with a diagnostic that names the fix.

This is **purely compile-time**: no allocation counter, no runtime ceiling, no saturation,
no rollback. OOM — the one fault that bypasses every other guarantee because `catch_unwind`
cannot see it — becomes a load-time concern.

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

### P6 — Unified capability model: `capability` decls + `group#right` for functions AND members [compile-time core, §3 + §7]

> **Consistency migration (pre-customer, deliberate).** v1 shipped the *function* capability
> as an unchecked STRING — `#cap "fs.read"` → `Definition.cap: String` (`src/data.rs:2451`),
> parsed at `src/parser/definitions.rs:1172`, matched via `allow_caps` /
> `cap_prefix_match` (`src/sandbox.rs:65`). This ladder lands ONE mechanism for functions and
> data members: a declared, namespaced `capability` referenced by a validated `group#right`
> token. The `.`-segment split (`fs.read` / `fs.write`) becomes the right split
> (`fs#read` / `fs#update`). Do NOT ship the member model beside the old string model.

- **6.1 The `capability` declaration + the `group#right` token + a resolver (foundation).** ✅ **DONE.**
  - `parse_capability` (`src/parser/definitions.rs`) parses a `capability <dotted.name>`
    top-level declaration (a contextual keyword, also added to `starts_top_level_def`) and
    records the dotted name in a parser-side `declared_capabilities: HashSet<String>` — the
    dotted name IS the namespace (like the `fs.read` groups), so this needs no new `Definition`
    kind. Cleared in the reset + `parse_str` paths.
  - `enum Right { Read, Update, Append }` + `Right::parse`/`as_str` (`src/sandbox.rs`).
  - `Parser::cap_is_declared(group)` resolver; an undeclared/mistyped group in a link is a
    LOAD error (validated at admission). *(IR persistence of the registry for a warm-cached
    stdlib is the later 6.8; sandboxed programs parse fresh.)*
  - *Verify:* `capability_declarations_register_and_resolve` — `fs` / `cmd.move` resolve,
    `typo` does not; `Right::parse` covers read/update/append.
- **6.2 The function call-gate link in the SIGNATURE; drop `#cap`.** ✅ **DONE.**
  - `try_cap_link` (`definitions.rs`) parses a `group#right` token where one is OPTIONAL
    (silent `None` when the next token is the `;`/`{` terminator; errors only on a malformed
    link). `parse_function` parses it **after the output** (return type, or the param list for
    a void fn) into `Definition.cap`; the **`#cap` annotation branch is removed** — the link is
    a first-class part of the contract, not plumbing. `cap()` + IR persistence (`DEF_CAP`)
    unchanged, now round-tripping the `group#right` token.
  - Retag `default/02_files.loft`: declare `capability fs` + `capability env`; **37 links moved
    into the signatures** — `fs.read`→`fs#read`, `fs.write`→`fs#update`, `env`→`env#read` —
    covering both native `;`-decls (`-> boolean fs#read;`) and loft-bodied functions
    (`-> text fs#read {`). (`fs.write`→`#update`: a write modifies existing content.)
  - *Verify:* `mtime` carries `fs#read` in its signature; `cap_annotation_is_parsed_and_readable`
    + the cap IR round-trip + the 8 admission tests green.
- **6.3 Profile grants: the unified `allow` list of `group#right` tokens.** `SandboxProfile`
  (`src/sandbox.rs:22`): fold `allow_caps` into `allow: Vec<String>` of `group#right` tokens
  (keep `allow_libs` wholesale). Extend `cap_prefix_match` (`:65`) to split each side on `#`:
  **namespace-prefix match on the group, EXACT on the right**. `parse_sandbox_config` (`:127`)
  reads `allow`. *Verify:* `allow=["fs#read"]` admits `mtime`, rejects `write` (`fs#update`);
  `game#read` covers `game.stats#read`; round-trip.
- **6.4 Member link parse + carrier.** ✅ **DONE.** `parse_field_links` (`definitions.rs`)
  parses the bare `group#right` link after a struct field's type in BOTH field-type branches
  (named/scalar + vector/generic). `try_cap_link` is **non-destructive** (saves the cursor +
  reverts on a miss), so a `not null` / default after a field type is never mis-consumed.
  Recorded in `member_access: HashMap<(struct def_nr, field name), Vec<group#right>>` off `Data`
  (first-pass; persist in 6.8). *Verify:* `field_capability_links_are_recorded`. *(Keyed by
  field NAME, not member index; enum-variant links are out of scope — see 6.7.)*
- **6.5 Read admission.** ✅ **DONE.** A sandboxed read of a `#read`-linked host field is
  recorded at the field-access site (`fields.rs::field`, second pass) into `sandbox_field_reads`;
  `field_read_violations` (`sandbox.rs`) rejects a read whose token the profile does not grant.
  Read is default-allow, so only a `#read`-linked field ever gates. *Verify:*
  `field_read_gates_private_field_admits_unlinked`.
- **6.6 Update admission (generalises 2.4).** ✅ **DONE.** A one-level field write `e.f = v`
  whose field has an `#update` link is diverted at the 2.4 site (`expressions.rs`) into
  `sandbox_field_updates` — the written field resolved via a `last_field_target` stash VERIFIED
  against the base var's struct; admitted iff granted, else `field_update_violations`. A field
  with NO update link stays the coarse 2.4 reject (read-only by default). *Verify:*
  `field_update_gates_writable_fields_unlinked_read_only`.
- **6.7 Append admission.** ✅ **DONE.** A `+=` to an `#append`-linked field is routed BY THE
  OPERATOR to `sandbox_field_appends`; admitted iff granted, else `field_append_violations`.
  `=` stays the `#update` path (6.6), so `bag#read bag#append` (no update) is append-only.
  *Verify:* `field_append_gates_collection_grow`. *(Construction is **unrestricted** — the
  position-1 decision — so there is NO enum-variant construct gate; the design dropped it.)*
- **6.8 Group-existence + IR persistence + diagnostic polish.** Wire `cap_is_declared` into
  admission so a link to an **undeclared** `capability` is a clean LOAD error (today an unknown
  group simply never matches a grant); round-trip `member_access` through the store codec the
  way `Definition.cap` already does, so a warm-cached host type keeps its field links; tighten
  each rejection's wording. *Verify:* `typo#read` → load error naming the unknown capability; a
  tagged member survives the store round-trip.
- **6.9 Parameter `#default` locks (§7.2).** Parse a `…#default` link on a function parameter;
  at a sandboxed call site, gate an argument that DIFFERS from the parameter's default on the
  lock token — an untagged parameter is free (set is inherited from the call). *Verify:*
  `spawn(count: 5)` rejected unless `spawn.count#default` is granted; bare `spawn()` admits.

### P7 — Data envelope (compile-time footprint bound) [compile-time core, §8]
- **7.1 Coefficient.** Extend `intrinsic_space` (`sandbox.rs:1088`) / `space_degree` (`:1145`)
  to also accumulate `Σ record_size` over accumulating sites — record size = the exact type
  stride (`LinkedFieldGroup::group_size`, `data.rs`). Return `(degree, coeff)`. *Verify:* a
  per-entity struct-build loop reports `(degree 1, coeff sizeof(struct))`.
- **7.2 Static-sizing gate.** In the space scan, flag an allocation whose record size is not
  statically bounded (uncapped dynamic string; a host-value-sized alloc not tied to
  `max_input_n`) → a new `DataViolation::UnboundedAlloc` rejection. *Verify:* an uncapped
  string build is rejected; a `max_string_len`-capped one admits.
- **7.3 Envelope fields.** `max_input_n` / `max_depth` / `max_string_len` / `data_budget` on
  `SandboxProfile` (`sandbox.rs:22`) + `parse_sandbox_config` (`:127`). *Verify:* round-trip.
- **7.4 Bound + reject.** In `sandbox_admission_errors`, compute `coeff · max_input_n^degree`
  (reuse `sandbox_complexity_degree`, `:1055`) and reject if `> data_budget` or unprovable,
  with the figure + fix; extend `complexity_report` (`:1200`) to print the absolute bound.
  *Verify:* an over-budget script is rejected naming the figure; an under-budget one admits.

### P8 — `sandbox-check` verdict + the access corpus [tooling]
- **8.1 No-run verdict.** A `loft sandbox-check <profile> <file>` subcommand (`src/main.rs`) +
  a `sandbox_check(src, profile) -> Verdict` entry (`src/lib.rs`) that run the admission walk
  (`Parser::sandbox_admission_errors`) ONLY — print Admitted / Rejected+diagnostics, never
  execute. *Verify:* a side-effecting body proves no run on Admitted; a violation prints the
  diagnostics. This is the "will this be allowed?" loop the modder + a mod-registry submit-gate
  iterate against.
- **8.2 RED/GREEN access corpus.** Extend `tests/sandbox.rs` (CLI-level: `tests/sandbox_cli.rs`)
  with the battery: RED — write a read-only field, append to an update-only structure, construct
  an un-forgeable variant, read a private field, call a `fs#update` fn under a `fs#read`-only
  grant — each Rejected; GREEN — read/update/append within grants, match any variant — Admitted.
  Each RED probe proven to fail WITHOUT its rule (not vacuously rejecting). *Verify:* green on
  the real type defs + the migrated `02_files.loft`, both backends where applicable.

**Dependency order:** 1.1→1.2 (1.2 unblocks 0.1) → 1.3/1.4 → 2.x → 3.x → 4.x → 5.1.
Within P6 the migration ran sequentially: **6.1 → 6.2 → 6.3** landed the unified function
model first (it touches shipped code), then **6.4 → 6.7** added the struct-field rights
(read/update/append). **6.1–6.7 are DONE**; **6.8** (group-existence + IR persistence) and
**6.9** (parameter `#default` locks) complete the surface and are the remaining P6 work.
**P6 and P7 are independent compile-time arcs** — both reject at load, neither
needs the @PLN85 memory-safe interpreter (that was only the dropped runtime layer), so they
slot alongside the P0–P3 core; **P8 rides on both**. **P0–P3 + P6 + P7 are the compile-time
core** (reject at load, game-safe); P4 has no abort path to make fail-safe; P5 proves *fast +
safe*. **Admission diagnostics (2.5, 3.5, 6.8) are first-class** — a clean compile is the
safety contract. A rung graduates its probe to `tests/scripts/` / `tests/sandbox.rs` when
green on both backends where applicable.

### The build flow — a verifiable sequence (every gate is runnable)

The ladder above is grouped by *arc*; this is the **order you actually build in**. It is
sequenced so the **shipped function surface migrates atomically** (the suite never sees a
mixed string/`group#right` model), then the new member + data surfaces layer on
**additively**, each behaviour change gated by a single RED→GREEN test. **The invariant of
the flow: `make ci` is green after every F-step**, so each lands as one PR-sized change on a
releasable tree.

**Foundation (additive — no admission change yet)**
- **F1 — capability decl + `group#right` token + resolver** (P6.1). ✅ **DONE.** *Gate:*
  `capability_declarations_register_and_resolve` — `capability fs` parses, `fs#read` resolves,
  `typo#read` is a load error.

**Migrate the function surface (the ONE atomic step over shipped code)**
- **F2 — parse the function call-gate link in the SIGNATURE (after the output, `-> int fs#read`),
  drop the `#cap` annotation, retag `default/02_files.loft` (`fs#read`/`fs#update`/`env#read` +
  the `capability fs`/`capability env` decls), fold `allow_caps`→`allow` with a `#`-splitting
  `cap_prefix_match` + a quote-aware `strip_comment`** (so a `#` survives in a TOML token). ✅
  **DONE.** *Gate:* `allow=["fs#read"]` admits `mtime`, rejects file `write` (`fs#update`); the
  plan86 admission suite (29) + `sandbox_cli` (5) + the cap IR round-trip green. This is the
  consistency cut — after F2 there is one capability model and `#cap` is gone.

**Per-member access — read / update / append (all DONE)**
- **F3 — member link parse + `member_access` carrier** (P6.4). ✅ **DONE.** *Gate:*
  `field_capability_links_are_recorded` — `loot: Item bag#read bag#append` records both rights;
  an unlinked field is empty. Parsed in BOTH field-type branches; `try_cap_link` made
  non-destructive (a `not null` / default after a type is never mis-consumed).
- **F4 — read admission** (P6.5). ✅ **DONE.** *Gate:*
  `field_read_gates_private_field_admits_unlinked` — a sandboxed read of a `#read`-linked field
  with no grant → Rejected; an unlinked read admits (read default-allow). Recorded at the
  field-access site (`fields.rs::field`).
- **F5 — update admission** (P6.6). ✅ **DONE.** *Gate:*
  `field_update_gates_writable_fields_unlinked_read_only` — `e.f = v` admits iff `#update` is
  granted; an unlinked field stays read-only (coarse 2.4). Generalises 2.4 per-field via a
  `last_field_target` stash verified against the base var's struct.
- **F6 — append admission** (P6.7). ✅ **DONE.** *Gate:* `field_append_gates_collection_grow` —
  `e.f += [x]` admits iff `#append` is granted; `=` stays the `#update` path. Routed by the
  operator, so `bag#read bag#append` (no update) is genuinely append-only.

**Complete the authored model surface (the remaining model work)**
- **F7 — parameter `#default` locks** (§7.2). A non-default argument to a parameter tagged
  `…#default` is gated at the call site; an untagged parameter is free (set is inherited from
  the call). *Gate:* `spawn(count: 5)` Rejected unless `spawn.count#default` is granted; bare
  `spawn()` (the default) admits. *(The one model surface still unbuilt.)*
- **F8 — group-existence validation + IR persistence + diagnostics** (P6.8). Wire
  `cap_is_declared` into admission so a link to an **undeclared** `capability` is a clean load
  error; round-trip the `member_access` carrier through the store codec so a warm-cached host
  type keeps its field links; polish each rejection's wording. *Gate:* `typo#read` → load error
  naming the unknown capability; a tagged member survives the store round-trip.

**Data envelope**
- **F9 — coefficient on `space_degree`** (P7.1). *Gate:* a per-entity build loop reports
  `(degree 1, coeff sizeof(struct))`. *(Reported — additive.)*
- **F10 — static-sizing gate** (P7.2). *Gate:* an uncapped string-build loop → Rejected; a
  `max_string_len`-capped one admits.
- **F11 — budget reject** (P7.3 + 7.4). *Gate:* a script whose `coeff · max_input_n^degree`
  exceeds `data_budget` → Rejected naming the figure; an under-budget one admits.

**Prove it**
- **F12 — `sandbox-check` verdict** (P8.1). *Gate:* `loft sandbox-check <profile> <file>`
  prints Admitted / Rejected and **never executes** (a side-effecting body proves no run).
- **F13 — RED/GREEN corpus** (P8.2). *Gate:* `cargo test --test sandbox` green on the real
  type defs + the migrated `02_files.loft`, both backends where applicable.

**Status:** F1–F6 are **landed** (the call gate + all three field rights). The next runnable
increment is **F7 (parameter locks)** — it completes the host-authored surface — then **F8**
closes the model (validation + persistence). F9–F13 (data envelope + tooling) follow and are
independent of the access work. F9 is additive; F4–F6 / F7 / F10–F11 each flip one RED probe.

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
