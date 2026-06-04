<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 02 — Layout-driven yield codec (the full channel collapse)

**Status: BUILT + validated (scalar + DbRef-ref slice).**  The layout-driven
flatten-walk is implemented (`src/coroutine_layout.rs` + the four wiring sites);
three previously-`--native`-broken composite shapes — `(integer, float)`,
`(integer, boolean)`, `(vector<integer>, integer)` — now compile and run
correctly on both backends through the single walk with **zero per-shape code**,
`coroutine_matrix` 18/18 green on both backends, no regression.  `(text, …)` is
the one excluded cell (a text element's native repr is `&str`, needing a store
intern — `codegen_runtime::db_from_text` — with the lifetime question that
entails: a separate slice).  This was produced as the *with-arm* of the
[`DESIGN_VERIFICATION.md` § C1](../../DESIGN_VERIFICATION.md) predict-validate
protocol — see § Provenance.  Supersedes the migration tail of
[01-unified-channel.md](01-unified-channel.md): phase 01 unified the
*transport* (one `next_into` writing into one `[i64]` buffer); this phase
unifies the *codec* (how a value enters and leaves that buffer), which phase 01
left per-shape.

## The invariant

> **The yield buffer is `T`'s slots flattened into *transport* form: each scalar
> slot inline as an `i64`; each reference slot (`text` / `vector` / `struct` /
> `closure` / nested composite) as its *full absolute* `DbRef`
> (`store_nr`,`rec`,`pos`), packed.  Every site that touches the buffer — the yield
> write (producer), the `next_into` read (consumer), and the eager-collect factory
> — derives the same flatten-walk from `T`'s slot kinds.  No site hand-rolls a
> per-shape codec.**

Both ends know `T` statically (`iterator<T>` is concrete at every use — loft
monomorphises; there is no dynamic-yield case, including `iterator<closure>`,
which is a `DbRef` like any record).  So producer and consumer derive the *same*
flatten-walk from the *same* `T`, and **agree by construction** — the runtime tag
that currently communicates the shape is redundant and is deleted.

**Tested, not assumed — the first probe falsified the original framing.**  An
earlier draft of this invariant said "the buffer is `T`'s *store-layout* image."
Probing the hardest cell — the producer for `(u32, DbRef)` — showed it packs the
**full absolute `DbRef`** (`generation/coroutine.rs:638–642`:
`dest[0] = (u32) | (store_nr << 32); dest[1] = rec | (pos << 32)`), whereas a store
*field* of reference type is a 4-byte store-*relative* ref.  So the codec is the
value's **transport ABI** (the full pointer), **not** the store-field layout.  The
collapse claim survives the refinement — the flatten-walk is still *one* codec
derivable from `T` — but the per-slot encoding is transport form, and the
distinction is **byte-exact-critical**: the packing must match producer↔consumer
exactly (the `i64`-packing of sub-`i64` slots is where "derive from `T`" has to be
exact, not approximately-the-same).  This is the design-as-hypothesis discipline in
action: the framing would have shipped on assertion; the probe caught it.

## What collapses

| Today (phase 01 tail) | Mechanism | After |
|---|---|---|
| `next_i64` / `next_text` / `next_dbref` legacy trait methods | one per scalar-class / text / ref | **deleted** — `next_into` is the sole channel |
| Per-shape decode *templates* in `OpCoroutineNextEmitter` (tuple-of-int, `(u32,DbRef)`, …) | one hand-written template per shape, selected by the tag | **one layout-driven emit** keyed on `T`'s field layout |
| `value_size` high-byte **channel tag** | static shape encoded as a runtime discriminant | **deleted** — both ends derive layout from `T` |

These three are *one family* — the per-shape **yield codec** — so the single
flatten-walk invariant makes all three degenerate.

**Probe 2 trimmed this family.**  An earlier draft added a fourth row claiming the
eager-collect factory's `@P325` (17 GB store-offset overflow) was the same family
— "hand-rolled buffer offsets."  Testing the claim (reading `@P325`'s root cause)
falsified it: `@P325` was a **missing loop-termination check** for coroutine
sources in `build_comprehension_code` (the infinite loop appended until the store
hit its 2 GiB limit — the "17 GB" was the *symptom*), a **control-flow** bug,
already FIXED, unrelated to the codec.  Pulling it in was **over-unification**
(C1's *wider than the domain* failure — making the collapse look bigger by
absorbing a distinct case); the probe caught it and trimmed the family back to the
three codec rows above.

## Matrix consequence

Phase 01 collapsed the **Y axis transport** but left the codec per-shape, so the
matrix was still partly *constitutive* (a tuple cell green because a tuple template
exists).  With the layout codec the **Y axis is fully confirmatory**: a new
composite yield type (another tuple arity, a struct, a future shape) is absorbed
with **zero** new code — its slot kinds already exist — so its row is green *by
construction*, not by a new template.  That is the value: the per-shape codec
spray stops growing combinatorially with yield shapes (the `@PLAN15` closure
problem the plan's own § Why cites).

This is **cohesion / future-proofing, not unblocking** — and probe 2 is why the
distinction is now explicit.  The previously-blocked comprehension cells (`y1_x4`,
`y2_x4`) were blocked on `@P324` / `@P325`, **both control-flow/guard bugs, both
already FIXED** — *not* the codec; the collapse does not turn them green (they
already are).  The cells the codec actually governs are the composite ones still
pending: the tuple-through-higher-order cells (`y4_x3`, `y4_x4`, phase 04) and
every future composite yield, which the flatten-walk makes uniform.

## Implementation sequence

1. **Layout source.**  Expose the yielded type's **slot kinds** (scalar vs
   reference, per slot) to codegen at the yield site *and* the `next`/factory sites
   — the schema `Stores`/`Data` already hold this.  The slot kind is all the codec
   needs: scalar → inline `i64`; reference → the full-`DbRef` packing.  (Both sites
   have `T`; this is plumbing the kinds, not inventing them — and note it is *not*
   the store-field's 4-byte ref, per the probe above.)
2. **One emit.**  Replace the per-shape templates in `OpCoroutineNextEmitter`
   (and the producer's yield-write, and the factory's collect) with a single
   layout walk: `for slot in T.layout { buf[slot.i] = <read/write per slot> }`.
3. **Delete the tag.**  `value_size` keeps only the buffer size (for stack
   allocation, derivable from the layout); the high-byte channel field is removed,
   along with the interp mask sites (`fill.rs::coroutine_next`, the slot-allocator
   `OpCoroutineNext` arm) that exist only to strip it.
4. **Delete the legacy methods.**  Remove `next_i64`/`next_text`/`next_dbref`
   once every shape routes through `next_into`.
5. **Verify the collapse on the frozen matrix** (`tests/coroutine_matrix.rs`):
   every Y×X cell green on both backends with **no new code per yield shape** — the
   composite cells (`y4_*`) green via the one codec, and **no regression** of the
   already-green scalar / text / comprehension cells.

## Risks / what to verify (do not assert these)

- **`@P325` is NOT in this family (probe 2, resolved).**  An earlier draft listed it
  as a codec-family member.  Reading its root cause falsified that — a missing
  loop-termination check (control flow), already fixed; pulled in by
  over-unification, now excluded.  The standing caution: when the factory *is*
  migrated to the codec, confirm the migration doesn't reintroduce a per-shape copy
  path (the factory's value-copy is a legitimate codec site; its old *offset*
  symptom was not the codec).
- **Nested / recursive layouts.**  A `struct` field that is itself a `struct`
  stores a ref (4 bytes) — the codec stays one level deep, which is correct *iff*
  every composite is a store ref.  Verify no yield shape inlines a nested
  composite into the buffer.
- **Endianness / `i64`-packing of sub-`i64` slots.**  The `(u32,DbRef)` template
  packs two u32s into one `i64` (lines 74–77).  The layout walk must reproduce the
  exact packing the Store uses, or producer/consumer disagree — this is the one
  place "derive from layout" must be byte-exact, not approximately-the-same.

## Provenance — the C1 with-arm, and design-as-testable-hypothesis

Produced *with* the predict-validate protocol, as the measured counterpart to @PLN9
(the control arm).  Two sub-decisions were **predicted before the code was opened**:
(a) the tag is subtractable because both ends know `T` statically; (b) the blocked
cells are one path, possibly two mechanisms.  Both held on first inspection.

Then the design itself was **treated as a hypothesis and probed** — *a design is a
variant of an assumption you can test.*  Two probes, two refinements:

- **Probe 1 — the invariant's framing.**  "The buffer is `T`'s *store-layout* image"
  was **falsified**: the `(u32,DbRef)` producer packs the *full absolute DbRef*, not
  a store-field's 4-byte ref.  The invariant became "transport-ABI flatten"; the
  collapse survived, the framing did not.
- **Probe 2 — the family's boundary.**  The `@P325` subsumption was **falsified**:
  `@P325` was a missing loop-termination check (control flow), already fixed, not
  buffer math.  It was pulled in by **over-unification** — C1's own
  *wider-than-the-domain* failure, committed by the author and caught only by the
  probe, not by re-reading the prose.

That second catch is the result worth keeping: the prediction *located* the
invariant, but only the probes kept it *matched to the domain* — without them the
doc would have shipped a real cohesion fix wrapped in a false "and it fixes @P325
too."

- **Probe 3 — empirical, the premise.**  A `(text, integer)` yield — a `(ref,scalar)`
  mixed shape the spray does not cover — was run on both backends.  **interp:**
  correct (`out=abb n=30`, uniform).  **native:** `error[E0605]: non-primitive cast:
  (&'static str, i64) as i64` — the producer's scalar arm fired on the whole tuple
  for lack of a template.  This **confirms** (not falsifies) the design's premise:
  the brittleness is real and native, and the interp's uniformity proves the
  flatten-walk target is reachable (same program, correct, zero per-shape code).
  Regression-in-waiting: `/tmp/coro_mixed.loft` (graduate to `coroutine_matrix.rs`
  when the walk lands).

The one remaining test no desk reasoning settles: **build** the flatten-walk and
confirm `(text,integer)` (and the other composite cells) compile + run native with
**zero shape-specific code** — and the already-green scalar/text/int-tuple cells
do not regress.  That build is a real codegen change (the producer `next_into`
match → per-element walk; the consumer emitter templates → per-element decode;
delete the tag + legacy methods), and it carries this design's own probe list
(byte-exact producer↔consumer agreement; ref-elements store-then-pack; no nested
inlining) — i.e. it deserves the full predict-validate-with-a-matrix pass, not a
tail-of-session edit.

## See also

- [01-unified-channel.md](01-unified-channel.md) — phase 01 (transport unified).
- [00-matrix.md](00-matrix.md) — the Y×X cell space this collapses.
- [COROUTINE.md](../../COROUTINE.md) — frame design + the `iterator<T>` contract.
- [DESIGN_VERIFICATION.md § C1](../../DESIGN_VERIFICATION.md) — the protocol this run tests.
