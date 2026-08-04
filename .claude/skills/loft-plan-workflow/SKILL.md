---
name: loft-plan-workflow
description: Procedures for plan-shaped work — opening a plan, closing/deferring one, promoting a doc to a plan, choosing the lightest workflow, applying value categories. Apply when the task involves plans, plan directories, a roadmap, or doc-to-plan promotion. The method is tree-agnostic; the concrete paths/templates/tracker that bind it to this repo live in one § at the bottom. Cites source docs for definitions; does not restate.
user-invocable: false
---

# Plan workflow

## What this skill is — and what's portable

The methodology below is **tree-agnostic**: the way you decide *whether* work is a
plan, open it, run an investigation, and close it is identical in any project that
organizes multi-phase work as plans.  The concrete paths, templates, tracker, and
naming that bind it to *this* repository live in **one place** — [§ Bind to your
tree](#bind-to-your-tree) at the bottom.  Carrying this skill into another tree
means keeping the body unchanged and repointing that one section.

This skill is **procedural-only**.  Definitions, rationale, and templates live in
the source docs the bindings point at — not restated here.

**Cross-cuts:** branch / commit / push policy and bug-filing policy live in the
repo's master instructions (loft: `CLAUDE.md`).  This skill assumes you've already
followed those.

---

## Pick the lightest workflow that fits

The lightest path that holds the work is the right one.  Promote up a tier only
when the work genuinely needs it — most TODOs never become plans, even ones that
span several sessions.

| Work shape | Path |
|---|---|
| **Bug fix** (single root cause, fits one commit) | Fix + regression test + commit.  Reference the tracker issue if one exists; otherwise the fix + test *are* the record.  No plan, no open-work row, no archive entry. |
| **Tiny deliverable** (demo deploy, version bump) | One overview/roadmap row, or nothing.  No plan. |
| **Operational change** (CI tweak, doc fix) | Direct commit. |
| **Light TODO** *(the default)* — fits in one row of a reference doc's table | An `## Open work` section in the relevant reference doc.  Same lifecycle as a plan, just one row.  Edit that doc's architecture content directly when you implement; the row and the design share a file. |
| **Plan** — multi-phase, explicit phasing, design-before-build, cross-arc deps, or its own document space | A full plan directory.  Cap active plans at 2–3. |

The light flow is the default.  A plan earns its directory only when the work is
genuinely multi-phase **and** benefits from its own document space.

---

## A plan's identity is its tracker issue

One model, stated once:

- **Identity = the issue number** in the shared plan tracker — *not* a local
  directory integer.  **File (or find) the issue FIRST, then name the directory after
  the number the tracker returns.**  Never pick the number by scanning existing plan
  directories: a sibling *branch* may already hold an unmerged `<N>-<slug>/` for that
  number (e.g. a `15-debugger/` on another branch while `main` shows only up to 14), so
  scanning mints a duplicate that collides the moment that branch merges — and the
  issue number, once GitHub assigns it, is immutable, so the collision is expensive to
  unwind.  The tracker's auto-incremented issue number is the one global, collision-free
  source of truth; the local directory tree is not.
- **The directory is flat**: `<id>-<slug>/`.  There are **no
  `future/`/`finished/`/`deferred/` subdirectories** — lifecycle **state lives on
  the issue** (a label), not in the path.
- **There is no hand-maintained roadmap table.**  The overview is *derived* from
  the tracker.  Don't curate a parallel roadmap of rows.

Everything below assumes this model.  A tree mid-migration will still have **legacy
plans** in the older layout — lifecycle subdirectories plus hand-maintained
overview rows.  Leave them where they are (or migrate opportunistically), but
author **new** plans in the model above; don't extend the legacy layout.  Your
tree's legacy specifics are in [§ Bind to your tree](#bind-to-your-tree).

---

## Opening a plan (standard shape)

Use when the deliverable is a feature ship or a fix landing.

1. **Identity — claim the issue FIRST.**  Create (or find) the tracker issue *before*
   the directory; the number it returns is the plan's id.  Do **not** derive the number
   from the local directory tree — that misses unmerged plans on sibling branches and
   mints colliding duplicates (see [§ A plan's identity](#a-plans-identity-is-its-tracker-issue)).
2. **Flat directory** `<id>-<slug>/` named for the returned issue number, README copied
   from the standard template.
3. **Fill Status + Goal first.**  Add Sub-arcs / Phase ordering / Open questions /
   Cross-arc dependencies / See-also as the design clarifies.  Link the source
   issue(s) and carry the plan id in the body.
4. **Label the issue** — subject + status (`future` planned · `active` in
   progress · `finished`).  (No `plan` type label: in a dedicated plans repo
   every issue is a plan, so it partitioned nothing — retired 2026-06-14.)
5. **No roadmap row, no per-plan entry in the master-instructions index** — the
   plan is discoverable via the tracker and the plans-overview doc.  Add a
   master-index entry *only* if the plan introduces a genuinely new top-level
   reference concept (vanishingly rare).

**Length budget: 100–300 lines.**  Longer means reference content is leaking into
the plan — extract it to its own reference doc.

**When NOT to use this shape:** if the plan's first phase is *characterize the
problem space* rather than *design + build*, use the investigation shape below.

---

## Opening an investigation-style plan

Use when the deliverable is **mechanism understanding before fix design** —
multiple failure clusters to catalogue, where source-reading alone won't converge
and the fix-design decision can't be made without the catalogue.

1. Identity + flat directory as above, README from the **investigation** template;
   add a `probes/` subdirectory.
2. **Stage A first — write probes before reading source.**  Probes are the
   executable spec for what "understood" means: a hypothesis is confirmed when the
   probe-pair diff confirms it.  Be **liberal** — missing a crucial shape is the
   worst failure; redundant variants cost nothing because they get attic-curated at
   the *end* of Stage A, not during.  Extract at least one probe from a *real
   consumer*, not only synthetic cases — real extraction catches classes synthetic
   probes miss.
3. **Run every probe on every execution mode** the result can diverge across
   (e.g. interpreter vs compiler).  Record the full results matrix.
4. Keep a **flat probe table** (one row per probe: file, shape, cluster, status).
   Curate into groups only when the suite is large *and* multiple people read it
   cold.
5. For each failure mode, write a cluster doc with a **verified-vs-hypothesized
   accountability table** — every mechanism statement is either VERIFIED (cited
   trace/code-line) or HYPOTHESIZED (marked).  Without this column, hypotheses
   drift into the prose as if they were facts.
6. **Track severity as two separate fields** — corruption/panic/hang *and* leak —
   so "FIXED" can't conflate them (closing corruption while leaks persist is a
   false-fix trap).
7. **Tools as needed, not upfront.**  Don't build a debugging framework first; add
   the *one* tool blocking progress, revert nice-to-haves, and list what you added
   in a `Tool gaps` section — tools added during the plan are part of its output.
8. Add a Status + next-session roadmap (per-cluster action items with effort
   estimates).

**Probe → regression migration.**  Probes stay in `probes/` during the
investigation and graduate to the regression suite **per cluster, as each cluster's
fix lands** — not all at once.  The plan stays open during phased implementation
and closes when the last cluster's regression is in the suite.  A probe is
graduation-ready only when it passes **all** of: assertions pass · clean process
exit (no crash at teardown — "PASSED prints" is not enough, check the exit code) ·
no leak warning · bounded runtime (seconds, not a hang).  A probe that passes
assertions but fails any other gate stays in `probes/` with a status note;
graduate a representative sibling from the same cluster instead.

---

## Closing or deferring a plan

**Close the moment the work is FINISHED — do NOT gate on merging to main.**  "Finished"
means design + build + tests + docs are all done (on the branch); at *that* moment the plan
closes.  A plan issue is a claim about the DESIGN being settled and delivered, not about the
commit having reached the trunk — the code lands on main later, on its own clock, usually
**batched** with other work.  So:

- The doc-side closing below (trim the README, move reference content, rewrite links, swap
  the label + close the issue) is **doc-only work that does not need the code on main** —
  do it when the work is finished.
- **Close plans in a batch, never a PR-per-plan.**  A ~30-min PR/CI cycle to close a single
  plan is waste; bundle the closing doc-changes (across several plans, even unrelated) into
  the next substantive push (`CLAUDE.md` § no-cycle-for-trivial-docs / bundle-subjects).
- Because the close is a **hand-close** (issue closed before/independent of a `Closes
  @PLN<n>` PR merge), you MUST swap the status label yourself — see step 4's hand-close note.

**Pick the outcome first:**

- All phases shipped → **close**.
- Some/all phases paused with a **concrete trigger** → **defer** (Status table
  grows SHIPPED / DEFERRED rows; the deferred phases keep their full design
  content).
- Paused with **no** concrete trigger → the design moves to the closed-by-decision
  register, not a deferred state.

**For each shipped phase:**

1. **Tag** sections REFERENCE / CLOSURE-RECORD / HISTORICAL.
2. **Move reference content OUT** to its home doc — either create-and-move (a phase
   that grew a whole subsystem → its own reference doc) or trim-only (content
   already has a home → just delete the duplicate).
3. **Trim the README** to a lead `Status — DONE/SHIPPED <date>` line + a cross-link
   to where the reference content now lives.

**Common to close + defer:**

4. Reclassify any overview rows (shipped parts leave; deferred parts stay if
   tracked).  Set the lifecycle **state on the issue** (not a directory move):
   closing → swap `status:active` for `status:finished` **and close the issue**;
   deferring → swap for `status:future`, issue stays open.  **Don't rely on
   GitHub's `Fixes #N` — it's same-repo only and can't reach the plans repo.**
   Instead the finishing PR carries a cross-repo close directive (`Closes
   @PLN<n>`); on merge to the trunk a close-on-merge workflow runs the repo's
   close-shipped-plans script to do the `status:finished` + close, with a
   stale-plan audit as the drift safety net.  The manual `gh issue` edit is the
   fallback / out-of-band path.
   - **A CLOSED plan's status label must be `status:finished` (delivered) or
     `status:declined` (de-scoped) — NEVER a live status (`active` / `future` /
     `next` / `closing`).**  The swap is automatic ONLY when the finishing PR used
     `Closes @PLN<n>`; a **hand-close** (the issue closed directly) or a PR that used
     **`Refs`** instead of `Closes` closes the issue but leaves the label stale, so
     you must swap it yourself: `gh issue edit <n> -R loft-lang/plans --remove-label
     status:<live> --add-label status:finished`.  This drifts silently — plans
     107-110 were all closed-but-mislabeled (`active`/`future`/`next`) from
     `Refs`-only or hand closes — so when you touch a closed plan, verify the label
     matches the state; don't trust the audit to have caught it.
5. **Grep + rewrite incoming links — THE most-skipped step.**  Reference content
   embedded in a finished plan gets linked to from other docs; those links rot when
   the content moves.  Grep every doc for the plan's path, rewrite the links to the
   new home, then run the repo's drift checker to catch what grep missed.
6. **Check the feature catalogue against what actually shipped** — see the next
   section.  A plan that shipped, changed, renamed or removed a feature is not closed
   until that feature's own issue and labels describe the built thing.

---

## Check the catalogue against what shipped

Where a tracker issue is the **canonical** description of a feature — and the
reference docs and example tests are GENERATED from it — shipping the code is only
half the change.  **The issue *is* the documentation.**  Code that lands without its
issue being updated publishes a wrong description, and the generated shadow gives
that wrong description the authority of a checked-in file.

Do this whenever work **adds, changes, renames, or removes a feature** — not only at
close.  The likeliest thing to drift is a feature whose behaviour moved *during* the
plan: the issue was written from the design, and then the design changed.

Verify three things, in this order:

1. **Existence** — every feature the work shipped has an issue.  Something built with
   no catalogue entry is invisible to everyone who reads the catalogue instead of the
   code.  Something *removed* still has one, and shouldn't.
2. **Accuracy** — the prose and the example describe what the code does **now**:
   the real name, signature, defaults, and behaviour, not what was proposed.  **Run
   the example** — do not eyeball it.  If the generator turns an example into a test,
   a stale example is a test asserting the old contract.
3. **Tags** — the labels match reality.  A label is a query surface: the wrong one
   hides the feature from everyone who filters by it, which is indistinguishable from
   the feature not existing.  Check the kind/subject partition and any status or tier
   the catalogue sorts on.

Then **regenerate the shadow and run the drift guard**.  Never hand-edit a generated
file — edit the issue and regenerate.  A drift guard that fails on hand-edits is
protecting the one-home rule, not obstructing you.

**Two traps worth naming.**  A generator that promotes the FIRST fenced example into
a runnable test means a teaching snippet placed above the real example silently
becomes the test — put the runnable example first.  And a feature issue closed or
labelled by hand skips whatever automation the merge path would have run, so its tags
drift exactly like a hand-closed plan's do.

---

## Promoting a reference doc to a plan

1. **Audit shipped status FIRST.**  A doc titled "design" often has shipped phases
   in its body.  Grep the body for `shipped|done|implemented|landed|completed` and
   check the tree for matching code *before* choosing a destination — misroute risk
   is real, and a mid-promotion status question from the user is the brake: apply it.
2. **Route by the audit:** mostly shipped → close it directly · mostly
   trigger-deferred → defer · genuinely-future + multi-phase → a plan ·
   genuinely-future + one row → leave the doc in place and add an `## Open work`
   section (the light flow).  Reference content with an open tail stays a doc + an
   `## Open work` section — a thin "pointer-plan" that only links back to a doc is
   over-engineering; don't create one.
3. If promoting to a full plan: move the doc into the flat plan directory, apply
   the opening steps from step 4 onward, and rewrite incoming links to the old path
   (same grep as closing, step 5).

---

## Transferable pitfalls

Hard-won and tree-independent:

1. **Audit shipped status before routing.**  The commonest misroute is filing a
   partly-shipped doc as "future".  Audit first; the user's mid-flight status
   question is the brake.
2. **Prefer an `## Open work` section in the reference doc over a pointer-plan.**  A
   thin plan that only links back to a doc adds a layer of indirection without
   saving content.  One home, no pointer.
3. **Split when broad; single-file when bounded.**  A doc with broad
   intended-to-finish scope → split into focused files.  A doc with an explicit
   scope ceiling ("never going to ship past X") → one file with a status block.
4. **Close = move reference out + rewrite incoming links.**  This is the
   most-skipped step and the source of link rot.  Don't skip the grep + drift
   check.
5. **Named value categories beat numbered tiers.**  Categories that name the *kind*
   of value are stable across sessions; numbered tiers (V1/V2/V3) get re-ranked
   constantly.  Re-categorize only when scope actually changes.
6. **No per-plan entry in the master-instructions index.**  It duplicates the
   plans-overview doc and costs tokens every session.  Plans are discoverable via
   the overview.
7. **Never calendar-time language** in plans, roadmaps, or memory — "2–3 weeks"
   ships in 2 days and "quick fix" takes weeks.  Use effort letters.  Historical
   retrospectives that *document* the rule's validity are fine to keep.
8. **Investigation plans get their own template and reading order**
   (Status → Probes → Cluster docs → Roadmap).  Forcing investigation work into the
   standard feature-ship template either bloats the README or loses the catalogue.
9. **Probe-first beats source-first** for failure-class work.  Source-reading
   without a probe to ground it explores code paths without converging; the probe
   suite is the executable spec for "understood".
10. **Liberal probes, attic-curate after.**  Make probes liberally; curate at the
    end of Stage A.  Real-consumer extraction is non-negotiable — it surfaces
    classes synthetic probes never imagine.
11. **Verified-vs-hypothesized accountability prevents drift.**  Mark every
    mechanism claim; the table answers "do we actually know?" honestly.
12. **Tools as needed, not upfront.**  Add the one tool blocking progress; list it
    in `Tool gaps`; revert nice-to-haves.
13. **Claim the issue before the directory — never number a plan by scanning local
    dirs.**  The issue number is the identity; file it first and name the directory
    after it.  Scanning the local tree for "the next free number" misses unmerged plans
    on sibling branches (a `15-debugger/` on a feature branch invisible from `main`),
    so you mint a duplicate that collides on merge — and GitHub issue numbers are
    immutable, so it cannot be cleanly reassigned afterward.
14. **Shipping a feature is not done until its catalogue entry describes the built
    thing.**  Where the issue is canonical and the docs are generated from it, stale
    prose, a stale example, or a wrong label ships as authoritative documentation.
    Verify existence → accuracy → tags, run the example, regenerate, let the drift
    guard confirm.  Behaviour that moved mid-plan is the likeliest to have drifted.

---

## Filing bugs found during plan work

A tracker issue is a **claim about the mainline**.  So, working inside a plan:
**file a problem only when it reproduces *outside* the plan — already on the
mainline.**  A pre-existing mainline bug you stumble on during plan work gets filed
(and cross-linked to the plan); a breakage the plan's own in-progress work caused
is branch-internal — it lives in the plan's docs and is fixed on the branch, never
filed.  Investigation plans are the strongest case: the probes + cluster docs
already document every shape, so a separate issue would just double-document it.

Full policy in the master instructions (loft: `CLAUDE.md § Bug-filing policy`).

---

## Bind to your tree

Everything above is tree-agnostic.  This section is the **only** loft-specific part
— repoint these to port the skill to another tree.

| Generic term | loft binding |
|---|---|
| Plan tracker / issue id | [`loft-lang/plans`](https://github.com/loft-lang/plans/issues) issues; id = `@PLN<N>`; next free: `gh issue list -R loft-lang/plans --state all --limit 1` |
| Plan directory (new model) | `doc/claude/plans/<N>-<slug>/` (core language) · `doc/claude/lib_plans/<N>-<slug>/` (libraries) — flat |
| Legacy layout (mid-migration — most existing plans) | `doc/claude/plans/{future,finished,deferred}/<N>-<slug>/` with rows in `doc/claude/ROADMAP.md` (still maintained — being retired in favor of the tracker).  Plans 51 (finished/) and 54 (future/) live here; new plans use the flat/tracker model above. |
| Standard plan template | [`doc/claude/plans/_TEMPLATE.md`](../../../doc/claude/plans/_TEMPLATE.md) |
| Investigation template | [`doc/claude/plans/_INVESTIGATION_TEMPLATE.md`](../../../doc/claude/plans/_INVESTIGATION_TEMPLATE.md) — canonical example `plans/finished/51-hidden-buffer-aliasing/` (5 clusters, 39 probes) |
| Close / defer procedure (full) | [`doc/claude/plans/_LIFECYCLE.md`](../../../doc/claude/plans/_LIFECYCLE.md) |
| Docs-vs-plans rule, three workflows, lifecycle, value categories | [`doc/claude/plans/README.md`](../../../doc/claude/plans/README.md) |
| Reference-doc `## Open work` homes | `NATIVE.md` / `PERFORMANCE.md` / `PACKAGES.md` / `QUALITY.md` |
| Value-category labels | `S/R/G/F/U/C/Q/N` (issue labels — definitions in `plans/README.md § Value categories`) |
| Feature catalogue (canonical) | [`loft-lang/features`](https://github.com/loft-lang/features/issues) issues; id = `@F<N>` (`kind:feature`) / `@I<N>` (`kind:infra`).  The ISSUE is the source; @PLN92 is the catalogue's own plan |
| Feature catalogue tags | `kind:feature` \| `kind:infra` — exactly one.  The wrong one files a language feature under infrastructure, where nobody looking for it will filter |
| Feature generated shadow (never hand-edit) | `index/features.json` + `doc/features/` + `tests/docs/features/*.loft` |
| Feature regenerate + drift guard | `make features-fetch && make features-gen`, then `make features-check` (fails on hand-edits or a stale shadow) |
| Feature example → test | the generator promotes the **first** ` ```loft ` fence in the issue body into a RUN test — put the runnable example first, never a teaching snippet |
| Drift checker | `scripts/check_doc_drift.sh` |
| Incoming-link grep (close/promote) | `grep -rn "plans/<NN>-<slug>" CLAUDE.md doc/claude/ --include="*.md"` |
| Investigation regression suite | `tests/scripts/NN-<slug>.loft` |
| Probe gates (leak / exit) | `LOFT_STORES=warn`; the `loft_suite` leak gate; both backends = interpreter + native |
| Execution modes to verify across | `--interpret` and `--native` |
| Canonical examples | partial defer: plan-28 / plan-12 · create-and-move close: `31-html-export → HTML_EXPORT.md` · trim-only close: `04-slot-assignment-redesign → SLOTS.md` |
| Branch / commit / bug-filing policy | `CLAUDE.md` |
| Full dev procedures (rebase, commit hygiene) | `doc/claude/DEVELOPMENT.md` |
