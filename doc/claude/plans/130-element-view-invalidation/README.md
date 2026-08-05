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
| A — Probe catalogue | 🟡 14 probes written + run on both backends; producer/invalidator axes not yet exhausted |
| B — Mechanism investigation | 🟡 2 of 5 clusters verified to a code line |
| C — Fix design | ⏸️ pending B — the model is chosen (below), the enforcement point is not |
| D — Implementation | ⏸️ pending C |

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

   Consequence: draining the `Avoidable` set is a **prerequisite of the plan**, not a
   follow-up. Default-on is only livable once a clean program is quiet — otherwise the
   accepts become noise-suppression rather than statements of intent.
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

Runner: `probes/run_set.sh [view|copy|secret|all]` — per probe, pass/uncovered/executed-copies
on both backends.

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

  **Still not verified — the one case that could still break it:** whether any shape keeps two
  results from the SAME call site simultaneously live. Reassignment cannot (each call displaces
  the previous binding) and a container insert deep-copies on the way in, but that is reasoning,
  not a sweep. Enumerate it before changing `return_adopts_fresh_store` — this is the last gate,
  and it is a bounded case analysis rather than an open question.
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
