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

After Phase 0 + 1 landed the scope grew — the initiative now
covers four distinct codegen holes plus the audit / spec work.
Each row below is a stand-alone phase with its own plan file and
budget.

| File | Phase | Status |
|---|---|---|
| `README.md` | Goal + index (this file) | — |
| `00-p181-diagnostic.md` | Variant inventory, bug site confirmation, fix-direction pick | **Done** — Option B chosen |
| `01-p181-fix.md` | Phase 1 — gate `0x8000` on callee return-dep (two codegen sites) | **Done-partial** — covers consistent-view callees |
| `01b-return-dep-inference.md` | Phase 1b — teach return-dep inference to UNION over return paths so mixed-return accessors get tagged as borrowed (blocks `map_get_hex`) | **Done** — Reference + Enum arms; Vector deferred |
| `01c-dynamic-dispatch.md` | Phase 1c — CallRef (fn-ref / interface-method) safe default when the callee isn't statically knowable | **Closed** — variants 08 + 21 (fn-ref to mixed-return) both PASS; hidden-ref-arg mechanism protects this path |
| `01d-owned-with-aliasing.md` | Phase 1d — sibling: `Value::Var`-only lock filter for OWNED-return callees that alias an expression arg | **Closed** — variant 09 passes; no reachable shape found |
| `02-audit-adjacent-sites.md` | Phase 2 — audit every `OpCopyRecord` emission + cross-ref with P143/P150/P152/P155 | **Done** — clean, no new bugs; variant 18 probe confirms tuple-destructure path safe |
| `02a-multi-inline-lifts.md` | Phase 2a — the REAL Phase 2a target: TWO or more inline-lift calls to the same (or aliasing) callee in one expression.  Variant 17 crashes `println("a={f(o.x).n} b={f(o.x).n}")`; first call's `0x8000` frees o.x's source, second call walks freed memory.  Narrower than "non-format contexts" (those pass). | **Closed** — variants 17, 20, 22 all PASS (multi-call, Vector multi-branch, chained transitive) |
| `02b-native-codegen-emission.md` | Phase 2b — audit `src/generation/dispatch.rs` direct-emission `OpCopyRecord` sites | **Closed** — subsumed by Phase 2 audit (native sites never set 0x8000) |
| `03-spec.md` | Phase 3 — document the inline-lift + view-vs-owned invariant as a language commitment | **Done** — section added to `LIFETIME.md` |

Each phase's plan file is opened at the start of its session and
closed when the phase commits.  Phases can produce their own
follow-up plans if the audit surfaces non-trivial sub-issues; add
them to this table and number them under the triggering parent
(e.g. `02c-…md`).

## Scope summary — what's in / what's adjacent

**In scope** (all phases above):
- The `0x8000` free-source flag's interaction with borrowed-view
  returns.
- Return-dep inference soundness for the shapes that the gate
  relies on.
- The lock filter around `OpCopyRecord` that protects args from
  being freed by owned-return callees.
- Every inline-lift emission site (format string, condition,
  return, for, assignment) in every codegen path (interpreter +
  native).

**Adjacent but separate** (NOT in scope — would spin out as a new
initiative under `doc/claude/plans/`):
- Source-level syntax for hand-annotating return-dep (`-> Hex[m]`).
  The current plan relies on inference; authors can't override.
- Wholesale refactor of `src/scopes.rs`'s inline-lift pattern.
- Generic "view vs owned" as a first-class type distinction in
  the language spec (Phase 3 touches the invariant; a full type
  theory is a separate effort).

**Out of scope** (may come back as follow-ups years from now):
- New opcodes beyond what each phase strictly needs.
- Changing `OpCopyRecord`'s runtime semantics.
- Adding new language features.

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

## Snippet inventory

Fixtures in `snippets/` probe specific expression shapes.  Status
below is current as of the commit that writes this table; re-run
any snippet to re-confirm.

| # | File | Shape | Current status | Phase |
|---|---|---|---|---|
| 01  | `01_field_access.loft`         | `{f(o.x).n}` format-interp (consistent view)         | **PASS** (was SIGSEGV pre-Phase-1) | 0 / 1 |
| 01b | `01b_without_lift.loft`        | Same body, no inline-lift (control)                   | PASS | 0 |
| 01c | `01c_inline_only.loft`         | Minimal: one inline-lift line                         | **PASS** (was SIGSEGV pre-Phase-1) | 0 / 1 |
| 01d | `01d_var_arg_inline.loft`      | Inline-lift, Var arg (control)                        | PASS | 0 |
| 04  | `04_owned_control.loft`        | Owned-result callee, inline-lift (control)            | PASS | 0 |
| 07  | `07_mixed_return.loft`         | Mixed-return callee (view + owned fallback)           | **PASS** (was SIGSEGV pre-Phase-1b) | 1b |
| 08  | `08_dynamic_dispatch.loft`     | fn-ref call with borrowed-view result                 | PASS | 1c probe (no hole found) |
| 09  | `09_owned_with_aliasing.loft`  | Owned-return callee mutating an expression arg        | PASS | 1d probe (no hole found) |
| 10  | `10_inline_in_condition.loft`  | Single inline-lift in `if` condition                  | PASS | 2a (consistent view) |
| 11  | `11_inline_in_return.loft`     | Single inline-lift in `return expr`                   | PASS | 2a (consistent view) |
| 12  | `12_inline_in_for.loft`        | Single inline-lift as for-iterator                    | PASS | 2a |
| 13  | `13_inline_in_assign.loft`     | Single inline-lift on assignment RHS / `+=`           | PASS | 2a (consistent view) |
| 14  | `14_mixed_return_various_contexts.loft` | Mixed-return in condition / assign (single calls)  | PASS | 2a / 1b interaction |
| 15  | `15_println_format.loft`       | SINGLE mixed-return inline in `println` format        | PASS | 2a |
| 16  | `16_single_call_assert.loft`   | SINGLE mixed-return in assert cond, literal msg       | PASS | 2a |
| 17  | `17_println_two_calls.loft`    | TWO mixed-return inline calls in one `println` fmt    | **PASS** (was SIGSEGV pre-Phase-1b) | 2a — closed by Phase 1b |
| 18  | `18_tuple_destructure.loft`    | Tuple destructure of two struct-returning calls       | PASS | 2 probe (tuple_copy site safe) |
| 19  | `19_vector_mixed_return.loft`  | Vector mixed-return inline in `println` format        | PASS | loose-end probe (Vector arm deferral validated) |
| 20  | `20_vector_two_calls_both_branches.loft` | Vector mixed-return, TWO calls hitting both branches | PASS | loose-end probe (Vector + multi-inline) |
| 21  | `21_fnref_mixed_return.loft`   | fn-ref dispatch to mixed-return, inline + owned fallback | PASS | loose-end probe (1c — fn-ref on mixed return) |
| 22  | `22_chained_mixed_returns.loft` | Chained `get_inner(h.w, i).n` with transitive mixed deps | PASS | loose-end probe (chained + transitive dep propagation) |

Key findings from the inventory:
- Phase 1b (Reference + Enum arms in `parse_return`) closes both
  SIGSEGV shapes.  Variant 07 and 17 both PASS post-fix.
- The Vector arm was deliberately NOT added — doing so promoted
  globals and locals (e.g. `HEIGHT_STEP_LABELS`, `pi_list` in
  `palette_items_for_tool`) to hidden ref-args, breaking callers
  with `Incorrect var __ref_2[65535]`.  See 01b for details.
- The "non-format context" hypothesis is moot — all single-call
  non-format variants pass, and all mixed-return variants pass
  after Phase 1b.  Phase 2a remains open only as a safety net if
  a new multi-call shape ever surfaces.

### Loose-end validation (2026-04-18)

After closing the initiative, four follow-up probes verify the
"likely-closed" phases and the Vector deferral:

- **Variant 19** — A Vector-returning mixed callee (`fn f(c) -> vector<Inner>
  { if ... return c.items; ... empty }`) used inline in `println`.  PASSES.
  Static dump shows the callee's signature is
  `fn n_first_items_or_empty(c, empty) -> vector<ref(Inner)>["empty"]` —
  i.e. `block_result`'s tail-side Vector arm already promoted `empty` to
  a hidden ref arg, giving the return type a non-empty dep `[empty]`.
  The Phase 1 codegen gate fires, `0x8000` clears, no corruption.
  **Conclusion**: Vectors are safe *by construction* via the existing
  hidden-ref-arg mechanism.  The Phase 1b Vector-arm skip is validated.

- **Variant 20** — Same shape, TWO calls in one format string, one hitting
  the view path and one hitting the owned fallback.  PASSES.  Multi-call
  Vector case covered.

- **Variant 21** — fn-ref (`f = first_or_empty; f(h.c, 0)`) to a
  Reference-returning mixed callee, inline in `println` with both view-
  and owned-branch calls.  PASSES.  Dispatch through a fn-ref also relies
  on the hidden-ref-arg mechanism, so no `OpCopyRecord | 0x8000` emission
  path exists.  **Conclusion**: Phase 1c has no reachable corruption class.

- **Variant 22** — chained `get_inner(h.w, i).n` where `get_inner` mid-body
  returns the result of ANOTHER mixed-return call.  Tests transitive dep
  propagation: the inner call's `[w.w -> w]` dep flows through the outer
  function's return type merge.  PASSES.  **Conclusion**: Phase 2a has no
  reachable corruption class for chained / nested mixed-return calls.

## Provenance

- Surfaced: moros_sim walkable-editor Step 21 uncovered P181 in
  `lib/moros_sim/tests/picking.loft::test_edit_at_hex_raise`.
- Root cause identified: session on branch `moros_walk_steps_9_10`
  at commit `65a174c` (P181 entry in `doc/claude/PROBLEMS.md`).
- Workaround currently in place: hoist inline struct-returning
  calls into locals before referencing in format strings / chained
  accessors.  Documented in `test_edit_at_hex_raise` source comment.
