<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Cluster IV-Spacial — parser infinite loop on `spacial<X[k]>` types

**Severity:** parser infinite loop — `loft` process is unresponsive until `LOFT_TIMEOUT` fires (or user `kill -9`s).  No backend reaches its run phase.

**Affected probes:** 51 (canonical with `??`), 62 (no `??`, still hangs), 88 (integer key, hangs), 89 (bare declaration, hangs), 90 (Node without spacial field — PASS reference baseline).  See [Probe set Z](README.md#curated-probe-sets--for-fix-attempt-validation).

**Backend asymmetry:** BOTH backends hang — the loop is in the PARSER, which runs before any backend selection.

## Mechanism (verified to a partial level)

Probes 51 / 62 / 88 / 89 together establish:

1. **The hang fires on type DECLARATION alone** — probe 89 hangs with just `s: spacial<Point[name]> = []; println(...);` — no insert, no read, no `??`, no other use.  The infinite loop is in `spacial<>` type registration itself.
2. **Key type doesn't matter** — probe 88 (integer key) hangs identically to probe 51 (text key).  The bug isn't key-resolution-specific.
3. **`??` isn't required** — probe 62 hangs without any `??`.
4. **A regular struct doesn't hang** — probe 90 (Node with `name: text, x, y` but NO spacial field) parses fine.

So the loop is somewhere in `src/parser/` or `src/typedef.rs` where `spacial<>` types are registered into the database.

PLAN49 breadcrumb (`LOFT_TIMEOUT=8 LOFT_TIMEOUT_CLEAN_EXIT=1`) localises to `phase=parse fn=? file=…:0` — the breadcrumb's fn-tracker doesn't get past the first checkpoint, so we know the hang is during file-level type registration, before any user function gets entered.

## Reference probe — 90 (struct without spacial field, PASS)

```loft
struct Node { name: text, x: float not null, y: float not null }
fn main() { n = Node { name: "root", x: 0.0, y: 0.0 }; }
```

Parses and runs cleanly.

## Problem probe — 89 (bare spacial declaration, HANG)

```loft
struct Point { name: text, x: float not null, y: float not null }
fn main() {
  s: spacial<Point[name]> = [];
  println("never reached");
}
```

Parser hangs at line 0 (per breadcrumb) — never reaches main()'s body.

## The divergence

`spacial<Point[name]>` triggers a parser code path that doesn't terminate.  The struct `Point` itself parses fine (probe 90); only the `spacial<>` wrapping causes the loop.

## What we know vs. don't

| | Status |
|---|---|
| Hang fires during parse-phase, before any backend | ✅ Verified via PLAN49 breadcrumb |
| Hang fires on type-declaration alone, no operations needed | ✅ Verified — probe 89 |
| Key type (text vs int) doesn't matter | ✅ Verified — probes 51 (text), 88 (int) |
| `??` doesn't matter | ✅ Verified — probe 62 |
| Regular structs without spacial field parse fine | ✅ Verified — probe 90 |
| Exact source location of the loop | ❌ Not yet pinpointed — needs source-side instrumentation |
| Does the loop exist in other keyed-collection types (sorted/index/hash)? | ❌ Not directly probed at type-registration level — Set E probes use them with operations that pass.  Likely spacial-specific |

## Investigation tasks

1. ~~Confirm hang fires on bare declaration~~ — done (probe 89).
2. ~~Confirm key-type independence~~ — done (probe 88).
3. **Locate the parser/typedef loop site**:
   - Grep `src/parser/` and `src/typedef.rs` for `"spacial"` / `Type::Spacial` / spatial-index-specific code paths.
   - Add an `eprintln!` at the top of every spacial-related function found above; run probe 89; the eprintln that prints repeatedly localises the loop.
   - Alternative: use `RUST_LOG=loft::parser=trace` if that lever exists (check `src/log_config.rs`).
4. **Hypothesise & verify**:
   - **Hypothesis A**: spacial type-registration recursively expands `Type::Spacial(box T, _)` where T's recursion isn't depth-capped.  Verify by inspecting the recursive call.
   - **Hypothesis B**: spacial's key-index layout calc loops on a self-referential cycle in the type-field map.
   - **Hypothesis C**: a `for` loop in spacial's setup uses a counter that's been mis-initialised (e.g. `for i in 0..u16::MAX` where the early-exit condition never fires).

## Fix surface

Depends on the loop site, which Investigation task 3 will localise.  Three likely shapes:

### Option A: depth-cap on recursive type expansion

If hypothesis A holds, add a depth-limit check (similar to existing protections in struct-recursion handling):

```rust
fn expand_spacial(t: &Type, depth: usize) -> ... {
    if depth > MAX_TYPE_DEPTH {
        return Err(/* clean diagnostic */);
    }
    // ... existing logic ... call expand_spacial(.., depth + 1) ...
}
```

**Effort**: XS (one-line cap + existing-error-path reuse).
**Risk**: LOW (depth cap can only reject more, not accept worse).

### Option B: memoisation on type-resolution

If hypothesis B holds, cache resolved-type results so the second visit returns the cached value rather than re-resolving.

**Effort**: S (add a HashMap to the resolver context).
**Risk**: LOW-MEDIUM (cache invalidation rules need care).

### Option C: counter-bug fix

If hypothesis C holds, fix the off-by-one or wrong sentinel.

**Effort**: XS.
**Risk**: LOW.

**Picking between options**: Wait for Investigation task 3's output.  Don't commit to a fix until the loop site is known.

## In-plan vs spinoff decision

Per the [in-plan vs spinoff policy](README.md#in-plan-vs-spinoff-policy), keep in-plan unless the investigation reveals 2+ additional parser-recursion sites elsewhere.  Currently spacial is a single site; the fix is one of Options A/B/C above.

If Investigation task 3 reveals the same loop pattern in `sorted<>` / `index<>` / `hash<>` declarations under specific conditions (currently passes for them, but the underlying machinery might be shared), spin off a PLAN53 for "parser type-registration recursion family" and move all the related probes there.

## Why this isn't a runtime cluster

Unlike clusters I/III/IV-Vec/etc. (which involve the `??` lowering and scope-pass interaction), IV-Spacial is purely in the parse phase.  It surfaced via PLAN52's cluster IV completion sweep (probe 51 was testing IV-Spacial's value-block behaviour with `??`, but the parser never got past type-registration to test the value-block path).  Even if the entire `??` lowering were removed, probe 89 would still hang.
