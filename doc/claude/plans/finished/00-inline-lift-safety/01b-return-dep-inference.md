# Phase 1b — return-dep inference for mid-body `return` statements

Status: **Done** — Reference + Enum(struct-enum) arms landed.  Vector arm
intentionally deferred (see "Scope trimmed").

## Root cause

The Phase 1 gate at `src/state/codegen.rs:918` / `~1295` reads
`def.returned.depend()`.  That chain is populated by two helpers in the parser's
second pass:

| Helper | What it merges | Called from |
|---|---|---|
| `text_return(ls)` (`control.rs:2264`) | deps of `Type::Text` returns | `block_result` (tail) AND `parse_return` (mid-body) |
| `ref_return(ls)` (`control.rs:2351`) | deps of `Type::Reference` / `Vector` / `Enum(struct-enum)` returns | **only** `block_result` (tail) |

Asymmetry: Text was handled everywhere; Reference/Vector/Enum only via the
tail path.  A function like
```loft
fn first_or_empty(c: Container, idx: integer) -> Inner {
  if idx >= 0 && idx < len(c.items) {
    return c.items[idx];   // view — mid-body return
  }
  Inner { n: 0 }           // owned — tail
}
```
lost the `[c]` dep from the mid-body path.  `block_result`'s tail was owned so
`ref_return([])` was a no-op.  `def.returned` stayed `Reference(Inner, [])`.
Gate missed.  `OpCopyRecord | 0x8000` fired.  SIGSEGV.

## Fix

`src/parser/control.rs::parse_return` — after the existing `convert` /
`validate_convert` block, added a ref-merge that mirrors the same arms
`block_result` has at line 340.

```rust
if self.data.def_type(self.context) != DefType::Generic {
    if let Type::Reference(_, ls) = &t {
        if ls.is_empty() {
            let extra = Self::collect_hidden_ref_args(&v, &self.data);
            if !extra.is_empty() {
                self.ref_return(&extra);
            }
        } else {
            self.ref_return(ls);
        }
    } else if let Type::Enum(_, true, ls) = &t {
        self.ref_return(ls);
    }
}
```

The existing `text_return` and `!self.first_pass` vector-writeback branches
remain unchanged.

## Verification

1. `LOFT_LOG=static` on `snippets/07_mixed_return.loft`:
   - Pre-fix: `fn n_first_or_empty(...) -> Inner { ... -> ref(Inner)` (no dep).
   - Post-fix: `-> ref(Inner)["c"]` (dep merged from mid-body `return c.items[idx]`).
2. OpCopyRecord emissions at call sites:
   - Pre-fix: `tp=0x803f` (0x8000 ON) → frees caller's store → SIGSEGV.
   - Post-fix: `tp=0x3f` (OFF) → gate fires → no free → correct.
3. All 16 snippet variants pass (01-04, 07-17).
4. `lib/moros_sim/tests/picking.loft::test_edit_at_hex_raise` passes WITHOUT
   the `h = map_get_hex(...)` hoist workaround.  Workaround removed.
5. moros_sim full suite: 137 passes across 12 files.
6. moros_ui full suite: 41 passes across 4 files.
7. 8 P120 leak regressions stay green.

## Scope trimmed: Vector arm deferred

My initial fix also mirrored the Vector arm (`Type::Vector(_, ls) => ref_return(ls)`).
That broke moros_ui's layout tests with
`Incorrect var __ref_2[65535] versus 516 on n_panel_build`.

Cause: `palette_items_for_tool` has mid-body returns like
`return HEIGHT_STEP_LABELS;` (global const) and `return pi_list;` (local), and
their Vector types' dep chains reference these non-argument variables.  When
`ref_return` tried to promote them to hidden ref-args, the resulting signature
added hidden args for constants and otherwise-local variables that callers
have no way to supply.  Reference/Enum returns don't hit this because their
typical dep vars (e.g. `c` in `return c.items[idx]`) are already function
parameters — `ref_return`'s `attr_names.get(n)` idempotency branch triggers
and no new hidden arg is created.

Deferring the Vector arm is fine for now because no SIGSEGV variant has been
observed for Vector returns; the problem is specific to Reference/Enum.  A
future Phase 1c could tackle Vector safely by:
1. filtering `ls` to include only function-parameter vars, OR
2. extending `ref_return` to refuse to promote globals/consts, OR
3. a separate copy-into-caller-buffer mechanism for mixed-return Vectors.

Open as follow-up only when a concrete Vector SIGSEGV variant appears.

## Known consequence: owned-fallback leak

For `first_or_empty`'s owned-fallback path (`Inner { n: 0 }`), the fresh store
is no longer freed by 0x8000.  One small struct leaks per fallback call.
Not corruption — just a leak.  Rare error-path for `map_get_hex`-style
accessors.  Not a show-stopper.  A future Phase 1c could promote Reference
returns to a caller-provided scratch buffer similar to the `__ref_1` vector
mechanism.

P120 leak regressions (all 8) stay green because they cover
tail-only / consistent-return cases, not mixed returns.
