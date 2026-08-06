<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 131 — Suggestions: tell the author what to write instead

Tracker: [@PLN131](https://github.com/loft-lang/plans/issues/131) · starts from
[@PLN130](https://github.com/loft-lang/plans/issues/130)'s copy notice.

**Status:** active — the prerequisite arc and ship steps 1–2 and **5** are BUILT; steps 3–4
are open.

Built: diagnostic codes (index at [DIAGNOSTICS.md](../../DIAGNOSTICS.md)), the surviving
use's location through `VerdictRow` (Q6.1), the structured `Fix` shape (`kind` / `title` /
`condition` / `edit` / `concept` / `concept_ref` — which answers Q2 as a side effect),
`--explain` rendering tiered fix lines, and fixes on **8 of the 10 codes**. Guards in
`tests/suggestions.rs` (the copy notice) and `tests/e1_code_set.rs` (every code).

```
advice[avoidable-copy]: copy of vector<integer> — `src` is still used after this point …
  fix  build the value in place   [move · @F106]
  fix  drop the later use of `src`   needs: `src` is used again at line 8 — you do not need that   [move · @F106]

error[format-unescaped-brace]: a literal `}` in a format string must be written `}}`
  fix  double the brace   }}   [interpolation · @F35]
```

Open: **3** (self-verification) and **4** (apply). Both wait on the same thing, recorded
under Q6: too few fixes spell an `edit`. Step 5 raised that count from zero to one —
`format-unescaped-brace` is the first fix whose rewrite the code alone settles, and
`as T?` on the two cast codes is the second and third — which is what finally makes 3 and 4
testable on something.

Step 5's finding: **the tier is a property of the diagnostic's evidence, not of the fix's
shape.** `lost-write` reads like it has a mechanical fix (`d = &s.items` is a one-token
edit), but the analysis proves only that the write is lost, never which of the two
resolutions the author meant — so both are `Conditional`. A tier assigned from how the
rewrite LOOKS rather than from what the analysis KNOWS is how the unattended lane
(step 4) would apply a guess.

Two codes are **blocked**, both on the same missing piece: `superseded-unknown-successor`
and `superseded-not-folded` have `#superseded` itself as the concept, and the feature
catalogue has no entry for it. This is exactly the Q6.2 prerequisite that `@F106` cleared
for `move`. `FIX_BLOCKED` in `tests/e1_code_set.rs` is the live list; filing the catalogue
entry empties it.

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

## Prerequisite arc — diagnostic codes (adopt what already exists)

A suggestion needs a **stable identity** to attach to: the message prose changes, the
suggestion must not. loft already decided this and built it — `Diagnostics::add_at_coded`,
whose own doc says *"the code is the frozen identity; prose is free"* (@PLN102 arc-E E1) —
and then never adopted it:

| | count |
|---|---|
| coded call sites | **1** |
| uncoded `add_at` | 15 |
| uncoded `add` | 3 |

There is also no index of codes anywhere in the docs, so nothing can be looked up.

**This is the right anchor, and it beats the catalogue id in Q5.** A code names the
*diagnostic*; a `@F` id names a *feature*. The suggestion, the doc section, the test and the
grep all want the former. One identity, four consumers:

- the message carries it, so a user can search it;
- the suggestion attaches to it rather than to a message string that drifts;
- `doc/claude/DIAGNOSTICS.md` is named by it — the door @PLN131 needs;
- `grep -rn "<code>"` finds emitter, tests and docs together.

loft chose **kebab slugs, not numbers** (`avoidable-copy`, not `E0142`), which is the stronger
choice here: a slug is self-describing *and* greppable, while a number means nothing until the
lookup table exists.

Two costs to respect:

1. **A code is a public surface — frozen once emitted.** The naming pass matters more than
   the edit; a renamed code breaks every link and search that ever pointed at it.
2. **A code with nothing to grep to is the same dead door** the prototype already rejected.
   The index must land WITH the codes, not after.

Order: code @PLN130 F5's copy notice first (it is the diagnostic this plan builds on, and it
currently ships with **no** code), then the remaining sites, then the index.

## Open questions

- **Q1 — where does a suggestion live?** Next to the diagnostic definition, or in a separate
  table? A suggestion that drifts from its diagnostic is misinformation, so one home.
- **Q2 — what is the IDE surface?** The eventual target is an LSP code action. Does the
  compiler emit structured suggestions (JSON) that both the CLI and the LSP render, rather
  than each formatting its own?
- **Q3 — how is a suggestion tested?** The doc-link half is RESOLVED and generalised:
  `every_offered_door_resolves_to_a_catalogue_entry` reads every `@F` door out of every
  pinned code's `--explain` output and checks it against the catalogue snapshot, so a
  renumbered or deleted feature breaks the build whichever fix pointed at it — and
  `every_pinned_code_offers_a_fix` makes a code with no fix red rather than merely
  unfinished. Both live beside `CODES` in `tests/e1_code_set.rs`, for the reason the
  doc-index check does: the pinned set is the one home for "which codes exist", and a second
  scan can disagree with the first.

  Still open is the half that needs step 3: a fixture with the diagnostic, the applied
  result, and an assertion that the diagnostic count drops and behaviour is identical.
- **Q6 — RESOLVED, both prerequisites** ([`fix-line-prototype.md`](fix-line-prototype.md)).
  The condition now names the surviving use by LINE: the walker already tracked `cur_pos` for
  the copy site, so recording it at the USE too (`last_use_loc`, kept in step with
  `last_use_pos` so the two never describe different uses) carries it to `VerdictRow`. And
  the `move` concept has its door — `@F106` covers copy and move semantics.

  **What the build added to the prototype's finding.** The prototype said the append shape
  has no mechanical fix; the stronger fact is that loft cannot presently tell the two shapes
  apart at all. The IR desugars `S { f: src }` and `x.field += src` to the same
  `OpAppendVector(OpGetField(…), src)` node, so `construct_copy` holds both. That is why no
  fix currently spells an `edit`: a synthesised "build it in place" rewrite would be offered
  for the append shape too, where it does not exist. **Distinguishing them at parse time is
  the prerequisite for step 3**, and it is a bigger piece of work than Q6.1 was — the fix
  line is still useful without it, because the title and the concept carry the teaching.

  **What step 5 changed.** That finding is about the copy notice, not about fixes in
  general: three of the eight now spell an `edit`. `format-unescaped-brace` spells `}}`,
  which the code alone settles completely, and the two cast codes spell `as T?`. So steps 3
  and 4 no longer wait on the IR shape split to have anything to run against — they can be
  built and proved on the brace fix first, and the copy notice joins when the split lands.
- **Q5 — RESOLVED: anchor on the diagnostic CODE, not a catalogue id.** See § Prerequisite
  arc. A code names the diagnostic (what the suggestion belongs to); an `@F` id names a
  feature (what the concept links to). Both exist — `@F106` now covers copy/move semantics —
  but the suggestion attaches to the code, and the code's doc entry links onward to the
  feature. Hand-written URLs inside diagnostics are still refused: they rot exactly like the
  stale doc claim @PLN130 F6 had to correct.
- **Q4 — how much verification is affordable** at `--explain` time versus `--apply` time?

## See also

- @PLN130 — the copy notice this starts from, its measured resolutions, and the
  breakage/misinformation bar this inherits.
- `doc/claude/CODEGEN_METHOD.md` § the companion principle — build the instrument, and let it
  name the gate on a fix rather than only the fix.
