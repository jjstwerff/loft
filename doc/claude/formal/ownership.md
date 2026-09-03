<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# formal/ownership.md — the `deps` ownership / borrow system (strict; register at `OPEN: 0`)

**Catalogue:** @F21 (references `&T`), @I60 (deps / lifetime tracker) — Goal E. Roadmap: @PLN85, @PLN87.

> **Rules then deviations** (see [README](README.md)). The rules below are loft's
> ownership model, and as of 2026-07-04 the deviation register is at **`OPEN: 0`** — all
> five `D-own-*` deviations are CLOSED on the **shipped path** (@PLN85 store-lifetime,
> @PLN87 the `&` law both landed), validated by the @PLN89 differential oracle + the
> `program_ownership` fuzzer. This is *validation, not a machine-checked proof*: the
> `Join` fact still resolves through a runtime witness, and the pre-fact shape-scans
> survive under opt-out as differential-control machinery. The residual is not a
> correctness deviation but the *substrate* — the fact is computed flow-INSENSITIVELY.
> [@PLN94](../plans/94-cfg-ownership-dataflow/) has now built the flow-SENSITIVE
> replacement as an independent oracle that runs BESIDE the shipped analysis (a machine
> check on every `cargo test`, `tests/ownership_oracle.rs`); its abstract-interpretation
> soundness — the over-free half — is now PROVED given the rules here (the local-transfer
> lemma discharged case by case). See §"Machine-checkable soundness" below; only a Coq/Lean
> rendering of the prose remains.
>
> The rules are loft's borrow checker. **Rust is the reference model.** Beacon + rationale:
> [OWNERSHIP_MODEL.md](../OWNERSHIP_MODEL.md); the typed-`deps` design:
> [DEPS_INVENTORY.md](../DEPS_INVENTORY.md). This doc is the **checker** (lifetimes /
> free placement); the **surface** (`&τ`, reference-default) is [binding.md](binding.md).

## Notation

- **owner** — the binding or slot responsible for freeing a heap store. Exactly one at a
  time.
- **borrow / alias** — a value that *refers to* a store it does not own (a parameter, a
  field/element read, a `&τ` link). It must not free, and must not outlive its source.
- **`deps`** — the per-binding list recording what a binding borrows from. (Today a
  `Vec<u16>`; see D-own-3.) An EMPTY list is read as "owner" at many sites — that reading
  is `O-Proxy`, and it is a stand-in for the oracle rather than the oracle itself.
- **the oracle** — `use_analysis::ownership_of`, which computes own-vs-borrow for a VALUE
  from the IR. `O-Oracle`. It does not consult `deps`.
- **the never-free override** — a per-binding flag (`is_skip_free`) that forbids emitting a
  free for that binding at all. `O-Override`.
- **the latest-assignment memo** — `Scopes::owned_refs`, the oracle's answer for a binding's
  most recent assignment, tagged with the loop depth at which it was taken. `O-Latest`.
- **transfer / move** — handing ownership to another binding (e.g. a return). The giver
  stops owning; it must not free what it moved.

---

## Rules

> The model is **sound** (no use-after-free, no double-free, no leak) **and complete**
> (computed for *every* binding, every path). The five invariants:

```
  (O-Owner)     SINGLE OWNER.  Every heap store has exactly one owner at any moment.
  (O-Move)      MOVE ON RETURN.  A returned heap value's ownership transfers to the
                caller's binding; the callee never frees what it transfers.  If the return
                *borrows* a parameter, the return type records it (`{Attr(param)}`) and the
                caller COPIES to obtain its own store.
  (O-Borrow)    BORROW TRACKING.  A value aliasing another (param / field / element / `&τ`)
                carries the source in its `deps`; the borrower is skip-free; the single
                owner frees once.
  (O-Derived)   FREE PLACEMENT IS DERIVED, NOT DECIDED.  Free a local iff it owns its store
                and does not transfer it out — once, at scope exit.  No per-site heuristic.
  (O-Complete)  PER BINDING, PER PATH, COMPLETE.  Every binding, including every `match`/`if`
                arm — a set-and-reconcile, not a single-variable structural walk.
```

**In words.** One thing owns each piece of heap, and it's the only thing that frees it.
When you return a heap value you *give it away* (the function stops owning it); if you
only return a view into an argument, the type says so and the caller makes its own copy.
Anything that just borrows is tracked but never frees. Crucially, *where* to free is
**computed** from these facts, not guessed per code-site — and it's computed for **every**
binding on **every** branch, not just the easy ones.

**`O-Detach` is about ORDER, and it is the one rule here that a correct ownership FACT cannot
save you from.** Every other rule answers *who owns this store*; this one answers *when may the
lowering act on that answer*. A binding whose ownership is computed perfectly is still read as
`null` if the sentinel that detaches it is emitted before the expression that reads it — which is
what `p = mk(p.a + 1)` did on a heap parameter, on both backends, with nothing reported
(loft#1312). The same order appears three more times: the `--native` adopt-vs-copy guard cleared
the destination while the source still named that store; the reassignment path in `codegen.rs`
avoids it by asking `Value::reads_var` and deferring the free; and the @PLN87 P2.1 literal
lowering avoids it by hoisting the field reads into temporaries. Declining the detach is NOT a
third option — that is what D-own-16's open half does, and it trades a wrong answer for a
retained store rather than resolving the order.

**This is an INTERNAL system — it never rejects a program it can compile.** loft has no
user-facing borrow checker; the user writes naively and the compiler always finds a valid
lowering, copying when it cannot prove an alias is safe
([OWNERSHIP_MODEL.md § Internal and invisible](../OWNERSHIP_MODEL.md)).
That makes **`O-Complete` the load-bearing invariant**: an incomplete fact is not a compile
error the user fixes — it is a miscompile or a leak. So the failure mode to fear here is
*incompleteness* (D-own-2), not just unsoundness — the analysis must be **total**.

The single carve-out is not in this doc's rules at all, and stays that way: where NO correct
lowering exists — an explicit `&` reference whose place the program then destroys — the
*binding surface* declines the program ([binding.md](binding.md) B-Ref-Reshape, C79 revisited
2026-08-05). That is a decision about what `&` MEANS, not a lifetime the checker failed to
prove, so it changes nothing here: these rules still never reject, and the deps fact is still
computed for every binding on every path. A program this doc's rules would have handled fine is
never refused.

### The mechanism — one fact, derived everywhere

```
  (O-Deps)      every store-lifetime codegen decision — free placement, adopt-vs-copy,
                move-vs-clone, drop — DERIVES MECHANICALLY from the single `deps` fact.
                If a decision is re-derived by a codegen condition, that is the bug.
  (O-NoDiverge) because both backends translate the SAME `deps` facts, the interpreter and
                `--native` cannot diverge.  (This is the soundness side of
                [operational.md](operational.md)'s shared contract: O-NoDiverge is *why*
                E-Op/E-Trap agree across backends.)
```

**In words.** Every "do I free / copy / move this store?" question is *answered by reading
a carried fact*, never re-worked out in the code generator. And because both backends read
the same answer, they can't disagree — which is exactly what makes the operational rules
hold on native as well as interp.

### The facts that answer it — there are four, and `deps` is not the oracle

`O-Deps` above is written as though `deps` were the single source of truth. It is not, and
the gap between that sentence and the implementation is where this subsystem's bugs live
(loft#723 is the worked example). Four distinct carried facts answer ownership questions,
and a decision that reads the wrong one is wrong in the silent direction:

```
  (O-Oracle)    the own-vs-borrow answer for a VALUE is computed by ONE oracle
                (`use_analysis::ownership_of`), from the IR: a store mint is Owned, a
                projection is Borrowed(base), a call resolves through the callee's return
                summary.  It does NOT read `deps` — the two are INDEPENDENT derivations of
                the same question.  A chokepoint reads the oracle rather than re-deriving.
  (O-Proxy)     an EMPTY `deps` list is a cheap PROXY for "this binding owns its store",
                and it is UNSOUND ALONE: a borrow whose dep list was never populated also
                reads empty, so the proxy answers "owner" for a borrower.  A site that
                FREES on the proxy MUST also consult O-Override — otherwise it frees a
                store someone else owns.
  (O-Override)  a binding may carry an explicit never-free flag
                (`variables::Function::is_skip_free`).  Its contract is exactly "no
                `OpFreeRef` is ever emitted for this binding", and it VETOES the proxy and
                the scope-exit sweep alike.  It exists BECAUSE O-Proxy is unsound alone.
  (O-Latest)    ownership is a property of the LATEST assignment to a binding, at the LOOP
                DEPTH at which that assignment was taken (`Scopes::owned_refs`, a memo of
                O-Oracle plus that depth).  A type-level `deps` list can express neither,
                so the ownership-TRANSITION free — freeing the store a binding is about to
                stop pointing at — reads THIS and not `deps`.
  (O-Detach)    DETACH AFTER THE READS.  A binding's DETACH — the free, sentinel or
                re-allocation that stops it naming its current store — is sequenced AFTER
                every read of that binding by the value being assigned to it.  A lowering
                that must detach early hoists those reads into temporaries first; one that
                cannot hoist defers the detach past the assignment.  Detaching before the
                reads is never admissible, and DECLINING the detach to avoid the hazard
                trades the wrong answer for a retained store rather than resolving it.
```

**In words.** There is a real oracle, and it is not the dep list. `deps` is a cheap
stand-in for it that is right most of the time and wrong for a borrow nobody recorded a dep
for; `is_skip_free` is the patch that makes the stand-in safe at a free site; and
`owned_refs` carries the two things a type cannot — *which* assignment, and *how deep in
loops* it happened.

⚠ **`(O-Oracle)`'s interprocedural half has a failure mode of its own: it can lose the
callee's answer on the way back to the caller.** The summary is stated in the CALLEE's
parameter space, and delivering it means naming the caller value the returned store may lie
in. Where that translation gives up, the honest answer is *"a borrow whose base I cannot
name"* — but a `CallRef` answered `Owned`, the one verdict that licenses a free, so the
caller released a container it still held and the next read came back `null` with nothing
said (loft#1318). Three shapes reached it: an argument that is itself a CALL (the structural
walk stops at one, on the ground that the caller lifts it into a temp first — which a
BORROW-returning callee is not), a keyed lookup (`m[k].v`, missing from the oracle's own
projection set, so a view read as a mint), and a hidden `__retbuf` (refused as "not a
parameter the author wrote", though the caller allocates that buffer and passes it).

**The rule that decides all three is one sentence: a translation that cannot name a base must
not upgrade the verdict.** Naming the base is always preferable to declining — the base is
what `(O-Oracle)`'s run-time test compares against, so a named one keeps the mint arm's free
as well — but between an unnameable base and `Owned` there is no trade: `Owned` is the
over-free direction, and a leak is recoverable where a premature free is not.

⚠ **The reason to write this down is that the choice is currently invisible.** 38 functions
test `depend().is_empty()`; some legitimately want the proxy (they are asking "is this a
view?", not "may I free it?"), some memo the oracle, and some free. Nothing in the source
distinguishes them, so a reader cannot tell a site that correctly reads one fact from a site
that reached for the wrong one — and both compile. `O-Proxy`'s "MUST also consult
O-Override" is the first checkable obligation in that space.

**Seventeen of the 38 decide a free, and six of those consult the override.** Measured in
the 2026-09 bug review ([BUG_REVIEW.md](../BUG_REVIEW.md)), which converted the largest
group: `Scopes::owns_freeable_store` is now the one home for *"may this function free the
store this source names?"*, discharging both obligations together — the `is_skip_free` veto
and the carve-out that a user PARAMETER belongs to the caller while the promoted NRVO buffer
is the one argument that is really a local. It replaced three copies inside `free_vars`,
extended separately by loft#688, loft#1022 and loft#1078; the keyed copy never gained the
promoted-buffer half at all.

⚠ **And the obligation was holding by ACCIDENT, not by construction.** Before the fold, none
of those three consulted `is_skip_free` — and the corpus never noticed, because no
`skip_free` binding currently reaches them (measured over `tests/scripts` and `tests/docs`,
zero hits). A site that frees on the proxy and happens never to meet a marked binding is
indistinguishable from one that asks correctly, which is the same invisibility this section
is about, one level down.

⚠ **And the "seventeen decide a free" above is a HAND count the gate could not reproduce.**
Re-measured 2026-09-03, after `scripts/o_proxy_check.py` was given a decidable predicate for
*"does this site reach a free?"*: of 24 positive sites **6 reach one, and all 6 discharge the
veto**. Two of those six were undischarged until that run — `scan_set`'s displaced-owned dep
strip and `gen_set_first_ref_var_copy`'s move — and both now consult it.

**The seventeen the gate cannot decide lexically now DECLARE what they ask**, which is the
cure the ⚠ above names — the choice is written at the site instead of inferred from it. Each
proxy site carries one of four verdicts, and the census is countable:

| declares | sites | what the empty dep list decides there |
|---|---|---|
| `free`   | 9 | ownership, and a free follows — **@FR-O-Override is required with it** |
| `copy`   | 8 | copy-vs-alias / materialise-vs-view; a wrong answer costs a copy, never a release |
| `alloc`  | 4 | whether to ALLOCATE or null-init a store — the opposite direction from a free |
| `oracle` | 3 | an independent derivation that drives no emission (@PLN94, witness accounting) |

A declaration is a claim, so the gate contradicts it where it can: a site declaring anything
but `free` while a free IS visible in the region it gates is reported rather than trusted.
What it cannot do is catch a site that declares `copy` and frees somewhere the region cannot
see — that residual risk is real, and it is a much smaller one than *"nothing in the source
distinguishes them, and both compile."*

⚠ **The pass corrected one of its own conclusions, which is why it is worth writing down.**
`parse_field_iteration` looked like a `free` site and its own prose said so — *"a
borrow/skip_free binding owns no allocation"* — so it was first declared `free` and given the
veto. The differential probe then reported **8 of 1119 corpus files** reaching it with a
`skip_free` binding, i.e. a live behaviour change rather than a latent guard. Reading the
mechanism settled it: the frees that follow are of FRESH per-field bindings
(`copy_variable` + `remap_var_deep`), never of the binding tested — so the veto does not
belong there, and the site is `copy`. **A site's own comment is not a measurement**, and the
prose there still overstates its filter.

⚠ This does **not** re-open `D-own-1` (CLOSED: *"every free/copy/move reads `deps`"*). That
remains true in the letter — these sites do read `deps`. What was never true is the
implication that reading `deps` is *sufficient*.

---

## Deviations

**OPEN: 1.**
- **D-own-8** — a Join's ownership fact is true on one path only.  Its Face A's SYMPTOM —
  a binding joined from TWO arms carries both arms' deps, reads as a borrow, and the MINTING
  arm's store is owned by nobody — is CLOSED 2026-09-03 (loft#1320): each qualifying arm tail
  is given its own binding, so `(O-Complete)`'s *per binding, per path* holds structurally
  for that shape.  ⚠ Not the dep COLLAPSE — that was fixed 2026-08-26 (`depend_on_all`, six
  sites and three siblings, asserted on the predicate in `variables/mod.rs`), and a line here
  said otherwise for an afternoon.  What stays open is narrower: the fact for a value branch
  whose arm MINTS A LITERAL beside an arm that borrows (loft#1098 closed its symptom, not the
  fact — the arm rewrite lifts a CALL tail, not a literal), a named local bound at two sites
  from two different bases and a base reassigned in the function are DECLINED rather than
  answered, and a NAMED call's record arm still accumulates records in its `__ref_N` buffer
  (loft#1323).  loft#1321 (the join binding's COPY face) is the same binding from the other
  side; whichever lands first narrows the other.
  [ownership-history.md](ownership-history.md) has the matrix

**D-own-26 CLOSED 2026-09-03**, against the bar its own entry set: *"the honest cure is a way
to fail a build in which a free-deciding site reads the proxy without the veto."* That gate
now exists, is falsified on five separate paths, and passes — 9 sites declare `free` and all
9 consult `O-Override`; the other 15 declare which of the other three facts they read. The
"eleven of seventeen" it opened with was a hand count that could not separate *asking* the
proxy from *freeing* on it. What the close does NOT cover: a site that declares a non-free
question and frees somewhere the gate cannot see. The full record is in
[ownership-history.md](ownership-history.md).

**D-own-16 CLOSED 2026-09-03.** Every cell that should reach zero does, on both backends, with
every value unchanged: a minting call that reads the local, the self-referential join
`c = mk(i) ?? c`, a conditional borrow, and a local bound from a PARAMETER and then minted.
The one shape that still retains a store is a lambda-CAPTURED local, and that is
`(L-CapHeap)` holding rather than a leak — a captured heap value is SHARED, so declining the
free is the right answer and its right answer keeps a store.  Guard:
`tests/scripts/1085b-a-nullable-local-frees-what-it-displaces.loft`; the full record, including
the two mechanisms that were tried and reverted, is in
[ownership-history.md](ownership-history.md).

The full register — these entries in full, plus every closed one with its dates and
issue numbers — is the companion [ownership-history.md](ownership-history.md).

## Machine-checkable soundness — the @PLN94 flow-sensitive oracle (proof skeleton)

The register above is **validation, not a machine-checked proof**: the shipped fact is computed
flow-INSENSITIVELY (the join of all defs) and the `Join` case discharges through a runtime witness.
[@PLN94](../plans/94-cfg-ownership-dataflow/) builds the flow-SENSITIVE replacement — a monotone
dataflow fixpoint (`src/ownership_cfg.rs`) run BESIDE the shipped analysis as an independent oracle,
never driving codegen (SI-1). Being a textbook abstract interpretation, that oracle is the piece that
CAN carry a machine-checked proof. This section states the obligations and discharges them — the
substantive lemma (4), local transfer soundness, is now proved case by case below (hand-written prose;
a Coq/Lean rendering is the only rigour polish left). The result: the flow-sensitive fact is
**over-free-sound given the O-\* rules**, so a green check is a proof-backed over-free-freedom
certificate on every program where the oracle and shipped analysis agree.

**What is proved, and what is not.** The target is the **over-free** class only — no free of a store
the fact does not own (⇒ no use-after-free, no double-free of that store). It is **NOT** a no-leak
proof (under-free is a disjoint class the shipped leak-check owns — @PLN94's coexistence finding:
`LOFT_NO_JOIN_OWN` leaks past this oracle but not past the leak detector). And it proves the
**oracle**, not the codegen: the shipped path inherits the certificate only where the two agree,
which is why they run beside forever.

**(1) The abstract domain — DISCHARGED.** `OFact = ⊥ | Owned | Borrowed(b) | Join(b)` with meet `⊔`
is a join-semilattice (finite height ≤ 3), and `refines` is its partial order. *Proof:*
`ofact_meet_is_a_join_semilattice` + `ofact_refines_marks_precision_and_flags_the_unsound_direction`
(unit tests, `src/ownership_cfg.rs`).

**(2) The concrete property.** In [operational.md](operational.md)'s `⟨e, σ⟩ → ⟨e', σ'⟩` with the
[heap.md](heap.md) store `H`, define at each program point the relation *owns(v)* = the var whose
binding is responsible for freeing `v`'s store (per **O-Owner**: exactly one). A free of `v` is
**sound** iff `owns(v) = v` at that point (**O-Derived**). Over-free = a sound-fact says `Owned`
where concretely `owns(v) ≠ v`.

**(3) The Galois connection.** `γ(Owned) =` { states where `owns(v)=v` }; `γ(Borrowed(b)) =` { states
where `v` aliases `b`'s store, `owns(v)=owns(b)≠v` } (**O-Borrow**); `γ(Join(b)) = γ(Owned) ∪
γ(Borrowed(b))` (runtime-dependent); `γ(⊥) = ∅`. `α` is the pointwise best abstraction. Obligation:
`γ` is monotone w.r.t. `refines` and `⊔` is its sound join — *straightforward from (1); to write.*

**(4) Local soundness of the transfer — DISCHARGED for the over-free property (given the O-\* rules).**
The property the over-free check needs is **no false `Owned`**: wherever the fixpoint reports
`st(v)=Owned` at a site the check trusts (a free, a return), `v` genuinely owns a store there — and a
non-`Owned` fact authorizes no free. This is *weaker* than full `σ'∈γ(f)` (a `Borrowed`-where-owned
fact is a leak-direction imprecision, out of scope) and is exactly what obligation (2) defines as
over-free. The transfer (`ownership_dataflow`) is per-var — `st'[var]=f(rhs,st)` — so prove per RHS
shape that `f=Owned ⇒ owns'(var)=var`, and that a non-`Owned` `f` authorizes no free:

- **(a) `OpDatabase(var,…)` / a record `OpNewRecord` → `Owned`.** heap.md `alloc` mints a FRESH store;
  **O-Owner** ⇒ its unique owner is `var`. `owns'(var)=var`. ∎ *(independent)*
- **(b) projection `OpGet*(base,…)` → `Borrowed(root)`.** A non-`Owned` fact — authorizes no free; and
  it correctly names the view's owner (`borrow_base`'s root, **O-Borrow**). ∎ *(independent)*
- **(c) bare `var = u` → `st[u]`.** When `st[u]=Owned`, `u` owns a store, and `var=u` — a MOVE, or a
  non-move alias materialised as `OpCopyRecord` (a/e) — leaves `var` owning one either way, so
  `owns'(var)=var`. When `st[u]` is `Borrowed`/`Join`, `f` authorizes no free. The
  `unwrap_or_else(ownership_of)` boundary (`u` absent — a parameter) yields `Borrowed(u)`, non-`Owned`.
  ∎ *(the `Owned` sub-case is independent; see the bridged gap below for the moved SOURCE)*
- **(c′) bare `var = u` where `var` is `OpDatabase`-re-minted on some path → `Owned`** (the `reminted`
  rule, taking precedence over (c)). If `var` is the arg-0 of an `OpDatabase` anywhere in the body then
  **O-Owner** gives `var` a fresh store on that path, so `var` is a materialised OWNED local; a
  whole-value `var = u` copy into it is a materialised copy (`OpCopyRecord` at codegen, as a/e) that
  owns a fresh store — `owns'(var)=var`. NARROW by construction: it fires ONLY for a bare `Var` RHS,
  never a projection (a `OpGet*` view stays `Borrowed(root)` per (b)), so no borrowing view is
  manufactured `Owned` — the property that keeps the A1b returned-view catch intact. ∎ *(independent —
  rests on O-Owner + the C86 copy-materialisation, like (c)'s `Owned` sub-case)*
- **(d) non-native call `var = f(args)` → `call_own`.** `f=Owned` only when the callee
  `return_ownership` is `Owned` = *returns a fresh store* (**O-Move**), so `owns'(var)=var`; a
  `Borrowed(argᵢ)` return authorizes no free. Sound by induction over the call graph — the callee
  summary is (4) applied to `f`; the recursion back-edge is `Borrowed(⊤)`, never `Owned`, so no false
  `Owned` is manufactured. ∎ *(inductive over the call graph)*
- **(e) else.** Record → `Owned` (fresh, O-Owner). Scalar/literal → `Owned`, but it owns no heap store,
  so a "free" of it is a no-op — no over-free. Native op → `call_ownership` (as d). ∎
- **(f) `= null` skip.** No store minted; a free of a null DbRef is a no-op. ∎ *(independent)*
- **(g) self-borrow `Borrowed(var) → Owned`.** The @P302 self-dep `[s]` is an ownership marker (re-init
  in place), not a borrow — **O-Owner** ⇒ `owns(s)=s`, so `Owned` is correct. ∎ *(independent)*
- **(h) the meet `IN[b] = ⊔ₚ OUT[p]`.** `IN=Owned` **only if every** predecessor's `OUT=Owned`
  (`Owned⊔Borrowed=Join`, not `Owned`), so `owns=var` on every incoming path — no false `Owned` from a
  join. **O-Complete**: no arm is dropped. ∎ *(independent, from (3)'s sound join)*

**The one bridged gap (the over-reach guard — honest).** `OFact` has no `Moved` state, and the
transfer does NOT kill a moved-out *source*: after `var = u` (a move), `u`'s fact stays `Owned` though
`u` no longer owns its store. So the FACT is not a full sound abstraction for moved sources. The CHECK
is over-free-sound anyway, because **O-Move** forbids the shipped plan from *freeing* a moved source —
the sole site where the stale `u=Owned` could authorize a bad free never arises. This, plus the
interprocedural induction (d), is where the over-free guarantee rests on the O-\* rules rather than the
fact alone; the INDEPENDENT part is the flow-sensitive structure — the meet, the structural-op
classification, the self-borrow/null carve-outs. A disagreement in the rule-relative cases is exactly
what the shadow-diff (Check A) surfaces — which is why the two run beside forever.

**(5) Fixpoint soundness — from (1)+(4), DISCHARGED.** The round-robin least fixpoint over the CFG
converges (**≤ n+2** passes, asserted SI-3), and by (4)'s local soundness + monotonicity + Tarski, the
per-block OUT-state soundly over-approximates the concrete ownership at every reachable point (given
the O-\* rules). *Bound, convergence, and — with (4) now discharged — the soundness step all hold.*

**(6) The check corollary.** With (5): if the oracle's check is **GREEN** — Check B finds no
unconditional `OpFreeRef(v)` with `st(v) = Borrowed`, and Check A finds no fact the shipped analysis
disagrees with in the unsound direction — then the emitted plan performs **no over-free** of the
covered classes. Contrapositive is the A1b catch: the `LOFT_NO_A1B` plan returns a store the fact
reads `Join`/`Borrowed` while the shipped fact reads `Owned` → RED (verified end-to-end,
`tests/ownership_oracle.rs`), a wrong plan every runtime gate passes.

**(7) Coexistence conclusion.** The proven oracle is a machine-checked *certifier*: on every program
where oracle and shipped analysis agree (empirically: 505-corpus + 54-cell fuzzer + 377 scripts, all
0 RED), the program carries a proof-backed over-free-freedom certificate; a residual disagreement
indicts one side for adjudication. This upgrades the register's *"validated"* to *"the flow-sensitive
fact is over-free-sound given the O-\* rules; the shipped analysis is certified per-program by the
proven oracle running beside it."*

**Obligation ledger.** DISCHARGED: (1) lattice; (3) `γ` sound-join (used in 4h); **(4) local transfer
soundness for the over-free property (the substantive lemma) — proved case by case above; the
flow-sensitive structure (4a,b,f,g,h) independently, the interprocedural summary (4d) by induction,
and the one moved-source staleness bridged by O-Move**; (5) fixpoint bound/convergence (SI-3) +
soundness step; (6) the check corollary; backend fact-identity (SI-2, `tests/ownership_oracle.rs`);
no-crying-wolf at corpus + fuzz scale (empirical). REMAINING (rigour polish, not a gap): a
machine-checked (Coq/Lean) rendering of this prose — the argument is complete but hand-written. OUT OF
SCOPE: no-leak (under-free — the leak detector's + the `check-leak` scan's class); the `Join`
runtime-witness discharge (a separate `OpFreeRefIfDistinct` lemma); proving the shipped 8-mechanism
analysis directly (the certifier sidesteps it).

## Conformance

- **A call-shaped argument names its base (`O-Oracle`, loft#1318)** — a fn-ref `??` whose
  argument is `pick(vs, 0)`, `m[k].v` at the hash / sorted / index kinds, `vs[0]`, or a value
  delivered through the caller's own `__ref_N` buffer keeps the caller's container intact over
  five borrows, while the mint arm of the same closure still costs no store per call at 70 000
  iterations and a MINTING argument (`g(mk())`) is still adopted. Both backends. Guard
  `tests/scripts/1318-a-call-shaped-argument-names-the-store-a-fn-ref-may-hand-back.loft`,
  14 cells, 8 of which fail on `b1bd3212`; the other 6 are its controls.
  ⚠ Its two interpolated cells are interpolated deliberately: `s += g(vs[0])[1]` and
  `c = g(pick(vs, 0))[1]` are CORRECT on the broken build, so the accumulate and bind
  spellings of the same read score nothing. Statement context is an axis here.

This area's "falsifying programs" are the store-lifetime bugs themselves — each is a
program where the derived-free invariant (O-Derived) or completeness (O-Complete) fails
and a store leaks, double-frees, or a backend diverges. The area is **formal when OPEN
reaches 0**: when every store-lifetime decision is one `deps` read (O-Deps) over a complete,
typed fact, the bug class is closed by construction and `binding.md`/`types.md`'s
`deps`-fused rough spots (the `Deps`-in-`Type` fusion) resolve with it.
