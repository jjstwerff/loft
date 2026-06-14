<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 22 — Enum-scoped variants, prelude shadowing, and `use … as …`

Tracker: [@PLN22](https://github.com/loft-lang/plans/issues/22).  Standard plan
(design-before-build).  Every M+ phase runs the
[design-protocol](../../DESIGN_PROTOCOL.md); every fix runs matrix-first.

## Status (REQUIRED)

**Phase 1 BUILT (2026-06-14) — two enums may share a variant name; matrix green on
both backends; full suite green bar known-environmental (WASM rlib / port-bind).
Phases 2–4 drafted.**

The build landed via the chokepoint-first order.  **Final design (a deliberate
choice with the project owner):** a bare variant used as a VALUE resolves ONLY via
context (match subject, typed decl, typed reassignment / `rec.field`, parameter,
return incl. an `if`-branch tail, struct-field type & default, `==` LHS,
`Enum::`/`Enum.` qualifier).  A bare variant used to DEFINE a new untyped variable
(`x = Red`) is a **hard error — even when the name is currently unique** — so that
adding a second enum with that variant name can never silently re-point an existing
bare assignment (the friction the owner asked to eliminate).  The variant name
stays usable as a TYPE / constructor (`Circle { … }`, `s: Circle`,
`fn f(self: Circle)`), so struct-variant construction is unaffected.

This SUPERSEDES an interim "ambiguity-tracking" build (which preserved no-context
*unique* `s = Idle`) — the owner chose the stricter rule on 2026-06-14.

**Phase 1 design round done (2026-06-13). Phases 2–4 drafted.**
An earlier draft framed this as
"loft has a single flat namespace / `match North` resolves by global lookup."
**That was wrong** — validated against the current tree (probes, 2026-06-13).
loft's *resolution* is already scope- and context-aware; the residual problem is
narrow and lives on the **definition** side.  This plan fixes exactly that.

### What already works (VERIFIED — do not rebuild)

| Capability | Evidence |
|---|---|
| `::` path separator | `consts::E`, `char::from`, `f32::from_bits` in the stdlib |
| Library-qualified names + **auto-`use`** + use-region pre-scan | `lib::fn`, `objects.rs:27` (`get_source`), `mod.rs:4475` |
| `use lib;` (wildcard), `use lib::*;`, `use lib::name;` | `imports.rs` (5 pass); `ImportSpec` at `mod.rs:4479` |
| **Context-directed bare variants** (the "context not namespace" model) — resolve against the known type in match arms, typed locals (`d: Dir = Up`), comparisons (`d == Up`), function args (`pick(Down)`) | probe `g1` → `typed=1 arg=2`; `tests/lib/match_lib_enum_main.loft` (C53) |
| `Enum::Variant` and `lib::Variant` qualification | C53 test: `Status::Yay`, `enumlib::Yay` |

### The actual gaps (VERIFIED)

1. **Variant definitions are keyed by BARE name** (`add_def` keys `(name, source)`,
   `data.rs:3001`), so two enums that share a variant name **panic at parse time**:
   `enum A { Red }` + `enum B { Red }` → `Dual definition of Red` (`data.rs:3003`).
   Resolution is context-aware, but *storage* is flat-per-source. (probe `g2`)
2. **No prelude shadowing**: a user definition that collides with an imported /
   stdlib name is rejected — `enum E` → "conflicts with a constant … pick a
   different name" (the stdlib `E`). (probe `g3`)
3. **No `as` aliasing**: the `use` grammar has no `as` clause — so two libraries
   (or two enums) exporting the same name have no disambiguation escape hatch.
4. **The selective-import syntax reads poorly**: `use lib::a, b, c;` — a flat
   top-level comma list where `b`/`c` don't visually bind to `lib::`.

## Goal (REQUIRED)

Variant names stop being global; a user name may shadow the prelude; collisions
have an `as` escape hatch — *without* regressing the context-directed bare-variant
resolution that already works.

## The load-bearing invariant

> A definition is keyed by its **scope path**, not a bare name: a variant by
> `(enum, variant)`, a top-level name by `(source, name)` with **user defs
> shadowing the prelude**.  Two definitions collide **iff** their full scope
> paths are equal — so two enums may both have `Red`, and a user `enum E` is
> legal.  Resolution is unchanged where it already works (context + `::`).

## Phases

### Phase 1 — enum-scoped variant definitions (M; the core fix)

**Design round DONE** (design-protocol steps 1–4, 2026-06-13); build pending.

**Invariant:** a variant is identified by `(enum, variant_name)` and found *only*
via its enum's member index — never a bare global `def_nr`. Two variants collide
iff their `(enum, name)` paths are equal.

**Probed and confirmed — the mechanism already half-exists.** Resolution already
routes through the enum's members in both places that matter: the value path uses
`def(enr).attr_names.get(name)` (`objects.rs:301`), and the match path falls back
to `children_of(e_nr).find(name)` (`control.rs:1414`, the C53 fix). The variant
*def* legitimately coexists for its substructure (struct-enum fields, discriminant,
codegen) — so this is NOT "variants stop being defs"; it's "variants stop being
**globally keyed**." The collision is one redundant thing: `add_def(variant, …,
EnumValue)` (`definitions.rs:216`) *also* inserts the bare `(name, source)` key
into `def_names` — that key is what panics and what every site tries *first*.

**The fix (3 moves):**
1. **Chokepoint** — `Data::variant_of(enum, name) -> Option<def_nr>`, unifying the
   two existing enum-member lookups (`attr_names.get` + `children_of.find`). Every
   resolution site consults it with the contextual enum.
2. **De-globalize** — `add_def` for `EnumValue` stops inserting the bare
   `(name, source)` key (the def still exists, reachable via the enum). Panic gone;
   two enums share `Red`.
3. Drop the `def_nr(name)`-first try for variants at the resolution sites
   (`control.rs:1411/1451/1536/1548/2987/3017/3028`, `objects.rs:290`,
   `fields.rs:491`); `Enum::V` / `lib::V` resolve the same member path; a
   no-context bare variant stays an error (demand qualification).

**Brittleness (step 2): low by construction.** N ≈ 9 sites, but **omission is
loud, not silent** — once variants aren't globally registered, a missed site gets
`def_nr(variant)==MAX` and fails to resolve (a hard error a test catches), never a
silently-wrong variant. The C53 `children_of` fallback already exists at the main
sites, so de-globalizing mostly *promotes the fallback to primary*.

**Matrix:** two-enums-sharing-a-variant × {match, typed local, `==`, fn arg, `is`,
`Enum::V`, `lib::V`} × both backends; the C53 tests (`tests/lib/match_lib_enum_main.loft`)
stay green; probe `g2` (the panic) flips to passing. (`g3` `enum E` is the
prelude-shadow case → **Phase 2**, not Phase 1.)

**Build spike (2026-06-13) — scope VALIDATED; de-globalize is necessary but NOT
sufficient.** A throwaway spike (the `variant_of` chokepoint + de-globalizing
`add_def` for `EnumValue` + the second-pass re-resolve `definitions.rs:220` →
`variant_of`) confirmed the easy half: the `Dual definition of Red` panic is gone,
two enums share `Red`, and the stdlib still loads. Loud-omission then pinpointed
the residual (exactly as the brittleness argument predicted):

- **Value-position bare variants** (`f(Red)`, `l: Light = Red`, `l == Red`) fail
  with `Unknown variable 'Red'`. They resolve through `parse_var`'s `parent_tp`
  context branch (`objects.rs:301`), which (a) checks `Type::Enum` while the
  typed-declaration path threads `Type::Reference(enum)` (`objects.rs:1673`) — so
  it misses — and (b) is never populated with the expected enum at the
  comparison-RHS and call-arg sites.
- The match / `is` paths already carry the enum (the C53 `children_of` fallback),
  so they are fine.

So Phase 1's true cost = de-globalize + `variant_of` (small) **plus threading the
expected enum to the value-position resolver**: normalize `Reference(enum)`↔`Enum`
at `objects.rs:301`, and set `parent_tp` at the `==` / arg / `return` / field-init
sites. This is a design-protocol step-6 ESSENTIAL divergence (a real domain axis:
value-position variant resolution genuinely needs expected-type plumbing — the
original "resolution already has the context" claim held only for match).

**Build order next session:** (1) `variant_of` + de-globalize + `definitions.rs:220`;
(2) `objects.rs:301` Reference-enum normalization; (3) thread the expected enum
INTO `parse_var` at the value-position sites; (4) verify the matrix + C53 +
`loft_suite`/`native_scripts` both backends. Spike matrix: two enums sharing
`Red` across match / arg / typed-local / `==` / `Enum::V`.

**Confirmed (2026-06-13, 2nd spike):** steps 1+2 *alone* break `loft_suite` —
existing code resolves value-position bare variants — so **step 3 is mandatory,
not feature-only**. And step 2 alone is insufficient: `parent_tp` is **not set to
the expected enum** at the typed-local-declaration / `==`-RHS / call-arg sites
(my Reference-enum branch never fired there), so step 3 is the **bulk** of Phase 1
— locate and thread the expected type *into* `parse_var` at each value-position
parser site (decl initializer, comparison RHS, call arg, `return`, struct-field
init), after which the context branch resolves the variant. A focused
session-sized parser change; do NOT interleave with other work (it leaves the
tree red until step 3 lands). Both spikes reverted; `2026-07` clean.

**Resume artifact (SUPERSEDED — Phase 1 is built):** the old `phase1-step1-2.patch`
encoded the de-globalize-FIRST approach the build-order correction reversed; it was
removed once Phase 1 landed. The as-built record is *Phase 1 — BUILT* above.

**Step 3 exploration (2026-06-13) — the `var_tp` unifying lever is FALSIFIED.**
Tried: add an `expected_tp` param to `parse_var` + a `variant_ctx(name, parent_tp,
expected_tp)` helper (resolve a variant against either context enum), and pass
`var_tp` (the operand's expected type, already threaded through `parse_operators`)
as `expected_tp` from `parse_single` (vectors.rs:400). Result on the four
isolation probes:
- `p_arg` (`classify(Red)`) and `p_ret` (`-> Light { Red }`): **still fail** —
  `var_tp` is NOT the param / return type at those leaves; the call-arg parser
  and the return-body parser don't thread their expected type as `var_tp`.
- `p_decl` (`l: Light = Red`): now resolves but `l == Light::Red` is **false** —
  a **bare-vs-qualified variant-value representation mismatch** (the bare
  `attr_value` form ≠ the qualified `Light::Red` form). A correctness bug, not
  just a resolution gap.

So step 3 is **genuinely per-site**, not one lever: at each value-position site
find where its expected type actually lives and thread it as the variant context
— call arg (param type in `parse_call`), `return` (fn return type at the body
parse), comparison RHS (the LHS `current_type` at `operators.rs:1295`, NOT the
outer `parent_tp`), typed-local decl — **and** reconcile the bare vs `Enum::V`
variant value representations so `==` agrees. Each is its own probe + targeted
edit; watch the silent create-placeholder-var fallback (it turns a missed site
into a wrong result, not a loud error). The `var_tp` attempt was reverted (not in
the patch); the helper idea may still seed step 3, but with the *correct*
per-site type, not `var_tp`.

**BUILD-ORDER CORRECTION (2026-06-13, 3rd probe) — de-globalize LAST, not first.**
The `p_decl` "wrong value" was a symptom of a bigger problem: de-globalization
regresses **qualified `Enum::Variant` too**. `Light::Red == Light::Red` is `true`
on clean `main` but **`false` under steps 1+2** — the qualified path
(`parse_var`'s `::` branch → `parse_constant_value`) *also* relied on the global
`def_nr(variant)`. So de-globalize-FIRST makes the tree red until EVERY
variant-resolution path is rewired at once — it cannot land incrementally (each
"continue" surfaced another broken path: bare value, then qualified, …).

**The right order is CHOKEPOINT-FIRST.** Refactor every variant-resolution path
to route through `variant_of(context_enum, name)` *while variants stay globally
keyed* — each refactor is then non-breaking (variant_of finds the same variant)
and independently landable on a green tree. De-globalize is the **final flip**.
Paths and where each gets its enum context:
- **match arm** — the subject type (the `children_of` fallback already exists).
- **`Enum::Variant` qualified** — the qualifier `name` IS the enum; rewire the
  `::` path so when the qualifier resolves to an enum, the variant is
  `variant_of(qualifier_enum, nm)` (not a global/source lookup).
- **`is`** — the subject type.
- **bare value-position** (arg / decl / `return` / `==`-RHS) — the *expected*
  type, which is NOT threaded today (the per-site work; do these incrementally
  too, each landable while still global).

**Revised build order:** (1) `variant_of` + `definitions.rs:220` [in the patch,
non-breaking]; (2) route match / qualified / `is` through `variant_of` [land each];
(3) thread the expected enum at the bare-value sites [per-site, land each]; (4)
de-globalize `add_def` [the flip — now safe, all paths already route through the
chokepoint]; (5) verify matrix + C53 + suites both backends. The step-1+2 patch's
de-globalize half moves to step 4; its `variant_of` + `definitions.rs:220` halves
stay at step 1.

### Phase 1 — BUILT (2026-06-14)

Built in the chokepoint-first order above.  The boundary matrix (`/tmp/claude/pln22`,
graduated to [`tests/scripts/369-pln22-shared-enum-variants.loft`](../../../../tests/scripts/369-pln22-shared-enum-variants.loft)
for the working cases + a `@EXPECT_ERROR` case in
[`102-expected-errors.loft`](../../../../tests/scripts/102-expected-errors.loft))
is green on **both** backends.

**The rule (owner decision, 2026-06-14):** a bare variant resolves as a VALUE only
from CONTEXT; defining a new untyped variable from a bare variant is a hard error
even when unique.  This SUPERSEDES the interim "ambiguity-tracking" build — which
kept no-context *unique* `s = Idle` working — because that very inference is the
friction the owner wanted gone (add a second enum with the name later → the bare
assignment silently re-points or breaks).  An enum VARIANT keeps a first-wins flat
`(name, source)` key so it stays reachable as a TYPE / constructor, but a bare
variant VALUE never resolves through that key.

**As-built sites:**
- `data.rs` — `variant_of(enum, name)` + `variant_in_source(source, name)` +
  `enums_with_variant(name)` chokepoints; `add_def` keeps EnumValue as a FIRST-wins
  flat key (no panic on a shared name); `rebuild_indices` mirrors it.
- `parser/objects.rs` — `parse_constant_value` resolves a qualified `Enum::Variant`
  via `variant_of(qualifier_enum)` and a `lib::Variant` via `variant_in_source`,
  but DEFERS a bare variant VALUE (no `{`-construction) to context; the flat
  `def_nr` branch excludes `EnumValue`; the context branches + a new
  `Type::Reference(enum)`↔`Type::Enum` branch resolve via `emit_variant_value`
  (the single emitter, plain discriminant or mixed-enum allocation); the resolver's
  error path emits a targeted "bare variant … qualify it as `Enum.X`" diagnostic and
  RECOVERS by resolving against the enum so no placeholder var poisons the second
  pass (without this, one bad site cascades into dozens of "Unknown variable").
- `parser/control.rs` — match arm / or-pattern / `is` route through `variant_of`;
  call-arg sets `enum_hint` from the param enum (BOTH passes); `parse_block` sets
  `enum_hint` from the block's expected type and SAVES/RESTORES it (so every
  `if`-branch tail of a typed-return fn sees it, not just the first).
- `parser/vectors.rs` — `parse_single` seeds the variant context from `var_tp` else
  `enum_hint` whenever `parent_tp` is not itself an enum (covers typed decl, `==`,
  call arg, return, struct-field init); `enum_context` helper.
- `parser/definitions.rs` — enum second-pass re-resolve via `variant_of`; struct
  field DEFAULT (`level: Level = Warning`) sets `enum_hint` from the field type.
- `parser/mod.rs` — `enum_hint` parser field (mirrors `lambda_hint`).

**Known follow-up (branch-internal):** a MIXED struct-enum unit-variant used as a
struct field DEFAULT (`sig: Sig = Idle`) combined with an explicitly-provided
sibling field mis-builds the sibling on both backends.  Narrow combination
(plain-enum defaults — the real lib/logger pattern — and mixed-enum typed locals
both work); deferred, not a Phase-1 blocker.

### Phase 2 — prelude shadowing (S–M)

A user definition in the user source **shadows** a wildcard-imported / stdlib
name of the same key, instead of being rejected. Makes `enum E` (and a user
`PI`, `value`, `name`, …) legal. Removes the "pick a different name" wall.

- Decide the shadow direction explicitly (user def wins in its own source; a
  *qualified* `consts::E` still reaches the shadowed one).

### Phase 3 — `use … as …` aliasing (S–M)

Add the missing alias clause to the `use` grammar (`mod.rs:4475`):

- `use math as m;` → `m::sqrt`.
- `use compass::North as CNorth;` → bare alias, disambiguating a collision.
- This is the escape hatch Phase 1/2 leave open (two libs export `parse`; two
  enums you want both bare).

### Phase 4 — selective-import syntax (S; decision + migration)

Drop the flat top-level comma list (`use lib::a, b, c;`), which reads poorly.

- **Idiom**: one `use lib::name;` per line — greppable, unambiguous, no grouping.
- **Optional grouped form**: `use lib::(a, b, c);` — `()` (loft's existing
  arg-list delimiter) visually binds the names to `lib::`; lighter than Rust's
  `::{}`, which stays reserved for blocks/structs.
- Most code needs neither: `use lib;` (wildcard) or qualified `lib::fn()`
  (auto-`use`) already cover the common cases. Migrate the few `::a, b` sites.

## Open questions

1. **Variant storage shape** — reuse `attr_names` (variants as enum attributes),
   or a dedicated `variant_names` map keyed `(enum_def, name)`? Settle in the
   Phase-1 design round; `attr_names` reuse is the leaner hypothesis.
2. **Bare variant with NO context** (`x = Up`, untyped) — RESOLVED (Phase 1
   build, owner decision): it is ALWAYS a hard error, even when `Up` is currently
   unique → qualify it (`x = Dir.Up` / `x: Dir = Up`).  The original premise
   ("already an error") was inaccurate (it used to infer the enum, and ~16 tests +
   2 doc/lib fixtures relied on that), but the owner chose to make it an error so
   adding a second enum with the variant name can never silently re-point an
   existing bare assignment.  Those fixtures were migrated to the qualified form.
3. **Shadowing scope** — only wildcard/prelude names shadowable, or any imported
   name? Narrow first (prelude only).
4. **1.0 sequencing** — the `(enum, variant)` key change is observable (two-enum
   programs that panic today start compiling); land before the 1.0 stability
   contract.

## Cross-arc dependencies

- **[PACKAGES.md](../../PACKAGES.md)** — Phase 3/4 touch the `use` grammar; keep
  the library-author docs in step.
- **1.0 stability contract** ([ROADMAP.md](../../ROADMAP.md)) — Phase 1 before 1.0.

## See also

- DESIGN_DECISIONS.md — record the selective-import syntax decision (drop flat
  comma list; one-per-line idiom + optional `::(…)`) and the shadow direction.
- C53 (`tests/lib/match_lib_enum_main.loft`) — the existing bare/`Enum::`/`lib::`
  variant-resolution coverage Phase 1 must keep green.
