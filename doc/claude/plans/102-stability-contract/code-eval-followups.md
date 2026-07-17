<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN102 — Code-eval sweep follow-ups (kinks + small-step designs)

> **Status: OPEN designs (2026-07-17).** A code-eval + formal-verification sweep — run after
> #584 (const model + null-flow + gate-2) rebased onto the F2/arc-C stack — FOUND and FIXED, in
> the same session: two silent-corruption/soundness bugs in F2 compound-assign place-once (the
> nonzero-offset double-eval + the value-const bypass, `5deea479`) and a fold-lint false-negative
> (`8f915023`). This doc records the REMAINING kinks it surfaced, each with a small-safe-steps
> design so it can be picked up independently. **K1 is a pre-freeze DECISION** (an error-add — it
> can only land while `CONTRACT_VERSION` is 0); K2–K4 are low-risk cleanups. Method throughout:
> matrix-first, one chokepoint, both backends, gated where it changes an observed value
> ([engineering-rigor](../../STABILITY_METHOD.md), [loft-codegen](../../CODEGEN_METHOD.md)).

Everything else the reviewers checked came back clean: clippy, the hand-maintained IR offset
constants (`DEF_SUPERSEDED` vs #584's `ATTR_CONST_FIELD` — different store records, no collision),
the two byte-identical layout dumps (F9), the `#superseded` store round-trip, and the const-model
+ null-flow behavior.

---

## K1 — enum-variant `const` is NOT enforced (D-const-1) · **PRE-FREEZE DECISION**

**What.** `const` on an enum-VARIANT field is declared, constructed, and read, but its write-once
guarantee never fires. With `enum Shape { Circle { const radius: integer }, … }`, after a
narrowing `if s is Circle { … }`, the direct write `s.radius = 9` is **accepted and mutates on
both backends** — where the same `const` on a struct field is correctly rejected.

**Root cause.** `validate_write` (`src/parser/expressions.rs` ~3616) resolves the written type's
field table only through `Parts::Struct(fields)`; an enum's variant fields live under
`Parts::EnumValue(tag, fields)`, so the `const_field` / `value_const` / key lookup never matches
and the guard silently no-ops for a variant field. (This is the *field-const* Part 2 of
`validate_write`; the *value-const base* Part 1 walks the expression and is unaffected.)

**Why it is a one-way door.** Enforcing `const` here means **adding a compile error** — rejecting
a write that compiles today. Under [compatibility-is-absolute](../../COMPATIBILITY.md) an error-add
can land **pre-freeze or never**: after the `CONTRACT_VERSION` 0→1 flip, a program that mutates an
enum-variant `const` field would newly fail, which is forbidden. So "Phase 3, post-1.0" (the
const-model's current deferral) actually means *unenforceable forever*. `CONTRACT_VERSION` is
still 0 → **the window is open now.** The decision is: enforce it (make `const` uniform), or
deliberately accept that `const` on enum-variant fields is decorative and freeze it that way.
Recommendation: **enforce** — an unenforced `const` is a footgun, and the fix is localized.

**Design — small safe steps.**

0. **Matrix first (throwaway `/tmp` probes, `--interpret`, hand-computed).** One axis per probe;
   prove the instrument can fail (today every write is accepted):
   - variant field × const kind: `const radius` (binding-const) vs `radius: const T` (value-const);
   - value kind: scalar (`integer`) vs collection (`vector<T>` / `text`) variant field;
   - write shape: direct `s.f = v`, compound `s.f += v`, element `s.f[i] = v`, through a value-const
     variant field `s.vc[i] = v`;
   - **and the copy/view boundary** — a pattern-bound local (`if s is Circle { radius }` then
     `radius = 9`) binds a *scalar* field by COPY (B-Copy), so mutating the LOCAL is legitimate and
     must STAY allowed; only the direct `s.radius = …` write on the enum value is the violation.
     Get this line right before touching code — over-reaching into the copied local would be a false
     positive.
   Hand-compute each cell's expected verdict (reject vs allow), on both backends.
1. **One chokepoint.** Extend `validate_write`'s Part-2 field lookup to also accept
   `Parts::EnumValue(_, fields)` beside `Parts::Struct(fields)` — the `const_field` / `mutable`
   attribute checks are identical. VERIFY the attribute index aligns with the `EnumValue` field
   order (the same `attributes()[f_nr]` indexing the struct arm uses); if it does not, that
   alignment is the real fix, not a wider net.
2. **Verify.** Re-run the matrix on BOTH backends: variant-const writes now reject, struct-field
   behavior byte-identical, the copy-local case still allowed. Full suite green.
3. **Conversion set + close.** MEASURE the blast radius (any in-tree or consumer program that
   mutates an enum-variant `const` field — run the suite + the dogfood consumers under the change).
   Add reject cells to `tests/scripts/40-const-fields.loft`; flip **D-const-1** to CLOSED in
   [binding.md](../../formal/binding.md) and drop binding.md's open-count 1→0
   (+ the README/ROADMAP index rows). If MEASURED conversions are non-zero, that is the owner's
   call on whether to still enforce.

**Blocked-on:** the owner's pre-freeze decision (enforce vs accept-decorative). Everything above is
the *enforce* branch.

---

## K2 — the fold-lint's coverage · **AUDITED 2026-07-17 — no live gap**

**Audit result.** `superseded_fold_diagnostics` is called at `src/main.rs:5581`, in the SAME
author-facing-lint block as `warn_copies` / `warn_dead_stores` — consistent wiring, not an anomaly.
The "`make ci` fold-lint" promise ([COMPATIBILITY.md § Folding](../../COMPATIBILITY.md)) IS met: the
unit test `fold_lint_flags_dangling_and_unfolded_superseded` (`src/ir_read.rs`) loads the REAL
stdlib (`parse_dir("default")`) and runs the lint over it, so every in-tree fold (`sum_of→sum`,
`contains→find`) is checked in `cargo test`, and the test's synthetic dangling/unfolded snippets are
the positive control. Any author who compiles their program or lib via `loft <file>` also triggers
it. No code change made — wiring the lint into MORE paths than its sibling author-lints would be the
inconsistency.

**Residual (inert, deferred).** Whether the `loft publish` / lib-build path ALSO runs it is a
consistency nicety, not a live gap — a lib author already triggers it on any CLI compile. If wired
there later, gate it identically to the steer (`LOFT_NO_STEER`) and add a positive control (a
synthetic unfolded `#superseded`) in that path first — silence is evidence only after the control.

---

## K3 — `method_type_prefix` / `display_name` is a 5th, looser copy of the name-mangling parse

**What.** The `t_<LEN><Type>_<method>` method-name mangling is parsed independently in five places
— `api_surface.rs::method_name`, `generation/mod.rs::is_t_param_stub`, `parser/mod.rs`
(`h5_names_a_generic_template`, ~3376 / ~3821), and `data.rs::method_type_prefix` / `display_name`.
The `data.rs` copy is subtly LOOSER: it checks only that *something* follows the parsed
type-name-length (`rest.len() > nd + len`), never that the following byte is the `_` separator the
other four require. Not a live bug (the convention is compiler-enforced), but a fifth divergent
copy of load-bearing string parsing is a drift trap for whoever next touches the naming scheme.

**Design — small safe steps.**

1. **Tighten the outlier (minimal, do first).** Add the `_`-separator check to
   `data.rs::method_type_prefix` / `display_name` so it matches the other four. Behavior-preserving
   for every valid mangled name; it only rejects a malformed near-match that should never have
   matched. Guard with the existing `display_name` / `method_type_prefix` unit tests + a case with
   a colliding non-mangled prefix.
2. **Consolidate (optional, later — byte-identical gate).** Extract ONE helper
   (`Data::parse_method_mangling` or a free fn in `data.rs`) that returns `(type_name, method)` and
   have all five sites call it. This touches codegen-adjacent name parsing, so run it as a
   [loft-codegen Mode-B refactor](../../CODEGEN_METHOD.md): a one-fn-per-site corpus, capture
   `loft introspect` BEFORE, replace ONE site per commit, prove the diff EMPTY each step. Low
   reward, real blast radius — Step 1 alone closes the correctness gap; Step 2 is hygiene.

---

## K5 — general too-few-arguments check (the routing SIGSEGV, fn-typed half FIXED)

**Context.** The `../routing` consumer hit a SIGSEGV inside stdlib `len`, minimised to: **loft has
no too-few-arguments check** (too-MANY is caught: "Too many parameters"). `add_defaults` fills
EVERY missing trailing slot with a type-specific empty/default — vector→empty, scalar→null,
fn-typed→a broken `()`. The fn-typed fill is the crash (and it corrupts the *earlier* args). Likely
regressed with named parameters: `call_with_named` builds `args = [Null; n_params]` and fills gaps
by name/default, which made an internal Null slot normal and lost the required-arg check.

**FIXED (fn-typed half).** A missing FUNCTION-TYPED param with no default is now a parse-time error
("missing argument for parameter '…' of `F` …"). Scoped to fn-typed because it is the crash and a
compiler-promoted return buffer (`ref_return`) is never fn-typed → **zero false positives**. The
error fires before codegen, so the earlier-arg corruption is unreachable too. Regression:
`tests/issues.rs::call_missing_fn_typed_arg_is_rejected`. Suite green.

**OPEN (the general half).** A missing SCALAR still fills null (`f(7)` for `f(a, b: integer)` →
`b = null`, a silent wrong), a missing non-null VECTOR fills empty. Extending the check to them is
blocked by ONE thing: `add_defaults` iterates raw attribute slots, which include compiler-PROMOTED
params (a heap-return's source local promoted to a caller-provided buffer by `ref_return`, e.g.
`tv` in `via_index() -> text { tv: vector<text> = […]; return tv[0] }`). Those are NOT `hidden` and
NOT `__`-named, so a naive "missing non-null no-default slot → error" false-positives on them (it
did, in the first attempt — `tret_bind_forward_ref_pass_stable`).

**Design — small safe steps.**
1. **Find the reliable user-param vs promoted-param signal.** The promotion marks the VARIABLE
   `caller_hidden_buf` (`src/variables/mod.rs`), not the attribute. Either surface that onto the
   attribute, or record the USER-declared param count on the def (before `ref_return` promotion)
   and check the call against THAT, not `attributes(d_nr)`.
2. **Matrix (both backends).** missing scalar / non-null vector / ref / `&`-param / nullable
   (stays optional) / `= default` (stays filled) / named-arg-skipping-a-defaulted-middle-param
   (stays legal) / a return-promoted fn (`via_index`, must NOT error) / method self.
3. **Extend the check** at the `add_defaults` chokepoint (both the positional and named paths
   converge there) using the step-1 signal, gated to pass 2 (forward-ref `&` args lower to Null on
   pass 1).
4. **MEASURE the blast radius** — the first attempt at the general check turned up ~a handful of
   fixtures that omit real args relying on the silent fill (e.g. `greet` in `multiplayer_v2`); each
   is either a genuine arity bug to fix or an intentional omission to make explicit. This is an
   error-add → must land pre-freeze.
5. **Diagnostic position.** `add_defaults` has no call-site position, so the error currently falls
   back to the lexer cursor (usually the call, sometimes the callee body). Thread the call position
   (`call_nr` already has `arg_pos`) if the fall-through proves confusing for real consumers.

## K4 — trivial: `plans/35-match-peg/README.md` status is stale

**What.** `doc/claude/plans/35-match-peg/README.md` still reads "Status: design draft… Phase 0 in
progress" (2026-07-10), though @PLN35 phases 1–7 + PC1–PC5 shipped and the formal spec
([matching.md](../../formal/matching.md), [VERIFICATION.md](../../formal/VERIFICATION.md)) is
already reconciled to SHIPPED. No design needed — a one-line status flip to
"SHIPPED (phases 1–7 + PC1–PC5)". Batch it with the next @PLN35 touch.

---

## See also

- [formal-audit.md](formal-audit.md) — the pre-freeze formal audit; F1–F9 rows.
- [compound-assign-place-once.md](compound-assign-place-once.md) — F2, with the gap this sweep
  found and closed.
- [recommended-idiom-channel.md](recommended-idiom-channel.md) — arc-C (`#superseded` + steer +
  fold-lint), where K2 lives.
- [binding.md § const](../../formal/binding.md) — D-const-1, the K1 deviation.
