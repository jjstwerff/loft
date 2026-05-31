<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# TESTING.md — what must pass to switch the default to the aligned stack layout (V2)

## Premise

**"Switch"** = make the aligned eval-stack stepping + the V2 slot allocator
(`LOFT_ALIGN=1 LOFT_SLOT_V2=drive` today) the **production default**, retiring
the V1 byte-packed *unaligned* eval stack — the path that still carries the
latent cluster-2 UB and is what ships today.

**The validation always runs on GitHub, after a green full PR run.**  Local
runs (`make ci`, `run_guard.sh`, a single `miri` test) are for iteration only;
the switch decision is made exclusively on a **full GitHub run across all three
OSes**.  The V2 gates below run *in addition to* the standard PR run, on GH —
never as a per-PR blocker (a 3-OS full suite under V2 + Miri is far too slow for
that; see § Invocation).

## Why this is not a one-line flip (the risk surface)

The new layout changes, relative to V1:

- **eval-stack stepping** — every push advances `stack_step(size)` =
  `next_multiple_of(8)` instead of the raw `size` (`src/variables/mod.rs`,
  `src/state/mod.rs`, `src/state/codegen.rs`).
- **V2 slot allocator** — `assign_slots_v2` places locals at `align`-multiples
  (`src/variables/slots_v2.rs`); different frame layout from V1.
- **frame model** — stepped `local_start`, args region, return slot, frame base
  (`execute_argv`), coroutine frames, par-worker frames.
- **text offsets** (`src/state/text.rs`) and **native marshalling**
  (`Stores::get`/`put`, `src/database/`).
- **more stack memory** — padding to 8 inflates per-frame size; deeper frames
  and recursion use more of the stack store.

Because the change is behaviour-*preserving* by design (same program output,
different byte layout), the dominant test is **differential**: V2 output must
equal V1 output everywhere.  The dangerous part is the platforms/backends where
the byte layout actually matters.

## The validation matrix (all on GitHub)

A "V2-switch candidate" PR/branch must show ALL of the following on one full GH
run.  Platforms: **ubuntu-latest (x86-64), macos-latest (ARM64 / Apple
Silicon), windows-latest (x86-64)**.

### A. Standard full PR run stays green — V1, 3 OS  (no production regression)

`ci.yml`'s existing 3-OS matrix: all ~60 test binaries, both backends,
including the Windows `--native` backend (main #233 / @P229).  This proves the
candidate doesn't break what ships today.  Pre-condition for everything below.

### B. The full suite under V2 — 3 OS, differential V1 ≡ V2  (the core)

The same 3-OS matrix re-run with `LOFT_ALIGN=1 LOFT_SLOT_V2=drive`.  Every
interpreter-exercising binary must produce the **same result as its V1 run** —
zero output/exit divergence.  Watch especially the binaries whose path V2
changes:

- core IR / eval-stack: `issues`, `expressions`, `expressions_auto_convert`,
  `wrap`, `strings`, `format`, `template_matrix`, `data_structures`,
  `frame_vars`, `spans_on_ir`
- slots / layout: `slots`, `slot_v2_baseline` (the dedicated V2 allocator
  gate), `codegen_emitter`
- closures / fn-refs / tuples / coroutines (the cluster-2 hot shapes):
  `closure_matrix`, `mut_closure_matrix`, `tuple_matrix`, `coroutine_matrix`
- collections (sorted/hash/par): `parallel_rebase`, `multiplayer_v2/v3/v5`,
  `threading`, `threading_chars`
- text/leak/store: `leak`, `leak_cases`, `leak_cross_mode`,
  `store_durable_format`, `store_durable_loft`, `store_durable_tier1`,
  `store_persist_loft`

**macOS-ARM is THE platform.**  x86-64 (ubuntu/windows) tolerates unaligned
access in hardware, so there V2-vs-V1 is mostly a correctness check.  ARM64
(macOS) is strict-alignment — it is where V1's unaligned `&Str`/`&DbRef`/`i64`
reads are genuine faults/UB and where V2's whole purpose pays off.  **@P383 was
a macOS incident.**  A green macOS-ARM V2 run where V1 is at-risk is the single
most convincing result.

### C. Sanitizers under V2 — on GH

- **`stack_align_guard`**: build with `--features stack_align_guard`, run the
  FULL suite under V2 on all 3 OS → **zero guard fires**.  (Status today:
  `issues` 685/0 zero fires, ubuntu, local.  Must extend to every binary × 3
  OS on GH.)  This is the cheap homegrown detector; it must be silent corpus-
  wide before the flip.
- **Miri (hard-UB, `-Zmiri-disable-stacked-borrows -Zmiri-disable-isolation`)**:
  a curated subset covering each UB shape — struct, closure, fn-ref, text,
  coroutine (`while`/`yield`/`yield from`), sorted/hash iteration, tuple,
  tuple-in-par — must be clean (test ok, no UB, no leak).  At minimum on
  ubuntu; ideally also macOS-ARM.  (Status today: `p213` clean.)

### D. Backends under V2

- **interpret** — primary; V2 applies directly.
- **native** — native codegen uses real Rust values, not the byte-packed stack,
  but the V2 marshalling (R2) touches `Stores::get`/`put`.  `native`,
  `native_ext`, `native_loader`, and the `cross_mode!` native side must be
  green under V2.
- **wasm** — `wasm_entry`, `html_wasm`: confirm the browser interpreter under
  V2 (decide explicitly whether aligned stepping applies to the wasm build, and
  rebuild `make wasm` / `DEFAULT_FILES` if so).

### E. Performance + memory

- **`make bench` V1 vs V2** — V2 pads slots to 8, inflating stack usage and
  touching more cache lines.  Record the delta; agree an acceptance threshold
  (the S0 "round-8 everywhere" spike was reverted partly over this — V2 keeps
  real sizes + gap-fill, so the regression should be small, but it must be
  *measured*, not assumed).
- **stack-depth / memory** — V2 frames are larger.  A deep-recursion + large-
  frame stress test must not overflow the stack store sooner than V1 in a way
  that breaks a previously-passing program.  Characterise max stack bytes V1
  vs V2 on the corpus.

### F. Fuzzing — the differential V1 ≡ V2 stress (high value here)

The hand-written corpus (§ A/B) is a floor, not a ceiling.  Because the switch
rests on a *behaviour-preserving* claim, **differential fuzzing is the highest-
leverage tool we have** — it stresses V1 ≡ V2 over millions of programs the
corpus never enumerates, and the known-open corner cases (2f/2g, the tuple-par
edges) are precisely the kind a structure-aware generator surfaces.  This
realises the plan README's case-finding lanes 2/4/5.

Three tiers, cheapest first:

1. **Mutation-based differential (cheap, start here).**  Take the existing
   `tests/scripts/*.loft` / `tests/docs/*.loft` corpus, apply small structural
   mutations (rename/reorder locals, wrap in extra scopes/loops, widen/narrow
   types, add args), and run each mutant under **V1 and V2** (`--interpret`),
   diffing stdout+exit.  Any divergence is a V2 bug (V2 must match V1).  Reuses
   the `cross_mode!` running shape; no new grammar needed.

2. **Structure-aware generator (lane 2/4).**  A `cargo-fuzz` `fuzz_target!` with
   an `arbitrary`-driven valid-loft AST generator over `parse → byte_code →
   execute`.  Two oracles, run together:
   - **differential**: V1 output vs V2 output must be identical.
   - **guard/sanitizer**: run V2 under `--features stack_align_guard` (fast) —
     ANY guard fire is a defect *even when the two outputs agree* (the masked
     family agrees on x86-64; the guard is what makes it loud).  Optionally also
     build the fuzz target with ASan for heap-UAF/overflow coverage.
   Combine with the homegrown **`LOFT_POISON`** arena poison-on-free (lane 3) so
   a dangling-`DbRef` read returns loud garbage instead of stale-but-correct
   bytes — the store-internal family Miri/ASan can't see.

3. **Targeted slot/stack fuzzing (lane 5) — the V2-specific one.**  Generate
   programs that maximise *layout* stress: many overlapping variable lifetimes,
   names reused across sibling scopes (cf. @P344), deeply nested blocks, deep
   recursion + large frames, and every mixed-width combination
   (`bool`/`char`/`single`/`int`/`float`/`text`/`DbRef`/fn-ref/tuple, in every
   order) — to drive `assign_slots_v2`, `validate_slots`, the two-zone model and
   the stepped frame math into corners.  Run under V2 + the armed guard; an
   unaligned access fires at the site.  This is the most direct attack on the
   exact surface the switch changes.

**Scope notes (the "if appropriate"):**
- Fuzz under the **guard** (normal speed) or ASan, **not Miri** — Miri is far
  too slow for fuzzing throughput.  Miri stays the targeted gold-standard check
  (§ C); the fuzzer is the wide net.
- The structure-aware generator (tier 2) is real work (the grammar); tier 1 is a
  day's work and already very productive for the differential.
- A fuzz finding is a *seed*: minimise it to a probe in `probes/`, fix, add to
  the corpus — same loop as the cluster work.

**As a gate:** a **time-boxed fuzz soak** (tier 1 + tier 3 for N CPU-hours with
zero V1≠V2 divergences and zero guard fires) is a strong pre-switch confidence
signal.  Run it as a dedicated GH job per release-candidate (nightly for a fixed
budget, longer before a flip) — not per-PR.

## Known-open items that BLOCK the switch (close + probe first)

These are residuals from the cluster-2 sweep that are NOT yet exercised/fixed;
a real generator/coroutine/par program in production could hit them under V2:

- **2f — `remove()` keyed-iteration aligned deltas** (`src/state/io.rs`).  No
  test exercises `#remove` during keyed iteration; its `state_var - N` deltas
  were hand-tuned for V1 and do not all survive stepping.  Needs a probe + fix.
- **2g — `serialise_text_args` raw offset** (`src/state/mod.rs` ~826).  A
  generator with a sub-8-byte arg (`character`/`boolean`/`single`) BEFORE a
  `text` arg mis-locates the captured `Str` under V2.  Needs a probe + fix.
- **2e — `n2` sorted content-type registration**: re-verify it is genuinely
  closed by the 2d fix (it stopped failing, but confirm the mechanism, not just
  the symptom).
- **Flag-OFF (V1) bugs found while probing** — char-/boolean-first tuple `par`
  returning 0, and sequential `for p in v { f(p) }` tuple-arg unification
  (`__tuple<...>` vs `(...)`).  These are V1 bugs, separate from the switch, but
  should be fixed so the corpus is clean in BOTH modes (the differential in § B
  needs a clean V1 baseline).

## Exit criteria (binary — the bar to flip the default)

On a single full GH run of the candidate, ALL must hold:

1. Standard PR run (V1) green, 3 OS — no production regression.  (§ A)
2. Full suite under V2 green, 3 OS, **differential V1 ≡ V2** zero divergence.  (§ B)
3. **macOS-ARM green under V2** — the decisive strict-alignment platform.  (§ B)
4. `stack_align_guard` zero fires, full suite, 3 OS.  (§ C)
5. Miri hard-UB clean on the curated UB-shape subset.  (§ C)
6. Leak gates green under V2 + Miri leak check.  (§ C/D)
7. native + wasm backends green under V2.  (§ D)
8. Performance regression within the agreed threshold; stack-memory increase
   characterised and accepted.  (§ E)
9. Open sub-clusters 2e/2f/2g closed, each with a probe.  (§ Known-open)
10. **Strongly recommended (confidence soak, not a one-shot pass):** a
    time-boxed differential + slot/stack fuzz run (§ F) with **zero V1 ≠ V2
    divergences** and **zero guard fires** over an agreed CPU-hour budget.  Not
    binary like 1–9, but a flip without it is flying on the hand-written corpus
    alone.

If 1–9 hold (and 10 is clean for its budget) → flip the default (Phase 2).  If
any of 1–9 fail → that axis is the next work item; do NOT flip.

## Staged rollout (de-risk the flip itself)

- **Phase 1 (now):** V2 behind the flag; `stack_align_guard` + Miri CI gates
  green; this TESTING matrix not yet fully met.
- **Phase 2:** V2 becomes the default; V1 retained as an opt-out (`LOFT_ALIGN=0`
  / `LOFT_SLOT_V2=off`) for a transition window of N releases.  The differential
  (§ B) stays a release-candidate gate so a V2 regression is caught against the
  V1 baseline.
- **Phase 3:** remove the V1 path once V2 has soaked across ≥ N releases with no
  field issue.  At that point the env gating, the `aligned_stack` branches, and
  the V1 slot allocator can be deleted; `stack_align_guard` becomes a permanent
  CI gate against alignment regressions.

## Invocation (how the run happens on GH)

The V2-switch validation is a **dedicated GH workflow** (a V2 variant of the
`ci.yml` 3-OS test job + the guard + a Miri-under-V2 job), run **on demand /
per release-candidate**, AFTER the standard full PR run is green.  It is
**never a per-PR gate** — the 3-OS full suite under V2 plus Miri is far too slow
to block normal review.  Trigger: manual `workflow_dispatch` (once the workflow
is on `main`) or a deliberate trigger-file bump on the candidate branch; not
`pull_request`.

## See also

- [`cluster-2-fix-design.md`](cluster-2-fix-design.md) — the alignment fix
  design + the LANDED records for clusters 2a–2j / 3 / 4 / 5 + the Miri
  validation + the rustc-green baseline.
- [`cluster-2-S4-progress.md`](cluster-2-S4-progress.md) — the S4 (eval-TOS
  alignment) implementation state + the three hard-won process rules.
- [`probes/`](probes/) — the cluster-2 probe suite + `run_guard.sh` (the
  homegrown guard runner) + `run.sh`.
- [`.github/workflows/miri.yml`](../../../../../.github/workflows/miri.yml) —
  the hard-UB Miri gate shipped at D-final.
