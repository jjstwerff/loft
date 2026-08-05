<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 130 — A copy nobody is told about, and a view that outlives what it names

Tracker: [@PLN130](https://github.com/loft-lang/plans/issues/130) · opened from
[loft#774](https://github.com/loft-lang/loft/issues/774).

## Status

| Stage | Status |
|---|---|
| A — Probe catalogue | ✅ 34 probes, both backends. Cluster II matrix COMPLETE: producer + invalidator sets closed, boundary measured |
| B — Mechanism investigation | 🟡 Cluster I verified to a code line; Q5 answered; cluster II F1 mechanism VERIFIED (missing borrow fact on an element-read bind -> pre-Set store-free kills the container) |
| C — Fix design | ⏸️ the model is chosen (below), the enforcement point is not. The Q5 one-liner was tried and reverted — it needs an analysis, not a flag test |
| D — Implementation | ⏸️ F1+F2 designed to one analysis, two triggers; F6 is a doc edit. Closure bar is INFORMATION, not exhaustive correctness — see § Must resolve before close |

loft#774 asked why `b = a` copies while `c = v[0]` aliases. The copy half is **not** the
defect — @PLN90's classifier calls that repro `Forced` (*"source survives AND is written
after — an independent copy is required"*, `use_analysis.rs:1275`), and any probe that can
*observe* copy-vs-alias must write one name and read the other, which is exactly that
condition. The repro cannot reach the aliasing cell.

Two real defects sit underneath it, and they are the same defect seen from two sides —
**loft decides between a copy and an alias silently, and neither answer is checked.**

## Goal

Make the copy/alias decision *stated*: every copy loft makes is either provably necessary
or reported, and every alias it keeps is provably safe for as long as the binding lives —
with the interpreter and `--native` deriving that from one shared fact.

## The model (owner's decisions — design constraints, not open questions)

1. **Alias whenever it is semantically possible; copy only when needed.** The fix may not
   be "copy on every container read" — that trades a correctness bug for a blanket cost.
2. **Follow rustc's ownership model, with loft's ending.** Where rustc *errors* on a
   use-after-move, loft **copies and warns**. The program keeps working; the author is told
   it cost a copy.
3. **The copy diagnostic is default-ON, and there is no global off switch.** A diagnostic
   nobody enables reports nothing — which is exactly the state Stage A measured. It is on
   under normal conditions, always.

   Suppression is an **acceptance in the source**, at **per-file and per-function**
   granularity: the author states *"this copy is intended here"* and the notice goes quiet
   for that scope only. This is deliberately not an env opt-out — an env flag silences a
   whole run and leaves no record of who decided what, whereas an accept is reviewable, and
   scoped to the code it excuses. `#superseded "…"` (@PLN102 arc C, `data_store.rs:313`)
   is the existing per-definition annotation precedent; the per-file form has none yet and
   needs designing.

   Consequence — and this is NOT "drain `Avoidable` to zero first". A copy may be **allowed
   to stand** while it is still eliminable in principle. Three kinds, and only the last
   blocks anything:

   - **Necessary** — the source survives and is mutated; no analysis will ever remove it.
     Stated once, accepted at the site, quiet thereafter.
   - **Allowed for now** — we *could* eliminate it with a better analysis but have not.
     It stays, it is **stated**, and it is tracked as a future-elimination candidate. This
     is the `Avoidable` bucket, and it is a legitimate resting state, not a debt that must
     be paid before the plan closes. `exists()` lives here.
   - **Unknown** — a copy no diagnostic accounts for. Never acceptable: the author cannot
     decide about something they are not told. The manifest guard exists to keep this empty.

   So default-on is livable as soon as the notices are TRUE and actionable — not once they
   are absent. An accept then records a real decision ("intended here"), and an allowed copy
   records a real trade-off, rather than either being noise-suppression.

   **"Allowed for now" covers COPIES ONLY — never wrong behaviour.** A copy that rests is a
   correct program paying a cost. Breakage, misinformation and silent copies are all
   excluded, and a warning does not buy any of them off (§ Must resolve before close).
4. **Both backends, one semantics.** `--native` is the path that must be quickest, so it may
   not be the backend that always deep-copies. Backend parity is in scope here, not a
   follow-up.
5. **Everything is decided at COMPILE time. No runtime checks.** The diagnostic, the accept,
   and the guard are all static. A compiled program carries no copy bookkeeping — when a copy
   is genuinely needed, loft simply performs it at full speed. Consequence accepted knowingly:
   loft can say *where* a copy happens, never *how much* it moved (a deep copy's size is
   runtime data). That is the rustc bargain, and the author knows their own data.

## Method — engineering is an information problem, so build the instrument first

The principle this plan runs on, stated once because every result below came out of it:

> **Most of engineering is information.** Before writing a fix, build the instrument that
> tells you *where* the problem is — one more precise than an oracle — then use it to find
> the real cases and code paths. Only then do you know what to actually write.

### An oracle answers; an instrument localizes

An **oracle** gives one bit about a case *you already constructed*: pass or fail. Its
resolution is bounded by your imagination — it can only speak about shapes you thought to
write down.

An **instrument** reports *where*, continuously, across code nobody wrote a case for. Its
resolution is per-site, over the whole corpus.

This plan opened because an oracle-shaped thing gave a confident wrong answer. On a program
that provably deep-copies, `--report-copies` printed *"none — every structure copy is a move,
a literal, or already borrowed"* (probe 10). Not silence — a clean bill of health. No amount
of writing more test cases against that oracle would have found the copies, because the
oracle was blind to a whole *class*, not to particular inputs.

### Put the instrument where the fact is created

The report walked the **IR**. The copies are minted **later**, during code generation. That
gap is not a bug in the report's logic — it is unreachable by construction, so no careful
reading of the IR could ever have found them.

The manifest instead records at the emitter, *"the one place that cannot be wrong about
whether a copy exists"* — the branch that writes the copy. That relocation is the entire
design, and it is why the guard is compile-time and costs a compiled program nothing.

### An instrument is untrustworthy until calibrated against known answers

Ours was mis-installed **twice**, and code-reading caught neither:

1. `gen_set_first_ref_copy` reads exactly like the call-return emitter. Its own doc records
   **zero fires** in the corpus — a dead path. Probe 11 stayed silent.
2. Only first-bind emitters were instrumented, so **every reassignment copy in the language**
   was missing — including the one in `exists()`.

Both were caught by running the instrument against cases whose answer was already known:
probes 10/11 (known uncovered) and 13/14 (known covered) are the **calibration**, not
decoration. An instrument that has never been made to fire, and to stay quiet, on cases you
already understand is an unread dial.

### Negative results localize; confirmations do not

Probes 15 and 17 were written to *demonstrate* the expression-temp and loop-carried copies.
They measured **zero**. That refutation is what moved the cause from "the syntax at the call
site" to "the callee's return" — and a fix aimed at expression temps would have missed
entirely. Both are kept precisely because they failed to confirm.

A probe that agrees with you teaches almost nothing. Write the one that could prove you wrong.

### Survey turns a bug into a distribution

Pointed at the corpus (583 `tests/scripts`), the instrument answered a question no single
repro can: **exactly one uncovered site per compile, always the same one** — the stdlib's
`exists()` — plus a second family at 107 hits. That is what tells you where the cost actually
is, rather than where the first report happened to land.

### The same information tells you what NOT to write

Q5's answer makes a one-line change to `return_adopts_fresh_store()` look obvious. The same
investigation surfaced the reuse hazard that gates it — and probe 17 is the exact shape that
breaks if that hazard is real. **Knowing the cause is not permission to fix.** An instrument
that only ever argued *for* the change would be a worse instrument.

### The instrument is unfinished, and that is part of the reading

Building one costs real effort, and **ours is not honed to every case yet.** Today it
records **8** sites: the interpreter's five emit paths and native's two (one instrumented
path turned out to be dead). The **parser's ~5 IR-level `OpCopyRecord` emitters**
(`parser/expressions.rs`, `operators.rs`, `control.rs`, `mod.rs`) are **not instrumented**.
Those land in the IR, so the analysis can see them *in principle* — which is why the guard
targets the post-IR emitters first — but "in principle" is not "verified".

So the reading to hold onto: **`uncovered = 0` would mean "nothing on the paths we watch",
never "no blind spots".** A partial instrument reporting zero is the oracle that said
`none` all over again, just with better manners. Extending coverage is remaining work
(Tool gaps), and until it is done every number here carries that qualifier.

This is also the honest limit on the method: the instrument is an **investment**. It repays
when the class is broad, recurs, or hides where an oracle cannot look — as here. For a bug
whose scope and root cause are already pinned, skip it and fix the bug.

### The order, as a checklist

1. Distrust a clean answer from an oracle that cannot see the whole class.
2. Put the instrument where the fact is *created*, not where it is consumed.
3. Calibrate on cases whose answer you already know — both directions.
4. Survey the whole corpus; convert the bug into a distribution.
5. Write the probe that could refute you; keep it when it does.
6. Let the instrument name the gate on the fix, not just the fix.
7. State its coverage beside its readings — an unfinished instrument reporting zero
   is the oracle again.
8. **Always add a probe when a new case turns up** (investigation ground rule). Every
   case found is a case that must stay found; the suite is the only thing that
   remembers. Probe 23 exists because the fix that probes 21/22 blessed was wrong.
9. **Count the axes you held fixed, not just the ones you varied.** Probes 21/22 swept
   call shape while pinning depth, Set-count, parameter kind and caller-count at 1.
   A clean sweep over one axis reads as proof and is not — the cell that broke needed
   all four moved at once.
10. **Propose three or more cases? Write them all down BEFORE starting the first**
    (investigation ground rule). Detail decays while you work: by the time case one is
    done, the reason case three mattered has usually gone, and what gets lost is the
    *specifics* — which axis, which shape, why it was suspected — not the headline. The
    § Probe gaps list was written in full before any of it was worked, and that is the
    order to keep. It also makes the list reviewable while it is still cheap to change.

## Stage A finding — which copies happen with no warning

This is the base of the plan: the copies loft makes today and says nothing about. Every
row's copy is proven **by value** (an alias would give a different answer), then checked
against `LOFT_REPORT_COPIES=1 LOFT_WARN_COPIES=1 LOFT_COPY_SURVIVAL=1` and
`LOFT_MATERIALIZE_DUMP=1`. Identical on both backends.

| Probe | Shape | Copy happens? | Internal verdict | User-facing |
|---|---|---|---|---|
| 13 | `Holder { v: src }`, src mutated after | yes | `bucket=forced` | **reported** — `line 5 … [forced]` |
| 14 | `v[0] = src`, src mutated after | yes | `bucket=forced` | **reported** — with `file:line:col` |
| 12 | `w = h.v`, `h.v` mutated after | yes | `bucket=AVOIDABLE` | **silent** |
| 10 | `b = a` whole-record bind | yes | **no verdict at all** | **silent** |
| 11 | `cp = ident(orig)` call return | yes | **no verdict at all** | **silent** |

Probes 13/14 prove the instrument is live, so the "silent" rows are real blindness rather
than a dead harness. Two distinct mechanisms, and they need different fixes:

- **Mechanism 1 — seen, classified, then dropped (probe 12).** The analysis emits
  `MAT fn=n_main v=2(w) src=0 verdict=Copy bucket=AVOIDABLE [source not a parameter / not
  provably read-only local]`. `AVOIDABLE` is precisely the bucket `LOFT_WARN_COPIES` is
  documented to warn on (`keys.rs:374`), and the report still prints `none`. The gap is
  between the verdict and the report.
- **Mechanism 2 — never seen (probes 10, 11).** No `MAT` row exists. The copy-vs-alias
  analysis runs over the **IR** in `scopes::check`, but these copies are invented *later*,
  at bytecode generation — `state/codegen.rs:2656 gen_set_first_ref_var_copy` in the
  interpreter, `generation/dispatch.rs:578` in native. `MoveKind::Record` covers `v[i] = e`
  and `o.f = src`, never a plain local bind. No diagnostic can reach these even in
  principle, and each backend re-derives the decision with its own proxy.

**And all of it is opt-in.** In a default build every copy is silent: `LOFT_WARN_COPIES` is
default-OFF by design (`keys.rs:377` — *"the Avoidable set is not yet drained"*). Draining
that set is what makes decision 2 above shippable.

### The report is not merely quiet — it asserts the opposite

On probe 10, a program that provably deep-copies, the report prints:

```
loft copy report — unbound structure copies (a copy the alias-default did not make silently)
  none — every structure copy is a move, a literal, or already borrowed.
```

A blind spot that reports "none" reads as a clean bill of health. Fixing the wording is not
the fix, but it belongs to the same change.

## Stage A finding — the alias that outlives what it names

The mirror of the above: where loft *keeps* an alias that is no longer safe. A binding read
out of a container is a VIEW of an **element**, and must keep naming that element for as long
as it is live. Today it is pinned to an **index**, and `remove` renumbers the indices without
telling the outstanding views.

| Probe | Shape | Result |
|---|---|---|
| 01 | view, then `v += [..]` ×3 | pass — growth alone never disturbs a view |
| 02 | view of `v[2]`, `remove(0)`, read | **pass by luck** — the vacated slot still holds the old bytes |
| 03 | same, then **write** through the view | **FAIL** — write lands past the live length, silently lost |
| 04 | `remove(0)` then `+= [Box{44,444}]`, write | **FAIL — corruption.** `e2` reads `99/444`: a **torn record**, `n` from the stray write, `tag` from the real element. The intended target `e1` is untouched |
| 05 | same, **read only, no write anywhere** | **FAIL** — the view reads `44/444` instead of `33/333`: a *completely different element* |
| 06 | view of a field-of-element, then `remove(0)` | **FAIL** — nesting inherits the index pinning |
| 07 | view bound before a loop, `b#remove` inside | **FAIL** — the spelling of the removal is irrelevant |
| 08 | keyed `hash<Entry[name]>`, remove another key | pass — keyed removal does not shift |
| 09 | view, then whole-container reassign `v = other` | pass — the view keeps its element |

The invalidator is specifically **positional shift on vector removal** (`v.remove(i)`,
`v#remove`) — not append, not reassignment, not keyed removal. Probe 05 is the sharpest
form: no write anywhere, and the read answers a different element.

**No detector fires.** `LOFT_STORES=warn` and `LOFT_POISON=1` are silent on probes 04 and
05. They cannot see it — nothing is freed and no store is invalid. The `DbRef` is perfectly
live; it just names the wrong element. That is why this class survived every store-lifetime
sweep, #775's included.

## Cluster catalogue

| ID | Cluster | Severity | Backend asymmetry | Probes | Doc |
|---|---|---|---|---|---|
| I | Silent copies — the inventory and its two mechanisms | wrong cost, no wrong value | both silent; native has no move at all | 10–14 | pending |
| II | Index-pinned views survive a shifting removal | **corruption + silent wrong read** | both identical | 01–09 | pending |
| III | Producer set — which binds mint a view | unknown | unknown | pending | pending |
| IV | Invalidator set — what else shifts | unknown | unknown | pending | pending |
| V | Backend parity for the alias/move decision | perf (native) | **native-only gap** | 10, 11 | pending |

**III — producers.** Verified: `v[i]`, `v[i].f`. Untested: `&` params bound from an element,
`match` captures, `for` loop vars held across a removal, tuple elements, a view stored into
another record.

**IV — invalidators.** Verified: `remove(i)`, `#remove`. Untested: `sorted`/`index` reordering
on key mutation, `par` writes, nested-container removal, removal during iteration of a
*different* view of the same container.

**V — parity.** For the whole-record bind the interpreter has a last-use move
(`state/codegen.rs:2656`) gated on `uses(src) == 1` — a raw parse-time appearance count
(`variables/mod.rs:1084`), far stricter than "dead after the bind". Native has **none**:
`generation/dispatch.rs:578` emits `OpDatabase` + `OpCopyRecord` unconditionally. Under
decision 3 native must gain the move, not the interpreter lose it.

## Probe suite

`probes/`, run on `--interpret` and `--native`. Probes 01–09 assert the view invariant;
10–14 assert the copy inventory (each proves its copy by value, so it stays meaningful once
the diagnostics change).

| File | Shape | Cluster | Status |
|---|---|---|---|
| `01-append-baseline.loft` | view survives growth | II | passes — baseline |
| `02-remove-read-stale-luck.loft` | read after shift | II | passes **by luck** — contrast for 05 |
| `03-remove-write-lost.loft` | write after shift | II | FAILS — write lost |
| `04-remove-reoccupy-write.loft` | stale index re-occupied, write | II | FAILS — torn record |
| `05-remove-reoccupy-read.loft` | stale index re-occupied, read only | II | FAILS — wrong element |
| `06-nested-field-view.loft` | field-of-element view | II/III | FAILS |
| `07-loop-remove.loft` | `#remove` invalidator | II/IV | FAILS |
| `08-keyed-remove-baseline.loft` | keyed removal | II | passes — baseline |
| `09-container-reassign-baseline.loft` | whole-container reassign | II | passes — baseline |
| `10-silent-record-bind.loft` | `b = a` | I/V | asserts pass; copy **unreported, unclassified** |
| `11-silent-call-return.loft` | `cp = ident(orig)` | I/V | asserts pass; copy **unreported, unclassified** |
| `12-silent-field-copyfill.loft` | `w = h.v` | I | asserts pass; `AVOIDABLE`, **unreported** |
| `13-reported-construct.loft` | `Holder { v: src }` | I | passes — the instrument fires |
| `14-reported-elemset.loft` | `v[0] = src` | I | passes — the instrument fires |
| `15-secret-expression-temp.loft` | `make(i).tag` | I | **reference** — 0 copies; contrast for 18 |
| `16-secret-reassign-var.loft` | `b = a`, both live | I | secret copy — `InterpReassignVar` |
| `17-secret-loop-carried.loft` | `cur = make(i)` in a loop | I | **reference** — 0 copies; contrast for 19 |
| `18-secret-stdlib-exists.loft` | stdlib `exists()` | I | secret copy — 5 calls, 5 `File` copies |
| `19-secret-call-bind.loft` | `f = file(path)` | I | secret copy — `InterpCallReturn` |
| `20-q5-return-dep-discriminator.loft` | 4 callees, identical but for spelling | I | Q5 answered — `[]` adopts, `[1]` copies |
| `21-buffer-reuse-across-iterations.loft` | loop over a `[1]`-dep callee | I | buffer VAR reused, store freed per call |
| `22-same-site-liveness.loft` | 6 escape routes, all axes pinned at 1 | I | **superseded by 24** — its sweep was too narrow |
| `24-escape-routes-axes-moved.loft` | the same routes, four axes moved | I | E3/E4 break under adoption; E2/E5/E6 hold |
| `23-chained-return-buffer.loft` | mixed literal/call Set, struct params, interleaved | I | **guard** — fails if the Q5 predicate is widened |

Runner: `probes/run_set.sh [view|copy|secret|all]` — per probe, pass/uncovered/executed-copies
on both backends.

## Must resolve before close — no deferral

An investigation plan OWNS every problem it surfaces. Probing is the work, but a catalogue is
not a result: each row below is **resolved in-plan**, with a regression test. Nothing gets
handed to the tracker to be somebody's later problem — filing is not a marker of
significance, it is a deferral.

**The closure bar is INFORMATION — but three things are never allowed, and a warning does not
buy them off.**

| never allowed | why a diagnostic is not enough |
|---|---|
| **Breakage** — a wrong value, a lost write, corruption, data loss | telling the author their program is silently wrong does not make it right. There is nothing for them to decide |
| **Misinformation** — a diagnostic or doc that states something false | worse than silence: it is trusted. `--report-copies` answering `none` on a copying program, and `LOFT.md`'s "whatever the field's type" |
| **Silent copies** — a copy no diagnostic accounts for | the author cannot weigh a cost they are never shown |

So the plan can close without every case *optimised*, but not with any case *broken*. Each
finding resolves exactly one of:

- **FIXED** — the program produces the right answer. Required wherever the defect is a wrong
  value (F1, F2, F3, F4). For F2 the materialise IS the fix; the warning that rides with it
  is information about the cost, not a substitute for correctness.
- **CORRECTED** — a false statement is made true (F5's `none`, F6's doc).
- **STATED** — reserved for a program that is already CORRECT and merely pays a cost. A copy
  that is necessary, or one we could remove with a better analysis and have not. This is the
  only category that may rest.

"Allowed for now" therefore applies to **copies, never to wrong behaviour**.

| # | problem | evidence | resolves as | state |
|---|---|---|---|---|
| F1 | View reassigned from a loop var **destroys the container** (interp-only, silent, total) | probe 30, loft#778 | **FIXED** — silence is not an option here | mechanism VERIFIED to a code line; fix designed, not applied |
| F2 | Index-pinned views survive a shifting removal — wrong reads and cross-element corruption | probes 03–06, 29 | **FIXED** — materialise (the warn states the cost, it is not the fix) | designed; shares F1's analysis |
| F3 | `&` param bound from an element loses its write after a shift | probe 26 | **FIXED** — a lost write is breakage; a diagnostic does not buy it off | open |
| F4 | Re-keying a `sorted` element through a view makes it unreachable by key | probe 28 | **FIXED** — an unreachable live element is breakage | open |
| F5 | Copies **no diagnostic accounts for** — the `exists()` family | probes 10–12, 18, 19 | **CORRECTED** (the `none` report is misinformation) + **STATED** (the copies then rest as *allowed for now*) | guard built; notice not yet default-on |
| F6 | `LOFT.md` claims a match capture is a view "whatever the field's type"; scalars copy | probe 31 | **CORRECTED** — misinformation in the doc | open |

**loft#778 was filed and should not have been** — it is F1, this plan's own finding, and the
tracker entry defers what the plan is supposed to close. Keep it cross-linked, fix it here.

### F1 mechanism — VERIFIED (this is cluster II's mechanism, no longer hypothesised)

Repro + capture: `bytecode-comparisons/f1-view-loopvar-reassign.loft` / `f1-capture.txt`.

**1. The bind types the view as an OWNER.** The IR for `k = a[0]`:

```
[9] k(1):ref(Box) = OpGetVector(a(1), 16i32, 0i32);
```

`k(1):ref(Box)` carries **no dep**, while every sibling does — `_elm_1(1):ref(Box)["a"]`,
`x(3):ref(Box)["_vector_1"]`. So nothing records that `k` borrows `a`.

**2. The reassignment then frees "its" store.** Bytecode for `k = x` inside the loop:

```
331: VarRef(k)
334: FreeRef ; [store-free]      <- frees k's store, which IS a's store
335: InitRef(k)
338: Database(k, db_tp=65)
349: CopyRecord(x -> k)
```

`k` is a raw pointer into `a`'s store, so the pre-Set store-free releases **the whole
container**. `len(a)` is 0 from that instant — before any removal, and before the read that
later answers `null(oob)`.

**3. Why native escapes it:** the generated Rust declares `let mut _own_store_k: DbRef` — it
materialises an owned store for `k` at the bind, so the free hits that instead of the
container. One backend already implements the safe answer.

**The precedent is in the tree, and names this exact failure.** `parser/vectors.rs:2522`
(loft#664) on the element MINT path:

> *"an element NEVER owns a store… That was encoded only as a DEPENDENCY on the container
> VARIABLE, so a container with no variable left the dep list empty, and **empty reads as
> 'owns its store': the answer came back WRONG rather than unknown**. State the fact at the
> mint site instead, through the marker that already means 'borrow, don't allocate'."*

That fix added `mark_inline_ref(elm)`. It is applied when an element is **minted**
(`OpNewRecord`) and **not** when one is **read** into a local (`OpGetVector`) — which is
exactly the hole F1 falls through. Same invariant, same marker, one uncovered producer.

### F1 + F2 design — one producer, two different questions

They meet at the same bind (`c = v[i]`) and it is tempting to call them one bug. They are
not, and conflating them is how a fix for one silently fails the other:

| | question | missing fact | fixable by |
|---|---|---|---|
| **F1** | *who frees this store?* | "this binding is a BORROW" | a marker at the bind — ownership |
| **F2** | *which element does it name?* | "the element moved" | nothing at the bind — identity |

**F1 is closable as stated.** A dep says "borrow, don't free"; the pre-Set store-free is
suppressed; the container survives. Native's `_own_store_k` is a working reference.

**The fix has an exact precedent — P250, `parser/expressions.rs:3230`**, which repaired the
same failure for tuple destructuring:

> *"each LHS Reference element is a VIEW into the tmp's storage… Without a dep, scope
> analysis emits an independent `OpFreeRef` for the LHS at scope exit; that free works on a
> store_nr basis and **frees the entire tmp's underlying store**… Marking the LHS dependent
> on tmp suppresses its independent free."*

Its remedy is one line — `self.vars.depend(v_nr, tmp)` — and `create_elm`
(`vectors.rs:2535`) does the same for minted elements. So the F1 change is
`self.vars.depend(<lhs>, <container var>)` at the bind where the RHS is a bare element read.

Note this is a dep on the **variable**, not a dep smuggled into a returned `Type` — the
latter is a dep-space crossing and the wrong route.

**CORRECTION — the dep is not missing at the bind. It is STRIPPED later.** Measured with
`LOFT_VAR_TABLE=main` on two programs that both bind `= a[…]`:

```
rm.loft  (safe)      c  ref(657)  def deps=[a(0)]     <- dep present
f1 repro (destroys)  k  ref(657)  def OWNS            <- dep stripped
```

The only difference is that `k` is later reassigned. So `c = v[i]` **does** record the
borrow; the reassignment removes it. Adding a dep at the bind would have changed nothing —
this is why the var table was worth reading before writing the fix.

**The stripper is `scopes.rs:3225`:**

```rust
// When `Set(v, Var(src))` and both are References to the same struct, codegen
// takes gen_set_first_ref_var_copy which deep-copies src into a FRESH store
// owned by `v`.  Strip v's declared deps so get_free_vars emits OpFreeRef.
if let Value::Var(src) = unspanned_value && … d_nr == *src_d {
    for d in deps { function.make_independent(v, d); }
}
```

Its reasoning is correct for its own case: after the reassignment `v` genuinely owns a fresh
store and genuinely needs a scope-exit free. What it does not account for is that stripping
the dep **also re-enables the pre-Set free**, and that free runs on the value `v` held
BEFORE the reassignment — which was the borrowed container. One strip, two consequences,
only one of them intended.

**This unifies F1 and F2 into one analysis with two triggers.** Materialise an element-read
binding when either:

- **(F2)** the container is reshaped while the binding is live, or
- **(F1)** the binding is later reassigned — it must own a store from the start, exactly as
  native already does with `_own_store_k`.

Otherwise keep the borrow. One rule, one implementation, and it is the option-2 machinery in
both cases — materialise when the alias cannot be held, warn, keep the alias everywhere else.

**Decision site it feeds:** `state/codegen.rs:1899` computes
`owned_ref = … && tp(v).depend().is_empty() && !is_skip_free(v)`, and line 1927 emits the
unconditional pre-Set `OpFreeRef` when `owned_ref` holds. A non-empty dep makes `owned_ref`
false and the free never fires. `is_inline_ref` is deliberately NOT the lever here: the
comment at `codegen.rs:1897` records that an owned `Vector` `??` subject is marked
`inline_ref` *precisely so it keeps* that free (loft#615), so widening on that marker would
regress it.

**F2 cannot be fixed by a dep**, because the defect is not ownership. A view is a `DbRef`
`(store, rec, pos)`; `remove` renumbers the positions; no amount of ownership information
tells a fixed `pos` that its element moved. Only three answers exist, and picking one is a
**semantics decision**:

1. **Removal stops shifting** (tombstone / stable slots). Views stay valid, write-through
   keeps working, and #774's documented alias survives intact. Cost: holes in the store, a
   compaction policy, and a changed iteration/`len` contract.
2. **Materialise the view + WARN** when it is live across a reshape. No corruption, program
   keeps running, author is told. Cost: write-through is **lost** for that binding — probes
   03/04/05 currently assert the write reaches the element, and they would have to assert the
   copy instead.
3. **Diagnose only.** Cheapest, and leaves the corruption in place. Rejected — the plan
   exists because silence is the defect.

**Recommendation: option 2**, because it is what this plan's own model already prescribes —
constraint 2 is *"where rustc errors on a use-after-move, loft copies and warns"*, and a view
outliving its element's position is exactly a use-after-move. It is compile-time (constraint
5): the analysis asks whether a reshaping op on the container occurs in the binding's live
range, and materialises only then — so the alias is kept whenever it is safe (constraint 1).

Option 1 is the better *language* if the store can afford it, and it is the only one that
keeps write-through. It is a representation change well beyond this plan, so it belongs to
the owner rather than to me.

**This is a semantics change either way, so it needs sign-off before implementation** —
option 2 makes `c = v[i]` copy in reshape-containing scopes, and probes 03/04/05 flip from
asserting write-through to asserting the copy. F1 does not need that sign-off and is being
implemented first.

## Probe gaps — what is NOT covered yet

Ordered by what would cost most to keep not knowing. Written **in full before any of it was
worked** — see § Method rule 10: with three or more candidate cases, the specifics of the
later ones decay while you work the first.

**1. Re-run probe 22's escape routes with the axes moved — DONE (probe 24). Claim partly
RETRACTED.** Probe 22 concluded *"every escape route yields independent values"* with all four
axes pinned at 1. Probe 24 re-runs the same routes through a two-Set, struct-param, nested
callee with an interleaved sibling. Measured against the widened predicate:

| route | verdict |
|---|---|
| E2 container insert `v += [mixed(p)]` | passes — the insert deep-copies |
| **E3 second binding `b = a`** | **FAILS** — `b.tag 501 want 901`; `b` aliases `a` |
| **E4 struct field `Holder { inner: a }`** | **FAILS** — `h.inner.tag 901 want 1` |
| E5 recursion | passes — each frame owns its buffer |
| E6 return up a level | passes |

So the finding stands for **E2/E5/E6** and is **false for E3 and E4**. Under adoption `a`, `b`
and `h.inner` collapse onto one store — E4 reads `901`, the value E3's `b` was given, so the
contamination crosses all three names rather than leaking a single write.

Every cell passes on `--native`, so the widening is **interpreter-only** in its damage — the
second independent reason it is unlandable, and a hint that native's path already handles the
case correctly and could be read for the answer.

This also sharpens what a real fix must do: it is not enough to know a buffer is freed after
the copy (probe 21). The predicate must also know whether the adopted value will be **bound or
stored a second time** — E3 and E4 are exactly that, and they are the routes that break.

**2. Cluster II's producer × invalidator matrix — WORKED (probes 25–30). Four new failures,
one of them a different and worse bug.**

| # | cell | interp | native | result |
|---|---|---|---|---|
| 25 | producer: `for` loop var captured out of the loop | pass | pass | **safe** — the capture COPIES; not an index-pinned view |
| 27 | producer: a view stored into another record | pass | pass | **safe** — the store copies |
| 26 | producer: a `&` param bound from an element | **FAIL** | **FAIL** | write through the `&` view is lost after a shift |
| 29 | invalidator: removal during iteration of a *different* view | **FAIL** | **FAIL** | same class as 03/07, reached with two live views |
| 28 | invalidator: re-keying a `sorted<T[key]>` element through a view | **FAIL** | **FAIL** | see below — worse than a stale index |
| 30 | reassigning a view from a loop var | **FAIL** | pass | **silent total data loss, interpreter-only** |

**So "reading out of a container" is not one rule.** `v[i]` and `v[i].f` alias (03–06), a `&`
param from an element aliases (26), but a captured loop var (25) and a stored view (27) copy.
That split is not visible in the source.

**Probe 28 is sharper than "the index goes stale".** After `c.key = 5` on a live element:
`c` itself updates, `s[5]` answers **null**, `s[30]` answers **null**, and iteration still
yields 3 elements. The element becomes **unreachable by key while remaining in the
collection** — the write updated the record but never re-indexed. A program that keys into
`s` loses an element it can still iterate over, with no error and a count that still looks
right.

**Probe 30 is a different bug and the most severe thing found so far — filed as [loft#778](https://github.com/loft-lang/loft/issues/778).** `k = a[0]; for x in a
{ … k = x … }` leaves `len(a) == 0` — the container is destroyed **before any removal**, and
`a[0]` then reads `null(oob)` rather than faulting. Interpreter-only; native is correct.
Reproduced on the installed 2026-08-04 binary, so it is mainline. Hypothesised mechanism: the
reassignment frees `k`'s previous store, and `k` was a *view*, so the free releases the
CONTAINER — the same "a view is not a store" invariant as #775, reached through a loop-var
reassignment. The boundary is narrow, which is why it survived: no capture, capture into an
owned local, and reassigning a view *outside* a loop are all safe.

**COMPLETE — the last four cells (probes 31–34) all pass, and the boundary is now sharp.**

**Producers — which binds mint an index-pinned view:**

| producer | verdict | probe |
|---|---|---|
| `v[i]` | **VIEW, index-pinned → breaks** | 03–05 |
| `v[i].f` | **VIEW, index-pinned → breaks** | 06 |
| `&` param bound from an element | **VIEW, index-pinned → breaks** | 26 |
| `for` loop var captured out | copy — safe | 25 |
| view stored into another record | copy — safe | 27 |
| tuple element | copy — safe | 32 |
| `match` capture, scalar payload | copy — safe | 31 |
| `match` capture, text/heap payload | view, but **cannot outlive its arm** — safe | 31 |

**Invalidators — what disturbs a live view:**

| invalidator | verdict | probe |
|---|---|---|
| `v.remove(i)` | **breaks** | 03–06 |
| `v#remove` | **breaks** | 07 |
| removal during another view's iteration | **breaks** | 29 |
| re-keying a `sorted` element through a view | **breaks** (unreachable by key) | 28 |
| append `+=` | safe | 01 |
| whole-container reassign | safe | 09 |
| keyed-collection removal | safe | 08 |
| removal from a NESTED container | safe | 33 |
| `par` writes | safe | 34 |

**The invariant, now measured rather than guessed: the invalidator is a POSITIONAL SHIFT
inside one vector's own store.** Growing it, replacing it, removing from a keyed collection,
and reshaping a container one level down are all harmless. Only an operation that renumbers
the indices in the store the view points into breaks it — and re-keying a `sorted` element
(28) is the same thing reached without deleting anything.

**And the producer split has no marker in the source.** Three shapes alias and five copy,
with nothing at the binding site to say which. The `match`-capture case is safe for a
different reason from the rest — a lifetime bound (the capture dies with its arm), not a
copy — so it would stop being safe if captures ever became holdable.

Two side-findings worth keeping:

- **LOFT.md overstates the match rule.** It says a destructured field is a view "whatever
  the field's type"; measured, a **scalar** payload capture is a copy on both backends.
- **loft#778** was filed from this matrix. Per § Must fix before close, filing it was the
  wrong move — it is this plan's to fix, not to hand off.

**Original scoping note:**
This is #774's actual defect, it corrupts silently, and it is unfixed. Probes 01–09 cover
**2 producers × 2 invalidators**:

- *Untested producers:* a `&` param bound from an element · a `match` capture · a `for` loop
  var held across a removal · a tuple element · a view stored into another record.
- *Untested invalidators:* `sorted`/`index` reordering on key mutation · `par` writes ·
  nested-container removal · removal during iteration of a **different** view of the same
  container.

Every unprobed cell is a potential live corruption nobody has looked at. Given probe 05 — a
pure read answering a different element with no detector firing — hits are likely rather than
hypothetical.

**3. The two uncatalogued copy families.** `R666` (recursive enum payload) and `RbOuter`
(borrow-return), both surfaced by the corpus survey and both still unprobed. Lower priority:
cost, not corruption, and the guard already names them.

**Not needed:** more probes for the copy inventory (10–20 cover it) or for the Q5 boundary
(20/21/23 pin it). Extending the manifest to the parser's emitters is instrument work, not a
probe gap — see Tool gaps.

## Reference ↔ problem pairings

| Problem | Reference | What the diff reveals |
|---|---|---|
| 03 | 01 | removal, not growth, is the invalidator |
| 05 | 02 | a passing read proves nothing — 02 survives only because the vacated slot was untouched |
| 03 | 08 | keyed removal does not shift; vector compaction is the mechanism |
| 03 | 09 | replacing the container is safe; the in-place positional shift is not |
| 07 | 03 | the removal spelling is irrelevant — the shift is the cause |
| 10 | 13 | both copy; only the one present in the IR at `scopes::check` is reported |
| 12 | 13 | both are seen by the analysis; only one survives into the user report |

## Open questions

- **Q1 — which diagnostic tier?** Decision 2 says *warning*. The repo rule is *a diagnostic
  gates iff ignoring it can produce a wrong result* (`CLAUDE.md`), and an unwanted copy
  yields a correct-but-slower program — which points at `advice`. Recommendation: ship the
  copy notice as `advice` and reserve `warning` for cluster II, where ignoring it **does**
  produce a wrong value. To be confirmed; if warning is still wanted, the tier rule needs
  the amendment written down with it.
- **Q2 — can cluster II be fixed without weakening the alias default?** Materialising every
  container read would close it and violate decision 1. Tombstoning, or re-pointing live
  views on shift, keeps the alias. Decide against cluster III's mechanism, not before.
- **Q3 — what does the `Avoidable` set actually contain?** Decision 3 makes draining it a
  prerequisite, and `keys.rs:377` says it is not drained today (*"the Avoidable set is not
  yet drained"* is the stated reason the lint is off). Stage B must size it: run the report
  across the stdlib, the test corpus, and a real consumer, and split the set into *copies we
  can eliminate*, *copies that should be reclassified* (`Forced`/`Implicit` mislabelled as
  `Avoidable`), and *copies a human must accept*. Only the last group should ever reach an
  author, and if it is large the accept mechanism is being used to hide analysis weakness.
- **Q4 — what is the per-file accept spelled as?** `#superseded "…"` gives the per-definition
  shape; nothing in the language currently scopes an annotation to a whole file. Needs a
  syntax decision before cluster I can ship.

## The guard — codegen reality vs. what the diagnostic claimed

The load-bearing instrument for this plan, and cheaper than it looks: **both halves already
exist and nothing joins them.**

- **Runtime ground truth** — `LOFT_COPY_DUMP=1` (`keys.rs:339`), one line per executed deep
  structure copy. Its own doc states the intent verbatim: *"the runtime ground truth for
  every copy + its size, so the compile-time copy-vs-borrow decision can be checked to cover
  them all"*. The cross-check was designed for and never built.
- **Compile-time claim** — `use_analysis::report_copies`, the classified set with positions.

**The guard is the diff: an executed copy with no verdict is a blind spot.** Verified today
on probe 10 — the `b = a` copy the static report calls `none` — `--interpret` prints
`[copy] record line=17 tp=65`. The runtime already names the line the compiler denies.

**Why the runtime, not the emitters.** Deep copies are emitted from ~15 sites across
`src/parser/`, `state/codegen.rs` and `generation/`. Funnelling them through one registering
helper is both a large refactor and *bypassable by the next raw emission* — the exact failure
already recorded for `generate_call`'s `skip_free` guard. The runtime is downstream of every
emitter, so a new emitter cannot escape it.

**Split the guard by what is knowable where. A copy is DEEP, so its cost is not a
compile-time fact — only its existence is.**

*Static, and complete:* whether a copy happens at all, its site, its type, and the **flat**
record size. The type number is already a compile-time literal in the emitted call
(`OpCopyRecord(cell, var_a, var_b, 65_i32)`), and the flat size is a fixed per-type constant
(`database/types.rs:2066` — `self.types[tp].size`). Native re-derives that size at runtime
only because it ships no layout data: the generated `init()` REPLAYS the type registration
(`db.structure("FvBool", 1)`, `db.field(…)`) — the same replay `LOFT_STRICT_SCHEMA_IDS=1`
polices. Nothing about the *flat* size is discovered by running.

*Runtime only:* the **deep** cost. `copy_claims` duplicates the record's nested vectors,
hashes, texts and sub-records, and how much that is depends entirely on runtime data. No
compile-time analysis can bound it.

**The deep cost is reported by nothing today — on either backend.** Measured: a `Big { n,
v }` whose vector holds **1000 elements**, copied by `b = a`, prints exactly one line and no
magnitude:

```
--interpret   [copy] record       line=9  tp=65
--native      [copy] OpCopyRecord src=#0@1,8 dst=#1@1,8 tp=65 size=12 free_src=false
```

`size=12` is the flat record. The 1000 copied elements appear nowhere. The
`LOFT_COPY_DUMP` element-count hook lives in `vector_add` (`database/structures.rs:404`),
the explicit-append path — and its own comment states the goal it was written for: *"the
runtime size — the 'hundreds of MB just to be sure' the user cannot see today."* The
`copy_claims` deep walk (`allocation.rs:2133` plus its five `_body` variants) has **no hook
at all**, so the record-copy path — the one every probe in the inventory above runs
through — misses exactly the case the instrument exists for. This is also precisely the
moros shape from loft#774: `held = current` over a `World` wrapping a chunk store would
report `size=<flat World>` and say nothing about the chunks.

### The guard — BUILT (`LOFT_COPY_MANIFEST=1`)

`src/copy_manifest.rs`. Each generator records every deep copy it WRITES, at the branch that
writes it; the guard diffs that manifest against `use_analysis`'s verdicts and reports the
copies no diagnostic accounts for. Compile-time only — nothing reaches a compiled program.

Registration points (recorded *past* every early return, so a last-use move or an adopt is
never miscounted as a copy):

| Origin | Site |
|---|---|
| `InterpRecordBind` | `state/codegen.rs` `gen_set_first_ref_var_copy` |
| `InterpCallReturn` | `state/codegen.rs` `gen_set_first_ref_call_copy` |
| `InterpTupleBind` | `state/codegen.rs` `gen_set_first_ref_tuple_copy` |
| `NativeRecordBind` | `generation/dispatch.rs`, `Value::Var(src)` arm |
| `NativeCallReturn` | `generation/dispatch.rs`, call arm — a **may-copy** (runtime adopt-or-copy branch), rendered as such |

**Mode-B gate passed:** `loft introspect` on `bytecode-comparisons/manifest-corpus.loft`
(one function per emission path) is **byte-identical** before and after, re-checked after
`cargo fmt`. `introspect` output was first confirmed deterministic across two runs, so the
gate means something. Nothing emitted changed.

**Validation — it flags exactly the Stage A blind spots and nothing else:**

| Probe | expected | guard |
|---|---|---|
| 10 `b = a` | uncovered (no verdict) | **flags** `InterpRecordBind` / `NativeRecordBind` |
| 11 `cp = ident(orig)` | uncovered (no verdict) | **flags** `InterpCallReturn` / `NativeCallReturn` |
| 12 `w = h.v` | classified `AVOIDABLE` | quiet — correct: the analysis *did* see it; its gap is verdict→report, not the manifest |
| 13 `Holder { v: src }` | covered | quiet |
| 14 `v[0] = src` | covered | quiet |

Two findings the probes did not contain:

- **The guard caught its own mis-instrumentation.** `gen_set_first_ref_copy` looked like the
  call-return emitter but its own doc records *zero fires* in the corpus; instrumenting it
  left probe 11 silent. The real emitter is `gen_set_first_ref_call_copy`. A manifest built
  by reading the code rather than by validating against known-uncovered cases would have
  shipped that hole.
- **A stdlib copy nothing accounts for, native only:** `fn exists` → `__lift_1` (a
  compiler-generated binding), `NativeCallReturn`. It appears in *every* native compile. The
  interpreter does not report it, so this is cluster V again — native emits an adopt-or-copy
  where interp adopts outright.

Consequence: the uncovered set is **not empty today**, so the guard stays opt-in
(`LOFT_COPY_MANIFEST=1`) until it is drained. Its audience is CI and this repo — it reports a
hole in the *compiler*, not a fault in a user's program.

### The secret-copy catalogue (probes 15–19, run via `probes/run_set.sh secret`)

What the guard found once pointed at the corpus. Surveyed across all **583** `tests/scripts`:
**exactly one uncovered site per compile, always the same one** — the stdlib's `exists()`.

| Family | Where | Measured |
|---|---|---|
| stdlib `exists()` lift | `default/02_files.loft:248` — `file(path).format` | 5 calls → **5 deep `File` copies**, both backends. Uncovered in **every** compile that loads the stdlib |
| bind a `file()` result | `f = file(path)` | 1 copy per bind; **107** hits across the corpus — the second-largest family |
| reassignment `b = a` | both bindings live | 1 copy; a different emitter from the first bind, which is why it was invisible |
| enum payload / borrow-return | `R666`, `RbOuter` in the corpus | uncovered, not yet probed |

Each `File` copy duplicates 33 flat bytes **plus a reallocated `path` text** — `OpCopyRecord`
duplicates owned sub-structures — to read one enum field and discard the record.

**Two probes measured the opposite of what they were written to show, and that is the
sharpest result here.** Probes 15 and 17 use the *same syntax* as 18 and 19 — an expression
temp and a loop-carried reassignment — with a user-defined `make()` instead of `file()`.
They copy **zero** times. So the copy is a property of the **callee's return**, not of the
lift, the loop, the reassignment, or the field read. A fix aimed at "expression temps" would
miss entirely. Both are kept as reference probes: if return-ownership ever tightens, they are
the cases that silently become one copy per iteration.

Reading the runner: there is an **ambient baseline of 1 uncovered site** (the `exists` one) in
every compile, and a `--native` run reports both generators, so its column roughly doubles.

## Open questions (added by the guard)

- **Q5 — ANSWERED: naming the result in a local is the whole trigger.** See probe 20.

  `return_adopts_fresh_store()` (`src/data.rs:3328`) is exactly: returned-type deps **empty**,
  or the lone `[u16::MAX]` one-buffer marker → adopt; **any other dep → copy**. Four
  observationally identical callees, measured:

  | shape | returned deps | caller |
  |---|---|---|
  | `Rec { … }` returned **directly** | `[]` | adopts — no copy |
  | `r = Rec { … }; r` — **unmutated** | `[1]` | **deep-copies** |
  | bind, mutate a field, return | `[1]` | **deep-copies** |
  | bind, mutate via a call, return (the `file()` shape) | `[1]` | **deep-copies** |

  Dep `[1]` names the callee's **own hidden `__retbuf`**. So binding the result to a named
  local before returning it — no mutation, no semantic difference, the two functions compute
  the same value — costs a full deep copy at **every call site**. `n_file` is
  `result = File{…}; OpGetFile(result); result` (`02_files.loft:238`), which is why `exists()`
  copies and probe 15's `make()` does not.

  **The scope is far wider than `exists`.** "Build into a named local, then return it" is the
  idiomatic way to write a constructor-ish function, and every function written that way pays
  a deep copy per call. `exists` is simply the instance that is compiled into every program.

  **Why this looks like a mis-classification, not a real distinction:** both shapes have a
  `__retbuf` attr — the difference is only whether the return TYPE records a dep naming it.
  `[u16::MAX]` and `[<index of __retbuf>]` describe the *same* situation (returned via my own
  hidden buffer, nobody else owns it), and `return_adopts_fresh_store` tests for the marker by
  VALUE rather than asking whether the named attr *is* a hidden retbuf.

  **The reuse hazard — MEASURED, and it splits in two.** The gating question was whether the
  caller reuses that buffer before the binding dies (`gen_set_first_at_tos`: "a hidden
  `ref_return` work-ref the caller REUSES across iterations"). Traced on native with a
  three-iteration loop over a `[1]`-dep callee:

  ```
  keep = mk_local(100)   [copy] src=#1 dst=#0 free_src=true
  loop i=1              [copy] src=#2 dst=#1 free_src=true
  loop i=2              [copy] src=#2 dst=#1 free_src=true
  loop i=3              [copy] src=#2 dst=#1 free_src=true
  ```

  - **The buffer VARIABLE is reused** — `var___ref_2` is declared once at function scope and
    handed to all three calls, which is what the warning was about.
  - **The STORE it names is NOT.** `free_src=true` on *every* call: `OpCopyRecord` frees the
    source right after copying it. The constant `#2` is the allocator returning the slot it
    just freed, not a store living across iterations.
  - **Distinct call sites get distinct buffers** — `keep` uses `__ref_1`, the loop call uses
    `__ref_2` — so cross-site aliasing is not a concern.

  So the emitted sequence is **copy-then-free: a move implemented the expensive way.** At the
  moment of the copy the source belongs to nobody else and is about to be discarded — which
  is precisely the situation the `[u16::MAX]` marker path already adopts.

  **The predicate this implies is sharper than "dep names a hidden attr":** adopt when the dep
  names the callee's OWN retbuf **and** the return is not a borrowed view (`free_src` set). A
  genuine borrowed-view return leaves the free bit clear, and there the copy is required.

  **The fix was applied and REVERTED. The one-liner is wrong** — the doc comment on
  `return_adopts_fresh_store` was right and the measurement that contradicted it was too
  narrow. Recorded here because it looks correct from every angle probes 21/22 examined.

  Widening the predicate to accept a lone hidden-attr dep eliminated exactly the copies it
  should: probe 20's three → **0**, probe 18's `exists()` five → **0**, probe 19 one → **0**,
  while probe 16 (a genuine copy between two live bindings) correctly stayed at 1. The
  emitted form became byte-identical to the proven-working `mk_literal` adopt sequence. Every
  probe passed, no leaks.

  Then `tests/scripts/143-plan51-cluster3-mixed-lit-call.loft` failed on iteration 2 with a
  stale element. **Probe 23** is the minimal form, and reproducing it needed **four**
  ingredients at once — drop any one and it passes even when widened:

  1. a callee returning a named local (dep names its own buffer),
  2. a caller whose local is set from a **struct literal** first, then **reassigned** from
     that call, then returned,
  3. the callers take **struct** parameters, not scalars,
  4. **two** such callers interleaved in one loop.

  Under the widening the corruption lands on the *other* caller's value — a shared buffer
  across two call sites — and it is **interpreter-only**, so the change also split the
  backends. Unlandable twice over.

  **Why probes 21/22 said "safe":** they varied the call SHAPE while holding nesting depth,
  the number of Sets on the returned local, the parameter KIND, and the number of interleaved
  callers all at **1**. Four composition axes pinned at once — the matrix could not see this
  cell, and a clean sweep across it read as proof. The `free_src=true` reading in probe 21 was
  accurate and still did not generalise.

  **The copy stays.** `exists()` keeps copying a `File` per call. Making it not copy needs a
  predicate that can tell a buffer that is safe to transfer from one that is about to become
  another frame's buffer — which is a real analysis, not a flag test.
- **Q6 — which of the uncovered families should be silenced rather than removed?** The owner's
  framing: prevent the copy where possible, and where it is genuinely needed allow it silently
  *but closely guarded* — i.e. an accept recorded at the site, not a blanket exemption.

### Why compile-time only (owner's decision)

No runtime checks, no runtime accounting, no runtime cost. **If a copy is really needed, loft
just does it** — silently, at full speed. The deep magnitude is deliberately *not* the
diagnostic's job, and that is the same bargain rustc makes: it tells you at compile time that
a clone happens, never how many bytes it moved. The author knows their own data.

**The guard: an emission manifest, diffed against the verdicts.** Each backend's emitter
records every copy it writes — site, type, flat size — at the moment it writes it. The guard
diffs that manifest against `use_analysis`'s classified set. **An emitted copy with no
verdict is the blind spot**, and that check is entirely static: it runs at compile time, on
both backends, and costs a compiled program nothing.

This is also strictly better than threading a site-id through `OpCopyRecord`: no ABI change,
nothing at runtime, and it carries the source position native's runtime dump does not have.

Report the copy's **site and type**. Do not report the flat size as a cost — a
12-byte-looking copy can move a megabyte, so a number that excludes the deep content teaches
the wrong thing. Existence and location are what the author can act on.

The runtime dumps (`LOFT_COPY_DUMP`, `LOFT_TRACE_COPY`) stay what they are: **developer
debugging aids**, used to *investigate* this plan. They are not part of the guard and not
part of the shipped diagnostic.

*(Supersedes three earlier revisions: per-site attribution never needs native's runtime hook
to carry a source position; the flat size is not the copy's cost; and the deep cost is not
measured at all — by design.)*

**Blocker the guard immediately exposes — PINNED.** Native is not missing the hook. It
reports record copies under a **different env flag, with a different output format**. The
documented "runtime ground truth" flag therefore covers all five copy shapes on the
interpreter and only two on native:

| copy shape | probes | `--interpret` gate | `--native` gate |
|---|---|---|---|
| record copy | 10, 11, 14 | `LOFT_COPY_DUMP` — `state/io.rs:1468` | **`LOFT_TRACE_COPY`** — `codegen_runtime.rs:636` |
| vector append | 12, 13 | `LOFT_COPY_DUMP` — `database/structures.rs:404` | same hook (shared store code) |

Verified as a clean diagonal across all ten cells. The interpreter's record copy runs
`State::do_copy_record`; native's runs `codegen_runtime::OpCopyRecord`; the vector path is
shared, which is the only reason two of the cells agree.

Three consequences, all load-bearing for the guard:

1. **The formats are not interchangeable, and neither is a superset.** Interp emits
   `[copy] record line=17 tp=65` — a **source line**, no size. Native emits
   `[copy] OpCopyRecord src=#0@1,8 dst=#1@1,8 tp=65 size=12 free_src=false` — stores and
   size, **no source position at all**. So the guard's tier-2 per-site attribution needs
   native's hook to *gain a position*, not merely be renamed.
2. **Native's gate is a raw `std::env::var` per copy**, not the cached `keys::` accessor the
   other diagnostics use — so it also pays a lookup on every copy executed.
3. This is the same one-home-per-derived-fact failure the plan keeps meeting, now in the
   measuring device: two instruments for one fact, grown separately. Unify before the guard
   can gate both backends — measuring native with the documented flag reads as copy-free.

*(Corrected: an earlier revision of this section recorded native as silent under both flags
and listed three unpinned explanations. That reading came from a run whose working directory
had been reset, so loft never opened the probe file. Native reports normally under
`LOFT_TRACE_COPY`.)*

## Tool gaps

- **The manifest is not honed to every path yet — the top instrument task.** 8 sites
  recorded (interp × 5, native × 2, one dead); the parser's ~5 IR-level `OpCopyRecord`
  emitters are not instrumented. Until they are, `uncovered = 0` cannot mean "complete".
- No probe-set runner yet; add at ≥20 probes.

Investigation-only aids (NOT shipped, NOT part of the guard — recorded so the next session
does not mistake them for gates):

- Two env flags for one runtime fact — `LOFT_COPY_DUMP` (interp record + shared vector) and
  `LOFT_TRACE_COPY` (native record), different formats. Confusing, but dev-only.
- Neither reports a deep copy's actual content; `copy_claims` has no hook. Left alone by
  design — the guard does not measure cost.
- Native's `LOFT_TRACE_COPY` gate re-reads the environment on every copy rather than using a
  cached `keys::` accessor. Worth fixing if that path ever stays in a release build.

## See also

- [loft#774](https://github.com/loft-lang/loft/issues/774) — the report. Its `b = a` half is
  not the defect; its `c = v[0]` half is cluster II.
- [loft#775](https://github.com/loft-lang/loft/issues/775) — the sibling: a field view
  escaping through a `&` write-back. Same "a view is not a store" invariant, different
  invalidator (frame exit rather than container reshape). Fixed by materialising — a
  candidate shape for cluster II.
- `doc/claude/OWNERSHIP_MODEL.md`, `doc/claude/LIFETIME.md`,
  `doc/claude/COPY_DIAGNOSTICS.md`.
