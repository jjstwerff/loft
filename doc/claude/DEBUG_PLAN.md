
// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

# Debugging plan: safety, data-loss, leak, and library issues

Systematic approach for reproducing and fixing runtime safety,
data-loss, leak, and library issues in the loft runtime.  Each issue
follows the same five-phase methodology: reproduce minimally, validate
outside GL, drill to root cause, analyse before fixing, fix and verify
back in the GL environment.

Completed fixes are removed — history lives in git and `CHANGELOG.md`.

---

## Issue inventory

| #    | Title                                | Category | Severity | Status |
|------|--------------------------------------|----------|----------|--------|
| P135 | Sprite atlas row indexing swap       | Library  | Low      | Open — cosmetic |

---

## Methodology

Every issue follows this sequence.  Do NOT skip Phase D.

### Phase A — Minimal reproduction from GL

Strip the failing GL example to the smallest loft program that still
triggers the bug.  Remove rendering, input, game logic.  Keep only the
data structures and the operation sequence that causes the failure.

```bash
# Run a GL example under Xvfb with backtrace
RUST_BACKTRACE=1 xvfb-run -a target/release/loft --interpret \
    --path $(pwd)/ --lib $(pwd)/lib/ lib/graphics/examples/<example>.loft

# Execution trace (last 50 opcodes before crash)
LOFT_LOG=crash_tail:50 xvfb-run -a target/release/loft --interpret \
    --path $(pwd)/ --lib $(pwd)/lib/ lib/graphics/examples/<example>.loft
```

### Phase B — Validate outside GL

Convert to a pure-loft unit test in `tests/issues.rs` via the
`code!()` macro.  No GL, no Xvfb, no native cdylib.  Must reproduce the
same symptom (panic / leak / wrong value) as the GL version.

If it does NOT reproduce outside GL, the bug is in the native GL bridge
or the native-cdylib ↔ interpreter store boundary — narrow the
investigation there.

```bash
cargo test --release --test issues <test_name>
LOFT_STORES=warn cargo test --release --test issues <test_name>
valgrind --tool=memcheck --leak-check=full \
    target/debug/deps/issues-<hash> --test-threads=1 <test_name>
```

### Phase C — Drill to root cause

1. Read the IR: `LOFT_LOG=static` dumps bytecode; find the exact
   opcode sequence.
2. Read the execution trace: `LOFT_LOG=minimal` shows each opcode as it
   runs; find where the invariant breaks.
3. Read the Rust path from the opcode handler through store/database
   to the assertion or leak.
4. Write the causal chain as "A → B → C → panic at file:line".

### Phase D — Analyse BEFORE fixing

1. Identify all callers of the broken path — fixing one and breaking
   another is worse than no fix.
2. Check for existing `TODO` / `FIXME` / `debug_assert` that anticipated
   this failure.
3. Design the fix on paper in [PROBLEMS.md](PROBLEMS.md) BEFORE touching
   source.  Include: what changes, what stays, what could break, how to
   verify nothing broke.

### Phase E — Fix and test back into GL

1. Implement.
2. Unit test passes: `cargo test --release --test issues`.
3. GL reproduction passes under Xvfb.
4. Full GL suite: `make test-gl-headless` with empty `GL_HEADLESS_SKIP`.
5. Golden image: `make test-gl-golden`.
6. Full CI: `make ci`.
7. Valgrind the reproduction — no new leaks.

---

## Open issue plans

### P135 — Sprite atlas row indexing swap

**Phase A — Already reproduced:** smoke test pixel sampling confirms
sprites 1 and 3 are at wrong canvas positions in the 2×2 atlas.

**Phase B — Validate:** the smoke test IS the non-GL validation.

**Phase C — Root cause:** interaction between `gl_upload_canvas`'s
Y-flip (row reversal during upload, `lib.rs:837`) and `draw_sprite`'s
V-coordinate computation (`graphics.loft:773-776`).  The orthographic
projection in `create_painter_2d` also flips Y (`-2/H`).  The
combination creates a double-flip for certain sprite indices.

**Phase D — Fix:** trace the full coordinate chain from canvas pixel
to screen pixel.  Either remove the upload flip (and adjust
`TEX_VERT_2D`'s shader which already has its own V-flip) or invert the
V computation in `draw_sprite`.  Must not break brick-buster's
existing sprite atlas layout.

---

## Final verification gate

After any runtime-safety fix lands:

```bash
make ci                       # full test suite
make test-gl-headless         # all GL examples — GL_HEADLESS_SKIP empty
make test-gl-golden           # golden image pixel-for-pixel
valgrind --leak-check=full \  # no OS-level leaks on the brick-buster sim
    target/debug/loft --interpret … brick_buster_headless.loft
```

---

## See also

- [PROBLEMS.md](PROBLEMS.md) — individual bug entries with reproducers
- [TESTING.md](TESTING.md) § Headless OpenGL testing — Xvfb + screenshot pipeline
- [ROADMAP.md](ROADMAP.md) § Zero-regressions rule
