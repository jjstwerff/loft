<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# formal/capabilities.md — capability admission (strict, aspirational + conditional)

> **Rules then deviations** (see [README](README.md)). Two things set this area apart from
> `types` / `grammar` / `operational`, and both must be held in mind while reading:
>
> 1. **Conditional, not always-on.** The core areas constrain *every* program. These rules
>    bite only inside code a sandbox profile **designates** as restricted; trusted code
>    satisfies the judgment vacuously (`Cap-Trusted`). The judgment is parameterized by a
>    profile `P` — it is a refinement *layered on* the core under a policy, not part of what
>    every program obeys.
> 2. **Aspirational.** The call gate (incl. closures — a lambda body is descended into), the
>    field-level rights, AND the parameter `#default` lock are enforced today (`src/sandbox.rs`:
>    `admit_capabilities`, `field_*_violations`, `param_lock_violations`; `parser`:
>    `mark_lambda_sandboxed`); the one remaining deviation is the owned-vs-host classification's
>    completeness ([@PLN86 §7](../plans/86-sandbox-subset-flag/README.md)). Writing the rules now
>    is direction: it turns "what exactly may a mod do?" into a relation the admission walk is
>    checked against.
>
> The model with worked examples lives in [SANDBOX.md S10](../SANDBOX.md) +
> [@PLN86 §7](../plans/86-sandbox-subset-flag/README.md); this doc is the **rule**. The
> owned-vs-host predicate these rules lean on is [ownership.md](ownership.md)'s — cited, not
> redefined.

## The frame, in one line

A capability is a permission the **host** requires of a **restricted caller**. The host
annotates *its own* surface (functions, parameters, struct fields); a profile grants a set
of `group#right` tokens; admission gates every point a restricted caller's code touches the
host. **A modder's own code carries no links and is never restricted** — only host reach is
gated.

## Notation

- **`P`** — the active profile: `P.allow` (granted `group#right` tokens) and `P.libs`
  (libraries allowed wholesale). The host owns it; a script cannot grant itself (@PLN86 §1).
- **`ctx`** ∈ `{restricted, trusted}` — `restricted` inside a sandbox-designated def,
  `trusted` everywhere else.
- **`P ; ctx ⊢ e ✓`** — "under profile `P` in context `ctx`, operation `e` is **admitted**."
- **`g#r ∈ P`** (granted) ≝ `∃ a#r ∈ P.allow` with `cap_prefix_match(a, g)` — the **right is
  equal** and the grant's group `a` is a dotted-segment prefix of `g` (so `game#read` grants
  `game.entity#read`, never `game#update`). Groups are declared (`capability g`); a link to
  an undeclared group is a load error, prior to this judgment.
- **`owned(e)`** — `e` operates only on **script-owned** data (a locally constructed value,
  or a script-defined type). The complement is **host** data (a parameter root, a
  host-library type). This is a *provenance* fact — [ownership.md](ownership.md).
- **`gate(f)`** — function `f`'s call-gate link (its signature link, §7.1); `⊥` if untagged.
- **`default(p)` / `lock(p)`** — a parameter's default value, and its `…#default` lock
  (§7.2); `lock(p) = ⊥` if the parameter is untagged.
- **`readcap(m)` / `cap(m, r)`** — field `m`'s read link, and its link for right
  `r ∈ {update, append}` (§7.3); `⊥` if untagged.

---

## Rules

> The judgment is **sound** (an admitted operation lies within `P`'s grants — no host effect
> escapes the profile) and **complete** (every host-touching construct has a rule, so
> `Cap-Deny` is reached only by genuine absence of a grant, never by an un-judged construct).

```
  (Cap-Trusted)  ctx = trusted                                           ⟹  P ; ctx ⊢ e ✓
  (Cap-Own)      owned(e)                                                ⟹  P ; ctx ⊢ e ✓
  (Cap-Call)     e = call f ;  gate(f) = g#r ;  ( g#r ∈ P  ∨  lib(f) ∈ P.libs )
                                                                          ⟹  P ; restricted ⊢ e ✓
  (Cap-Set)      e = pass arg a to param p of host f ;
                 a ≡ default(p)  ∨  lock(p) = ⊥  ∨  lock(p) ∈ P           ⟹  P ; restricted ⊢ e ✓
  (Cap-Read)     e = read host field m ;   readcap(m) = ⊥  ∨  readcap(m) ∈ P
                                                                          ⟹  P ; restricted ⊢ e ✓
  (Cap-Write)    e = write host field m, right r ∈ {update, append} ;
                 cap(m, r) = g#r ∈ P ;   ( r = append ⟹ m : collection ) ⟹  P ; restricted ⊢ e ✓
  (Cap-Deny)     no rule above applies                                   ⟹  e is REJECTED at load
```

**In words.** Outside the sandbox nothing is gated (`Cap-Trusted`). Inside it, anything a
script does to data **it owns** is free (`Cap-Own`) — only reaching into the *host* is
checked. To **call** a host function you need its gate granted, or its whole library allowed
(`Cap-Call`); once you may call it, you may pass any **argument** — except a parameter the
host **locked** to its default, which needs the lock (`Cap-Set`). **Reading** a host field is
free unless the host marked it private (`Cap-Read`); **updating or appending** to one needs
the field's grant, and *append* is only meaningful on a collection (`Cap-Write`). Anything
with no admitting rule is rejected **at load** — deny-by-default for the write side,
allow-by-default for reads and for argument-passing.

### Construction is unrestricted (the closed decision)

There is **no** `(Cap-Construct)` rule, by decision (the @PLN86 design's "position 1").
Building a host value — an enum variant `Command.Shutdown`, a struct `Entity{…}` — is
**free**: a constructed value is script-owned, so `Cap-Own` already admits it. Such a value
is inert until it **enters host state**, and every entry point is one of the boundary rules
above (`Cap-Write` to put it in a host field, `Cap-Call`/`Cap-Set` to pass it to a host
function). The "un-forgeable variant" property is therefore a property of the **boundary**,
not of construction: a granted `q#append` of element type `Command` *is* a grant to append
any `Command`, so if a privileged variant must not enter `q`, the host's obligation is to
narrow the element type or validate at the consumer — the **L-write contract** (a granted
write op is invariant-preserving for any value its type admits), not a new right.

### Why the rule set is closed

Every way a restricted caller can touch the host is one of: **call** a function, **pass** it
an argument, **read** / **update** / **append** a field. `Cap-Call`/`Cap-Set`/`Cap-Read`/
`Cap-Write` cover those four, `Cap-Own`/`Cap-Trusted` carve out the un-gated remainder, and
`Cap-Deny` is the deny-by-default floor. The model compressing to **six judgments** is the
evidence it is at the right altitude — an operation that needed a seventh rule would be an
edge the rules can't express, i.e. a signal the *rule* is wrong (README), not a new case.

---

## Deviations

OPEN: **1**. `Cap-Call` (the call gate + closures), `Cap-Read`/`Cap-Write` (the field rights),
AND `Cap-Set` (the parameter `#default` lock) are all enforced today; the one remaining deviation
is the owned-vs-host classification's completeness (`Cap-Own` soundness).

**`Cap-Set` (the parameter `#default` lock) — CLOSED (2026-07-04).** A `group#default` link on a
parameter (`count: integer = 1 spawn.count#default`) now parses (`definitions.rs`, first pass →
`pending_param_locks` → `param_locks`) and gates at a sandboxed call site: an argument that
DIFFERS from the parameter's default without the lock granted is a violation
(`param_lock_violations`; an argument equal to the default is not an override). Group-existence
validation + `member_access` IR persistence (6.8) landed alongside. Proven by the non-vacuous
RED/GREEN twin `param #default lock override` in `access_corpus_red_green` (`src/parser/mod.rs`):
`spawn("g", 5)` is rejected under `world#append` alone and admitted once `spawn.count#default` is
granted.

### D-cap-2 — a closure may carry authority across the boundary — CLOSED (2026-07-04)
- **Violates:** Cap-Call (completeness — an indirect call must resolve to its callee's gate)
- **What it WAS (corrected by probing — the earlier framing was off).** A lambda def created in
  a sandboxed body was never added to `def_sandbox`, so the admission walk treated it as an
  **untagged leaf** (`sandboxed \`f\` reaches \`__lambda_N\` … neither an allowed library nor a
  granted capability`). That was SOUND (no escape — every lambda rejected) but a **blunt
  over-reject**: even a script-only `[1,2,3].map(|y| y*2)` was rejected, making lambdas unusable
  in sandboxed code, and a lambda that DID reach a host cap was rejected without naming the real
  reach.
- **The fix (`src/parser/vectors.rs::mark_lambda_sandboxed`, called from `parse_lambda` /
  `parse_lambda_short`).** A lambda created while its enclosing def is sandboxed is itself marked
  sandboxed under the SAME profile, so the admission walk **DESCENDS into its body** (checks its
  calls / fn-refs / raw-writes precisely) instead of stopping at an untagged leaf. Nested lambdas
  inherit transitively; a no-op outside a sandbox (`def_sandbox` empty), so non-sandbox lowering
  is byte-identical.
- **Why this is complete for the closure class** (probed): (1) a host call in the lambda body →
  caught by descending, naming the reach; (2) a captured host **fn-ref** (`cap = host_fn; …|y|
  cap()…`) → gated at its CREATION site in the enclosing def (the `cap = host_fn` Set that
  `referenced_defs` records — the capture cannot outrun that); (3) a raw write to a captured host
  **struct** is not an escape — writing a captured struct field is an unsupported construct that
  panics codegen on BOTH backends, so it can never run. A fn-ref laundered through a host-call
  RETURN then captured is the separate L4-return residual (`sandbox.rs::referenced_defs` §RESIDUAL,
  not closure-specific), not D-cap-2.
- **Proven by:** the escape `cap: lambda body reaches ungranted host (D-cap-2)` + the control
  `script-only lambda is usable (D-cap-2)` in `admission_escape_suite_rejects_every_breakout`, and
  the non-vacuous RED/GREEN twin `lambda body reaches host cap (D-cap-2)` in
  `access_corpus_red_green`.

### D-cap-3 — `Cap-Own`'s owned-vs-host split is a conservative syntactic heuristic, not the fact
- **Violates:** Cap-Own (completeness — a script-owned mutation should be admitted, not rejected)
- **Where:** `src/parser/expressions.rs::raw_write_is_host_owned` — the classifier that gates a raw
  field/index write BEFORE it is recorded. It already admits the common owned case (`s.f = v` where
  `s`'s root is a non-parameter local of a **script-defined** struct type → `Cap-Own`), and rejects
  host data (a parameter root, or any host-library struct — the type catches aliasing like
  `x = player; x.health = …`). But it is a **syntactic type-provenance heuristic**, deliberately
  conservative ("when ownership can't be proven script, treat as host"), NOT ownership.md's
  `ownership_of` fact. The dependency has CHANGED: `D-own-2` (O-Complete) is now **CLOSED**
  (2026-07-04, @PLN90 loft#495), so a total provenance fact now EXISTS to replace the heuristic.
- **Effect:** admission is SOUND (the heuristic never admits a host write) but INCOMPLETE — it
  under-admits genuinely-script-owned writes the syntax can't prove: notably a **script-owned
  vector/collection element write** (`v[i] = e` — the classifier requires a `Reference(struct)`
  root, so any owned vector/scalar target falls to host), and any owned root whose script
  provenance is only visible in the ownership fact, not the declared type.
- **Status:** OPEN — the dependency (D-own-2) is now met; the residual is sandbox-side: widen the
  classifier from the syntactic heuristic to the `ownership_of` fact so every genuinely-owned
  target is admitted, host targets still rejected.
- **Removal:** `raw_write_is_host_owned` consults `ownership_of` for the write target's root —
  admit when the root is script-owned (incl. owned vectors/collections), reject only a host-rooted
  write. Add GREEN twins to the escape suite: a sandboxed `v[i] = e` / `s.f = v` on locally-owned
  data is admitted; the host-aliasing RED still rejects.
- **Design note (the load-bearing invariant, probe before building — it is the security
  boundary).** The candidate invariant is *a raw write is admissible iff its target store is
  `Owned` (not a borrow of host state)* — which `ownership_of` gives directly. This is SOUND only
  because of **C86** (a whole-value heap bind COPIES): `x = player; x.health = …` produces an
  OWNED copy, so the write cannot reach the host `player` — the exact aliasing case the current
  type-heuristic rejects, now admissible *because it is provably a copy*. The residual risk is any
  case where `ownership_of` returns `Owned` yet the store still aliases host state (a view/
  projection mis-classified as owned) — that would be a sandbox ESCAPE, not just an over-reject.
  So the build must be gated by adversarial escape-suite probes: for every shape where a script
  value could still reference host state (`&`-borrow, projection `p = host.inner`, a view returned
  from a host call), a raw write must still hit `Cap-Deny`. Do NOT widen the boundary until each of
  those RED probes is proven to still reject under the `ownership_of` gate.

---

## Conformance

This area's **falsifying programs** are the adversarial escape suite
(`admission_escape_suite_rejects_every_breakout`, `src/parser/mod.rs`): each is a restricted
program that *tries* to perform a host operation outside its grants — an ungranted call, an
indirect fn-ref call, a raw write — and the rule it must hit is `Cap-Deny`. The positive
controls (a granted call, a read, a script-owned mutation) are the `Cap-Call` / `Cap-Read` /
`Cap-Own` side. The field-right and parameter-lock breakouts have landed as RED/GREEN twins in
`access_corpus_red_green` (update a read-only field; append where only update is granted; pass a
non-default value to a locked parameter) — each a program whose admission flips to the exact rule
above, its GREEN twin proving the rejection is the rule firing, not an incidental reject.

The area is **formal when OPEN reaches 0**: every host-touching operation in a restricted
context decided by exactly one of the six rules, over a complete provenance fact (D-cap-3) —
at which point an admitted script provably performs no host effect outside its profile, by
construction.
