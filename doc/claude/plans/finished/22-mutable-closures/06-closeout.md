<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 06 — Conservative-default audit + retrofit + closeout

**Status: done** — 2026-05-13

## Goal

Three closeout tasks:

1. **Audit `default/01_code.loft`** for purity annotations
   that cause false-positive Case B/C/D classifications.
   Tighten where the false-positive cost matters.
2. **Retrofit the two drivers** that motivated @PLAN22's
   promotion: TTT v6 server + @PLN6 audience-demo server.
   Convert their `.inner` ceremony to writable closures.
3. **Doc closeout** — LIFETIME.md updates, CHANGELOG_TECHNICAL.md
   entry, ROADMAP.md cleanup, plan moved to `finished/`.

## Audit (task 1)

Walk `default/01_code.loft` and flag every user-fn declaration
that's missing an explicit purity annotation.  Per
[DISCUSSION § Q2](DISCUSSION.md#q2--user-fns-with-unknown-purity),
`Unknown` purity defaults to "potentially mutates" — every
unannotated fn the closure analyzer sees on a captured argument
forces a conservative case-B/C/D classification.

For each unannotated fn, decide:
- **Pure (no side effects)** → annotate `#pure`.
- **Reads only** → annotate `#read`.
- **Writes parent (mutates first arg's content)** → `#impure(parent_write)`.
- **Writes self (mutates self field)** → `#impure(self_write)`.
- **Genuinely unknown / IO** → leave as `Unknown`.

Aim: every `default/01_code.loft` user-callable fn has a purity
annotation.  Reduces phase-02 false-positive rate at the
ecosystem-foundational layer.

## Retrofit (task 2)

### TTT v6 server retrofit

Per [README § Drivers](README.md#drivers), TTT v5 server uses
`Reference<T>` to mutate captured server state (`world`,
`next_session_id`, `replay_cache`, `last_active_player`,
`tick_counter`).  Each access reads `state.inner.X`.

Phase 06 retrofits to writable closures: `state.X` directly,
no `.inner` ceremony.  Verify the v6 server passes the same
multiplayer test suite (`tests/multiplayer_v5.rs` adapted for
v6 — or new `tests/multiplayer_v6.rs`).

### Plan-36 audience-demo retrofit

Plan-36 ([plans/6-audience-generative-art/](../../6-audience-generative-art))
hasn't shipped its server yet.  Phase 06 ships the server using
writable closures from day one.  Loft snippets projected to the
audience read `state.X` instead of `state.inner.X` — the
on-stage code-elegance driver mentioned in README § Drivers.

The @PLN6 audience server is itself a smaller integration
test: if writable closures hold up across that surface, @PLAN22
is production-ready for the lib/server use cases.

## Doc closeout (task 3)

- **LIFETIME.md** — add a "Mutating closure capture" subsection
  describing the four-case classifier, the lowering for case B
  (Reference + hidden cell), and the rejection for case D.
  Cross-link from the `Type::Function` section.
- **CHANGELOG_TECHNICAL.md** — full @PLAN22 retrospective entry
  per the @PLAN15 closeout pattern (per-phase summary + bug yield).
- **ROADMAP.md** — remove the @PLAN22 row + active-plan index
  entry; replace with a one-line note in the "Closed" section
  pointing at the finished plan.
- **PLANNING.md** — if a @PLAN22 row exists, mark closed.
- **DESIGN_DECISIONS.md** — update C38 entry (closure capture
  is copy-at-definition) with a note that @PLAN22 ships
  implicit-by-body mutation classification on top of C38.
- **CAVEATS.md** — remove the "closure capture is by-value /
  no mutation" caveat row (now spec'd via @PLAN22).
- **`git mv plans/22-mutable-closures plans/finished/22-mutable-closures`**.
- Update incoming references (TESTING.md, ROADMAP.md, @PLN32
  EVENT_LOOP, @PLN39 TTT v6 row, @PLN6 audience-demo).

## Test surface

`tests/multiplayer_v6.rs` (new) — TTT v6 server tests parallel
to v5's, asserting writable-closure behaviour matches the v5
`.inner` baseline.

Plus integration: `make ci` should pass with all closure_matrix
+ mut_closure_matrix + multiplayer_v5 + multiplayer_v6 cells
green.

## Critical files

| File | Change |
|---|---|
| `default/01_code.loft` | Purity annotations on every user-callable fn |
| `lib/game_protocol/examples/v6_server.loft` (new) | TTT v6 server using writable closures |
| `tests/multiplayer_v6.rs` (new) | TTT v6 multiplayer tests |
| `plans/6-audience-generative-art/server.loft` (new) | Plan-36 server using writable closures |
| `doc/claude/LIFETIME.md` | "Mutating closure capture" subsection |
| `doc/claude/CHANGELOG_TECHNICAL.md` | Plan-22 retrospective entry |
| `doc/claude/ROADMAP.md` | Remove @PLAN22 rows |
| `doc/claude/CAVEATS.md` | Remove closure-capture-by-value caveat |
| `doc/claude/DESIGN_DECISIONS.md` | C38 update |

## Verification

- `tests/multiplayer_v6.rs` 3+ scenarios green (parallel to v5).
- Plan-36 server compiles with writable closures + matches the
  audience-demo expected behaviour.
- All previous-phase cells (Case A, B, C, D, M) still green.
- `cargo test --release --test issues --test wrap --test
  closure_matrix --test mut_closure_matrix` all green.
- `bash scripts/check_doc_drift.sh` reports `clean`.
- `make problems` zero open from @PLAN22 work.
- `make ci` green.

## Bug yield retrospective

Per the @PLAN15 phase 06 lesson recorded in
[@PLAN15 § Phase 06 finding](../15-closure-validation/00-matrix.md):
"aggressive probing of the CLOSED-cell boundary during
closeout is the highest-yield part of the validation arc when
the underlying surface is already mostly clean."

Plan-22 is DIFFERENT — it ships actual production change
(case classifier + lowerings + diagnostic).  Bug yield is
expected to be MUCH higher than @PLAN15's because the surface
is being created, not validated.  Phase 06 retrospective
should report:
- New P-issues filed across phases 01-05
- Specific cells that surfaced bugs
- Whether phase 06 retrofit (TTT v6 + @PLN6) surfaced
  additional bugs the matrix didn't catch — those are the
  closeout-surfaced findings analogous to @P257.

## Risks

| Risk | Mitigation |
|---|---|
| Audit task pulls in scope creep — annotating every default/ fn is hours of work | Time-box: tighten only fns called on captured args in @PLAN22's matrix cells.  Other defaults stay `Unknown` (still correct, just over-conservative).  Full audit deferred to a separate ROADMAP item if it surfaces as a real cost. |
| TTT v6 retrofit surfaces multiplayer-protocol bugs unrelated to closures | File as separate P-issues; don't block phase 06 closeout.  TTT v6 ships when v5's tests pass against v6 + writable-closure semantics; the protocol-correctness work belongs to @PLN39. |
| Plan-36 retrofit competes with @PLN6's own scoping work | Plan-36 hasn't shipped its server.  Phase 06 ships the FIRST server cut.  Plan-36's later work builds on it.  Coordinated via plans/6-audience-generative-art README. |
| Doc closeout lands while ROADMAP cross-refs are stale | `scripts/check_doc_drift.sh` runs in CI; any stale refs surface immediately. |

## Cross-references

- [README § Drivers](README.md#drivers) — TTT v6 + @PLN6 motivation.
- [README § Verification](README.md#verification) — original verification list (subset of phase 06's).
- [@PLN39 TTT § v6](../../39-tic-tac-toe/README.md#tic-tac-toe-v6--ergonomic-retrofit-using-writable-closures) — driver.
- [@PLN6 audience demo](../../6-audience-generative-art) — driver.
- [@PLAN15 phase 06](../15-closure-validation/00-matrix.md) — closeout pattern.
