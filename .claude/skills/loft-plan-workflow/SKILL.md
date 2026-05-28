---
name: loft-plan-workflow
description: Procedures for plan-shaped work in the loft project — opening a new plan, closing a plan, promoting a doc to a plan, ROADMAP cleanup, applying value categories. Apply when the task involves plans/, lib_plans/, ROADMAP.md, or doc promotions. Cites source docs for definitions; does not restate.
user-invocable: false
---

# Plan workflow procedures

Use this skill when working on **plan organization**:
- "open / create a plan for X"
- "close plan X" / "extract documentation from finished plan"
- "promote `doc/claude/X.md` to a plan"
- "audit ROADMAP" / "ROADMAP cleanup"
- "what value category" / "where does X belong"
- "evaluate the plan workflow"

This skill is **procedural-only**.  Definitions / rationale / templates live in source docs:
- [`doc/claude/plans/README.md`](../../../doc/claude/plans/README.md) — docs-vs-plans rule, three workflows, value categories (S/R/G/F/U/C/Q/N), closure rule, light-flow lifecycle, roadmap workflow
- [`doc/claude/plans/_TEMPLATE.md`](../../../doc/claude/plans/_TEMPLATE.md) — canonical new-plan README shape (standard implementation plan)
- [`doc/claude/plans/_INVESTIGATION_TEMPLATE.md`](../../../doc/claude/plans/_INVESTIGATION_TEMPLATE.md) — investigation-plan shape (probes/ + cluster docs + verified-vs-hypothesized accountability; canonical example: `plans/future/51-hidden-buffer-aliasing/`)
- [`doc/claude/plans/_LIFECYCLE.md`](../../../doc/claude/plans/_LIFECYCLE.md) — 6-step procedure for closing OR deferring plans (Steps 4-6 are shared)
- [`doc/claude/ROADMAP.md`](../../../doc/claude/ROADMAP.md) — the work tables

**Cross-cuts:** branch / commit / push policy is in CLAUDE.md § Branch policy and memory entries.  This skill assumes you've already followed those.

---

## Three workflows — pick the lightest that fits

| Work shape | Path |
|---|---|
| Bug fix (single root cause, fits in one commit) | PROBLEMS.md row + regression test + commit.  No plan, no Open work entry. |
| Tiny deliverable (demo deploy, version bump) | ROADMAP row only.  No plan. |
| Operational change (CI tweak, doc fix) | Direct commit, no ROADMAP row needed. |
| **Light TODO** (the normal flow) — work that fits in a row of a reference-doc table | `## Open work` section in the relevant `doc/claude/<NAME>.md` (NATIVE.md / PERFORMANCE.md / PACKAGES.md / QUALITY.md are canonical examples).  Same lifecycle as a plan, just one row. |
| **Plan** — multi-phase initiative with explicit phasing, design-before-implementation discipline, cross-arc dependencies, or its own document space | Full `plans/<NN>-<slug>/` directory.  Capped at 2-3 active. |

The light flow is the **default**.  Promote to a plan only when the work is genuinely multi-phase and benefits from its own directory — most TODOs don't, even ones that take several sessions.  See [README § Three workflows](../../../doc/claude/plans/README.md#three-workflows-for-todo-items--pick-the-lightest-that-fits).

---

## Procedure A — Open a new plan (standard implementation shape)

Use when the plan's primary deliverable is a feature ship or fix landing.

1. Pick the next free integer in the relevant tracker (`plans/future/` for core-language; `lib_plans/future/` for libraries — independent counters).
2. `mkdir doc/claude/{plans,lib_plans}/future/<NN>-<slug>`
3. `cp doc/claude/plans/_TEMPLATE.md doc/claude/{plans,lib_plans}/future/<NN>-<slug>/README.md`
4. Fill in Status + Goal first.  Add Sub-arcs / Phase ordering / Open questions / Cross-arc dependencies / See also as the design clarifies.
5. Add ROADMAP row(s) citing this plan + tag with value category (S / R / G / F / U / C / Q / N).
6. Add `lib_plans/README.md` or `plans/README.md` Future-table row.
7. **Do NOT add a CLAUDE.md doc index entry by default.**  Plans are discoverable via plans/README.md; per-plan CLAUDE.md entries are redundant.  Add only if the plan introduces a NEW top-level reference concept (vanishingly rare).

Length budget: 100-300 lines.  Longer means reference content is leaking in — extract to `doc/claude/<NAME>.md`.

**When NOT to use Procedure A:** if the plan's first phase is "characterize the problem space" rather than "design + build", use [Procedure F](#procedure-f--open-an-investigation-style-plan) instead.

---

## Procedure F — Open an investigation-style plan

Use when the plan's **primary deliverable is mechanism understanding** before fix design.  Characterized by:

- Multiple sub-mechanisms / failure clusters to catalogue.
- Need to write deterministic probes (.loft files) to characterize each shape.
- Source-reading alone won't converge — needs probe-driven hypothesis cycles.
- The fix-design decision (Path C vs. shape-by-shape vs. hybrid) can't be made without the catalogue.

Canonical example: `plans/future/51-hidden-buffer-aliasing/` — 5 clusters, 39 probes, 4 per-cluster docs, 2 verified mechanisms.

### Steps

1. Pick the next free integer.  Same numbering pool as Procedure A.
2. `mkdir -p doc/claude/{plans,lib_plans}/future/<NN>-<slug>/probes`
3. `cp doc/claude/plans/_INVESTIGATION_TEMPLATE.md doc/claude/{plans,lib_plans}/future/<NN>-<slug>/README.md`
4. **Stage A first** — write `.loft` probes in `probes/` BEFORE source reading.  Be liberal about adding probes; better to attic a redundant variant than to miss a crucial shape (real-library extraction has caught NEW findings synthetic probes missed).
5. Run every probe on both backends.  Document results in `RESULTS.md` (full matrix).
6. **Curate A/B/C** in the README's probe table once the suite stabilises (typically when adding new probes stops yielding new failure modes).  A = reference (passes), B = problem (one per failure mode), C = attic (variants).
7. For each distinct failure mode, write `cluster-<id>-<slug>.md` with the verified-vs-hypothesized accountability table.
8. **Add Status & next-session roadmap** to the README — per-cluster action items with effort estimates.
9. **Tool gaps** — add a `Tool gaps` section to the README listing tools added or verified during the work.  Tools added during a plan are part of its output, not separate work.
10. Add ROADMAP row + tracker README row (same as Procedure A step 5-6).

### Length budgets

- README: 100-300 lines (same as standard plans).
- `RESULTS.md`: 200-500 lines (probe matrix + cluster summaries).
- Each `cluster-*.md`: 100-300 lines.
- Probe `.loft` files: 20-100 lines each (header comment + minimal repro + assertions).

A complete investigation plan with 5 clusters and 40 probes = ~45-50 files.  This is heavy — proportional to investigation depth, NOT a default.  A shallow investigation belongs in `## Open work` on a reference doc (Procedure E).

### Probe migration

Probes stay in `probes/` during investigation.  They migrate to `tests/scripts/NN-<descriptive>.loft` **per cluster, as each cluster's fix lands**, not all at once.  The plan stays open during phased implementation; closes when the last cluster's regression is in `tests/scripts/`.

---

## Procedure B — Close or defer a plan

Full spec: [`_LIFECYCLE.md`](../../../doc/claude/plans/_LIFECYCLE.md) (single checklist; close + defer share Steps 4-6).

**Pick the outcome first**:

- All phases shipped → **close** (`finished/`)
- Some/all phases paused with concrete trigger → **defer** (`deferred/`).  Partial defer: Status table grows SHIPPED / DEFERRED rows (canonical: plan-28, plan-12).
- All paused without concrete trigger → design moves to `DESIGN_DECISIONS.md`, not `deferred/`

For each shipped phase (whether closing or deferring):

1. **Tag** sections as REFERENCE / CLOSURE-RECORD / HISTORICAL.
2. **Pick reference home** — CREATE-AND-MOVE (`31-html-export → HTML_EXPORT.md`) or TRIM-ONLY (`04-slot-assignment-redesign → SLOTS.md`).
3. **Trim plan README** to ~50-150 lines.  Lead with `Status — DONE/SHIPPED YYYY-MM-DD` + reference cross-link.

For partial defers, the README also keeps full design content for the deferred phases.

Common to close + defer:

4. **Reclassify ROADMAP rows** — shipped parts leave; deferred parts stay if roadmap-tracked.
5. **`git mv` directory** + update tracker tables (Finished/Deferred row in `plans/README.md`).  Defer adds a row to `DEFERRED.md` with the trigger.
6. **Grep + rewrite incoming links** (THE most-skipped step):
   ```bash
   grep -rn "plans/<NN>-<slug>\|plans/future/<NN>-<slug>\|plans/finished/<NN>-<slug>\|plans/deferred/<NN>-<slug>" \
     CLAUDE.md doc/claude/ --include="*.md"
   ```
   Then run `scripts/check_doc_drift.sh` to catch what grep missed.

---

## Procedure C — Promote a `doc/claude/*.md` doc to a plan

1. **Audit shipped status FIRST** (the CONST_STORE / HTML_EXPORT lesson — see Pitfalls below).  Grep for `shipped|done|implemented|landed|completed` in the doc body; check tree for matching code.  Misroute risk is real.
2. Decide destination based on audit:
   - Mostly shipped → `plans/finished/` (apply Procedure B closure-rule directly)
   - Mostly trigger-deferred → `plans/deferred/` (apply Procedure E)
   - Genuinely future work + multi-phase → `plans/future/` or `lib_plans/future/`
   - Genuinely future work + fits one row → **light flow** (Procedure F): leave doc at root, add `## Open work` section, write the row there
   - Reference content + open-tail → leave doc at root + Procedure F (`## Open work`).  Pointer-plans were tried (33/35/lib-11) and shown to be over-engineering — prefer the light flow.
3. `git mv doc/claude/X.md doc/claude/{plans,lib_plans}/future/<NN>-<slug>/README.md`
4. Apply Procedure A from step 5 onwards (ROADMAP row, tracker README row, no CLAUDE.md entry).
5. Grep + rewrite incoming links to the old `doc/claude/X.md` path.  Same shell command as Procedure B step 4.

---

## Procedure D — ROADMAP cleanup pass

1. **Sweep for time projections.**  Grep for: `weeks of focused`, `multi-week`, `next [0-9]+ months`, `expected to take`, `2-3 weeks`, milestone version numbers as time anchors.  Replace with effort letters (XS/S/M/MH/H/VH/L) or remove.
2. **Sweep for shipped items.**  Per the Maintenance rule: completed items get removed from ROADMAP (closure lives in CHANGELOG + git history).
3. **Re-tag value categories** if scope changed (rare — categories stay stable per the user's "value of customers stays the same" direction).
4. **Verify all plan citations resolve.**  Grep for `(plans|lib_plans)/.../README\.md` and `ls` each path.
5. **Methodology stays out.**  ROADMAP holds work tables only.  Methodology lives in `plans/README.md § Roadmap workflow`.

---

## Procedure E — Light flow (`## Open work` in a reference doc)

The default for TODOs that fit in a row of a reference-doc table.  Same lifecycle as a plan, just lighter.

**Open**:

1. Pick the relevant `doc/claude/<NAME>.md` reference doc (or `lib/<name>/<NAME>.md` for library-scoped).  If it doesn't have an `## Open work` section, add one near the bottom (after `## See also` if present).  Canonical examples: NATIVE.md / PERFORMANCE.md / PACKAGES.md / QUALITY.md.
2. Add a row to the `## Open work` table: `| Item | Section | Status |` (or whatever columns the doc uses).  Each row links back to the section it touches.
3. Add a ROADMAP row tagged with value category (S / R / G / F / U / C / Q / N) + link directly at the section: `[NATIVE.md § Open work](NATIVE.md#open-work)`.  Don't create a tracker README row (`## Open work` rows are NOT plans). <!--noindex-->
4. **No CLAUDE.md entry, no plan directory, no DEFERRED.md row** — the work is discoverable via the reference doc + ROADMAP.

**Work**: edit the reference doc's architecture content directly when implementing.  Same file holds the row + the design.

**Close** (when shipped):

1. Remove the row from `## Open work`.
2. Update the surrounding architecture content to reflect the new state (the same edit that closed the work usually does this).
3. Remove the ROADMAP row.
4. Closure record lives in commit message + CHANGELOG_TECHNICAL.md.  No closure-record file.

**Defer** (work paused with a concrete trigger):

- Annotate the row inline (e.g. `**Blocked on X** — unpauses when Y`) OR move the trigger to [`DEFERRED.md`](../../../doc/claude/plans/DEFERRED.md) if cross-cutting.
- Keep the row in `## Open work` (it's still tracked, just paused).

**Promote to plan**: if the row grows into multi-phase work, copy `_TEMPLATE.md` and migrate.  Don't promote prematurely — multi-row clusters often stay light if rows are independent.

---

## Value category quick-reference

Full definitions: [`plans/README.md § Value categories`](../../../doc/claude/plans/README.md#value-categories--what-kind-of-value-not-just-how-much).  Read order top-to-bottom (highest priority first):

| Tag | What |
|---|---|
| **S** | Silent failure / data-loss prevention (validation matrices, JSON-correctness, leak prevention) |
| **R** | Regression / release-blocker (known broken, gates next tag) |
| **G** | Goal-enabling (browser games, multiplayer, native debug) |
| **F** | Foundation (unblocks 2+ downstream plans) |
| **U** | Ease of use (UX, DX, ergonomics) |
| **C** | Clean features (correctness, removes corners) |
| **Q** | Internal quality (perf, refactor) |
| **N** | Niche / opportunistic |

S sits above R because silent failures have no error message — invisible to users, erodes trust most.

---

## Pitfalls (lessons from the cleanup arc)

1. **CONST_STORE misroute pattern.**  Doc title said "design"; body had Phase A + D shipped + Phase B + C deferred.  My initial route to `future/` was wrong.  **Lesson:** audit shipped status BEFORE picking destination directory.  When user asks a status question mid-promotion, that's the brake — apply it.

2. **Pointer-plan over-engineering.**  Created 4 pointer-plans (33-native-codegen-followups, 34-performance-followups, 35-quality-followups, lib_plans/11-packages) that were thin status tables linking back to reference docs.  Trial collapse of 35 → QUALITY.md `## Open work` section showed the pointer-plan added a layer without saving content.  **Lesson:** prefer `## Open work` section IN the reference doc over a separate pointer-plan.  Single source of truth, no indirection.

3. **WEB_SERVICES split was right; TIC_TAC_TOE single-file was right.**  WEB_SERVICES was broad ("fully functioning library" intent → split into 3 files).  TIC_TAC_TOE had an explicit scope ceiling ("visual playable game NEVER going to ship") → single file with status block.  **Lesson:** split when scope is genuinely broad and intended-to-finish; keep single-file when scope is deliberately bounded.

4. **finished/ link rot.**  Reference content embedded in finished plans → other docs link to closed plans for design reasons → links rot when content moves.  The closure rule (move reference out, update links) prevents this.  **Lesson:** Step 4 of Procedure B (grep + rewrite) is the most-skipped step.  Don't skip it.

5. **Categories felt arbitrary as V1/V2/V3.**  Replaced with named categories (S/R/G/F/U/C/Q/N) that capture the KIND of value, not just intensity.  **Lesson:** named categories are more stable across sessions than numbered tiers.  Re-categorization happens when scope changes (rare); re-ranking happened constantly with V1/V2/V3.

6. **CLAUDE.md per-plan entries are redundant.**  Adding a CLAUDE.md doc-index entry for every promoted plan duplicated plans/README.md content + cost tokens every session.  **Lesson:** plans are discoverable via plans/README.md.  Don't add per-plan CLAUDE.md entries; the index entries for `plans/README.md` and `lib_plans/README.md` are sufficient.

7. **Time projections rot in both directions.**  "Will take 2-3 weeks" has shipped in 2 days; "quick fix" has taken weeks.  **Lesson:** never use calendar-time language anywhere in ROADMAP, plans, or memory.  Use effort letters (XS-L).  Historical retrospectives that DOCUMENT the rule's validity are fine to keep ("Faster than the 2-3 week original estimate" in finished/09 stays as evidence).

8. **Investigation-plan shape needs its own template** (PLAN51 lesson, 2026-05-28).  The standard `_TEMPLATE.md` assumed feature-ship deliverables: Goal → Sub-arcs → Phase ordering.  When the deliverable is mechanism understanding, the right reading order is Status → Probes → Cluster docs → Roadmap.  Forcing investigation work into the standard template either bloats the README with probe headers or loses the catalogue structure.  **Lesson:** investigation plans get [`_INVESTIGATION_TEMPLATE.md`](../../../doc/claude/plans/_INVESTIGATION_TEMPLATE.md) and Procedure F; standard plans get `_TEMPLATE.md` and Procedure A.  The shapes diverge enough that one template can't serve both.

9. **Probe-first beats source-first for failure-class investigation** (PLAN51 lesson).  Source-reading without a probe to ground it tends to recursively explore code paths without convergence.  A probe suite is the executable spec for what "understood" means — a hypothesis is verified when the probe-pair diff confirms it.  **Lesson:** for failure-class investigations, write probes BEFORE opening the source.  Stage A's deliverable is "every shape we can think of, with a deterministic probe"; Stage B's deliverable is "every cluster's mechanism pinned via probe-diff + source reading".

10. **Be liberal with probes, attic-curate after** (PLAN51 lesson, user-confirmed).  Missing a crucial case is the worst failure mode; carrying a few redundant variants costs nothing because they go to attic when curated and don't pollute the suite.  Real-library extraction (e.g. PLAN51's probe 39 from `lib/moros_map`) is non-negotiable — it surfaced a leak class no synthetic probe found.  **Lesson:** the rule is "make probes liberally, attic them directly"; not "be selective".  Curation happens at the end of Stage A, not during.

11. **Verified-vs-hypothesized accountability prevents drift** (PLAN51 lesson).  Every mechanism statement in a cluster doc is either VERIFIED (with cited trace file path or code-line read) or HYPOTHESIZED (marked with 🤔).  When asked "do we know what's going on?", the table answers honestly instead of waving at "we kinda know".  **Lesson:** investigation plans need an explicit knowledge-accounting column.  Without it, hypotheses drift into the prose as if they were facts.

12. **Tools-as-needed, not tools-upfront** (PLAN51 lesson).  Don't write a debugging framework upfront for an investigation plan.  Check existing toolchain (CLAUDE.md inventory); add the ONE tool blocking progress; keep "nice-to-have" tools as ad-hoc eprintln patches reverted before commit.  Tools added during an investigation plan are part of its output, listed in a `Tool gaps` section.  **Lesson:** PLAN51 added one new env var (`LOFT_KEEP_NATIVE_RS=1`) and verified one existing flag (`LOFT_LOG=slots:`) — sufficient for the whole investigation.  Building more would have been over-engineering.

---

## Cross-doc map (quick reference)

| For… | Read… |
|---|---|
| Why docs vs plans split | `plans/README.md § The rule — docs vs plans` |
| Pick light vs plan vs bug | `plans/README.md § Three workflows` |
| Pick standard plan vs investigation plan | This skill § Procedure A vs F |
| New plan README shape (standard) | `plans/_TEMPLATE.md` |
| New plan README shape (investigation) | `plans/_INVESTIGATION_TEMPLATE.md` |
| Closing or deferring a plan (full procedure) | `plans/_LIFECYCLE.md` |
| Light-flow lifecycle (open / work / close / defer in `## Open work`) | `plans/README.md § Light flow lifecycle` |
| Drift detection (broken paths, stale claims, time projections) | `scripts/check_doc_drift.sh` |
| Value category definitions + rationale | `plans/README.md § Value categories` |
| Roadmap organization rules | `plans/README.md § Roadmap workflow` |
| Branch / commit / push policy | `CLAUDE.md § Branch policy` |
| Bug-filing workflow | `CLAUDE.md § Bug-filing policy` |
| Full dev procedures (rebase, commit hygiene) | `doc/claude/DEVELOPMENT.md` |

When work spans plan + dev concerns, follow the relevant procedure here for the plan side, and CLAUDE.md / memory for the dev side.
