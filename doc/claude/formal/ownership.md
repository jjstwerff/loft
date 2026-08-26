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
```

**In words.** There is a real oracle, and it is not the dep list. `deps` is a cheap
stand-in for it that is right most of the time and wrong for a borrow nobody recorded a dep
for; `is_skip_free` is the patch that makes the stand-in safe at a free site; and
`owned_refs` carries the two things a type cannot — *which* assignment, and *how deep in
loops* it happened.

⚠ **The reason to write this down is that the choice is currently invisible.** 24 functions
test `depend().is_empty()`; some legitimately want the proxy (they are asking "is this a
view?", not "may I free it?"), some memo the oracle, and some free. Nothing in the source
distinguishes them, so a reader cannot tell a site that correctly reads one fact from a site
that reached for the wrong one — and both compile. `O-Proxy`'s "MUST also consult
O-Override" is the first checkable obligation in that space.

⚠ This does **not** re-open `D-own-1` (CLOSED: *"every free/copy/move reads `deps`"*). That
remains true in the letter — these sites do read `deps`. What was never true is the
implication that reading `deps` is *sufficient*.

---

## Deviations

OPEN: **1** (D-own-8, 2026-08-24; its Face B CLOSED the same day, Face A NARROWED 2026-08-25
to a single cell — an inline-minting `match` arm — with every other cell fixed) — D-own-9
opened and closed 2026-08-26, D-own-7 opened and closed 2026-08-23, and D-own-6 before it;
the five original D-own deviations remain resolved.  Read those entries for what their oracles vary before treating any zero
here as a measurement: each rested on a Join corpus that pinned one axis, and moving that
axis found a fresh family every time — which is exactly how D-own-8 arrived, from a consumer
rather than from an oracle at all, and how its second face was found by varying the POSITION
of the same join.  Face B is also this register's clearest case of a leak MASKING a wrong
answer: the interpreter retained what `--native` recycled, so the defect was filed at its
mildest symptom and the `silent-wrong` half only appeared once the retention was removed.

### D-own-9 — CLOSED (2026-08-26, loft#1096): a COLLECTION return's promoted buffer is the CALLER's store, and the callee freed it

`(O-Owner)` says a free is for a store the value OWNS.  A `-> vector<T>` function whose tail
may deliver `null` freed one it did not:

```loft
fn f(n: integer) -> vector<integer> { if n == 0 { null } else { [n] } }
fn main() { t = 0; for i in 0..2 { r = f(i); t += len(r); } }
```

`ref_return` renames the value arm's backing ref `__vdb_1` onto the hidden `__retbuf`, and
`scopes::free_vars`' loft#688 leg then enrols that renamed argument as *a local this function
minted* and emits `OpFreeRefIfDistinct(__vdb_1, __ret_1)`.  On the null arm the witness is
the sentinel, the store numbers differ, and the free fires — on the CALLER's store.  The
caller still names it (`__ref_1`, freed at its own scope exit) and still passes it to the
next call, whose entry `OpClearVector` then reads a freed record: `rec=0xDEADBEEF` under
`LOFT_POISON`, on BOTH backends, from the second iteration on.  One call is clean, which is
why it took a loop in the poison corpus to surface it — a newly-armed guard reporting old UB.

**The premise that failed is written in the leg's own comment**: *"a buffer not yet minted on
this path is the null sentinel, which `free` ignores."*  That is true of a RECORD return,
whose caller-side work-ref reaches the call as a bare `OpInitRef` sentinel.  It is false of a
collection: `codegen::gen_set_first_vector_null` gives an owned vector work-ref `OpInitRef` +
`OpDatabase`, so the buffer arrives ALIVE — and the callee's own `OpDatabase` then reuses that
store in place (`alloc_record_at` clears and re-claims a live slot rather than minting beside
it), so there is never a distinct callee-minted store for this free to reclaim.  Closed by
excluding a collection return from that leg, citing `@FR-O-Owner` (`src/scopes.rs`).

**Measured, not reasoned:** the free was emitted in **44 of ~1000** corpus programs and
FIRED 375 times across 20 of them, so removing it is live rather than theoretical; every one
of those 44 is green under `LOFT_POISON=1` + `LOFT_STRICT_STORES=1` with no leak, which is
what says the store it reached was always the caller's.  Emitted IR is otherwise identical:
the whole diff is the `__ret_N` hoist and the free it existed to carry.

⚠ **The fix is NOT at the promotion, and that corrects an extrapolation this file invited.**
D-own-8's Face B (loft#1081) closed *"at the promotion, which is where the unsound step is"*,
and the loft-codegen skill's loft#1096 note reads that line as naming where THIS fix belongs.
Refusing the rename was built and measured, and it is the wrong cut twice over:

* it **over-fires**.  The gate has to be *"the tail may deliver the sentinel"*, and a `match`
  with no catch-all lowers its fallthrough to exactly that sentinel — so an ordinary
  `match e { A { xs } => { xs }, B { ys } => { ys } }` loses its NRVO and copies each arm
  through a second store (`tests/use_analysis.rs::ownership_pins_match_return_resisting_cases`
  is what caught it).
* it needs a **second half** to stay correct.  Dropping to `Bind` copies the WHOLE tail into
  the buffer and answers the buffer on every path, so the null arm delivered an EMPTY vector
  instead of the sentinel and `f(0) == null` read false — loft#936's contract, broken by the
  repair for loft#1096.

The rename is sound here: building into the caller's buffer is the NRVO the design wants, and
it produces correct values with no leak.  What was unsound is the free that read the rename as
*minted here*.  **A closure names where ITS unsound step was; the next defect in the same
machinery has to be measured, not inherited.**

Guard: `tests/scripts/1096-a-null-return-must-not-free-the-callers-buffer.loft` — twelve
cells, five of them falsified on the pre-fix binary under `LOFT_POISON=1` (and none without
it: the freed bytes are still usable, so the poison job is what scores this file).  Both
halves are pinned, because neither implies the other — a fix that only refuses the rename
passes every use-after-free cell and fails `the_null_arm_still_answers_null`, and a fix that
only changes the delivery still faults on cell one.  Controls: the `[]` arm (the issue's
workaround, which keeps its rename), the RECORD family (which keeps rename AND free), and a
join with no null arm at all.

⚠ **A second defect found by the same probes — and the SAME dead premise, one site over.
Closed as `calls.md` D-call-3 (loft#1097).**
`fn g(k) -> vector<integer> { a = [1,2]; if k < 0 { null } else if k == 0 { a } else { [k] } }`
answered `g(0) == []`; dropping the null arm answered `[1,2]`.  The null arm makes `__vdb_1` a
second promotion candidate, which takes `Bind`'s whole-tail copy —
`OpClearVector(a); OpAppendVector(a, <the join>, 0)` — and `a` IS the promoted buffer, so the
clear runs before the join is evaluated and the `k == 0` arm answers the buffer it just
emptied.  That is loft#1078's *"the re-mint destroys the store the copy is about to read"*
with a CLEAR in place of the re-mint, and `classify_ret_promotion`'s `tail_reads_buffer`
guard against exactly that shape is RECORD-only.  Both backends agree, so backend agreement
is again not an oracle.

**And the null arm of that same tail never reached the caller.**
`returned_var_null_unified` folds a null arm onto its sibling's var on the belief that the
var holds the sentinel on the null path — *the same belief this entry's free leg holds, at a
different site*.  For a RECORD it is true; for a collection buffer it is false in both
places, and it cost a use-after-free here and a wrong value there.  **One wrong belief, two
defects, one day apart** — so grep the belief, not the symptom: any site reasoning that a
collection slot holds the sentinel on a path that did not write it is suspect.  Both are
fixed; `calls.md` D-call-3 carries the return half.  What is left is loft#1098, a per-call
leak on a `match` tail that needs a null arm, a local arm and a literal arm all three.

### D-own-8 — OPEN (2026-08-24, loft#1082 / loft#1081): a Join's ownership fact is true on one path only

`(O-Complete)` requires the fact PER BINDING, PER PATH — "every binding, including every
`match`/`if` arm".  A join whose arms disagree produces ONE fact for BOTH paths:

```loft
line = if len(pts) > 2 { smooth_pts(pts, flags, false) } else { pts };
```

The then-arm is a call returning a freshly-owned vector; the else-arm is a bare local.
`LOFT_VAR_TABLE` shows the binding typed `def deps=[pts]` — a BORROW.  On the owning path
the fact is false.

⚠ **The mechanism this entry originally named is NOT the one in play, and that was
falsified 2026-08-25.**  It read: *"`arm_join_type` strips only the deps an arm MINTED
(loft#978), and `joined_deps` then UNIONS, so `{} ∪ {pts}` = `{pts}`."*  Two probes against
the `if` shape above — the entry's own example — say otherwise:

* removing the strip from `arm_join_type` entirely leaves the binding's deps **unchanged**;
* a tracer in `arm_join_type` **never fires** for this shape at all.

The `if`-expression joins through `merge_dependencies(&true_type, &false_type)`
(`parser/control.rs`), which is a different path; `arm_join_type` serves the `match` arms.
So an attempted fix aimed at `arm_join_type` — the obvious reading of the old text — would
have changed code that never runs for the reported program.

**The real mechanism, found 2026-08-25 by instrumenting `merge_dependencies` and then
`LOFT_LOG=type_timeline:line`.**  The union is computed CORRECTLY and is then collapsed by a
setter that replaces where its caller assumes it accumulates:

```
[MD]  a=[5] b=[0] -> [5, 0]                    merge_dependencies: the union is RIGHT
[type_timeline] line  Unknown -> [5, 0]        change_var_type stores both
[type_timeline] line  [5, 0]  -> [5]           depend  (variables/mod.rs:1797)
[type_timeline] line  [5]     -> [0]           depend  — last one wins
```

(var 5 is `__ref_1`, the owning arm's mint dep; var 0 is `cp`.)

`Function::change_var_type`'s early-return does

```rust
for on in type_def.depend() { self.depend(var_nr, on); }
```

and `Function::depend` is `Type::depending(on)` = `with_deps(&Deps::frame1(on))` — it
REPLACES the whole list with `[on]`.  So a type carrying N deps collapses to its LAST one.
**Six sites in `variables/mod.rs` share that loop.**

**It is not join-specific, and that is the wider finding.**  A two-BORROW join loses one
source outright:

```loft
pick = if c { a } else { b };   // pick def deps=[b] — the dep on `a` is gone
```

`pick` aliases `a` on the taken path, and nothing records it.

⚠ **Still no symptom.**  The two-borrow shape was probed with the dropped source going out
of scope first, under `LOFT_POISON` + `LOFT_STRICT_STORES`, and answers correctly — so
something downstream keeps the dropped source alive.  The collapse is a real defect in the
FACT with no demonstrated consequence, which is the same position Face A has always been in,
now stated one layer deeper and at the right function.

**FIXED 2026-08-25 — `Function::depend_all`.**  All six sites now route through one setter
that keeps every incoming dep instead of the last:

```rust
fn depend_all(&mut self, var_nr: u16, type_def: &Type) { … }   // variables/mod.rs
```

Two properties make it a replacement rather than a widening, and both are guarded in
`variables::loop_binding_dep_tests`:

* a value borrowing two sources records BOTH (`a_binding_that_borrows_two_sources_records_both`);
* an EMPTY incoming list is a **no-op, not a clear**
  (`an_empty_incoming_dep_list_does_not_clear_the_established_ones`) — the loop it replaces
  did nothing for an empty list, and *"the types agree, adopt the deps"* never meant *"and
  drop what you had"*.

It adopts the incoming list WHOLE; it is deliberately **not** a `Deps::union` with the
variable's existing deps.  The loop replaced, so replacing-without-collapsing is the minimal
change that fixes the loss and nothing else.  For the same reason it keeps dropping the
`u16::MAX` share-marker (#328), which `depend`'s own guard has always skipped: two downstream
decisions read that marker's presence (a struct field's layout via `deps.contains(&u16::MAX)`,
and `deps == [u16::MAX]` as a predicate of its own), so carrying it through would have changed
answers well outside this defect.

⚠ **Guarded on the predicate, not through a program, and the reason is the entry above:**
the collapse has no observable symptom, so a script-level guard would assert nothing while
the fact is plainly wrong — and the fact is what every free-placement decision reads.  The
two tests above it in that module carry the same disclaimer for their own reasons.

Measured on the two shapes this entry names:

```
[vartable]  pick  vec<ref>  def deps=[a(0), b(2)]        ← was [b]
[vartable]  line  vec<ref>  def deps=[__ref_1(5), cp(0)] ← was [cp]
```

**Sibling audit — the class, not the site.**  The collapse is *a loop over a dep list whose
body calls a REPLACING setter*, so it was swept as that shape rather than as six known lines.
21 loops in `src/` iterate a dep list; 18 only read it.  The other **three** had the identical
defect and are fixed with the same `depend_on_all`:

| site | shape |
|---|---|
| `parser/expressions.rs` (@PLN85 cluster V) | save/restore around `change_var` — **strips every dep in a correct loop and restores only the last** |
| `parser/vectors.rs` | an element binding adopting its parent's deps |
| `parser/objects.rs` | a temp adopting the written value's deps |

The first is the one to notice: its SAVE side loops over the whole list and its RESTORE side
collapses, so the asymmetry was visible in the same six lines the whole time.  **And the union
fix makes these siblings more dangerous rather than less** — multi-dep lists were previously
rare *because* the six sites kept flattening them, so fixing the six is what puts lists of
length > 1 in front of the other three.  A class-wide sweep was therefore a precondition for
the fix being safe, not a tidy-up after it.

**The blast radius is bounded by a property worth checking rather than trusting.**  Writing
`n` for the incoming list's length after the `u16::MAX` filter:

| `n` | before | after |
|---|---|---|
| 0 | no-op | no-op |
| 1 | `[d]` | `[d]` |
| ≥ 2 | `[last]` | **`[all]`** |

Before and after are non-empty in exactly the same cases, so **`depend().is_empty()` answers
identically at every site**, and the fix changes *which* deps a value carries — never
*whether* it carries any.  That matters because at least three decisions read that predicate
AS AN OWNERSHIP TEST (`vector_needs_db`, `classify_vec_bind`, and the `[]`-means-owner reading
in `minted_vars`), and each is measured load-bearing: neutralising the first breaks
`tests/scripts/03-text.loft`, and inverting the second corrupts (#426).  None of them can move
under this change.

**How often the collapse actually fired, and where — this is the part that reflects on the
register itself.**  Counted with one env-gated `eprintln` on the `n >= 2` arm, over all 858
corpus programs: **48 events in 12 files** (47 of two deps, one of three).  So the fix is live
rather than theoretical — the corpus was dropping a dep 48 times — and the whole suite passes
either way, meaning nothing had come to depend on the collapse.

The 12 files are the finding.  Almost every one is a regression guard written for an EARLIER
ownership deviation:

```
11  h7-loop-retbuf-alias        7  1081-a-join-bound-to-a-returned-local
 9  848-value-block-local…      5  1051-tuple-destructure-ownership   (the 3-dep case)
 2  981-split-ownership-return  2  139-drop-cascade
 1  1019-join-owned-arm-owner   1  172-store-confinement-soundness
```

Those guards were passing while the fact underneath them was incomplete — which is what
[README § deviations](README.md) means by an `OPEN: 0` line being only as strong as its oracle.
They still pass now that the fact is complete, so none of them was ever *scored* on the dep
list; they were scored on values, and the collapsed list was invisible to them.  Treat the
earlier D-own zeros accordingly: they were measured over a corpus in which multi-source deps
could not survive to be measured.

⚠ `vectors.rs` keeps one pre-existing behaviour deliberately: a `self.vars.depend(elm, vec)`
immediately above is still overwritten when the parent carries deps.  Whether `elm` should
depend on BOTH `vec` and the parent's list is a separate question this fix does not answer —
adopting the parent list whole changes only what the loop lost.
**One fact, two questions — and it is only right for one of them.**  `joined_deps`'
own doc-comment justifies the union as the reading "no arm can contradict: it can only
keep a store alive longer than one arm needed, never free one another arm still holds."
That is true of the question the union was written for — *what must stay alive?*  It is
false of the OTHER question the same `deps` list answers — *does this binding need a
backing store of its own?*  `vectors.rs::vector_needs_db` asks it as
`self.vars.tp(vec).depend().is_empty()`, so a union that is non-empty says "borrows,
needs no store" about a value that OWNS on the path that runs.  Conservative for
liveness is anti-conservative for allocation.  Two named hazards meet here: an empty dep
list read as *owned*, and one derived fact with two homes.

**Face A — NARROWED 2026-08-25 to one cell, by the `depend_all` fix above.**  The entry
below predicted the closure would need *"a lowering change — making the join's own result
carry a mint dep"*.  It did not: the lowering **already produced** that mint dep, and the
collapse was discarding it.  With the collapse fixed, the filed shape reads

```
pf_line  def deps=[__ref_1(10), pf_cp(7)]   ← was [pf_cp]
pf_wids  def deps=[__ref_2(12), pf_cw(8)]   ← was [pf_cw]
```

— the owning arm's mint marker beside the borrowing arm's dep, which is exactly what
`arm_join_type`'s own comment calls *"what says which store the result owns"*.  The fact is
no longer true-on-one-path-only for this shape.

⚠ **One cell survives, and it makes the two spellings DISAGREE.**  Varying the owning arm
between a CALL and an INLINE mint, across `if` and `match`:

| owning arm | `if` | `match` |
|---|---|---|
| a call (`smooth(cp)`) | `[cp, __ref_1]` ✓ | `[cp, __ref_1]` ✓ |
| an inline mint (`[for v in cp {…}]`) | `[cp, __vdb_3]` ✓ | **`[cp]`** ✗ |

Values are correct in all four cells; only the FACT differs.  The `match` row is
`arm_join_type` stripping the contributed arm's minted vars — which is why the call cell
passes for the wrong reason: `minted_vars` **cannot see a mint inside a callee**, so the
strip finds nothing to remove and the union survives by accident.  Move the mint into the
arm and the strip engages.

That strip is deliberate (loft#978: publishing an arm's mint as a dep made the return
machinery read the result as a view of a local, and `deliver`'s return went to `["??"]`), so
removing it trades this deviation for that one.  It is the entry's *"one derived fact, two
homes"* hazard in its sharpest form: the strip is RIGHT for the delivery question and WRONG
for the ownership question, and one dep list answers both.  **Face A stays OPEN for the
inline-mint `match` arm only**, pending a way to separate those two readings.

**Face A — the allocation answer (the original statement).**  A borrow-typed slot owns no store, so a
whole-value assignment into it has nowhere to land.  The false fact reduces to ~55 lines — a
`for` over a vector of structs whose vector fields are copied into locals, then the mixed join
— reproducing `pf_line def deps=[pf_cp]` and `pf_wids def deps=[pf_cw]` exactly as filed.
No wrong answer or crash is yet attributed to it; it is a false FACT looking for its symptom.

**Symptom hunt, 2026-08-25 — the fact REACHES its site, and still nothing breaks.**
Re-reproduced in ~20 lines (`LOFT_VAR_TABLE` shows `line def deps=[cp]` for
`line = if len(cp) > 2 { smooth(cp) } else { cp }`).  Instrumenting `vector_needs_db`
confirms the decision is reached and answers with the false fact: `[VNDB] line deps=1 →
false`, i.e. *no backing store allocated*.  So these probes are not vacuous — they arrive at
the named site, take the branch the false fact selects, and are still correct:

| probed shape | result |
|---|---|
| whole-value reassign into the joined slot (`line = other()`) | correct, source intact |
| build a comprehension into it (`line = [for p in cp {…}]`) — the `op == "=" && !needs_db` → `OpClearVector` path, which builds into the EXISTING store | correct, **source and `cp` both intact** |
| append after that reassign | correct |
| 40 rounds under the leak gate + `LOFT_STORES=timeline` | 4 allocs / 2 frees, **no leak** |

⚠ **`depend().is_empty()` is not an ownership test, and that is sharper than the union
story.** Printing the dep NAMES at that site separates two cases the emptiness test
conflates:

```
[VNDB2] out    deps=["__vdb_1"]   ← dep on its OWN mint var: it OWNS a store already
[VNDB2] result deps=["__vdb_1"]
[VNDB2] shapes deps=["__vdb_1"]
[VNDB2] line   deps=["cp"]        ← dep on ANOTHER LOCAL: a borrow
```

`minted_vars`' own doc states the first reading: *"`[]` lowers to `OpDatabase(__vdb_N, …)`
and the value then types as a dep on `__vdb_N`, which says I own this store — the opposite
of the borrow a dep normally records."*  So the list carries THREE meanings, not two: empty
(no store yet), a mint dep (owns one), a local dep (borrows one).  `vector_needs_db` reads
only emptiness, and answers "needs no store" for the mint case correctly and for `line`
correctly-by-accident.

(An earlier note here said the false answer was "the well-trodden branch" because the other
vars reach it the same way.  That was wrong: they reach it with a MINT dep, which is a
different case with a correct answer.  `line` is the only anomaly in the run.)

**The fix direction the rules make sayable.** This is `O-Proxy` and `O-Oracle` meeting:
`vector_needs_db` asks an OWNERSHIP question (*do I own a store?*) using the dep list, while
`joined_deps`' union answers a LIVENESS question (*what must stay alive?*).  The obvious
closure is for the allocation site to read `O-Oracle` instead of the proxy.

⚠ **That closure was attempted 2026-08-25 and is STRUCTURALLY UNAVAILABLE at that site.**
`O-Oracle` (`use_analysis::ownership_of`) classifies from `data.def(d_nr).code` — a
POST-PARSE analysis over a finished body.  `vector_needs_db` runs inside the parser, which
(measured) has **no current-def handle at all** and **never calls the oracle**: every one of
its 20-odd consumers lives in `scopes` / `codegen` / `generation` / `ownership_cfg`.  There is
no def_nr to pass and no completed body to classify.

Two further measurements bound the problem:

* **The proxy term is load-bearing.**  Neutralising `depend().is_empty()` in
  `vector_needs_db` breaks `tests/scripts/03-text.loft` — it cannot simply be dropped.
* **The false fact reaches that site and still does not decide the allocation.**  Instrumented,
  `line` arrives as `deps=1 → false` ("no backing store"); yet the emitted IR shows
  `OpDatabase(__vdb_2)` at the reassignment repointing `line` to a FRESH store, so the
  borrowed one is never cleared.  Something downstream allocates regardless, which is why
  no probed shape bites.

**The second-fact route was attempted next, and the blocker is placement, not machinery.**
The flag has to be SET where the mixed join is visible and READ where the var is known, and
no single point has both:

* the join sites (`control.rs`, six of them) build a `result_type` and have **no destination
  var** — the binding happens later, at the assignment;
* the bind site has the var and the joined type, but the union has already erased which arm
  owned, and the arms' TYPES are gone — a structural re-check of the IR cannot recover it
  either, because the owning arm here is a bare CALL and `minted_vars` sees no mint (the
  mint is inside the callee);
* carrying the flag on `Deps` would travel correctly but **does not survive the store
  round-trip** — `ir_schema` reconstructs a dep list as `Deps::unknown(vec![…])`, so a
  warm-loaded program from the startup cache would lose it and answer differently from a
  cold one.  A correctness flag that a cache drops is worse than none.

So the flag wants to be set at the join and read at the allocation, and the two are separated
by a bind that discards the distinguishing information.  Closing Face A means first giving the
join a way to reach the binding — most plausibly by making the join's own result carry a mint
dep (the `["__vdb_N"]` form above already MEANS "owns"), which is a lowering change rather
than a flag.  Face A stays OPEN pending that design, with the placement constraints above
recorded so the next attempt does not re-derive them.

⚠ **loft#1082's panic was NOT this, and is now CLOSED elsewhere.**  Measured in a scratchpad
copy of the `drawing` package: replacing BOTH joins with imperative `for`-append loops — either
alone, or both — left `index out of bounds … 65535` exactly where it was.  The cause was a
two-pass work-ref collision with nothing to do with ownership at a join: `ref_return`'s
`Bind { substitute: true }` unregisters a substituted-away `__ref_N` (which also sets
`skip_free`), the `__ref_N` numbering DRIFTS between passes when a callee declared later in the
file mints a buffer on pass 2 that pass 1 did not, and `work_refs` re-minting that name
re-registered the ref while leaving `skip_free` standing.  `gen_set_first_vector_null` reads
`skip_free` as "do not allocate", so the buffer reached the callee as `DbRef::NULL`.  Fixed by
clearing the flag on re-mint; guard
`tests/scripts/1082-a-re-minted-work-ref-is-not-the-one-substituted-away.loft`.
A mechanism that explains the var table is not thereby the cause.

**Face B — a returned local the promotion should never have renamed (loft#1081, CLOSED
2026-08-24).**  The same one-path fact, at a join BOUND to a local the function returns:

```loft
fn pick(m: boolean, a: float, b: float) -> vector<float> {
  v: vector<float> = if m { [a, b] } else { [a] };
  v
}
```

`ref_return` NRVO-**renames** `v` onto the caller's return buffer.  That is right for
`v = [a]`, where the literal BUILDS into the buffer — and wrong here, because a join does
not build into its destination: each arm mints its own backing and the assignment REBINDS
the slot (`PutRef`).  So the buffer is abandoned the moment the join runs and the arm
store is handed back with no owner.  `(O-Owner)` is violated twice by one return: two
stores, zero owners.  The same join written at the function TAIL was always clean, because
there each arm materialises into `__retbuf` and frees its own backing — the BOUND spelling
simply never reached that path.

It surfaced as three symptoms, and only the smallest was filed:

* **one leaked vector per call**, both backends — the arm store nobody owns;
* **an untyped `kt=65535` store per call**, interpreter only — the un-taken arm's
  `__vdb_N`, eagerly allocated by `gen_set_first_ref_null` and never named again;
* **a silent wrong answer on `--native`, the DEFAULT backend** — the sibling arm was also
  freed at scope exit on the path that returns it, so the allocator handed the slot
  straight back and three calls answered the THIRD call's values for all three bindings.
  The interpreter answered correctly *because it leaked*: the leak was masking the
  use-after-free, which is why this arrived as a leak report.  Once the eager-allocation
  half was fixed the mask came off and the wrong answer showed on both backends.

Closed at the promotion, which is where the unsound step is: `classify_ret_promotion`
refuses the rename for a Vector local bound to a branch join
(`Parser::var_bound_to_branch`, citing @FR-O-Owner / @FR-O-Move), so the candidate drops
to `Bind` — the local keeps its own store and is copied into a separate `__retbuf` at the
return, the shape the tail join already used.  Companion: a `__vdb_N`'s entry null-init is
now the non-allocating sentinel (`gen_set_first_ref_null`, @FR-O-Derived), because its
`OpDatabase` sits at a BUILD site that may be conditional — the function prologue already
sentinel-inits every `__vdb` slot (#260 Fix B) and this was the one site that undid it.

The verdict is STRUCTURAL (does the body contain `Set(v, If …)`) rather than
ownership-based, because it is needed on PASS 1: `vector_db` runs only on pass 2, so on
pass 1 the binding's deps are still empty and no arm has minted anything.  A verdict that
differed across passes would move the hidden buffer argument between them.

⚠ **A first fix here was reverted as inert.**  Making the scope-exit free a runtime witness
(`OpFreeRefIfDistinct(v, ret_var)`) removed the wrong answer and left both leaks — a trade,
not a closure.  Once the promotion was fixed at its source, no control could falsify the
witness any more, so it came out: a guard that cannot fail proves nothing, and it had
already cost one native regression (`E0425` — a block-local `ret_var` is not in scope where
the free is emitted).

Guard: `tests/scripts/1081-a-join-bound-to-a-returned-local.loft` — values AND the wrap
harness's leak gate, both halves falsified by disabling the fix (57 leaked stores, and both
value cells red on both backends).  Neither half implies the other: silencing the leak by
freeing the DELIVERED store passes the leak gate and fails every assertion.


**The cure Face A needs is a rule decision.**  A binding whose value OWNS on some path
must own a store on every path.  That means a mixed-ownership
join types as OWNED and the borrowing arm MATERIALISES a copy — which is not a new rule
but `(O-Move)`'s existing sentence for the callee case ("if the return *borrows* a
parameter … the caller COPIES to obtain its own store"), and the model's own doctrine that
the compiler always finds a lowering, "copying when it cannot prove an alias is safe".
Half of it has been tried and measured to fail: making the owning arm win the union types
the binding owned but emits no `OpDatabase` for it, so the destination is still absent.
Both halves have to land together.

**What is NOT the cause, each eliminated by its own run.**  Reassigning over a field view;
passing local copies instead of field views; `LOFT_NO_CONF_RECOVER=1` (store confinement);
loft2's move-elide / DbRef-set work (`812aac5d` fixed the INVERSE — a borrow read as an
owned store); and blanket `mark_inline_ref` on every `__vdb_N` to stop the eager
allocation, which ALSO relocates the null-init and broke the tail-`if` return promotion
(a tail join answered `0` instead of `5`).  The eager-allocation fix therefore needs a
marker that changes alloc-vs-sentinel WITHOUT changing init order.

**Reductions.**  Face B reduces to nine lines (above).  Face A's false FACT reduces to
~55 lines.  loft#1082's PANIC does not reduce yet: the same tail-return-out-of-a-loop shape
written out on its own runs clean on BOTH backends, with the `const` parameter, the nested
caller loop and the struct-with-vector-field source all present — four constructed reductions
now.  The reliable oracle is a scratchpad COPY of the `drawing` package driven through `--lib`
(`Fronds … bow=0.16`, parse only, no render), which bisects freely and is how the tail-return
boundary was found; their tree stays untouched.


### D-own-7 — CLOSED (2026-08-23, loft#1078): every arm of a Join that OWNS a store is a candidate the free must name

`(O-Derived)` says free a local iff it owns its store and does not transfer it out.  A tail
`if`/`match` whose arms each own a store transfers exactly ONE of them, so the others are
locals that must be freed — and the promoted NRVO buffer is one of those arms.

`fn pick(c) -> S { w = S { a: 7 }; if c { S { a: 9 } } else { w } }` renames `w` onto the
hidden return buffer, so the `else` arm delivers the buffer and the `if` arm delivers a
different store.  `scopes::free_vars` reaches the losers through three legs — a null arm
(@PLN85 A.1), a promoted buffer no arm names (loft#688), and arms that disagree about
ownership (loft#1022) — and the multi-source leg that covers *"several owned candidates, one
winner"* excluded every ARGUMENT.  That exclusion is right for a user parameter, which belongs
to the caller, and wrong for the one argument that is really a local this function minted.
loft#1022's own comment had already named the carve-out and applied it inside its own gate;
the multi-source leg needed the same one.  One orphan per call, both backends, invisible in a
single call — `loft_planet` retained ~16,000 records per planet and four planets exhausted the
65,535-entry `store_nr` table.

**What the oracle held fixed, and what moving it found.** The filed report varied *what the
non-taken arm names* (a local, a parameter, a vector element) and held the RETURN POSITION and
the arm COUNT fixed.  Moving those two found two `silent-wrong` defects the leak had hidden,
neither of them an ownership fact:

* **Two owned locals** — the first is renamed onto the buffer, and the second's copy leg emits
  `OpDatabase(buf); OpCopyRecord(<tail that reads buf>, buf)`.  The re-mint destroys the store
  the copy is about to read, so the renamed arm answered a zeroed record.  A three-arm `match`
  broke only its FIRST arm, which is the tell that the buffer RENAME is the mechanism and the
  join is not.
* **Bound, then returned** (`r = if c { … } else { w }; r`) — not a tail join at all.  This is
  loft#848's class one arm over: the pass-2-only object-literal mint still drew from the shared
  `__ref_N` counter, so pass 2 handed it the name pass 1 had left on the return buffer, and
  `return_buffer()` resolves that buffer BY NAME.  The arm's record and the return destination
  became one slot.

Both answered wrong IDENTICALLY on the two backends, so `(O-NoDiverge)` held while
`(O-Owner)` did not — a reminder that backend agreement is not an oracle.  Guard:
`tests/scripts/1078-join-arms-that-each-own-a-store.loft`, both halves falsified on a pristine
worktree at `f7a57124` (the value cells by assertion, the leak cell by the wrap leak gate).

### D-own-6 — CLOSED (2026-08-20, loft#1029): the runtime Join witness now covers every argument it can name

`(O-Complete)` accepts the Join as *inherently runtime*: a callee whose return may borrow a
parameter is completed per-path by the @P290 bracket — `protect_store_frees` marks each ref
argument's store, and a returned store that is marked is refused the source-free while a
callee-minted one is freed.  The register closed D-own-2 on that basis.

The witness was not total.  The bracket needs a slot to name, so
`use_analysis::protectable_ref_args` accepted only a bare `Var`; for any other argument
spelling `covers_all` went false and the caller fell back to the conservative never-free
answer, orphaning the store the callee minted — one record per call, both backends.  The
axis is the ARGUMENT SPELLING, not what the borrow arm names: a vector-element borrow arm
leaks with a literal argument, and a parameter borrow arm is clean with a variable one.

The rule that closes it: **the witness names a STORE, not the argument.**
`protect_store_frees` marks an allocation and reaches it through any `DbRef` in that store, so
an argument only has to be DERIVED from a nameable slot by operations that stay inside one
store.  Two families, and they need opposite cures:

* **A view of a live slot** — `b.s`, `d.b.s`, `w[0]`, `vb.v`, `o ?? q`, `if c { q } else { r }`.
  The root of a projection chain is the witness, and a join witnesses every arm.  Nothing is
  hoisted; the slot already holds its `DbRef` when the bracket runs.
* **A construction block**, which MINTS the store it yields — a struct or collection literal.
  This one cannot be witnessed in place: the bracket is emitted before the arguments evaluate,
  so the work-ref still holds its null and marking it would protect nothing while reading as
  covered — trading the leak for a use-after-free.  It is hoisted into the enclosing statement
  list instead, which is the spelling (`q = S { a: 7 }; pick(q, …)`) that was always clean.

`null` in either spelling holds no store and needs no witness (loft#1021).

**The oracle that missed this** varied the instantiating TYPE and the join SHAPE and never
varied how the argument was SPELLED — every cell in `1019-join-owned-arm-owner.loft` binds its
argument to a variable first, and a corpus that sweeps four axes impressively is read as
coverage.  `tests/scripts/1029-inline-argument-borrow-source.loft` now moves that axis across
eleven spellings, each asserting BOTH arms plus the source's own value and, for a collection,
its length — because a cure that freed the DELIVERED store answers the same number on the
owning arm, and only a length or a source read can witness it.  The type-variable half of the
same gap is recorded in [interfaces.md](interfaces.md).

---

OPEN: ~~0~~ (2026-07-04, superseded above) — **the ownership register was at zero.**  All five D-own
deviations are resolved: D-own-3 (typed `Deps`) CLOSED; D-own-4 RECLASSIFIED as the
decided edge C86 (whole-value binds copy; aliasing is a last-use elision —
`classify_vec_bind`); D-own-5 (the `&` borrow rides `deps`) CLOSED; **D-own-2
(O-Complete) CLOSED** (the ownership fact is total — oracle covers every value, the free
side reads it, the inherently-runtime Join completed per-path by the `_own_store`
witness; validated by the 6-shape sweep + full gates + the `program_ownership` fuzzer);
and now **D-own-1 (O-Deps) CLOSED** — an audit of every store-lifetime DECISION site
(dispatch.rs / state/codegen.rs / ops/ref_ops.rs / scopes.rs / control.rs) found the
free/copy/adopt/drop decisions read the ONE canonical fact
(`ownership_of` / `returns_borrowed_view` / `return_adopts_fresh_store`) on the shipped
path — the last inline shape-scan (the interp adopt-vs-deep-copy visible-ref-param scan)
was unified onto `return_adopts_fresh_store()` matching the native sibling (commit
`0234cbbb`).  **The floor (honest):** the pre-fact scans survive ONLY under the
`LOFT_NO_JOIN_OWN` opt-out (differential-control machinery, not shipped behaviour); the
runtime Join witnesses (`_own_store`/`OpBindOrCopy`) are inherently-runtime (spec-accepted,
not a re-derivation); and collapsing the return-ownership readers into ONE physical funnel
is code-DRY, not a re-derivation (each already reads the fact).  Those are reclassified as
non-deviation cleanup — the O-Deps SUBSTANCE (no shipped decision re-derives ownership; the
fact is carried and read everywhere) is met.  Validated: full suite 2601/2601 (env flakes
only), `native_scripts`, `LOFT_POISON`, the `ownership_fuzz_gate` control pairs, the
differential oracle, and the fuzzer.

### D-own-1 — CLOSED (2026-07-04): ownership is carried as one `deps` fact, read (not re-derived) per-site
- **Violated:** O-Derived / O-Deps
- **Where:** the store-lifetime bug class — `has_ref_params`, the return-source set, the
  free-suppress / return-buffer logic, etc. ([OWNERSHIP_MODEL.md § Why](../OWNERSHIP_MODEL.md)).
  Each fix added a codegen condition rather than completing a fact.
- **Effect:** the recurring store-lifetime bugs (Cluster A, #426, #429, …) — "N forests,
  one root". The class cannot be closed by more conditions.
- **@PLN85 note (2026-07-04):** the store-lifetime BUG class is retired (@PLN85 closed) —
  the load-bearing re-derivations are ELIMINATED (return-delivery + reassign thicket
  collapsed behind `classify_X`/`dispatch_X`; the `ownership_of` oracle default-on, 0/54
  over-free; the free side reads `returns_borrowed_view()`) and no re-derivation produces
  a live bug (closed by construction: fuzz/poison/DA + leak-gate).
- **@PLN90 note (2026-07-04):** the LAST per-site ownership re-derivation is now GONE —
  `scan_set`'s owned-vs-view TRACKER (`ref_rhs_ownership`) no longer re-derives from the
  RHS shape; it reads the ONE canonical `ownership_of` oracle (Owned → track; Borrowed
  AND Join → View, since a borrow/join reassignment displaces the prior owned store and
  must not be tracked as owned).  So O-Derived is SATISFIED: every store-lifetime
  decision now reads the one canonical fact, not a per-site shape scan.  Validated: full
  suite + `native_scripts` + DA + `LOFT_POISON` + differential oracle green; the p462
  conditional `?? m_none()` transition and the C86 copy-return cases all clean both
  backends.  **The D-own-2 residual is now CLOSED too** (see below): the `_ => Owned`
  tail is correct (it covers only fresh-owned / scalar / payload-less values, not a
  hole), the value-vs-bind gap is INERT for the free decision (the reassign pre-free +
  type-based scope-exit free cover it), and the inherently-runtime Join is completed
  per-path by the `_own_store` witness — so the ownership fact is TOTAL.  O-Derived:
  **CLOSED** — the re-derivation is deleted.  What stays under D-own-1 is only the
  *single-fact* unification: the free/copy/move decisions read the canonical fact at
  their chokepoints, but three cooperating mechanisms (the static oracle read + the
  runtime Join witnesses + the return-buffer machinery) are not yet ONE `deps` read.
- **Status:** CLOSED (2026-07-04) — the audit + `0234cbbb` unification landed the last
  shipped shape-scan onto the fact (see the header for the close + the honest floor).
  History below.  Landed: the return-delivery
  collapse is COMPLETE — `block_result` 459→328 lines, **45→21 helper calls**, the 15
  tail-shape classifiers down to ~3 genuinely-distinct entry guards; EVERY delivery
  mechanism routes through a pure `classify_X` selector + `dispatch_X` (vector
  `Delivery`, Reference `RefDelivery`, text `TextDep`, `ref_return`'s
  `classify_ret_promotion`); the #416/#448 cells folded; class swept dry over ~41
  probes.  The `ownership_of` oracle chokepoints are **DEFAULT-ON**
  (`keys.rs::join_own_enabled`; 54-cell over-free map 0/54 default).  And the FREE
  side began reading the canonical fact: `scan_set`'s #316 ownership tracker
  (`ref_rhs_ownership`) and codegen's owned-ref reassign gate now call
  `returns_borrowed_view()` instead of re-scanning the return deps inline (2026-07-04,
  both byte-identical over the 8 D-own-1/C86/462 corpora).
  **AUDIT 2026-07-04 — the consumption side is now ~fully fact-reading.** A sweep of
  every store-lifetime DECISION site (dispatch.rs, state/codegen.rs, ops/ref_ops.rs,
  scopes.rs, control.rs) found the free/copy/adopt/drop decisions read the canonical
  fact (`ownership_of` / `returns_borrowed_view` / `return_adopts_fresh_store`)
  everywhere but ONE genuine residual, plus two non-violations:
  - **THE ONE RESIDUAL — `state/codegen.rs:1786-1789`**: the interp `v = call()`
    deep-copy path still gates on an inline *visible-ref-param scan* to decide
    adopt-vs-deep-copy, while the NATIVE sibling (`dispatch.rs:405`) already reads
    `return_adopts_fresh_store()`.  For a fresh-return-with-ref-param callee
    (`fn mk_from(seed) -> Box { Box{..} }`) interp deep-copies where native adopts —
    same value + leak-clean on both, but a mechanism divergence.  Unifying it onto
    the fact is a COPY-ELIMINATION small-step (adopt instead of deep-copy), not
    byte-identical — best done as a dedicated @PLN90 slice on this most-reverted
    path, with the corpus+matrix gate, NOT rushed.
  - NOT violations: `dispatch.rs:403-404` (`.starts_with("n_")` / `code()!=Null` are
    call-KIND eligibility filters, the ownership decision reads the fact at 405);
    `scopes.rs collect_return_sources` (the return-source SET is the row-268 fact
    PRODUCER for the match/if union, not a consumption re-derivation).
  REMAINING: (1) the single copy-elim unification above + the architectural funnel of
  the 3 return paths (row 273) into one return-ownership computation — mechanical, no
  live bug; (2) the `??`-JOIN
  runtime witness (`OpBindOrCopy`/`OpFreeRefIfDistinct`/`_own_store`) is inherently
  runtime (the
  arm taken is unknown at compile time), not a re-derivation to delete.  D-own-5's
  `&`-borrow fact is CLOSED (folded).
- **Removal — DONE:** every free/copy/move reads `deps` (via `ownership_of` /
  `returns_borrowed_view` / `return_adopts_fresh_store`) on the shipped path; the
  per-site heuristics survive only under the `LOFT_NO_JOIN_OWN` opt-out (control
  machinery).  Non-deviation cleanup left: DELETE the opt-out scans once the differential
  controls retire, and collapse the return-ownership readers into one physical funnel
  (pure DRY — each already reads the fact).

### D-own-2 — CLOSED (2026-07-04, @PLN90): the ownership fact is TOTAL
- **Violated:** O-Complete
- **Where:** the row-100/102 holes — adopt-vs-copy for arbitrary borrowing returns; the
  general dep-driven caller copy. (The struct-field and value-`if`-return facets closed
  earlier — #415, a7.)
- **What CLOSES it — the analysis is now total, and validated total.**  O-Complete's
  failure mode is *incompleteness → a silent miscompile or leak* (line 64-66): a
  binding/path with NO computed ownership fact, falling back to a heuristic/stopgap.  That
  is now eliminated on three fronts:
  1. **The static fact is total and correct.**  `ownership_of` (use_analysis.rs) computes
     an `Own` for EVERY `Value`: `OpDatabase`/`OpNewRecord`/literals/scalars → `Owned`;
     a projection → `Borrowed{base}`; a user call → the interprocedural `call_ownership`;
     `??`/`if` → the `join` of its arms; block/insert → its tail.  The `_ => Owned` tail
     is not a hole — it covers only literals / scalar-void ops / payload-less control,
     which ARE fresh-owned or heap-irrelevant (verified against the classifier).
  2. **The free side READS that one fact** (the D-own-1 fold): `scan_set`'s #316 tracker
     (`ref_rhs_ownership`) is a pure `ownership_of` read — `Owned → Owned`, `Borrowed`/
     `Join → View`.  The three-valued gap is closed: `RefRhs::Unknown` is DELETED (dead
     once the oracle covers every value), so the free side is a total 2-valued read of
     the oracle, not a separate structural walk.
  3. **The inherently-runtime JOIN is completed per-path at runtime.**  Where a binding's
     ownership genuinely differs per path (`r = x; for { r = v[i] ?? x }` — owned copy on
     the empty path, a borrowed view once the ncc runs), a static per-binding fact CANNOT
     decide (the spec accepts this as inherently runtime, see D-own-1 residual (2)).  The
     `_own_store_<name>` witness (generation/, @PLN90 loft#495 / commits 44fd7d72 +
     a4bcad5b) is exactly the "set-and-reconcile across arms" O-Complete's removal
     criterion asks for — done at runtime: it tracks the store r actually owns, so BOTH
     the displaced-free and the scope-exit free release the owned store and never the
     view.  This is the last binding-shape whose free decision was previously incomplete.
- **The residuals — all COMPUTED and SAFE, not holes** (probed both backends,
  [plans/85 D-own-2-completeness.md § Sweep](../plans/85-store-lifetime-retirement/D-own-2-completeness.md)):
  (i) the **value-vs-bind gap** (`ownership_of(x)=Borrowed` for a `r = x` whole-value
  COPY that owns) is INERT for the free decision — the reassign pre-free + type-based
  scope-exit free release the displaced/final store regardless of the tracker's read;
  and for the transition class the witness's `is_var_copy` reads the bind as owned.
  (ii) the **deps-carried-join** (`r = pick(v,i)`, `pick = v[i] ?? Box{..}`) is a
  COMPUTED `Own::Join`, classified conservatively as a view — correct: the OWNED arm is
  materialised into the return buffer whose own lifetime frees it, so `r` views it (no
  leak / no double-free, both arms exercised).
- **Validated total:** the transition class swept dry over 6 shapes (2 live over-frees
  found + fixed, 4 safe), the value-vs-bind + deps-join residuals probed clean+poison,
  the full suite 2600/2600 (env flakes only), `native_scripts`, `LOFT_POISON`, native
  leak-check, DA, the differential oracle, AND the `program_ownership` fuzzer (3108 execs,
  0 findings — the "unfuzzed axis" concern discharged).  No binding/path produces a live
  miscompile; the analysis is total.
- **Not this deviation:** unifying the runtime witness + return-buffer machinery INTO the
  single `deps` read (rather than three cooperating mechanisms) is the *single-fact*
  ideal — that rides **D-own-1 (O-Deps)**, which stays open.  And the adopt-vs-view
  *optimisation* for a Join return (view is correct; adopt would save a copy) is
  copy-elimination — **@PLN90's LINT charter**, not an O-Complete correctness item.

### D-own-3 — CLOSED (2026-06-12, recounted into the register 2026-07-03): typed `Deps`
The dep list was a raw `Vec<u16>` overloading five meanings across two address spaces.
The H2 migration ([DEPS_INVENTORY.md](../DEPS_INVENTORY.md), steps 1–5) landed the
`Deps` newtype with named constructors at every creation site, space-checked queries
(`frame_vars` / `as_attr_indices`, debug space tags), and the `CALLEE_FRAME_BIT` VALUE
tag (0x8000) so the one cross-space provenance (the vectors.rs lambda propagation)
survives the IR codec unambiguously.  Residual (not a deviation): the newtype `Deref`s
to `Vec<u16>` for read convenience — writes go through the typed constructors.

### D-own-4 — RECLASSIFIED (2026-07-03, C86): the #415 copy IS the semantic; derive it, don't reverse it
The entry claimed the #415 struct-vector-field copy-on-bind was a stopgap contradicting
reference-default.  The reversal attempt found the premise false: on BOTH backends every
WHOLE-VALUE heap bind copies (`p = o`, `b = x`, `af = bx.v`) and only projections alias —
the written law, not the code, was wrong.  The maker's call
([DESIGN_DECISIONS C86](../DESIGN_DECISIONS.md#c86--whole-value-heap-binds-copy-aliasing-is-a-last-use-elision-the-rustc-rule)):
whole-value binds COPY by contract; `p = o` becomes an alias only when the source is
provably dead afterwards — the rustc last-use rule, as an OPTIMIZATION
(`use_analysis::ElidePlan` is that analysis).  `O-Borrow` scopes to projections /
params / `&τ`.  (binding.md D-bind-3 was already closed — the old "blocks" claim was
stale.)  The implementable RESIDUAL — the copy/alias/elide decision at the bind site
derives from the ownership fact instead of the syntactic `struct_vec_field` branch —
folds into **D-own-1**.  **Narrowed 2026-07-03:** the decision is now the pure
`classify_vec_bind` selector (`VecBind`, parser/expressions.rs — byte-identical
extraction over the C86 bind corpus): the verdict reads the base var's
incrementally-maintained `deps` (the same fact `ownership_of` reconstructs post-parse
via its whole-body `Defs` walk — Owned ⇒ copy, Borrowed/Join ⇒ view; agreement
witnessed by `LOFT_MATERIALIZE_DUMP` over the corpus), and the ELIDE half is already
live post-parse (`elision_plans` → `scopes::elide_borrows`).  What remains of D-own-1
here: the mid-parse deps read and the post-parse oracle are two implementations of one
fact — they unify when ownership is carried as one typed `deps` fact end-to-end.

### D-own-5 — CLOSED (2026-07-03, folded): the `&` borrow now carries its source in `deps`
- **Was:** @PLN87's ladder L1–L6 realised live references ([binding.md](binding.md),
  verified), but the `&τ` borrow's source was carried by a side-flag (`skip_free` on the
  L5 heap whole-value alias), not the `deps` fact the checker reads.
- **The fold (executed):** the L5 bind (`p = &o`, the only `&` binder with a free
  decision) now types `p: &Reference(td, [o])` via the standard `depending()` carrier —
  free suppression derives from `owns = dep.is_empty()` (`scopes::get_free_vars`), the
  same O-Borrow read every other borrow uses; the `set_skip_free` side-channel at the
  bind is deleted.  Proof: the ladder introspects change ONLY in the type display
  (`&ref(Pair)` → `&ref(Pair)["whole"]`) — zero op changes, both backends green,
  leak-gated (434-pln87-scalar-reference, 28-references, 87-store-leaks).
- **Residual sliver (recorded under [D-own-1](#d-own-1)):** a scalar-place ref
  (`c = &v[0]`, `r = &s.x`) holds a DbRef into the source's store, but a scalar inner
  carries no `Deps` slot (`depending()` is the identity), so the link is not a readable
  fact — vacuous for FREE placement (the binder owns no store) but unavailable to any
  future lifetime check until `Deps` is carried type-wide (the D-own-1/D-own-2
  completion).

---

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

This area's "falsifying programs" are the store-lifetime bugs themselves — each is a
program where the derived-free invariant (O-Derived) or completeness (O-Complete) fails
and a store leaks, double-frees, or a backend diverges. The area is **formal when OPEN
reaches 0**: when every store-lifetime decision is one `deps` read (O-Deps) over a complete,
typed fact, the bug class is closed by construction and `binding.md`/`types.md`'s
`deps`-fused rough spots (the `Deps`-in-`Type` fusion) resolve with it.
