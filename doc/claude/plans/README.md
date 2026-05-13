# Plans

Multi-phase initiatives that span more than one session.  Each
subdirectory holds the README (goal + index) plus one markdown file
per phase.

## The rule — docs vs plans

**Documentation about how things work** (architecture, runtime
semantics, data structures, language reference, API surface)
lives at `doc/claude/*.md` — or, when scoped to a single
library, inside that library (e.g. `lib/<name>/README.md`).
This is the reference layer: anyone reading or modifying the
code reads these.

**Future work — things that need to be built** lives in
`plans/` (core language / compiler / runtime) or `lib_plans/`
(library work).  This is the actionable layer: anyone
planning the next session looks here.

These connect through the **roadmap**: every row in
[`../ROADMAP.md`](../ROADMAP.md) eventually points at a plan
in `plans/` or `lib_plans/` (loose features without a plan
home become the exception, not the rule).

**Bug fixes are the explicit exception** — they land directly
via PROBLEMS.md + a regression test + a focused commit, no
plan required.  The plan path is reserved for major
development that benefits from explicit phasing, multi-
session sequencing, or design-before-implementation
discipline.

When a `doc/claude/*.md` reference doc has open follow-up
work mixed into the architecture content, add an
`## Open work` section IN THAT REFERENCE DOC and link
ROADMAP rows directly at it (e.g.
[NATIVE.md § Open work](../NATIVE.md#open-work),
[QUALITY.md § Open work](../QUALITY.md#open-work--actionable-summary)).
Single source of truth, no indirection — pointer-plans were
tried (33/35/lib-11) and shown to be over-engineering.

## Three workflows for TODO items — pick the lightest that fits

| Flow | Where the TODO lives | When to use |
|---|---|---|
| **Bug fix** | [`../PROBLEMS.md`](../PROBLEMS.md) row + regression test + focused commit | Single root cause, fits in one commit, no design choices, no multi-phase sequencing. |
| **Light — `## Open work` section** | A `## Open work` section in the relevant `doc/claude/<NAME>.md` reference doc | The normal flow.  TODO is co-located with the architecture it touches; one row per item; closure is "remove the row + update the reference content."  Used by NATIVE.md, PERFORMANCE.md, PACKAGES.md, QUALITY.md today. |
| **Plan — `plans/<NN>-<slug>/`** | Full directory with README + per-phase files | Multi-session initiative with explicit phasing, design-before-implementation discipline, cross-arc dependencies, or a long arc that needs its own document space.  Capped at 2-3 active per `plans/` (see `feedback_max_three_active_plans`). |

The light flow is the default.  Promote to a plan only when
the work is genuinely multi-phase and benefits from its own
directory — most TODOs don't, even ones that take several
sessions.

## Roadmap workflow

[`../ROADMAP.md`](../ROADMAP.md) is the work-list view: every
open work item, grouped by value, with dependencies and
effort.  This section documents how it's organized — the
methodology lives here so ROADMAP itself stays tight.

### Value categories — what KIND of value, not just how much

ROADMAP rows are grouped by **value category**, not by release
milestone (0.8.5 / 0.8.6 / etc.).  Eight categories in default
reading order (read top to bottom; pick from the highest tier
that has open work):

| Tag | Meaning | Examples |
|---|---|---|
| **S** | **Silent failure / data-loss prevention** — features that "appear to work" but don't; corruptions that have no error message; data loss without indication.  HIGHEST priority because invisible to users and erodes trust most | Validation matrices (catch backend divergence), JSON-correctness sweeps, closure-DbRef leak, native-vs-interpreter parity gates |
| **R** | **Regression / release-blocker** — known broken behavior, PROBLEMS.md High-severity, gates the next tag | (none today; bugs land as P-issues, not plans — would surface here if a regression blocked a release) |
| **G** | **Goal-enabling** — directly enables loft's core use case: browser games anyone can play via shared link, multiplayer, native-game debugging | Server / game-client libraries, scriptable scenes, multiplayer protocol stack, web IDE, graphics library, native debug |
| **F** | **Foundation** — unblocks 2+ downstream plans (lattice point in the dependency graph) | Lazy stdlib, package registry, FFI generic marshaller, LSP server MVP, library extraction |
| **U** | **Ease of use** — first-time-user experience, daily ergonomics, IDE polish | Better error messages, REPL, syntax highlighting, VS Code extension, tutorial, LSP editing surface, developer warnings |
| **C** | **Clean features** — language correctness, removes special cases | Mutable closures (cleaner capture), match-PEG (cleaner pattern syntax), sorted slicing (removes special-case), JsonValue (replaces text-based JSON), error recovery, factory methods |
| **Q** | **Internal quality** — performance, refactor, internal cleanup with clear payoff | Performance follow-ups, native codegen follow-ups, retire-scratch, const-store completion |
| **N** | **Niche / opportunistic** — small specific features, low-priority items | Route decorators, asset pipeline, HTTP client, tic-tac-toe (protocol-only validator), AOT auto-compile |

**Why S is its own category, separate from R:**

A regression (R) is a **known broken** behavior — we have a
test failure, a panic, or a wrong output we can document and
gate releases on.  A silent failure (S) is **invisibly broken**
— the program runs to completion, returns "successfully," but
the result is wrong / data is lost / state is corrupted.

S is more urgent than R because:
- Users hit S without warning (no error message to file)
- Data loss without indication is the worst class of bug
- "Appears to work" hides decay in the codebase
- Trust erodes faster from one silent corruption than from a
  loud crash

The validation matrices (plans 14 / 15 / 16 / 18 / 19 / 20)
are precisely S-prevention work: every {input × backend × API
shape} cell is exercised end-to-end with byte-identical
output assertions.  They're cataloged here as S, not C, because
their value is preventing silent divergence.

**Why named categories instead of V1/V2/V3:**

The previous V1/V2/V3 collapsed two distinct dimensions —
"importance" and "what kind of work" — into one ranking.
Named categories separate them: a plan in **G** (goal-enabling)
ranks above a plan in **U** (ease-of-use) because of WHAT the
work delivers, not just because someone declared it more
important.  Re-ranking happens when categories change (rare),
not when calendar perception shifts.

**Why this works better than time-based grouping:** value
categories stay stable across the project's life.  A goal-
enabling item is goal-enabling whether it ships this month
or next year.  Milestone groupings (0.8.5, 0.8.6, ...) imply
calendar timelines that constantly drift; named categories
don't need that maintenance.

**Effort estimates, not time projections.**  ROADMAP has an
`E` column (XS / S / M / MH / H / VH / L) calibrated to
relative work, not calendar time.  Time projections are
inaccurate at both ends — items projected as "weeks" have
shipped in days; items projected as "quick" have taken
weeks.  Effort buckets are stable; calendar projections
aren't, so we don't make them.

### Maintenance rule

When an item completes, **remove it from ROADMAP**.  Completed
work belongs in CHANGELOG.md (user-facing notes) +
CHANGELOG_TECHNICAL.md (contributor detail) + git history.
Keeping closed rows in ROADMAP turns the work list into a
log; we already have logs.

### Features need plans

Every feature row in ROADMAP should cite a plan in its `Source`
column — or be small enough for direct PROBLEMS.md + commit
(the bug-fix path).

The plan-cadence rule: any FEATURE (multi-step, has design
choices, touches multiple files) goes through the plan
path even if small.  Small bug fixes, deliverables (demo
deploys, single-action items), and operational changes
(CI tweaks, doc fixes) can stay on ROADMAP without plans
or land directly with no ROADMAP row at all.

When a feature row still cites a flat reference doc
(PLANNING.md, INTERFACES.md) rather than a plan, that's a
**plan promotion candidate**.  Promote it to a plan when it
surfaces as next-up work.

### How to read the roadmap

The work-table sections (V1 / V2 / V3) are the **what to
do next**.  Within each value tier, items are loosely
grouped by theme — pick whichever theme is closest to what
you're touching.

The "All open plans — index by value" section is the
**comprehensive view across both trackers** (`plans/` +
`lib_plans/` + their deferred subdirs).  Single place to
read for "what's open, what depends on what, how valuable."

The "Cross-tracker dependency chains" subsection captures
the cross-plan arrows: which plan unblocks which.  Useful
for picking next-up work.

The "Features still needing plan promotion" subsection
tracks ROADMAP rows that should have plans but don't yet —
the next-up promotion candidates.

### Closing a plan — documentation must move out

When a plan ships and moves to `finished/`, its
**documentation content must move to its proper home** in
the reference layer:

- Library-scoped reference content → `lib/<name>/README.md`
  (or other library-internal docs).
- Project-wide reference content → `doc/claude/*.md`
  (architecture, runtime semantics, language / API surface).

**The `finished/<NN>-<slug>/` directory is for closure
record only** — git history pointer, commit chain, P-issues
filed/closed, lessons learned.  It is NOT a place that other
docs link to for ongoing reference.

**Why this matters:** retaining links to `finished/`
plans across the docs creates drift.  A future contributor
clicking through to a closed plan finds a closure record
where they expected design content; the actual design has
moved elsewhere or the design has been superseded by the
implementation itself.  Links to closed plans rot fastest
because nothing in the project keeps them honest.

**How to apply on close:** see
[`_LIFECYCLE.md`](_LIFECYCLE.md) for the 6-step procedure shared
by close + defer.  Two shapes: **CREATE-AND-MOVE**
(`31-html-export → HTML_EXPORT.md`) and **TRIM-ONLY**
(`04-slot-assignment-redesign → SLOTS.md`).

This rule applies retroactively: a `finished/` plan being linked
from current docs is a cleanup signal — either the reference
content never moved out, or the link wasn't updated when it did.
`scripts/check_doc_drift.sh` catches the common shapes
automatically.

### Authoring a new plan

Copy [`_TEMPLATE.md`](_TEMPLATE.md) to
`<NN>-<slug>/README.md` (next free integer in the relevant
tracker).  The template captures the canonical shape:
Status / Goal / Effort / Sub-arcs / Phase ordering / Open
questions / Cross-arc dependencies / See also.  Length
budget: 100-300 lines; longer plans usually have reference
content that should move to `doc/claude/*.md`.

### Deferring a plan

When some/all phases can't reach completion in the current arc
but the remaining work has a **concrete trigger**, move to
`deferred/`.  Procedure in [`_LIFECYCLE.md`](_LIFECYCLE.md) —
shared with closing (Steps 4-6 are identical).

Partial defers (some phases shipped, others paused) grow a
SHIPPED / DEFERRED Status table at the top of the plan README.
Canonical shapes: @PLAN28, @PLAN12.

If remaining phases have no concrete trigger, the design moves to
[`../DESIGN_DECISIONS.md`](../DESIGN_DECISIONS.md), not
`deferred/`.  "Will get to it later" is not a trigger.

### Light flow lifecycle — `## Open work` in reference docs

The light flow follows the same lifecycle as plans, just with a
single row instead of a directory.

**Open** — add a row to the relevant reference doc's
`## Open work` table (create the section if it doesn't exist;
see NATIVE.md / PERFORMANCE.md / PACKAGES.md / QUALITY.md for
canonical shape).  Add a ROADMAP row tagged with value category
+ link directly at the section.

**Work** — when implementing, edit the reference doc's
architecture content directly (it's the same file).  Cross-link
between the Open work row and the section it touches if useful.

**Close** — when shipped:
- Remove the row from `## Open work`.
- Update the surrounding architecture content to reflect the new
  reality (the same edit that closed the work usually does this).
- Remove the ROADMAP row.
- Closure record lives in the commit message + CHANGELOG_TECHNICAL.md;
  no separate closure-record file.

**Defer** — if work is paused with a concrete trigger:
- Annotate the row inline (e.g. `**Blocked on X** — unpauses
  when Y happens`) or move the trigger detail to
  [`DEFERRED.md`](DEFERRED.md) if it's cross-cutting.
- Keep the row in `## Open work` (it's still tracked work, just
  paused).

**Promote to plan** — if the row grows into a multi-phase
initiative, copy `_TEMPLATE.md` and migrate.  Don't promote
prematurely; multi-row clusters often stay light if each row is
independent.

The recent collapses of pointer-plans 33 / 34 / 35 / lib-11 into
NATIVE.md / PERFORMANCE.md / QUALITY.md / PACKAGES.md `## Open
work` sections are the canonical examples.

## Companion indexes — every parked item is discoverable

Two files complement this README; together they ensure deferred
work is never silently dropped.

- **[`DEFERRED.md`](DEFERRED.md)** — trigger index for the
  `plans/deferred/` plans only.  Each row gives the concrete signal
  that should re-activate the plan.  Future plans (`plans/future/`)
  live on [`../ROADMAP.md`](../ROADMAP.md), not here — they're
  paused with intent to finish, not awaiting a trigger.
- **[`../USER_FACING.md`](../USER_FACING.md)** — the subset of
  DEFERRED.md that downstream users would notice if shipped, with
  release-note language, workarounds, and severity tiers
  (S0 / S1 / S2 / S3).  S0 items are release-blocking; S1 items
  must ship within two releases of being filed.

**Pre-release ritual:**

```bash
# 1. Every parked test (lock-in regression net):
cargo test --release -- --ignored 2>&1 | grep "^test " | head -50
#    Items now passing → un-ignore + add release note.

# 2. Every parked doc trigger:
grep -r "Trigger to unpause:" doc/claude/
#    Walk the list, refresh `Last reviewed:` lines.

# 3. USER_FACING.md status pass:
#    Every open row gets shipped / still-deferred / dropped tag.
```

### Closed-work hygiene rule

DEFERRED.md and USER_FACING.md are **open-queue documents**.
When an item closes, its row is **removed entirely** — closed
work lives in the right place for its shape: git history (commit
message), regression tests, plan READMEs (architectural lesson),
PROBLEMS.md (closed P-id entries stay for cross-reference),
CHANGELOG.md (user-facing notes).  Five homes, each correct for
its information shape.

The grep target `grep -r "Trigger to unpause:" doc/claude/` should
always show only currently-actionable items.

**Sole exception**: USER_FACING.md's "Closed-by-decision" section
is a permanent record of explicit non-goals so future contributors
find the decision before re-proposing.

### Validation-first default

Default discipline: finish validation plans before shipping new
feature work.  When validation matrices stop yielding bugs (a
phase closes with 0-1 P-issues across 5+ cells), the matrix is
mature — promote that bandwidth to feature / showcase work.
Override only when USER_FACING.md surfaces an S0 item or an S1
item that's been deferred for two releases.

## Conventions

- Subdirectory names are numbered (`NN-slug`); number is monotonic
  open-order, not priority.
- New initiative opens with `NN-slug/README.md` (from `_TEMPLATE.md`)
  + `00-<first-phase>.md`.
- Phase files begin with `Status: open | in-progress | done`.
- Closed plans → `finished/` (apply [`_LIFECYCLE.md`](_LIFECYCLE.md)).
- Paused-with-trigger plans → `deferred/` (apply [`_LIFECYCLE.md`](_LIFECYCLE.md)
  + add row to [`DEFERRED.md`](DEFERRED.md)).

## Ground rule — plans never allow regressions

**A plan's job is to split work into manageable chunks that can
each land cleanly without introducing new problems.**  That is the
entire point of a plan vs. an ad-hoc fix.  Every phase, and every
step within a phase, must:

- Preserve every currently-green test across the full suite.
- Preserve every currently-correct user-facing behaviour.
- Either ship a new invariant or be a no-op refactor — never a
  degrade-now-fix-later bargain.

When a step surfaces a scope surprise (e.g. a prerequisite was
wrong, a shared code path breaks under the new invariant, a
previously-undocumented consumer exists), the plan document is
updated BEFORE the next commit lands.  The chunks may shrink, a
new sub-phase may be added, or the initiative may pause until the
surprise is understood — **but no regression ships as "we'll fix
it in the next phase"**.

Single-commit fixes outside a plan may exceptionally trade a
regression for a critical fix (documented explicitly in the commit
message).  Plans never — their entire raison d'être is the
discipline of no-regression progress.

Corollary: when a plan's acceptance criteria lists a condition like
"full test suite green" before proceeding, that condition is
binding.  A step that violates it gets reverted (not amended) and
the plan is re-scoped.  The 2026-04-21 P184 Phase 0 attempt (bulk
4-tuple extension, then reverted when test failures surfaced) is
the canonical example of this discipline in action.

## Ground rule — file pre-existing bugs surfaced during a phase

A plan phase fixing one bug or implementing one cell routinely
surfaces *other* bugs while probing variants, reading code, or
comparing backends — sibling shapes, latent issues flagged in
comments, symptoms unrelated to the active fix.

**File those P-issues before the phase closes, not later.**  See
[CLAUDE.md § Bug-filing policy](../../CLAUDE.md#bug-filing-policy--mandatory)
and [DEVELOPMENT.md § Bug-filing During a Hunt](../DEVELOPMENT.md#bug-filing-during-a-hunt--mandatory)
for the full policy.  Plans-specific notes:

- The phase's commit message lists every P-id filed and every
  P-id closed in this phase.
- New P-issue rows in [PROBLEMS.md](../PROBLEMS.md) are part of
  the phase's deliverable, not a follow-up TODO.
- Follow-ups belong to their own future phase or session — do not
  scope-creep the active fix to "while I'm here, also fix X".  One
  fix per commit; one follow-up per row.

The May 2026 @P211 hunt is the canonical example: the original
P-issue was native `yield text`, but the diagnostic probes
surfaced @P217 (text accumulator), @P218 (format-with-param in
generator body), @P219 (vector-for-yield), @P220 (`""` in
`vector<text>`) and @P221 (server-side BufReader).  All five were
filed in the same commit window; none were lost.  The @P217
follow-up hunt then surfaced @P222 / @P223 (narrower self-concat
shapes) — same rule applied.

### Per-plan status lives in the plan README — not on ROADMAP

ROADMAP's "All open plans — index by category" tables carry only
the stable parts of each plan: name, remaining effort (E),
dependencies, and a one-line "what is this plan about" descriptor.
Per-phase status (what's shipped, what's in flight, what's
blocked) lives in the plan README's Status block — that's the
single source of truth.

Why: per-phase status changes every time a phase ships or is
deferred.  Mirroring it on ROADMAP doubled the edit cost and
created a recurring drift surface (manual audit 2026-05-09 caught
@PLAN14 phase 08 deferral missing, @PLAN07 phase 1 status stale).
ROADMAP's job is "which plans exist + how big + what blocks them";
the plan README's job is "where it stands today."

## Where to look for plans by state

The filesystem is the source of truth — duplicate per-state tables
in this README rotted faster than they helped.  Use:

```bash
ls doc/claude/plans/[0-9]*/             # current (max 2-3)
ls doc/claude/plans/future/             # paused, intent to finish
ls doc/claude/plans/deferred/           # paused, awaits trigger (see DEFERRED.md)
ls doc/claude/plans/finished/           # closed (closure records only)
ls doc/claude/lib_plans/{future,deferred,finished}/   # library plans
```

Each plan directory contains a `README.md` with Status block, Goal,
and per-phase index — that's the per-plan source of truth.

For a work-list view across plans, see `../ROADMAP.md`: every active
plan has rows there tagged with value category + dependencies.
Deferred plans live in `DEFERRED.md` (trigger index).  Finished
plans have a one-line closure note in their README pointing at the
reference home.

Run `scripts/check_doc_drift.sh roadmap` to verify every active plan
is on ROADMAP at the right path and no finished/deferred plan has
crept into ROADMAP as an action item.

## One-off plans elsewhere

Per-session ephemeral plans not tied to a multi-phase initiative
live under `~/.claude/plans/` (flat, generated filenames).  Those
are not committed to the repo.
