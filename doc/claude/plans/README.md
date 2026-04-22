# Plans

Multi-phase initiatives that span more than one session.  Each
subdirectory holds the README (goal + index) plus one markdown file
per phase.

## Conventions

- Subdirectory names are numbered (`NN-slug`) so they sort in the
  order they were opened.  The number is a monotonic counter — it
  does not imply priority.
- A new initiative opens with an `NN-slug/README.md` stating the
  goal, phase layout, and ground rules, plus a first phase plan
  file (conventionally `00-<first-phase>.md`).
- Every phase plan file begins with `Status: open | in-progress |
  done` so a fresh session can orient quickly.
- When an initiative is fully closed (all phases committed, no open
  follow-ups), move its entire subdirectory into `finished/`.
  That way `ls doc/claude/plans/` at a glance shows only live work.

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

## Current initiatives

| Dir | Initiative | Current phase |
|---|---|---|
| `04-slot-assignment-redesign/` | Replace the two-zone slot allocator + orphan-placement post-pass with a single-pass liveness-driven algorithm.  Driven by a recurring class of heap-corruption bugs (P178, P185) that each required a targeted patch on top of the existing heuristics. | Phase 0 — characterize current behaviour with an exhaustive fixture catalogue |

## Finished initiatives

| Dir | Initiative | Closed |
|---|---|---|
| `finished/00-inline-lift-safety/` | Eliminate silent memory corruption from inline struct-returning calls in expression contexts (P181 family). | 2026-04-18 — all phases done; 18 snippet variants pass; spec captured in `doc/claude/LIFETIME.md` |
| `finished/01-integer-i64/` | Eliminate `i32::MIN`-as-null sentinel and silent wrap / div-by-zero; decouple arithmetic width (i64) from storage width. | 2026-04-21 — `integer` is i64 end-to-end; `Type::Long` + `long` keyword + `l` suffix removed; 34 duplicate `Op*Long` opcodes reclaimed; binary-format lint; `.loftc` cache removed. |
| `finished/02-narrow-collection-elements/` | Make `vector<i32>` / `hash<T[key]>` / `sorted<T[key]>` / `index<T[key]>` honour the `size(N)` annotation on integer aliases (P184 — post-C54 follow-up). | 2026-04-22 — all phases (0/1/2/3/4a/4b/5/6) done.  Phase 4b landed via Option L-minimal after two earlier attempts uncovered a pre-existing `narrow_int_cast` bug in iter-next blocks (Bug α) — fixed alongside the `Parts::ShortRaw` direct-encoding variant. |
| `finished/03-native-moros-editor/` | Wire the Moros editor into a runnable native OpenGL program (windowed or fullscreen), filling the input API + fullscreen gaps the existing graphics library didn't cover. | 2026-04-22 — all seven phases (0/1/2/3a/3b/4/5/6) done.  Phase 3b landed with a native codegen fix for the `s.const_refs` / `s.string_from_const_store` gap that previously blocked any loft function reconstructing constants under `--native`.  `make editor-dist` produces a shippable `dist/moros-editor/`. |

## One-off plans elsewhere

Per-session ephemeral plans not tied to a multi-phase initiative
live under `~/.claude/plans/` (flat, generated filenames).  Those
are not committed to the repo.
