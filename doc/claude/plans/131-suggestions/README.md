<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 131 — Suggestions: tell the author what to write instead

**Status — SHIPPED 2026-08-06.** Tracker: [@PLN131](https://github.com/loft-lang/plans/issues/131) ·
started from [@PLN130](https://github.com/loft-lang/plans/issues/130)'s copy notice.

**The reference lives in [DIAGNOSTICS.md](../../DIAGNOSTICS.md)** — the code index, the fix
tiers, `--explain`, `loft fix`, the editor surfaces, and the rules a new code has to satisfy.
The user-facing capability is catalogued as `@F110`. This file is the closure record: what
was decided, and what the build learned that the design had not.

## What shipped

All five ship steps, plus coverage the plan did not ask for.

| | |
|---|---|
| diagnostic codes | 49 sites — **100% of warnings and advice** (26/26, 11/11), plus 12 errors |
| fixes | every coded diagnostic offers one; tiered `Mechanical` / `Conditional`, each with a concept and a catalogue door |
| `--explain` | the fix lines, opt-in, with a once-per-run pointer so a quiet run still says where they are |
| `loft fix` | checks a fix by APPLYING it to an in-memory copy and re-running the analysis |
| `loft fix --apply` | writes the mechanical, verified ones; never a conditional one |
| editor | quick-fixes, `relatedInformation` for every fix, `codeDescription`, and `source.fixAll` delegated to the same applier |

Guards: `tests/suggestions.rs` (23) and `tests/e1_code_set.rs` (5, beside the pinned code
set). `tools/diag_inventory.py` reports coverage — the numbers above are a snapshot, the
tool is the source.

## Open questions — all resolved

- **Q1 — where does a suggestion live?** Beside the diagnostic that raised it: `fix_last`
  attaches to the entry just emitted, so a fix cannot drift from its diagnostic.
- **Q2 — what is the IDE surface?** One `Fix` shape renders to both. The CLI prints
  `title` + `condition`; the LSP puts `title` on the lightbulb and `condition` in the
  confirm step. Neither formats its own.
- **Q3 — how is a suggestion tested?** Both halves. The door check reads every `@F` out of
  every pinned code's `--explain` output and checks the catalogue, so a renumbered feature
  breaks the build. The applied-result half is `applying_a_fix_produces_a_program_that_runs`
  — the fixed program must compile AND print what the author wanted.
- **Q4 — how much verification is affordable?** Measured: ~48 ms per fix, about one full
  parse. Affordable in `loft fix` (a deliberate invocation), not in `--explain` (every
  compile), and not on `didChange`. If it ever runs in an editor it belongs on the
  code-action *resolve* step.
- **Q5 — anchor on the diagnostic CODE, not a catalogue id.** A code names the diagnostic;
  an `@F` id names the concept the fix uses. Both are carried, for different jobs.
- **Q6 — the two prerequisites**, both cleared: the condition names the surviving use by
  line, and `@F106` gave `move` its door.

## What the build learned that the design had not

1. **An edit needs a SPAN, and that — not the count of fixes — was the real prerequisite.**
   `Fix.edit` was a bare string with no idea where it went, and the diagnostic's own position
   cannot stand in: by detection time the lexer has often drifted past the statement
   terminator. Spans are captured where the parser knows them, and measured rather than
   reasoned — hand-counting got it wrong twice.
2. **A fix's tier is a property of the EVIDENCE, not of how short the rewrite looks.**
   `lost-write`'s fix is one token and still conditional, because the analysis proves the
   write is lost and never which resolution was meant. Both checked-cast fixes were
   `Mechanical` in name only until this was applied to them.
3. **Ranking is on soundness first; teaching is the tiebreak between fixes that both hold.**
   Two codes cost a tier to get this right.
4. **A fix may only spell an edit it can PLACE.** "Drop the `#superseded` attribute" is a
   rewrite the compiler knows exactly and still gets `edit: None`, because the diagnostic
   sits at the definition and not at the attribute's span.
5. **An unmasked error is not one the fix caused.** Pass 1 erroring means pass 2 never runs,
   so fixing the blocker lets the next parse reach diagnostics the first could not report.
   A positional rule was proposed and **measured wrong** — a brace hid a cast three lines
   below it *and* another two lines above. The mechanism is the phase, not the line.
6. **Giving a diagnostic a code turned it into a build break, in seven places.** Each
   classified by rendered prefix (`"Advice:"`), and a coded diagnostic renders
   `Advice[code]:` — matching none of them, and every one of those sites treats "not a
   warning" as "an error". The classifier now lives beside the renderer that produces the
   string.
7. **Verification pays for itself on real cases, not in principle.** It refused the checked
   cast where the target is annotated non-null, and refused `helpr` → `helper` where the
   spelling is right and the call does not fit.

## Decided, not deferred

- **The program is never run.** The design asked for a behaviour comparison across both
  backends; that means executing the author's code as a side effect of asking what to write
  — code that may write files, take a network turn, or not terminate. Verification is
  static, and stops where acting on the author's behalf begins.
- **The 500-odd remaining errors stay uncoded.** A code is frozen the moment it ships;
  minting one for a message that will never carry a fix buys a permanent name for nothing.
  Code an error when it gets a fix — which is how the did-you-mean family earned theirs.

## Residue — small, and named

- `DiagEntry::suggestion` is fully shadowed by `fixes` at every current site; retiring the
  field is a small, separate change.
- `suggest_field_name` misses `nme` → `name`, so `unknown-field` fires less often than its
  edit-distance budget suggests — a weakness in the suggestion channel, not in this
  machinery.
- The copy notice still cannot spell an `edit`: the IR desugars `S { f: src }` and
  `x.field += src` to the same node, so a synthesised "build it in place" would be offered
  for the append shape, where it does not exist. Distinguishing them at parse time is
  @PLN130's territory.
- Two codes have no single-file trigger, listed in `NO_MINIMAL_TRIGGER` with reasons.
  `missing-return-path` is **unreachable** — gated on the deprecated `not null` return
  spelling and pre-empted by a hard error.

## See also

- [DIAGNOSTICS.md](../../DIAGNOSTICS.md) — the reference.
- [`fix-line-prototype.md`](fix-line-prototype.md) — the prototype that found Q6's two
  prerequisites.
- @PLN130 — the copy notice this started from.
