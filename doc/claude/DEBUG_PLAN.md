
// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

# Debugging plan: safety, data-loss, leak, and library issues

This document describes a systematic approach to debugging and fixing
every remaining safety/data-loss/leak/library issue in the loft runtime.
Each issue follows the same five-phase methodology: reproduce minimally,
validate outside GL, drill to root cause, analyse before fixing, fix
and verify back in the GL environment.

Created: 2026-04-10.

---

## Issue inventory

| #    | Title                                              | Category     | Severity | Status           |
|------|----------------------------------------------------|-------------|----------|------------------|
| P120 | ~~Store leak on struct field overwrite in loop~~    | ~~Safety/Leak~~ | ~~**High**~~ | **Fixed** — high-bit on CopyRecord type in `copy_ref()`; isolation + GL tests pass in debug |
| P127 | ~~File-scope vector constant corrupts caller slots~~ | ~~Data loss~~ | ~~Medium~~ | **Fixed** — pre-built in CONST_STORE via `OpConstRef`; both reproducers un-ignored |
| P117 | ~~Struct-text-param store leak~~                   | ~~Leak~~    | ~~Medium~~ | **Fixed** (2026-04-11) — GL-pattern tests pass in debug |
| P121 | ~~Float tuple heap corruption (interpreter)~~      | ~~Safety~~  | ~~**High**~~ | **Fixed** (2026-04-11) — sustained-loop tests pass in debug |
| P122 | ~~Store leak: struct/vector in tight game loops~~  | ~~Leak~~    | ~~**High**~~ | **Fixed** (2026-04-11) — mat4 + collision GL tests pass in debug |
| P123 | ~~Per-frame vector literal allocation leaks~~      | ~~Leak~~    | ~~Medium~~ | **Fixed** (2026-04-11) — multi-vector GL tests pass in debug |
| P126 | ~~Negative integer tail expression~~               | ~~Parser~~  | ~~Low~~  | **Fixed** (2026-04-11) — test un-ignored |
| P133 | ~~RGB↔BGR channel swap in GL output~~              | ~~Library~~ | ~~Low~~  | **Fixed** — Xvfb/Mesa capture artifact; `snap_smoke.sh` swaps channels post-`import`; golden regenerated. |
| P135 | Sprite atlas row indexing swap                     | Library     | Low      | Open — cosmetic |

---

## Methodology

Every issue follows this sequence. Do NOT skip Phase D.

### Phase A — Minimal reproduction from GL

Take the failing GL example and strip it to the smallest loft program
that still triggers the bug. Remove all rendering, input handling, and
game logic. Keep only the data structures and the operation sequence
that causes the failure.

**Tools:**

```bash
# Run a GL example under Xvfb with backtrace
RUST_BACKTRACE=1 xvfb-run -a target/release/loft --interpret \
    --path $(pwd)/ --lib $(pwd)/lib/ lib/graphics/examples/<example>.loft

# Execution trace (last 50 opcodes before crash)
LOFT_LOG=crash_tail:50 xvfb-run -a target/release/loft --interpret \
    --path $(pwd)/ --lib $(pwd)/lib/ lib/graphics/examples/<example>.loft
```

### Phase B — Validate outside GL

Convert the minimal reproduction into a pure-loft unit test in
`tests/issues.rs` using the `code!()` macro. No GL, no Xvfb, no native
cdylib. The test must trigger the **same symptom** (panic message, leak
warning, or incorrect value) as the GL version.

If it does NOT reproduce outside GL, the bug is in the native GL bridge
or in the interaction between native cdylib functions and the interpreter's
store model — narrow the investigation to that boundary.

```bash
# Run the unit test
cargo test --release --test issues <test_name>

# With store-lifecycle diagnostics
LOFT_STORES=warn cargo test --release --test issues <test_name>

# With valgrind (heap-level verification)
valgrind --tool=memcheck --leak-check=full \
    target/debug/deps/issues-<hash> --test-threads=1 <test_name>
```

### Phase C — Drill to root cause

With a minimal reproducer in hand:

1. **Read the IR:** `LOFT_LOG=static` dumps the bytecode. Identify the
   exact opcode sequence that triggers the failure.
2. **Read the execution trace:** `LOFT_LOG=minimal` shows each opcode as
   it executes. Find where the invariant breaks.
3. **Read the source:** trace the Rust code path from the opcode handler
   through the store/database layer to the assertion or leak.
4. **Document the causal chain:** write it down as "A happens, which
   causes B, which causes C, which panics at file:line."

### Phase D — Analyse BEFORE attempting to fix

Before writing any fix code:

1. **Identify all callers** of the broken function/path. A fix that works
   for one caller but breaks another is worse than no fix.
2. **Check for existing guards** — the code may already have a
   `TODO`/`FIXME` or a `debug_assert` that anticipated this failure.
3. **Design the fix on paper.** Write the approach in
   [PROBLEMS.md](PROBLEMS.md) BEFORE touching any source file. Include:
   - What changes
   - What stays the same
   - What could break
   - How to verify it didn't break

### Phase E — Fix and test back into GL

1. Implement the fix.
2. Verify the unit test passes: `cargo test --release --test issues`.
3. Verify the GL reproduction passes under Xvfb.
4. Verify the full GL suite: `make test-gl-headless` with an empty
   `GL_HEADLESS_SKIP`.
5. Verify the golden image: `make test-gl-golden`.
6. Verify the full CI: `make ci`.
7. Run valgrind on the reproduction to confirm no new leaks.

---

## Per-issue plans

### P120 — Delete on locked store (CRITICAL — blocks all releases)

**Root cause:** `const` reference/vector parameters get their backing
store LOCKED at function entry (`src/parser/expressions.rs:163-178`)
via `n_set_store_lock(var, true)`. There is **no corresponding unlock
at function return.** After `render_frame(sc: const Scene, cam)` returns,
`sc`'s store stays locked. The next loop iteration's
`sc.nodes[0].transform = mat4_trs(...)` calls `OpCopyRecord` →
`remove_claims` → `store.delete` on the locked store → panic at
`src/store.rs:357`.

**Phase A — Minimal GL reproduction:**

```loft
use render; use scene; use mesh; use math;
fn main() {
  sc = scene::Scene { name: "p120" };
  sc.add_mesh(mesh::cube());
  sc.add_material(scene::material_color("c", 0.5, 0.5, 0.5));
  sc.add_node(scene::node_at("n", 0, 0, math::mat4_identity()));
  sc.add_light(scene::directional_light("l", 1.0, 1.0, 1.0, 2.0,
    math::normalize3(math::vec3(-1.0, -1.0, -1.0))));
  r = render::create_renderer(400, 300, "p120");
  cam = scene::Camera { name: "c", fov: 45.0, near: 0.1, far: 10.0,
    position: math::vec3(0.0, 0.0, 3.0),
    target: math::vec3(0.0, 0.0, 0.0) };
  for _ in 0..10 {
    sc.nodes[0].transform = math::mat4_rotate_y(0.1); // panics iter 2
    if !r.render_frame(sc, cam) { break; }
  }
  r.destroy();
}
```

**Phase B — Unit test without GL:**

```loft
struct Inner { data: vector<integer> }
struct Outer { items: vector<Inner> }
fn read_only(c: const Outer) -> integer { c.items[0].data[0] }
fn test() {
  c = Outer { items: [] };
  c.items += [Inner { data: [10, 20, 30] }];
  assert(read_only(c) == 10, "first call");
  c.items[0].data = [40, 50, 60];       // should NOT panic
  assert(read_only(c) == 40, "second call");
  c.items[0].data = [70, 80, 90];       // should NOT panic
  assert(c.items[0].data[0] == 70, "third reassign");
}
```

**Phase C — Root cause files:**

| File | Line | What |
|------|------|------|
| `src/parser/expressions.rs` | 163-178 | Where `n_set_store_lock(var, true)` is emitted at function entry for const params |
| `src/scopes.rs` | 679+ | `get_free_vars` — where unlock SHOULD be emitted at exit but **isn't** |
| `src/database/allocation.rs` | 816-817 | `remove_claims` → `store.delete(cur)` on the locked store |
| `src/store.rs` | 355-357 | The assertion that fires |
| `src/codegen_runtime.rs` | 1271-1277 | `n_set_store_lock` runtime implementation |

**Phase D — Fix design:**

Emit `n_set_store_lock(var, false)` for each const reference/vector
parameter at every function exit point. Add to `scopes.rs::get_free_vars`
alongside the existing `OpFreeRef` / `OpFreeText` emission:

```rust
// In get_free_vars, for each variable leaving scope:
if function.is_const_param(v) && function.is_argument(v)
    && matches!(function.tp(v), Type::Reference(_, _) | Type::Vector(_, _))
{
    let lock_fn = data.def_nr("n_set_store_lock");
    if lock_fn != u32::MAX {
        ls.push(Value::Call(lock_fn, vec![Value::Var(v), Value::Boolean(false)]));
    }
}
```

**Risk — re-entrant const borrows:** if function A locks store S and
calls function B which also takes `const S`, B's unlock would release
A's lock prematurely. May need a lock COUNTER instead of a boolean.
Check whether any existing test exercises this pattern; if so, implement
a counter in `Store::lock()`/`unlock()`.

**Phase E — Verification:**

```bash
cargo test --release --test issues p120        # unit test
make test-gl-headless                          # all 26 examples — GL_HEADLESS_SKIP must be EMPTY
make test-gl-golden                            # golden image
make ci                                        # full gate
```

---

### P122 — Store leak in tight game loops

**Symptom:** `Allocating a used store` panic after ~30-60 seconds in a
game loop that creates struct instances per frame (collision Boxes).

**Phase A — Minimal reproduction:**

```loft
struct V { x: float not null, y: float not null }
fn make(a: float, b: float) -> V { V { x: a, y: b } }
fn test() {
  for p122_long in 0..50000 {
    v = make(p122_long as float, 0.0);
    if v.x < 0.0 { break; }
  }
}
```

Existing tests pass at ~500 iterations but leak stores — the pool just
isn't exhausted yet. Run at 50k to trigger the panic.

**Phase B — Validate:**

```bash
LOFT_STORES=warn cargo test --release --test issues p122_long_running -- --ignored
```

Count stores allocated vs freed per iteration. If stores grow
monotonically, that's the leak.

**Phase C — Root cause:**

Struct-returning functions allocate a callee store for the return value.
`gen_set_first_ref_call_copy` (`src/state/codegen.rs`) deep-copies the
struct to the caller's store. But the callee's store is never freed:

- `is_ret_work_ref` suppresses `OpFreeRef` on the work variable
- The O-B2 adoption path skips the callee store when no reference params
  exist, but doesn't clean up the `__ref_N` work ref's store

**Phase D — Fix design:**

In `gen_set_first_at_tos` (`src/state/codegen.rs`), after the O-B2
adoption copy is complete, emit `OpFreeRef` for the `__ref_N` work
variable that was allocated by `add_defaults` but bypassed by the
adoption path.

**Phase E — Verification:**

```bash
cargo test --release --test issues p122        # all p122 variants
LOFT_STORES=warn cargo test … p122_long_running -- --ignored  # stable store count
valgrind … target/debug/deps/issues-* p122_long_running       # no OS-level leak
make ci
```

After fixing: brick-buster can replace bitmask/raw-float workarounds with
idiomatic struct-based collision code.

---

### P127 — File-scope vector constant corrupts caller slots

**Symptom:** codegen panic — `[generate_set] first-assignment of 'n'
contains a Var(0) self-reference`.

**Phase A — Already has a minimal reproducer:**

```loft
QUAD = [1, 2, 3];
fn count(v: const vector<integer>) -> integer { v.len() }
fn test() {
  n = count(QUAD);
  assert(n == 3, "got {n}");
}
```

**Phase B — Already a unit test:**
`tests/issues.rs::p127_file_scope_vector_constant_in_call` (now passing,
`#[ignore]` removed).

**Phase C — Root cause (confirmed):**

`parse_constant` (`src/parser/definitions.rs:416`) stores the vector
literal as a `Value::Block` with `var_size: 0` and `Var(0)`/`Var(1)`
temporaries. When the constant is referenced (`src/parser/objects.rs:467`),
the Block is cloned verbatim into the calling function's IR — its `Var`
indices collide with the caller's local variables.

**Phase D — Fix design:**

At the constant reference site (`src/parser/objects.rs:467`), walk the
cloned IR and remap each `Var(i)` to `Var(caller_var_count + i)`, then
bump the caller's `var_count` by the constant's `var_size`. This requires
also fixing `v_block` (`src/data.rs:798`) to track the correct
`var_size` (currently hardcoded to 0).

Alternative: re-emit the literal at each reference site with fresh var
numbers, bypassing the clone entirely.

**Phase E — Verification:**

```bash
cargo test --release --test issues p127        # both tests run unconditionally
make ci
```

---

### P117 / P121 — Verify "appears fixed" issues definitively

**Status:** Valgrind shows 0 OS-level leaks. Regression-guard tests
pass. The original symptoms haven't been re-validated under their
original conditions.

**P117 — Struct-text-param store leak:**

```bash
# Run a file()-style pattern under LOFT_STORES=warn for 1000 iterations
LOFT_STORES=warn cargo test --release --test issues p117
```

If no "Database N not correctly freed" warnings appear → mark as
~~Fixed~~ in PROBLEMS.md.

**P121 — Float tuple heap corruption:**

```bash
# Run under valgrind in a debug build
valgrind --tool=memcheck target/debug/loft --interpret \
    --path $(pwd)/ tests/scripts/50-tuples.loft
```

If no heap corruption / SIGSEGV → mark as ~~Fixed~~ in PROBLEMS.md.

If either fails, re-open and debug per the full A→E methodology.

---

### P133 — RGB↔BGR channel swap — **Fixed**

**Root cause:** Xvfb + Mesa-swrast framebuffer stores pixels in an
order that ImageMagick's `import` reads with R and B swapped.  The
on-screen render and the native `loft_gl_clear`/`loft_gl_upload_canvas`
paths were always correct — only Xvfb-captured PNGs exhibited the swap
(verified: `convert golden.png -format "%[pixel:p{5,5}]"` showed
`srgb(35,25,20)` for an input of `rgba(20, 25, 35, 255)`).

**Fix:** `tests/scripts/snap_smoke.sh` now applies
`convert ... -separate -swap 0,2 -combine` to the captured screenshot
before handing it to the golden comparison.  The golden PNG was
regenerated in-place so it holds the colours the loft program actually
requested.

**Verification:** `make test-gl-golden` reports `0 px differ` against
the (corrected) golden PNG.

---

### P135 — Sprite atlas row indexing swap

**Phase A — Already reproduced:** smoke test pixel sampling confirms
sprites 1 and 3 are at wrong canvas positions in the 2×2 atlas.

**Phase B — Validate:** the smoke test IS the non-GL validation.

**Phase C — Root cause:** the interaction between `gl_upload_canvas`'s
Y-flip (row reversal during upload, `lib.rs:837`) and `draw_sprite`'s
V-coordinate computation (`graphics.loft:773-776`). The orthographic
projection in `create_painter_2d` also flips Y (`-2/H` in the
projection matrix). The combination creates a double-flip for certain
sprite indices.

**Phase D — Fix:** trace the full coordinate chain from canvas pixel to
screen pixel. The fix is either removing the upload flip (and adjusting
`TEX_VERT_2D`'s shader which already has its own V-flip) OR inverting
the V computation in `draw_sprite`. Must not break brick-buster's existing
sprite atlas layout.

---

## Execution order

Priority based on severity and blocking impact:

1. **P120** — blocks all releases (zero-regressions rule)
2. **P122** — blocks idiomatic game code for 0.8.4
3. **P127** — blocks file-scope vector constants
4. **P117 / P121 verification** — close or reopen definitively
5. **P133** — determine if real bug or Xvfb artifact
6. **P135** — cosmetic sprite ordering

For each issue, follow Phases A → B → C → D → E strictly. Do NOT skip
Phase D (analyse before attempting to fix). Document the root cause in
[PROBLEMS.md](PROBLEMS.md) BEFORE writing any fix code.

---

## Final verification gate

After ALL fixes land:

```bash
make ci                       # full test suite (643+ tests + packages + GL smoke + golden)
make test-gl-headless         # all GL examples — GL_HEADLESS_SKIP must be EMPTY
make test-gl-golden           # golden image pixel-for-pixel comparison
valgrind --leak-check=full \  # no OS-level leaks on the brick-buster sim
    target/debug/loft --interpret … brick_buster_headless.loft
```

**No release ships until all four commands pass clean.**

---

## See also

- [PROBLEMS.md](PROBLEMS.md) — individual bug entries with reproducers, fix paths, and status
- [TESTING.md](TESTING.md) § Headless OpenGL testing — Xvfb + screenshot pipeline
- [ROADMAP.md](ROADMAP.md) § Zero-regressions rule — the release gate that requires this work
