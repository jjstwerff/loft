# Phase 1b — return-dep inference for mixed-return callees

Status: **open** (follow-up to Phase 1).

## Problem

The Phase 1 gate on `0x8000` only fires when the callee's
`returned` type carries a non-empty `dep` chain.  Return-dep
inference populates this correctly for callees whose body has
ONE return path that's a view:

```loft
fn first_inner(c: Container) -> Inner {
  c.items[0]           // always a view
}
// Inferred as: Inner["c"]  ← dep non-empty, gate fires, safe.
```

But it FAILS for callees with mixed return paths:

```loft
pub fn map_get_hex(m: Map, q: integer, r: integer, cy: integer) -> Hex {
  for gh_c in m.m_chunks {
    if ... {
      return gh_c.ck_hexes[idx];   // view into m
    }
  }
  Hex {}                             // owned-fresh fallback
}
// Inferred as: Hex  ← dep EMPTY, gate misses, still crashes.
```

Both returns go through the same return slot, but one is a view
and one is owned.  The inferencer appears to take the
intersection (both paths must share the dep) instead of the union
(any path with a dep tags the return as borrowed).

Demonstrated by:
- Variant 01 (consistent-view `first_inner`) → correctly tagged,
  Phase 1 gate fires, fixture passes.
- `lib/moros_sim/src/moros_map.loft::map_get_hex` → mixed return,
  tagged `Hex`, Phase 1 gate misses, `test_edit_at_hex_raise`
  still needs the hoist-to-local workaround.

## Goal

Make return-dep inference conservative: if ANY return path
produces a borrowed-view type, the declared return type's `dep`
chain should contain (at least) that view's deps.  Over-tagging
is fine — tagging a truly-owned return as borrowed merely leaves
the caller responsible for freeing (the caller already is, via
scope analysis).

## Fix path candidates

1. **Inference-level union.**  Find the pass that resolves
   declared-without-annotation return types.  When walking
   `return expr` statements, take `expr.type().depend()` at each
   return and merge into the function's return dep.  Keep the
   existing intersection as the "the return is owned" signal
   only if EVERY path is owned.
2. **Syntax-level explicit annotation.**  Provide a way to write
   `-> Hex[m]` in source so authors of accessor-style functions
   can hand-annotate.  Rename or extend an existing annotation
   syntax; don't invent a new one unless needed.  This is a
   language-surface change and probably belongs in Phase 3 (spec)
   rather than here.
3. **Callee-side audit + fix.**  For each known accessor in the
   ecosystem with mixed returns, rewrite the body so every return
   is a view (e.g. pre-allocate an "empty Hex" in a well-known
   location of the map and return a view to IT for the not-found
   case).  Invasive per-call-site work, doesn't fix the core
   compiler issue.

Prefer Option 1.  Option 2 is a possible follow-up that reduces
reliance on inference for tricky cases.  Option 3 is a last resort.

## Suspected inference site

Look for the pass that computes `def.returned` after parsing a
function body without an explicit return-type dep annotation.
Candidates:

- `src/parser/definitions.rs` — function-parsing, type capture.
- `src/typedef.rs` / `src/parser/mod.rs::actual_types` — type
  resolution passes.
- `src/scopes.rs` — scope analysis may adjust types with inferred
  deps.

Use `LOFT_LOG=static` on variant 01 and map_get_hex side-by-side;
the render-level display of `fn <name> … -> X[dep]` tells us what
the pass has already inferred.  If the render shows `-> Hex` for
map_get_hex even when the body contains `return gh_c.ck_hexes[idx]`,
the inference pass isn't propagating the dep for that return — find
the code that walks return statements and fix it.

## Variants to add

Create these in `snippets/` and promote to `tests/lib/` post-fix:

- `07_mixed_return.loft` — callee with `if ... return view; else
  return owned` shape.  Pre-fix: inline-lift crashes.  Post-fix:
  passes.
- `08_all_owned_return.loft` — control: every path is owned.
  Gate doesn't fire, flag stays on, no regression.
- `09_all_view_return.loft` — control: every path is a view (this
  is variant 01's shape, already covered).

## Success criteria

1. `lib/moros_sim/src/moros_map.loft::map_get_hex`'s return shows
   `-> Hex[m]` in the LOFT_LOG=static dump after the fix.
2. `test_edit_at_hex_raise` in `picking.loft` can use the
   natural inline form: `assert(map_get_hex(e.es_map, 3, 2, 0).h_height
   == 4, "... {map_get_hex(…).h_height}")` — without the hoist
   workaround.
3. The `07_mixed_return.loft` regression passes.
4. No new failures in the full workspace suite.

## Non-goals

- Eliminating the need for `0x8000` entirely.  Owned-return
  callees still use it.
- Adding new syntax for return-dep.  Option 2 is deferred until
  the inference-based fix proves insufficient.

## Budget

90-120 minutes for the inference fix + verification.  Bail out if
the inference pass turns out to be more intricate than it looks;
in that case, fall back to Option 3 (patch `map_get_hex` body to
return a view in the not-found case too — e.g. store an "empty
hex" sentinel in each chunk and return a view to it).
