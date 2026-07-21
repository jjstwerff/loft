<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN113 — Contract-keyed semantics: in-depth design

Companion to [README.md](README.md).  This is the mechanism in depth and, more importantly, the
**decisions it blocks on** — each `❓ Qn (NEEDS ANSWER)` is a fork an implementer cannot resolve alone
because the answer changes the design, not just the code.  A recommendation is given for each, but the
owner's call is what unblocks the arc.  The decisions are collected in the table at the end.

## The load-bearing invariant

**A contract is a property of the source that *wrote the token*, not of the runtime call stack.**  When
a contract-1 program calls a contract-0 library whose body does `len(s)`, that `len` uses **contract-0**
(byte) semantics — because the library's logic was authored against byte-length and depends on it.  The
caller's own `len` uses contract-1.  This is exactly Rust's edition rule (a 2015 crate stays 2015 even
when a 2021 crate calls it), and it is what makes "carry both, break nothing" actually hold.  It also
tells us **where** the decision is made: at *compile* time, per call site, from the enclosing
definition's contract — never a runtime branch on a dynamic caller.

Everything below is downstream of this one invariant.

---

## Arc A — Contract propagation + persistence

Each definition (fn/method/struct/operator) is compiled from source owned by a package (or the entry
program, the stdlib, or a bare script).  We must attach to every definition the single integer "which
contract was this authored under," and have the `len/size(text)` call site read it.

The manifest already carries `package.contract` (`src/manifest.rs`), but as a **compatibility
predicate** (`>=1`, `1,3`, `=2`) consumed by `check_contract` for *resolver tolerance* — "which binaries
can run me."  Semantic keying needs a different thing: a single *authored-at* anchor — "which meaning of
`len` did I intend."  These are not obviously the same number.

> ### ❓ Q1 (NEEDS ANSWER) — One field or two: is the semantic anchor the same as the resolver predicate?
> A package that declares `contract = ">=1"` tolerates any binary ≥1.  On a contract-5 binary, does its
> `len(text)` use contract-**1** semantics (the meaning it was written for) or contract-**5**?
> - **Option 1 (recommended): anchor = the lower bound of the existing `contract` predicate.**  `>=1`
>   → authored-at 1 → `len` uses contract-1 semantics forever, even on a contract-5 binary.  One field;
>   matches "authored for."  A bare `1` already means `>=1` under the promise, so most packages get the
>   right anchor for free.
> - **Option 2: a separate `edition`/`semantics` field** distinct from `contract` tolerance.  More
>   precise (a package could tolerate ≥1 but pin semantics to 3), but two knobs to teach and keep
>   consistent.
> **Why it blocks:** Option 2 adds a manifest field and a store field; Option 1 does not.  The whole
> shape of arc A depends on this.

> ### ❓ Q2 (NEEDS ANSWER) — What contract does *undeclared* source get (manifest with no `contract`, and bare scripts / REPL)?
> Compatibility-is-absolute forces existing programs to keep old `len` → undeclared must default to
> **contract 0** (oldest).  But then every *new* project and every bare `loft run script.loft` gets the
> old byte-`len` by default — the "wrong," pre-flip behavior is what a new user hits first.
> - **Option A: undeclared = 0 everywhere.**  Safest; but new users must know to add `contract` to get
>   char-`len`, and bare scripts never get it.
> - **Option B: undeclared-with-manifest = 0; bare-script / REPL = latest.**  New scripts feel modern;
>   but an *old* bare script silently changes meaning under a new binary (a real, if narrow,
>   compatibility gap for the manifest-less surface).
> - **Option C (recommended): undeclared = 0, and `loft new` scaffolds `contract = "<latest>"`.**  New
>   *projects* opt in via tooling (like `cargo new` writing `edition`), existing/bare code stays 0.
>   Leaves only the bare-script default open (fold into A: bare = 0).
> **Why it blocks:** decides the default a million future programs inherit; irreversible in spirit once
> shipped.

> ### ❓ Q9 (NEEDS ANSWER) — Granularity: package-level only, or also a per-file / per-block override?
> Editions are per-crate.  loft's natural unit is the package `contract`.  Do we also allow a per-file
> pragma (a large package migrating one file at a time)?  **Recommended: package-only for v1** — a
> per-file override is addable later without breaking anything.  Answer needed only to *close* the door
> deliberately vs. leaving it implied.

**Persistence.**  The authored-at contract must survive to the IR store, so a reloaded/compiled store
replays the semantics it was built under.  `DEF_SUPERSEDED` (@PLN102 arc C) added +4 to
`DEFINITION_STRIDE` (159); a `DEF_CONTRACT` slot adds +4 more.  An old store lacks the field → reads as
**0** on load, which is exactly correct (it predates the flip).

> ### ❓ Q7 (NEEDS ANSWER) — Is a store-format stride bump acceptable, with "missing field ⇒ contract 0" as the migration?
> This touches @PLN97 (the layout contract / byte-layout the store commits to).  "Absent ⇒ 0" is the
> clean, correct default, but it is a store-schema change and should be acked against @PLN97's
> conformance/golden-hash gate rather than slipped in.

---

## Arc B — Compile-time op selection (the mechanism)

The mechanism, illustrated on `len/size` (both ops changed meaning per `phase0-inventory.md`).  A
diverged op *would* get two permanent variants — e.g. `length_text` (chars) + `length_text_v0` (bytes),
and the `size` counterparts — in `src/fill.rs` **and** `src/generation/` (both-backends rule).
Compiling a `len(text)` call, codegen reads the enclosing definition's contract and emits the new op if
≥ the change's contract, else the `_v0` op.  No runtime cost, no per-call lookup — a compile-time fact,
per the loft-codegen method.  (For `len/size` this stays hypothetical: the flip is pre-1, so no `_v0` is
actually built — see below.)

For a *post-1* diverged op, this is a **`CONTRACT_VERSION` bump**: old-contract source keeps the old
meaning, new-contract source gets the new one, one binary carries both.  The `len/size` flip is the
worked example of the shape — but the **actual** flip is a pre-contract-1 free swap (#587), so it needs
**no `_v0` variant**: contract 1 simply ships with `len`=chars, and no frozen program ever expected
byte-`len`.  The Stage-A matrix (README) shows the mechanism on that example, not an acceptance gate for
the flip itself.

> ### ✅ Q5 (RESOLVED — owner, 2026-07-20) — The flip lands *before* contract 1.
> Neither "flip = contract 1" nor "flip = contract 2": the flip is a **pre-contract-1 free break**
> (already shipped as a hard swap, #587).  Contract 1 is declared only *after* everything is converted
> and stable — gated on real-program validation, not the mechanical minimum (COMPATIBILITY.md § *The road
> to contract 1*) — so it ships with `len`=chars already baked in.  There is therefore **no frozen
> program that expects byte-`len`**, and `len/size` need **no `_v0` variant**.  Consequence: `len/size`
> is *not* a customer of this mechanism, only its worked example; the mechanism's first real customer is
> whatever first *post-1* change turns out genuinely unavoidable.  This is *why* the plan is designed
> proactively rather than driven by a live trigger.

The stdlib itself contains text-length logic, and the stdlib is compiled at *some* contract.

> ### ❓ Q3 (NEEDS ANSWER) — What contract is the stdlib compiled at, and may stdlib internals use the contract-keyed `len`/`size` at all?
> If the stdlib is "latest," its internal `len` uses new semantics for everyone (including a contract-0
> user's call into it) — correct, because it's the stdlib's *own* code.  But it means the stdlib's
> correctness now silently depends on the contract it was built at.
> - **Option 1: stdlib pinned to latest; internals may use `len`/`size`.**  Simple; but a future flip
>   would force auditing every stdlib text site again.
> - **Option 2 (recommended): stdlib internals never call the contract-keyed `len`/`size` — they use
>   explicit, contract-stable primitives** (see Q4).  The keyed ops become a *pure surface convenience*;
>   the stdlib is contract-independent and immune to future flips.
> **Why it blocks:** Option 2 requires Q4's primitives to exist and a one-time stdlib refactor; Option 1
> doesn't.  Directly gates arc scope.

> ### ❓ Q4 (NEEDS ANSWER) — Do we ship contract-stable explicit primitives (`char_count` / `byte_size`) and steer toward them?
> The additive-rename half of the compatibility toolkit.  If yes: new code and the stdlib (Q3) can be
> unambiguous and contract-independent, and arc C's steer points at these instead of at "declare a new
> contract."  If no: `len`/`size` are the only spellings and *everything* text-length is contract-keyed
> forever.
> - **Recommended: yes — ship `char_count(text)` / `byte_size(text)` (or the agreed names) as
>   contract-stable, and make them the steer target and the stdlib's internal spelling.**  Contract
>   keying then covers only *legacy* `len`/`size` call sites, which is the minimal permanent surface.
> **Why it blocks:** changes the public API surface (Goal A/F review), the stdlib refactor (Q3), and the
> steer target (arc C).

---

## Arc C — Author steering, keyed to the contract bump

Reuses the @PLN102 arc-C owned-source gate (`steer_enabled()`, the call chokepoint) but keys the alert
to a `contract` bump, not a bare `#superseded`.  The design tension: **contract 0 is a legitimate,
supported-forever choice** — nagging every contract-0 `len(text)` call violates "old works, no nag."

> ### ❓ Q6 (NEEDS ANSWER) — Is the steer silent, warn-once, or lint-shape-only for owned contract-0 source?
> - **Option A: silent.**  Contract 0 is legitimate; the existing @PLN110 default-on *strict-index*
>   lint already catches the real bug shape (a char-`len` driving a byte index).  Purest "no nag."
> - **Option B: warn-once per compile** — "you're on contract 0; `len(text)`=bytes here.  Declare
>   `contract <N>` for char-count, or use `char_count`/`byte_size` (Q4)."  Informative but noisy for a
>   deliberately-legacy project.
> - **Option C (recommended): lint-shape-only** — stay silent on bare `len`/`size`, keep steering only
>   the demonstrated-bug shapes the strict-index lint already flags.  "Old works, no nag" + real bugs
>   still caught.
> **Why it blocks:** sets the default developer experience of the whole feature; over-warning is itself
> a soft compatibility regression (COMPATIBILITY.md § *Deprecation is soft steering, never
> warn-then-remove*).

---

## Arc D — Resolver contract-matching for libs

The binary implements semantics for **all** contracts `[0, CONTRACT_VERSION]` ("carry both").
`CONTRACT_VERSION` is the MAX supported; `check_contract(required, current=MAX)` already returns
`TooOld` when a lib needs more than the binary offers.  Arc D makes `loft install` **iterate versions**:
pick the newest lib version whose declared `contract` predicate is satisfiable at the binary's MAX, so
an old binary transparently stays on the last compatible line instead of pulling a too-new lib or
failing hard.  This is the arc that deletes "forced to make versions."

> ### ❓ Q8 (NEEDS ANSWER) — Confirm: the contract floor never rises (contract-0 semantics carried forever)?
> "Carry both" is permanent: dropping the contract-0 variant needs *provable* zero usage, and private
> programs are invisible (the registry scan proves only public usage → zero is unprovable), so the
> honest answer is **never remove** — the binary keeps every historical text-len variant for all time.
> Bounded (only genuinely-diverged ops), but permanent.  Needs an explicit "yes, that's the accepted
> cost" so the permanent-surface budget is a decision, not a drift.
>
> Sub-item (confirm, likely not a fork): does today's resolver already iterate versions on a failed
> `check_contract`, or only consider `@latest`?  If only `@latest`, the version-fallback loop is net-new
> arc-D work.  (`PKG_REGISTRY.md`: `loft install <pkg>` picks the highest non-prerelease — the
> contract-skip fallback must be added.)

---

## Decisions needed before implementation (summary)

| # | Decision | Recommended | Blocks |
|---|---|---|---|
| **Q1** | Semantic anchor = resolver predicate's lower bound, or a separate field? | lower bound of existing `contract` | Arc A shape; a manifest/store field |
| **Q2** | Undeclared / bare-script default contract | undeclared=0; `loft new` scaffolds latest; bare=0 | The default a million programs inherit |
| **Q3** | Stdlib's own contract; may internals use keyed `len`/`size`? | internals use stable primitives (Q4), not keyed ops | Arc B scope + stdlib refactor |
| **Q4** | Ship stable `char_count`/`byte_size` primitives + steer to them? | yes | API surface; Q3; arc-C target |
| **Q5** | ✅ RESOLVED — is the flip contract 1, or later? | **flip lands pre-contract-1** (free break, #587); no `_v0`; len/size is the example, not a customer | (resolved) |
| **Q6** | Steer: silent / warn-once / lint-shape-only? | lint-shape-only (reuse strict-index lint) | Default developer experience |
| **Q7** | Store stride bump with "absent ⇒ contract 0"? | yes, ack'd against @PLN97 | Store format; @PLN97 gate |
| **Q8** | Contract floor never rises (carry every variant forever)? | yes — accepted permanent cost | Permanent-surface budget |
| **Q9** | Granularity: package-only, or per-file pragma? | package-only for v1 | Whether to close the door deliberately |

**Critical path:** Q5 is resolved — the flip is pre-1 and needs no keying, so the mechanism has **no
urgent customer**; that removes the schedule pressure but not the design.  Q1+Q2 still gate arc A
whenever it is built; note **arcs A + D are the parts contract 1 needs regardless** (a binary must
advertise its contract and the resolver must refuse a too-new lib), while arc B (dual-variant op
selection) waits for a real post-1 change.  Q3+Q4 travel together (stdlib + primitives); Q6 is arc C
only; Q7 a store ack; Q8 a budget ack; Q9 a scope door.

## See also

- [README.md](README.md) — status, sub-arc table, Stage-A matrix.
- [COMPATIBILITY.md § The escape valve for the genuinely unavoidable](../../COMPATIBILITY.md) — the
  policy; [§ Open decisions](../../COMPATIBILITY.md) item 3 links here.
- [plans/110-len-size-semantics/phase0-inventory.md](../110-len-size-semantics/phase0-inventory.md) —
  the pre/post-flip semantics table this design must reproduce.
- Resolutions land in [DESIGN_DECISIONS.md](../../DESIGN_DECISIONS.md) as each `Qn` is answered.
