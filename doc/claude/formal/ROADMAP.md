<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# formal/ROADMAP.md — the path to a spec-conformant implementation

The single ordered view of every **open deviation** across `formal/*` — the gaps between
the rules each area states and what the code does today — sequenced into the order to
resolve them. Detail stays in each area doc (this is order/size/direction only, like
[STABILITY_ROADMAP.md](../STABILITY_ROADMAP.md) for stability).

## The principle

> **The code changes to match the rules.** That is the default for every row below.
> But the spec is a *hypothesis*, not scripture: where a rule turns out to be wrong or
> missing a real detail, the **rule** is what changes (flagged **spec-may-adjust**). Most
> rows are code→spec; a few are decisions where the spec itself is on the table.

Closing a row means the implementation obeys the rule (then the deviation entry is deleted),
**or** the rule is corrected and the row becomes a decided edge (moved to
[INCONSISTENCIES.md](../INCONSISTENCIES.md) / [DESIGN_DECISIONS.md](../DESIGN_DECISIONS.md)).

## Distance today

| area | open | what's left |
|---|---|---|
| [types.md](types.md) | 0 | **@PLN25 value/null model landed (2026-07-02): DN1–DN6 CLOSED**, D2 closed (C83); DN3 covers integer `/`/`%`, indexing, text-parse; overflow-arith a decided edge (C85). **@PLN102 null-flow generalisation — SHIPPED default-on 2026-07-11 (#559):** the general § Null-flow laws across EVERY type — (N-Domain) fit-failing→`τ?`, (N-Prop) null propagates through arithmetic, (N-Cast) a cast/parse asserts (parse folds in, no auto-`τ?`), (N-Store) warns except narrow widths (which error), (N-Store) also at the call-argument site (#583, 2026-07-16) — plus the float instance DN3-Float, all CLOSED + verified both backends.  **D-Null-Join OPENED AND CLOSED (2026-08-26, loft#1103)** — that verification covered the DIRECT store; at a branch JOIN a nullable in a LATER arm reached a non-null slot silently, narrow widths included, and the join is now nullable when ANY arm is, whichever arm and however it is spelled |
| [binding.md](binding.md) | 0 | @PLN40 two-level `const` model shipped (Const-Bind/Value/ScalarCollapse/Compose); **D-const-1 CLOSED** via @PLN102 K1 (enum-variant `const` now enforced identically to struct fields). `&`-ladder: D-bind-7 (bare `&a;`) landed; **D-bind-8 CLOSED** by adding B-Ref-Reshape (@PLN130 F9, loft#779). **D-bind-9 CLOSED same day** — the refusal now covers all three of `B-Disturb`'s events; a `&` across a re-key or a container reassignment is refused rather than silently copied |
| [grammar.md](grammar.md) | 0 | ✓ closed — D-gram-1/3 landed; D-gram-2 (non-CFG) + D-gram-4 (`&` overload) resolved as decided edges → DESIGN_DECISIONS C81/C82 |
| [operational.md](operational.md) + family | 2 | **D-op-1/2** the differential oracle (@PLN89, the META deviation — differential-not-definitional conformance; D-op-4 the spreadsheet runtime is CLOSED). The operational rules are now written across sibling files: heap / iteration / coroutines / concurrency / calls / matching / tuples / closures (2026-07-04) + **formatting.md** (`"{x}"` interpolation + value→text) and **interfaces.md** (interfaces + generics — monomorphization, structural satisfaction) (2026-07-05) — **all 0 own**. Closures' D-clo-1 (the `\|…\|` and `fn(){}` forms now capture IDENTICALLY, pure sugar) AND D-clo-2 (a stored un-inferrable short lambda in `map` now emits a clean "cannot infer" diagnostic instead of panicking) both CLOSED 2026-07-04, verified on both backends. With formatting + interfaces written, the operational contract now spans the whole family — nothing is left unwritten except the D-op-1 meta-gap itself |
| [ownership.md](ownership.md) | 0 | the `deps` borrow checker — **✓ CLOSED (2026-07-04)**: all five D-own deviations resolved. D-own-3 (typed `Deps`) CLOSED; D-own-4 → decided edge **C86**; D-own-5 (`&` rides `deps`) CLOSED; **D-own-2 (completeness) CLOSED** — the fact is total (oracle over every value + the `_own_store` runtime-Join witness, @PLN90 loft#495); **D-own-1 (O-Deps) CLOSED** — an audit + the `0234cbbb` unification landed the last shipped shape-scan (interp adopt-vs-deep-copy) onto `return_adopts_fresh_store()`, so every store-lifetime decision reads the ONE fact on the shipped path. Floor (non-deviation cleanup): the `LOFT_NO_JOIN_OWN` opt-out scans + one physical return-funnel. Validated: suite 2601/2601, native_scripts, poison, fuzz-gate controls, differential oracle, fuzzer |
| [capabilities.md](capabilities.md) | 0 | the `deps` borrow checker's sibling — **✓ CLOSED (2026-07-04)**: sandbox admission enforces all six rules, each with a RED/GREEN pair. **D-cap-1** the parameter `#default` lock (`param_lock_violations`); **D-cap-2** the closure descent (`mark_lambda_sandboxed` — a script-only lambda is usable, a host-reaching one is rejected naming the reach); **D-cap-3** the owned-vs-host write split (`raw_write_is_host_owned` gained a `Type::Vector` owned arm — a probe proved a local vector never aliases host, every whole-value bind incl. `&` COPIES, so only a PARAMETER-root write is a host effect and the `arguments()` check already IS that boundary; the feared `ownership_of` consultation was NOT needed) |
| [layout.md](layout.md) | 1 | **D-layout-1** — no version guard on persisted bytes (#477: same types, different bytes, silently misread; `L-Sound`). **Mechanism shipped (@PLN97):** the golden byte-layout test catches a change at commit; the `.dschema` sidecar (`CorruptReason::SchemaMismatch`) detects a stale store at load → the `on_corruption` rebuild. **Residual:** the durable store ([plans/43](../plans/43-loft-store-durable/)) isn't loft-driven yet, so nothing auto-invokes the load-time gate — closes when a persistence consumer wires `check_beside` into its open path |

Binding + grammar closed; **types has one spec-first open item** — the @PLN25 value/null model
landed (2026-07-02) with DN3 covering integer `/`/`%`, indexing, and the text→numeric parse flip
(`(N-Parse)` types `τ?`), overflow-arith reclassified as a decided edge (C85, not a deviation);
the **@PLN102 DN3-Float extension** (float `/`/`%` + domain-partial fns type `τ?`) is written
ahead of the code (2026-07-11). **Ownership is now CLOSED (2026-07-04):** typed
`Deps` (D-own-3), C86 (D-own-4), the `&`-borrow fact (D-own-5), the TOTAL fact — oracle over
every value + the `_own_store` runtime-Join witness (D-own-2, @PLN90 loft#495), and finally
**D-own-1 (O-Deps)** — an audit + the `0234cbbb` unification put every shipped store-lifetime
decision on the ONE `deps` fact (the last inline shape-scan, interp adopt-vs-deep-copy, now
reads `return_adopts_fresh_store()` like native). **Capabilities is now CLOSED too (2026-07-04 —
D-cap-1/2/3 all resolved).** The one **new** static area is **layout.md** (2026-07-07, the store
byte-layout contract) at **1 open** — `D-layout-1`, the #477 version-guard gap, mechanism-shipped
(@PLN97) and awaiting a durable-store consumer to auto-invoke it. The remaining open formal
deviations are therefore: the operational **D1** (differential oracle, @PLN89) — an inherently
open-ended coverage instrument, not a one-shot close — and layout's **D-layout-1** (the persistence
consumer wiring). The @PLN89 differential oracle + LOFT_POISON grow alongside as the safety net.

**Spec-first entry — @PLN35 (PEG match patterns).** Unusually, its formal rules are written *ahead*
of the code ([matching.md § Rules — PEG patterns](matching.md), [types.md § Pattern captures](types.md),
[grammar.md § Pattern-operator precedence](grammar.md), [binding.md § Pattern captures](binding.md);
design [plans/35-match-peg/FORMAL-DESIGN.md](../plans/35-match-peg/FORMAL-DESIGN.md)). This opens
**no deviation** — there is no implementation yet to break a rule — so it is not a row above; the
build obligation is tracked in [VERIFICATION.md § matching.md — PEG patterns](VERIFICATION.md) and
each rule graduates to a ✓ there as its phase lands. It is the last planned *syntax* feature; the
rules exist so the implementation is built to satisfy them (the maker's spec-first directive).

---

## Phase A — turnkey (days, no new design)

Cheap, well-scoped, no plan needed — clear the easy distance first.

| # | deviation | change | direction |
|---|---|---|---|
| ~~A1~~ **DONE** | ~~**D-bind-7**~~ | extended the prefix-`&` guard to the bare-statement position `&a;` (and block-final `{ &a }`, the same leak) at the `parse_assign` chokepoint → binding.md is **0**. `pln87_d_bind_7_*` in `tests/parse_errors.rs`. | code→spec, landed |
| ~~A2~~ **DONE** | ~~**D-gram-1**~~ | lifted the 12-level precedence ladder + associativity into [LOFT.md § Operators](../LOFT.md#operators) (fixed the stale table: `**`@10, `as`@11, assoc statement, unary-tightest note) and enumerated `binary_op` in § Summary of grammar | code→spec (doc), landed |
| ~~A3~~ **RESOLVED (subsumed)** | ~~**D-op-3**~~ | **obsolete — do NOT thread the flag.** The C80 decision (this cycle) eliminated trap-suppression as a concept: every uncomputable op yields null+continue in ALL modes, so the trapping variant *and* the `??`-position rewrite (`rewrite_outer_arith_to_nullable`, the per-site flag) both disappear. operational.md folded D-op-3 into **D-op-4** (`E-Coalesce`: "closes the old D-op-3"). The remaining work is D-op-4 (below), not a Phase-A consolidation. | spec-adjusted → D-op-4 |

## Phase B — three decisions (a sentence each; may change the spec, not the code)

These are **spec-may-adjust** — your call resolves them, then they close or reclassify.

| # | deviation | the decision | likely outcome |
|---|---|---|---|
| ~~B1~~ | ~~**D-gram-3**~~ **DONE** | `**` is now **right**-associative (`2**3**2 == 512`) — the maker-centric call (don't carry a surprise). | code→spec, landed; `tests/issues.rs::power_is_right_associative` |
| ~~B2~~ | ~~**D-gram-2**~~ **DONE** | loft's surface IS deliberately not context-free — accepted on purpose. | reclassified → decided edge, [DESIGN_DECISIONS C82](../DESIGN_DECISIONS.md#c82--lofts-surface-is-deliberately-not-context-free) |
| ~~B3~~ | ~~**D-gram-4**~~ **DONE** | A1 made prefix `&` total — keep one `&` token, disambiguated by position (like Rust). | reclassified → decided edge, [DESIGN_DECISIONS C81](../DESIGN_DECISIONS.md#c81---stays-one-token-disambiguated-by-position-bitwise-and-vs-reference) |

## Phase C — tracked projects (have or need a plan; weeks)

The real weight. Each is a `loft-lang/plans` issue, sequenced.

| # | deviation(s) | project | plan | size |
|---|---|---|---|---|
| C1 | **D2** | integer model i64 end-to-end. **AUDITED** ([plans/88-integer-i64.md](../plans/88-integer-i64.md)) — reframed: do NOT widen `Value::Int` (the runtime is already i64; `Int(i32)`/`Long(i64)` is a compact value-size encoding). The change is `IntegerSpec` bounds → i64 + template unify + an `int_const(i64)` keystone (compact `Int` if it fits, else `Long`). | **[@PLN88](https://github.com/loft-lang/plans/issues/88)** | M–L |
| ~~C2~~ **DONE** | ~~**D-own-3**~~ | typed `Deps` landed (H2 steps 1–5, 2026-06-12: newtype + named constructors + space-checked queries + the `CALLEE_FRAME_BIT` value tag) — recounted into ownership.md 2026-07-03 | H2 ([DEPS_INVENTORY.md](../DEPS_INVENTORY.md)) | M, landed |
| C3 | **D-own-1, D-own-2, D-own-5** | the `deps` borrow checker: ownership computed once per binding/path; free/copy/move derive from one `deps` fact; `&`-borrow source tracked in `deps`; the bind-site copy/alias/elide decision reads `ownership_of` + last-use (the C86 residual — #415's copy is the SEMANTIC, D-own-4 reclassified) | **[@PLN85](https://github.com/loft-lang/plans/issues/85)** | L (the north star) |
| ~~C4~~ **DONE** | ~~**D-cap-1/2/3**~~ **ALL CLOSED** | capability admission fully landed: field read/update/append (F3–F6), the `…#default` parameter lock (D-cap-1), the closure descent (D-cap-2 — `mark_lambda_sandboxed`), AND the owned-vs-host write split (D-cap-3 — `raw_write_is_host_owned` `Type::Vector` owned arm; a probe proved a local vector never aliases host so only a parameter-root write is a host effect, no `ownership_of` needed). Each with a RED/GREEN adversarial pair. | **[@PLN86](https://github.com/loft-lang/plans/issues/86)** | S–M, landed |

## Phase D — the operational arc (two projects: oracle + spreadsheet runtime)

| # | deviation(s) | project | direction |
|---|---|---|---|
| D1 | **D-op-1, D-op-2** | **DECIDED (2026-06): a differential oracle** — run a growing corpus on BOTH backends and assert they agree (value / null / halt / stdout / stderr / leak — the stderr channel added 2026-08-21, loft#1056); the operational.md rules guide coverage. Turns the interp/native divergence class (D4/#433) from a coverage lottery into a caught failure. Switchable later to an executable shared semantics; the rules reuse either way. **Scope addition (2026-07-02, routing feedback): DRIVER agreement — well-typedness is ONE static judgment, so accept/reject must agree across `--interpret` / `--dump` / `--native` / `--native-wasm` for every corpus program** (the reported `--dump` divergence traced to mixed binaries, but the property needs a guard, not an assumption; a first-pass abort already makes the *diagnostic set* phase-dependent). Tracked: **[@PLN89](https://github.com/loft-lang/plans/issues/89)**. | code→spec (the chosen model) |
| ~~D2~~ **DONE** | ~~**D-op-4**~~ | **BUILT the spreadsheet runtime** (formalize4). Div/mod-by-zero and integer overflow now yield the null sentinel and CONTINUE on both backends (`raise_recoverable` + `checked_long!`→`i64::MIN`); OOB already complied; `NullDereference` was never raised. Two refinements vs the original plan: an UNGUARDED div0 reports a Warn log (`E-Report`), overflow is silent (rustc-release default). The `??` trap-suppression mode is gone behaviourally (the `*Nullable` op split is now dead code — separable cleanup). Guard: `tests/scripts/184-i333-div-zero-null-continues.loft`. | code→spec (C80), landed |

---

## Resolving order, in one line

**~~A1~~ ~~A2~~ ~~A3~~** clears Phase A **·** **~~B1~~ ~~B2~~ ~~B3~~** all decided (D-gram-2/4 →
decided edges C82/C81; grammar.md at 0) **·** binding.md + grammar.md + **types.md (@PLN25 CLOSED —
DN1–DN6, D2 reconciled; DN3 parse-flip landed)** + **~~D2~~ (D-op-4 spreadsheet runtime,
formalize4)** now **closed** **·** NEXT: the tracked arcs
**~~C2~~ (typed Deps) → ~~C3~~ (@PLN85/@PLN90: ownership — all five D-own CLOSED 2026-07-04)** ·
**~~C4~~ (capabilities, @PLN86 — D-cap-1/2/3 all CLOSED 2026-07-04)** · every static area now at 0 ·
NEXT: only the operational **D1** (differential oracle, @PLN89) remains — an open-ended coverage
instrument, not a one-shot close.

## What is NOT on this list (already clean or decided)

- types.md D1/D3/D4/D5, binding.md D-bind-0..6 + doc — **closed in code this cycle** (with tests).
- A row that turns out **spec-may-adjust** leaves `formal/` and becomes a decided edge — it is
  *resolved*, not deleted-by-fix. The deviation count is "distance from the current spec," and
  the current spec is allowed to be wrong.
