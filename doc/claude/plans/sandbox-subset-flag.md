<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# sandbox-subset-flag — per-subset sandbox mode amidst unrestricted code

## Status

**DRAFT — design-ready, no implementation.** Pending a `loft-lang/plans` issue to
claim its `@PLN<N>` identity (then rename to `<N>-sandbox-subset-flag/README.md`).
The concrete first slice of [SANDBOX.md](../SANDBOX.md): a compiler-enforced flag
that runs **designated subsets** of a program (user scripts) under an admission
policy — capability/library allow-list + loop/recursion bounds + fault isolation —
while the **surrounding host code runs unrestricted**, in one process, sharing the
store at full speed.

## Goal

Let a host program (a game engine, an in-game console) embed user scripts that have
fast, direct `DbRef` data access **without letting them break the host**. The host
marks which code is sandboxed and under which **named policy**; the compiler admits
that subset (rejecting anything outside the policy at compile time) and injects
runtime guards (fuel, depth) into the sandboxed code only. Unrestricted code pays
nothing.

## Design

### 1. The host designates subsets — not the script

The sandbox designation is **authoritative from the trusted side**, so a script
cannot opt itself out. Two complementary inputs, both host-controlled:

- **Manifest / CLI (primary).** `loft.toml` or a flag maps file/function globs to a
  named profile:
  ```toml
  [sandbox]
  mod-script = ["mods/**/*.loft"]      # these sources compile under profile "mod-script"
  console    = ["fn:player_eval"]      # a single function
  ```
- **Source annotation (secondary, for first-party authoring).** `#sandbox("mod-script")`
  on a `fn`/module — only honoured for sources the host already trusts to self-mark;
  for untrusted scripts the manifest wins and the annotation is ignored.

The compiler tags each `def` with its profile (or "unrestricted"), then computes the
**sandbox-reachable set** = the call-graph closure of the sandboxed entry points.
Admission runs over that whole set, not just the entry function.

### 2. The boundary semantics (sandboxed ↔ unrestricted)

| Direction | Allowed? |
|---|---|
| sandboxed → another sandboxed def | yes (stays in the set) |
| sandboxed → an **allow-listed** host symbol | yes — the host symbol is the trusted API; it runs *unrestricted* (no guards) as a vetted primitive |
| sandboxed → a non-allow-listed unrestricted def / native FFI / file·net·env | **compile error** (capability escape) |
| unrestricted → a sandboxed def | yes — the caller gets a `Result`; a sandbox abort/error never crashes the caller (the [SANDBOX.md S8](../SANDBOX.md) boundary) |

The allow-list **is** the host-exposed API surface; everything else is denied.

### 3. Capability groups — declared in the code, not a drift-prone external list

The allow-list must **not** be a separate layer that enumerates symbol names: a
rename or a move silently breaks such a list, and a function's *capability is a fact
that belongs with the function* (one home per fact — the
[STABILITY_REDFLAGS.md](../STABILITY_REDFLAGS.md) thesis applied here). So each API
function / method / type is tagged with a **capability group at its definition**, and
a sandbox profile selects **groups** — never individual names.

**Declared at the definition** (alongside the existing `#native` / `#rust`
annotations), a `#cap "<group>"` tag; groups are dotted + hierarchical so granularity
is whole-type *or* single-method, in the same mechanism:

```
fn read_file(path: text) -> text;
#native "n_file_read"
#cap "fs.read"                              # this fn is in capability group fs.read

fn http_get(url: text) -> HttpResponse { ... }
#cap "net"

pub fn get(self: vector, i: integer) -> ...;   #cap "collections.read"
pub fn clear(self: vector);                     #cap "collections.write"
```

A whole type / module may carry a group its members inherit, overridden per-method —
so `Vector.get` and `Vector.clear` land in different groups with no extra layer.

**The profile selects groups** (deny-by-default) + the limits:

```toml
[profile.mod-script]
backend       = "interpret"     # never native/rustc — RCE by construction (S2)
native_ffi    = false           # no [native] cdylib (S2)
max_loop_iter = 1_000_000       # per-script fuel; abort the SCRIPT, not the host (S4)
max_call_depth = 256            # recursion cap (S3)
allow_caps    = ["game.read", "game.write", "math", "collections.read", "text"]
# anything tagged OUTSIDE these groups — fs.*, net, env, collections.write — is denied.
# Prefix match: allow "game" ⊇ {game.read, game.write}; allow "game.read" is exact.
```

**Why groups beat a name-list (the no-drift property):** the group travels with the
symbol. Rename/move a fn → its `#cap` moves with it. Add a fn to `game.read` → it is
covered automatically. Change a fn's capability → edit one site. The config is a set
of *group names* that change only when the capability **taxonomy** changes (rare),
not when the **API** changes (constant). An **un-tagged** symbol has *no* capability
→ denied by default, which forces the API author to classify it (un-tagged = unsafe).

### 4. Compile-time enforcement (admission)

Over the sandbox-reachable set, in the existing two-pass compiler (no new engine —
it has the typed IR + resolved call graph + one import resolver `Parser::lib_path`):

1. Every referenced symbol carries a `#cap` group whose name is matched (prefix) by
   the profile's `allow_caps` — **or** is another sandboxed def. Else a compile error
   naming the symbol *and* its group (or "untagged → denied"). The group is read from
   the symbol's own definition; there is no separate list to consult or drift.
2. Every `use` resolves to a lib with no `[native]` crate (S2). Per-symbol capability
   is the group check in (1), not a per-lib allow-list.
3. Recursion in the subgraph is bounded (static cap or flagged for the runtime depth
   guard); loops are instrumented for the fuel guard.
4. The backend for sandboxed defs is forced to the interpreter.

### 5. Runtime guards (sandboxed code only)

Injected into sandboxed defs, zero cost elsewhere:
- a **fuel counter** decremented at loop back-edges + calls; exhaustion → recoverable
  `ScriptError`, not a process kill;
- a **call-depth** counter on the sandbox stack;
- faults (fuel, depth, runtime errors) surface through the `run_script(...) -> Result`
  boundary (SANDBOX.md S8).

## Phases

| Phase | Deliverable | Validation gate (runnable) |
|---|---|---|
| **P0** | Parser recursion-depth guard (clean error, not SIGSEGV) — SANDBOX.md S5; prerequisite for any admission pass | the nested-input probe returns a parse error, not `rc=139` |
| **P1** | Per-`def` profile tag + sandbox-reachable-set computation + `backend=interpret`/`native_ffi=false` enforcement (S2) | a sandboxed def that `use`s a `[native]` lib or compiles `--native` is rejected |
| **P2** | `#cap "<group>"` annotation + tag the stdlib/API surface with groups + group-based admission (S1) | a sandboxed script calling `read_file` (`fs.read`, not in `allow_caps`) or `Vector.clear` (`collections.write`) fails admission naming the *group*; `Vector.get` (`collections.read`, allowed) passes |
| **P3** | Runtime fuel + call-depth guards in sandboxed defs; `run_script() -> Result` boundary (S3/S4/S8) | `while true {}` in a sandboxed def aborts the script; the host keeps serving |
| **P4** | Effect containment — sandboxed writes to a journaled/scratch store, commit-or-discard (S7) | a sandboxed script mangles state, discard → live world byte-identical |

P0→P3 are the "buildable now" slice; P4 builds on the `journal` substrate.

## Open questions

- **Where the group tag lives** — a `#cap` source annotation per def (least drift,
  but touches every API source), vs a per-library group table in its `loft.toml`
  (fewer edits, one indirection). Source annotation is the default; a lib table may
  be the pragmatic form for large existing surfaces.
- **Default group for un-tagged symbols** — hard deny (forces classification) vs a
  reserved `unsafe`/`host` group that no script profile may allow. Leaning hard-deny.
- **Tagging the existing stdlib surface** — the one-time effort to assign `#cap`
  groups to `default/*.loft` + the registry libs (`fs.*`, `net`, `env`, `proc`,
  `collections.{read,write}`, …); itself valuable capability documentation.
- **Group hierarchy semantics** — prefix-allow (`game` ⊇ `game.*`) vs exact-only;
  whether a method may carry multiple groups.
- **Profile composition** — base + per-mod override merge.
- **Data-write policy** — does v1 give sandboxed code direct store *writes* (fast,
  contained only by P4) or route writes through `*.write`-grouped API only?
- **Per-frame vs per-invocation fuel** — budget reset cadence for game loops.

## See also

- [SANDBOX.md](../SANDBOX.md) — the invariants S1–S8 + the buildable-now slice this
  plan implements.
- Symbol naming (`n_<name>` / `t_<LEN><Type>_<method>`): `CLAUDE.md § conventions`.
