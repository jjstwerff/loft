<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 22 — Enum-scoped variants, prelude shadowing, and `use … as …`

Tracker: [@PLN22](https://github.com/loft-lang/plans/issues/22).  Standard plan
(design-before-build).  Every M+ phase runs the
[design-protocol](../../DESIGN_PROTOCOL.md); every fix runs matrix-first.

## Status (REQUIRED)

**Phase 1 design round done (2026-06-13); build pending. Phases 2–4 drafted.**
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
(2) `objects.rs:301` Reference-enum normalization; (3) thread `parent_tp` at the
value-position sites, leaning on loud-omission to find any missed one; (4) verify
the matrix + C53 + `loft_suite`/`native_scripts` both backends. Spike matrix:
two enums sharing `Red` across match / arg / typed-local / `==` / `Enum::V`.

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
2. **Bare variant with NO context** (`x = Up`, untyped) — already an error today
   in non-match positions; keep requiring `x = Dir::Up`. Confirm no regression to
   the working context cases.
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
