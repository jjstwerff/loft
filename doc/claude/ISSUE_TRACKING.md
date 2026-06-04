<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->
# Issue tracking — investigations in files, open bugs in GitHub Issues

**Status: DRAFT (2026-06).**  Pilot done (#246/#247); the rest of the migration is
gated on approval.  Rationale + evaluation: this is the multi-project answer —
loft / dryopea / lavition / `loft-lang/*` all have bug-filing needs, and discrete
bugs are a commodity that GitHub Issues serves better than N per-repo markdown
files, while **investigations** (which work, and which GitHub can't hold) stay in
files.

## The split — what lives where

| Kind | Home | Why |
|---|---|---|
| **Investigations** (multi-phase, probe-driven bug *classes*) | files: `plans/*/` + `probes/` + cluster catalogue | working artifacts GitHub can't hold (probe corpora, cluster docs); agent-grepped; versioned with the code |
| **Open bugs** (discrete: repro / severity / workaround / fix-path) | **GitHub Issues**, per repo | commodity records; cross-repo refs; triage / search / milestones; external discoverability (Goal B); one uniform tool across all repos |
| **Closed bugs** (the FIXED record) | `PROBLEMS.md` (frozen archive) + git history + the regression test | the fix + its test ARE the record; keep the history greppable |
| **Design entries** (the big `###` P-sections, e.g. @P213) | reference docs (`doc/claude/*.md`) | these are design docs, not bug rows — they don't belong as one-line Issues |
| **Benign tradeoffs / open work** (not defects) | `QUALITY.md § Open work` | a known tradeoff with a fix mapped, not a bug |

**Investigations PRODUCE Issues.**  The closure rule
([`plans/_INVESTIGATION_TEMPLATE.md § Closing`](plans/_INVESTIGATION_TEMPLATE.md#closing-an-investigation-plan-required))
now files **GitHub Issues** (not PROBLEMS.md rows) for the still-open findings +
sibling bugs.  The file-based deep-dive emits commodity bug records into the
commodity tool; the active-phase no-file rule is unchanged.

## Convention (apply in EVERY repo — this is what makes multi-project pay off)

Without one shared convention, N repos' Issues drift exactly like N PROBLEMS.md
files would.  The win is the *uniform* convention, not GitHub itself.

- **Templates** — copy `.github/ISSUE_TEMPLATE/{bug_report,feature_request}.yml`
  into each repo (dryopea, lavition, `loft-libs-*`).
- **Title** — `[<area>] <one-line>`.  For any issue migrated from PROBLEMS.md,
  keep the legacy `@P###` token in the title so OLD doc references still grep
  (`gh search issues "@P396"`).  The canonical way docs *reference* an issue is the
  indexed `@GH###` token (below), not the bare gh number.
- **Reference token: `@GH###`** (the indexed tracker) — docs reference a gh issue
  as `@GH<number>` (e.g. `@GH247`), NOT bare `#247` / `loft#247`.  The `@`-prefix
  makes it an INDEXED token exactly like `@P###` / `@PLAN###`: `scan.loft` finds
  every reference + backlink **fully offline**, and `@GH247` maps to a
  deterministic URL (`github.com/<repo>/issues/247`) with no `gh` call.  Token
  families: `@P###` = legacy/closed (PROBLEMS.md archive), `@PLAN###` = plans,
  `@GH###` = live issues.  Optional validation (does it exist / is it closed) is a
  bolt-on `make index-gh` (`gh issue list --json number,state`), not a
  prerequisite.  Cross-repo: bare `@GH###` = this repo; a qualified spelling for
  other repos (`@GH:<repo>:<n>`) is TBD and the less-common case.
- **Labels** — meanings live in [`.github/LABELS.md`](../../.github/LABELS.md)
  (the glossary so other agents don't dig into the loft source to learn what
  `area:codegen` covers) + each label's GitHub `description` (the inline gloss).
  Beyond GitHub's defaults `bug`/`enhancement`/…:
  - severity: `sev:high` / `sev:medium` / `sev:low`
  - **workaround** (created): `wa:clean` / `wa:partial` / `wa:none` — see
    [§ Workarounds](#workarounds--the-agents-can-you-keep-moving-signal)
  - area: `area:codegen` / `area:closures` / `area:store-lifetime` /
    `area:parser` / `area:native` / `area:wasm` / `area:stdlib` /
    `area:packages` / …
  - cross-cutting: `both-backends`, `needs-design`
  - triage-state: `attention` (stuck after 2+ tries), `design` (blocked on a
    user decision), `by-design` (closed — intended, cites a `DESIGN_DECISIONS`
    `C##`)
  - lifecycle: `fixed-pending-merge` (fixed on the working branch, awaiting the
    merge to `main` — see [§ Issue lifecycle](#issue-lifecycle--what-each-state-means-read-this-before-picking-work))
- **Cross-repo** — a bug in repo A that blocks repo B → an Issue in A, referenced
  from a `blocked-by`-labelled tracking Issue in B (`jjstwerff/loft#247`).  The
  dogfood loop (moros / dryopea drive loft) lives on these links.
- **Roadmap** — a `gh` Project board across the orgs for "which release bundles
  which consumer-driven work"; ROADMAP.md can't span orgs.

## Workarounds — the agent's "can you keep moving?" signal

A bug's workaround is the **primary thing the loft agent communicates to others**
(moros / dryopea / library authors / users): *can you route around this, or are
you blocked?*  Every bug carries both halves:

- a **`### Workaround` section** in the body — what to do today, OR what you tried
  that did NOT work;
- a **`wa:*` label** (the queryable metadata):
  - `wa:clean` — a simple idiomatic alternative, **verified working**;
  - `wa:partial` — a workaround exists but is awkward / loses the intended
    behaviour, **verified working**;
  - `wa:none` — nothing works; this **blocks** whoever hits it.

**THE RULE: a workaround is VERIFIED or it is not claimed.**  Run it on current
HEAD, both backends, and put the command + result in the section.  An **unverified
or WRONG workaround is worse than `wa:none`** — a downstream developer who follows
a confident-but-broken workaround loses time *and* stops trusting the signal,
which is the signal's entire value.  Same rigor as the probe-first fix rule: a
claim is a thing to *verify*, not assert.  (Pilots: #246 `wa:none` — the rebind
was tested, yields `[]`; #247 `wa:partial` — the non-capturing alternative was run
on both backends, printed `10`/`7`.)

**Triage:** `gh issue list --label "wa:none"` is the blocked-set — often more
urgent than raw `sev:`, because severity is "how bad when hit" and `wa:` is "can
you avoid being hit."

## Issue lifecycle — what each state means (read this before picking work)

**`open` means there is work to be done.**  The tracker's open set is the agent's
worklist; nothing closed or in the pending state is a pick-up candidate.

| State | Meaning | Is it a pick-up? |
|---|---|---|
| **open**, no blocking label | a real defect with work remaining | **yes** — investigate + fix |
| **open** + `design` / `needs-design` | open, but **blocked on a decision** — the next move is a design call (often the user's) | no — surface options, don't grind a fix |
| **open** + `fixed-pending-merge` | **done** on the working branch; the only step left is the merge to `main` (not the agent's action) | **no** — it's finished, just in transit |
| **closed** | terminally resolved: `by-design` / `duplicate` / `wontfix` (a non-fix outcome, no merge pending), **or** the fix reached `main` (auto-closed by `Fixes #NNN`) | no |

So the agent's actionable worklist = **open AND NOT `fixed-pending-merge` AND NOT
`design`/`needs-design`** (the last two need a decision first).

**Why `fixed-pending-merge` instead of closing on the working branch.**  `main` is
the release branch; a fix that lives only on a long-lived working branch is **not
in `main`**.  Manually closing it would make the tracker say "fixed" while the
released code still has the bug — and the eventual merge's `Fixes #NNN` would then
close-an-already-closed issue, the **close ↔ reopen ping-pong** we explicitly
avoid.  The pending label keeps the issue **honestly open** (work is *not* awaiting
the agent) until the merge auto-closes it in one clean transition.

## Resolving an issue (the close half)

Filing is half the loop; closing is the other half.

- **Reference the issue in the fixing commit** — `Fixes #NNN` / `Closes #NNN` in
  the commit (or PR body); GitHub auto-closes it when that lands on the default
  branch.
- **On a working branch with no PR, do NOT close manually.**  After pushing, add
  the **`fixed-pending-merge`** label (and a comment naming the fixing commit +
  the regression test).  The `Fixes #NNN` line closes it when the branch merges to
  `main` — one transition, no ping-pong.  Manual `gh issue close` is reserved for
  **terminal non-fix outcomes** (`by-design` → cite a `DESIGN_DECISIONS.md` `C##`;
  `duplicate` → cite the canonical issue; `wontfix`).
- **A fix needs a regression** — link the `tests/scripts/NNN` / `tests/*.rs` that
  locks it in.  A `fixed-pending-merge` issue with no regression is a re-opening
  waiting to happen.
- **Re-verify the workaround on resolve** if the issue had one — a fix can make a
  `wa:partial`/`wa:none` moot; keep the record accurate.
- **Don't file a bug you fix in the same change** — the fix + its test ARE the
  record (CLAUDE.md § Bug-filing policy).

## Item lifecycle — the `status:` axis (bugs + enhancements)

Every item carries a `status:*` label = **what it is waiting on**, on one shared
axis.  Bugs take the **short** path; enhancements take the **full** path — because a
*want* must clear value-vs-cost before it is committed, where a *fault* is committed
to by default.  (An enhancement is a small plan; the outcomes below are the plan
outcomes at issue scale — see *Beyond bugs — the unified model* below.)

**Intake gate (well-formedness).**  An item is not actionable until its body makes
the gap explicit — a **bug**: *expected vs observed* (+ a reproducer); an
**enhancement**: *what works now* vs *what you want from us*.  Until then it sits at
**`status:unclear`** (blocked on **information**).

**Intermediate (open):**

| `status:` | waiting on | bug | enhancement |
|---|---|---|---|
| `unclear` | information | ✓ | ✓ |
| `need-approval` | a decision — needs the **cost** beside the value: effort, systems changed, risks | — *(a fault is fixed by default; `needs-design` / `attention` cover the rare fix that needs a design call)* | ✓ |
| `designed` | doing it (approved + design settled) | — *(a bug's design is the inline investigation)* | ✓ |

**Resolution:**

| outcome | bug | enhancement | closes? · register |
|---|---|---|---|
| implemented / fixed | fixed | implemented | yes, on merge · `fixed-pending-merge` interim |
| **deferred** | — *(edge: a parked low-sev bug)* | ✓ — **stays open**, parked, un-defer trigger | no · `status:deferred` + DEFERRED.md |
| declined | `wontfix` / `by-design` | `rejected` | yes · rejected → DESIGN_DECISIONS.md |

A **bug has 2 terminals** (fixed / declined); an **enhancement 3** (implemented /
deferred / rejected) — the extra `deferred` is the *want* that can wait.  An
enhancement **links out to a `@PLN` / `plans/<NN>/` lazily** — only when the design
grows phase-worthy (usually at `status:designed`); no renumber, the issue stays the
lightweight capture.  Feature requests use the `feature_request` template; the plan /
ROADMAP carries the design + sequencing.

## Beyond bugs — the unified model (plans · lib-plans · enhancements)

**Initial design (2026-06) — draft.**  Bugs were the pilot; the same split
generalises to **every** work item, with one addition: **Subject** (the
consumer/deliverable) as the primary axis — the multi-project answer a per-repo
`ROADMAP.md` structurally can't give.

### As built (2026-06) — where the plans actually live

The rest of this section is the *design rationale*; here is the **current reality**
(it supersedes the draft where they differ):

- **Plans live as Issues in [`loft-lang/plans`](https://github.com/loft-lang/plans)** —
  the central, public, cross-ecosystem overview (the generic *org* home, not
  `jjstwerff/loft`).  Distributing plans across individual product repos would lose
  the overview; centralising the tracking keeps it built-in.  The labelled issue
  list **is** the overview today.
- **`@PLN<N>` is the canonical plan identity** = that repo's issue number
  (`@PLN3` → `loft-lang/plans/issues/3`).  Simple, globally unique, scales to every
  subject — the tag *is* the number.
- **`@PLAN<NN>` is now only a legacy *dir-pointer*** ("design lives in
  `plans/<NN>/`"), carried in the issue body; per-tree and harmless because the
  `@PLN` number is the real key.
- **Dimensions ride on labels** for now — `subject:*` (the primary cut) +
  `status:*` (lifecycle).  A gh **Project board** (the richer field schema below)
  is a later browser add-on; the labelled list already gives the overview.
- **Design stays in each code repo's `plans/` dir**; the loft-lang issue links to it.

### Why a clean view at all — the resonance test

The plans view is **not a management board.**  Its job is to be a *resonance
surface*: when someone who **shares the sensibility** sees the list, they grasp
**what kind of project this is — at a glance.**  It is the builder's counterpart to
a game playable in the browser — legible-on-contact for the right people, never a
funnel.  (The project is built for its own sake on a long horizon — see
[GOALS.md § Goal F "Grounding"](GOALS.md); adoption is a *consequence*, not a goal.
This view exists so the people who *would* resonate **can**, not to convert anyone
who wouldn't.)

**So the success test is character-legibility, not work-organization:** the board
succeeds when a kindred mind reads it and feels *"that's my kind of project"* — not
when "everything is tracked."  Every field and view decision is measured against
that, and read back through this lens the choices below already serve it.

The list must transmit three things at a glance, and **only** these:
- **coherence** — one idea, many expressions, visibly serving a single vision →
  this is why **Subject is the primary axis**;
- **depth / ambition** — serious, long-horizon, the hard plumbing genuinely being
  done → the **Value / Effort / Driven-by** framing carries it;
- **taste** — what is valued, and that it is valued for real → the **"why"** on each
  item, and the narrative.

Three things kill it — each already the reason for a design choice:
- a **ticket-dump** buries the soul under mechanics → keep it clean; *per-phase
  status + numeric priority stay off the board*;
- a **grab-bag** hides the single idea → *Subject-first + the dependency DAG keep
  the coherence visible*;
- **mechanics-without-why** shows no sensibility → *Value / Driven-by / the "why"
  are first-class, not optional*.

So **"clean" is not aesthetics — it is the medium.**  Clutter isn't just noise; it
is the project's character made *illegible*.

### The split, generalised

| Item type | Design home (files) | State home (gh Project) |
|---|---|---|
| **bug** | repro + probes + the regression test | the Issue + its fields |
| **plan** | grows with maturity: a `PLANNING.md` section (backlog) → a `plans/<NN>/` (loft) or `lib_plans/<NN>/` (libs) directory with phases (active) — see [`_PLAN_TEMPLATE`](plans/_PLAN_TEMPLATE.md) | a Project item |

Design in files; **state + sequencing + dependencies in one cross-org gh
Project**.  loft + libs plans are the **blueprint exemplars**; other subjects
replicate the `_PLAN_TEMPLATE` shape in their own repos.  Thin subjects (moros,
the demos) stay **unpadded** — a board that invents placeholder plans for an empty
subject is the same drift in a new costume.

#### Two kinds of item, not four — the PEP lesson

PEP (Python's enhancement-proposal process) already settled this: what PEP calls a
*proposal* is **the unit of intentional change**, and a one-paragraph one and a
multi-phase one are the *same kind* — size only changes length.  loft's "plan" vs
"enhancement" was never a real type split; it was **size + maturity wearing two
hats** (ROADMAP's own "features needing *plan promotion*" is a maturity transition
*within* one kind, not a conversion between kinds).  loft's word for it is **plan**
(matching `@PLAN` / `plans/` / `_PLAN_TEMPLATE`), so the Type axis is just **`bug`
vs `plan`**:

- a **bug** is a *fault report* (reactive — reality diverged from the spec);
- a **plan** is *intentional change* (proactive), whose **design home grows with
  maturity** — a `PLANNING.md` section while it's a backlog sketch, a `plans/<NN>/`
  directory with phases once active.  A "plan" is thus any maturity; Status tells
  you which.

Three things follow, the way PEP does them:

- **One number space (lazily).**  A plan carries one identity for life.  We do
  *not* renumber dormant catalog items: a backlog plan keeps its lightweight
  `PLANNING.md` ID; **on activation it earns an `@PLAN<NN>` number + a directory.**
  Promotion stops being a rename — it's just "the same item grew a home."
- **Identity on the board — the `@P###` trick, reused.**  Putting a plan on the
  board does **not** renumber it or rewrite a single reference.  Exactly as the bug
  migration did with `@P###`, the gh Issue's **title embeds `@PLAN<NN>`**
  (`[loft] @PLAN48 integer-width discipline`), so `gh search issues "@PLAN48"`
  resolves to the Issue while the `@PLAN` index keeps resolving doc references to
  the **dir**.  The gh number `#N` is plumbing (`Fixes #N`, closing) — **`@PLAN<NN>`
  stays primary**, the same way `@P###` stays primary for a migrated bug.  Because
  the `@PLAN` token is flatten-proof (the index re-resolves wherever the dir sits),
  the directory **need not move** and no `plans/*` path or `@PLAN` ref is touched;
  the `future/`/`deferred/` subdir becomes a vestigial hint with the **board
  authoritative for state**, and any `plans/*` *path* links flatten lazily, on
  touch, or never.
- **Status carries the terminal outcome**, because loft files each outcome
  *differently*: **shipped** (closure-record in the dir) · **declined** →
  `DESIGN_DECISIONS.md` (loft's closed-by-decision register *is* a declined-plan
  archive) · **superseded** (link to the replacement) · **deferred / withdrawn**.  A
  flat "done" would erase the distinction the routing depends on.
- **No Standards/Process/Informational "Kind" axis.**  PEP needs it; loft doesn't —
  it already routes Process (DEVELOPMENT.md) and Informational (DESIGN_DECISIONS.md,
  the reference docs) *out* of `plans/` by filing location.  Importing the axis
  would be cosplay.

What loft does **not** take from PEP: the Draft→Accepted acceptance ceremony (that
exists for a multi-stakeholder council; solo+agent needs only `backlog → active →
shipped/declined`).

**Investigation is not a board type.**  An investigation is a *file-level flavour*
of a plan — it additionally follows
[`_INVESTIGATION_TEMPLATE`](plans/_INVESTIGATION_TEMPLATE.md) for its clusters /
probes.  gh sees `Type: plan`; the "(investigation)" parenthetical lives in the
title + the README header.  Its specialness — it *produces* bugs — surfaces as
**outgoing links** to the Issues it spawned, which is data, not a type.  (Same for
"validation" / "feature" sub-kinds: title parenthetical + the Area field slice
them; none earns a structured board value.)

### What's good to track — the field schema

**Track a field iff** it is (1) **state** or a **relationship** (changes over
time, or links items), (2) something you **triage / sequence / group** by, (3)
**not derivable** from the file or another field, and (4) meaningful **across** the
board.  Design content, write-once-never-filtered facts, and second copies of what
the file already holds stay in the file.

| Field | Shape | Applies to | Why it earns a slot |
|---|---|---|---|
| **Type** | select · bug / plan | all | picks the lifecycle + the design home (a plan's home grows `PLANNING.md` → directory with maturity) |
| **Subject** | select · loft / libs / moros / dryopea / bumper-plane / audience / lavition / … | all | the **primary axis** — which consumer/deliverable |
| **Status** | select · backlog / next / active / deferred / shipped / declined / superseded | all | the lifecycle **+ terminal outcome** (shipped · declined→`DESIGN_DECISIONS` · superseded) — the **single authority** replacing dir-location + the README tables |
| **Area** | select · codegen / closures / store-lifetime / parser / native / wasm / stdlib / packages / … | all | subsystem; **unifies with the bug `area:*` labels** — one taxonomy for "all closure work" |
| **Milestone** | select · 0.9.0 / 1.0.0 / 1.1+ / ‹game› | all | release bundling — the cross-repo "which release ships this" that drove the move |
| **Depends-on** | linked items / @refs | all | the dependency **DAG** — ROADMAP's hand-drawn chains made real; powers a *derived* "blocked" view |
| **Effort** | select · XS / S / M / MH / H | plan | sizing, for sequencing |
| **Value** | select · Correctness / Enabling / Polish / Quality | plan | *why it matters / what kind*; doubles as coarse priority (board **order** refines) |
| **Driven-by** | select · ‹subject› | plan | the **dogfood link** — which consumer demanded this language/lib work |
| **Trigger** | text | deferred items | what un-blocks / revives it (the concrete defer-trigger) |
| **Severity** | select · high / medium / low | bug | how bad when hit (the existing `sev:`) |
| **Workaround** | select · clean / partial / none | bug | the "can you keep moving?" signal (the existing `wa:`) |
| **Repo** | select · loft / loft-libs-* / moros / dryopea / lavition | all (optional) | cross-org ownership + click-through (often implied by Subject) |
| **Owner** | assignee | all (optional) | who's on it — low value solo, grows with contributors |

**Deliberately NOT on the board** (lives in files, or derived):

- **Per-phase status** → the plan README is the source of truth (ROADMAP already
  says "read the plan README directly").  Mirroring phases is a guaranteed-drift trap.
- **Numeric priority (P0–P3)** → use the board **order**; a number drifts against
  the order it is meant to encode.
- **"Blocked" boolean** → **derived** (Depends-on has an open item), not hand-set.
- **Design / probes / repro / dates** → the dir + git (gh stamps created/updated
  automatically — enough to power a "stale" view).

#### Driven-by — the field the dogfood loop earns

loft's whole development model is *build a consumer → harvest the language lesson →
fix the language*.  **Driven-by** makes that loop **queryable**: "show every loft
plan dryopea is waiting on", "if we cut bumper-plane, which language work loses its
justification", "what lessons did moros harvest into 1.0".  It is distinct from
Depends-on (a *blocking* edge) — Driven-by is a *motivation* edge: the consumer
that would notice if this work vanished.  Without it, the consumer→language
dependency the whole project runs on is invisible to the tracker.

#### Open calibration — Value granularity

The draft collapses ROADMAP's eight bands (**S/R/G/F/U/C/Q/N**) to four:
**Correctness** (S + R), **Enabling** (G + F), **Polish** (U + C), **Quality**
(Q); **N**iche becomes "low board order", not a band.  Four sorts cleanly and
still answers *must-fix / unlock / nicety / refactor*; the finer eight remains
available.  **Decision point — keep four or the eight.**

### Access model — transparent by default, mutation gated by role

**Read is open; write is role-gated.**  The default is **public** for everything
with public value; a repo, board, or item goes private *only* when there is little
in it for the public (early scratch, a throwaway prototype, an idea not yet worth
showing) — a **usefulness** gate, not a secrecy one.  Building the games in the
open *is* the adoption story (the consumer→language dogfood loop, visible — Goal B),
so a game in development stays public.

| Who | Read | Write |
|---|---|---|
| **Public** (no role) | all public repos · Issues · the board | **only** open an Issue + comment — no label / field / Status / Milestone edits, cannot change a plan |
| **Triage** role | + | + label / set fields / close / reopen, *without* code write — the lever for a trusted non-code helper |
| **Write / admin** | + | everything |

GitHub has **no per-field public-write** — access is all-or-nothing by role, and the
public has none.  So "not all fields publicly changeable" is the *default*: every
structured field (`sev:` / `area:` / `wa:`, Status, Milestone, all Project fields)
is maintainer-only without configuring anything.

- **Plans are doubly read-only to the public.**  A plan is a *maintainer-authored*
  Issue (titled `@PLAN<NN>`) + a directory: the public can **comment** on the Issue
  and **view** the dir; changing the dir is a **PR you merge**.  Viewed + commented,
  never changed.
- **One public board** for the public-value ecosystem (loft / libs / games /
  demos) — public **views** the roadmap; editing fields or order needs project
  write.  The rare low-public-value item stays **off** it (a private note/repo)
  until it earns a place — and mind that a public board's draft cards are
  world-visible, so don't park a not-ready-to-show idea there.

> **Transition note.** "PROBLEMS.md" / "P-issue row" references elsewhere in the
> docs (plans/README, DEVELOPMENT.md, …) are repointed to GitHub Issues as they're
> touched.  PROBLEMS.md is frozen to OPEN bugs — it's the closed/historical archive.

## Migration plan

| Step | Action | Status |
|---|---|---|
| 1 | **Pilot** — file @P396/@P397 as Issues (#247/#246), drop from PROBLEMS.md | ✅ done |
| 2 | Create the `sev:*` / `area:*` / `wa:*` / cross-cutting labels | ✅ done (17: sev:/area:/wa:/regression/flaky/blocked-by/hit-by:) |
| 2.5 | **`@GH###` indexed tracker** (the one remaining CODE task — `tools/indexer/src/scan.loft`): (a) add `@GH<n>` to the tag tokeniser + a deterministic issue URL in `scripts/idx`; (b) in `tag_is_valid`, ALSO accept a `@P<n>` that appears in PROBLEMS.md's **freeze-banner `@P→#` map** (so the 7 migrated tags resolve instead of reading as broken — fully offline, the banner IS the map); (c) optional `make index-gh` validation bolt-on (`gh issue list --json number,state`). | ✅ done (04576e74) — @GH<n> tokenised + indexed; migrated @P resolve via the freeze-banner map; `idx tag:@GH247`/`gh:247` print the URL; index_hygiene green |
| 3 | Migrate the OPEN PROBLEMS.md rows → Issues | ✅ done — @P391→#248, @P389→#249, @P384→#250, @P351→#251, @P340→#252 (+ pilots #246/#247) |
| 4 | Freeze PROBLEMS.md — closed/historical record; FIXED rows + `###` design entries stay | ✅ done — freeze header + `@P→#` map; 0 open rows left |
| 5 | Flip the meta-doc filing rule "→ PROBLEMS.md" → "→ GitHub Issue" | ✅ done — CLAUDE.md (bug-filing + doc index + reading-by-goal), `_INVESTIGATION_TEMPLATE § Closing`, `plans/README § workflows` |
| 6 | USER_FACING.md — **KEEP** (revised: it's a curated *release-note-worthy deferred-work* tracker — features + user-visible bugs/perf + a showcase track — NOT a bug mirror, so not redundant).  Mark downstream-visible gh Issues with the `user-facing` label; USER_FACING.md cross-links them. | ✅ `user-facing` label created; cross-link as items arise |
| 7 | Apply the template + labels in dryopea / lavition / `loft-libs-*` as each needs bug-filing | ☐ ongoing |

## Agent note

`gh issue list/view/create/search` is the bug-layer interface; `idx` + files keep
serving plans/investigations.  Repros stay in-repo (commits, `tests/scripts/`,
`probes/`), so fixing a bug still has its context local — the Issue is the
tracker, the repo holds the artifacts.  The grep-ability lost on open bugs is
small; the grep-ability kept on everything that matters for *fixing* is total.
