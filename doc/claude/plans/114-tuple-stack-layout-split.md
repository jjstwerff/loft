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
> **RE-SCOPED to a rewrite** (2026-07-21): tuples adopt the RECORD field format.
> Steps 0-2 done (corpus, diagnostic assert, alignment model); step 3's push-site
> workaround is REVERTED. Open work is phases **A-E** (§ Steps), sized so every
> commit leaves the tree green and either preserves behaviour or flips one named
> instrument. Success criterion is § Unification: one place left that can be wrong.
> Probe corpus:
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

## Unification — there must be no second opinion about a tuple's layout

Every symptom in this plan is one shape: **two pieces of code independently computing
where a tuple's elements go, and drifting.** Fixing the values without removing the
duplication just resets the clock. So the rewrite's success criterion is not "the
tests pass" but **"there is only one place left that can be wrong."**

### The duplicates, as found (2026-07-21)

| # | Site | What it computes | Mismatch |
|---|---|---|---|
| 1 | `data::element_align` (data.rs:1977) | per-type alignment for tuple offsets | `Function => 8` |
| 2 | `tuple_def`'s **inline** align table (data.rs:~4396) | the same thing, hand-copied | `Function => 4` — **they disagree today** |
| 3 | `data::element_size` / `element_offsets` Tuple arms | stack-width layout | `Integer` always 8B, so storage over-reserves 3.4× |
| 4 | `stored_tuple_offsets` / `stored_tuple_field_offset` | record layout via the synthetic struct | the correct one |
| 5 | `LinkedFieldGroup::group_size` / `group_member_offsets` | storage re-computation from DB widths | a third packer |
| 6 | `variables::size(Tuple)` / `variables::align(Tuple)` | stack slot size/align | delegate to 3, inheriting its errors |
| 7 | `read_tuple_at_wide` (parallel.rs) | packed → stack inflation | the only honest conversion, but private to the par worker |

Item 2 is not hypothetical: the two tables **already disagree** about `Function`
(8 vs 4). Nothing detects it, because nothing compares them. That is the whole bug
class in miniature — a copied rule that silently rots.

Per-shape patches that exist only because the rules disagree, and which must die with
them: `codegen.rs:3336` (`stack.position += stack.step(20) - stack.step(16)`, P249)
and `codegen.rs:2776` (the `#493` fn-ref `OpSetInt4` projection).

### The target shape

- **One canonical layout: the record.** The synthetic `__tuple<…>` struct is the
  single source of truth for element widths and offsets. Nothing re-derives them.
- **The stack view is DERIVED, never computed in parallel.** One named
  inflate/deflate (generalising `read_tuple_at_wide`) converts record layout → stack
  slots. If the stack view is a *function of* the record view, the two cannot drift —
  which is stronger than keeping them equal by discipline.
- **One width/alignment table.** Delete the inline copy in `tuple_def`; every caller
  reads the same function.
- **No per-shape corrections** anywhere in codegen: with one layout, the fn-ref and
  `OpSetInt4` special cases have nothing left to correct.
- **A drift guard, not a convention.** While any duplication survives a step, a test
  asserts the two agree for a matrix of element types — so the *next* copy to rot
  fails a build instead of a user's data.

### Unification is phases A4 + C + D3 + E1, not a separate pass

The removals below are distributed through § Steps rather than saved for the end —
each is its own commit with the four instruments green:

1. Delete `tuple_def`'s inline align table; call the shared function. Fixes the
   `Function` 8-vs-4 disagreement as a side effect — expect it to move emitted code,
   so check `introspect` diffs and hand-verify what changed.
2. Make the stack view a derivation of the record view; give it one public name and
   one home.
3. Collapse `variables::size(Tuple)` / `align(Tuple)` onto that derivation.
4. Delete `codegen.rs:3336` and the `#493` branch; each deletion must leave the
   corpus, matrix and guard green.
5. Add the drift-guard test for whatever duplication genuinely cannot be removed, and
   record WHY it cannot — an accepted duplicate needs a reason on the page.

**Done means:** searching for a second tuple-layout computation finds nothing, and a
reader asking "which function decides where element *i* lives?" has exactly one
answer.

## Steps — the rewrite: tuples use the record field format

> **Re-scoped 2026-07-21.** Steps 0-2 stand (they produced the corpus, the
> diagnostic assert, and the alignment model). Step 3's push-site workaround is
> **reverted** — see the § below. What replaces it is not a bigger patch but a
> smaller system: tuples stop having a layout of their own.

### The target

**A tuple is laid out exactly like a record of the same fields.** Element widths and
offsets come from the synthetic `__tuple<…>` struct that `tuple_def` already
registers, resolved through `stored_tuple_offsets` — the path whose own doc comment
says "storage reads / writes for tuple elements MUST use the same field offsets that
ordinary struct fields use … rather than recomputing via `element_offsets`".

The eval stack keeps its 8-stepped slots, because every push advances a whole slot.
That difference becomes an **explicit conversion at the boundary**, not a second
layout: the codebase already names the operation — `read_tuple_at_wide` is described
as "the par worker's tuple-arg **inflation**".

So: **one canonical layout (record) + an explicit inflate/deflate at the stack
boundary.** Today there are two layouts that silently disagree, which is the root of
every symptom in this plan.

### What this fixes, measured

Same three fields, `u8` / `u32` / `u16`, on `main` today:

| | stride | round-trip |
|---|---|---|
| `struct M { a: u8, b: u32, c: u16 }` | **7 B** | `1,2,3` → `1,2,3` ✔ |
| `(u8, u32, u16)` | **24 B** | `1,2,3` → `1,2,`**`4`** ✘ |

- **3.4× memory** — `element_size` reports STACK widths for storage (`Integer` is 8B
  regardless of `forced_size`), so a tuple reserves 8+8+8 where the record packs
  1+4+2.
- **Silent corruption** — the narrow trailing element reads back **+1**, reproducible
  in `vector<(u8,u16)>` and `vector<(u32,u16)>` as well. Read and write already
  disagree today; expect latent compensation elsewhere.
- **The two SIGSEGVs** (`e4_d2_closure_arg`, `e5_d2_struct_ref_arg`) and the
  `(character,character)` / `(boolean,·)` crashes, because caller and callee stop
  deriving placement from different functions.

### Validation instruments (every step re-runs all four)

1. **`sizeof(T)` per scalar** — `u8=1 i8=1 u16=2 i16=2 i32=4 u32=4 character=4
   single=4`. Correct today; must stay correct. Catches width inflation at the type
   level.
2. **`size(vector<T>)`** — the fully allocated size, `length × stride`. Correct today
   for every scalar (`u8×4=4`, `char×4=16`, `bool×4=4`); **wrong for tuples**
   (`(u8,u32,u16)×2 = 48`, should be ~14). This is the space regression detector.
3. **Record-vs-tuple parity** — the same fields as a `struct` and as a tuple must
   agree on BOTH stride and values. The record is the oracle; the tuple must match
   it. This is the single most important check, and it is what the whole rewrite is
   aiming at.
4. **Mixed-width round-trip** — `(u8,u32,u16)`, `(u16,u8,u32,character)` etc. stored
   and read back element-by-element, hand-computed. Catches the `+1` class.

Plus the existing gates: the 27-cell probe corpus on **both backends**, the 201
ignored matrix cells, the `stack_align_guard` build (which retired step 3), and
`loft introspect` diffs for anything claiming to be behaviour-preserving.

### The safety property

Every commit below leaves the tree **green**, and each one either **preserves
behaviour** (provable: byte-identical `loft introspect`) or **fixes exactly one
measurable thing** (provable: one named instrument flips). No commit does both, and
none leaves an intermediate state that only works once the next lands.

Steps are lettered by phase. Within a phase they are ordered; phases A and B must
complete before C.

---

## Phase A — nets and facts (no behaviour change)

### A1 — land the four instruments as a permanent test

`sizeof(T)` per scalar · `size(vector<T>)` per scalar · record-vs-tuple parity for
the same fields · mixed-width round-trip values. Assert the **target** state, so the
test starts RED on exactly the two defects and goes green as the rewrite lands.

Gate: fails only on tuple stride + mixed-width values; passes every record and scalar
case; a deliberately wrong expectation must make it fail (prove the harness can).

### A2 — pin the `+1` to read-side or write-side — **DONE: BOTH, crossed**

**Verdict: the tuple's write and read ops are a crossed pair, and each deviates from
the record in the opposite direction.** Not a layout fault at all.

For `(u8, u16)` versus `struct P { a: u8, b: u16 }`:

| | write | read |
|---|---|---|
| **tuple** | `OpSetShort` → `set_short_nullable` | `OpGetShortRaw` — returns stored bits, no decode |
| **record** | `OpSetShortRaw` — stores raw | `OpGetShortFull` — decodes |

The encoding is at `store.rs:2097`:

```rust
*self.addr_mut(rec, fld) = (val - min + 1) as u16;   // 0 reserved for the null sentinel
```

So the tuple writes `val − min + 1` and reads the bits back undecoded → exactly `+1`,
for every value and every narrow width. A clean constant shift rather than garbage is
the signature of an encode/decode mismatch, not a misread offset.

**Consequence for phase C:** this is NOT fixed by moving offsets. The element access
must select the record's op pair for the field type — which is what routing through
the synthetic struct gets, since that is where the record's own selection happens.

**But moving offsets is what found it.** Both reverted attempts earned their keep as
instruments, and the record should say so rather than file them as pure error:

1. Step 3 trimmed the push offsets. Reference and vector cells went green — and the
   `text` cells failed *differently*, in a new way. A fix that changes which cells
   break is a probe: the residue after it named the alignment axis, which is how
   `element_align(Text) = 4` and its stale "4 bytes via interned heap pointer"
   comment came to light.
2. The stepped-layout change made all 27 cells pass, which looked like success and
   was challenged on space grounds. Chasing *whether* it cost memory is what walked
   the code: `element_size` callers → the vector element stride → `size(vector<T>)`
   as a user-visible number → the mixed-width test → the 24B-vs-7B gap and the `+1`.
   None of that was reachable from the crash alone.
3. Only then did the record become an obvious oracle to diff against, and the
   `introspect` comparison of tuple vs record bytecode showed the crossed op pair in
   one read.

The lesson to carry, not just the conclusion: **a wrong fix that changes behaviour is
a legitimate diagnostic, and its value is in what still breaks afterwards.** What
made these safe to use that way was that each was cheap to revert and gated by
instruments that said plainly when the change was wrong — not that either was
correct. Revert the change; keep the map it drew.

**Persisted-data assessment** (the stop condition): the write side IS deviant, so
tuple bytes on disk differ from record bytes for the same value. But the read has
never decoded them, so **no user has ever read a correct value out of a narrow tuple
element** — there is no correct prior behaviour to stay compatible with, and no
migration to design. Fix both ends onto the record's pair. This is the one case where
"the write is wrong" does not stop the plan, and the reason is written here so the
stop condition is not silently waived.

<details><summary>original A2 text</summary>

Store a known mixed-width tuple; read the same bytes through element access and
through the record view. Diagnosis only, no edit.

Gate: a written verdict. A read fault means C is a reroute; a **write** fault means
stored bytes are already wrong and any persisted data is suspect — stop and assess
before touching layout.

### A3 — drift guard for the two alignment tables

A test asserting `data::element_align` and `tuple_def`'s inline table agree for every
element kind. **It fails today** (`Function`: 8 vs 4).

Gate: the test fails for exactly that one type, naming both values.

### A4 — delete the inline table

Route `tuple_def` at the shared function, removing the copy. A3 turns green.

Gate: A3 green; `introspect` diff reviewed — the `Function` change WILL move emitted
code, so hand-verify what moved rather than accepting a non-empty diff.

---

## Phase B — DONE, and it falsified its own premise

**Result: offsets are NOT the divergence.** Phase B was built to enumerate sites
whose element offsets disagree with the canonical layout. Instrumented and run, the
cross-check fired **nowhere** — and this time that silence is trustworthy, because
the reason is visible in the code: `stored_tuple_field_offset` (codegen.rs:38) is
already the canonical function, and both parser sites
(`fields.rs:1128`, `mod.rs:5310`) already call `stored_tuple_offsets_for_def`, using
`element_offsets` only as a defensive fallback for un-registered shapes.

So B1 was already in the tree, the B2 cross-check was deleted rather than kept (a
check that cannot fire is false comfort), and B3's inventory is empty. The
instrument's job was to tell us where to look; it told us *not here*.

### What the divergence actually is

Two things, both upstream of any offset arithmetic:

**1. The element's declared width is lost.** A tuple element keeps the alias's
*range* but drops its `size(N)`:

| | bytes per element |
|---|---|
| `struct M { a: u8, b: u16 }` | **3** (1+2) |
| `struct N { a: integer, b: integer }` | **16** (8+8) |
| `(u8, u16)` | **16** — identical to the plain-integer struct |

The synthetic struct is therefore built from `integer(0,255)` / `integer(0,65535)`
rather than from `u8` / `u16`, so it packs like `N`, not like `M`. The record oracle
is right because it keeps the alias; the tuple is wrong because it doesn't.

**2. The narrow op pair is crossed** (the A2 verdict), and it only bites the elements
that stayed range-limited:

| tuple | round-trip |
|---|---|
| `(integer, integer)` | correct — plain 8-byte int ops, correctly paired |
| `(u8, integer)` | correct |
| `(u8, u16)` | `+1` on the `u16` |

A narrow field is written with the sentinel encoding (`val − min + 1`) and read raw.
`(integer, integer)` escapes because 8-byte ints never take the narrow path.

### What this means for phase C

The target moves upstream and gets much smaller: **preserve the element's declared
type — including `forced_size` — when a tuple type is formed**, so the synthetic
struct is built from `u8`/`u16` and packs like `struct M`. That one change should
cover both defects, because the narrow op selection follows the field's type; a
field carrying its real width has no reason to pick a mismatched pair.

This is a *type-preservation* fix, not a layout rewrite — which is why every layout
theory in this plan kept almost-explaining the evidence without ever predicting it.

## Phase C — C1 attempted, measured, REVERTED; the target is now exact

### C1 — size tuple elements as record fields in `tuple_def` (reverted)

Hypothesis: `tuple_def` sizes elements with `element_size` (`Integer` => 8 flat,
discarding the alias's `size(N)`), so making it use `IntegerSpec::byte_width` — the
documented one home for storage width, "honours `forced_size` first, falls back to
the bounds-range heuristic" — would pack `(u8,u16)` like `struct M`.

**Result: no measurable change.** Stride still 32, `+1` still present, corpus
unchanged at 21, record-vs-tuple still 24 v 7. Reverted rather than kept: a change
that moves no instrument is a speculative special case, and this plan has been
punished for those three times.

**The negative result is the value.** `tuple_def`'s pre-registered size/alignment is
**inert** for every path these instruments touch — consistent with
`database/types.rs:399-407` ("for STORAGE layout we re-compute from `sizes[]`"), and
now measured rather than inferred.

### Where the two defects actually come from

- **Stride** — `parser/collections.rs:219` computes a vector's element stride as
  `element_size(inner).max(4)`. For a tuple element type that is
  `element_size(Type::Tuple)`, the sum of per-element **stack** widths (8 per
  integer) — 16 for `(u8,u16)`, hence 32 for two.
- **`+1`** — narrow op selection follows the element `Type`'s spec, which stays
  range-limited; `(integer,integer)` never takes the narrow path and round-trips
  correctly.

So both trace to `element_size(Type::Tuple)` serving two masters: the **vector /
storage stride** and the **stack slot** (`variables::size(Tuple)`). Changing it in
either direction breaks the other view — which is exactly what the two reverted
experiments demonstrated from opposite sides:

| attempt | direction | broke |
|---|---|---|
| stepped layout (reverted) | wider | storage — inflated vector strides |
| C1 / record widths (reverted) | narrower | nothing measurable, but would desync the 8-stepped stack pushes if it had |

**Conclusion: phase C cannot be done as a local edit.** The split of
`element_size(Type::Tuple)` into a storage view and a derived stack view — phases D
and E — is a *precondition* for it, not a follow-up. The plan's phase order is
therefore wrong: D/E must precede C.

## The working design — two views, one home each, and the exact code points

Written after the oracle made the target measurable (108 -> 19 divergent shapes) and
after four candidate edits were tried and judged by it. Every claim below is backed
by a measurement or a read, not by inference.

### The two views (both are legitimate — the bug is that they share a name)

| view | rule | one home | who needs it |
|---|---|---|---|
| **STORAGE** | record packing: `IntegerSpec::byte_width` per element (`forced_size` first), packed TIGHT | `stored_tuple_offsets` / `element_storage_size` | tuple in a record, a vector row, a DB page, a worker message |
| **STACK** | one `aligned_stack_step` slot per element — a push occupies a whole slot whatever the alias says | `element_offsets` / `element_size` | tuple in a frame slot or on the eval stack |

They must differ: `(u8,u16)` is **3 bytes** stored and **16** on the stack. The defect
was never that two views exist — it is that both were called `element_offsets`, so a
site could consult the wrong one silently. **Rename them** (`tuple_storage_offsets` /
`tuple_stack_offsets`) as the first act, so every remaining site declares which it
means and a wrong choice reads wrong.

### Code points, classified

Enumerated by reading every call site, not by grep-and-guess.

**Legitimately STACK-side — address a tuple in a frame slot; leave alone:**

| site | what |
|---|---|
| `codegen.rs:604` | `TupleGet` from a stack tuple variable |
| `codegen.rs:705` | `TuplePut` into one |
| `codegen.rs:1272` | `emit_tuple_var_push_recursive` |
| `codegen.rs:1323` | `emit_tuple_var_pop_put` |
| `codegen.rs:1363` | `emit_tuple_null_init` |
| `codegen.rs:2188` | `emit_tuple_put_ops` |
| `codegen.rs:2839` | the `#493` fn-ref `d_nr` projection (`stack.function.stack(tvar) + …`) |
| `codegen.rs:3374` | `generate_var` whole-tuple read |

**Already STORAGE-correct — prefer `stored_tuple_offsets_for_def`, fall back only for
un-registered shapes:** `collections.rs:1920/2576/2585`, `mod.rs:5317/5398`,
`expressions.rs:2814/2902`, `fields.rs:1134`, `codegen.rs:46`.

**WRONG — read STORAGE bytes with STACK offsets:**

| site | what |
|---|---|
| `parallel.rs:1568` | `read_tuple_at_wide` walks an in-vector row using `element_offsets` |
| `generation/ops/parallel.rs:208` | the native par-worker twin of the same read |

Those two are the only outright misclassified consumers, and they explain the
par-worker "smearing" note at `state/mod.rs:4797` as the same family.

### The three remaining defects and where each is fixed

**1. `+1` on narrow elements — the crossed op pair.** `typedef.rs:710-730` picks the
schema Part by NULLABILITY (`Short` = `+1` sentinel write, `ShortRaw` = direct), and
its own comment records an identical off-by-one already fixed once for `u16 not null`.
The tuple element READ selects `OpGetShortRaw` (the narrow-vector split in
`collections.rs:367`), so a nullable element is written shifted and read raw.
**Fix at the pair, not either end:** the read-op choice must be derived from the
schema Part the write used, so the twin table in `operators.rs:475-484` stays the
single source of pairing.

**2. The last 19 shapes — fn-ref alignment.** Full tight packing is correct for every
shape except those carrying a fn-ref, whose 8-byte `d_nr` truncates to 4 without its
boundary (#493, P249). `Parts` cannot separate it: `(u8,integer)` and the fn-ref case
are both `Parts::Base`, size 8, align 8, with opposite requirements — measured, and
Parts-keying scored 29 vs 19. **Fix by making the requirement explicit:** carry a
per-element "needs its own alignment" flag from the type (a fn-ref does; a plain
integer does not) instead of inferring it from size/align/Parts.

**3. Storage read via stack offsets** — convert `parallel.rs:1568` and
`generation/ops/parallel.rs:208` to the storage view. Both backends, one commit each.

### Order, and why

1. **Rename the two views** — no behaviour change, makes every later diff self-explaining.
2. **Fix the op pair** (defect 1) — the `+1` is silent corruption and independent of layout; it needs no other step.
3. **Convert the two par-worker readers** (defect 3) — isolated, and the guard/matrix cover them.
4. **Add the explicit fn-ref alignment flag, then tighten fully** (defect 2) — oracle to 0.
5. **Delete what is then unreachable**, and flip the oracle to `assert_eq!(defects, 0)`.

Each step is judged by: the oracle count, the four instruments, corpus + 201 matrix,
the `stack_align_guard` build, and `loft_suite`. A step that moves no instrument is
reverted — that rule has already saved this plan four times.

## The oracle — stop guessing, compare (2026-07-21)

Three attempts to fix this by editing a plausible site each returned "no measurable
change".  That is not three unlucky guesses; it is the search being at the wrong
altitude.  `tests/pln114_layout_oracle.rs` replaces guessing with a comparison: for a
matrix of element-type shapes it computes three answers side by side —

| column | meaning |
|---|---|
| **record** | size of `struct { f0: T0, f1: T1, … }` — GROUND TRUTH, records already pack and round-trip correctly |
| **tuple** | what the running system gives `(T0, T1, …)` |
| **D1-new** | what `data::element_storage_size` computes |

Run: `cargo test --test pln114_layout_oracle -- --nocapture`.

### First run

```
125 shapes compared · 108 where tuple != record · 0 where D1 != record · 0 skipped
```

**1. The defect is far wider than the crashes suggested.** 108 of 125 shapes, not the
handful the probe corpus found:

| shape | record | tuple |
|---|---|---|
| `(u8, u8)` | 2 | **4** |
| `(u8, u16)` | 3 | **16** |
| `(u16, u8)` | 3 | **10** |
| `(u8, boolean)` | 2 | **3** |
| `(u8, u32, u16)` | 7 | **24** |
| `(u8, u8, u8, u8)` | 4 | **8** |

The 17 agreeing shapes are exactly those where the stack width already equals the
storage width (`character`/`single`/`boolean` pairs, plain `integer`).

**2. The replacement routine is already correct — everywhere.** `D1-new` matches the
record for **all 125 shapes**, including all 108 defective ones. So
`element_storage_size` is validated as the target layout *before* being wired to
anything, which is what the previous three attempts lacked: each changed a site
without an independent statement of what the right answer was.

### What this changes about the remaining work

The question is no longer "what is the correct layout" — that is settled and tested.
It is only **which consumers still read the wrong one**, and the oracle turns each
conversion into a measurable step: the `tuple` column must move to the `record`
column, shape by shape, with the count in the summary line as the ratchet.

Flip the test from inventory to assertion (`assert_eq!(defects, 0)`) when the count
reaches zero.

## Phase D — D1 landed; D2 attempted twice with no effect

### D1 — the storage view exists and is tested — **DONE**

`data::element_storage_size` / `element_storage_offsets`: a tuple element sized as a
record FIELD (`IntegerSpec::byte_width` — `forced_size` first, else the range),
packed **tight**, because records have no alignment padding (`struct M` measures
1+4+2 = 7) and store access is unaligned-tolerant.

Additive and unused by production code. Unit-tested against hand-computed values,
including a test that pins the two views apart so they cannot be confused later:

| tuple | storage view | stack view |
|---|---|---|
| `(u8, u32, u16)` | offsets `[0,1,5]`, size **7** | size **24** |
| `(u8, u16)` | offsets `[0,1]`, size **3** | size 16 |
| `(integer, integer)` | 16 | 16 — they agree, which is why this shape never broke |

### D2 — two attempts, both no-change

Both were reverted; neither moved a single instrument.

1. **`tuple_def` sizes elements by `byte_width`** (this was C1). No change — the
   registered group size is inert.
2. **`db_type` honours `forced_size` itself** rather than requiring callers to
   pre-check the alias. `Type::size`'s own doc says "forced_size is handled by the
   callers", which is exactly the split responsibility this plan removes — so this
   looked like the unification fix. No change either.

What the traces DID establish, by instrumenting `tuple_def` rather than guessing:

```
@PLN114 tuple_def element 0: Integer(0, 255, false, size(1)) -> element_size=8 storage=1
@PLN114 tuple_def element 1: Integer(0, 65535, false, size(2)) -> element_size=8 storage=2
```

**`forced_size` survives all the way to `tuple_def`.** The information is present and
correct at the point the synthetic struct is built; something downstream discards it.
`element_storage_size` reads it correctly (1 and 2) — so D1's function is right and
the remaining question is purely *who consumes it*.

### Next — instrument, do not guess

Two consecutive no-change edits mean the altitude is wrong. The synthetic struct's
field sizes are decided somewhere between `add_attribute` and `finish_type`, and the
vector stride reads `self.database.size(db_tp)` for that struct
(`parser/collections.rs:219`, the `else` arm). The next step is to trace where that
size becomes 8 per element **before** changing anything else — a third guess is not
warranted, and the instruments have been reliable at saying so.

## Phase E — deletion and reach

### E1 — remove the dead Tuple arms

`element_size` / `element_offsets` Tuple arms go (or are renamed to the stack view
they actually are). The A3 drift guard becomes trivially true.

Gate: all four instruments green; full suite; `introspect` diffs non-empty only where
a layout was genuinely wrong before. Searching the tree for a second tuple-layout
computation finds nothing — the § Unification success criterion.

### E2 — extend the guard's reach, then un-ignore

Add `tuple_matrix` and the sibling matrix suites to the `stack_align_guard` CI job
(it was blind to this whole class). Then drop `#[ignore]` from the 201 cells and wire
them into the nightly, advisory for the first few nights.

Gate: three consecutive green nightlies on all three OSes before dropping advisory.

---

### Stop conditions

- Any instrument regresses → stop. The rewrite may not trade space for correctness or
  the reverse.
- A step needs the DB page format changed → stop and re-plan; the record layout is
  the oracle *because* it is already right.
- A2 says the WRITE is wrong → stop and assess persisted data before changing layout.

</details>
- A phase-C site resists conversion and the fix wants a special case → that is the
  old bug returning under a new name; write the reason down and re-plan instead.

<details><summary>the R0-R7 sketch these replaced (same intent, phase-sized rather than step-sized)</summary>

### Step R0 — land the validation tests FIRST

Turn instruments 1-4 into a permanent test (a `.loft` script plus a Rust harness
entry), asserting the **target** state. On `main` today the record cases pass and the
tuple cases fail, so the test starts RED on exactly the defects being fixed and goes
green as the rewrite lands. Nothing else in this plan is safe without it.

Gate: the test fails only on tuple stride + mixed-width values, passes everything
else; a deliberately wrong expectation must make it fail (prove it can).

### Step R1 — pin the `+1` bug to read or write

Store a known mixed-width tuple, then read it through both paths (element access and
the record view of the same bytes). Determine whether the WRITE lays the bytes down
correctly and the READ misinterprets, or the write is already wrong.

Gate: a written verdict naming the side. R2's shape depends on it: a read-only fault
is a reroute; a write fault means the stored bytes are wrong and any existing data is
suspect.

### Step R2 — route tuple ELEMENT ACCESS through the synthetic struct

Replace `element_offsets`-based element addressing with
`stored_tuple_offsets` / `stored_tuple_field_offset` wherever the tuple lives in a
record, vector or DB page. That is the reroute onto the already-correct path; no new
layout code.

Gate: instrument 3 (record-vs-tuple parity) goes green for stride AND values;
instruments 1, 2, 4 green; matrix and corpus no worse than the recorded baseline.

### Step R3 — make the stack boundary an explicit inflate/deflate

Give the stack view an honest name and one home (extend `read_tuple_at_wide`'s
concept to both directions). Caller and callee both derive the frame block from the
record layout plus that conversion, so they cannot disagree.

Gate: the corpus goes 27/27 on both backends, the matrix 201/201, and the
`stack_align_guard` build reports **zero** fires — the check that retired step 3.

### Step R4 — delete the second layout

Remove the `Tuple` arms of `element_size` / `element_offsets` (or rename them to the
stack-inflated view they actually are), so no caller can pick the wrong one. Triage
the 26 non-`data.rs` `element_offsets` callers as storage or stack while doing it.
See § Unification for the full duplicate inventory this must clear — R4 and R7 are
the same work seen from two angles (deleting the wrong layout / leaving one home).

Gate: all four instruments green; full suite; `loft introspect` diffs reviewed for
every corpus cell — non-empty only where a layout was genuinely wrong before.

### Step R5 — extend the guard's reach

Add `tuple_matrix` and the sibling matrix suites to the `stack_align_guard` CI job.
The gate existed and was blind to this class because its suite list excludes them.

### Step R6 — un-ignore

As the old step 7: drop `#[ignore]` from the 201 matrix cells and wire them into the
nightly, advisory for the first few nights.

### Stop conditions

- Any instrument regresses → stop; the rewrite is not allowed to trade space for
  correctness or vice versa.
- A step needs the DB page format changed → stop and re-plan; the record layout is
  the oracle precisely because it is already right.
- The `+1` verdict says the WRITE is wrong → stop and assess existing persisted data
  before changing anything.


</details>

<details><summary>the original steps 0-7 (superseded by the rewrite; 0-2 still describe what was done)</summary>

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

**Deferred: tuples carrying `text`** — diagnosed in step 4 below. Applying the
relocation to them turns a clean SIGSEGV into *heap corruption*. The fix therefore
skips any tuple with a `text` element — those cells stay broken exactly as before,
loud and no worse — and `e4_d2_closure_arg` (`(fn, text)`) remains the one failing
matrix cell.

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

### Step 4 — what step 3 left broken: `text` at a non-8-aligned offset

**Diagnosed; needs its own design. It is NOT a defect in the argument push.**

The bytecode step 3 emits for `(P,text)` is exactly what was intended — `P` at block
offset 0, `FreeStack(0,4)` to trim, text at +12, `ReserveFrame(4)` to pad, and
`Call(args_size=32)`. The placement is right and it still corrupts, because **+12 is
not a legal address for a `text`**.

`data::element_offsets` is the RECORD layout, where `element_align(Text) = 4`. On
the stack a `text` is a `Str { *const u8, u32 }` whose raw pointer needs 8-byte
alignment — `variables::align` says so explicitly, and calls itself "deliberately
STRONGER than `data::element_align` (which aligns the record-stored `Str` to 4);
records keep their own weaker layout on the `element_align` path." Placing a text
element at a 4-mod-8 offset misaligns its pointer; freeing it then hits
"refused to free the stack store (#306)" / invalid `free()`.

The rule predicts every cell, and it is **independent of step 3** — these cells
failed the same way before it:

| cell | text's packed offset | 8-aligned? | result |
|---|---|---|---|
| `int_text` | 8 | yes | pass |
| `text2` | 16 | yes | pass |
| `ref_text` | 12 | **no** | fail |
| `fn_text_read` / `fn_text_call` | 20 | **no** | fail |

So the defect is in the **layout convention for stack-resident tuples holding
pointer-bearing elements**: the packed record layout is not sound there, whoever
writes it. That is why step 3's guard is the right boundary rather than a hack —
correcting the push cannot fix a position that is illegal for the type.

Fixing it means choosing one of:

1. **Align tuple elements by `variables::align` on the stack** — correct, but that
   is the "second offsets function" this plan rejected in § The invariant, and it
   desyncs stack tuples from record tuples.
2. **Raise `element_align(Text)` to 8 everywhere** — one rule, but it changes the
   RECORD and DB-page layout, i.e. the on-disk format. Almost certainly too wide.
3. **Forbid the shape** — reject a tuple that would place a pointer element at a
   non-8-aligned offset, at parse time, until 1 or 2 is designed. Turns silent
   corruption into a clear error and is cheap.

This wants its own plan (or an explicit phase here) with its own matrix over
pointer-bearing element kinds — `text`, and anything else whose stack alignment
exceeds its record alignment. Not started.

<details><summary>original step 4 text (superseded — it assumed the remaining work was more push sites)</summary>

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

</details>

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


</details>

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
