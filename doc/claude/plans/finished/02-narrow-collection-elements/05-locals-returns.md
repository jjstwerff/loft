<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 5 — Narrow vector storage for locals, parameters, and return types

**Status:** open — **scope corrected**.  The original plan estimated
"mostly a test phase"; actual scope is a narrow-detection helper
plus migrations at ~6 parser call sites.

**Goal:** `vector<i32>`, `vector<u8>` (and `vector<u16>` / `vector<i16>`
once Phase 4b lands) store with narrow stride regardless of WHERE
the type is declared — local variables, function parameters, return
types, vector literals, and compound expressions.  Today only
struct fields narrow, because `typedef.rs::fill_database` runs only
on struct definitions.

---

## Why the original "test phase" framing was wrong

The prior plan assumed forced_size on `Type::Integer` flowing
through `Box<Type>` was sufficient to narrow everywhere, leaving
only verification.  In reality, **narrowing happens at the
`database.vector(content_tp)` call site**, not in the Type carrier.
There are ~6 such sites in `src/parser/` that pass
`data.def(c_nr).known_type` as the content:

```bash
$ rg 'database\.vector\(' src/parser/
src/parser/mod.rs:762:            let tp = self.database.vector(self.data.def(c_nr).known_type);
src/parser/vectors.rs:937:                self.database.vector(self.data.def(ed_nr).known_type)
src/parser/vectors.rs:1413:                    self.database.vector(ek)
src/parser/vectors.rs:1417:            let known = Value::Int(i32::from(self.database.vector(elem_known)));
src/parser/vectors.rs:1459:                        Value::Int(i32::from(self.database.vector(elem_known)))
src/parser/objects.rs:742:            let vec_kt = self.database.vector(elem_kt);
src/parser/objects.rs:1459:                    let vec_tp = self.database.vector(elem_tp);
src/parser/expressions.rs:1153:                self.database.vector(self.data.def(ed_nr).known_type)
src/parser/collections.rs:710:                    let v = self.database.vector(db_tp);
```

For each, the content is the default 8-byte `integer` slot when the
user typed `i32` / `u8` / etc., because the alias's own known_type
isn't populated (only structs run through `fill_database`).

Evidence: `lib/graphics/src/glb.loft`'s natural `glb_idx_buf() ->
vector<i32>` form fails `test_map_export_glb_header` because the
function's local `result: vector<i32> = []` gets wide storage, so
`f += result` emits 8 bytes per element instead of 4.

---

## Design — extract the helper

The narrow-detection logic currently lives inline in
`typedef.rs::fill_database`'s Vector arm (`3b6fd43`):

```rust
let narrow_c_tp = if let Type::Integer(spec) = &*c_type
    && let Some(forced) = spec.forced_size
{
    match forced.get() {
        1 => Some(database.byte(spec.min, false)),
        4 => Some(database.int(spec.min, false)),
        _ => None,
    }
} else {
    None
};
```

Extract as `Data::narrow_vector_content(&self, content: &Type,
database: &mut Stores) -> Option<u16>` — returns `Some(narrow_tp)`
when the content Type warrants a narrow database element type, or
`None` to signal "use the default wide storage".

```rust
// src/data.rs

impl Data {
    /// P184: map a `Type::Integer` content type with `forced_size`
    /// annotation to a narrow database element type.  Returns `None`
    /// for non-narrow cases (no forced_size; or a width that
    /// `IntegerSpec::vector_narrow_width()` rejects — see that
    /// method for the current gate).  Caller falls back to the
    /// default wide `integer` known_type when `None`.
    pub fn narrow_vector_content(
        &self,
        content: &Type,
        database: &mut crate::database::Stores,
    ) -> Option<u16> {
        let spec = if let Type::Integer(spec) = content {
            spec
        } else {
            return None;
        };
        let n = spec.vector_narrow_width()?;
        match n {
            1 => Some(database.byte(spec.min, false)),
            4 => Some(database.int(spec.min, false)),
            // Phase 4b opens the 2-byte arm:
            // 2 => Some(database.short_raw(spec.min, false)),
            _ => None,
        }
    }
}
```

Phase 4b later updates this single helper to include the
`2 => short_raw` arm; callers don't change.

Introduce a thin wrapper `Parser::vector_of(content: &Type) -> u16`
that combines narrow-detection with the default fallback:

```rust
// src/parser/mod.rs

impl Parser {
    /// P184 Phase 5: canonical entry point for building a vector
    /// database type from a content `Type`.  Consults
    /// `Data::narrow_vector_content` first; falls back to the
    /// content's own `known_type` (or the default `integer` slot)
    /// when narrow doesn't apply.  Every `database.vector(...)`
    /// call site in `src/parser/` should route through this helper
    /// so locals, params, returns, and literals all get the same
    /// narrow storage that struct fields get via `fill_database`.
    pub(crate) fn vector_of(&mut self, content: &Type) -> u16 {
        if let Some(narrow) = self.data.narrow_vector_content(content, &mut self.database) {
            return self.database.vector(narrow);
        }
        // Default wide path — look up content's known_type, fill if needed.
        let c_nr = self.data.type_elm(content);
        if c_nr == u32::MAX {
            return self.database.vector(u16::MAX);
        }
        let mut c_tp = self.data.def(c_nr).known_type;
        if c_tp == u16::MAX {
            // Don't call fill_database here — the caller context may
            // not permit recursion.  Return 0 (default integer slot)
            // as a safe fallback; fill_database resolves it later if
            // needed.
            c_tp = 0;
        }
        self.database.vector(c_tp)
    }
}
```

---

## Work breakdown

### Step 1 — Land the helpers

Add `Data::narrow_vector_content` and `Parser::vector_of`.  Zero
behaviour change on their own — nothing calls them yet.

### Step 2 — Migrate `typedef.rs::fill_database` to use the helper

Replace the inline narrow-detection block in the Vector arm with

```rust
let c_tp = data
    .narrow_vector_content(&c_type, database)
    .unwrap_or_else(|| { /* existing fallback: def(c_nr).known_type */ });
```

No behaviour change — the logic is identical, just factored out.
This is the cross-check that the helper produces the same result as
the inline version.  Full test suite must stay green.

### Step 3 — Migrate each `database.vector(...)` site in `src/parser/`

For each of the 6+ sites listed above, replace
`self.database.vector(c_tp)` with `self.vector_of(&content_type)`
where `content_type` is reconstructed from the local context
(usually already available as a `Type` value via `Type::Vector(inner, _)`
pattern match or similar).

Per-site audit — each needs its own assessment:

| Site | Context | Notes |
|---|---|---|
| `parser/mod.rs:762` | Vector literal element type inference | Likely has `content_type: Type` already. |
| `parser/vectors.rs:937` | `iterator` / `for` loop vector | Uses `data.def(ed_nr).known_type`; needs to construct the content Type from `ed_nr` (probably via `data.def(ed_nr).returned.clone()` or similar). |
| `parser/vectors.rs:1413` | Lambda/closure body vector type | Similar. |
| `parser/vectors.rs:1417,1459` | `Value::Int` encoding of db_tp | Compile-time constant; route through `vector_of`. |
| `parser/objects.rs:742` | Struct initialisation vector | Content available as `elem` Type. |
| `parser/objects.rs:1459` | Assignment target vector | Content available. |
| `parser/expressions.rs:1153` | Expression type inference | `data.def(ed_nr).returned`. |
| `parser/collections.rs:710` | Generic collection handling | Inspect context. |

Hold-out strategy: migrate sites one at a time, run tests after
each, verify the diff.  If a site's Type isn't reachable or is
`Type::Unknown(_)`, fall back to the old `database.vector(db_tp)`
call to preserve behaviour — a TODO note in code flags it for a
follow-up audit.

### Step 4 — Variable table integration

Local variable declarations go through `src/variables/mod.rs` which
may compute types via `variables::size(tp, context)`.  Audit
whether local-var storage uses a separate db-tp lookup that also
needs the narrow-detection helper.  Expected: the local's Type is
stored verbatim, and the narrow db_tp is looked up JIT when the
variable is used in an operation that needs it — in which case
migrating the parser sites (Step 3) is sufficient.

### Step 5 — Vector-literal type inference

`[1 as i32, 2 as i32]` infers a vector type from element casts.
The inferred Type should carry the same `IntegerSpec` as the
explicit `vector<i32>` annotation.  Audit
`parser/vectors.rs::parse_vector_literal` (or equivalent) and
confirm the inferred content's `forced_size` is populated.

If inferred types don't carry forced_size, add a follow-up commit
that stamps it from the cast target's alias.

### Step 6 — Function parameter + return types

Function signatures declare Types directly via `parse_type`.  Phase
1 already stamps `forced_size` correctly for aliases there.  Step 3
migrations should cover the parameter-passing and return-value
storage paths automatically (parameters are locals at the callee's
stack; returns are locals at the caller's receive site).

Verify via a test: `fn make() -> vector<i32> { ... }` followed by
`x = make(); assert(x.size == len*4, ...)`.

---

## Test matrix

`tests/issues.rs::p184_*` additions:

| Test                                      | Assertion                                               |
|-------------------------------------------|---------------------------------------------------------|
| `p184_vector_i32_local_var` *(new 5)*     | `x: vector<i32> = []` followed by `f += x` → `len × 4` bytes. |
| `p184_vector_i32_return` *(new 5)*        | `fn make() -> vector<i32>` returns narrow storage.      |
| `p184_vector_i32_param` *(new 5)*         | `fn sum(v: vector<i32>)` sees narrow stride.            |
| `p184_vector_u8_local_var` *(new 5)*      | 1-byte narrow for locals.                               |
| `p184_vector_integer_local_wide_control` *(new 5)* | Plain `vector<integer>` local still wide.      |

Integration:

- Revert `lib/graphics/src/glb.loft`'s `glb_write_indices` workaround
  back to the natural `glb_idx_buf() -> vector<i32>` form.
- `test_map_export_glb_header` must pass.
- `lib/graphics/tests/glb.loft` all pass via `make test-packages`.

---

## Migration strategy — minimise risk

Each step is independently testable.  Recommended order:

1. Step 1 (helpers) + Step 2 (fill_database rewires to helper) —
   **zero behaviour change**, commits as a refactor.
2. Step 3 sites one-at-a-time — commit after each if the site's
   change is substantial, otherwise batch 2-3 low-risk sites per
   commit.
3. Steps 4-6 are verification / audit — likely no-op if the
   infrastructure is correct.

Full suite must stay green between every commit.  Any site where
Type reconstruction is non-obvious gets a TODO + fallback to keep
suite green.

---

## Acceptance

- [ ] `Data::narrow_vector_content` and `Parser::vector_of` exist.
- [ ] `fill_database` Vector arm uses the helper.  Zero test changes.
- [ ] Every `database.vector(...)` call in `src/parser/` either uses
      `vector_of` OR has a documented TODO for follow-up.
- [ ] `glb_idx_buf() -> vector<i32>` natural form works.
      `test_map_export_glb_header` green.
- [ ] `test-packages` for graphics + moros_render green.
- [ ] All new `p184_*` tests green.
- [ ] Full `cargo test --release --no-fail-fast` green.
- [ ] `p184_vector_integer_wide_control` still passes — plain
      integer vectors unchanged.

---

## Risks

- **Hidden db_tp caching.**  Some parser sites may cache db_tp
  lookups in Parser state.  If the narrow lookup produces a new
  db_tp where the cache expects the default, stale cache entries
  could surface as runtime crashes.  Audit via full suite under
  `LOFT_LOG=full` for any affected tests.
- **Scope analysis / lifetime tracking.**  `src/scopes.rs` and
  `src/variables/*.rs` track ref types for store liveness.  Narrow
  vector types with a new db_tp number may need visibility there.
  Expected: since we're reusing `database.vector()` with different
  content_tp, the vector's db_tp is new but its Parts variant is
  still Vector — scope / lifetime code should dispatch the same.
- **Native codegen.**  `src/generation/` emits Rust for vector
  types.  For `vector<i32>` it already emits `vec<i32>`-shaped
  signatures where needed.  Audit for any path that hard-codes
  `vec<i64>` for all integer vectors.

---

## Rollback

Phase 5 is a refactor plus ~6 independent migrations.  Each
migration is a `database.vector(a)` → `self.vector_of(&b)` swap
where `b` is a Type available in scope.  Per-site revert is
mechanical.

Helpers (Step 1) stay in place on rollback — they're unused but
harmless.
