<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN102 arc D — the public bug-intake path (design + safe small steps)

> **Status: MVP LANDED (2026-07-16) — steps 1–4 built, step 5 deferred.** Arc D of the stability contract. The
> internal "fix, don't file" discipline works because the finder can fix — it *"does not survive
> contact with anyone who cannot fix; filing is a stranger's only available move"*
> ([STABILITY_ROADMAP.md](../../STABILITY_ROADMAP.md)). Arc D is the path that lets a stranger's
> report reach someone who can fix, and land the never-break promise. No dependencies on A/B/C
> ([README § Phase ordering](README.md)).

## What arc D is (and what already exists)

Loft's internal rule is **fix, don't file**: the person who finds a bug fixes it (repro warm, paths
loaded), so issues are only fix-in-flight records, never a backlog. That is *right* — but it is a
rule for people who **can** fix. A stranger cannot; for them, **filing is the only move**. Arc D adds
the intake path for strangers **without** weakening the internal discipline: a public report is the
*file* half of a `file → fix` pipeline, where the file is a stranger's and the fix stays internal
(fix-not-file). It does not reintroduce a backlog — it feeds the fix flow.

**Already built (do not rebuild):**

- **The template** — `.github/ISSUE_TEMPLATE/bug_report.yml`: minimal-repro **required**, expected,
  actual, version, mode. It already asks the stranger for the *one actionable thing* (a small repro)
  and does **not** demand `sev:`/`area:`/`wa:` labels — the maintainer adds those in triage.
- **The chooser** — `config.yml`: `blank_issues_enabled: false` (forces the guided form) + contact
  links (playground "try a repro first", docs, the label glossary).
- **The public doc** — `CONTRIBUTING.md § Reporting a bug`: points to the template, stresses the
  minimal repro, explains the `wa:`/`sev:` labels a browser will see.

So the *public-facing form* is done. What is missing is the **internal bridge** (how a public report
becomes a warm fix), the **promise** (never-break, stated at the intake), and **routing** (which repo).

## The one invariant (design-protocol step 1)

> **A stranger's bug report becomes a warm, reproducible fix-input that a maintainer who CAN fix picks
> up — the public intake FEEDS the internal fix-not-file discipline, it does not replace it — and the
> reporter learns the promise: their working program stays working, so a regression is a
> top-priority bug, not a managed limitation.**

## The failure paths (why each piece exists)

Enumerated first (design-protocol: the failure paths surface the requirements):

- **No repro → not actionable → the report sits → the stranger feels unheard.** Mitigation: the
  template already *requires* a minimal repro; the triage bridge's first job is to *minimise it to a
  both-backend repro* (the actionable core), because "the smaller the program, the faster the fix."
- **Reaches no one who can fix → sits.** Mitigation: the triage bridge routes it into fix-not-file
  (a maintainer picks it up warm, `Fixes #NNN`) — the intake is not a backlog, it is a queue with a
  standing consumer.
- **The reporter doesn't know the promise → files a regression as a timid "is this expected?" or
  doesn't file at all.** Mitigation: state the never-break promise at the intake — an upgrade that
  broke a working program is a **top-priority regression** (arc A), say so plainly and it is treated
  as a bug, never "wontfix / managed change."
- **Wrong repo → misrouted (a `web` library bug filed on `loft/loft`).** Mitigation: routing in the
  chooser + a one-line "which repo?" note (language → `loft/loft`; a library's behaviour → its
  `loft-libs-*` repo; a game → its repo).
- **Duplicate / already-fixed on a newer loft → triage noise.** Mitigation: the version field +
  the triage's "does it still reproduce on `main`?" check (the arc-B version axis makes "fixed in N"
  answerable) — closed with a pointer, not silence.
- **The report vanishes → the promise rings hollow.** Mitigation: an acknowledgement discipline — a
  public report always gets a triage response (a label + a reply), so the reporter knows it landed.

## The re-assertion count (step 2) — N = 1

- **One intake channel** (the template + chooser + CONTRIBUTING), already single-homed.
- **One triage bridge** — a single documented process (`ISSUE_TRACKING.md`) turning any public report
  into the fix-not-file flow. Not a per-report improvisation.

So there is nothing to spray: one form in, one bridge to the existing fix discipline.

## The safe small steps (the MVP)

Mostly process + docs (arc D is a discipline, not compiler code); each is small and independently
useful.

| # | Step | What lands | Verify |
|---|---|---|---|
| 1 | ✅ **The triage bridge — document how a public report enters fix-not-file.** *(LANDED — `ISSUE_TRACKING.md § The public intake bridge (arc D)`.)* In `ISSUE_TRACKING.md`, a section: a `public-report` arrives → **acknowledge + label** (`needs-triage`, and after reproduction the `sev:`/`area:`/`wa:`) → **reproduce + minimise** to a both-backend repro (the actionable core) → **fix-not-file** (a maintainer who can fix picks it up warm, regression test, `Fixes #NNN`) → close with the fix. The reconciliation stated explicitly: *fix-not-file governs those who CAN fix; a stranger's file is the complementary intake, not a violation.* | the section reads end-to-end; a maintainer can follow it for a real public report |
| 2 | ✅ **State the never-break promise at the intake.** *(LANDED — `CONTRIBUTING § Reporting a bug`, `bug_report.yml` intro, `SUPPORT.md`.)* Add to `CONTRIBUTING § Reporting a bug` + the template's intro: *"loft's promise — your working program keeps working. If an upgrade broke something that worked, that is a top-priority **regression**, not a managed change; say so and we treat it as a bug."* Link [COMPATIBILITY.md](../../COMPATIBILITY.md). | the promise is one click from "New issue"; a reporter of a regression is told it is a bug, not a limitation |
| 3 | ✅ **Repo-routing.** *(LANDED — `config.yml` chooser contact link + `CONTRIBUTING`/`SUPPORT.md`.)* Add to the chooser (`config.yml`) a routing note / contact links: language or compiler → `loft/loft`; a **library's** behaviour → its `loft-libs-*` repo; a **game** → its repo. A stranger who doesn't know the multi-repo layout is routed, not misfiled. | filing from `loft/loft` surfaces the "is this actually a library bug?" pointer before submit |
| 4 | ✅ **The acknowledgement discipline.** *(LANDED — `ISSUE_TRACKING.md` bridge step 1 + the `needs-triage` label in `.github/LABELS.md` + `SUPPORT.md`.)* In `ISSUE_TRACKING.md` (+ a `SUPPORT.md` GitHub reads): a public report always gets a triage response (a label + a reply) — it never vanishes. A regression is never closed `wontfix`; the never-break promise forbids it. The queryable enforcement is the **`needs-triage`** label: `gh issue list --label needs-triage` is the un-drained intake. | a `public-report` with no maintainer response for a stated window is itself a lapse (the standing consumer, not an SLA number) |
| 5 | ☐ **(Later, DEFERRED) `loft report` — a repro-bundle helper.** A CLI that captures the failing program + `--version` + backend + environment into a ready-to-paste report, so a stranger's report is reproducible by construction. Defer until the manual path proves the friction is real. | a bundled report reproduces on a maintainer's machine unchanged |

**Shape:** steps 1–4 are docs/process — the *bridge* (step 1) is the load-bearing one (it makes the
intake feed the fix discipline instead of becoming a backlog); step 2 lands the promise; steps 3–4
remove the two friction/trust leaks. Step 5 is a friction tool, deferred. The whole MVP is in this
repo's `.github/` + docs — no compiler change.

## Reconciliation with fix-not-file (the load-bearing claim, probed)

The obvious objection: *arc D reintroduces filing, which the internal discipline forbids.* Falsify it
— the discipline's actual statement is **"the person who CAN fix, fixes (does not file)"**, and its
rationale is *repro warm, no re-derivation to re-pay later*. A stranger has **no fix ability**, so
filing is not the discipline's forbidden move; it is the **only** move, and it is the *input* the fix
discipline consumes. The triage bridge (step 1) is exactly the seam: the stranger files (warm to
*them*), the maintainer reproduces + minimises (re-warming it *internally*), then fix-not-file runs
unchanged. So arc D does not weaken fix-not-file — it **extends its reach to bugs no insider found**,
which is precisely the scale boundary the roadmap names (*"the policy is right; its boundary is
scale, not size"*).

## See also

- [STABILITY_ROADMAP.md](../../STABILITY_ROADMAP.md) — the fix-not-file discipline + the scale
  boundary arc D addresses.
- [ISSUE_TRACKING.md](../../ISSUE_TRACKING.md) — the issue process this extends (the triage bridge
  lands here).
- [COMPATIBILITY.md](../../COMPATIBILITY.md) — the never-break promise the intake states (arc A).
- [README.md](README.md) — the arc table; arc D is independent of A/B/C.
- `CONTRIBUTING.md § Reporting a bug` · `.github/ISSUE_TEMPLATE/{bug_report.yml,config.yml}` — the
  already-built public-facing form.
