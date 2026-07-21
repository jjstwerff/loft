# Tuple layout: the call-argument push violates the packed layout

> **Reframed after step 0.** This started as "split two competing layout regimes and
> give tuples one owner". The probe corpus showed there is no competition to
> resolve: **packed already wins at every destination** — local, struct field,
> return, `TuplePut` — and the `Call` op's own `args_size` and the callee's
> `ReserveFrame` agree with it. Exactly one path disagrees: the call-argument push,
> which derives element placement implicitly from each op's 8-rounded step instead
> of reading `element_offsets`. So this is a **repair of a lone violator**, not a
> design. The steps below are sized for that.

> **@PLN114** — [loft-lang/plans#114](https://github.com/loft-lang/plans/issues/114).
> **Steps 0-3 DONE** (step 3 except `text`); steps 4-7 open. One of the two
> original SIGSEGVs is fixed; the matrix is 200/201. Probe corpus:
> `114-tuple-stack-layout-split/probes/` (`./run.sh`), baselines in
> `bytecode-comparisons/`.

## Symptom

`tests/tuple_matrix.rs::e4_d2_closure_arg` and `::e5_d2_struct_ref_arg` SIGSEGV on
`--interpret`. Both are `#[ignore]`d, so the crash is invisible in CI; it surfaced
only when the whole ignored set was run (2026-07-21). `DESIGN_DECISIONS.md:318`
names `e5_d2_struct_ref_arg` as one of the six cells that *lock* the tuple
move-semantics decision — so the guarantee has been unenforced, not merely untested.

Under the @PLN85 debug-assertions calibration build the fault attributes itself
instead of segfaulting:

```
src/state/codegen.rs:2820: generate_call [n_main]: mutable arg 0
  (t: Tuple([Reference(640), Reference(640)]))
  expected 24B on stack but generate(Tuple([Var(0), Var(1)])) pushed 32B
```

## What is actually wrong

Two layout regimes both claim tuple element placement:

| Regime | Rule | Where it is defined |
|---|---|---|
| **Packed / record** | each element at its natural alignment; `DbRef` aligns 4 | `data::element_size`, `data::element_offsets` |
| **Stack uniform-8** | every push advances `aligned_stack_step` (round up to 8) | @PLAN53 S4, `variables::aligned_stack_step` |

A tuple's *declared* stack size and its *element offsets* come from the packed
regime — `variables::size` (`src/variables/mod.rs:2129`) delegates its `Tuple` arm
to `data::element_size`, and every reader locates elements with
`data::element_offsets` (e.g. `codegen.rs:2793`, `codegen.rs:3320`). But a tuple is
*pushed* element-by-element through ordinary ops, so each element advances by its
own stepped width. `DbRef` is 12B packed, 16B stepped.

The two agree only by coincidence, which is exactly what the boundary matrix shows:

| cell | packed (declared) | stepped (pushed) | result |
|---|---|---|---|
| `(P,P)` | 12+12 = **24** | 16+16 = **32** | assert 24 v 32 |
| `(P,P,P)` | 36 → step → **40** | 16·3 = **48** | assert 40 v 48 |
| `(vector,vector)` | **24** | **32** | assert 24 v 32 |
| `(P,integer)` | 12+8 = 20 → **24** | 16+8 = **24** | **passes — by coincidence** |
| `(integer,P)` | 20 → **24** | 8+16 = **24** | **passes — by coincidence** |
| `(integer,integer)`, `(text,text)`, `((int,int),int)` | 16 / 32 / 24 | 16 / 32 / 24 | pass |

The passing mixed cells are two errors cancelling. Any change to either regime
moves them, so they are the cells to watch, not the crashing ones.

**This has bitten before, and the precedent picks the winner.** `state/mod.rs:4797`
documents the identical class in the par-worker path: byte-by-byte `put_stack::<u8>`
advanced `stack_step(1) = 8` *per byte*, "smearing the packed tuple buffer (p.0@0,
p.1@8) across 16 separate 8-byte slots; the worker body then reads tuple fields at
the raw `element_offsets` [0, 8] into that smeared layout → padding zeros → every
worker returned 0." The fix was a block copy "to keep the buffer byte-identical to
the raw layout the worker reads." The packed buffer was treated as canonical there,
and the same choice is available here.

Two more standing fixups are the same pressure, already in the tree:

- `codegen.rs:3336` — `stack.position += stack.step(20) - stack.step(16)` after
  `OpVarFnRef`, hand-correcting one element kind's advance (P249).
- `codegen.rs:2776` — the `#493` branch projecting only a fn-ref's 8-byte `d_nr`
  for `OpSetInt4`, whose comment already describes the failure as "imbalances the
  eval stack … a `generate_call` width assert under debug-assertions."

Per-shape fixups accreting around one missing rule is the tell.

## Scope: the call-argument push is the ONLY broken destination

Step 0's destination axis (2026-07-21, `probes/run.sh`, both backends) narrows this
a long way. With two `Reference` elements:

| destination | cell | result |
|---|---|---|
| local | `ref2_local` | pass |
| struct field | `ref2_field` | pass |
| function return | `ref2_return` | pass |
| element mutation (`TuplePut`) | `tupleput_ref` | pass |
| **call argument** | `ref2`, `ref3`, `ref4`, `vec2` | **mismatch** |

So the packed layout is not merely *declared* — every other destination already
**lays tuples down packed and reads them back correctly**. Only the argument push
walks the elements through ops whose natural step is 8-rounded. The callee agrees
with the packed side: `bytecode-comparisons/ref2.before.txt` shows `n_f` doing
`ReserveFrame(size=24)` and reading `var[0]` / `var[12]`, while the call site's two
`VarRef` pushes advance `[32]` → `[48]`, i.e. 16 apart.

The arity model is confirmed linear at n = 2, 3, 4 (24v32, 40v48, 48v64) — 4B lost
per reference, no fixed header term.

**This is the over-broad check from the design protocol.** The universal rule
("everything reads `element_offsets`") is *true*, but rewriting every producer to
enforce it would be blast radius far past the defect, which sits in a single
consumer. The special circumstance is that the consumers already hold the right
information — the callee's frame is already packed-shaped. So the fix is to make the
**argument push** lay elements down at `element_offsets`, like the four destinations
that already work; the 26 `element_offsets` sites stay untouched.

## The invariant

> **A tuple that is consumed DIRECTLY as a memory block — a record, a DB page, a
> callee's frame, a par-worker message — must have its elements at
> `data::element_offsets`. The stack's uniform-8 step applies to such a block as a
> whole, never to its elements.**

**Amended at step 1, after the first form was falsified.** The original wording said
"in every location a tuple can live", which sounds cleaner and is wrong: a tuple
*bound to a local* is relocated element-by-element into the variable's slots by
`emit_tuple_put_ops` (`codegen.rs:2346` → `2141`, which reads `element_offsets`
itself). Its intermediate eval-stack placement is scratch and legitimately does not
match. A check written to the wide form fires on `ref2_local` and `tupleput_ref`,
both of which are correct at runtime.

What separates the two is the **consumer**: a block read directly as memory must be
packed; a block that gets relocated on the way to its home need not be. That is why
step 1's assert is conditioned on `in_call_arg` rather than applied to every tuple.

An untested cell is then correct for the same reason a tested one is: nobody
computes element placement, everybody reads it from one function.

The alternative invariant — *stack tuples are uniform-8-stepped per element, with a
separate `stack_element_offsets`* — is rejected: it needs a second offsets function,
must be threaded through all 26 non-`data.rs` `element_offsets` sites, and would
desync the par-worker buffer that `state/mod.rs:4797` already fixed toward packed.
More mechanism, more sites, against the precedent.

## Re-assertion sites (the prospective tell)

**An earlier draft of this section was wrong and the correction matters.** It counted
`element_offsets` (39 sites, 26 outside `data.rs`) and `element_size` (`parallel.rs`
30, `native.rs` 16, …) and called that count the brittleness — "N is large, so the
design must not rely on remembering it at each site."

Step 0 falsified that. Those sites are **already consistent with each other**; not
one of them is a violator. `N` is not the risk here, and a fix that "routes all N
through a helper" would be churn against working code.

The real brittleness is a different shape, and `N = 1`: exactly one path
**derives** element placement (implicitly, from each op's 8-rounded stack step)
where every other path **reads** it (`element_offsets`). A derived fact that must
coincide with a stored one, with nothing checking the two agree, is the defect —
and when they disagree the failure is **silent** (`ref4` returns
`2,null,0,360287970323857408`, no crash) or a SIGSEGV, never a compile error.

So the cure is not "collapse N sites" but: make the derived path *read* the stored
fact (step 3), and make any future divergence **loud** (step 1) — still worth doing,
now justified by the silent corruption rather than by a site count.

Note the seam already exists: `variables::size(tp, context)` takes
`Context {Argument, Reference, Result, Constant, Variable}` — all stack/frame
contexts. It is *already* the stack-side sizing function; its `Tuple` arm reaching
into `data::element_size` is the single cross-regime leak.

## RESOLVED (step 2) — the `text` family is the SAME bug, and `text` was a red herring

Verdict: **same root, no separate plan.** The distinguishing axis was never `text`;
it is the **packed alignment of the element that follows a sub-8-byte element**.

`element_align` (`data.rs:1977`) gives record-stored `Str` **4**, `boolean` 1,
`character` 4, while `integer`/`float`/`Function` are 8. So after a `Reference`
(12B, align 4) the packed offset of element 1 is:

- **16** when the next element aligns 8 → coincides with the 8-stepped push → passes
- **12** when it aligns ≤4 → the push lands at 16 → **broken**

Predicted, then measured (`probes/ref_float`, `ref_bool`, `ref_char`):

| cell | next element | predicted | measured |
|---|---|---|---|
| `ref_float` | `float` (align 8) | pass | pass — `10,2.5` |
| `ref_bool` | `boolean` (align 1) | fail | **SIGSEGV**, assert `+16 vs +12` |
| `ref_char` | `character` (align 4) | fail | assert `+16 vs +12` |

`ref_bool` is the decisive one: **no `text` anywhere** and it crashes identically.
Two earlier readings were wrong — "widths match at 32" (computed with stack
alignment 8 for text instead of the record's 4) and "a backend split proves they
differ". The lifetime hypothesis is also falsified by cells already in the corpus:
`(text,text)` and `(integer,text)` both pass, so text elements are not inherently
unsafe.

The model now predicts every cell: a tuple argument breaks exactly when an
element's packed offset differs from the 8-stepped cumulative push position. It
also explains `fn2` passing (`Function` is 20B/align 8 → packed `[0,24]`, push
`step(20)=24`) while `fn_text_*` fails (`[0,20]` vs push 24).

<details><summary>the original open question (superseded)</summary>

**Almost certainly not — the corpus now says so.** Three cells crash with *no*
assert: `(P,text)`, `(fn,text)` read-only, `(fn,text)` with a call. By the width
model they should be fine: packed `(P,text)` = 12 → text aligns 8 → offset 16,
total 32; pushed = 16 + 16 = 32. **The widths match and it still segfaults**, so a
width fix may not touch them. `Str { *const u8, u32 }` is a raw pointer into a
buffer, so the likely mechanism is ownership/lifetime of a text element inside a
tuple (a dangling `ptr` after the source is freed), not placement.

**A backend split was claimed here and it was wrong — do not reuse that argument.**
An earlier draft said the width family failed on *both* backends while the text
family failed only on `--interpret`, and used that contrast to separate them. The
contrast was an artifact: the width `debug_assert` lives in `generate_call`, i.e. in
**codegen**, which runs before either backend executes — so it fires under
`--native` too while the native *runtime* is perfectly correct.

Measured at runtime with the release binary (2026-07-21):

| cell | `--interpret` | `--native` |
|---|---|---|
| `ref2` | SIGSEGV | `10,20` ✔ |
| `ref4` | `2,null,0,360287970323857408` (silent garbage) | `1,2,3,4` ✔ |
| `vec2` | SIGSEGV | `7,9` ✔ |
| `ref_text`, `fn_text_*` | SIGSEGV | correct ✔ |

**Every** failure in this corpus is interpreter-runtime only; `--native` is correct
throughout. That further localizes the defect to the interpreter's bytecode
argument-push (which is what step 3 targets) and makes step 6 cheap — native already
works and only has to be kept that way.

What still separates the two families is weaker but real: the width family has an
arithmetic model confirmed at n = 2, 3, 4 plus a codegen assert that names it, while
the text cells' widths already agree (32 both ways) and nothing fires. Step 2 still
decides it by probe, and the expectation is "different root, separate plan" — but on
the width evidence, not on a backend split.

</details>

Absorbing these into the layout story because they are in the same test file is
precisely the over-unification this design must avoid. Step 2 decides it with a
probe, and if the root differs they get their own plan.

## Steps

Each step lands on its own, is separately revertable, and states the gate that must
pass before the next one starts.

### Step 0 — probe corpus + baselines (no code change)

**Partly done** — `plans/114-tuple-stack-layout-split/probes/` holds the 18 cells
run so far (`./run.sh` reports each against its hand-computed expectation on both
backends). Extend it with one `.loft` per remaining cell,
each printing hand-computed values (never "both backends agree" — that is not a
pass). Cells, beyond the 16 already run:

- **arity**: 2, 3, 4 refs; 1-element tuple if the grammar allows.
- **kind × position**: ref/vec/fn/text/int/float/bool/char in slot 0 and slot 1 of
  a 2-tuple, and in the middle of a 3-tuple (position is a composition axis; the
  middle slot is the one padding errors hide in).
- **destination**: local, arg, return, struct field, vector element, nested tuple
  element, par-worker input (`state/mod.rs:4797`'s path — it shares the regime).
- **mutation**: `TuplePut` into each element kind, then read back.
- **cardinality**: 2 vs 3 refs distinguishes a per-element error (scales with n)
  from a constant one — the existing data says per-element (8B deficit at n=2 and
  n=3 came from 16-vs-12 per ref, not a fixed header).

Gate: every cell runs on **both** backends with hand-checked values; the crashing
cells crash identically under the release and DA builds; `loft introspect` captured
for each cell into `bytecode-comparisons/` (it carries IR + bytecode + native Rust,
so one capture covers both backends).

### Step 1 — make the mismatch loud (no emitted-code change) — **DONE**

Landed as a per-element check in `codegen.rs`'s `ValueType::Tuple` arm, conditioned
on a debug-only `State::in_call_arg` flag set around argument generation in
`generate_call`. Absent from release builds entirely — no field, no flag, no cost.

Two premises in the original text were wrong and are corrected here:

- *"widen the assert past `a.mutable`-only"* — the `generate_call` width assert was
  never the gap. It stayed silent on the text cells because their totals genuinely
  agree; it compares only the aggregate, so a tuple can push the right total with
  every element misplaced. The gap was per-element, not per-argument.
- *"put it at the tuple read path"* — the first attempt instrumented
  `generate_var`'s `Type::Tuple` arm and fired on **nothing**, including `ref2`.
  These cells pass tuple *literals*, which are built by `ValueType::Tuple`
  (`codegen.rs:452`, "generate each element onto contiguous stack slots" — with no
  offset control at all). A dead-path sentinel reads exactly like a healthy one;
  only a known-failing cell that *must* fire distinguishes them.

Calibration (the reason to trust it): fires on all 9 runtime-broken corpus cells,
silent on all 15 runtime-OK ones, and across the **201 ignored matrix cells** under
a debug-assertions build — 199 passed, and the only 2 failures are the 2 already
known broken. Zero false positives on the whole population. Byte-identical
`introspect` on all 24 corpus cells confirms nothing emitted changed.

Gate note learned the hard way: `#[cfg(debug_assertions)]` code is **not compiled**
by `cargo build --release`, so the byte-identical gate passed once on an assert that
did not compile. Always build the DA binary too.

<details><summary>original step 1 text (superseded)</summary>

The width assert at `codegen.rs:2820` only guards `a.mutable` args, which is why
two of the three crash families reach a SIGSEGV instead of a message. Widen it:

- assert for **every** call argument, not just mutable ones;
- add the same expected-vs-actual assert at each tuple **push** site, comparing the
  advance against `element_size`;
- keep all of it `#[cfg(debug_assertions)]`.

Gate: **byte-identical `loft introspect` before/after** on the step-0 corpus — the
asserts are debug-only, so an empty diff is the proof nothing emitted changed. Then
the DA build must report a width mismatch for `(P,P)`, `(P,P,P)`, `(vector,vector)`
and stay silent for the 199 passing cells. A positive control is required: if the
new asserts fire nowhere, they are on a dead path, not proof of health.

</details>

### Step 2 — decide the `text` family — **DONE: same bug, no separate plan**

Probe, do not reason: run `(P,text)` under `LOFT_LOG=ref_debug` and the lifetime
inspector (`--show-ownership`, `LOFT_STORES=timeline`) and read whether the `Str`
pointer is dangling at the crash. Compare against `(text,text)`, which passes.

Gate: a written verdict. Same root → it stays in this plan and step 3's matrix must
include it. Different root → file it separately with its own matrix, and this plan
explicitly does **not** claim it. Do not proceed to step 3 with this unanswered —
it decides what "fixed" means.

### Step 3 — lay the argument push down packed — **DONE except `text`**

Implemented in codegen's `ValueType::Tuple` arm. The layout is taken from the
**declared parameter type** at the call site (`generate_call` computes
`element_offsets` + the frame total and hands them down via
`State::arg_tuple_offsets`) — the callee's own view, not the inferred element types.
After each element the step padding is reclaimed with `OpFreeStack(0, n)` so the next
lands where the callee reads it, and the block total is matched to the frame.

The total misses in **both** directions, which the first attempt got wrong: a
trailing step pad overshoots (`(P,P)` ends at 28 for a 24B frame) *and* packing
sub-8 elements can undershoot (`(P,text)` ends at 28 for a 32B frame). `(P,P,P)`
happens to land exactly on 40 and passed even while `(P,P)` failed — a coincidence
that briefly made the fix look right. Both directions are now corrected, trimming
with `OpFreeStack` and padding with `OpReserveFrame`.

**Result:** `e5_d2_struct_ref_arg` — one of the two original SIGSEGVs — is fixed.
The 201-cell matrix is 200/201; corpus is 23/27 with the 4 remaining being the
text-carrying cells. Fixed cells are value-correct and leak-clean under
`--interpret`.

**Deferred: tuples carrying `text`.** Applying the relocation to them turns a clean
SIGSEGV into *heap corruption* — "refused to free the stack store (#306)", invalid
`free()`, out-of-bounds index. A stack `text` element's ownership is tracked by its
stack POSITION (`State::text_positions`; `free_stack` prunes that range), so moving
the element desyncs the bookkeeping. Placement and ownership have to move together,
and that is a design step, not an increment. The fix therefore skips any tuple with
a `text` element — those cells stay broken exactly as before, loud and no worse —
and `e4_d2_closure_arg` (`(fn, text)`) remains the one failing matrix cell.

That deferral is the honest boundary of this step: the mechanism is proven for 6 of
the 9 broken corpus cells and demonstrably wrong for the other 3.

<details><summary>original step 3 plan (superseded by the above)</summary>

Per § Scope, this is the whole defect. In the tuple element-push path used for call
arguments (`codegen.rs:3316`'s `Type::Tuple` arm, reached via `generate_call`), stop
letting each op's natural step set the next element's position and advance to
`element_offsets[i+1]` instead — the layout the callee's frame already expects.

One helper, one place. The ad-hoc fn-ref correction at `codegen.rs:3336` should
become redundant; leave it and assert it is a no-op rather than deleting it
(deletion is step 5).

**The target bytecode is already captured** — the codegen gate is satisfied before
the edit, from a real runnable artifact rather than a guess. `ref2_local` (working)
stores the tuple's elements **12 apart** (`var[56]`, `var[68]`); `ref2` (broken)
pushes them **16 apart** (`[32]` → `[48]`) while its own
`Call(args_size=24, fn=n_f)` and the callee's `ReserveFrame(size=24)` both say 24.
Two of the three parties already agree; step 3 makes the third agree. Diff
`bytecode-comparisons/ref2.before.txt` against `ref2_local.before.txt` while working.

Gate: `ref2`/`ref3`/`ref4`/`vec2` match; the four coincidence cells
(`ref_int`, `int_ref`, `int_ref_int`, `ref_int_ref`) still pass with *correct
hand-computed values*, not via a compensating adjustment; the four already-working
destinations (`ref2_local`, `ref2_field`, `ref2_return`, `tupleput_ref`) show a
**byte-identical** introspect diff — they are not supposed to move; full 201-cell
matrix green on `--interpret`; leak-clean under `--interpret` (`check_store_leaks`
needs `--interpret` — bare `loft` is native and skips it).

</details>

### Step 4 — only what step 3 proves is still broken

**Do not pre-emptively route the other sites.** The destination axis says they are
already correct, so touching them is blast radius without a defect. Re-run the full
corpus after step 3 and extend *only* to a site the corpus still shows failing —
candidates in likelihood order: the `#493` `OpSetInt4` branch, then par-worker input
(`state/mod.rs:4797`'s path, which shares the regime but was fixed toward packed
already).

`parallel.rs`, `database/format.rs` and the DB format are **record** consumers and
must not change. If a change is ever needed there, the invariant is wrong and the
plan stops.

Gate per commit: the corpus stays green on both backends, and the introspect diff is
non-empty **only** for the site touched.

### Step 5 — delete the fixups

Remove `codegen.rs:3336`'s `step(20) - step(16)` and, if step 4 subsumed it, the
`#493` special case. This is the payoff: the accretion goes away because the rule is
now in one place.

Gate: byte-identical introspect against the end of step 4 for every corpus cell
where the fixup was already a no-op, and hand-checked values where it was not.

### Step 6 — native must not regress (it is already correct)

Cheaper than first written: `--native` produces the right answer for all 24 corpus
cells today, so there is no parity to *achieve*, only to keep. `src/generation/` is
a separate generator reading the same IR, and an interpreter fix that breaks it is
not landable.

Gate: re-run the whole corpus and the 201-cell matrix on `--native` with
`LOFT_NATIVE_LEAK_CHECK`; every cell that passes today still passes, value + length
+ leak. Watch for the codegen-time width assert specifically — it fires in *both*
backends' runs because it lives in `generate_call`, so "the assert stopped firing
under `--native`" is part of this gate, not evidence about native's runtime.

### Step 7 — un-ignore and wire in

Drop `#[ignore]` from the tuple/closure/binary-io/template/coroutine matrices (201
cells, ~10s on Linux — the "too heavy for the default path" header comment is stale
and should be corrected in the same commit), and add them to the nightly. Land as
**advisory** (`continue-on-error`) for the first few nights: they have never run on
Windows or macOS and platform fallout should not red the nightly before triage.

Gate: three consecutive green nightlies on all three OSes, then drop advisory.

## Validation that applies to every step

- **Both backends, always** — `--interpret` and `--native`. Divergence on the same
  IR *is* the bug; "the suite is green" does not close a rung.
- **Value AND length AND leak.** A delivery that doubles a buffer reads as
  leak-free; only a value/length check catches it.
- **Hand-computed expectations.** Agreement between two binaries is not a pass. (In
  the first matrix run my own hand-computed value for `(fn,fn)` was wrong and loft
  was right — the discipline is what caught it, and it cuts both ways.)
- **Prove the harness can fail.** A cell that produces no output is vacuous.
- **Stale-artifact check.** A surprising cell is a stale-lens suspect before it is a
  bug: the debug rlib, release rlib, wasm rlib and fixture cdylibs each lag
  independently. Rebuild the lens behind the surprising cell and re-read.
- **DA build for anything assert-guarded.** Ordinary builds compile every lib-side
  `debug_assert!` out (`[profile.dev.package.loft] debug-assertions = false`), so
  "green" says nothing about a DA-guarded claim. Use the separate target dir; never
  set `RUSTFLAGS` against the main `target/`.

## Stop conditions

Revert and re-plan rather than pushing through if:

- a change makes one backend pass and the other regress — not landable;
- the patch starts growing per-element-kind conditions — that is the current bug
  reappearing, and the fact belongs in the layout function;
- `parallel.rs` / `database/format.rs` / the DB format need to change — the
  invariant is wrong;
- the coincidence cells (`(P,integer)`, `(integer,P)`) can only be kept green by a
  compensating adjustment — that means the two regimes are still both live.

## What the first attempt got wrong (keep this)

The first proposed fix was a one-liner: change `variables::size`'s `Tuple` arm to
sum *stepped* element widths. It would have been wrong. The readers locate elements
with `data::element_offsets` (packed), so inflating the declared size desyncs every
reader from its offsets and breaks the 199 cells that pass today.

What caught it was the codegen rule — *read the emitted bytecode on both backends
before editing the compiler* — not a test run and not re-reading the prose. The
elegant one-line story was the signal to look harder, not permission to act.
