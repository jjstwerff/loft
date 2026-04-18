# Inline-lift safety — initiative

## Goal

**Eliminate silent memory corruption when a struct-returning call
appears inline in an expression context (format-string
interpolation, chained accessor, assertion) and its argument is not
a plain variable.**

This is broader than P181.  P181 is one manifestation.  The
underlying codegen shortcut —
`src/state/codegen.rs::gen_set_first_ref_call_copy` applies
`OpCopyRecord` with the `0x8000` free-source flag AND the
lock-protection that should spare borrowed-view sources only
covers `Value::Var` args — is language-wide, latent, and hits any
codebase using accessor functions that return field / element
views.

## Why this is an initiative, not a one-off fix

1. **Silent corruption** is the worst bug class — no diagnostic,
   no stack trace pointing at the cause, just downstream symptoms.
   The P181 investigation spent hours on wrong theories before
   instrumenting found the real site.  Every user who hits this
   pattern loses the same hours.
2. **Attacks core idioms.**  `"got {player().team().score}"`,
   `assert(f(o).field == x)`, chained accessors in config / JSON /
   test assertions — bread-and-butter loft.  Baking "hoist into a
   local first, or memory corrupts" into the style guide punctures
   the expressiveness + safety pitch.
3. **Language-wide.**  moros_sim surfaced P181; server-response
   formatting, test assertions, introspection have the same trap.
4. **Stress-tests return-type / dep inference.**  A gate on
   callee return-dep forces validation that accessor-style
   functions tag their returns as "view into arg N".  Either the
   inference is already right (two-line gate) or we uncover a
   second latent issue.  Both outcomes improve the language.
5. **Adjacent call sites likely share the shortcut.**  The
   "locks only `Value::Var` args" pattern is the sort of shortcut
   that gets reused.  An audit will probably flush out P183 /
   P184 next door.

## Phase layout

| File | Phase | Status |
|---|---|---|
| `README.md` | Goal + index (this file) | — |
| `00-p181-diagnostic.md` | Variant inventory, bug site confirmation, fix-direction pick | Open |
| `01-p181-fix.md` | Implement chosen fix + regression fixtures | Not started |
| `02-audit-adjacent-sites.md` | Review codegen for same-shape shortcuts elsewhere | Not started |
| `03-spec.md` | Document the inline-lift invariant as a language commitment | Not started |

Each phase's plan file is opened at the start of its session and
closed when the phase commits.  Phases can produce their own
follow-up plans (e.g. `02-a-some-specific-issue.md`) if the audit
surfaces non-trivial sub-issues.

## Ground rules

- **Instrument before hypothesizing.**  P181 wasted hours on
  slot-allocator / `skip_free` theories.  Only instrumenting the
  actual execution identified the site.  Every subsequent phase
  does the same.
- **Every fix ships with a dedicated regression fixture**
  (`tests/lib/…loft`) + a Rust test in `tests/issues.rs`.  The
  fixture stays on the branch after the fix lands — future
  regression bait.
- **Do not reintroduce issue #120.**  The `0x8000` flag exists
  because callee stores would leak without it.  Any fix that
  relaxes or gates the flag must ship with a matching leak
  regression test that the fix does NOT disable.
- **PROBLEMS.md is the public record.**  Plan files are execution
  scratch.  Keep the PROBLEMS.md entry for each P-ID accurate +
  up-to-date as the initiative progresses.
- **No opcode additions** unless the chosen fix strictly needs
  them.  Prefer gate-in-codegen / IR-shape changes over new ops.

## Non-goals

- Refactoring `src/scopes.rs`'s inline-lift pattern wholesale.
- Changing return-type / dep inference beyond what the gate
  requires.
- Adding new language features.  This is a safety fix for
  existing idioms, not a feature.

## Verification across all phases

At the end of every phase:

1. `lib/moros_sim test` — still 137 passes (including
   `test_edit_at_hex_raise`'s full invariant).
2. `lib/moros_ui test` — still 41 passes.
3. `scripts/find_problems.sh --bg` — full workspace suite 0
   failures.
4. `cargo fmt -- --check` + `cargo clippy --release
   --all-targets -- -D warnings` — clean.
5. Issue #120 leak-regression fixture stays green.
6. Every new bug variant added in Phase 0 has a fixture in
   `tests/lib/p181_*.loft` that FAILS pre-fix and PASSES post-fix.

## Provenance

- Surfaced: moros_sim walkable-editor Step 21 uncovered P181 in
  `lib/moros_sim/tests/picking.loft::test_edit_at_hex_raise`.
- Root cause identified: session on branch `moros_walk_steps_9_10`
  at commit `65a174c` (P181 entry in `doc/claude/PROBLEMS.md`).
- Workaround currently in place: hoist inline struct-returning
  calls into locals before referencing in format strings / chained
  accessors.  Documented in `test_edit_at_hex_raise` source comment.
