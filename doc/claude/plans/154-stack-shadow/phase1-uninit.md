<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN154 phase 1 — `uninit`, and what it turned out to witness

**Question.** Can a shadow over the whole frame report, at the read, a stack slot that no
path wrote — silent across the corpus, and reporting on the build a guard was written to
catch?

**Verdict — yes, and the falsification target had to be found rather than assumed.**  The
detector is silent on all 1106 runnable corpus programs and reports four distinct sites on
the build that motivated the nullable-local pre-init fix.  But **neither** of the two
defects the issue named as `uninit`'s evidence is in this state, and both were measured on
their own control builds rather than argued from the issue text.

---

## The mechanism

`LOFT_VERIFY_STACK=1`.  One tag byte per stack byte, carried by the stack `Store` itself,
so it grows with `grow_words`, truncates with `retile_tail`, and cannot drift from the
buffer it describes.

| | Where | Why there |
|---|---|---|
| **tag** | `Store::addr_mut::<T>` | phase 0: 32 of the 33 sites that write the stack already call it, and it carries `T` |
| **move** | `Stores::copy_block`, `Store::copy_block{,_between}` | the tags travel with the bytes, which is what makes a callee's missing return value report at the CALLER's read |
| **kill** | `get_stack` (the pop), `reserve_frame`, and one chokepoint at the dispatch loop | a slot that leaves the live frame inherits nothing to the next occupant |
| **check** | `get_stack` / `get_var` | phase 0: `Store::addr` is what the debugger and the frame renderer read stale slots with, on purpose |

`LOFT_VERIFY_STACK_INJECT=1` suppresses the tag at the write hook, so every checked read
reports.  It is the reason a silent run means something: a detector that cannot fire and a
clean corpus look identical.

## Two things the corpus changed

**The untyped byte move is seven sites, not one.**  Phase 0 named the return slide
(`Stores::copy_block`) and said "beside them sits one untyped byte move".  Running the
corpus found six more, each an `addr_mut::<u8>` followed by a raw multi-byte write — the
coroutine frame restore, its `Created` zone fill, the yielded-value slide,
`push_null_value`'s wide arm, and two worker-stack overlays.  The bound and the tag covered
ONE byte of a span of hundreds, which is why `1032-generic-iterator-return.loft` and
`1054-parallel-block-arms-run.loft` reported.  They now go through `Store::addr_span_mut`,
which bounds and tags the whole span, so the family is one primitive instead of seven
conventions — and the bounds check got stronger on the way.

**The tally has to be process-wide.**  It was thread-local first, and
`1054-parallel-block-arms-run.loft` printed four findings from a `par` arm and then closed
with the main thread's `no uninitialised stack reads` — the one line a sweep reads.

## `uninit` is the ABSENCE of a tag, not a partial one

An eval slot is stepped to eight bytes (`aligned_stack_step` is `next_multiple_of(8)`)
while a `boolean` writes one and a `character` four, so **every sub-word scalar leaves
padding nobody wrote**.  `OpSizeScalar` pops eight bytes to discard a `character`; its own
doc says so — *"every scalar occupies one 8-byte eval-stack slot, so the op reads that slot
to consume any scalar"*.  Checking every byte of the read reported three corpus programs on
day one, all of them the language's own stepped slots.

So the check answers three states, and phase 1 reports only the first:

* **`Unwritten`** — not one byte of the span was written.  The slot has no occupant.
* **`Partial`** — the span starts on written bytes and runs off the end of them: the
  occupant is NARROWER than the read.  Counted in the summary, not reported.
* **`Written`**.

That is the plan's own cut — *"`uninit` is the absence of a tag, `width + kind` is the
tag"* — and the `Partial` counter is phase 2's queue, already measurable.

## The falsification, and the two evidence corrections

Each control was built by cherry-picking the detector onto the guard's recorded
`@falsified-at:` ref, because `make falsify` builds the control tree as it was and the
detector does not exist there.

### In class — the pre-init defect (control `64437246`)

`a-nullable-local-first-assigned-inside-a-branch-or-loop-holds-null-on-the-other-path.loft`.
`scopes::needs_pre_init` listed the bare heap spellings without peeling `Optional`, so an
`S?` / `vector<T>?` / `text?` local first assigned inside a branch got no `Set(x, null)`,
and the other arm's guarded displacement free read a slot the frame never wrote.

```
stack verify: get_var<DbRef> reads 12 uninitialised byte(s) at frame offset 116 … line 57
stack verify: get_var<DbRef> reads 12 uninitialised byte(s) at frame offset 120 … line 58
stack verify: get_var<DbRef> reads 12 uninitialised byte(s) at frame offset 116 … line 60
stack verify: get_var<DbRef> reads 12 uninitialised byte(s) at frame offset 132 … line 85
```

At HEAD the same program is silent.  This is the phase's gate, and it is a better witness
than either issue-named one: the guard's own header calls the read *"an uninitialised
slot — a refused free of `0xDEADBEEF`"*, which is `LOFT_POISON`'s class stated in
`LOFT_POISON`'s words.

### Out of class — loft#1386 (control `964bab93`)

The issue reads *"a value-position `match` arm that yields nothing, so the consumer read
the slot below and answered null"*.  The bytecode on that build says otherwise:

```
:POS56
  56[40]: ConstText(_value="one") -> text
  61[56]: Call(d_nr=368, args_size=16, fn=n_println)
  80[40]: ConstInt(val=-9223372036854775808) -> integer      <- the arm pushes a null
:POS89
  89[48]: PutInt(var[32], value: integer)
```

The void arm materialises the integer null sentinel as a CONSTANT, and the mirror cell
(`v2`, void arm first) discards the good arm's value with a `FreeStack` and pushes the same
constant at the join.  The slot is fully written; the shadow is silent there, and correctly
so.  loft#1386 is a TYPING defect — a void arm given a value it never produced — and no
memory-state detector can see it.  The issue's evidence line for `uninit` is wrong.

### Out of class, but visible — loft#1254 (control `6b12385dd`)

The empty-body stub is the shape the issue should have cited, and it lands one state over.
`n_empty_float` compiles to a bare `Return(ret=0, value=8, discard=8)`, so `copy_result`
slides eight bytes out of the four-byte return-address slot `fn_call` pushed:

| | `Unwritten` | `Partial` |
|---|---|---|
| control `6b12385dd` | 0 | **1** |
| HEAD | 0 | 0 |

The caller reads an `f64` where a `u32` was written at the same base — structurally
identical to `OpSizeScalar` popping eight bytes off a `boolean`, and separable from it only
by KIND.  That is phase 2's tag, and phase 2's legal-pun list is exactly the question of
which of the two is admitted.

## What this says about the plan's cut

Two of the three defects the issue names as `uninit` evidence are not `uninit`, and the one
witness that is had to be found by reading guard headers for the `LOFT_POISON` vocabulary.
The state is real and the detector is calibrated both ways — but its yield is narrow,
because a stack slot is nearly always recycled with *something* in it.  **The width tag is
what the named evidence needs**, so phase 2 is where the class this plan was opened for
becomes visible, and phase 1 is the mechanism it rides on rather than a detector that pays
for itself alone.

## Reproducing

```bash
LOFT_VERIFY_STACK=1 loft --interpret <program>              # one program
LOFT_VERIFY_STACK=1 LOFT_VERIFY_STACK_INJECT=1 loft …       # the positive control
bash doc/claude/plans/154-stack-shadow/verify-run.sh <outdir> [binary] [stdlib-path]
```

Corpus at HEAD: **1106 clean, 0 reports**, 71 programs with no verdict (refusal tests that
exit before an operator runs, and one that aborts).
