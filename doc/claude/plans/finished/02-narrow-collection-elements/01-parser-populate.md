<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 1 — Parser populates forced-size on Type::Integer

**Status:** blocked by Phase 0.

**Goal:** every parser site that resolves a user-typed integer alias
(`i32`, `u8`, `u16`, `u32`, `i8`, `i16`) into `Type::Integer(...)`
sets the forced_size from `Data::forced_size(alias_d_nr)`.

After this phase, the forced-size travels with the Type value.  No
behaviour change yet — consumers still use the bounds heuristic —
but the data is in place for Phase 2.

---

## Where Type::Integer is built from an alias name

The primary path is `src/parser/definitions.rs::parse_type` (line ~1022):

```rust
pub(crate) fn parse_type(&mut self, on_d: u32, type_name: &str, returned: bool) -> Option<Type> {
    let tp_nr = if self.lexer.has_token("::") {
        // ... qualified path ...
    } else {
        self.data.def_nr(type_name)
    };
    // ... resolve tp_nr ...
    // ... eventually returns the def's `returned` type (see typedef.rs::complete_definition)
}
```

`complete_definition` for the `integer` primitive (typedef.rs:31)
returns a clone of `I32` (not populated with forced_size; I32 is
`None`).  User aliases like `i32` are registered via `type` decls
in `default/01_code.loft`; their `returned` field gets populated by
normal parse, and their `forced_size` is set by
`src/parser/definitions.rs:442` when the parser sees `size(N)`.

So when a field reads "i32" as its type name, `parse_type` returns
a `Type::Integer(IntegerSpec::signed32())` (the I32 template via
the Phase 0 helper) — the forced_size on the Type stays `None`
because the template is the plain-integer shape.  The `size(4)`
annotation on `i32` is stored on the alias's definition
(`Data::forced_size(i32_def_nr) = Some(4)`) but doesn't get
stamped onto the Type.

### The fix

`parse_type` needs to look up the alias's forced_size and stamp it
onto the returned Type's `IntegerSpec`:

```rust
// After the Type is resolved — near parse_type's existing Type::Integer path.
if let Type::Integer(mut spec) = tp {
    if let Some(forced) = self.data.forced_size(tp_nr).and_then(NonZeroU8::new) {
        spec.forced_size = Some(forced);
    }
    tp = Type::Integer(spec);
}
```

The critical rule: only stamp from aliases that ACTUALLY carry a
size annotation NARROWER than the default.  Plain `integer` has
`forced_size = Some(8)` from `default/01_code.loft` (`pub type
integer size(8);`) — matching the default 8-byte heuristic.  The
distinction matters only for narrow aliases (i32, i16, u16, u8, i8)
where the heuristic gives 8 but the alias demands 4 / 2 / 1.

**Guard:** skip stamping when `type_name == "integer"` (or when
`forced_size == Some(8)` — functionally equivalent).  This avoids
cluttering every `Type::Integer` with `Some(8)` for no benefit.

---

## Other sites that build Type::Integer from an alias

Grep: `rg 'Type::Integer\(' src/parser/` returns ~15 sites.  Most
are NOT alias resolutions — they're compiler-generated (e.g.
iterator index bounds, generic-inference placeholders).  Those
MUST keep `None` because there's no user-typed alias.

Audit list (from the Phase 0 rg output):
- `src/parser/definitions.rs::parse_type` — PRIMARY alias-resolver site, populate.
- `src/parser/definitions.rs:1655,1662` — pattern match on `Type::Integer(_, _, true)` to detect `not null`; no construction.
- `src/parser/control.rs:1318` — `for` loop index bound; compiler-generated, keep `None`.
- `src/parser/collections.rs:1740` — `FvLong` detection; pattern match only.
- `src/parser/objects.rs` — struct literal type inference; investigate whether alias info flows from the typed field.
- `src/parser/expressions.rs:1143` — vector literal type inference from first element; no alias available, keep `None`.
- `src/parser/vectors.rs` — vector iteration type; typically flows from source type.

The rule: **only populate forced_size when the Type::Integer was
built from a user-typed identifier.**  When the compiler generates
a Type::Integer from scratch (loop bounds, inference defaults), keep
it as `None`.

---

## Consumers in this phase

Only one, and it's already wired: the existing struct-field
binding path at `src/typedef.rs:343-362` today reads
`Attribute.alias_d_nr` to look up `forced_size(alias)`.  That path
is REDUNDANT after Phase 1 (the forced_size now lives on the
Type itself).  **Don't remove it yet** — keep both code paths live
and assert they produce the same result for every struct field.

Add a debug-assertion in `fill_database`:

```rust
Type::Integer(spec) => {
    let field_nullable = nullable && !spec.not_null;
    let alias = data.def(d_nr).attributes[a_nr].alias_d_nr;
    let via_attr = data.forced_size(alias);
    let via_type = spec.forced_size.map(NonZeroU8::get);
    debug_assert_eq!(via_attr, via_type,
        "Phase 1 regression: forced_size via alias_d_nr ({via_attr:?}) \
         != via IntegerSpec.forced_size ({via_type:?}) for field {}.{}",
         data.def(d_nr).name,
         data.attr_name(d_nr, a_nr));
    let s = via_type.or(via_attr).unwrap_or_else(|| a_type.size(field_nullable));
    // ... rest unchanged ...
}
```

When the `debug_assert_eq!` never fires across the full test suite,
you know Phase 1 populated the Type::Integer correctly for every
field.  Then (Phase 2) the `alias_d_nr` attribute can be retired.

---

## Acceptance

- [ ] `parse_type` produces `Type::Integer(s)` with
      `s.forced_size == Some(NonZeroU8::new(4).unwrap())` for `i32`,
      `Some(2)` for `u16` / `i16`, `Some(1)` for `u8` / `i8` —
      verify via a one-off Rust unit test that runs `parse_type`
      on a stub identifier and reads the result.
- [ ] The debug_assert in `fill_database` never fires across
      `cargo test --release --no-fail-fast`.
- [ ] Plain `integer` fields keep `s.forced_size == None`
      (forced-size stays implicit).
- [ ] Full test suite green.
- [ ] `lib/graphics/src/glb.loft::glb_write_indices` workaround
      still in place — no behaviour change yet.

---

## Risks

- **Inference paths**.  If a loft program writes `x = []` and later
  appends `i32` values, the vector's content type infers as... what?
  Today it's whatever the first append produces.  Post-Phase-1,
  that SHOULD still be plain `Type::Integer(..., None)` because no
  alias name was typed.  Verify via `tests/issues.rs::p184_inference`
  — append `1 as i32`, check the inferred content type, confirm
  it's still wide.  If inference accidentally narrows, that's a
  bug to fix here.
- **`as i32` cast output type**.  `x as i32` returns a Type — does
  it populate forced_size?  Probably YES — the cast explicitly
  names `i32` so the forced_size SHOULD travel with it.  Audit
  `src/parser/expressions.rs` for the cast handling.

---

## Rollback

Same as Phase 0: the change is purely additive (one field
population), reverting removes the population and falls back to
the attribute plumbing.  No migration.
