<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 131 — Suggestions: tell the author what to write instead

Tracker: [@PLN131](https://github.com/loft-lang/plans/issues/131) · starts from
[@PLN130](https://github.com/loft-lang/plans/issues/130)'s copy notice.

**Status:** future — design written, nothing built.

A diagnostic tells you something is wrong. It rarely tells you what to *write instead*, and
that second half is where most of the learning is. The model here is Eclipse's quick-fix: its
lasting value was never the automation — you often did not apply the proposed fix — it was
that each proposal **taught you a language capability you did not know existed**.

So this is an *explain* feature that can optionally apply, not an auto-fixer.

## Goal

For a diagnostic loft raises, offer the concrete resolution(s) in the author's own code — the
variable names, the line, the rewrite — so that following the suggestion is a way of learning
the language rather than obeying the compiler. Opt-in to apply; never applied silently.

## The two hard rules

1. **Opt-in.** A suggestion is shown; the author chooses. Nothing rewrites source on its own.
2. **Sound.** A suggestion that changes what the program DOES is worse than no suggestion —
   it turns a diagnostic into a bug, and the author trusted us. This is the same rule
   @PLN130 arrived at from the other side: a warning does not buy off breakage, and neither
   does a fix.

Rule 2 is the whole engineering problem. Rule 1 is a flag.

## Soundness has two tiers, and they must be visibly different

@PLN130's copy notice already shows why one tier is not enough. Its advice names two ways out,
measured to take the diagnostic to zero:

```loft
src = [1, 2, 3];  h = Holder { v: src };  use(src);   // copies
src = [1, 2, 3];  h = Holder { v: src };              // move — no copy
h = Holder { v: [1, 2, 3] };                          // built in place — no copy
```

- **Mechanical** — "build the value in place" is a rewrite whose meaning is determined by
  the code alone. It can be offered as a concrete diff and applied.
- **Conditional** — "stop using `src` afterwards" depends on whether the author still NEEDS
  that use. loft cannot know. Offering it as a diff invites a silent behaviour change; it
  must be phrased as a question and never auto-applied.

Conflating the two is the failure mode to design against. A suggestions feature that
auto-applies a conditional rewrite is a bug generator with good intentions.

## What loft can do that an IDE quick-fix historically could not: verify its own advice

The compiler holds the analysis that raised the diagnostic. So a candidate rewrite can be
**executed before it is offered**:

1. apply the candidate to an in-memory copy of the source,
2. re-run the analysis — does the diagnostic actually disappear?
3. run the program's tests / compare behaviour on BOTH backends — is it unchanged?

Only then is it shown. A suggestion that has been tried is a different class of artefact from
one that was pattern-matched, and it makes rule 2 checkable rather than aspirational. This is
@PLN130's method turned into a user-facing feature: do not ship a fix you have not measured.

Cost is the open question — step 3 is expensive, and it may only be affordable for mechanical
rewrites, or behind an explicit `--verify`.

## Scope — start with the copy notice

Deliberately one diagnostic, because it is the one where the resolutions are already
**measured** rather than imagined (@PLN130 F5): two rewrites, one mechanical, one conditional.
That makes it a real test of the machinery — including the tier split — without first needing
a catalogue of rewrites for every diagnostic.

Ship order:

1. `--explain` (or `loft explain prog.loft`) expands each copy notice into: what happened,
   why, and the candidate resolutions written in the author's own names.
2. Mark each candidate mechanical or conditional; show conditional ones as questions.
3. Self-verification for mechanical candidates (steps 1–2 above at minimum).
4. `--apply` for verified mechanical candidates only.
5. Only then consider a second diagnostic.

## Open questions

- **Q1 — where does a suggestion live?** Next to the diagnostic definition, or in a separate
  table? A suggestion that drifts from its diagnostic is misinformation, so one home.
- **Q2 — what is the IDE surface?** The eventual target is an LSP code action. Does the
  compiler emit structured suggestions (JSON) that both the CLI and the LSP render, rather
  than each formatting its own?
- **Q3 — how is a suggestion tested?** Probably: a fixture with the diagnostic, the applied
  result, and an assertion that the diagnostic count drops and behaviour is identical. Same
  shape as @PLN130's probes.
- **Q4 — how much verification is affordable** at `--explain` time versus `--apply` time?

## See also

- @PLN130 — the copy notice this starts from, its measured resolutions, and the
  breakage/misinformation bar this inherits.
- `doc/claude/CODEGEN_METHOD.md` § the companion principle — build the instrument, and let it
  name the gate on a fix rather than only the fix.
