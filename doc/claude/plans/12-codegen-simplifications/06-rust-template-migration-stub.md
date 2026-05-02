# Phase 06 — `#rust"..."` template migration (stub for plan-13)

**Status:** STUB — do not implement under plan-12.

**Closes (planned):** unifies the template-substitution path
and the registry-based emitter path.  Removes the dual
representation that makes Op emission have two sources of
truth.

**Tier:** 3 (deep refactor; deferred to plan-13)

**Estimated cost:** ~2-3 weeks of focused work.

## Why this is a separate plan

Plan-12 covers Tier 1 + Tier 2 simplifications:
- Tier 1 (phase 01-02): walker audit + dead-weight removal
- Tier 2 (phase 03-05): dispatch arm migration + narrow_int_cast split

The `#rust"..."` template migration is structurally different:
- ~200 Op annotations in `default/01_code.loft` (and a smaller
  set in `default/02_images.loft` and `default/03_text.loft`)
- Each annotation becomes a runtime fn in `codegen_runtime.rs`
  + a registered emitter (or a generated forwarder)
- Touches the loft-side library files, not just src/
- Affects every native-emitted program

The work is well-defined but LARGE.  Trying to fold it into
plan-12 would balloon plan-12 beyond a sensible PR boundary.
Plan-13 is the right home.

## Sketch (for plan-13's eventual design)

### Phase 13.1 — Catalog all `#rust` annotations

Survey `default/*.loft` for every `#rust"..."` annotation.
Group by:
- Op family (arithmetic / text / vector / hash / etc.)
- Annotation complexity (single-substitution vs multi-line vs
  conditional-branch)
- Whether the annotation calls a helper fn (`ops::op_add_int`,
  `vector::get_vector`, etc.) or inlines logic

### Phase 13.2 — Auto-translate trivial annotations

The simple annotations (single-substitution to a helper fn) can
auto-migrate via a script:
- Generate a stub runtime fn calling the existing helper.
- Register a passthrough emitter.
- Run byte-identical baseline.

### Phase 13.3 — Hand-port complex annotations

Multi-line annotations and conditional-branch ones (e.g. the
let-bind-on-repeat ones from plan-09 phase 00 step 0.7b) need
hand-ported emitters.

### Phase 13.4 — Remove `#rust` parsing path

Once all annotations are migrated, the `#rust"..."` parsing
code in `src/parser/` and the template-substitution code in
`src/generation/calls.rs::output_call_template` can be retired.

### Phase 13.5 — Validate parity

Native suite stays at 95/95 (or whatever the current PR-ready
floor is).  Per zero-regression rule.

## Why NOT plan-12

Plan-12's goal is "post-09 simplifications that fit one PR or
two."  Plan-13 is its own multi-week effort.  Mixing them would:
- Make plan-12's review surface unmanageable.
- Block plan-12's smaller wins (Tier 1 + 2) on plan-13's
  longer timeline.
- Risk merge conflicts with future Op additions during the
  multi-week migration window.

## When to open plan-13

Two preconditions:
1. Plan-12 merges cleanly (Tier 1 + 2 land, dispatch.rs is
   close to empty, narrow_int_cast is split).
2. There's appetite for a multi-week structural refactor.

If precondition 2 doesn't hold, plan-13 stays a stub
indefinitely.  The `#rust"..."` path is functional today; it's
just not ideal.

## Memory candidate

If plan-13 ever opens, save:

```
project_template_emitter_unification.md — Plan-13 unifies the
`#rust"..."` template path with the registered emitter path.
~200 Op annotations migrate to runtime fns + emitters.  Single
source of truth for Op emission.  Multi-week effort; opens
after plan-12 merges.
```

This stub itself is documentation — no code.
