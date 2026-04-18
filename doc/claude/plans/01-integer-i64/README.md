# Integer → i64 + safe arithmetic — initiative

## Goal

**Eliminate the `i32::MIN`-as-null sentinel and the silent wrap/divide-by-zero
failure modes of integer arithmetic.**  Decouple arithmetic width (always
i64) from storage width (unchanged for bounded fields).  Give users a
predictable "either you get the right answer or you get a diagnostic/null at
the contract boundary" arithmetic model across interpreter, native, and
WASM backends.

This is the C54 family from `doc/claude/QUALITY.md § 392-567`.  The design
is extensive and decided at the option-level; execution remains.  This
initiative is the execution plan.

## Why this is an initiative, not a single fix

1. **Cross-backend semantics.**  The fix must hold in the interpreter, the
   native codegen, and the WASM path — three diverging runtimes that have
   historically drifted (P171 was one such drift).  Uniform behaviour
   requires a coordinated landing, not three independent patches.
2. **Pre-decision gate.**  The design offers two semantic alternatives
   (C54.G trap, C54.G′ null-on-overflow).  G′ composes with `??` / `?? return`
   — decisively better UX — but depends on the `not null` enforcement
   surface being water-tight.  That audit must run BEFORE either option
   lands, so the work order matters.
3. **Architectural churn.**  C54.A widens `integer` storage to i64.  That
   bumps the `.loftc` cache format version, requires a `--migrate-i64`
   persisted-DB tool, and replumbs the `Op*Int` arithmetic family onto
   i64 registers.  Not a point fix.
4. **Stdlib-wide sweep.**  C54.B removes `long` + `l` literal suffix.
   Every `default/*.loft`, `lib/*.loft`, and `tests/*.loft` reference
   needs a rewrite.  Migration tooling is mandatory; otherwise the repo
   splits.
5. **Opcode-budget prize.**  C54.E reclaims ~26 duplicate opcodes after A
   lands, freeing room for the O1 superinstruction peephole work that's
   currently deferred indefinitely.  This is a knock-on unlock worth
   planning for explicitly.
6. **Safety-gate impact.**  `RELEASE.md`'s safety gate forbids silent
   corruption / silent data loss.  `i32::MIN → null` is exactly that
   class.  While not a crash, it is a silent wrong-result channel that
   blocks 1.0.0 under the stability gate.

## Phase layout

| File | Phase | Status |
|---|---|---|
| `README.md` | Goal + index (this file) | — |
| `00-null-enforcement-audit.md` | Phase 0 — audit `not null` enforcement surface; decide G vs G′ | **Done** — 7/11 holes found; decision: ship **G-hybrid** (trap default, null inside `??`) |
| `01-checked-arith.md` | Phase 1 — land C54.G-hybrid: trap on bare overflow, null inside `??` so idiom `x = (a*b) ?? default` still works | Not started |
| `02-i64-storage.md` | Phase 2 — C54.A: widen `integer` to i64, opcode replumb, `.loftc` version bump, `--migrate-i64` tool | Not started |
| `03-u32-type.md` | Phase 3 — C54.C: add `u32` as a stdlib type; RGBA use-case probe | Not started |
| `04-deprecate-long.md` | Phase 4 — C54.B: remove `long` + `l` suffix, `--migrate-long` tool, stdlib/tests/lib sweep | Not started |
| `05-opcode-reclamation.md` | Phase 5 — C54.E: delete 26 duplicate `Op*Long` arithmetic opcodes; reclaim for O1 | Not started |
| `06-spec.md` | Phase 6 — document the new arithmetic invariant in LOFT.md + PROBLEMS.md + CAVEATS.md | Not started |

Phase files open at the start of their session and close when the phase
commits.  Phases can produce follow-up plans if the work surfaces
non-trivial sub-issues; add them to this table under the triggering parent
(e.g. `02a-migration-tool-design.md` if the migration tool outgrows its
section of `02-i64-storage.md`).

### Follow-up holes filed by Phase 0

The audit surfaced 7 pre-existing null-enforcement gaps orthogonal to
C54.  They do NOT block any C54 phase; tracked for future enforcement
work:

- **H1** — `not null` field write runtime check (probes 01, 02, 03).
- **H2** — `not null` function parameter runtime check (probes 04, 05).
- **H3** — `-> T not null` return narrowing runtime check (probe 06).
- **H4** — array/hash index null/bounds runtime check (probe 09).

Each opens its own sub-phase only when prioritised
(`07-enforcement-H1-field-writes.md`, etc.).  A future C54.G′
migration (null-on-overflow everywhere) depends on H1-H4 closing.

## Dependency ordering

```
                 ┌─→ 00 audit ─→ decide G vs G′ ─→ 01 checked-arith ┐
                 │                                                   ├─→ 02 i64-storage ─→ 05 opcode-reclamation ─→ 06 spec
                 │                                                   │          │
stdlib-aware ────┤                                                   ├──────────┤
                 │                                                   │          │
                 └───────────────────────────────────────────→ 03 u32 type      └─→ 04 deprecate long
```

- **Phase 0 gates Phase 1**: the G vs G′ decision depends on whether the
  `not null` contract can hold tight enough to make G′ safe.  If the
  audit surfaces holes, G ships first; G′ becomes a later relaxation.
- **Phase 1 lands independently of storage work.**  It's the cheap
  semantic fix — no `.loftc` bump, no migration.  After 1, overflow /
  div-zero cease being silent-wrong-result bugs even on today's 4-byte
  integer storage.
- **Phase 2 (A) depends on Phase 1**: once G/G′ is live, A's remaining
  value is headroom for timestamps / bitmasks / checksums — no longer a
  safety argument.  Splitting keeps the high-risk cache bump isolated.
- **Phase 3 (C, u32)** is independent and can land at any time after 2.
- **Phase 4 (B, deprecate long)** depends on 2 (the widen) and is a
  stdlib sweep.
- **Phase 5 (E, opcode reclamation)** depends on 2 (the widen makes
  `Op*Long` duplicates) and on 4 (the stdlib sweep clears `long`
  references).
- **Phase 6 (spec)** at the end captures the landed invariants.

## Scope summary — what's in / what's adjacent

**In scope** (all phases above):
- Arithmetic semantics on `integer` and `long`: overflow, div/mod by
  zero, `i32::MIN` sentinel.
- Storage layout for `integer`: 8 bytes unbounded, narrowed-on-store for
  `limit(...)` fields.
- Opcode set: `Op*Int` / `Op*Long` arithmetic family, `OpConst*` stream
  encoding, deletion of duplicates.
- `.loftc` cache format version + migration tool.
- Persisted-database format + `--migrate-i64` tool.
- Stdlib (`default/*.loft`) + lib (`lib/*.loft`) + test (`tests/*.loft`)
  sweep for `long` / `l` suffix.
- Three backends: interpreter (`src/fill.rs`), native codegen
  (`src/generation/`), WASM (`src/wasm/` + `codegen_runtime.rs`).
- Cross-backend parity tests.

**Adjacent but separate** (NOT in scope — would spin out as a new
initiative):
- C54.F tagged-null format on small-board targets.  The design mentions
  this as a hanging prerequisite for G′ on 32-bit microcontrollers;
  loft doesn't target those yet, so defer until a concrete board is
  picked.  Open as a later initiative if/when needed.
- Saturating arithmetic as a user-selectable mode.  Explicitly
  rejected in the design (§ 561).
- Auto-widening type system (`i32 + i32 → i64` Python-style).  Design
  § 564 treats C54.A as the capped instance; wider type-level widening
  is a separate conversation.
- C56 `?? return` ergonomics.  Orthogonal to C54; may land separately.

**Out of scope** (may return as follow-ups years from now):
- New arithmetic operators beyond what C54 strictly needs.
- Changing `Type::Integer`'s variant structure beyond the width field.
- Adding new language features.

## Ground rules

- **Every phase ships with `#[ignore]`-free regression fixtures.**  The
  QUALITY.md design lists ~30 tests by name, all currently `#[ignore]`'d.
  Each phase's landing un-ignores its own tests.  The final full-suite
  run must show zero `#[ignore]` among the C54 tests.
- **Cross-backend parity is a hard gate.**  Every semantic test runs
  under interpreter, native, and WASM.  A test green in one backend but
  not the others is a failure, not a partial win.
- **Instrument before hypothesizing.**  Inline-lift-safety's lesson:
  hours lost to wrong theories before instrumenting the actual
  execution.  Each phase opens by constructing a minimal repro and
  capturing a baseline dump before touching compiler code.
- **Do not regress the `RELEASE.md` safety gate.**  Zero tolerance for
  new silent-corruption / silent-data-loss channels introduced by C54
  work.  Every phase's PR runs `scripts/find_problems.sh --bg --wait`
  and ships only on 0 failures.
- **PROBLEMS.md / QUALITY.md stays the public record.**  Plan files are
  execution scratch.  Keep the QUALITY.md C54 entry up to date as the
  initiative progresses; move it to "Closed" in CHANGELOG.md when Phase
  6 commits.
- **Migration tools before breaking changes.**  C54.A and C54.B both
  require migration tooling (persisted DBs, stdlib rewrite).  The
  tooling ships on the SAME commit as the breaking change — no interim
  state where users are on their own.
- **No new opcodes unless the phase strictly needs them.**  Prefer
  gate-in-codegen or IR-shape changes over new ops.  C54.E deletes
  opcodes; the rest of the initiative should net-neutral at worst.

## Verification across all phases

At the end of every phase:

1. Full workspace suite: `./scripts/find_problems.sh --bg --wait` — 0
   failures.
2. `cargo fmt -- --check` + `cargo clippy --release --all-targets
   -- -D warnings` — clean.
3. All phase-specific regression tests un-ignored and passing on
   interpreter + native + WASM.
4. Safety-gate items from `RELEASE.md`: no new SIGSEGV / signal crashes,
   no new panics, no new leak regressions (compare `tests/issues.rs`
   P120 family — all 8 stay green).
5. `lib/moros_sim test` + `lib/moros_ui test` — arithmetic-heavy
   downstream suites stay green.
6. Phase 0 specifically: produce a written decision (G vs G′) and
   commit it to `00-null-enforcement-audit.md` before any code change.

## Provenance

- Design captured: `doc/claude/QUALITY.md § 392-567` (2026-03 to 2026-04).
- Decision tree (G vs G′): QUALITY.md § 479-557.
- Closed-by-decision: C54.D (Rust-style literal suffixes) —
  `doc/claude/DESIGN_DECISIONS.md § C54.D`.
- Initiative opened: 2026-04-18, branch TBD (per-phase branches).
