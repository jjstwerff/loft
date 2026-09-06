<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN154 phase 0 — the bypass census

**Question.** The plan proposed keying the shadow's tags on the two typed accessors,
`State::put_stack` and `State::get_stack`.  Are they where the bytes actually arrive?

**Verdict — no, and the fix makes phases 1–3 simpler.**  `put_stack` is **1 of 33** sites
that write the stack store; it carries **74.5 %** of the bytes that change there, and of
1106 corpus programs **not one** is covered by it alone.  The tag cannot live at the
accessor.  It can live one level down, at `Store::addr_mut::<T>`,
which **32 of those 33 sites** already call and which — being generic — sees the width and
kind of every write, where `put_stack` sees them for one.  The read side is the opposite:
reads *are* concentrated in the accessors, so the check stays where the plan put it.

Phase 0 was worth its compile.  The audit that never ran — grep `fill.rs` for `s.put_stack`
(168 calls) against the frame writers (11) — reads as *94 % covered* and is wrong, because
an operator template calls a `State` helper (`string_from_code`, the text appenders) that
does the writing further down.  Only measuring the memory shows that.

---

## The instrument

`LOFT_STACK_CENSUS=1` (`src/stack_census.rs`).  After every operator it diffs the stack
store's live frame against a snapshot taken before it, subtracts the spans `put_stack`
declared, and attributes the remainder to the opcode that ran.  It takes **no list of
sites on trust**: the ground truth is the memory, so a write through a route nobody has
enumerated is counted like any other.  That is the property the question needed — an
inventory of callers is exactly what was in doubt.

Two hooks: one line in `put_stack` that records `(absolute offset, size_of::<T>)`, and a
snapshot/diff pair around the dispatch in `execute_argv`.

---

## The static half — the write surface is closed

Every site that writes the interpreter stack reaches it through
`Stores::store_mut(&stack_cur)`, and there are 33:

| Family | Sites | Where |
|---|---|---|
| the typed TOS writer | 1 | `put_stack` |
| **frame-slot writers** | 15 | `put_var`, `mut_var`, `init_ref`, `init_ref_sentinel`, `init_create_stack`, `set_frame_dbref`, `set_frame_value` (one each) and `set_frame_literal` (8 arms, one per literal type) |
| host calls, workers, FFI | 7 | `execute_host` (2), `execute_at_text` (2), `execute_at_raw_primitive_input_wide`, `push_worker_arg`, `execute_at_void_with_snapshot` |
| text | 4 | `state/text.rs` — `set_string`, `string_mut`, and two appenders |
| coroutine frames | 3 | `coroutine_next` (2), `coroutine_yield` |
| frame reserve / null fill | 2 | `reserve_frame`, `push_null_value` |
| buffer growth | 1 | `ensure_stack` → `grow_words` (a realloc, not a value write) |
| **total** | **33** | |

Which primitive each one uses is the load-bearing count:

- **32 of 33 call `Store::addr_mut::<T>`** — typed, so the tag is free at every one of them.
- **1 calls `grow_words`** (`ensure_stack`): a realloc, which moves the whole shadow.
- Beside those, **one untyped byte move**: the return-value slide at `state/mod.rs:2247`
  (`Stores::copy_block`), which reaches its destination store through `store_mut(to)`.
  This is the route `LOFT_UAF_GEN` had to be taught by hand, and whose omission produced
  its residual false positive.

So the write surface is **two hooks and a realloc**: a typed one that can stamp a tag, an
untyped one that must *move* the tags with the bytes, and a resize that rebases them.

The **frame-slot** family is the interesting one.  It is the biggest after `put_stack`,
and it is precisely the residence `LOFT_UAF_GEN` cannot watch — DEBUG.md records that
detector as seeing "only the window between a push and its pop … which is why it never saw
loft#723".  A tag placed at `addr_mut` covers it on the first day.

## The read side is not symmetric

Reads concentrate where the plan assumed: in `fill.rs`, **246** calls to `get_stack` and
**9** to `get_var` account for the evaluation reads.  So the *check* can stay at the
accessors — and it must, because the low-level reader `Store::addr` is also what the
debugger, `render_frame_local`, `write_stack_hex_dump` and `scan_stack_anomalies` call, and
reading an uninitialised or stale slot is their JOB.  A check at `Store::addr` would report
the diagnostics as defects.

**Tag low, check high.**

## The dynamic half — how much arrives where

`tests/scripts/` on `--interpret`, release build, 200 000 ops per program:

| | |
|---|---|
| programs that ran | **1106** of 1177 (the rest are refusal tests and scripts wanting `@ARGS`) |
| operators | 11 547 794 |
| bytes changed via `put_stack` | 14 728 262 — **74.5 %** |
| bytes changed by another route | 5 052 501 — **25.5 %** |
| distinct opcodes writing by another route | **51** |
| **programs with NO bytes outside `put_stack`** | **0** |

The last row is the verdict, and it is not a matter of degree.  Not one program in the
corpus is covered by the accessor alone, so a shadow tagged only at `put_stack` would
have a hole in **every** program it ran on — and the hole is not exotic memory, it is the
frame slot that `OpPutInt` writes.  A phase-1 `uninit` check keyed at the accessor would
therefore report the language's commonest assignment as a read of an unwritten slot: a
false positive on almost every program, on day one.

The routes, by bytes:

| opcode | bytes | ops | the writer underneath |
|---|---|---|---|
| `OpConstText` | 751 138 | 112 369 | `string_from_code` → `set_string` (`state/text.rs`) |
| `OpPutRef` | 747 989 | 278 155 | `put_var::<DbRef>` — a frame slot |
| `OpPutInt` | 745 059 | 590 902 | `put_var::<i64>` — a frame slot |
| `OpReturn` | 536 213 | 160 266 | the return slide, `Stores::copy_block` |
| `OpDatabase` | 493 994 | 115 329 | mints a store, writes the ref to a frame slot |
| `OpInitRefSentinel` | 480 548 | 109 470 | `init_ref_sentinel` |
| `OpInitRef` | 310 153 | 78 274 | `init_ref` |
| `OpFreeText` / `OpAppendText` / `OpInitText` | 491 768 | 112 949 | the `state/text.rs` writers |
| `OpInitCreateStack` | 162 389 | 48 094 | `init_create_stack` |
| … 42 more | | | |

Every one of these resolves to a site in the static table above, which is the cross-check
that matters: the diff found no route the static reading missed, and the static reading
named none the diff never exercised.  Two instruments, one answer.

## What this changes in phases 1–3

1. **The tag is set in `Store::addr_mut::<T>`, for the stack store only**, not in
   `put_stack`.  One home, and it carries `T` — which is also what phase 2 needs, so
   phases 1 and 2 share a hook instead of phase 2 re-visiting every writer.
2. **`Stores::copy_block` moves tags with bytes**, generalising the hand-written
   `uaf_move_shadow` at `state/mod.rs:2247` rather than adding a second special case.
3. **`ensure_stack` rebases the shadow** when the buffer grows.
4. **The check stays at `get_stack` / `get_var`**, so no diagnostic reader trips it.
5. Identifying the stack store needs a store identity inside `Store` (it is store 0, and
   `Stores::store_mut` already knows the number) — a small piece of plumbing that phase 1
   now knows it owes.

## What the instrument cannot see

Stated because the numbers above are a **floor**, not a measurement:

- A byte written and then restored **within one operator**, or written with the value it
  already held, does not change and so is invisible to a diff.  Under-reporting the
  bypasses is the unsafe direction for this question, which is why the static half is here
  beside it: the two agree.
- Only the main dispatch loop in `execute_argv` is censused.  Ops running under
  `run_to_return` — `par` workers, `parallel` arms, host calls, a coroutine resume — are
  not, so an opcode's total is what it wrote on the main loop.
- The byte totals vary by a few bytes run to run.  A `Str` on the frame embeds a pointer,
  so what a text write CHANGES depends on where the allocator put the buffer; the control
  script has read 2283–2287 bytes across runs, moving the shares by under 0.1 pp.  Read
  the percentages, not the byte counts.
- The watched span is the frame plus a 4 KB margin, not the whole buffer.  A `put_stack`
  span landing outside it is counted and reported (`WARNING: … beyond the watched frame
  span`); a random 80-program sample raised none.  The unbounded version cost 42× on `1248b`, which
  put the corpus out of reach; bounded it costs ~20× and the control script's shares moved
  by 0.06 pp.

## Reproducing

```bash
LOFT_STACK_CENSUS=1 loft --interpret <program>          # one program
bash doc/claude/plans/154-stack-shadow/census-run.sh <outdir>   # the corpus sweep (serial:
                                                        # several scripts write real files)
```

Cost is ~20× on an op-heavy script, so it is a probe, never a sweep in CI — the same
standing as `LOFT_STRICT_STORES`.
