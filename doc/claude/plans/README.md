# Plans

Multi-phase initiatives that span more than one session.  Each
subdirectory holds the README (goal + index) plus one markdown file
per phase.

## Companion indexes — every parked item is discoverable

Two files complement this README; together they ensure deferred
work is never silently dropped.

- **[`DEFERRED.md`](DEFERRED.md)** — internal index of every parked
  validation phase, deferred P-issue, and "noted but not now" item.
  Each row carries an explicit `Trigger to unpause:` value.
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

**Default discipline:** finish the validation plans before shipping
new feature work.  Override only when USER_FACING.md surfaces an
S0 item or an S1 item that's been deferred for two releases — see
USER_FACING.md § "Severity override".

### Two tracks: validation (primary) and showcase (parallel)

Sessions follow two complementary tracks:

- **Validation track (primary).**  Finish-plans-first: validation
  matrices, P-issue closure, language-quality work.  This is where
  the bug yield comes from and what makes loft *worth using*.
  Severity-driven (USER_FACING.md S0/S1/S2 tiers).  **Default
  candidate for every session.**

- **Showcase track (parallel).**  Strategic-recruitment work:
  brick-buster, browser-parallel (A10), world-rendering demos,
  performance benchmarks.  This is what makes loft *visible* and
  attracts contributors.  Strategic-driven (USER_FACING.md
  § Strategic showcase track).  **Worked when validation work hits
  a natural breakpoint** — phase closes, pre-flight survey shows
  low yield for the next phase, or the showcase item is gating an
  external commitment (demo date, talk).

The two tracks are orthogonal — a piece of work isn't both
validation and showcase.  The user's 2026-05-04 priority statement
locked this in: "[the demo] will not keep me from improving loft
(that is my first priority) but the OpenGL demo in a good state is
our biggest asset to get more developers."  Improving loft is the
first track; the demo is the second.

When a session opens with no clear next step, pick from the
validation track first.  Only if validation has no S0/S1 work in
flight AND no S2 work in mid-investigation, advance the showcase
track.

**Yield-based transition rule.** The validation track stays
primary while matrix bug-yield rates remain high.  When a plan's
pre-flight survey closes a phase with **0-1 P-issues found in 5+
cells**, that's the signal that the cheap bugs in that surface
are exhausted.  Plans that consistently hit the 0-1 threshold
across consecutive phases get demoted to "matrix-as-documentation"
(per the gating already documented in plans 15/16/17 risk
sections) and the freed time advances the showcase track.

This rule keeps both tracks honest:
- Real-world workloads (OpenGL/world-chunk) DO find bugs, but at
  lower per-hour rates than the matrix work currently produces
  (3-6 P-issues/session in May 2026).  Most real-world bugs are
  also downstream of matrix-foundational bugs, so fixing the
  matrix first means the showcase work doesn't trip over them.
- The matrix work is finite — once validated, the same surface
  doesn't yield more.  When the rate drops, switching to
  showcase gets a higher marginal yield.

The May 2026 snapshot has matrix yield well above this threshold;
validation stays primary.  Reconsider per-session as plans close.

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
- When an initiative is intentionally paused — well-described, no
  driving bug or feature, picked up only when triggered — move its
  entire subdirectory into `deferred/`.  Deferred plans differ from
  finished plans: they're not done, they're parked.  Their READMEs
  must state the **trigger** that would unpause them (a P-issue in
  the relevant area, a user-visible feature need, contributor
  appetite, …).  Sitting in `deferred/` signals "available, not
  abandoned."

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

| Dir | Initiative | Status |
|---|---|---|
| [`06-typed-par/`](06-typed-par/) | Simple typed `par`: collapse the 7-variant runtime + 3-fn native dispatch into one store-stitch path; "everything is a store".  Retires ~1100 lines net across `src/parallel.rs` and `src/codegen_runtime.rs`.  **Doubles as a structured bug-hunt of the type-system × native-codegen × parallel-runtime intersection** — 14+ P-issues filed/closed during the work so far (P188-P201 family).  See plan README § "Realised value (so far)" for the running tally. | Phases 0/1/1.5/2a + ARC steps A1/A2/A6 done; A3 in-flight (parser gate pending); A4/A5/A7-A11 open; bug-finding regime active.  Phase 9a (T1.8a prerequisite) closed 2026-05-04; phases 9b/9c/9d/9e unblocked but still open.  **4 ignored canaries** (all par-side tuple dispatch — blocked on 9b/9c/9d, no longer on T1.8a) |
| [`07-error-messages/`](07-error-messages/) | Better error messages: every error reaches the user as `file:line:col` + concrete message + source line with caret + optional suggestion.  Spans on IR, pc→source-line table, typed `RuntimeError`, retire the implicit panic-vs-sentinel coin-flip. | Phases 0/1/2/3 shipped (rustc-style renderer + caret + UTF-8/tab + cascade dedup + summary line + `LOFT_ERRORS` env + `--errors` CLI + pc→source-loc on panic + SIGSEGV); phases 4-7 open |
| [`08-repl-and-introspection/`](08-repl-and-introspection/) | REPL + interpreter-introspection tool — `loft>` interactive prompt with persistent state plus a clean CLI surface for IR/Rust/slot-table dumps. | Phase 0 + 1 shipped; phases 2-6 open |
| [`14-tuple-validation/`](14-tuple-validation/) | Validate tuples are fully typed and round-trip-correct across every {element type × storage destination} cell, with mandatory **interp/native byte-identical stdout** under a new cross-mode harness.  Closes T1.8c (struct-ref move semantics) and decides T1.11a (tuples in struct fields).  Phase 00 freezes the matrix and ships the harness; phases 01-05 fill the cells; phase 06 doc-reconciles. | Phase 00 + 01 shipped (16/17 cells PASS, 1 P207-ignored); P206 (parser hang on `->` arm) and T1.8a (tuple-of-text return under `--native`) closed in passing.  Phases 02-06 open. |
| [`15-closure-validation/`](15-closure-validation/) | Validate closures (`Type::Function`) round-trip across every {capture composition × storage destination} cell.  Reuses the plan-14 cross-mode harness.  Active risk: closure-DbRef leak in `LIFETIME.md § Function — NOT YET HANDLED`.  Phase 03 (text captures) decides leak fix vs document. | Plan drafted; phase 00 ladder + matrix open. |
| [`16-coroutine-validation/`](16-coroutine-validation/) | Validate coroutines (`fn() -> iterator<T>`) round-trip across every {yielded type × drive context} cell.  Reuses the plan-14 cross-mode harness.  Pins `yield from` (CO1.4) deferral via CLOSED cells until 1.1+.  Phase 02 (yielded text) is the active state-machine-lowering risk. | Plan drafted; phase 00 ladder + matrix open. |
| [`17-template-validation/`](17-template-validation/) | Validate bounded generics / interfaces (`<T: Bound>`) across every {T-parameter usage × bound shape} cell.  Reuses the plan-14 cross-mode harness.  Pre-flight survey (5 tests) found a 60% bug rate — three subsystem-distinct issues: parser doesn't extend tuple element/destructure to bounded-generic results; type inference in generic body doesn't propagate bound-supplied method signatures; built-in `Printable` satisfaction contradicts the documented contract.  Highest expected bug yield of the four matrix plans. | Plan drafted; phase 00 ladder + matrix open. |
| [`18-match-validation/`](18-match-validation/) | Validate `match` expression dispatch across every {subject type × pattern shape} cell.  Reuses the plan-14 cross-mode harness.  Pre-flight (6 tests) found 33% hang rate on or-patterns (`1 \| 2 \| 3 => …`) and `@` bindings — likely sibling of the P206 fix in different `parse_*_match` variants.  Phase 01 closes those hangs; phases 02-05 conditional on yield. | Plan drafted; phase 00 ladder + matrix open. |
| [`19-struct-enum-validation/`](19-struct-enum-validation/) | Validate struct-enum dispatch (`is`, field capture, match arms, methods) across every {variant payload × dispatch context} cell.  Reuses the plan-14 cross-mode harness.  Pre-flight (5 tests) found 1 bug (20%): method-on-parent-enum-type called via `.method()` on a variant value fails with "Unknown field Variant.method".  Phase 03 closes the C5 method-resolution gap. | Plan drafted; phase 00 ladder + matrix open. |
| [`20-collection-validation/`](20-collection-validation/) | Validate keyed collections (hash / sorted / index / spacial) across every {collection × operation} cell, with a value-element sub-axis.  Pre-flight (3 tests) hit 67% panic rate — `sorted<>` and `index<>` cleanup both panic with `index out of bounds: the len is 66 but the index is 65535` at `src/database/structures.rs:609` (both backends; basic usage; correct output produced first then panic on scope exit).  Phase 01 closes the cleanup panic; phases 02-05 conditional on yield. | Plan drafted; phase 00 ladder + matrix open. |

## Deferred initiatives

Plans that are well-described but intentionally paused — picked up
only when a concrete trigger arrives.

| Dir | Initiative | Trigger to unpause |
|---|---|---|
| [`deferred/10-scope-exit-emission/`](deferred/10-scope-exit-emission/) | Scope-exit gate simplification.  Drops the `(dep.is_empty() \|\| is_work_ref) &&` prefix from `src/scopes.rs:1053` so cleanup emission no longer depends on dep-tracker precision.  Pure cognitive-clarity win; no P-issue closes here.  Originally framed as a P203 fix — that framing turned out wrong (P203 is a template double-sub bug). | A bug in this gate's territory, dep-tracking maintenance, or contributor interest. |
| [`deferred/12-codegen-simplifications/`](deferred/12-codegen-simplifications/) | Tier 1 (walker audit + forwarding-smoke retire) shipped on branch `plan-12-codegen-simplifications` (commits `c0c27e5` / `d446e5d`).  Tier 2 (dispatch arm migration phases 03-05) parked here.  Move-the-furniture refactor with no driving bug, no waiting feature, no performance gain.  Real value is "plan 13's preamble." | Same trigger set as plan 13: 3+ template-path bugs, OR major codegen evolution forcing ≥50 Op-annotation touches, OR contributor appetite.  Plan 12 Tier 2 only earns its keep if plan 13 unpauses. |
| [`deferred/13-rust-template-migration/`](deferred/13-rust-template-migration/) | Migrate ~200 `#rust"..."` template annotations in `default/*.loft` to hand-written runtime fns + registered emitters.  Single source of truth for Op emission.  Retires `output_call_template` and `Value::RawExpr`.  2-3 weeks of focused work. | 3+ template-path bugs accumulating, OR major codegen evolution that forces touching ≥50 Op annotations, OR contributor appetite for a multi-week structural refactor.  Plan 12 Tier 2 (phases 03-05) must land first — without it the migration target shape isn't uniform. |

## Finished initiatives

| Dir | Initiative | Closed |
|---|---|---|
| `finished/00-inline-lift-safety/` | Eliminate silent memory corruption from inline struct-returning calls in expression contexts (P181 family). | 2026-04-18 — all phases done; 18 snippet variants pass; spec captured in `doc/claude/LIFETIME.md` |
| `finished/01-integer-i64/` | Eliminate `i32::MIN`-as-null sentinel and silent wrap / div-by-zero; decouple arithmetic width (i64) from storage width. | 2026-04-21 — `integer` is i64 end-to-end; `Type::Long` + `long` keyword + `l` suffix removed; 34 duplicate `Op*Long` opcodes reclaimed; binary-format lint; `.loftc` cache removed. |
| `finished/02-narrow-collection-elements/` | Make `vector<i32>` / `hash<T[key]>` / `sorted<T[key]>` / `index<T[key]>` honour the `size(N)` annotation on integer aliases (P184 — post-C54 follow-up). | 2026-04-22 — all phases (0/1/2/3/4a/4b/5/6) done.  Phase 4b landed via Option L-minimal after two earlier attempts uncovered a pre-existing `narrow_int_cast` bug in iter-next blocks (Bug α) — fixed alongside the `Parts::ShortRaw` direct-encoding variant. |
| `finished/03-native-moros-editor/` | Wire the Moros editor into a runnable native OpenGL program (windowed or fullscreen), filling the input API + fullscreen gaps the existing graphics library didn't cover. | 2026-04-22 — all seven phases (0/1/2/3a/3b/4/5/6) done.  Phase 3b landed with a native codegen fix for the `s.const_refs` / `s.string_from_const_store` gap that previously blocked any loft function reconstructing constants under `--native`.  `make editor-dist` produces a shippable `dist/moros-editor/`. |
| `finished/04-slot-assignment-redesign/` | Replace the two-zone allocator + orphan-placer post-pass with a single-pass liveness-driven algorithm.  V2-drive retracted; landed the incremental refit (positional init ops, single function-entry `OpReserveFrame(frame_hwm)`, slot-move deletion, `OpText` deletion, I7 invariant).  V1 still drives codegen; V2 stays as a shadow validator. | 2026-04-23 — A / B.1 / B.2 / B.3 (atomic bundle `06a8d14`) / B.3-follow-up v2 (`f47cc93`) / B.4 all landed.  Original V2-drive goal retracted; companion plan-05 closed the orphan-placer elimination. |
| `finished/05-orphan-placer-elimination/` | Delete `place_orphaned_vars` by extending the main IR walk to reach every variable; fix P185. | 2026-04-23 — Phases 1a / 1b / 2 / 2c landed (`e0a020f` / `494e5c7` / `309e0f4` / `f74f78c`); ~150 LOC retired, P185 un-ignored.  Phase 2b (I8 invariant) dropped — defensive, no driving bug. |
| `finished/09-native-runtime-rewrite/` | Per-Op emitter dispatch on top of `#rust` template substitution; closed P200 / P202 / P203 / P205 in production via `OpEmitter` registry framework + 5 production custom emitters + `ParShape` runtime consolidation. | 2026-05-02 — all phases done; PR #197 merged. |
| `finished/11-p204-ref-propagation/` | Close P204 — refresh the unspan walker in `detect_ref_tail_capture` so the tail-call rewrite fires on Span-wrapped IR.  Surfaced the walker Span-miss pattern that plan-12 phase 01 generalised. | 2026-05-02 — bundled into PR #197. |

## One-off plans elsewhere

Per-session ephemeral plans not tied to a multi-phase initiative
live under `~/.claude/plans/` (flat, generated filenames).  Those
are not committed to the repo.
