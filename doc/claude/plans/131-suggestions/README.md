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

So this is a *fix-delivery* feature: the resolution is the product, and the concept it
names is a door to the documentation rather than a lecture inside the message.

## Goal

For a diagnostic loft raises, offer the concrete resolution(s) in the author's own code — the
variable names, the line, the rewrite — each carrying the NAME of the capability it uses so a
reader who wants more has somewhere to go. Opt-in to apply; never applied silently. The
diagnostic keeps saying what is wrong; the suggestion says what to write instead; the docs
explain why.

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
  the code alone. Safe to apply, including unattended.
- **Conditional** — "stop using `src` afterwards" depends on whether the author still NEEDS
  that use. loft cannot know; only the author can.

The tiers control **who may affirm the condition**, not whether a suggestion is clickable:

| | interactive (one click) | unattended (`--apply`, batch, CI) |
|---|---|---|
| mechanical | yes | yes |
| conditional | **yes — the click IS the affirmation**, provided the condition is stated in the line the author reads | **never** |

So a conditional suggestion is still a one-click fix for a veteran: *"if `src` is not needed
after line 12, deleting that use makes this a move"* is something they can judge instantly
about their own code, and clicking asserts it. What is forbidden is applying it with nobody
reading the condition.

The failure mode to design against is therefore not "offering conditional suggestions" but
**stating a condition badly** — a click that affirms something the author did not actually
read is how a suggestions feature becomes a bug generator with good intentions.

## The FIX is what we deliver — the rest is a door, not a lecture

The suggestion's product is **the fix**. What is *wrong* already belongs to the
error/warning and is stated there; a suggestion that re-explains it is duplication the reader
pays for every time. And the deeper material — what a move is, why a borrow works — belongs
in the documentation, not inlined into a compiler message.

So the reported Eclipse effect (a proposal sending you off to forums and docs) is served by
an **opening**, not by embedding the content:

```
copy of vector<integer> — `src` is still used after this point
  fix: build the value in place —  h = Holder { v: [1, 2, 3] }         [move · @F41]
  fix: if `src` is unused after line 12, delete that use               [move · @F41]
```

Three homes, no repetition:

| what | where |
|---|---|
| what is wrong | the diagnostic (already) |
| what to write instead | the suggestion — the deliverable |
| why, and what the concept means | the linked doc — followed only if wanted |

Design rules that follow:

1. **The fix line must stand alone.** A veteran reads it, agrees, clicks. No paragraph
   between the problem and the resolution.
2. **Name the concept as a handle, not an explanation.** `move` is the searchable noun; it
   opens the door. A sentence defining a move inside the message is the lecture we are
   avoiding — it belongs behind the link.
3. **The link must resolve to something real.** loft has a canonical home: the feature
   catalogue (`loft-lang/features` → `doc/features/`). A door onto nothing is worse than no
   door (see Q5 — derive the id, never hand-write a URL).
4. **Prefer the suggestion that teaches** when two are equally sound. Between "build the value
   in place" and "drop the later use", the first introduces an idiom reusable everywhere;
   the second is a local deletion. Rank on what it opens up, not on brevity.

Test that is otherwise hard to state: **a suggestion that names no concept and links nowhere
is not finished**, however correct its diff — and a suggestion that explains the concept
inline is not finished either, because it has taken the documentation's job.

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

1. `--explain` (or `loft explain prog.loft`) prints the FIX line(s) under each copy notice —
   the rewrite, plus a concept handle and its catalogue link. No inline prose.
2. Mark each candidate mechanical or conditional, and state a conditional one's condition in
   the resolution line itself — that line is what a clicking author affirms.
3. Self-verification (apply to an in-memory copy, re-run the analysis, compare behaviour).
4. Apply: interactive one-click for both tiers; unattended `--apply` for mechanical only.
5. Only then consider a second diagnostic.

## Open questions

- **Q1 — where does a suggestion live?** Next to the diagnostic definition, or in a separate
  table? A suggestion that drifts from its diagnostic is misinformation, so one home.
- **Q2 — what is the IDE surface?** The eventual target is an LSP code action. Does the
  compiler emit structured suggestions (JSON) that both the CLI and the LSP render, rather
  than each formatting its own?
- **Q3 — how is a suggestion tested?** Probably: a fixture with the diagnostic, the applied
  result, and an assertion that the diagnostic count drops and behaviour is identical. Same
  shape as @PLN130's probes. Plus the doc-link check from § entry point: assert every
  suggestion names a concept and resolves to a real catalogue entry, so a renamed or deleted
  feature breaks the build rather than shipping a dead door.
- **Q5 — can the concept link be derived rather than hand-written?** The feature catalogue is
  canonical and generated (`make features-gen`), so a suggestion could carry an `@F`/`@I` id
  and let the renderer resolve it — one home, and a link that cannot drift from the feature it
  describes. Hand-written URLs in diagnostics would rot exactly like the stale doc claims
  @PLN130 F6 had to correct.
- **Q4 — how much verification is affordable** at `--explain` time versus `--apply` time?

## See also

- @PLN130 — the copy notice this starts from, its measured resolutions, and the
  breakage/misinformation bar this inherits.
- `doc/claude/CODEGEN_METHOD.md` § the companion principle — build the instrument, and let it
  name the gate on a fix rather than only the fix.
