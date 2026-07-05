<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN90 — the source-survival split: indicate every UNBOUND structure copy

Tracker: [loft-lang/plans#90](https://github.com/loft-lang/plans/issues/90).
Design: [COPY_DIAGNOSTICS.md § The silent/indicate line: bound vs unbound](../../COPY_DIAGNOSTICS.md).
This is the **safe-implementation recipe** for the classification refinement the user set:

> A **simple type copy** need not be indicated; a **structure that is unbound** must
> **always** be indicated.

Each step has a runnable check on BOTH backends. The change is **diagnostic-only** — it
edits classification in `src/use_analysis.rs`, emits **no** new `ElidePlan`, and must leave
the lowered program **byte-identical** (the loft-codegen Mode-B gate: `loft introspect`
before/after is empty except the `MAT` dump line). Follow that gate strictly.

## What this is: a flag-gated REPORT now, staged to CLOSE later

This ships **off by default, behind a flag** — deliberately. We do **not** enforce yet; we
want to *read the data first*. Turning the flag on answers two questions:

1. **What still copies that we can fix** — the *Avoidable* set is our worklist: each row is a
   copy the compiler should learn to eliminate (drain bucket 2 → bucket 1). This audience is
   **us** (compiler devs).
2. **Where the hidden cost is** — a survey across a whole **library or program**: every
   *unbound* copy site, ranked, so a lib author / user can see the cost the alias-default was
   hiding. This audience is **them**.

So the primary deliverable is a **report** (aggregate rollup + per-site rows with locations),
not a warning that blocks a build. Enforcement is a **later, separate** decision — see the
lifecycle.

### Lifecycle to close (why the flag is temporary)

| phase | state | gate |
|---|---|---|
| **A — survey (this doc)** | flag-gated report; classify correctly (the survival split); read libs + programs | flag OFF by default |
| **B — drain** | grow auto-elision (bucket 2 → 1) until the *Avoidable* set is empty for the cases we can fix | worklist shrinks toward the forced remainder |
| **C — accept the remainder** | the copies that genuinely cannot be borrowed/moved are accepted; a library marks each with a **specific per-site opt-out annotation** so its PR is copy-clean | annotation, never a global allow |
| **D — close** | with Avoidable drained and the forced remainder annotated, promote the flag to an **enforced** lint / library-PR gate — every remaining copy is now either eliminated or explicitly acknowledged | plan #90 closes here |

**The opt-out must be SPARSE — a specific annotation at the copy site, NEVER a global/file
switch.** The inverse of `&`: an explicit copy-intent form (`.copy()` / `own(...)`, exact
surface TBD in [COPY_DIAGNOSTICS.md § escape hatch](../../COPY_DIAGNOSTICS.md)) that silences
*that one* copy. A global `allow(copies)` would re-hide the whole class and defeat phase D — the
whole point is that every accepted copy stays individually visible and auditable. A library PR
is "copy-clean" when every copy it emits is either eliminated (bucket 1) or carries the
per-site annotation — not when it flipped a blanket suppression.

**Do NOT flip the flag default-on in this plan** (that is phase D, gated on B+C). This plan
delivers phase A: the correct classification + the report + the flag.

## The bug (one line)

`construct_copy` (`S { f: src }`, `x.field += src`) and `record_copy` (`v[i] = e`, a struct
copy, `?? E{…}`) are **blanket-classified `CopyClass::Implicit` → silent**
(`use_analysis.rs:539–568`), with **no source-survival check**. So a construction / slot-set
that **duplicates a still-live source** — a genuine *unbound* structure copy — is silently
swallowed, exactly the copy the user wants surfaced.

## The invariant to enforce

> The Implicit/Avoidable/Forced split is keyed on the copy's **source fate**, never on the
> op that emitted it. **Bound** result (scalar · move — source consumed here · literal /
> freshly-built source) → **Implicit, silent**. **Unbound** result (a still-live pre-existing
> source is duplicated) → **indicated** (Avoidable if a borrow/move would have avoided it;
> Forced if genuinely required). No lowering changes; the suite stays byte-identical.

## Root cause (localized)

Two producers skip the survival analysis the var-buffer path already does:

- `use_analysis.rs:282` — `self.construct_copy.push((rec, src))` (records only dest+src).
- `use_analysis.rs:310` — `self.record_copy.push((tgt, src))` (records only dest+src).
- `use_analysis.rs:539–568` — both loops emit `class: CopyClass::Implicit` unconditionally,
  with reasons "struct/enum field owns its data" / "record deep-copy (OpCopyRecord)".

The var-buffer branch (`use_analysis.rs:454–489`) already keys its class on ordering facts
(`single_def`, `copyfill_pos`, `other_max_pos`, `src_local_stable`) — the same shape of fact
the two producers need. `src.is_none()` (a literal source) is already treated as Implicit at
`:467` for the var-buffer path — reuse that for the two producers verbatim.

## The discriminator — "does the source survive the copy?"

For a copy at position *P* whose source base var is `src`:

| case | test | bucket |
|---|---|---|
| **literal / built-for-this** | `src.is_none()` | **Implicit** (born owned — nothing duplicated) |
| **move** | `src`'s last use is the copy itself (no use strictly after *P*) | **Implicit** (single backing transfers) |
| **unbound, avoidable** | `src` survives (a use after *P*) AND is not independently mutated | **Avoidable** (a borrow/move would remove it — the worklist) |
| **unbound, forced** | `src` survives AND is later mutated independently, or is a temp that can't outlive the result | **Forced** (required, informational) |

**Position boundary — MEASURE it, don't assert it (engineering-rigor).** Pre-order `pos`
increments per visited node (`use_analysis.rs:211`); the copy CALL node reads `src` in its own
subtree, so a naive `last_use_pos[src] > P` would count the copy's own read. Record the
copy-site position as `self.pos` *after* the call's args are visited (the position just past
the whole copy expression) and compare `last_use_pos[src]` (max pos over ALL `Value::Var(src)`
visits, reader + non-reader — a new unconditional bump in the `Value::Var` arm at `:214`)
against that. A copy inside a loop (`copyfill_in_loop` / `loop_depth > 0`) re-reads `src` every
iteration → treat as **survives** (a per-iteration duplicate of a live source is a real
repeated copy). Every cell of this table is proven by the Step-4 matrix, never by reading the
code.

**The move-as-copy subtlety (record it, keep it user-silent).** If the lowering physically
copies even when `src` is consumed (an `OpCopyRecord` that could be a move), the *result* is
still bound — the source dies, so no independent duplicate coexists and the user cannot observe
a divergence → **user-silent (Implicit)**. But it is a compiler inefficiency (copy-then-free
instead of move) → keep it in the developer DUMP under a distinct reason
(`"move lowered as copy"`) so it stays on *our* worklist without adding user noise. The
user-facing lint fires only on the *survives* subset; the DUMP sees all.

> **STATUS 2026-07-05 — Steps 0–5 LANDED (gated).** The survival split + the four
> resolve-before-flip items (items 1–4 in [survey-findings.md](survey-findings.md)) are done, and
> **Step 5 — the user-facing `--report-copies` report** ships: `loft --report-copies prog.loft`
> (or `LOFT_REPORT_COPIES=1`) prints, ONCE (from `main` after the whole program is loaded), the
> *unbound* structure copies the user made — `Avoidable` (a `&`/restructure would remove it) and
> `Forced` — each with a location, the copied type and its reason, then a rollup + the Avoidable
> worklist. It shows only the SURVIVAL-SPLIT class (source duplications, `VerdictRow.survival`);
> the var-buffer / return-buffer copies (the elision/`__retbuf` class, where the stdlib's copies
> land — survival baseline 0) stay in the developer dump, so the report is user-only. All still
> gated + diagnostic-only (default off, suite byte-identical + green). Guard
> `report_copies_is_user_facing_and_prints_once`. **Remaining: Step 6 — graduation is DEFERRED to
> phases B–D** (drain Avoidable → auto-elision; accept the remainder behind the sparse per-site
> opt-out; only then flip to an enforced library-PR gate). Do NOT flip the default on here.

## Verifiable steps (each: run the check on BOTH backends)

**Step 0 — the gate (capture the current lie).** Build a one-fn-per-case corpus
`bytecode-comparisons/unbound-copy-corpus.loft` with one small fn per cell (construct-literal,
construct-move, construct-survives, slot-set-literal, slot-set-move, slot-set-survives,
record-copy-survives) + a `main` running them all. Capture the current classification:
`LOFT_COPY_DUMP=1 loft --interpret unbound-copy-corpus.loft 2>&1 | grep MAT`. **Check:** the
*survives* cells print `bucket=implicit` today — that is the bug on record. Also
`loft introspect unbound-copy-corpus.loft > before.txt` (both backends, via the native block)
and confirm the corpus is value- + leak-clean (`LOFT_STORES=warn`, `LOFT_NATIVE_LEAK_CHECK=1`).

**Step 1 — add the survival fact (inert).** Add `last_use_pos: HashMap<u16, usize>` (bump in
the `Value::Var` arm, unconditional). Extend `construct_copy` / `record_copy` to
`Vec<(Option<u16>, Option<u16>, usize)>` recording the copy-site-end position (move the two
`push`es to *after* the args loop; capture `self.pos`). **Check:** no classification reads the
new fields yet → `LOFT_COPY_DUMP` output and the suite are **unchanged** (the field is carried
inert first — the [[analysis-first-instrument-gated]] method).

**Step 2 — the survival selector (pure).** Add a pure helper
`fn survival_class(src, copy_pos, u, function) -> CopyClass` implementing the table above
(literal → Implicit; last-use ≤ copy_pos → Implicit; survives + unmutated → Avoidable;
survives + mutated/temp → Forced). Unit-probe it against the corpus's known cells before wiring.
**Check (positive control):** a gated `eprintln` prints the verdict per cell; it reads
`implicit` for the move/literal cells and `AVOIDABLE`/`forced` for the survives cells — the
inverse of Step 0. Only then trust it.

**Step 3 — wire it (gated).** In the two loops (`:539`, `:557`) replace the hard-coded
`CopyClass::Implicit` with `survival_class(...)`, behind an env gate
(`LOFT_COPY_SURVIVAL`, default off) so Step 4 can diff on/off. **Check:** gate OFF →
`loft introspect` byte-identical to `before.txt` (no lowering change, ever — this is
diagnostic-only); gate ON → the survives cells flip to indicated.

**Step 4 — the boundary matrix.** Cells `{construct, slot-set, record-copy} × {literal, move,
survives, survives+mutated} × {interp, native}`. Per cell assert: (a) the **bucket** matches
the table; (b) the lowered program is **byte-identical** to gate-off (`loft introspect` diff
empty — the Mode-B proof that nothing but the diagnostic moved); (c) **value + length + leak**
unchanged on both backends (a diagnostic must never perturb the run). **Check:** every
*survives* cell is indicated, every *move/literal* cell is silent, and the emitted program is
identical to before on both backends. Prove the move-as-copy note by reading the runtime
`LOFT_COPY_DUMP` for the move cells (a physical copy there → the `"move lowered as copy"`
reason, still user-silent).

**Step 5 — the REPORT (the phase-A deliverable, flag-gated, NOT a default warning).** Behind a
flag (`--report-copies` / `LOFT_REPORT_COPIES`, default off), print two views over the
**unbound** set — this is a *survey instrument*, not a build-blocker:
- **per-site rows** — `<file:line> copies <T> — <bucket> (<reason>); avoid: <& / restructure>`,
  so a lib author sees each hidden cost with its location and fix hint;
- **an aggregate rollup** — counts per bucket for the whole compilation unit
  (`copies: 12 avoidable, 3 forced, 41 implicit(silent)`) plus the ranked *Avoidable* worklist,
  so we can survey a library or program at a glance and prioritise the drain.

Implicit (bound) rows never appear in the report; Forced rows are informational; Avoidable rows
are the worklist. **Check:** on the corpus, the survives cells each produce a row + increment
the rollup; the move/literal/scalar cells produce none; a borrowed match-yield
(`match e { Filled{items} => items }`) produces none (it is `Eliminated`, F2 — never a false
positive). Run it over a real library (e.g. `lib/markdown`, `lib/audience_crystal`) and a demo
program — the rollup *is* the phase-A survey the user asked for.

**Step 6 — guard + LEAVE FLAG-GATED (do NOT enforce here).** Promote the corpus cells to a
`tests/scripts/` guard (`90-unbound-copy-lint.loft` — assert the classification via
`LOFT_COPY_DUMP`, and that the run is value/leak-clean). Run the full suite
(`find_problems.sh --bg`), both backends, poison-clean. Update
[COPY_DIAGNOSTICS.md](../../COPY_DIAGNOSTICS.md) and [REMAINING.md](REMAINING.md) to
"phase A landed". **The flag stays OFF by default** — enforcement (phase D) is a *separate*
plan step gated on draining the Avoidable set (phase B) and the per-site opt-out annotation
(phase C). Record the phase-B worklist (the ranked Avoidable rollup from Step 5) and the
phase-C annotation design as the follow-ups; do not flip the default here.

## Do-not-ship conditions (revert, don't push through)

- Gate-ON changes ANY lowered byte (`loft introspect` diff non-empty) → this is not
  diagnostic-only anymore; you touched codegen. Revert.
- A **move** or **literal** cell reads as indicated (F7 false positive), or a **survives**
  cell reads as `implicit` (the original bug) → the survival selector is wrong; fix the
  position boundary, re-run the matrix.
- One backend classifies a cell differently from the other → the fact isn't backend-agnostic
  (it should be — `use_analysis` runs once, pre-codegen); investigate before landing.
- Any cell perturbs value / length / leak → a diagnostic changed the run; revert.
