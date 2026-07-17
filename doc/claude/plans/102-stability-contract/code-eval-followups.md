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

## K5 — general too-few-arguments check · **DONE (2026-07-17): lenience REMOVED**

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

**The general half is NOT a bug — it is a DESIGN DECISION (2026-07-17).** Attempting the general
check answered both open questions:

- **The promoted-buffer distinction is SOLVED.** A compiler-inserted return slot is reliably
  identified — at the `add_defaults` call site, using only PERSISTED fields — as: `hidden` (a
  `ref_return` out-buffer) OR a `__`-prefixed name (`__retbuf` / `__work_N` / `__tret`) OR an
  attribute index in `self.data.def(d_nr).returned.depend()` (a local promoted to a caller buffer,
  e.g. the returned view `return tv[0]` keeps the local's name `tv`, so only the return-type deps
  name it). The callee's variable table (`caller_hidden_buf`) is EMPTY at the call site (transient),
  so the signal had to be a persisted one — `returned.depend()` is it. With those three exclusions
  the matrix was clean (routing / scalar / vector error; `via_index` / nullable / default /
  named-gap allowed).
- **But it REMOVES a deliberate, tested feature.** A missing SCALAR filling null and a missing
  VECTOR filling empty is loft's **"defaulted-null args"** behaviour — omitting a trailing argument
  fills it null/empty. It is intentional and #307 specifically fixed the frame accounting for it;
  `tests/n2_cdylib.rs::auto_native_text_return_shapes` asserts `greet("x") == "x::"` for
  `greet(a,b,c)`, and multiplayer_v2 fixtures rely on it too. So the general check is a **semantic
  feature removal**, not a bug fix — the routing consumer's "silent wrong" IS this feature, seen as
  a footgun.

**Decision (owner, 2026-07-17): REMOVE the lenience — too-few is now strict.** "defaulted-null args"
was a footgun (a real consumer hit it as a crash AND a silent-wrong) and a surprising default; the
pre-freeze window was the only chance to make it an error. Landed:
- `add_defaults` now errors on a missing slot with no default, non-nullable, and NOT compiler-inserted
  (the three-way exclusion above: `hidden` / `__`-prefixed / in `returned.depend()`). Nullable params
  stay implicitly-optional (fill null); `= expr` defaults still fill.
- Fixtures updated to the strict behaviour: `n2_cdylib::auto_native_text_return_shapes` passes explicit
  empty args (`greet("x","","")`, same result); the `multiplayer_v2` client (both copies —
  integration + game_protocol fixtures) passes `h_shutdown` to `frame_label` (it was in scope,
  matching the two other call sites). Regression `call_missing_fn_typed_arg_is_rejected` retargeted to
  the general message. Full suite green (2990/2992, only the known wasm/socket flakes); zero
  false positives (the promoted-buffer distinction holds across the whole corpus).

Residual (minor): the diagnostic falls back to the lexer position when `arg_pos` is empty (usually the
call, sometimes the callee body) — `add_defaults` has no call-site position. Thread it if a consumer
finds it confusing.

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
