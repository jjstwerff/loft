<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 86 — sandbox-subset-flag: per-subset sandbox mode amidst unrestricted code

## Status

**Open — full design, no implementation.** Plan **@PLN86**
([loft-lang/plans#86](https://github.com/loft-lang/plans/issues/86)) · `status:future`
· `subject:loft`. The concrete first slice of [SANDBOX.md](../../SANDBOX.md): a
compiler-enforced flag that runs **designated subsets** of a program (user scripts)
under an admission policy — capability-group allow-list + loop/recursion bounds +
fault isolation — while the **surrounding host code runs unrestricted**, in one
process, sharing the store at full `DbRef` speed.

## Goal

Let a host (a game engine, an in-game console, a player playground) embed user
scripts with fast, direct data access **without letting them break the host**. The
host marks which code is sandboxed and under which named profile; the compiler admits
that subset (rejecting policy violations at compile time) and injects runtime guards
into the sandboxed code only. Unrestricted code pays nothing.

## The invariant (the one thing every step defends)

> **No execution path reachable from a sandboxed entry point — direct or indirect —
> can (a) reach a symbol whose capability group the profile does not allow, (b) exceed
> the profile's fuel / call-depth budget, or (c) leave a store effect that survives a
> discard.** Admission proves (a) statically; runtime guards enforce (b); the
> transactional store gives (c).

Everything below either establishes part of this invariant or is a probe that tries
to break it.

## Design

### 1. The host designates subsets — not the script
Authoritative from the trusted side, so a script can't opt itself out:
- **Manifest / CLI (primary):** `loft.toml` `[sandbox]` maps file/function globs to a
  named profile (`mod-script = ["mods/**/*.loft"]`, `console = ["fn:player_eval"]`).
- **Source annotation (secondary):** `#sandbox("mod-script")` on a `fn`/module —
  honoured only for sources the host already trusts; the manifest always wins.

The compiler tags each `def` with its profile (or "unrestricted") and computes the
**sandbox-reachable set** — the closure over **references** (calls *and* fn-ref
literals *and* type uses), not just direct calls. Admission runs over that whole set.
Closing over *references* (not calls) is what makes indirect calls safe — see C1.

### 2. The boundary (sandboxed ↔ unrestricted)
| Direction | Allowed? |
|---|---|
| sandboxed → another sandboxed def | yes |
| sandboxed → an allow-listed host symbol | yes — runs *unrestricted* as a vetted, trusted primitive (see C2/C3 for its contract) |
| sandboxed → a symbol outside the allowed groups / native FFI / file·net·env | **compile error** |
| unrestricted → a sandboxed def | yes — caller gets a `Result`; a sandbox fault never crashes it (SANDBOX.md S8) |

### 3. Capability groups — declared in the code, never a drift-prone list
Each API function/method/type carries a `#cap "<group>"` tag **at its definition**
(beside `#native`/`#rust`); a profile selects **groups**, never names — a function's
capability is a fact that lives with the function (one home per fact,
[STABILITY_REDFLAGS.md](../../STABILITY_REDFLAGS.md)).

```
fn read_file(path: text) -> text;  #native "n_file_read"  #cap "fs.read"
fn http_get(url: text) -> HttpResponse { ... }            #cap "net"
pub fn get(self: vector, i: integer) -> ...;              #cap "collections.read"
pub fn clear(self: vector);                                #cap "collections.write"
```
```toml
[profile.mod-script]
backend = "interpret"; native_ffi = false       # never native/rustc, no cdylib (S2)
max_loop_iter = 1_000_000; max_call_depth = 256  # fuel + depth (S3/S4)
allow_caps = ["game.read", "game.write", "math", "collections.read", "text"]
# fs.*, net, env, collections.write → tagged outside → denied. Prefix: "game" ⊇ game.*.
```
The group travels with the symbol (rename/move/add-to-group all just work); an
**un-tagged symbol has no capability → denied** (forces classification). The config
changes only when the capability *taxonomy* changes (rare), not the *API* (constant).

### 4. Compile-time admission (no new engine — typed IR + call graph + `Parser::lib_path`)
Over the reachable set: (1) every referenced symbol's `#cap` group is prefix-matched
by `allow_caps`, or it's another sandboxed def, else a compile error naming the
symbol *and* its group; (2) no `use` of a `[native]` crate; (3) loops instrumented +
recursion flagged for the runtime guards; (4) backend forced to interpreter.

### 5. Runtime guards (sandboxed defs only, zero cost elsewhere)
A fuel counter (loop back-edges + calls), a call-depth counter, and faults delivered
through a `run_script(...) -> Result<_, ScriptError>` boundary — never `exit()`.

## Load-bearing claims — and the cheapest probe that falsifies each

A design is a hypothesis; these are the assumptions that, if wrong, sink it. Each
ships with a falsifying probe to run *before* trusting the claim.

- **C1 — the reachable set is statically complete (indirect calls can't escape).**
  loft has fn-ref values / closures. *Risk:* a script obtains a reference to a denied
  fn and calls it indirectly, bypassing the call-graph walk. *Mitigation:* admission
  closes over **references**, so `let f = read_file` *is* a reference to `read_file`
  → in the set → rejected; you cannot indirectly call what was never referenced.
  **Probe:** `let f = read_file; f("/etc/passwd")` in a sandboxed def → MUST be
  rejected naming `fs.read`. Also: a host-supplied callback fn-ref is the host's
  responsibility (an allow-listed boundary), not the script's.

- **C2 — an allow-listed host fn is safe to run unguarded.** It runs unrestricted (no
  fuel). *Risk:* `host_fn(n)` does O(n) work for a script-controlled huge `n` → hangs
  past the script's budget. *Mitigation:* allow-listing is a **trust contract** — an
  allow-listed host fn is the host author's promise it is bounded on any script input;
  plus the existing wall-clock `--timeout` as a defense-in-depth backstop. **Probe:** a
  deliberately-unbounded allow-listed fn called with a huge arg → confirm only the
  wall-clock backstop stops it, i.e. the contract is *necessary* (document it as a gate
  on what may be tagged `game.*`).

- **C3 — discard rolls back *all* of a script's effects.** Sandboxed code can't call
  natives, so its direct effects are store writes (journaled). *Risk:* a script calls
  an allow-listed **write** host fn whose mutation lands outside the scratch store →
  survives discard. *Mitigation:* any host API in a `*.write` group exposed to a
  sandbox **must** target the transactional layer (journal-aware writes). **Probe:** a
  `game.write` fn mutates state, then discard → world byte-identical; a write that
  escapes the journal is a tagging bug, caught here.

- **C4 — the group surface is fully tagged (no un-tagged escape).** *Risk:* a public
  API fn without `#cap` is reachable and silently denied-or-allowed inconsistently.
  *Mitigation:* un-tagged = deny + a CI lint. **Probe:** a lint over `default/*.loft` +
  registry libs fails if any public symbol lacks a `#cap` group.

## The verifiable step ladder

Bottom-up, each rung independently verifiable (a runnable check that is RED before and
GREEN after), each building on the prior. Cross-mode note: sandboxed code is
interpret-only (S2), so runtime checks are on `--interpret`; admission + the parser
guard are backend-agnostic (compile time).

**P0 — crash-safe validator (prerequisite: you can't gate on a parser that segfaults)**
- **0.1 Parser recursion-depth guard.** *Change:* a nesting-depth counter in the
  recursive-descent expression/operator parser; a parse *error* past a configurable
  depth. *Verify:* the 5000-deep nested-input probe returns a parse error, not
  `rc=139`; a legal program below the limit still compiles. *Establishes:* S5 — hostile
  source is rejected, never crashes the validator.

**P1 — designation + reachable set + coarse bans**
- **1.1 Policy model + parse.** *Change:* parse `[sandbox]` + `[profile.*]` into a
  `Policy` (allow_caps prefix-set, limits, backend, native_ffi). *Verify:* round-trip
  unit test; a malformed policy is a clear error.
- **1.2 Per-def profile tag.** *Change:* tag each `def` with `Option<ProfileId>` from
  the manifest globs. *Verify:* `loft introspect` shows exactly the designated defs
  tagged; an annotation on an untrusted source is ignored (manifest wins).
- **1.3 Reachable-set over references.** *Change:* compute the closure over calls +
  fn-ref literals + type uses from sandboxed entry points. *Verify (C1):* a probe whose
  graph includes an indirect `let f = g` puts `g` in the set; the computed set equals
  the hand-computed set.
- **1.4 Backend + FFI ban.** *Change:* sandboxed defs forced to interpret; a sandboxed
  `use` of a `[native]` lib rejected. *Verify:* sandboxed `--native` / native-lib use
  rejected; `cargo build --no-default-features` proves the cdylib path is removable.

**P2 — capability groups + group admission**
- **2.1 `#cap` annotation.** *Change:* parse `#cap "<group>"` onto the def (like
  `#native`); type/module group with per-method override. *Verify:* a def's group is
  readable; `Vector.get`=`collections.read`, `Vector.clear`=`collections.write`.
- **2.2 Tag the stdlib/API surface.** *Change:* assign groups across `default/*.loft` +
  registry libs (`fs.*`, `net`, `env`, `proc`, `collections.{read,write}`, `math`,
  `text`, …). *Verify (C4):* the lint passes — every public symbol carries a group.
- **2.3 Group-membership admission.** *Change:* every reachable symbol's group ∈
  `allow_caps` (prefix) or another sandboxed def, else a compile error naming
  symbol+group. *Verify:* `read_file` (`fs.read`, not allowed) rejected naming the
  group; `Vector.clear` rejected; `Vector.get` passes; **and the C1 probe**
  `let f = read_file; f(...)` rejected.

**P3 — runtime guards + embedding boundary**
- **3.1 Call-depth counter** (sandboxed defs only). *Verify:* deep recursion → a
  `ScriptError`, not a stack overflow; unrestricted recursion is untouched (no counter).
- **3.2 Fuel counter** at loop back-edges + calls in sandboxed defs. *Verify:*
  `while true {}` → `ScriptError`; a bounded loop within budget completes; unrestricted
  code carries no counter (a perf check confirms zero cost).
- **3.3 `run_script(src, policy, world) -> Result`.** *Change:* a `src/lib.rs` entry
  that compiles+admits+runs and returns faults as values, never `exit()`. *Verify
  (S8):* a host harness runs OOB / div-by-zero / fuel-exhaust / depth-exceed scripts
  and keeps serving after each.

**P4 — effect containment**
- **4.1 Transactional scratch store.** *Change:* sandboxed writes go to a journal
  snapshot (`journal.rs::snapshot` + `copy_block_cross_store`); commit applies
  atomically, discard rolls back. *Verify (C3/S7):* mangle-then-discard → the world's
  hash is byte-identical to before; commit → atomic apply.

**P5 — performance gate**
- **5.1 Benchmark.** *Verify (S6):* a sandboxed script reading/writing N entities/frame
  stays within a frame budget vs a native baseline (the in-process direct-store claim).

P0–P3 are the buildable-now slice; P4 builds on the `journal` substrate; P5 proves the
whole premise (fast *and* safe). A rung graduates its probe to `tests/scripts/` or
`tests/sandbox.rs` when it passes green on both backends where applicable.

## Open questions
- **Where the group tag lives** — `#cap` per def (least drift) vs a per-lib `loft.toml`
  group table (fewer edits, one indirection) for large existing surfaces.
- **Default for un-tagged** — hard-deny (leaning) vs a reserved `host`/`unsafe` group no
  profile may allow.
- **C2 contract enforcement** — can "allow-listed ⇒ bounded" be checked, or only
  documented + backstopped by the wall-clock timeout?
- **Group hierarchy** — prefix-allow vs exact; may a symbol carry multiple groups?
- **Profile composition** — base + per-mod override merge.
- **Fuel cadence** — per-frame vs per-invocation reset for game loops.

## See also
- [SANDBOX.md](../../SANDBOX.md) — invariants S1–S8 + the buildable-now slice.
- Symbol naming (`n_<name>` / `t_<LEN><Type>_<method>` — why method/type granularity is
  free): `CLAUDE.md § conventions`.
