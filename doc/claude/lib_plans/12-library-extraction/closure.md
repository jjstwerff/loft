<\!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Plan-12 — documentation harvest + close-out

Part of [@PLAN12 library extraction](README.md).  Covers
**Phase 6.13** — the closure ritual that prevents plan-12
from "just stopping" with valuable design content stranded
in a finished-plan README no one reads.

The harvest happens AS each 6.x phase ships (extract its
design content into the target permanent doc at landing
time), not all-at-once at close.  6.13's explicit phase
status keeps it on the radar through the plan's tail.

---

### Phase 6.13 — documentation harvest + close-out (proposed 2026-05-31)

**Trigger.**  Plan-12 is unusually doc-heavy.  By the time
6.5 + 6.6 + 6.7 + 6.8 + 6.11 + 6.12 + Stage B + lib-plan 30
have all landed, this README is ~2500+ lines of design
content, decision records, and lessons learned.  Closing the
plan by simply moving the file to `lib_plans/finished/`
strands all of that — finished plans are read for archaeology,
not as ongoing reference.  Without explicit doc-harvest work,
"where's the canonical authoring guide?" gets the wrong
answer "open this 2026 plan."

**Goal.**  When plan-12 closes:

- Every piece of durable design content lives in a PERMANENT
  reference doc (PACKAGES.md, PKG_REGISTRY.md, or a
  purpose-named new doc), not in the finished plan.
- The finished plan is a compressed retrospective +
  chronological landing log.  ~500 lines, not ~2500.
- User-facing onboarding for "install loft", "use libraries",
  "publish a library", "security model", "library catalog"
  exists as top-level docs (not buried inside Claude-internal
  references).
- No stale in-monorepo `lib/<pkg>/` references survive in any
  doc.
- `CLAUDE.md`'s doc index reflects the new layout; reading-
  by-goal paths route through current docs, not finished
  plans.

**Scope.**  Six categories of work, each enumerated in detail
in [§ Evaluation — doc state after plan-12 lands](#suggested-closure-sequence-the-work-to-actually-do)
above.  Recapped here as concrete deliverables:

| Category | Output | Lines |
|---|---|---|
| **Migration from plan-12** | Move design content from this plan to permanent docs.  Phase 6.5 template → PACKAGES.md § Library CI; Phase 6r per-symbol rule → PACKAGES.md or `LIBRARY_AUTHORING.md`; Phase 6.6 auto-install → PACKAGES.md § Auto-install; Phase 6.7 advisory schema → PKG_REGISTRY.md § Security advisories; Phase 6.8 `loft update` → `CLI.md`; Phase 6.11 offline → `OFFLINE.md`; Phase 6.12 dev-loop → DEVELOPMENT.md § Test fixtures; verify-on-recompile tables → PACKAGES.md § Verification + lib-plan-30. | varies |
| **New Claude-internal docs** | `LIBRARY_AUTHORING.md` (end-to-end "publish a library" guide); `OFFLINE.md` (air-gap + bundle workflow + loft-dev offline loop). | ~400 + ~250 |
| **New user-facing docs (repo root)** | `INSTALL.md` (install.sh + OS packages + self-update); `SECURITY.md` (trust model + vuln disclosure); `PUBLISHING.md` (author's view); `USING_LIBRARIES.md` (consumer's view of `use` + manifest + lockfile + CLI). | ~150 + ~200 + ~300 + ~250 |
| **Library catalog generator** | Script that pulls `index.json` from the registry and writes a markdown catalog page; CI auto-update; published at `loft-lang.org/libraries` or `doc/library-catalog.md`. | ~50 (script) + dynamic page |
| **CLAUDE.md table surgery** | Add new docs to index; retire obsolete reading-by-goal rows ("Implement `loft install`" → done; "Build the `server` library" → it lives in chunk repo now); update reading-by-goal paths so "Add a feature to the compiler" doesn't route through plan-12. | ~30 row changes |
| **Reference audit + sweep** | `grep -rln "PLAN12\|plan-12\|12-library-extraction" doc/` — rewrite every survivor to point at the new permanent doc OR cite the finished-plan retrospective.  No reference points at an open phase. | per-file edits across ~20 docs |
| **Plan-12 closure** | Split `lib_plans/12-library-extraction/README.md` into `README.md` (compressed retrospective, ~500 lines) + `LANDING_LOG.md` (chronological per-phase landing record, ~500 lines).  `git mv` to `lib_plans/finished/12-library-extraction/`. | retrospective rewrite + log compile |

**Harvest cadence — DON'T accumulate.**

The critical rule: harvest each 6.x phase's design content
INTO the permanent doc AT THE TIME THE PHASE SHIPS, not at
plan close.  Otherwise 6.13 becomes a 2-week migration
sprint where one person tries to remember why each design
decision was made.

```
Phase 6.6 ships → extract auto-install design into PACKAGES.md § Auto-install
Phase 6.7 ships → extract advisory schema into PKG_REGISTRY.md § Security advisories
Phase 6.8 ships → extract `loft update` UX into CLI.md
Phase 6.11 ships → create OFFLINE.md from the section
Phase 6.12 ships → extract fixture pattern into DEVELOPMENT.md § Test fixtures
...
Phase 6.13 close → just the user-facing docs + cleanup + plan split
```

At 6.13 time, the plan README is already ~half-migrated.
What's LEFT in the plan is just:
- Retrospective narrative (kept; that's the closure record).
- Stage A / Stage B per-chunk landing log (kept; chronological).
- Implementation lessons not yet folded elsewhere (rare; fold them).

**Implementation outline (M, ~3-5 work-days):**

1. **Per-phase harvest (continuous, ~half day per shipping
   phase)** — when a phase merges, extract its `### Phase 6.x detail`
   section's permanent content into the target reference doc.
   Leave a compact landing-record stub in the plan ("Shipped
   2026-Q? in commit `<sha>`; design now at PACKAGES.md § Foo").
2. **User-facing doc creation (sprint, ~2 days)** —
   `INSTALL.md`, `SECURITY.md`, `PUBLISHING.md`,
   `USING_LIBRARIES.md`.  Each ~150-300 lines, mostly
   reorganisation + tone shift from Claude-internal to
   user-facing.
3. **Library catalog generator (~half day)** — `scripts/gen_library_catalog.py`
   that pulls `index.json` and writes `doc/library-catalog.md`.
   Wire into CI to auto-update on registry change.
4. **CLAUDE.md surgery (~half day)** — add new docs to index,
   retire obsolete rows, rewrite reading-by-goal paths that
   still route through plan-12.
5. **Reference audit (~half day)** — `grep -rln` for the
   plan / phase identifiers; rewrite each surviving reference
   to point at the permanent doc.
6. **Plan-12 split + move (~half day)** — compress the
   plan's narrative into a retrospective README; extract
   per-phase landing chronology into LANDING_LOG.md; `git mv`
   to `lib_plans/finished/12-library-extraction/`.

**Verification (the gate that closes the plan):**

A `tests/doc_hygiene.rs` test plus a manual checklist:

```rust
#[test]
fn plan12_no_open_phase_references() {
    // After plan-12 closes, every reference to PLAN12 / plan-12 / 12-library-extraction
    // in doc/ must point at either:
    //   (a) lib_plans/finished/12-library-extraction/README.md (retrospective), OR
    //   (b) lib_plans/finished/12-library-extraction/LANDING_LOG.md (chronology)
    // NEVER at "Phase X" inside an open plan.

    // Walk doc/, grep for the identifiers, classify each survivor.
    // Fail if any "Phase 6.X" or "§ Phase X" reference survives outside the finished plan.
}
```

Manual checklist (the "done when" recital):

- [ ] `make ci` green (existing gates plus the new plan12_no_open_phase_references).
- [ ] All shipping 6.x phases have a one-line "Shipped {date}: see {permanent-doc}" entry in the plan.
- [ ] `INSTALL.md`, `SECURITY.md`, `PUBLISHING.md`,
      `USING_LIBRARIES.md`, `LIBRARY_AUTHORING.md`,
      `OFFLINE.md` exist and pass doc_hygiene.
- [ ] `doc/library-catalog.md` generates cleanly from `index.json`.
- [ ] `CLAUDE.md` doc-index table reflects new docs; obsolete
      reading-by-goal rows retired.
- [ ] `grep -rln "PLAN12" doc/` returns only references to the
      finished plan + LANDING_LOG (no "Phase X" pointers
      survive).
- [ ] Plan moved to `lib_plans/finished/12-library-extraction/`;
      readme compressed to retrospective shape (~500 lines).
- [ ] LANDING_LOG.md present with per-phase chronology.
- [ ] User who's never seen the plan can reach "how do I
      publish a library?" from `CLAUDE.md` in ≤2 clicks.

**Why this is its own phase, not "just close-out work."**

Closure is real work.  Other recently-closed plans
(`plans/finished/22-mutable-closures/`,
`plans/finished/52-value-block-borrow-cleanup/`,
`plans/finished/44-hash-semantics/`) demonstrate that doc
discipline at close determines whether the finished plan
serves as a useful artifact or becomes archaeology.  Plan-12
is unusually large; its closure work deserves explicit phase
status so it doesn't get treated as "the cleanup task someone
will get to."

**Open questions:**

1. **Catalog format.**  HTML page on `loft-lang.org`, or
   markdown in the repo, or both?  Recommendation: markdown
   in repo (commitable, no hosting dep) + auto-rendered HTML
   on loft-lang.org as a polished view.
2. **Retrospective compression target.**  Keep all design
   detail, or just decisions + outcomes?  Recommendation:
   decisions + outcomes + the few "what we learned" lessons
   that surface design pitfalls future projects should know
   about.  Implementation detail moves to permanent docs.
3. **Should 6.13 also fold lib-plan 30's design?**  No —
   lib-plan 30 is its own slot, with its own lifecycle.  Its
   doc harvest is lib-plan-30's responsibility when it
   closes.
4. **Versioned doc snapshots.**  Should the doc state be
   tagged with each minor release ("this is how things
   worked in v0.9.0")?  Out of scope for 6.13;
   CHANGELOG.md already serves this purpose at the user
   level.

