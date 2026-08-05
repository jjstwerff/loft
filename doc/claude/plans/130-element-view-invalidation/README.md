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

Two tiers, in order:

1. **Counting gate — ship first, no op change.** Compare executed copies against classified
   copies per run. `executed > classified` means copies nobody was told about. This fires on
   probes 10, 11 and 12 today, and it is the check that would have prevented the report ever
   printing `none` on a copying program.
2. **Per-site attribution — later.** Requires `OpCopyRecord` to carry a site-id; the op
   already carries a packed flag (`0x8000` free-source) in its type word, so a site-id is the
   same shape. The interpreter dump already emits `line=`, so half exists.

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

- The guard above — not built; both halves exist.
- `--native`'s record-copy dump carries **no source position** (`codegen_runtime.rs:636`),
  so tier-2 attribution is blocked until it gains one.
- Two env flags for one runtime fact (`LOFT_COPY_DUMP` / `LOFT_TRACE_COPY`), different
  formats, split by backend *and* by copy shape. Unify before either can gate.
- Native's gate re-reads the environment on every copy instead of using a cached
  `keys::` accessor.
- No probe-set runner yet; add at ≥20 probes.

## See also

- [loft#774](https://github.com/loft-lang/loft/issues/774) — the report. Its `b = a` half is
  not the defect; its `c = v[0]` half is cluster II.
- [loft#775](https://github.com/loft-lang/loft/issues/775) — the sibling: a field view
  escaping through a `&` write-back. Same "a view is not a store" invariant, different
  invalidator (frame exit rather than container reshape). Fixed by materialising — a
  candidate shape for cluster II.
- `doc/claude/OWNERSHIP_MODEL.md`, `doc/claude/LIFETIME.md`,
  `doc/claude/COPY_DIAGNOSTICS.md`.
