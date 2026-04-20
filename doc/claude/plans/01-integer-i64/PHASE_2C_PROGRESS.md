# Phase 2c — execution progress log

Status: **in-flight on branch `int_migrate`**.  Last commit
`9ecbf1f` (2026-04-20).  Suite failure count: **16 of ~1800
tests**.

This file captures the post-INCREMENTAL_PLAN execution state.
It supersedes the "not started" status in `PHASE_2C_EXECUTION.md`
and records what actually landed, what's stubbed, and what's
next.

## Shipped increments

| Round | Commit | Summary | Tests cleared |
|-------|--------|---------|---------------|
| **2c.0** | `d5f5ec9` | initial migration (types, ops, stores widened) | — |
| **2c.5** | `c7aa572` | round 5 — file I/O, generics, char literals | |
| **2c.6** | `ca53017` | round 6 — character set, sizeof, narrow-int file I/O | |
| **2c.7** | `d28be8a` | round 7 — file seek, dir listing, tuple offsets | |
| **2c.8** | `e37a494` | round 8 — honour `size(N)` on integer subtypes | |
| **2c.9** | `b4655bc` | round 9 — `v#remove` at iter-index 0 | |
| **2c.8.1** | `af5fc4e` | cast-aware write narrowing (`f += x as i32` → 4 B) | |
| **2c.10a** | `8454c10` | deprecate `l` suffix, sweep test scripts | |
| **2c.10b** | `67b537e` | `long` keyword aliases `integer` | |
| **2c.10b.1** | `291ce7a` | deprecation warnings for `long` / `l` | |
| **— cleanup —** | `89d42de` | binary-test integer-size updates | 2 |
| | `07b5b2d` | fmt + clippy across rounds 5–10 | — |
| | `59c9637` | doc-test size + p85b line drift | 6 |
| | `dddb8ea` | sweep test fixtures for `long` / `l` | 27 |
| | `09ddbb4` | native extension bridge i32 → i64 widening | 2 |
| | `8edb15c` | native_pkg cdylib vec<i32> → vec<i64> | 6 |
| | `54f649f` | slots.rs narrow-write i32 cast (same pattern as 89d42de) | 1 |
| | `7bf3558` | n_parallel_for extra-arg cleanup: `-= 4` → `-= 8` | 3 |
| | `edbc9f3` | worker extra-arg push: `as i32` → `as i64` | 3 |
| | `9ecbf1f` | execute_at_long + execute_at_text sibling push fix | 0 (preventive) |
| | `d3ac78c` | emit OpSetInt4 for collection/vector-header 4-byte writes | 0 (D.1 partial — minimal repro fixed, wrap tests still fail on unrelated path) |

**Since round 10b.1 (`291ce7a`)**: 59 → 16 failures, **50
tests turned green**, 0 regressions across 9 consecutive
no-regression commits.

`7bf3558` + `edbc9f3` are the first two runtime-codegen fixes
in this streak and validate the `CODEGEN_AUDIT.md` hypothesis:
post-2c stale hard-coded 4-byte integer widths in codegen +
worker setup explain the majority of D+E failures.  Between
them they closed:

- Category E (4 tests, all of them)
- Category D.2 (2 tests)
- Category D.3 (1 test)

That's 7 tests from 2 two-line fixes, both symmetric
(compile-time tracker + runtime push).  The remaining 16
failures cluster in C (native codegen), D.1 (allocation
OOB), G (WASM, deferred), and two misc native-path tests.

## Remaining failures (23, by category)

### Category A — deprecation-warning pollution (0 — **CLOSED**)

Cleared by commit `dddb8ea` (inline-source sweep) plus the
`testing.rs:137` fix (stop emitting `_l` suffix on
`Value::Long` result auto-assertions).

### Category C — native codegen i32 → i64 (5 tests, **in flight**)

Tests: `native_binary_script`, `native_dir`, `native_scripts`,
`native_tuple_return_script`, `native_tuple_script` + related
`moros_glb_cli_end_to_end`, `p171_native_copy_record_high_bit`.

**Attempted 2026-04-20, reverted.**  See
[CATEGORY_C_FINDINGS.md](CATEGORY_C_FINDINGS.md) for the full
write-up.  TL;DR: the native codegen side-path correctly widens
variable types (Layer A) and inserts explicit `as i64` casts at
call/assignment sites (Layer B), but a deeper structural issue
blocks completion — `Value::Int` is emitted for both user loft
literals AND internal type-number constants, and picking one
suffix (_i32 or _i64) breaks the other context.  After applying
all three layers, `native_binary_script` reaches a clean rustc
compile but then fails at runtime with a `types.rs:358` OOB —
which is NOT a C-category bug but a pre-existing D-style
runtime invariant violation.

Recommendation: either (a) make `Value::Int` emission
context-aware (track expected-type at emission site), or (b)
widen runtime signatures where loft-integer semantics are clear
(OpGetTextSub, OpLengthCharacter, n_assert, OpReadFile done;
extend to OpNewRecord/OpCopyRecord/etc. as needed) and narrow
only genuine tp-number constants.  See CATEGORY_C_FINDINGS.md
for the proposed minimum viable set of codegen_runtime.rs
signature changes.

### Category D — deep runtime panics (7 tests)

Three sub-bugs:

**D.1 — `allocation.rs:265` index OOB (3 tests)**
`wrap::dir`, `wrap::last`, `wrap::parser_debug` —
`index 8 out of 5`.  Fires on `tests/docs/15-lexer.loft` and
`16-parser.loft`.

*Investigation 2026-04-20 (partial)*:

- Backtrace: `State::execute_argv` → `State::new_record` →
  `Stores::record_new` → `Allocations::claim` with a
  `DbRef{store_nr=8}` against `allocations.len()=5`.
- Trigger: method call on a Lexer struct that appends to an
  indexed-collection field (`l.set_tokens(["x"])` alone
  reproduces it; even smaller scripts take too long to
  debug without better tooling).
- Same panic shape as D.2/E — a DbRef read from the stack
  has a corrupted store_nr field.  Different code path
  (main thread, not parallel worker), but the hypothesis is
  the same: post-2c stack-slot drift after a specific op
  sequence.
- Hot candidates for the drift:
  - Indexed-collection field access (`struct.field[idx]`
    reads + `+=` writes)
  - Method-call on struct forwarding the `self: Ref` through
    a RefVar argument
  - For-loop over a `vector<text>` with 8-byte-per-iter
    advancement when codegen assumed 4

*Deferred*: requires a dedicated debugging session with
(a) a minimal .loft reproducer that runs in < 5 seconds,
(b) `LOFT_LOG=crash_tail` output capturing the 20 ops
before the panic, (c) a bytecode disassembly of the
suspect function.  Estimate: 2-4 hours.

*Update (2026-04-20, late)*: **D.1 is actually an infinite
loop, not a direct crash**.  The `record_new` OOB panic in
the failing wrap tests is a defensive bounds check that
fires *eventually* after the interpreter spends unbounded
time spinning.

**Root cause pinpointed**: `OpSetInt` writes 8 bytes post-2c,
but collection-header fields (hash/sorted/vector/index/
spacial) are 4-byte u32 pointers.  The extra 4 bytes
overflow into the NEXT record's claim-size header,
writing 0 where a free-block marker (-93) belongs.
`Store::claim_scan` then loops forever: `pos += abs(0)`.

**Partial fix applied** (commit `d3ac78c`) — three codegen
sites emit `OpSetInt4` instead of `OpSetInt`:
- `parser/mod.rs::set_field_check` (collection field write)
- `parser/objects.rs::parse_object_field` (explicit-write)
- `state/codegen.rs` vector-header init

**Minimal 4-line reproducer now passes**.  But the wrap
suite tests (`wrap::{dir, last, parser_debug}`) still fail
with the same panic — they exercise a different code path
(the parser library's `parse(...)` → Lexer.set_tokens chain)
that has ADDITIONAL sites emitting `OpSetInt` for collection
headers.  Next session: find the remaining sites via
targeted grep for `stack.add_op("OpSetInt", ...)` or
`self.cl("OpSetInt", ...)` in contexts where the destination
field is a collection type.

Minimal 4-line reproducer saved at
`probes/probe_d1_hash_sorted_hang.loft`:

```loft
struct Poss { length: integer, token: text }
struct Tok { start: character, possible: sorted<Poss[-length, token]> }
struct Db { items: hash<Tok[start]> }
fn main() {
  db = Db { };
  db.items += [Tok { start: 'x', possible: [] }];
}
```

Trigger: append to a `hash<X[key]>` where `X` contains a
nested `sorted<Y[key]>` field.  Removing either the outer
`hash` or the inner `sorted` makes the script terminate.
Narrower isolation than previously thought — points at the
hash+sorted nested-key machinery in `src/vector.rs` /
`src/hash.rs` rather than a stack-layout bug.

*Also discovered*: two `data_structures` test binaries from
2026-04-18 had been spinning orphaned for 34+ hours (82 h /
49 h CPU time).  Killed; all 16 data_structures tests pass
in < 20 s each on the current tree — the bug that spun them
was fixed by the round 5-10 commits and only stale detached
processes remained.

**D.2 — `codegen.rs:1780` slot-width drift (**CLOSED** 2026-04-20)**
`wrap::loft_suite` + `wrap::script_threading` — cleared by
commits `7bf3558` (codegen tracker widen) + `edbc9f3` (worker
push widen).  Script 22-threading.loft uses `par()` with
extras; the 4-byte leak on every par call × multiple loops ×
worker-side 4-byte reads cancelled each other incorrectly
after post-2c, producing the 3-byte drift in `n_main`'s var
layout.  Both sides widened to 8 B; drift resolves.

*Earlier investigation note (retained for reference)*: the
three stale size-computation sites documented below were
ALL on the trail; the fix turned out to be sites 1 and
(symmetrically) the worker `put_stack(extra as i32)` rather
than sites 2 and 3:

- `src/state/codegen.rs:1628` — **FIXED** in `7bf3558`.
- `src/state/codegen.rs:1442-1447` — stale comments but code
  still correct because `OpConstInt` now pushes 8 B (the
  `< 16` threshold still triggers the right number of pads).
- `src/state/codegen.rs:1795-1799` — known `+= 4` adjustment
  for OpVarFnRef; unchanged, still correct.

**D.3 — `ops.rs:278` long overflow (**CLOSED** 2026-04-20)**
`wrap::threading` — cleared by commit `edbc9f3` (worker push
widen).  The bit pattern `0x8000_0000_FFFF_FFFF` traced back
to the worker reading an i64 slot whose low 32 bits held a
valid i32 value and whose high 32 bits held the high half of
a neighbouring slot's null sentinel.  Once the worker push
widens all extras to 8 B, slots no longer straddle — each
i64 read returns a clean integer with no sentinel
contamination.

**D.4 — `types.rs:358` vector type OOB** (surfaces under
Category C experiments) — `vector(content=60)` but
`types.len()=60`.  Off-by-one in type-table init.

### Category E — parallel keys OOB (0 — **CLOSED**)

All 4 tests cleared by commits `7bf3558` + `edbc9f3`.  The
panic at `keys.rs:211 len=4 index=4` in `get_vector` turned
out NOT to be a worker-store-init issue (the agent's
initial hypothesis) — it was the *main thread* reading a
corrupted DbRef slot because stack tracking drifted by 4 B
per par() call.  Widening the extra-arg tracker (codegen)
and the worker extra-arg push (runtime) aligns both
sides; the panic no longer reproduces.

### Category F — native_loader u32/i64 bridge (0 — **CLOSED**)

All 6 Category F tests cleared by commits `09ddbb4` (bridge
widening) + `8edb15c` (cdylib fixture vec<i64>).  Local suite
confirms 15/15 native_loader tests pass.

### Category G — WASM / HTML export (5 tests, **deferred**)

`p137_html_*` (4) + `q9_html_file_content_returns_empty_on_wasm`
— orthogonal to 2c, track in `HTML_EXPORT.md`.

### Category H — long-widening tautology (0 — **confirmed passing**)

`p180_int_widens_to_long_field` was listed in earlier drafts
as a residual failure.  Agent verification (2026-04-20)
confirms the test is actually passing in the current tree —
the widening is a no-op identity and the test trivially
passes.  No action needed.  Can be deleted at round 10c
(when `Type::Long` variant is removed and the test would
become genuinely empty).

## Remaining failure map (16 total, updated 2026-04-20)

| Category | Count | Tests | Status |
|----------|-------|-------|--------|
| C | 5 + 2 | native_{binary_script, dir, scripts, tuple_return_script, tuple_script} + moros_glb_cli + p171 | retry plan in CATEGORY_C_FINDINGS.md |
| D.1 | 3 | wrap::{dir, last, parser_debug} | needs trace — probably same class as D.2/D.3 |
| G | 5 | p137_html_* + q9_html_file_content_returns_empty_on_wasm | deferred (orthogonal to 2c) |
| F | 1 | moros_glb_cli_end_to_end | native-path; may clear with C |

Categories D.2, D.3, E all cleared in this session.

## Next-action recommendation (revised 2026-04-20)

| Priority | Category | Cost | Yield | Risk |
|----------|----------|------|-------|------|
| 1 | **D.1 codegen audit** — follow the same trace discipline that cleared D.2/D.3/E | 1-2 hr | 3 tests | low-medium |
| 2 | **Category C retry** — Strategy 1 | 3-5 hr | 5-7 tests | medium |
| Defer | G | — | 5 tests | — |

Recommendation: **D.1 first**.  D.2/D.3/E were theorised to be
deep codegen audit issues requiring a 2-4 h session; in
practice 2 two-line fixes cleared 7 tests.  D.1 shares the
same panic shape (DbRef store_nr out of bounds after a
complex operation) and is likely another narrow post-2c
hardcoded-size bug.  Apply the same pre/post snapshot
discipline that worked for the other categories.  After D.1,
schedule Category C as a separate dedicated session since it
involves `src/generation/` which was previously net-negative
on a surface-level attempt.

D.2 offers the clearest debuggable shape (3-byte drift points at
a specific `Type::size()` site or fixed-offset miscalculation).
D.1 has a concrete reproducer (`15-lexer.loft`).  Both are
runtime-only, so diagnosis via `LOFT_LOG=crash_tail:50` is
direct.

## Note on `PHASE_2C_EXECUTION.md`

That file's "not started — checklist for the eventual dedicated
session" header is stale.  The checklist's edit sequence was
executed across rounds 5-10 above, not a single 6-hour session.
The incremental 45-min per-round shape worked — the coupling
concerns raised in that doc turned out to be manageable when
split by file/subsystem.
