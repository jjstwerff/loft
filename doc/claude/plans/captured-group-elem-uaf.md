<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Captured-group element access — use-after-free of the group's backing store

**Status: root cause PINNED, fix decided, NOT yet implemented.** Ready to pick up.
`area:store-lifetime`, `sev:high` (UAF; benign-in-practice today but real UB).

Owns two failing scripts (same root):
- `tests/scripts/35m-mid-slice-repetition.loft` — `get_vector: use-after-free on store N`.
- `tests/scripts/35c-rest-capture.loft` — `sub-class A (enum + captured field + rest): null`.

These are why the nightly **miri** workflow's *Debug-assertions gate* and *LOFT_POISON
arena-UAF gate* have been red (both `#[cfg(debug_assertions)]`-only; the scripts PASS on a
plain release build, so the normal suite never catches them). @PLN35 Phase 6 (match-slice
repetition) introduced the shape.

## Repro (minimal)

```loft
enum Token { Punct { #lexeme sym: text }, Ident { name: text }, Num { value: integer } }
fn id(n: text) -> Token { Ident { name: n } }
fn p(s: text) -> Token { Punct { sym: s } }
// reading an ELEMENT of the captured group `arg` (index or nested rest match) UAFs;
// `len(arg)` is fine; the same access on a NORMAL vector is fine.
fn f(v: vector<Token>) -> text {
  match v { [_, "(", (arg: Ident)*(","), ")"] => match arg[0] { Ident { name } => name, _ => "?" }, _ => "!" }
}
fn main() { f([id("foo"), p("("), id("a"), p(","), id("b"), p(")")]); }
```

Build + run (the free-check is a `debug_assert!`, off in release):
```
RUSTFLAGS='-C debug-assertions=on' CARGO_TARGET_DIR=target-da cargo build --release --bin loft
LOFT_STORE_GUARD=1 target-da/release/loft --interpret f.loft
#   → thread 'main' panicked at src/vector.rs:356: get_vector: use-after-free on store N
```
(`LOFT_POISON=1` under a nextest run is the other gate; same root — freed memory read back.)

## Matrix (what triggers it — proven)

| pattern in the arm body | result |
|---|---|
| `len(arg)` | OK — no element deref |
| `arg[0]`, or nested `match arg { [x, ..] => … }` | **UAF** |
| the identical access on a **normal** `vector<Token>` (not a captured group) | OK |

So it is captured-group-specific and element-access-specific.

## Root cause (pinned — bytecode-level)

The captured group `arg` is a **view** into a fresh backing store `__vdb_1`:
`arg = OpGetField(__vdb_1, 0, 70)`, type `vector<...>` with dep `frame1(__vdb_1)`. In the arm,
the emitted bytecode order is:

```
OpDatabase(__vdb_1, …)                 ; create backing store
arg = OpGetField(__vdb_1, 0, 70)       ; arg VIEWS __vdb_1
… collection loop: OpNewRecord/OpFinishRecord(arg) …
OpFreeRef(__vdb_1)                      ; <-- frees arg's backing store (last DIRECT use of __vdb_1)
… arm body: OpGetVector(arg, 16, 0) …  ; <-- reads arg[0] AFTER the free  →  UAF
```

`scopes.rs` (the dep/lifetime analyzer) frees `__vdb_1` at its last **direct** use — the
collection loop — and does **not** count the arm body's use of **`arg`** as keeping
`__vdb_1` alive, even though `arg` carries dep `frame1(__vdb_1)`. So the owned backing store
is freed while a live view still points into it. Benign on release only because the freed
slot is not reused before the read; the sanitizer (debug-assertions / POISON) catches the UB.

Normal borrowed views (`e = v[i]`) keep `v` alive through `e`'s uses — that path works. The
captured-group view is the case that slips through.

## Where the fix goes

`src/scopes.rs` — the dependency/lifetime free-insertion. The free point for an OWNED
backing store (`__vdb_1`) must extend past uses of any var that **depends on** it (the
`frame1` view `arg`), not stop at the store's last direct reference. Relevant threads:
- `parse_slice_repetition` / `materialize_named_rest` / `materialize_field_projection`
  (`src/parser/control.rs` ~4155-4263, ~4649) build the group + set `arg`'s `frame1(__vdb)` dep.
- The slice-binding block is assembled at `control.rs:7205` (`v_block(bindings, …, "slice_binding")`),
  bindings (collection, incl. the early free) prepended to the arm body.
- `materialize_vector_arms_collect` (`control.rs:9558`) is the vector-RETURN free path — NOT
  this bug (the failing arm returns text), but read it to avoid double-handling.
- The actual premature free comes from `scopes.rs`'s last-use free-insertion for `__vdb_1`.

**Fix shape (decided direction, unverified):** in `scopes.rs`, when computing the free point
for an owned store `d`, treat a use of any var `x` with `d ∈ x.deps` as a use of `d` — so the
`OpFreeRef(__vdb_1)` lands AFTER the arm body's last use of `arg`, not after the collection.
Scope tightly to the view-dep case; do NOT over-reach (an over/under-reach here creates new
leaks or UAFs rather than a compile error — this is loft's #1-weakness subsystem).

## Verification matrix (required before commit)

A store-lifetime change must pass EVERY mode — do not trust one:
1. `--interpret` and `--native` on the two scripts (both must PASS, same value).
2. Debug-assertions build (`RUSTFLAGS='-C debug-assertions=on' LOFT_STORE_GUARD=1`) — no UAF panic.
3. `LOFT_POISON=1` (the POISON gate) — no null/garbage read.
4. A leak check (`LOFT_STORES=warn` / `LOFT_NATIVE_LEAK_CHECK`) — the fix must not orphan `__vdb`.
5. Full suite both backends (`./scripts/find_problems.sh --bg`) — no regressions (esp. other
   match/vector-return/borrowed-view cases; watch for a re-run of the `call_op`-style over-reach).
6. Graduate: the two scripts already exist; confirm they move from red-in-miri to green.

## Not this bug (ruled out during the investigation)

Name collision, field-name unification, API stubs, the `call_op` unary-inference bug
(that was the graphics-cluster issue, fixed in loft#592). This is purely the captured-group
view's backing-store lifetime.
