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
work mixed into the architecture content, extract the
follow-ups into a **pointer-plan** under `plans/future/`
that links back to the relevant doc sections (the
`33-native-codegen-followups/` plan is the canonical
example of this shape).

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

**How to apply on close:**
1. Identify which sections of the plan are reference (how
   things work) vs which are closure record (what was done,
   when, with what commits).
2. Move the reference sections into the appropriate
   `doc/claude/*.md` or `lib/<name>/README.md` (creating new
   docs if needed; updating existing ones if they cover
   adjacent material).
3. The `finished/<NN>-<slug>/README.md` keeps only the
   closure record + a "moved to: <path>" pointer for the
   reference content.
4. Grep the codebase for links to `plans/<NN>-<slug>/` or
   `plans/future/<NN>-<slug>/` and rewrite to point at the
   new reference home.  No links should remain to
   `finished/<NN>-<slug>/` after this step (other than
   genuine closure-record-history references — typically
   none).
5. The CLAUDE.md doc index entry that previously pointed at
   the plan is rewritten to point at the new reference home.

This rule applies retroactively too: if you discover an
existing `finished/` plan being linked from current docs,
that's a cleanup signal — the reference content from that
finished plan never moved out, OR the linker hasn't been
updated to the moved-out location.

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

### Closed-work hygiene rule

DEFERRED.md and USER_FACING.md are **open-queue documents**.
When an item closes, its row is **removed entirely** — not
struck-through, not moved to a "recently shipped" subsection,
not retained as historical record.

Closed work already lives in the right places, and duplicating it
across the open queues lets them drift from "actionable" to
"universal log":

- **Git history** — commit message documents what changed and why.
- **Regression test** in `tests/*.rs` — un-ignored when the fix
  lands; permanent behavioural lock-in.
- **Plan README** — the relevant plan's closed-section absorbs
  any architectural lesson learned.
- **PROBLEMS.md** — closed P-id entries stay (file convention)
  for cross-reference history.
- **CHANGELOG.md** — user-facing release notes.

Five places, each the right one for its information shape.  The
grep target `grep -r "Trigger to unpause:" doc/claude/` should
always show only currently-actionable items.

**Sole exception**: USER_FACING.md's "Closed-by-decision" section
is a permanent historical record of explicit non-goals.  Those
stay so a future contributor finds the decision before
re-proposing.  They're orthogonal to the open queue.

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

**Two quality metrics for confidence in the language**, in priority
order:

1. **Velocity of bug closure.** Not "how many bugs do we have"
   (every language has bugs) but "how fast can we close them when
   they appear."  May 2026 baseline: 5-7 P-issues closed per
   focused session, each pinned by a regression test and clean
   under both clippy gates.  This rate is the actual product-
   quality signal — what makes loft trustworthy is that bugs are
   resolved quickly when found.  Two weeks before this baseline,
   the rate was structurally lower because the regression net was
   thinner; the matrix infrastructure (cross-mode harness, lock-in
   tests, hygiene rule, plan-phase discipline) is what turned
   one-off fixes into compounding velocity.
2. **Primary vs. add-on bug location.** Equal-weight to velocity
   because where a bug lives determines its blast radius:
   - **Primary implementation bugs** (parser, type system,
     codegen, runtime) can break entire user projects.  Closing
     them pre-1.0 is foundational quality work.
   - **Add-on feature bugs** (specific stdlib functions, niche
     operators, format-spec corners) usually have viable
     workarounds; impact is bounded.

   The matrix work is currently primary-heavy by design — that's
   exactly the right yield for pre-1.0.  As the matrices close
   their high-yield phases, future bug yield will skew toward
   add-on features.  When that ratio inverts, the language has
   reached a new stability tier — fewer foundational issues, more
   "polish" issues.  That's the natural transition point for
   shifting attention to the showcase track and reducing matrix
   intensity.

May 2026 snapshot — closure breakdown (8 P-issues this session):

| ID | Where | Tier |
|---|---|---|
| P206 | parser core (match-arm separator) | primary |
| T1.8a | native codegen (tuple-of-text return) | primary |
| plan-17 (A) | parser/type-inference (generic-call return propagation) | primary |
| plan-17 (B) | parser/type-inference (bounded-T method dispatch) | primary |
| plan-17 (C) | stdlib `to_text` impls | add-on |
| plan-18 hang | parser core (match arm-arrow recovery) | primary |
| P207 | native codegen (char-tuple-elem comparison) | primary, narrow |
| P208 | native codegen (nested scratch.push wrapping) | primary, narrow |

Seven of eight bugs are primary-implementation work; two of those
are narrow codegen paths users can avoid.  The one add-on item
(stdlib `Printable` impls) was a doc-vs-stdlib mismatch, not a
runtime bug.  This is the right yield mix for pre-1.0.

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

The May 2026 P211 hunt is the canonical example: the original
P-issue was native `yield text`, but the diagnostic probes
surfaced P217 (text accumulator), P218 (format-with-param in
generator body), P219 (vector-for-yield), P220 (`""` in
`vector<text>`) and P221 (server-side BufReader).  All five were
filed in the same commit window; none were lost.  The P217
follow-up hunt then surfaced P222 / P223 (narrower self-concat
shapes) — same rule applied.

## Current initiatives

Maximum 2-3 plans in flight at a time.  When a current plan
closes, promote the next-highest-priority plan from `future/`
(paused work intended to be finished eventually) into this
section.  Long-term direction: drain `future/` to zero by
shipping each plan through to `finished/`.  `deferred/` is a
separate concept — items parked because they need a concrete
trigger to be worth doing at all (may never run).

| Dir | Initiative | Status |
|---|---|---|
| [`07-error-messages/`](07-error-messages/) | Better error messages: every error reaches the user as `file:line:col` + concrete message + source line with caret + optional suggestion.  Spans on IR, pc→source-line table, typed `RuntimeError`, retire the implicit panic-vs-sentinel coin-flip. | Phases 0/1/2/3 shipped (rustc-style renderer + caret + UTF-8/tab + cascade dedup + summary line + `LOFT_ERRORS` env + `--errors` CLI + pc→source-loc on panic + SIGSEGV); phases 4-7 open |
| [`14-tuple-validation/`](14-tuple-validation/) | Validate tuples are fully typed and round-trip-correct across every {element type × storage destination} cell, with mandatory **interp/native byte-identical stdout** under a new cross-mode harness.  Closes T1.8c (struct-ref move semantics) and decides T1.11a (tuples in struct fields).  Phase 00 freezes the matrix and ships the harness; phases 01-05 fill the cells; phase 06 doc-reconciles. | Phase 00 + 01 shipped (16/17 cells PASS, 1 P207-ignored); P206 (parser hang on `->` arm) and T1.8a (tuple-of-text return under `--native`) closed in passing.  Phases 02-06 open. |

## Future initiatives

Plans we intend to do — paused, not abandoned.  Each future
plan is a real commitment to finish; the long-term direction
is to **drain `future/` to zero** by completing each plan into
`finished/`.  This will take considerable time (years, not
months — there are 8+ here), and that is fine.  What matters
is that work flows in one direction: `future/` → current →
`finished/`, never back to "indefinitely parked."

Ready-to-resume — pre-flight already done, design already
drafted.  Promote one into "Current initiatives" when a
current plan closes (max 2-3 in flight).  Distinct from
`deferred/`, which is for plans we **won't do absent a
concrete trigger** (different intent — those may never run).

| Dir | Initiative | Pre-flight status |
|---|---|---|
| [`future/08-repl-and-introspection/`](future/08-repl-and-introspection/) | REPL + interpreter-introspection tool — `loft>` interactive prompt with persistent state plus a clean CLI surface for IR/Rust/slot-table dumps. | Phase 0 + 1 shipped; phases 2-6 open. |
| [`future/15-closure-validation/`](future/15-closure-validation/) | Validate closures (`Type::Function`) round-trip across every {capture composition × storage destination} cell.  Reuses the plan-14 cross-mode harness.  Active risk: closure-DbRef leak in `LIFETIME.md § Function — NOT YET HANDLED`.  Phase 03 (text captures) decides leak fix vs document. | Plan drafted; phase 00 ladder + matrix open.  Pre-flight 2026-05-04 — 6/12 probes fail (50% yield). |
| [`future/16-coroutine-validation/`](future/16-coroutine-validation/) | Validate coroutines (`fn() -> iterator<T>`) round-trip across every {yielded type × drive context} cell.  Reuses the plan-14 cross-mode harness.  Pins `yield from` (CO1.4) deferral via CLOSED cells until 1.1+.  Phase 02 (yielded text) is the active state-machine-lowering risk. | Plan drafted; phase 00 ladder + matrix open.  Pre-flight 2026-05-04 — 0/7 cells passing. |
| [`future/18-match-validation/`](future/18-match-validation/) | Validate `match` expression dispatch across every {subject type × pattern shape} cell.  Reuses the plan-14 cross-mode harness.  Pre-flight (6 tests) found 33% hang rate on or-patterns (`1 \| 2 \| 3 => …`) and `@` bindings — likely sibling of the P206 fix in different `parse_*_match` variants.  Phase 01 closes those hangs; phases 02-05 conditional on yield. | Plan drafted; phase 00 ladder + matrix open. |
| [`future/19-struct-enum-validation/`](future/19-struct-enum-validation/) | Validate struct-enum dispatch (`is`, field capture, match arms, methods) across every {variant payload × dispatch context} cell.  Reuses the plan-14 cross-mode harness.  Pre-flight (5 tests) found 1 bug (20%): method-on-parent-enum-type called via `.method()` on a variant value fails with "Unknown field Variant.method".  Phase 03 closes the C5 method-resolution gap. | Plan drafted; phase 00 ladder + matrix open. |
| [`future/20-collection-validation/`](future/20-collection-validation/) | Validate keyed collections (hash / sorted / index / spacial) across every {collection × operation} cell, with a value-element sub-axis.  Pre-flight (3 tests) hit 67% panic rate — `sorted<>` and `index<>` cleanup both panic with `index out of bounds: the len is 66 but the index is 65535` at `src/database/structures.rs:609` (both backends; basic usage; correct output produced first then panic on scope exit).  Phase 01 closes the cleanup panic; phases 02-05 conditional on yield. | Plan drafted; phase 00 ladder + matrix open. |
| [`future/21-retire-scratch/`](future/21-retire-scratch/) | Retire `stores.scratch` — the per-`Stores` `Vec<String>` lifetime-extension buffer that backs `Str` returns from text-producing natives and codegen wrap sites.  Drained at every statement boundary by `OpClearScratch`; design migrates the remaining producers to the destination-passing / `text_return` machinery already used by user-defined text-returning functions.  Eliminates a class of long-running-program hazards (in-statement unbounded growth, cross-statement escape, P227 family). | Design done (was `doc/claude/RETIRE_SCRATCH.md`).  Phases A / A.5 / B / C drafted; sequencing recommendation in section 6.  Blocked by P227 closure + parser machinery for codegen wrap-site migration (section 5). |
| [`future/22-mutable-closures/`](future/22-mutable-closures/) | Novice-fit closure capture — evolves [C38](../DESIGN_DECISIONS.md#c38--closure-capture-is-copy-at-definition) (closures are copy-at-definition).  Four-case classification (A read-only, B co-scoped, C moved, D aliased rejected) implicit by body, Reference + cell lowerings, multi-position diagnostic for case D.  Companion DISCUSSION.md: alternatives surveyed (A-F), implementation analysis sketch, open questions, design history. | Locked-in spec (was `doc/claude/MUTABLE_CLOSURES.md` + `MUTABLE_CLOSURES_DISCUSSION.md`).  Not yet implemented; three pieces of compiler analysis required (mutation detection, lifetime + liveness on captured bindings, multi-position diagnostics). |
| [`future/23-event-loop/`](future/23-event-loop/) | Prioritised event-loop abstraction (client + server) — bidirectional handlers, library-assigned ids, separate tuning phase, library-assembled streaming, JSON-by-default wire format.  Companions: DISCUSSION.md (open issues, alternatives considered, design history) and PROTOCOL.md (wire format — text-mode v1 shipped, binary-mode v2 designed, server-arbited MAP handshake, encoding modes, streaming reassembly). | Design spec (was `doc/claude/EVENT_LOOP.md` + `EVENT_LOOP_DISCUSSION.md` + `EVENT_PROTOCOL.md`).  Not yet implemented; depends on P213 v4 (Type::Function storage layout limit).  PROTOCOL v1 (text-mode) is implemented and validated by TIC_TAC_TOE v1; v2 (binary-mode) designed only. |
| [`future/24-multiplayer-editor/`](future/24-multiplayer-editor/) | First real-game milestone — multi-client hex editor in the moros stack.  Paint hexes red on click, propagate via WebSocket to all connected clients, snapshot replay on connect.  Validates the multi-client server primitives end-to-end with a non-trivial UX (vs the pure-protocol TIC_TAC_TOE v2/v3/v4 ground layers). | Plan, not yet built (was `doc/claude/MULTIPLAYER_EDITOR.md`).  Consumes TIC_TAC_TOE v2 ground layer (multi-client server primitives) — that arc must land first.  Cross-refs the just-promoted EVENT_LOOP / EVENT_PROTOCOL specs (now lib_plans free, see plans/future/23-event-loop/). |
| [`future/25-native-debug/`](future/25-native-debug/) | GDB / LLDB integration for `--native` builds — DWARF, source maps, plugins.  `loft --native` produces a real ELF / Mach-O / PE binary; this plan makes that binary debuggable with stock GDB or LLDB so users in CLI workflows, Emacs gud, vim termdebug, Eclipse CDT, KDevelop, and any IDE driving GDB through MI get **source-level debugging in `.loft` source**, not in the generated Rust intermediate. | Design (was `doc/claude/NATIVE_DEBUG.md`).  Three independently-shippable phases: NDB.0 (`--native-debug` CLI flag — XS effort, scheduled in ROADMAP.md), NDB.1 (`.loft.map` source map + `loft-gdb.py` / `loft-lldb.py` plugins — M), NDB.2 (DWARF rewrite — MH; stock debuggers need no plugin).  Mentioned in RELEASE_0_8_5_CHECKLIST.md § NDB.0 as a candidate item. |
| [`future/26-match-peg/`](future/26-match-peg/) | L3 PEG-style match patterns — sequence / alternation / optional / repetition / multi-variable capture / multi-pattern arms / anchor-revert backtracking modelled on the existing `Lexer::link()` / `revert()` mechanic.  Extends the base match syntax (LOFT.md § Match expressions).  Cooperates with the regex library ([lib_plans/01-regex](../lib_plans/future/01-regex/)) — regex handles all text matching; MATCH_PEG handles structural / numeric / Unicode-class patterns.  Avoids two text-pattern languages in the codebase. | Design draft (was `doc/claude/MATCH_PEG.md`).  Not yet implemented.  Sub-phases L3.1 (sequence) → L3.2 (alternation) → L3.3 (optional/repetition) → L3.6 (memoised cursor for `iterator<T>`).  L3.5 (string-shaped patterns) explicitly withdrawn — text matching goes through the regex library instead. |
| [`future/27-developer-experience/`](future/27-developer-experience/) | Developer Experience grab-bag — designs for the DX items on the ROADMAP: SH.1 (TextMate grammar), SH.2 (VS Code extension), DX.1 (quick-start `examples/` directory), DX.2 (CI: package + native tests), DX.3 ("Learn loft in 30 minutes" walkthrough), NT.1 (native codegen reliability — completed, kept as historical record).  Originally drafted as "0.8.4 Designs"; several items slipped to 0.8.5 and beyond. | Mixed status (was `doc/claude/DX.md`).  NT.1 completed; SH.1 / SH.2 / DX.1 / DX.3 open for 0.8.5 (heavily referenced from RELEASE_0_8_5_CHECKLIST.md as the source design); DX.2 open.  Stale "0.8.4" version label removed in promotion. |
| [`future/29-server-features/`](future/29-server-features/) | Language features for server / game-client library ergonomics — C55 type aliases, C56 `?? return` null-coalesce-with-early-return, A15 `parallel { }` structured concurrency, I13 iterator protocol (`for msg in ws` via `fn next`), C57 route decorator syntax (`@get` / `@post` / `@ws`).  Each item lifts an ergonomics gap that the upcoming server / game-client libraries would otherwise paper over with explicit boilerplate. | Design (was `doc/claude/SERVER_FEATURES.md`).  Items C57 + I13 are roadmap-tracked for 1.1+; C55 / C56 / A15 are core-language features with their own ROADMAP rows.  Prerequisite for `lib_plans/` server + game-client plan promotions (still in `doc/claude/`). |
| [`future/30-sorted-slice/`](future/30-sorted-slice/) | A8 — slicing, open-ended ranges, partial-key match, and comprehensions on `sorted<>` / `index<>` collections.  Brings these collection types up to feature parity with `vector<>` for range / open-ended access patterns.  Today: only exact key lookup `col[k]` works for `sorted`; some closed ranges work for `index`. | Design (was `doc/claude/SORTED_SLICE.md`).  Current State table at top of plan shows mostly ✗ — genuine future work.  **Notable runtime affordance**: `key_compare` in `src/keys.rs` already uses `zip(key.iter(), keys.iter())` so partial-key arrays compare correctly at runtime — only parser changes needed for the partial-key-match phase. |
| [`future/32-tic-tac-toe/`](future/32-tic-tac-toe/) | Protocol-validation vehicle — v1 SHIPPED (text-mode protocol verifier; `tictactoe_server.loft` lives in tree).  v2/v3/v4 are designed protocol-only ground layers: v2 (two clients, mutual spectatorship), v3 (server delivers the client + browser layer), v4 (client-uploaded scripts, server-side compile, hot WASM swap).  Each ground layer verified end-to-end by the smallest possible text-mode programs.  **Deliberate scope ceiling**: visual / playable tic-tac-toe is deferred indefinitely — never going to ship; real-game UX lives in plans/future/24-multiplayer-editor/. | Mixed status (was `doc/claude/TIC_TAC_TOE.md`).  Doc-internal status block + the deliberate ceiling spell out what the protocol validation deliverable is for each vN: a text-mode test program, not a graphical game.  Sibling to plans/future/24-multiplayer-editor/ which consumes the v2 ground layer. |
| [`future/33-native-codegen-followups/`](future/33-native-codegen-followups/) | Open follow-up items extracted from NATIVE.md — `--native` is shipped (CI-gated, 108/108 native tests pass), so NATIVE.md stays at doc root as architecture/history reference; this plan tracks what's still TO BUILD.  Items: N8b.3 (`yield from` coroutine delegation — feature work), N8c.1+N8c.2 (generic-instantiation text-return audit + fix — likely overlaps shipped plan-17 closures, needs verification), N20a+N20b (`fill.rs` auto-gen repair — small).  N10 in NATIVE.md is stale and should be pruned when this plan ships. | Pointer-plan (each row links to the relevant NATIVE.md section).  Created 2026-05-09 to make the open work visible in the `plans/future/` index without duplicating design content from NATIVE.md.  Suggested order: N8c.1 audit (likely close-on-arrival) → N20a/b → N8b.3 yield-from → NATIVE.md cleanup. |

## Deferred initiatives

Plans that are well-described but intentionally paused — picked up
only when a concrete trigger arrives.  Distinct from `future/`,
which is "we will do this, just not yet."  Deferred items are
"we won't do this unless something specific changes."

| Dir | Initiative | Trigger to unpause |
|---|---|---|
| [`deferred/10-scope-exit-emission/`](deferred/10-scope-exit-emission/) | Scope-exit gate simplification.  Drops the `(dep.is_empty() \|\| is_work_ref) &&` prefix from `src/scopes.rs:1053` so cleanup emission no longer depends on dep-tracker precision.  Pure cognitive-clarity win; no P-issue closes here.  Originally framed as a P203 fix — that framing turned out wrong (P203 is a template double-sub bug). | A bug in this gate's territory, dep-tracking maintenance, or contributor interest. |
| [`deferred/12-codegen-simplifications/`](deferred/12-codegen-simplifications/) | Tier 1 (walker audit + forwarding-smoke retire) shipped on branch `plan-12-codegen-simplifications` (commits `c0c27e5` / `d446e5d`).  Tier 2 (dispatch arm migration phases 03-05) parked here.  Move-the-furniture refactor with no driving bug, no waiting feature, no performance gain.  Real value is "plan 13's preamble." | Same trigger set as plan 13: 3+ template-path bugs, OR major codegen evolution forcing ≥50 Op-annotation touches, OR contributor appetite.  Plan 12 Tier 2 only earns its keep if plan 13 unpauses. |
| [`deferred/13-rust-template-migration/`](deferred/13-rust-template-migration/) | Migrate ~200 `#rust"..."` template annotations in `default/*.loft` to hand-written runtime fns + registered emitters.  Single source of truth for Op emission.  Retires `output_call_template` and `Value::RawExpr`.  2-3 weeks of focused work. | 3+ template-path bugs accumulating, OR major codegen evolution that forces touching ≥50 Op annotations, OR contributor appetite for a multi-week structural refactor.  Plan 12 Tier 2 (phases 03-05) must land first — without it the migration target shape isn't uniform. |
| [`deferred/28-const-store/`](deferred/28-const-store/) | Constant store — Phase A (P127 vector-constants fix: heap-backed const store, `OpConstRef` opcode, `text_code` retired) and Phase D (`.loftc` bytecode cache via `src/cache.rs`, SHA-256 keyed) are SHIPPED.  Two deferred-tail phases remain: Phase B (mmap cache loading — overhead exceeds memcpy savings at current 5-10 KB cache size); Phase C (WASM pre-compiled stdlib — needs `Data` struct serialisation across 130+ public members + recursive enums; MH effort).  Was `doc/claude/CONST_STORE.md`. | Phase B: Phase C lands a large embedded stdlib cache (mmap becomes worthwhile only above some size threshold).  Phase C: contributor appetite for multi-week `Data` serialization work, OR demonstrated need for sub-100ms WASM cold-start past what include_bytes! + parse achieves.  Both phases roadmap-tracked (CS.B / CS.C1-C3). |

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
| `finished/06-typed-par/` | Simple typed `par`: collapse the 7-variant runtime + 3-fn native dispatch into one store-stitch path; "everything is a store".  Doubled as a structured bug-hunt of the type-system × native-codegen × parallel-runtime intersection — 18+ P-issues filed/closed during the work (P188–P236 family). | 2026-05-09 — A1–A7 + A5b + A8.b + A11 shipped via `f974770` + `15a7aab` + `bcac52f`.  A8 deferred-with-audit (structural divergence in `src/parallel.rs` dispatchers); A9 superseded by A4 (light path retired); A10 (browser parallel) out-of-scope.  Ignored par canaries 8 → 1.  Closure record in `finished/06-typed-par/ARC.md`. |
| `finished/17-template-validation/` | Validate bounded generics / interfaces (`<T: Bound>`) across every {T-parameter usage × bound shape} cell.  Reuses the plan-14 cross-mode harness.  Pre-flight predicted 60% bug rate; final yield was 7 P-issues across 6 phases — close to predicted.  All filed bugs share one root-cause family: generic-fn codegen emits DbRef-shaped ops for T-typed values without substituting T's concrete type at monomorphisation. | 2026-05-09 — 26 PASS cells in `tests/template_matrix.rs` covering U1/U2/U3/U6/U7/U8 × B0/B1.O/B1.E/B1.A/B1.P/B2/B3/B4.  P-issues filed: P237 / P238 / P239 / P240 / P241 / P242 / P243.  Three closed in follow-up sweep: P242 (commit `2d23b13`), P238 (commit `2c96683`), P237 (commit `a05db00`).  P239 / P240 / P241 / P243 still open.  2 feature gaps confirmed (B5 two-T generics, U4 generic structs).  Closeout commits: ad854e4 + adcc6e6 + 61cbf06 + 80d6b49 + 42f9739 + 4854b2d + 2d23b13 + 2c96683 + a05db00. |
| `finished/31-html-export/` | W1.1 — single-file HTML export.  All 10 implementation steps shipped: rlib for `wasm32-unknown-unknown`, browser-WASM compile pipeline, cdylib entry-point codegen, HTML loader, `println` via WASM import, GL functions as WASM imports, frame yield for game loops, HTML assembly with GL bridge, `wasm-opt` integration, end-to-end test (`tests/html_wasm.rs`).  Doc kept as historical implementation record + reading-order guide for the export pipeline. | Pre-existing closure (was `doc/claude/HTML_EXPORT.md`).  Promoted to `finished/` 2026-05-09 after pre-promotion audit confirmed all 10 steps shipped per CHANGELOG + tree evidence (`/usr/local/share/loft/wasm32-unknown-unknown/libloft.rlib`, `tests/html_wasm.rs` 460 lines, Makefile targets `make game` / `make wasm-html-test`).  Subsequent W1.x evolutions (W1.18 Worker Threads etc.) tracked separately in CHANGELOG_TECHNICAL.md. |

## One-off plans elsewhere

Per-session ephemeral plans not tied to a multi-phase initiative
live under `~/.claude/plans/` (flat, generated filenames).  Those
are not committed to the repo.
