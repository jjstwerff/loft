<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN154 phase 5 — arming it

**Question.** The shadow is silent over the corpus and speaks on every control it was
measured against.  Is it worth CI minutes, and what exactly goes red?

**Verdict — yes, at ~2× the in-process interpreter corpus, in the nightly beside
`LOFT_POISON`.**  It needs no sanitizer and no nightly toolchain, and it covers the
residence poison cannot describe.

---

## What goes red, and where the check lives

The corpus runs **in-process** (`tests/wrap.rs` builds a `Parser` and a `State` inside the
test binary), so `main.rs`'s non-zero exit never fires for it.  The gate is therefore a
per-script check in the harness — `stack_verify_gate(shadow_before)`, a difference against
the process-wide tally so a `par` arm's finding counts and lands on the right script.

`main.rs` gained the exit code too, for the same reason `LOFT_STRICT_STORES` has one: a probe
that has to be read is not a gate, and a sweep that greps stderr goes green when the format
changes.

**Calibrated both ways, which is the whole point:**

* green over `--lib --test issues --test wrap --test strings --test frame_vars` today;
* **every** script fails under `LOFT_VERIFY_STACK_INJECT=1`, which suppresses the write tag so
  each checked read reports — measured on `11-vectors.loft`, which fails with the shadow's
  own report lines above the harness's message.

## Cost

| | instructions on a 4 M-iteration field-write loop |
|---|---|
| unarmed | 31.2 G |
| armed | 68.7 G — **2.2×** |

`loft_suite` under the shadow: **67-93 s** across runs against ~50 s unarmed — the spread is
box contention, not the shadow, and the instruction count above is the figure to quote.  That
is the same standing as the poison job, and an order of magnitude cheaper than the phase-0
census's 20×.

## Where it is registered, and why in three places

`.github/workflows/miri.yml`, as `stack-shadow`, beside `poison` — and in **`notify`'s**
`needs` and **`daily-status`'s**.  The workflow states the rule itself: *"a gate absent from
`needs` is invisible here, and its silence reads as green and AUTO-CLOSES the issue.  When you
add a gate that means 'the language is broken', add it here in the same commit."*  An
uninitialised, mistyped or stale frame-slot read is exactly that class — every one of them
answers a plausible value with nothing said.

The release gate picks it up for free: `release-gate.yml` calls this workflow whole.

## What it does NOT cover

* **`--native`.**  The shadow lives on the interpreter's value-stack store, and a compiled
  binary has none.  A native run says so rather than closing with a reassuring "no … reads"
  (the same treatment loft#865 gave the profiler).
* **The store side.**  loft#1070's wrong layout is in a heap RECORD; the frame slot holds a
  correct handle either way, and no stack shadow can see it
  ([phase2-width-kind.md](phase2-width-kind.md)).
* **`library_suite`**, and the native / wasm / html suites, which spawn separate binaries that
  never reach this path — the same lean surface the poison job takes, for the same reasons.
