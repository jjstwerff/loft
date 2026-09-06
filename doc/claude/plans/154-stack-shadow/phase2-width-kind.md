<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN154 phase 2 — the tag, and which disagreement is a finding

**Question.** The shadow knows a slot was written.  Can it also know the slot is being read
at the width and kind it was written at — silent across the corpus, and reporting on the
builds the monomorph-layout guards were written to catch?

**Verdict — yes, on the HANDLE axis, and the width axis had to be given up for a measured
reason.**  A strict *the read's width must match the write's* reported **43 of the first 180
corpus programs** on day one, every class of them the interpreter's own frame layout.  What
survives is *a handle crossing a value*, which needs no pun list and catches two of the
phase's three named guards.  The third is out of a stack shadow's reach entirely, and that
is a fact about the plan's scope rather than about the detector.

---

## The tag

The hook already carried the type: `Store::addr_mut` is generic over `T`.  Each byte's tag
becomes `family:8 | width:8 | index-in-value:8`, where the family separates a **handle**
(`DbRef`), a **text** (`Str`), a **float**, a **string** and a plain **word**, and the index
says whether the read starts at the value's base or inside it.

Two admissions are structural rather than listed:

* **A raw byte BLOCK on either side is opaque.**  A type whose name starts with `[` —
  `[MaybeUninit<u8>; 20]`, the fn-ref slot's spelling since @PLAN53 cluster 4 — is a block
  someone assembled, and so are the raw spans `Store::addr_span_mut` writes.  Whoever
  assembled it knows what is in it; the tag does not.
* **A heap-to-stack copy arrives opaque.**  Heap bytes carry no tag, and they are real data.

## Why the width comparison is not the finding

| class | the write | the read |
|---|---|---|
| the **fn-ref slot** | 20 bytes, whole, as `[MaybeUninit<u8>; 20]` | `i64` at the base (`d_nr`) and `DbRef` at +8 (the closure) |
| the **iterator state** | wider writes by `iterate` | `OpStep` reads two `u32`s four bytes apart |
| the **stepped slot** | a `boolean` writes one byte | `OpSizeScalar` reads eight to discard it — *"every scalar occupies one 8-byte eval-stack slot"*, says its own declaration |
| the **null sentinels** | the word the sentinel is | the type it stands for |

The frame carries **composite slots the compiler addresses field by field**, so a Rust
accessor type is a poor proxy for the loft type of a position.  A width rule needs a pun list
that is essentially *"every composite slot in the interpreter"*, and a list that long is a
list that goes stale.  The disagreements are still COUNTED, so the pun population stays
visible and a future phase can revisit it with the compiler's own slot types instead of the
accessor's.

## Why the handle axis needs no list

A `DbRef` is a **position in a store**.  Reading one as a number answers an address wearing a
number's clothes; reading a number as one indexes a store by data.  Neither can be an
intended pun, because a slot the compiler typed as a reference is a reference on every path —
and that is exactly what the monomorph-layout class breaks: the template chose an op against
`__typevar_T`, substitution retyped the slot, and the op stayed.

## The falsification

Controls built with `shadow-control.sh <ref>`, which cherry-picks the shadow onto the control
tree before building it — `make falsify` builds the control as it was, and the detector does
not exist there.

### loft#1028 — control `05ab4611` (parent of `60cdbd51`)

`pub fn nl1028<T>(x: T, c: boolean) -> T? { if c { return x; } null }`

| | `nl1028(7, false) ?? -42` | the shadow |
|---|---|---|
| control | **65535** | `get_stack<i64> reads 8 byte(s) — the value written here is handle 12 byte(s) wide` |
| HEAD | `-42` | silent |

`65535` is the guard's own documented value: the twelve-byte `OpNullRefSentinel` read back as
an integer, low sixteen bits.  The report names the mechanism, not just the site.

### loft#1016 — control `c398dd71` (the main tip before PR #1025)

`pub fn g1016<T>(v: vector<T>, a: T? = null) -> T { _ = len(v); a? }`

| | record cell | integer cell | the shadow |
|---|---|---|---|
| control | **4294967198** | **65535** | 4 distinct sites: `get_var<i64> reads 8 — handle 12` and `get_var<Str> reads 16 — handle 12` |
| HEAD | `3` | `0` | silent |

⚠ The first control tried was `655df646~1`, and it answered CORRECTLY — the commit is a
rebased copy whose parent already carries the fix.  A control ref taken from `git log --grep`
is a guess until the unarmed run reproduces the defect; check that first, or a silent
detector reads as a miss when the tree is simply not the broken one.

### loft#1070 — control `5156c175` (the main tip before PR #1068), NOT in class

`arms1070(S1070 { a: -7 }, true).a` answers **4294967198** on the control and `-7` at HEAD,
and the shadow is **silent on both**.  Correctly: the arm's record is ALLOCATED with the type
variable's row, so what is wrong is a heap record's layout.  The stack slot holds a correct
`DbRef` either way, and reading a field of the record it names never comes through
`get_stack` / `get_var`.

That is the boundary of a stack shadow, and it is worth stating in the plan's own terms: the
`width + kind` class has a STORE-side half this plan does not reach.  The leak warning
(`kt=9 __typevar_T×2`) is the only signal on that build, and it says nothing about a wrong
value — which is how the issue described the problem in the first place.

## The cost when it is OFF

An always-available detector is paid for on every run, so it was measured on a field-write
loop (4 M iterations, three struct-field writes each) by instruction count:

| | instructions | vs. the tree before the shadow |
|---|---|---|
| before (`48d1229f`) | 30.75 G | — |
| the shadow, first cut | 33.96 G | **+10.4 %** |
| hooks outlined `#[cold]` | 31.57 G | +2.7 % |
| the armed flag as a `State` field | **31.03 G** | **+0.9 %** |

Two findings, and the second is the one that would not have been guessed:

* **The `addr_mut` hook is not the cost.**  Deleting it entirely changed nothing measurable
  (31.70 G against 31.57 G with it).  What cost was INLINING the shadow's body at every
  instantiation of a generic accessor, which pushes the hot path's code apart; `#[cold]` +
  `#[inline(never)]` on `shadow_write` / `shadow_kill` recovered three quarters of the
  regression.
* **The read side cost 2.4 % in the ARMED TEST alone.**  `get_stack` and `get_var` run for
  every operand of every operator, and reading a `OnceLock<bool>` there is not free.  The
  flag is now a `State` field set at construction, which is a load from a struct already in
  registers.

## Corpus

**1106 of 1177 programs clean at HEAD on `--interpret`, zero reports.**  The remaining 71
exit before an operator runs (refusal tests) or abort.

## Reproducing

```bash
LOFT_VERIFY_STACK=1 loft --interpret <program>
bash doc/claude/plans/154-stack-shadow/shadow-control.sh <control-ref>   # build the control
bash doc/claude/plans/154-stack-shadow/verify-run.sh <outdir> [binary] [stdlib-path]
```
