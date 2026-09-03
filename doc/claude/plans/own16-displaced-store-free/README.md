<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# D-own-16 — a binding releases what it displaces, when the displacing value is known

> **STATUS (2026-09-03): step 0 RUN, step 0.5 FIXED; steps 1-6 not started.** Takes its `@PLN` number when filed as a
> `loft-lang/plans` issue. Closes the two open shapes of
> [`formal/ownership.md`](../../formal/ownership.md) `D-own-16`, and with them two thirds of the
> per-execution-witness cluster ([QUALITY.md](../../QUALITY.md)); `D-clo-7` and `D-clo-14` are the
> third and are **out of scope here** — see § What is not in this family.
>
> Every number below was measured on `9528ddc4` before the design was written. No step in
> § Steps has been run.

## What is broken, in one line

A nullable heap local never releases the store its reassignment displaces, when the value being
assigned READS that local — `c = bump(c, i)` and `c = mk(i) ?? c`. Nine stores per ten rounds,
both backends, **values correct throughout**, so only the leak channel speaks.

## The measurements this design rests on

**The cure is already in the IR and already emitted — for the DENSE twin.** `c: S` reassigned
from a callee that reads it generates, verbatim:

```rust
{ let _old_c: DbRef = var_c;
  var_c = n_mintd(cell, var_c, var_i, var___ref_1);
  if _old_c.store_nr != var_c.store_nr { OpFreeRef(cell, _old_c, "var_c(prev)"); } };
```

Stash the old ref → compute the value → free the old one only if distinct. That is
`stash_old_for_post_free` → `OpFreeRefIfDistinct` (`src/state/codegen.rs:2240` and `:2647`), and
it is exactly the release-after-the-value-is-computed that the `D-own-16` entry names as its own
cure. It leaks nothing.

**The nullable local never reaches it, and the axis is the LOCAL's `?` alone.**
`owned_ref` (`src/state/codegen.rs:2218`) opens with a `matches!` over the **unpeeled** type:

```rust
matches!(stack.function.tp(v),
    Type::Reference(_,_) | Type::Enum(_,true,_) | Type::Vector(_,_))
|| is_keyed(stack.function.tp(v).base())
```

`c: S?` is `Optional(Reference(S))` — it matches no arm, and the `base()` peel was added for the
keyed kinds only. So `owned_ref == false` and the *whole* free machinery is skipped: no pre-free,
no stash, no post-free. The IR confirms it — `c(1):ref(S)? = n_bump(c(1), i(4));` carries no free
at all. One axis at a time:

| local | return | leak |
|---|---|---|
| `S`  | `S`  | none |
| `S?` | `S`  | **9** |
| `S`  | `S?` | none |

The **local's** `?` is the whole difference; the return's nullability is irrelevant.

**The callee's ownership fact is already carried, and the leaks split exactly along it.**

| callee | return type | leak |
|---|---|---|
| `keep` — always borrows its argument | `ref(S)["s"]?` | none |
| `mint` — always mints | `ref(S)?` | **9** |
| `maybe` — mints or borrows per call | `ref(S)["s"]?` | **4** |

So the entry's sentence — *"nothing static separates the arm that MINTS from the arm that hands
back a caller's store, because they are the same call"* — is true of row 3 and **false of rows
1–2**, where `(O-Move)`'s `{Attr(param)}` already says. Correct that sentence when this lands.

**The peel alone is unsound, but for ONE reason, not the three the register lists.** Probed:

| borrow kind | deps on the local | reachable by a peel? |
|---|---|---|
| a local that borrows a PARAMETER (`d: S? = p`) | `ref(S)["p"]?` — **non-empty** | **no** — the dep clause already excludes it |
| a local a lambda CAPTURES | `ref(S)?` — **empty**, flags read `OWNS` | **yes — live exposure** |

The capture case is real and measurable: `g = fn() -> integer { c?.x ?? 0 }` answers `1` after
`c` has been reassigned three times, which is the closure reading the *capture-time* DbRef. Free
the displaced store there and `g()` reads freed memory. `is_skip_free` does **not** cover it
(`LOFT_VAR_TABLE` prints `c … def OWNS` with no override).

⚠ **`OpFreeRefIfDistinct` does not rescue this**, and that is the trap to name: it guards against
*aliasing the new value*, not against *not owning the old one*. Distinctness is not ownership.
The capture case is distinct and still must not be freed.

## The invariant

> **A binding releases the store it displaces at the point the displacing value is KNOWN, and
> only where it is that store's sole owner at that point.**

Two clauses, each carrying one of the two failure modes, and neither sufficient alone:

- **Placement (when).** After the RHS is evaluated, never before. Freeing before is a
  use-after-free whenever the RHS reads the binding — which is why the code declines today, and
  why declining is *required* rather than conservative.
- **Licence (whether).** Sole ownership at that point. Not "the deps list is empty" (a proxy that
  reads `OWNS` for a captured local), and not "the new store differs" (distinctness, which the
  capture case satisfies while still being wrong).

"At that point" is why a *static* fact cannot discharge it in general: the same binding is a
borrow on the first iteration and an owner afterwards. `rbuf_witness` (`src/scopes.rs:130`) is
that per-run fact and already exists — loft#1200 built it for the plain reassignment and proved
it there.

## Re-assertion sites — the prospective tell

`@FR-O-Proxy` resolves to **14 citation sites** (`python3 scripts/rule_tags.py sites
@FR-O-Proxy`). `owned_ref` at `src/state/codegen.rs:2218` is a **15th, uncited** implementation
of the same question — and it is the one carrying the peel bug. Omitting the rule at any of them
is **silent**: a leak or a use-after-free, never a compile error.

`N = 15 × silent` is the brittleness, known before any code. The cure is the first of the two the
protocol offers — **collapse N toward 1**, not thread the fix through the sites. There is already
a one-home for exactly this question:

```rust
// src/scopes.rs:5651
fn owns_freeable_store(&self, function: &Function, data: &Data, v: u16) -> bool {
    function.tp(v).depend().is_empty()
        && !function.is_skip_free(v)
        && (!function.is_argument(v) || self.is_promoted_ret_buffer(function, data, v))
}
```

whose own doc-comment records that it was written three times inside `free_vars` and each copy
had to be extended separately (loft#688, loft#1022, loft#1078). `owned_ref` restates its first
two clauses verbatim and spells the third as `!is_hidden_buf_arg`. **That is the same rule at two
sites with two spellings** — the condition under which the fix belongs in the shared home rather
than at the call site. Folding it is what makes the peel a one-line change in one place instead
of a fifteen-site sweep, and it retires part of `D-own-26` on the way.

## What is not in this family — the over-unification guard

The tempting sentence is *"the join and the call are the same bug and one mechanism covers
both."* It is **false**, and merging them would be the characteristic over-reach:

- **`c = mk(i) ?? c`** is desugared with the mint arm in its own named temp — the IR reads
  `__ncc_1 = n_mk(i); if OpConvBoolFromRef(__ncc_1) __ncc_1 else c(1)`. **Both arms are visible
  before the merge**, so this shape is closable *statically* by sinking the Set into the arms. It
  needs no witness at all.
- **`c = bump(c, i)`** hides the same choice inside the callee. For `keep` and `mint` the return
  deps decide it; for `maybe` nothing at the call site can, by construction.

They share a *symptom* and a *placement* fix, not a *licence* fix. Step 4 therefore lands them as
two commits with two guards, and step 5 removes the witness from the join shape only.

**The hard floor: this design does not make `D-own-16` static, and no IR rewrite would.** The
`maybe` row — a callee that mints or borrows per call — is undecidable at the call site;
measured, 4 leaked stores. It closes by *witness*, and it is the cluster's shared mechanism.
Anyone reading this hoping SSA deletes the witness should read that row first.

## Steps

Each step is one commit, independently revertable, with its own exit criterion. Steps 1–3 are
**behaviour-preserving** and prove it by byte-identical emit; only step 4 changes what runs.

### Step 0 — the falsifier, RED before anything is touched

Write `tests/scripts/own16-a-binding-frees-what-it-displaces.loft` with the cells below, and a
negative-control file for the shapes that must **stay** clean. Run both on both backends.

| cell | today | after |
|---|---|---|
| `c: S?`, `c = mint(c,i)` (callee reads it, always mints) | 9 leaked | 0 |
| `c: S?`, `c = mk(i) ?? c` (self-referential join) | 9 leaked | 0 |
| `c: S?`, `c = maybe(c,i)` (conditional borrow) | 4 leaked | 0 |
| `c: S`, dense twin | 0 | 0 (unchanged) |
| `c: S?`, `c = keep(c,i)` (always borrows) | 0 | 0 (unchanged) |
| `d: S? = p`, `d = mint(d,i)` (borrows a param first) | 3 leaked | 0 |
| **control** — lambda captures `c`, then `c` is reassigned | `g()` = `1` | `g()` = `1`, no UAF |

⚠ The corpus leak gate hard-fails a leaking script, so the RED cells cannot live in the suite
until step 4 closes them. **There is no `@IGNORE` annotation** (this doc said there was; the
annotations are `@NAME` / `@TITLE` / `@ARGS` / `@EXPECT_ERROR` / `@EXPECT_WARNING` /
`@EXPECT_FAIL`), and `@EXPECT_FAIL` is not the escape either: `tests/native.rs:929` drops any
file *declaring* the tag from the native suite wholesale, and step 0 needs both backends
(filed as loft#1311 — the fn-level skip that should have covered this cannot parse the
documented `@EXPECT_FAIL: <reason>` form).
`SCRIPTS_LEAK_ALLOW` was deliberately driven to empty, so grandfathering is out too. The probe
therefore lives at `probes/own16-matrix.loft` and is run by hand; it moves into `tests/scripts/`
in the step-4 commit that turns it green, and **that move is the guard's falsification record.**

**Exit:** every RED cell reproduces on `--interpret` AND `--native` with
`LOFT_NATIVE_LEAK_CHECK=1`; the control's `g()` answers `1`.

#### Step 0 RESULTS (2026-09-03, `9528ddc4`) — DONE

Interpreter, one struct type per cell so the report attributes per cell:

```
Warning: 5 stores not freed at program exit:
  kt=81 MintOnly×9, kt=82 JoinSelf×9, kt=83 MaybeBorrow×4, kt=84 ParamBorrow×3, kt=86 Captured×3
```

Every cell matches its prediction; `KeepOnly` and `Dense` are absent, i.e. 0. All seven values
are correct on the interpreter, which is the entry's *"only the leak channel speaks"*, confirmed.

**Probe defect found and fixed before recording.** Cells 4 and 6 first measured **6** where the
shape leaks **3**, because they called the subject twice — `assert(f() == n, "got {f()}")` runs
`f` in the condition *and* in the message, and loft evaluates the message eagerly. Call once into
a local and assert on that. A probe that double-counts reads exactly like a defect twice as bad
as the one you have, and a later half-fix taking 6 to 3 would read as progress while changing
nothing.

⚠ **Step 0 did not reach a native leak number, because the run fails an ASSERTION first.** See
the next section — that is the finding, and it is not `D-own-16`.

### Step 0.5 — a NATIVE silent wrong answer, found by step 0, and NOT `D-own-16`

Cell 3 answers **8 on `--interpret` and 0 on `--native`**. It is a wrong VALUE, with no
diagnostic and no crash — `silent-wrong`. The `D-own-16` entry cannot mention it because the
entry only ever measured the leak channel.

**It is shipped, not a branch regression.** The installed `loft 2026.8.0` release binary
diverges identically (`interpret=2`, `native=0`).

**The boundary — two conditions, and nullability is not one of them:**

| shape | verdict |
|---|---|
| nullable, conditional borrow, loop, self-reassign | **DIVERGE** |
| nullable, conditional borrow, **straight-line** | **DIVERGE** — the loop is not the axis |
| nullable, conditional borrow, **fresh** destination | agree |
| **DENSE**, conditional borrow, self-reassign | **DIVERGE** — so not a nullable defect |
| nullable, **unconditional** borrow (`-> M["s"]` always) | agree |
| conditional borrow returning a **DIFFERENT** store (`f(c, other, i)` → `other`) | agree |

So it needs (1) a callee that returns its argument on *one* path and a fresh record on the other,
and (2) the result assigned back to **the same binding that was passed in**.

**The mechanism, read off the emitted native.** The caller's adopt-vs-copy guard discriminates
on `store_nr` alone:

```rust
let _dst = var_c;
let _src = n_f(cell, var_c, 3_i64, var___ref_2);
if _src.store_nr == u16::MAX || _src.store_nr != var_c.store_nr {
    …adopt _src, free the displaced _dst…
} else {
    var_c = OpDatabase(cell, _dst, 81_i32);      // reallocates THROUGH _dst …
    OpCopyRecord(cell, _src, var_c, 32849_i32);  // … then copies FROM _src, same store
}
```

On the borrow arm `_src` **is** `var_c`, so the `else` leg runs — and `_dst` is that same store,
so `OpDatabase` reallocates over the record `_src` names before `OpCopyRecord` reads it. The copy
sources a record the reallocation just invalidated, and the field reads back `0`. The
different-store row above is the control that isolates it: change `_src`'s store and the same
`else` leg is correct.

**Why it belongs in this plan rather than beside it.** That guard is the *same emitted construct*
step 4 teaches the nullable local to reach. Widening `owned_ref` without fixing this would route
more shapes into a leg that is already wrong — the leak would close and the wrong answer would
spread. **So step 0.5 lands before step 4.**

**It also changes step 4's exit criterion for cell 3:** `4 → 0` on the leak channel is not
sufficient; the value must be `8` on both backends. A cell scored only on leaks would have
passed this defect through.

Family, all closed, none covering the self-reassignment axis: #1017 (an accessor returning a
borrow on one path and a fresh record on another), #1082, #982, #1140.

#### FIXED 2026-09-03 — the witnessed arm REPLACED the default instead of refining it

`src/generation/dispatch.rs`. The adopt condition has two forms and they disagreed about one
disjunct:

| arm | condition | same-store passthrough? |
|---|---|---|
| `None` (plain return) | `_src.store_nr == u16::MAX \|\| _src.store_nr == _dst.store_nr` | **yes** |
| `Some(witness)` (a `??`/JOIN return) | `_src.store_nr == u16::MAX \|\| _src.store_nr != var_<witness>.store_nr` | **no** |

The @P290 comment sitting directly above both already stated the requirement — *"When `_src`
lives in the destination's OWN store … clearing that store would wipe the very data we copy, so
pass the reference through unchanged instead"* — and the default arm carries it. The witnessed
form, added later for the join's owned/borrow split, rewrote the whole condition and dropped it.
So when the witness IS the destination (`c = cond(c, i)`), the borrow arm fell to the COPY leg,
`OpDatabase(cell, _dst, …)` cleared the store in place, and `OpCopyRecord` then read the record
it had just wiped.

**The cure is one disjunct, and it is spelled ONCE now.** Both arms read a shared `PASSTHROUGH`
constant, because the witnessed arm is a *refinement* of the default and not a second opinion —
writing the condition twice is exactly how it came to omit half of it. The fix is a strict
widening of the ADOPT arm: the new disjunct fires only where `_dst` and `_src` are one store, and
there the adopt arm's own displaced-free (`_dst.store_nr != _src.store_nr`) is already false, so
nothing is freed and the assignment becomes the no-op it always was.

Verified on `probes/step05-corpus.loft` (one function per path through the guard), both backends:

```
before   interpret: A=2 B=10 C=5 D=5 E=3 F=77
         native   : A=0 …                        <- the defect
after    interpret: A=2 B=10 C=5 D=5 E=3 F=77
         native   : A=2 B=10 C=5 D=5 E=3 F=77
```

The emitted diff is **five sites, each gaining only the disjunct**
(`probes/step05-adopt-condition.txt` carries the before/after of every adopt condition; the full
212K `introspect` dumps are regenerable and not committed); every `None`-arm site is
byte-identical, which is what says the change is confined to the witnessed form.
Guard: `tests/scripts/1017b-a-conditional-borrow-into-its-own-binding.loft`, seven cells, which
goes 7-failed → 7-passed on `loft --tests --native` across the fix. Cell **B** is the control that matters: a *different* local that
was bound from the witness (`d: M = c; d = cond(c,3)`) still answers 10, so the widened adopt did
not turn a materialise into an alias.

⚠ **`D-own-16`'s leak numbers are UNCHANGED** by this — `MintOnly×9, JoinSelf×9, MaybeBorrow×4,
ParamBorrow×3, Captured×3` before and after. That is the evidence the two are distinct
mechanisms rather than one defect seen twice, and it is why this is step 0.5 and not step 4.

#### And the native leak numbers step 0 could not reach

With the assertion no longer aborting the run, `--native` reports **four** leaking types, not
five:

```
kt=81 MintOnly×9, kt=82 JoinSelf×9, kt=84 ParamBorrow×3, kt=86 Captured×3
```

**`MaybeBorrow` does not leak on `--native` at all** — cell 3 is an INTERPRETER-only leak,
because native's materialise leg allocates the destination a fresh store and the displaced one
is released by the `_old_c` epilogue. So step 4's cell-3 criterion is `4 → 0` on the interpreter
and *already 0* on native, and a fix measured only on native would read as complete while the
interpreter still leaked.

---

### Step 1 — cite, change nothing

Add the `@FR-O-Proxy` citation to `owned_ref` and a comment naming `owns_freeable_store` as the
same question. Zero behaviour change.

**Exit:** `rule_tags.py check` ok; `@FR-O-Proxy` reports **15** sites. This makes the count above
honest rather than a lower bound, and it is the step that survives even if the rest is abandoned.

### Step 2 — fold the predicate onto its one home, no peel

Replace `owned_ref`'s `depend().is_empty() && !is_skip_free(v) && !is_hidden_buf_arg` with a call
to `owns_freeable_store`, keeping the type-shape `matches!` in front of it unchanged.

⚠ `!is_hidden_buf_arg` and `(!is_argument(v) || is_promoted_ret_buffer(..))` are **not obviously
the same predicate**. If the emit moves, they were not — and that difference is a finding to
record, not a diff to wave through.

**Exit:** emitted IR *and* generated native are **byte-identical** before/after across
`tests/scripts` + `tests/docs` (`loft introspect` diff, per
[CODEGEN_METHOD.md](../../CODEGEN_METHOD.md)). Any divergence stops the step.

### Step 3 — extend the witness, still gated shut

Widen `rbuf_witness`'s qualifying-local set to the nullable heap-record class. Do **not** touch
the type-shape gate, so no free changes placement yet: the flag is computed and unread.

**Exit:** emit byte-identical again (the flag must be dead code at this point); `make ci` green.
A step-3 that moves the emit means the witness is already being consulted somewhere it should
not be.

### Step 4 — open the peel behind the witness, one shape per commit

**4a — the `rhs_reads_v` stash path.** Peel `Optional` in the shape test **and** require the
witness before the post-free is emitted. This closes the `mint` and `maybe` rows.

**4b — the `??` join.** Same licence, applied where the RHS is an `ncc` whose borrow arm is
`Var(v)`.

**Exit (each):** its own step-0 cells go 9 → 0 / 4 → 0 on both backends; every control cell
unchanged; `LOFT_STRICT_STORES=1` and `LOFT_POISON=1` clean on both; the capture control still
answers `1`. Then flip the step-0 file into the runner and record `@falsified-at:` against the
pre-step commit.

### Step 5 — sink the Set into the join's arms (optional, and separable)

Rewrite `Set(v, ncc(arms))` where an arm is `Var(v)` into a form that frees inside the minting
arm only. The join shape then needs **no witness**.

**Exit:** the emitted IR for the join cell no longer reads the witness flag; leak stays 0. This
is an optimisation of 4b — if it fights, drop it. It is listed because it is the one place
"rewrite the IR" is genuinely the right answer, and because it removes a shape from the
witness's load rather than adding to it.

### Step 6 — record

Close the two shapes in [`formal/ownership-history.md`](../../formal/ownership-history.md); correct
the *"nothing static separates"* sentence to name the conditional-borrow row only; carry the
`maybe` row forward as the residual and as the cluster's remaining shared mechanism. Update
`formal/ownership.md`'s `OPEN:` line and the two index tables ([README](../../formal/README.md)
§ Areas, [ROADMAP](../../formal/ROADMAP.md) § Distance today) — the recipe in § Areas re-measures
them.

## Instruments — run all of these on every step

- `loft introspect <probe>` on **both** backends, before touching the compiler and after. Steps
  1–3 are byte-identical-or-stop.
- `LOFT_NATIVE_LEAK_CHECK=1` for the native leg — **a bare `--native` run prints no leak report
  at all** and reads clean on a build carrying every one of these leaks.
- `LOFT_STRICT_STORES=1` (never-freed + violation count) and `LOFT_POISON=1` (turns a
  use-after-free from a right answer into a wrong one).
- `LOFT_VAR_TABLE=<fn>` to read the ownership flags when a local's licence is in question.
- `make check-rlib` **before** treating any native result as evidence — `cargo build --bin loft`
  does not rebuild the rlib the native tests link.
- `./scripts/find_problems.sh --subject codegen` while iterating; ONE `make ci` before each
  commit.

## Predicted size, to validate against

Steps 1–3 are a citation, a predicate fold and a set-membership widening: **small, and each
proves itself by not moving the emit.** Step 4 is the only behavioural change and should be
*two* conditions at *one* site, because step 2 has already collapsed the predicate.

**If step 4 needs a third site, the fold in step 2 did not actually collapse anything** — stop
and re-read rather than threading it. That is the divergence this section exists to catch.

## See also

- [`formal/ownership.md`](../../formal/ownership.md) `D-own-16`, `D-own-26`, and
  [`ownership-history.md`](../../formal/ownership-history.md) for the four cures already **measured
  and ruled out** — a static free (double-frees against the work-ref), an ungated peel, naming
  the var-cycle base, and admitting `Value::Call` to `callref_join_first_bind`. Do not re-run
  them.
- [QUALITY.md](../../QUALITY.md) — the per-execution ownership witness cluster.
- [OWNERSHIP_MODEL.md](../../OWNERSHIP_MODEL.md), [CODEGEN_METHOD.md](../../CODEGEN_METHOD.md).
