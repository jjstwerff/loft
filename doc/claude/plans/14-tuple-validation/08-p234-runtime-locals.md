<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 08 — P234 runtime: LOCAL tuple-with-lifetime-concern variables

**Status: open (follow-up to Phase 07; independently executable)**

## Goal

Extend the synthetic-struct routing established by [Phase 07](07-p234-runtime.md)
to LOCAL tuple variables of types containing lifetime-bearing
elements.  After this phase: any tuple containing a Text /
Reference / Vector / Enum-struct / keyed collection / RefVar (or
recursively a tuple-of-those) is stored as a `Reference(__tuple<…>)`
**regardless of where it lives** — function-return slot (Phase 07),
local var (this phase), destructure temp.  Pure-value tuples
(`(integer, integer)` etc.) keep the Rust tuple ABI for
performance.

## Why this phase

1. **Uniformity.**  One routing decision (`has_lifetime_concern`)
   covers every tuple destination.  No "function returns work but
   locals are different" mental load.
2. **Code retirement.**  ~150 LoC of T1.8a's text-tuple
   special-case machinery in `src/generation/dispatch.rs::output_set`
   becomes dead and can be removed.
3. **Independent.**  Doesn't depend on any other Plan-14 phase or
   Plan-06 ARC step beyond Phase 07 (already shipped).  Can be
   prioritised, deferred, or skipped at will.

## Honest trade-off

This is a **refactor for uniformity, not a bug fix.**

LOCAL tuple-with-lifetime-concern is NOT broken today.  Verified
on `quality-pass` after Phase 07 closure (2026-05-08):

```loft
fn main() {
    p = Point { x: 10, y: 20 };
    r: (Point, integer) = (p, 5);
    print("{r.0.x}, {r.1}\n");   // → "10, 5" on both backends
}
```

The motivation is simpler code, not user-visible behavior change.

| | Benefit | Cost |
|---|---|---|
| Code | ~150 LoC retired (T1.8a tuple-text in `output_set`); single tuple ABI mental model | A few sites need centralised helper updates |
| Performance | Pure-value tuples unchanged | One heap allocation per local lifetime-bearing tuple (negligible — these are rare) |
| Risk | Low — extends an already-proven pattern | LOCAL tuple-text Plan-14 D1 tests (`e2_d1_text_*_local`) re-route through new path; verify byte-equality |

## Scope

**IN:**

- Local var declarations with explicit tuple type
  (`r: (Point, integer) = …`)
- Local var type inference from tuple expression (`r = (p, 5)`)
- Destructure temp vars (`(a, b) = make()` synthesizes a temp)
- Match subject of tuple type (`match (a, b) { … }`)

**OUT (separate phase if motivated):**

- **Function parameters of tuple type** (`fn f(p: (Point, integer))`).
  Changing parameter ABI affects every call site;
  `tuple_matrix::e2_d2_arg_text_text` and friends would need
  caller-side coordination.  Deferred to a hypothetical Phase 09
  if a use case surfaces.
- **Inline tuple expressions used as values without storage**
  (`(x, y).0`).  No storage to route.
- **Pure-value local tuples**.  Keep Rust ABI for performance.

## Design

### Step 1 — Centralised var-declaration helper

Add `Parser::add_local_with_tuple_rewrite(name, type_def) -> u16`
in `src/parser/mod.rs` (or near the existing `create_var` /
`create_unique` helpers).  Behaviour:

1. Check `data::has_lifetime_concern(type_def)`.
2. If matched AND `type_def` is `Type::Tuple(elems)`, replace it
   with `Type::Reference(self.data.tuple_def(&mut self.lexer, elems), Vec::new())`.
3. Call `self.vars.add_variable(name, &rewritten, &mut self.lexer)`.

Skip the rewrite when `self.data.def_type(self.context) ==
DefType::Generic` — same gate Phase 07's
`parse_function` rewrite uses.

### Step 2 — Update var-declaration call sites

Per the survey done before drafting this plan, the affected
sites are:

| File | Line | What |
|---|---|---|
| `src/parser/expressions.rs` | ~1053 | `change_var_type` for explicit `r: T = …` |
| `src/parser/expressions.rs` | ~1131-1138 | destructure temp + per-binder vars |
| `src/parser/mod.rs` | ~2074 | `change_var_type` for inferred tuple types |
| `src/parser/control.rs` | ~1820, 1834, 1985 | match binding contexts |

Each site: route through the new helper instead of calling
`add_variable` / `change_var_type` directly.

NOT updated:

- `src/parser/definitions.rs:1054` — function parameters.  Out of
  scope.
- `src/parser/control.rs:1938` — `create_unique("match_tuple", subject_type)`
  — handled transitively because `subject_type` is rewritten by
  Phase 07 when it comes from a function call.

### Step 3 — Tuple-literal RHS conversion in `convert()`

When a `Value::Tuple([elem_0, elem_1, …])` is being assigned to
a `Reference(__tuple<…>)` destination (e.g. after Step 1 rewrote
the destination var's type), the convert path should auto-rewrite
to synthetic-struct construction.  Mirrors what
`rewrite_tail_tuple_to_synthetic_struct` does for function tail.

Add a new arm in `src/parser/mod.rs::convert` (around line 669):

```rust
if let (Type::Tuple(_), Type::Reference(d_nr, _)) = (is_type, should)
    && self.data.def(*d_nr).name.starts_with("__tuple<")
    && let Value::Tuple(_) = code.unspan()
{
    self.rewrite_tail_tuple_to_synthetic_struct(*d_nr, code);
    return true;
}
```

This makes `Set(var, Tuple)` → `Set(var, synthetic_struct_construction)`
a one-line conversion at the parser level.  All downstream Set
codegen treats `var` as a Reference and uses the existing
struct-set machinery.

### Step 4 — Tuple element access on local vars (already works)

`src/parser/operators.rs:608-658` already handles
`Reference(__tuple<…>)` element access via `get_val` at the
synthetic struct's field offsets.  After Step 1, `r.0` on a
rewritten local routes through this arm automatically — same
path stored-tuple element access takes today (P189b).

No new code in the access path.

### Step 5 — Match destructure (already works)

`src/parser/control.rs::parse_tuple_match` (line 1846) currently
matches on `Type::Tuple` subjects, creates a temp var via
`create_unique`, and emits `Value::TupleGet(tmp, i)` for each
binding.  After Step 1, the temp's type becomes
`Reference(__tuple<…>)` when the subject has lifetime concerns.
`TupleGet` already has a Reference arm in codegen
(`src/state/codegen.rs:332`).

No new code in the destructure path.

### Step 6 — Retire `output_set`'s tuple-text handling

After Steps 1-5, LOCAL `Type::Tuple([Text, …])` variables no
longer exist — they're all rewritten to `Reference(__tuple<…>)`.
The machinery in `src/generation/dispatch.rs:295-359` becomes
dead:

- `tuple_text_to_string` flag setting (line 301-306) — never fires
- `tuple_text_elem_clone` detection (line 336-343) — never fires
- The `var_t.0.clone()` emission branch (line 349-354) — never fires

Retire the dead code with a comment pointing at this phase.

Also retire any related dead state in `src/generation/mod.rs`:
the `tuple_text_to_string` field on `Output` (line 271) and the
`rust_type` Result→Variable recursion (line 366-374, now
defensive only — left with a doc-only update noting Phase 07b
makes it unreachable for text-bearing tuples).

### Step 7 — Verification

| Test | Expected |
|---|---|
| `cargo test --release --test tuple_matrix -- --ignored` | All 17 pass |
| `cargo test --release --test tuple_matrix -- --ignored e2_d1_text_text_local` | Pass — local `(text, text)` routed through synthetic struct |
| `cargo test --release --test tuple_matrix -- --ignored e2_d1_text_int_local` | Pass — local `(text, integer)` routed |
| `cargo test --release --test issues p234` | All 3 still pass (Phase 07 regression net) |
| `cargo test --release --test threading_chars par_tuple_return_struct_text` | Still passes |
| Phase 07 reproducer `/tmp/p234_v6.loft` | "OK r.1 == 5" on both backends |
| `cargo fmt --check`, `cargo clippy --release --all-targets -- -D warnings`, `cargo build --release --no-default-features` | Clean |
| `bench/` — pure-value tuple paths | No regression (these keep Rust ABI) |
| `bench/` — tuple-with-lifetime-concern paths | Slight cost acceptable (rare paths) |

## Critical files

| File | Change |
|---|---|
| `src/parser/mod.rs` | New `add_local_with_tuple_rewrite` helper; `convert` arm for Tuple → Reference(__tuple) |
| `src/parser/expressions.rs` | Route var declarations + destructure temps through helper (~3 sites) |
| `src/parser/control.rs` | Route match binding contexts through helper (~3 sites) |
| `src/generation/dispatch.rs` | Retire `tuple_text_to_string` + `tuple_text_elem_clone` blocks (~150 LoC removed) |
| `src/generation/mod.rs` | Retire `tuple_text_to_string` field on `Output`; doc-update `rust_type` Tuple Result branch |
| `doc/claude/plans/14-tuple-validation/07-p234-runtime.md` | Add cross-link to this phase |
| `doc/claude/plans/14-tuple-validation/README.md` | Add Phase 07b row to phase table |

## Existing infrastructure to reuse

- **`data::has_lifetime_concern`** (`src/data.rs`) — shipped in
  Phase 07; identical predicate
- **`data::tuple_def`** (`src/data.rs:2397`) — idempotent
  synthetic struct registration
- **`Parser::rewrite_tail_tuple_to_synthetic_struct`**
  (`src/parser/control.rs`, shipped in Phase 07) — REUSE for the
  convert-arm rewrite
- **`parse_object`'s OpDatabase + per-field-init pattern**
  (`src/parser/objects.rs:1229-1347`) — this is what
  `rewrite_tail_tuple_to_synthetic_struct` mirrors
- **TupleGet's Reference(__tuple) arm** in `operators.rs:608-658`
  and `codegen.rs:332`
- **Match destructure with `Reference(__tuple)` subject** —
  already works via existing TupleGet machinery

## Risks

1. **Generic templates** — same gate as Phase 07.  Skip rewrite
   when `def_type(self.context) == DefType::Generic`.  Mirror
   the guard added to `parse_function`.
2. **First-pass type instability** — var types may be Unknown on
   first pass; `tuple_def` is idempotent so calling it twice is
   safe, but the rewrite gate should fire on second pass when
   element types are resolved.  Verify with two-pass inputs.
3. **Function parameters left as Tuple ABI** — when a local of
   `Reference(__tuple<…>)` is passed to a fn with `(Tuple, …)`
   parameter, type-mismatch at the call site.  Mitigations:
   (a) skip the parameter rewrite (current scope) and detect/
   convert at the call site, OR (b) include parameters in scope
   (broader change, more tests affected).  Decision deferred to
   Step 1 implementation.  If Plan-14's `e2_d2_arg_*` tests
   break, include parameters.
4. **Performance regression for short-lived locals** — heap
   alloc per tuple-with-lifetime-concern local.  Verify with
   `bench/` that no benchmark regresses; acceptable cost given
   how rarely locals carry tuple-with-Reference (refs typically
   passed directly, not boxed in tuples).
5. **Subtle interp/native asymmetry** — Phase 07's diagnosis
   originally had this backwards (binary defaults to `--native`).
   For Phase 07b, test BOTH backends explicitly at each step to
   avoid the same trap.

## Out of scope

- Function parameters of tuple type — separate phase if needed
- Pure-value local tuples — keep Rust ABI
- Tuple destructure binding gate (P235 par half — already
  separate; P235 lexer + non-par destructure shipped 2026-05-07)
- ARC.md A8/A11 plan-06 closeout — separate

## Verification

```bash
# Phase 07 reproducer (regression net)
./target/release/loft /tmp/p234_v6.loft           # → "OK r.1 == 5"
./target/release/loft --interpret /tmp/p234_v6.loft

# Local tuple-with-text repro (the new path this phase enables)
cat > /tmp/p07b_check.loft << 'EOF'
fn main() {
    t: (text, integer) = ("answer", 42);
    print("{t.0} {t.1}\n");
}
EOF
./target/release/loft /tmp/p07b_check.loft         # → "answer 42"
./target/release/loft --interpret /tmp/p07b_check.loft

# Tests
cargo test --release --test tuple_matrix -- --ignored
cargo test --release --test issues
cargo test --release --test threading_chars
cargo test --release --test wrap loft_suite
cargo test --release --test native -- --test-threads=1

# CI gate
cargo fmt --check
cargo clippy --release --all-targets -- -D warnings
cargo build --release --no-default-features
```

## Cross-references

- [Phase 07 — P234 runtime: lifetime-bearing tuple returns](07-p234-runtime.md)
  — predecessor.  Established the routing pattern; this phase
  extends it to LOCAL vars.
- [Phase 04 — struct references](04-references.md) — covers
  References as tuple ELEMENTS in storage destinations.
  Complementary.
- `src/data.rs::has_lifetime_concern` — the predicate shared by
  Phase 07 and 07b.
- `src/parser/control.rs::rewrite_tail_tuple_to_synthetic_struct`
  — the helper this phase reuses for convert-arm tuple → struct
  rewriting.
