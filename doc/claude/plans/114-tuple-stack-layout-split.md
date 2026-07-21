# Tuple layout: one owner for element placement

> **@PLN114** — [loft-lang/plans#114](https://github.com/loft-lang/plans/issues/114).
> Status: design. Nothing here is implemented — the investigation below stopped at
> the design gate deliberately (see § What the first attempt got wrong). The probe
> corpus is live: `114-tuple-stack-layout-split/probes/` (`./run.sh`).

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

## The invariant

> **A tuple is ONE packed buffer. Its element offsets are `data::element_offsets`
> in every location a tuple can live — record, DB page, stack frame, eval stack,
> par-worker message. The stack's uniform-8 step applies to the tuple as a whole,
> never to its elements.**

An untested cell is then correct for the same reason a tested one is: nobody
computes element placement, everybody reads it from one function.

The alternative invariant — *stack tuples are uniform-8-stepped per element, with a
separate `stack_element_offsets`* — is rejected: it needs a second offsets function,
must be threaded through all 26 non-`data.rs` `element_offsets` sites, and would
desync the par-worker buffer that `state/mod.rs:4797` already fixed toward packed.
More mechanism, more sites, against the precedent.

## Re-assertion sites (the prospective tell)

The invariant is only safe if **one** place decides how far a tuple element push
advances. Today that decision is spread across every op that can push an element,
and omitting it is **silent** — a wrong offset, not a compile error. Counted:

- `element_offsets`: 39 sites, 26 outside `data.rs` (codegen 12, parser 12,
  `parallel.rs` 1, `generation/ops/parallel.rs` 1, `state/mod.rs` 1).
- `element_size`: `parallel.rs` (30), `native.rs` (16), `data.rs` (13),
  parser (11), `sandbox.rs` (2), `database/format.rs` (1), `variables/mod.rs` (1).

`N` is large, so the design must **not** rely on remembering it at each site. Two
cures, both used below: collapse the *advance* decision to one helper (step 3), and
make omission **loud** before changing anything (step 1) so any site that forgets
trips an assert instead of corrupting a frame.

Note the seam already exists: `variables::size(tp, context)` takes
`Context {Argument, Reference, Result, Constant, Variable}` — all stack/frame
contexts. It is *already* the stack-side sizing function; its `Tuple` arm reaching
into `data::element_size` is the single cross-regime leak.

## Open question — is the `text` family the same bug?

**Almost certainly not — the corpus now says so.** Three cells crash with *no*
assert: `(P,text)`, `(fn,text)` read-only, `(fn,text)` with a call. By the width
model they should be fine: packed `(P,text)` = 12 → text aligns 8 → offset 16,
total 32; pushed = 16 + 16 = 32. **The widths match and it still segfaults**, so a
width fix may not touch them. `Str { *const u8, u32 }` is a raw pointer into a
buffer, so the likely mechanism is ownership/lifetime of a text element inside a
tuple (a dangling `ptr` after the source is freed), not placement.

**The decisive evidence is a backend split.** Running the corpus on both backends
(`probes/run.sh`, 2026-07-21):

- width family (`ref2`, `ref3`, `vec2`) — mismatch on **both** `--interpret` and
  `--native`;
- text family (`ref_text`, `fn_text_read`, `fn_text_call`) — SIGSEGV on
  `--interpret`, **correct output on `--native`**.

Shared layout *metadata* would hit both generators equally, because they read the
same IR and the same `element_offsets`. A fault only one backend shows is in that
backend's value delivery, not in the layout the two share. Treat the text family as
a separate defect unless step 2 proves otherwise.

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

### Step 1 — make the mismatch loud (no emitted-code change)

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

### Step 2 — decide the `text` family

Probe, do not reason: run `(P,text)` under `LOFT_LOG=ref_debug` and the lifetime
inspector (`--show-ownership`, `LOFT_STORES=timeline`) and read whether the `Str`
pointer is dangling at the crash. Compare against `(text,text)`, which passes.

Gate: a written verdict. Same root → it stays in this plan and step 3's matrix must
include it. Different root → file it separately with its own matrix, and this plan
explicitly does **not** claim it. Do not proceed to step 3 with this unanswered —
it decides what "fixed" means.

### Step 3 — one owner for the element advance (interpreter read path only)

Narrowest real change: in `codegen.rs:3316` (`Type::Tuple` read-by-element), stop
letting each op's natural step decide, and advance to the next
`element_offsets[i+1]` instead. One helper, one place; the ad-hoc fn-ref correction
at `3336` should become redundant — leave it in place for now and assert it is a
no-op rather than deleting it (deletion is step 5).

Gate: `(P,P)`, `(P,P,P)`, `(vector,vector)` now match; the coincidence cells
`(P,integer)`/`(integer,P)` still pass with *correct hand-computed values*; the full
201-cell matrix green on `--interpret`; leak-clean under `--interpret`
(`check_store_leaks` needs `--interpret` — bare `loft` is native and skips it).

### Step 4 — remaining stack-side sites

Route the other stack-side element pushes/reads through the same helper, one commit
per site group (codegen tuple-get/put; the `#493` `OpSetInt4` branch; parser paths
that compute a stack offset). `parallel.rs` and `database/format.rs` are **record**
consumers — they must not change; if a change there is ever needed, the invariant is
wrong and the plan stops.

Gate per commit: the corpus stays green on both backends, and the introspect diff is
non-empty **only** for the site touched.

### Step 5 — delete the fixups

Remove `codegen.rs:3336`'s `step(20) - step(16)` and, if step 4 subsumed it, the
`#493` special case. This is the payoff: the accretion goes away because the rule is
now in one place.

Gate: byte-identical introspect against the end of step 4 for every corpus cell
where the fixup was already a no-op, and hand-checked values where it was not.

### Step 6 — native backend parity

`src/generation/` is a separate generator reading the same IR; an interpreter fix
that leaves native diverging is not landable. Re-run the whole corpus and the
201-cell matrix on `--native` with `LOFT_NATIVE_LEAK_CHECK`.

Gate: both backends emit the proven layout and pass value + length + leak.

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
