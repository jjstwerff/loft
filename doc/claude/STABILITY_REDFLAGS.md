<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# STABILITY_REDFLAGS.md — the re-derived facts a stable future must compute once

> **This is a forward-stability map, not a fix-now list.** It names the
> implementations that will keep manufacturing bugs, grouped by the **missing
> fact** each one re-derives — so the path to a stable future is "land the fact
> once; N forests collapse together," not "patch N sites." Companion to
> [STABILITY_HOTSPOTS.md](STABILITY_HOTSPOTS.md) (the H-register) and
> `OWNERSHIP_MODEL.md` (the ownership holes table); this doc is the cross-cutting
> *red-flag* view those two are read through. This map is scoped to
> runtime/memory/codegen; the FRONT-END rough spots it structurally misses — the
> typing/conversion relation (where #432/#433 live) and grammar precedence — are in
> [FORMALIZATION.md](FORMALIZATION.md) (the formal-definition-as-lens companion).
>
> **Two referenced docs — `OWNERSHIP_MODEL.md` and `CODEGEN_METHOD.md` — are the
> ownership-migration docs from the active store-lifetime plan (`@PLN85`) and are
> not yet merged into this tree; they are referenced by name (no local link) until
> they land here.**

## The one thesis

Per `CODEGEN_METHOD.md`: **codegen re-deriving a non-local fact
(ownership, transfer/borrow, null-encoding, container layout) per call-site is a
diagnostic of a missing type-system fact — fix the fact, not the generator.** A
red flag here is therefore not "ugly code"; it is a *fact computed in the wrong
place / N times*. The test for every entry below is the method's own:

> **Would this code collapse to a simple read if the type carried one more fact?**
> If yes → the fact belongs on the type, and every re-derivation site is a sibling
> of one bug class.

**The goal this doc serves is generalization, not tidiness.** Every duplicated case
analysis is a place a situation can be missed, and the misses are not distributed
evenly: they land on the case the newest duplicate forgot. So the unit of work is
never "patch the arm that was wrong" — it is "make this the only place the cases are
enumerated, and make the enumeration exhaustive so the compiler names the next
missing case instead of a user finding it." Two shapes recur:

- **one fact, N spellings** — the same question answered by several predicates at
  different depths (Cluster F), or the same per-kind table re-encoded per consumer
  (Clusters C, D);
- **a total walk written as a partial one** — a traversal that must reach every
  node, spelled with a wildcard, so the nodes it forgets fail silently
  (Cluster F's second half, loft#815).

The counter-test matters as much as the rule: a walker that is *deliberately*
partial is correct and must stay. What is wrong is that deliberate and accidental
are spelled the same way, so nothing can tell them apart.

**The deliverable is the collapsed structure, not a list of the bugs it was
hiding.** A repro in this doc is evidence that a duplicate is load-bearing — it is
not a ticket, and enumerating the remaining symptoms is not progress. This follows
the [STABILITY_ROADMAP](STABILITY_ROADMAP.md) standing rule (*in stability work,
bugs get FIXED, not filed*) and states its reason: the bugs that matter most here
are the ones nobody has hit yet, and those have no ticket to file. Collapsing the
duplicate retires the discovered and the undiscovered cases together, which is
precisely what patching the arm that was wrong does not do. So the measure of
progress is **how many places can still miss a case**, not how many issues are
open.

The rigor failure modes ([engineering-rigor skill](../../.claude/skills/engineering-rigor/SKILL.md)):
**under-reach** (N mechanisms for one family — a spray), **over-reach** (one
mechanism forced over N families — a false invariant), **wrong-signal** (a decision
keyed on a type where the authority is a runtime delta), and
**defensive-at-manifestation** (guard the symptom, leave the root).

**Survey provenance:** four parallel read-only rigor-audits, 2026-06-21, against the
`../loft` `plan-85-store-lifetime-retirement` tree; line numbers below are grounded
in **this** tree (loft2, `tuxedo-work2`) where confirmed, else cited by function.
The *patterns* are durable; re-confirm exact lines before acting.

**Re-survey 2026-08-20** (loft2, `tuxedo-post-973`): clusters A–E re-measured by
counting their re-derivation sites, and one new cluster added (**F**). What each
count is and how to re-run it is in [§ Re-survey](#re-survey-2026-08-20--what-moved)
below.

---

## The map — six clusters, ~4 missing facts

| Cluster | Missing fact | Re-derivation sites | Failure mode | Tracking |
|---|---|---|---|---|
| **A** Return/bind ownership | return-dep: owned/transferred vs borrows-{source} | ~10 | under-reach + wrong-signal + divergence | OWNERSHIP_MODEL 99/102/103/104 (mostly OPEN) |
| **B** Stack-balance signal | net stack delta (not the last-expr type) | 2 (+1 fixed) | wrong-signal | NEW (gen_if `null_else` already fixed; siblings open) |
| **C** Container taxonomy | a `for_each_owned_child` traversal keystone + per-kind descriptor | ~27 + ~20 | under-reach (per-kind spray) | **NEW** (H7-sibling on the heap-cascade side) |
| **D** Null-sentinel codec | the `sentinel(tp)` encode/decode table (H6) | ~3 sites × 3 | under-reach (one fact, drifting copies) | H6 (consumers unconverged) |
| **E** Manifestation guards | — (downstream of A) | 4 | defensive-at-manifestation | #405 / @P290 / @P317 / @P377 |
| **F** Type-variable fact | *does this type still mention a type variable?* — one recursive question over `Type` | 3 spellings / 14 sites | under-reach + divergence | **NEW** (the loft#1014/#1016/#1020/#1023/#1025/#1028 family) |

Clusters **B/E are mostly *consequences*** of A and C: a missing ownership fact is
why the stack and the free-path get guarded at the symptom. Landing **A** (return
ownership) and **C** (the traversal keystone) is what retires the bulk — D and E
largely dissolve behind them.

### Re-survey 2026-08-20 — what moved

Each row is a count anyone can re-run, not a judgement. The commands are the ones
that produced the numbers.

| Cluster | 2026-06-21 | 2026-08-20 | Evidence |
|---|---|---|---|
| **A** Return/bind ownership | ~10 sites; `has_ref_params` at 11 | **largely landed** — `has_ref_params` at **2** | `grep -rc has_ref_params src` |
| **B** Stack-balance signal | 2 open siblings | **unchanged — deliberately deferred**, no RED probe fires | `state/codegen.rs` arm-join + `stack.rs` `size_code` (below) |
| **C** Container taxonomy | ~27 + ~20, no keystone | **keystone landed**, 24 references; one holdout | `grep -rn for_each_owned_child src` |
| **D** Null-sentinel codec | 3 consumers × 3 widths | **largely landed** — `is_null_field` gone, width fact now `IntegerSpec::range_to_width` | `grep -rn range_to_width src/data.rs` |
| **E** Manifestation guards | 4 | not re-measured (gated on A) | — |
| **F** Type-variable fact | — | **new, 3 spellings / 14 sites** | § Cluster F |

Two clusters therefore came off the critical path, and the landing order below is
re-cut around that. The instrument that found **F** — group every `match` block by
the enum it dispatches on, then rank enums by how many independent blocks re-match
their arm set — is worth re-running on any enum that grows a variant.

---

## The evidence — what the tracker says (2026-08-20)

Everything above this section is argued from reading code. This section is the
independent check: the same question asked of the 334 closed `bug` issues, which
know nothing about the clusters. It is here because a map of "brittle places" is a
claim about where bugs COME FROM, and that claim is measurable.

### Method (re-runnable)

```bash
gh issue list --state all --limit 1200 --json number,title,labels,state,closedAt
```

Keep the `bug`-labelled issues, classify each by keyword against its TITLE (these
titles state mechanisms, which is what makes this work), and bucket by ISSUE NUMBER,
not close date. The tracker opened around #134 in June 2026 and 282 of 513 closes
land in August, so calendar recency cannot separate "still firing" from "closed
during the August push" — issue number can.

**Bounds on the claim, so it is not over-read:** the classes are keyword-matched and
OVERLAP (one issue can count in several); issue number is a proxy for time; and a
rising share can reflect where dogfooding went rather than where the code is weak.
That last confound matters and is picked up under *Exposure* below.

### Result 1 — the classes that got a single-fact home are the classes that stopped

Share of bugs in each issue-number band:

| class | #100–400 | #400–700 | #700–900 | #900–1030 | |
|---|---|---|---|---|---|
| **narrow-int / width** | 8.7 % | 7.7 % | 7.4 % | **2.0 %** | ↓ after `IntegerSpec::range_to_width` (Cluster D) — a real before-population, so this one is measurable |
| **keyed collections** | 0 % | 4.6 % | 14.0 % | **7.8 %** | appeared only AFTER `for_each_owned_child`; no before-population, so no payoff can be claimed |
| **tuple** | 0 % | 0 % | 9.9 % | **9.8 %** | ↑ no keystone |
| **generic / monomorph** | 0 % | 0 % | 1.7 % | **4.9 %** | ↑ no keystone |
| **null / sentinel** | 13.0 % | 6.2 % | 5.0 % | **17.6 %** | ↑ |
| ownership / free | 8.7 % | 12.3 % | 14.9 % | 13.7 % | flat (Cluster A substrate, still open) |

**One of these is a measured payoff and one is not, and the difference matters.**
`narrow-int/width` had a real bug population BEFORE `IntegerSpec::range_to_width`
landed (9.6 % across the earlier bands) and 2.0 % after — a fall with something to
fall from. `keyed collections` shows 0 % before `for_each_owned_child` and ~10 %
after, so the class only came into existence once those collections were exercised:
the fold cannot be credited with a reduction there, and `scripts/bug-review.py`
deliberately ABSTAINS on it rather than printing a verdict the data does not carry.
An earlier draft of this section claimed both as payoffs; that was reading a
within-period dip as causation.

So the honest form of the claim is narrower and still worth having: **where a class
had a bug population and then got a single-fact home, the population fell by roughly
two thirds.** One instance is not a law, and the protocol in
[BUG_REVIEW.md](BUG_REVIEW.md) exists to accumulate more of them.

The classes rising instead are tuple, generic and null, which is
[Cluster F](#cluster-f--is-this-type-still-a-type-variable-new-2026-08-20) arrived at
from the other direction. Nothing here was tuned to produce that agreement: the
clusters came from reading `match` blocks, the shares from reading issue titles.

This is the argument for the whole approach, stated as a measurement rather than a
principle: **collapsing a duplicated case analysis moves the bug rate, and patching
arms does not.**

### Result 2 — a hypothesis this data KILLED

The obvious reading of 22 tuple bugs is *"`Tuple` is the variant enumerations
forget."* It is wrong, and being wrong is the useful part. Omission rate of each
child-bearing variant across the 115 partial (wildcard, non-delegating) recursive
`Value` walkers:

```
Block   9.6 %   Insert 14.8 %   Call 23.5 %   If 26.1 %   Return 27.8 %
──────────────────────────── cliff ────────────────────────────
Iter   65.2 %   CallRef 67.8 %   BreakWith 68.7 %   Yield  71.3 %
Tuple  72.2 %   Parallel 74.8 %  TuplePut  79.1 %   ParFor 87.0 %
```

`Tuple` is not special. It is ORDINARY for the tail. Coverage falls off a cliff
after the five shapes an author holds in mind while writing a walker, and everything
past that cliff is omitted at roughly the same rate. So the defect is not a variant
anyone dislikes — it is that a hand-written enumeration reproduces its author's
attention, and attention has a short tail.

### Result 3 — exposure, and the prediction it makes

If omission rate were the whole story, `ParFor` (87 % omitted) would carry more bugs
than `Tuple` (72 %). It carries almost none:

| variant | omitted | bugs filed |
|---|---|---|
| `Tuple` | 72 % | **22** |
| `Yield` (coroutines) | 71 % | 9 |
| `Parallel` / `ParFor` | 75–87 % | 6 |
| `TuplePut` | 79 % | 4 |

**Exposure = omission rate × usage.** The tail is not safe; it is unexercised.
`par` appears in 12 `tests/scripts/` files against tuple's 51, and the tuple bug
share was itself 0 % before issue #700 — tuples did not become fragile, they became
USED. This is also where the dogfooding confound from the Method note lands, and it
does not weaken the reading: exposure IS the mechanism.

The falsifiable prediction: **`Parallel`, `ParFor`, `Yield` and `TuplePut` are the
next `Tuple`.** When a consumer leans on `par` or coroutines the way one leaned on
tuples, expect a bug wave of the same shape and roughly the same size. If that wave
does not arrive after real `par` dogfooding, this model is wrong and should be
rewritten.

It also settles how to respond. Adding a `Tuple` arm to N walkers treats one point
of a distribution and leaves the rest of the tail loaded. The cure is the one the
tree already has: a walker that must be total delegates recursion to the keystone,
so the tail is never enumerated by hand at all.

---

## Cluster A — the return/bind transfer-vs-borrow fact (cluster-II root)

**The fact:** every heap value's binding should read one carried answer — *does this
value OWN its store (move it), or BORROW another's (copy on bind/escape)?* Today
that answer is re-derived from accreting heuristics (`has_ref_params`, `is_argument`,
`vector_bound`, `dep.is_empty()`, runtime store-nr witnesses) at **~10 independent
sites**, which is why the same class produced #405/#406/#409/#410 this cycle and why
the `a = x.v` field-read aliasing was still open after the `return v` fix.

Re-derivation sites (loft2 tree):

| Site | loft2 location | Smell | Mode |
|---|---|---|---|
| BlockTail/MidReturn/native-forwarder return thicket | `src/parser/control.rs` (`block_result`, `ref_return`) | per-callee-shape adopt/copy/borrow arms + a `RetSite` fork | under-reach |
| `returned_var` single-`u16` walk | `src/scopes.rs:2285` | differing match/if arms collapse to "no return var" → arm buffers freed | **over-reach** (false invariant: ≤1 return source) |
| return-save-to-temp forest (B5-L3) | `src/scopes.rs` (`free_vars`) | ~5 type-keyed save-temp shapes each deciding "hoist before frees?" | under-reach |
| `has_ref_params` adopt-vs-copy | `src/` — **11 sites** (`grep has_ref_params`), incl. `state/codegen.rs` ×2 + `scopes.rs` | a heuristic standing in for "transfer or borrow?"; admits "cannot resolve statically" → runtime `OpFreeRefIfDistinct` | wrong-signal |
| `is_borrowed_view` — computed **twice, divergently** | `src/state/codegen.rs:1716,2441` (interp) **vs** `src/generation/dispatch.rs:178` (native) | the same `0x8000` source-free bit, two different structural derivations → drift on the OOB/hidden-only edge | **divergence** (H4) |
| bind-site owned-vs-borrow | `src/parser/expressions.rs` (`assign`) | per-RHS-shape copy decision (bare Var / borrowing Var / field-read / non-Var) | under-reach |
| reassign free-strategy forest | `src/state/codegen.rs` (`gen_set_first_at_tos` region) | `is_hidden_buf_arg && owned_ref && rhs_reads_v && …` — the canonical `has_ref_params && …` forest | under-reach |
| scan_set paired-witness | `src/scopes.rs` (`scan_set`) | emits a **runtime** store-nr comparison because the static fact is absent | re-derived → runtime |

**`dep.is_empty()` also HIDES the class from one backend** (measured, loft#882). Empty
deps mean OWNED to `--native`'s assignment lowering, so a read whose dep is *missing*
(not absent-because-owned) gets a defensive `OpDatabase` + `OpCopyRecord` and the program
comes out right; the interpreter aliases and reads freed bytes. The keyed-element borrow
scored `--interpret` 6/17 boundary cells against `--native` 14/17 **on the same IR**. Two
consequences for anyone working this cluster:

- a store-lifetime matrix that is lopsided between backends is evidence of a missing dep,
  not of a codegen bug — read the emitted Rust and look for an `OpCopyRecord` beside a
  plain element read;
- a `--native` PASS is not evidence the dep is present, so every ownership probe has to
  run `--interpret` under `LOFT_POISON=1` as well. The defensive copy is also why this
  class survives a green suite: it is a silent perf cost on one backend and a
  use-after-free on the other.

**Twins drift exactly on the input nothing tests.** `State::append_copy` and
`codegen_runtime::OpAppendCopy` disagreed on a NEGATIVE count (`--native` clamped, the
interpreter cast `as u32` and walked off the store until glibc aborted) and on whether to
re-read the backing record after a resize. A twin gets hardened where its bug was
*observed*, and an observation happens on one backend. When touching one twin, diff it
against the other line by line, and land every new boundary row in the shared `.loft`
guard so both backends run it — a "keep the two in step" comment is not a gate.

**Two owners of one store, and only one of them knows.** A `0x8000` source-free bit means
"nobody else owns this store" — a fact about the *source expression*, decided in the parser.
`scan_args` then lifts an inline call result into a `__lift_N` the scope sweep frees, which
makes the bit false without telling anyone. `free_named` is a no-op only while the slot is
still free, so the second free is silent until the allocator hands that slot to somebody
else — and a record return allocates its buffer in exactly that window (loft#890). Two rules
follow:

- **the FREE hand-off and the DROP hand-off are separate facts** and must be recorded
  separately. @PLN139 stage C recorded only the drop, reasoning that "the free is left to
  the ordinary sweep, which is null-tolerant either way"; a recycled slot is what makes that
  untrue;
- **`skip_free` is not a free-only switch.** Both backends read it at ALLOCATION time too
  (`is_inline_ref || is_skip_free` in `state/codegen.rs`), so stamping it to suppress a free
  made the lift BORROW instead of own, and the append wrote into its own source — 3 of 54
  fuzz cells SIGSEGV'd. A "somebody else already freed this" note belongs in the pass that
  emits the free, not on the variable.

**A verdict that MINTS must be pass-stable, and skipping pass 1 is not "no verdict".** It is
the opposite verdict: `ref_return` reads a binding with no dep as OWNED and renames it onto
the return buffer, permanently. Pass 2 then sees the borrow and materialises — into the
buffer the binding now IS — so `materialize_return_into` emits `OpDatabase(e);
OpCopyRecord(e, e)` and the function answers the record it just re-minted (loft#889;
`e = make()[k] ?? d; e` had it from loft#882 onward). A parse-time site that names an owner
has to name it on BOTH passes; the way to keep the numbering stable is a separate counter
(`__ref_p2_N`), not a pass guard.

**Would-one-fact-collapse-it?** Yes — all of the above are the OWNERSHIP_MODEL
remedy verbatim: *return-dep empty ⇒ adopt; `{Attr(src)}` ⇒ copy* (row 102), plus
the *return-source SET over arms* (row 99) and *one funnelled return path* (row
104). Landing the carried return-ownership dep collapses these ~10 sites and
deletes the runtime witnesses. **This is the single highest-leverage migration in
the tree** (most-reused decision; the documented cluster-II root).

---

## Cluster B — stack-balance keyed on type, not runtime delta

**The fact:** "did this branch leave a value on the eval stack, and how many bytes?"
is a **runtime stack-position delta**, never the last-expression *type*
(`generate_block` reports the last expr's type, not the net push — a value-typed
but stack-neutral tail op like `OpAppendVector` pushes nothing).

- ✅ **Fixed precedent:** `gen_if`'s `null_else` gate now reads `true_stack !=
  stack_pos` (the #405 fix). It is the template for the rest.
- 🔴 **`gen_if` B5 rebalance** — `src/state/codegen.rs:1072` (re-confirmed
  2026-08-20, unchanged): uses
  `size(def(self).returned())` (the *function's* return type) as the bytes-to-
  preserve, not `size(tp)` (the if-**expression's** result type). Wrong whenever a
  non-tail value-`if`/`match` with eval-stack-divergent arms has result type ≠ the
  function return. **Masked** today because its only natural trigger is recursive
  functions where `tp == returned()`; latent, not dead.
- 🟡 **`size_code` If/divergent arm** — `src/stack.rs:113` (`size_code`,
  re-confirmed 2026-08-20, unchanged — the arm reads `self.size_code(node.if_then())`
  and the function ends `_ => 0`): an `if`'s
  drop-size is read from the then-arm's static type; a divergent then-arm falls to
  `0` while the else-arm pushed a value. Same wrong-signal family; uncommon shape.

**Cleared (right signal, not red flags):** fn-ref `step(20)`/`step(16)` and callee
`size(op.returned())` advances — there the type *selects the opcode that pushes
exactly those bytes*, so the type IS the authority.

---

## Cluster C — the per-Type-kind container taxonomy (the heap-cascade keystone)

**The fact:** "to traverse / construct / free a collection's owned nested heap —
which element type, at which stride, with which container walk" is ONE per-`Parts`
descriptor. Today it is hand-re-encoded across the family below — the **densest bug
cluster in the tree** (the @P29x/@P3xx history). <!--noindex-->

| Family | loft2 location | Count | Named bugs from the drift |
|---|---|---|---|
| `copy_claims` / `remove_claims` / `validate_claims` triad | `src/database/allocation.rs:1374 / 1682 / 984` (+ `copy_claims_{seq_vector,array,hash,index}_body` at `:1119/1163/1233/1303`) | 3 dispatchers × ~9 `Parts::` arms ≈ **27**, already drifting (copy has 4 helpers; remove/validate don't) | @P290 (SIGSEGV, `room*2` vs `(room-1)*2`), @P306/@P318 (hash slot-drift), @P309 (missing length header) |
| `record_new` / `record_finish` / `insert_record` construction | `src/database/structures.rs` | 3 fns × ~8 arms; **3 independent encodings** of "element-record word count" | @P309 class |
| `gen_set_first_*_null` codegen family | `src/state/codegen.rs:1091/1143/1303/1378/1390` + the multi-arm `gen_set_first_at_tos` ladder | 5 fns + ladder + 3× copy-pasted `sentinel \| owned-init \| borrowed-view` tri-state | #260/#330 class (wrong null-init → leak / use-before-init) |
| keyed `Type::{Sorted,Hash,Index,Spacial} → database.{kind}` re-dispatch | `src/state/codegen.rs` + `src/parser/vectors.rs` + `src/generation/dispatch.rs` (+more) | same 4-arm block in **≥4 files**, interp/native already shaped differently | interp/native drift (H4) |
| `Stores::{vector,hash,sorted,index,spacial,child_rec}` constructors | `src/database/types.rs` | 6–7 near-identical intern-or-push bodies + **3× key-resolution drift** (`hash` resolves vs `key_owner`, `sorted`/`spacial` vs raw content) | latent @PLN25 nullable-key bug |

**Would-one-fact-collapse-it?** Yes — a single `for_each_owned_child(tp, rec) ->
Iterator<(child, child_tp)>` keystone (the per-`Parts` walk as a carried fact), with
copy/free/validate/construct as thin visitors over it. This is the `for_each_child`
keystone **H7** already names for *codecs* — here on the heap-*cascade* side, and
**not yet a tracked H-row** (H3's pass-2 explicitly scoped itself to `scopes.rs`
free-*placement*, away from this traversal cascade). **Highest-leverage NEW finding.**

**LANDED (re-survey 2026-08-20).** `Stores::for_each_owned_child`
(`src/database/allocation.rs:254`) exists and carries the walk, with 24 references
across `allocation.rs` and `database/spans.rs`. `copy_claims` and its per-kind
bodies, `remove_claims_mode`, and the span walk are visitors over it. Its own
history already shows the keystone earning its keep: `allocation.rs:6364` records a
real fault from it having had no `Trie` arm — one arm, one place, instead of the
same omission repeated per consumer.

**The holdout — `validate_claims`.** `allocation.rs:2116` still hand-rolls its own
ladder: six `Parts` arms (`Base` at `tp == 5`, `Struct`/`EnumValue`, `Vector`/
`Sorted`, `ChildRec`, `Enum`, `Hash`) and then `_ => {}`. `copy_claims` knows
thirteen kinds, so the validator silently walks past **`Array`, `Index`, `Ordered`,
`Radix`, `Trie`**.

Its *divergence* from the keystone is deliberate, documented at `allocation.rs:96`,
and already SETTLED: a 2026-06-22 design probe falsified the wider fold and the
decision is recorded in
[STABILITY_ROADMAP § Red-flag remediation](STABILITY_ROADMAP.md) row C —
`validate_claims` runs on suspected-corrupt heaps and must bounds-check before
following a pointer, where the keystone trusts it. That reasoning holds, it is not
the red flag, and this section does not re-open it. The `_ => {}` is: a validator that answers "no
problems" for a container kind it never looked at cannot tell a clean heap from an
unwalked one, and `store_verify` (`allocation.rs:4640`) is one of its callers. The
fix is not to make it a visitor — it is to make the arm set exhaustive, so a new
`Parts` kind forces a decision here the way it already does in the keystone. An arm
that deliberately does nothing is fine when it says so.

---

## Cluster D — the null-sentinel codec (H6 consumers, unconverged)

**The fact:** the per-width null encoding (`u8`→255, `u16`→65535, `*Raw`→`i32::MIN`,
…) is **one** `sentinel(tp)` table. It is still hand-inlined at read AND write sites
beyond the two H6 already unified:

- `is_null_field` — `src/database/types.rs` (3 inline narrow arms).
- `set_default_value` — `src/database/structures.rs` (the write twin).
- `walk_parsed_into` (JSON path) — `src/database/structures.rs` (a third copy).

This is H6's thesis exactly ("one width-fact, N drifting copies" — already cost the
`389-h6` nullable-narrow bug). Tracked under **H6**; these specific consumers are the
named-but-unconverted ones. Lands behind the staged `NullEnc`/`sentinel(tp)` table.

**Landed — the heap-ref + character facet (Cluster D D.1/D.2).** The four named
typed-null encoders (`write_typed_null` native, `emit_typed_null` interp,
`STRING_NULL`, `init_ref_sentinel`) are converged:

- **Heap-ref null = one source, `DbRef::NULL` (`keys.rs`).** Every heap-ref null
  encoder (native `write_typed_null` / `default_native_value` / `dispatch` / `calls`
  / `coroutine`, the `codegen_runtime`/`parallel`/`structures` runtime writers, and
  the interp `init_ref_sentinel` / `null_ref_sentinel`) read the single
  `DbRef::NULL` const instead of re-spelling `DbRef { store_nr: u16::MAX, … }`. The
  drift was a mixed `pos: 0` (interp/canonical) vs `pos: 8` (native) literal —
  semantically inert (`is_null()` keys off `store_nr`, ignores `pos`) but real byte
  drift, now gone. Round-trip matrix byte-identical on both backends; regression
  `tests/scripts/407-cluster-d-null-sentinel-roundtrip.loft`.
- **Character null H4 cell, found + closed.** `Parser::null` folded `character`
  into `OpConvIntFromNull` (the i64 integer sentinel), so `-> character { return
  null; }` emitted `return i64::MIN` into an `i32` return slot — native rustc E0308,
  interpreter tolerated. Now routes to `OpConvCharacterFromNull` (char-domain null
  `'\0'`), correct on both backends.

**Deliberately LEFT (distinct representations, not the same fact).** `STRING_NULL`
(a text `Str` sentinel `"\0"`, already a single `const` read by all text-null sites)
and the interp `Reference` path's `database.null()` (which allocates a real null
*store*, a different runtime mechanism than the `DbRef::NULL` sentinel) — forcing
either onto `DbRef::NULL` would be a false merge. **Still open (the narrow-width
facet):** `is_null_field` / `set_default_value` / `walk_parsed_into` per-width arms
above — a separate `IntegerSpec::range_to_width` sub-thread, not the heap-ref one.

---

## Cluster E — defensive-at-manifestation guards (downstream of A)

These guard the *symptom* of the missing ownership fact rather than preventing the
bad value. They are correct stopgaps; the stable future **retires** them when A lands
(and a `debug_assert` that documents a contract is the GOOD form — keep those).

| Guard | loft2 location | What it hides | Tracking |
|---|---|---|---|
| `free_named` OOB-refuse | `src/database/allocation.rs:191` | a wrong/stale free with an out-of-range `store_nr` — refused (release) instead of not-produced | #405 (cluster II) |
| `free_protected` call-bracket | `src/database/allocation.rs:680` + `lock_store` | "safety net for `is_borrowed_view` mis-detection" (its own @P290 words) — runtime locking to stop the wrong free landing | @P290 (retire when dep inference is complete) |
| `["??"]` one-buffer return marker | `src/generation/mod.rs:714`, `src/generation/dispatch.rs:173` | a placeholder dep standing in for the unresolved return-ownership fact | OWNERSHIP_MODEL `"??"` row |
| `n_protect_store_frees` `rec != 0` guard | `src/native.rs` (per @P377) | a half-state `free_protected` leak across `unlock`/`init` | @P377 |

---

## Cluster F — "is this type still a type variable?" (NEW, 2026-08-20)

**The fact:** *does this type still mention a type variable?* is ONE recursive
question over `Type`. Three predicates answer it today, each seeing a different
depth, and none of them recurses through the keystone that already exists —
`Type::for_each_child` (`src/data.rs:1783`), whose own doc calls it *"the ONE place
that knows which `Type` variants carry child types … exhaustive on purpose — a new
variant forces a decision here and every walker inherits it."*

| Spelling | loft2 location | Depth it sees | Sites |
|---|---|---|---|
| `is_type_var_placeholder(d_nr)` | `src/data.rs:6309` | none — takes a def-nr, walks no type | 9 raw, each composing its own peel |
| `is_type_var_operand(tp)` | `src/parser/operators.rs:511` | `tp.base()` — peels `Optional`, stops | 2 |
| `type_mentions_tv(t, tv_nr)` | `src/parser/mod.rs:5240` | recursive, but ends `_ => false` | 3 |

The deepest of the three still misses four child-bearing variants the keystone
names: `Iterator`, `Function`, `RefVar`, `Rewritten`. The middle one additionally
misses `Tuple` and `Vector`. So a type that plainly mentions `T` can answer *"not a
type variable"*, and the decision keyed on that answer is then made against the
placeholder.

**Why this manufactures bugs.** A template's `T` is an attribute-less placeholder
STRUCT. Any parse-time decision keyed on `τ` — which null sentinel to write, which
null test to emit, which default to construct — picks the REFERENCE answer for it
and the monomorph keeps that already-chosen op after substitution rewrites only the
type. loft#1014, #1016, #1020, #1023, #1025 and #1028 are all that one shape.

The cure applied so far is per-site and correct as far as it goes: the template
stamps a marker (`TV_DEFAULT_BLOCK`, `TV_NULLTEST_EQ`/`_NE`, `TV_NULL_BLOCK`) and
`rewrite_generic_type_defaults` re-asks once `T` is concrete, so there stays exactly
one spelling of *"what is `τ`'s null?"*. But it is one marker per DECISION. The
markers arrived one per issue (`TV_DEFAULT_BLOCK` with #1016, the null-test pair
with #1020, `TV_NULL_BLOCK` with #1028), which is the signature of a fact being
retrofitted rather than carried.

**Reproducing, both backends, refused identically:**

```
fn f<T>(x: T) -> (T?, integer) { (x, 1) }

error: type layout: __nullable<T>::Some: field 'payload' has no position (u16::MAX)
```

Boundary matrix, both backends agreeing on every cell. Two axes are swept: what
HOLDS the `T?` (rows 1–5), and then, once the tuple was implicated, everything about
the tuple that could plausibly matter (rows 6–11).

| shape | `--interpret` / `--native` |
|---|---|
| `-> T?` | ok |
| `-> vector<T?>` | ok |
| `-> (T, integer)` — no `?` | ok |
| `-> (text?, integer)` — concrete nullable, still from a generic | ok |
| `-> (T?, integer)` | **refused** |
| `fn f<T>(x: (T?, integer))` — parameter position | **refused** |
| `-> (integer, T?)` — element position 1 | **refused** |
| `-> (T?, T?)` — two mentions | **refused** |
| `-> (integer, integer, T?)` — arity 3 | **refused** |
| `T` ∈ {text, integer, float, character, boolean, struct, vector} | **refused, all 7** |

**Boundary: `Tuple` containing `Optional(<type variable>)`** — any element position,
any arity, any `T`. Both halves are load-bearing, and rows 3 and 4 are the controls
that prove it: drop the `?` and it passes, make the nullable CONCRETE and it passes.
The type axis is inert because the refusal happens at LAYOUT time, before `T` is
substituted at all.

*What this matrix still holds FIXED* (per CLAUDE.md § Debugging policy — count the
axes you pinned, not the ones you swept; `tuples.md`'s `OPEN: 0` and
`ownership.md`'s D-own-6 are both worked instances of a corpus reading clean because
of an axis nobody moved): the generic has exactly ONE type parameter, the tuple is
never nested inside another tuple, and the first sweep of rows 1–5 used `T = text`
in every cell — rows 10 fixed that particular hole afterwards, and it turned out
inert, but it was a hole when the first four rows were written.

`substitute_type` has both a `Tuple` arm and an `Optional` arm (each retrofitted
after its own bug — read their comments), so the gap is the COMPOSITION, not either
arm. Confirmed on `tuxedo-post-973`; `type_mentions_tv` and `is_type_var_operand`
are byte-identical on `origin/main`, so the structure is not branch-local.

**Deliberately not filed.** It is here as the probe that proves the duplicate is
load-bearing, and it is expected to fall out of step 1 below rather than be fixed on
its own. Filing it would re-pay the derivation later and would fix one cell of a
matrix whose other cells nobody has run yet — the whole reason this cluster is worth
collapsing is the cases still undiscovered behind the other two spellings.

**The second half — the marker dispatcher is itself a partial walk.**
`rewrite_generic_type_defaults` (`src/parser/mod.rs:6676`) has to be TOTAL: its job
is to reach every stamped marker in a body. It descends ten `Value` variants and
ends `other => other`. `IrNode::for_each_child` names seventeen child-bearing ones,
so a marker sitting inside `Tuple`, `TuplePut`, `Parallel`, `ParFor`, `Iter`,
`BreakWith` or `Yield` is never rewritten, and the monomorph ships the placeholder.

**Measured, and it is SILENT.** The deferred `x?` default of loft#1016, placed inside a tuple,
is never rewritten, and the monomorph reads the placeholder's bytes as data:

```
pub fn ctl1016<T>(v: vector<T>, a: T? = null) -> T { _ = len(v); a? }
pub fn tup1016<T>(v: vector<T>, a: T? = null) -> T { _ = len(v); t = (a?, 1); t.0 }

control (no tuple): 0                 <- correct: the instantiation's zero
probe   (in tuple): 34359738369       <- the placeholder, read as an integer
Warning: 1 stores not freed at program exit: kt=9 __typevar_T x1
```

No diagnostic, no refusal, exit 0. That makes this half of Cluster F **`silent-wrong`** — the
freeze axis ([.github/LABELS.md](../../.github/LABELS.md)) — rather than the mere refusal the
`(T?, integer)` matrix above shows. A refusal can be frozen into the contract; an answer that is
quietly wrong cannot.

That missing set is very nearly loft#815's
([STABILITY_PASS2.md § The `IrNode` keystone](STABILITY_PASS2.md), which lists
`Tuple`, `Parallel`, `BreakWith`, `TuplePut`, `ParFor`) — the same variants absent
from a new total walker written after that lesson was recorded and guarded. The #815
guard asserts closure of the REACHABLE set, so it cannot see a rewrite walk. The
lesson generalises: **a keystone protects only the walkers that adopt it, so new
total walkers need the same audit as old ones.**

**Scale of the surrounding habit.** Counting recursive `Value` walkers with four or
more arms: **153** in `src/`, of which 19 delegate to `Value::for_each_child`, 19
are exhaustive with no wildcard, and 115 end in a wildcard. That 115 is *not* 115
defects — most are intentionally partial, and rightly so (`arm_is_null()` should
answer `false` for everything else). The red flag is that intentional and accidental
are spelled identically, so neither review nor the compiler can separate them. The
cheap discipline is the one PASS2 already prescribes: a walker that must be TOTAL
delegates recursion to the keystone and keeps only EXTRACTION arms of its own.

**Would-one-fact-collapse-it?** Yes, in two small steps:

1. Add `Type::mentions_type_var(&self, data) -> bool`, recursion delegated to
   `Type::for_each_child`, and collapse all three spellings onto it. A new `Type`
   variant then extends one exhaustive match instead of drifting three predicates.
2. Convert `rewrite_generic_type_defaults` to the extraction/recursion split: the
   `TV_*` arms extract, everything else delegates to the keystone.

Both are the PASS2 keystone method applied to code that POST-DATES PASS2 — these
walkers are newer than that doc's work list, which is why they are absent from it.

### Three instruments, independently, land on the same shape

This cluster was reached three different ways, and none knew about the others:

| instrument | what it saw |
|---|---|
| reading `match` blocks | `Tuple` in the forgotten tail of 115 partial walkers |
| the tracker (§ The evidence) | `tuple` the top RISING class, 0 % → 10.7 %, no keystone |
| a differential review of this branch's diff | **5 of its 15 findings** in the tuple / ref-tuple family |

The review's five were the marker walk above, a call-argument helper whose sibling gained a
`TupleGet` arm *in the same PR* while the mirror did not, and three sites of the ref-tuple
admitted-element set. That last re-opened a formal deviation the same branch had closed that
day — [formal/tuples.md](formal/tuples.md) D-tup-2: the element rule has ONE list, and only one
of the two sites that construct a `&(…)` asks it.

Agreement across three instruments is worth more than any one of them, because their failure
modes do not overlap: a code read finds shapes nobody has hit, the tracker finds what users
actually hit, and a diff review finds what was added last week. When all three name the same
shape, the shape is not an artefact of how you looked.

---

## Landing order (leverage-first — the stable-future roadmap)

> **Site-level steps + per-step verification gates for each cluster below:**
> [STABILITY_REDFLAG_REMEDIATION.md](STABILITY_REDFLAG_REMEDIATION.md) — the
> actionable *how* to this map's *what*.

Re-cut 2026-08-20, after A and D largely landed and C's keystone shipped.

1. **Cluster F — `Type::mentions_type_var` on the `Type::for_each_child`
   keystone**, with all three spellings collapsed onto it. Now the most-reused
   under-generalized decision in the tree, and the only cluster with a live
   both-backend repro and a six-issue history behind it. Small: one predicate plus
   its call sites. Step 2 (the `rewrite_generic_type_defaults` extraction/recursion
   split) is independent and equally small.
2. **Cluster A — carried return/bind ownership dep** (OWNERSHIP_MODEL 99/102/104).
   Still the deepest fact, and `has_ref_params` is down from 11 sites to 2, so the
   remaining work is the carried dep itself rather than the forest around it.
   **Dissolves Cluster E** and the `is_borrowed_view` divergence.
   *Prereq already laid:* typed `Deps` (H2 / DEPS_INVENTORY.md).
3. **Cluster C's holdout — make `validate_claims`'s arm set exhaustive.** XS, and
   it closes a diagnostic that currently reports clean on five container kinds it
   never walks. Keep its deliberate divergence from the keystone; delete only the
   silent `_ => {}`.
4. **Cluster B — apply the `true_stack`-delta template to the `gen_if` arm-join +
   `size_code`.** Small, bounded, unchanged since the first survey; the #405 fix is
   the worked precedent. Both sites are re-confirmed present, but the ROADMAP row
   deferred this cluster deliberately ("unverifiable, no RED probe fires; latent —
   pick up only on a real trigger"), and the re-survey found no such trigger. Listed
   here so the sites stay findable, not to re-open the decision.
5. **Cluster D — converge the remaining per-width consumers** onto
   `IntegerSpec::range_to_width`, now that the width fact has a home. S-sized; any
   gap.

Each is a *single fact computed once*, validated on **both backends** per
CODEGEN_METHOD — not a patch. The win is structural: a new collection kind / return
shape / narrow width then arrives *with its fact*, not as the next special case.

## What is NEW vs already-tracked

- **NEW (file a forward home / H-row when picked up):** **Cluster F** — the
  type-variable fact and the partial marker-rewrite walk (2026-08-20 re-survey),
  carrying the one reproducing defect in this doc (`-> (T?, integer)`); and the
  Cluster-B arm-join / `size_code` wrong-signal siblings, still open.
- **LANDED since the first survey:** the Cluster-C `for_each_owned_child` keystone
  (shipped, 24 references — `validate_claims` is the one holdout), Cluster A's
  `has_ref_params` forest (11 sites → 2), and Cluster D's width fact
  (`IntegerSpec::range_to_width`; `is_null_field` gone).
- **Already tracked (this doc is the cross-cut, not a new filing):** Cluster A →
  OWNERSHIP_MODEL holes 99/102/103/104 + H3; the `is_borrowed_view` divergence → H4
  + DEPS_INVENTORY H2; Cluster D → H6; Cluster E → #405/@P290/@P317/@P377.

No bugs filed: open items map to existing OWNERSHIP_MODEL/H rows; the genuinely-new
ones are structural-debt forward risks (this doc), not `main`-reproducing defects.
