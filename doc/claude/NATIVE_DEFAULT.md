
// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

# Native Code Generation: Path to Default

## Goal

Make `--native` the default execution mode for loft. Games will run
as compiled native binaries, not interpreted bytecode. The interpreter
remains available via `--interpret` for debugging and WASM builds.

---

## Current State (2026-04-07)

### What works

- **108/108 native tests pass** (29 docs + 79 scripts, 0 failures)
- **All language features**: structs, enums, match, closures, coroutines,
  tuples, generics, threading, file I/O
- **Binary caching**: FNV-1a hash, <200ms recompile on change
- **Codegen infrastructure for #native calls**: `output_native_direct_call`
  and `output_native_api_call` are implemented
- **Package rlibs exist**: `lib/graphics/native/target/release/` etc.
- **Linking flags**: `--extern` and `-L dependency` already wired

### What's blocking

| Blocker | Status | Effort |
|---------|--------|--------|
| ~~P61: text match coercion~~ | **Fixed** — stale skip entry removed | Done |
| P79: package function resolution in native mode | Infrastructure exists, CLI path issue | S |
| CLI default wiring | `--native` is opt-in, needs to be default | S |
| Package test validation | `make test-packages` needs native variant | M |
| Game validation | Breakout game in native mode | M |
| Performance baseline | Benchmark native vs interpreter | S |

---

## Step 1: Fix package path resolution (S)

### Problem

`cargo run --bin loft -- --lib lib --native program.loft` with
`use random` fails: "Unknown function rand". The same program works
via `make test-packages` (interpreter).

### Investigation needed

- How does `use <name>` resolve in the parser? (`src/parser/mod.rs`)
- How does `--lib` pass paths to the parser? (`src/main.rs`)
- How does `make test-packages` invoke loft differently?
- Does the `--path` flag set a root that includes `lib/`?

### Expected fix

The parser's `use` handler checks a list of search paths. The
`--lib` flag adds to this list. The issue is likely that `--lib lib`
adds `lib/` as a search directory but the `use random` handler looks
for `lib/random/src/random.loft` using a different base path.

### Files

`src/main.rs` (flag handling), `src/parser/mod.rs` (use resolution)

---

## Step 2: Wire `--native` as default (S)

### Changes

- `src/main.rs`: Change `native_mode` default from `false` to `true`
- Add `--interpret` flag (or `--bytecode`) for explicit interpreter mode
- Keep `--native` flag for backwards compatibility (no-op when default)
- Update `--help` text

### Fallback

If rustc is not available, fall back to interpreter with a warning:
```
Warning: rustc not found, falling back to interpreter mode
```

---

## Step 3: Validate packages in native mode (M)

### Approach

Run `make test-packages` equivalent with `--native` flag. Each
package test script should compile to a native binary and produce
the same output as the interpreter.

### Expected issues

- `#native` functions in graphics/server/crypto need their rlibs
  linked via `--extern`
- Some packages may use patterns not yet in native codegen
- The `loft.toml` manifests need `[native] crate = "..."` entries

### New target

```makefile
test-packages-native:
	@for f in $(PACKAGE_TESTS); do \
	  $(LOFT) --native $$f && echo "  $$f  ok" || echo "  $$f  FAIL"; \
	done
```

---

## Step 4: Game validation (M)

### Breakout game

`lib/graphics/examples/25-breakout.loft` — first playable demo.
Must compile natively and run with OpenGL rendering.

### Requirements

- Graphics package `#native` functions compile and link
- `gl_create_window`, `gl_swap_buffers`, `gl_draw` etc. work
- Frame timing (`time_ticks`) works
- Input handling works

### Test

```bash
cargo run --bin loft -- --native lib/graphics/examples/25-breakout.loft
```

---

## Step 5: Performance baseline (S)

### Benchmark

Compare interpreter vs native on a compute-heavy loft program
(e.g., recursive fibonacci, matrix multiplication, sort).

### Expected result

Native should be 10-100x faster than interpreter for compute-bound
work. The interpreter's overhead is bytecode dispatch + store
indirection. Native eliminates both.

### If native is slower

Investigate: the generated Rust code may have unnecessary
allocations, bounds checks, or store copies that the optimizer
can't eliminate. Profile with `cargo flamegraph`.

---

## Step 6: Documentation and cleanup (S)

- Update `CLAUDE.md` key commands section
- Update `--help` output
- Update `DEVELOPMENT.md` with native-first workflow
- Move P61/P79 to "fixed" in PROBLEMS.md
- Add native compilation notes to RELEASE.md

---

## Risk assessment

| Risk | Mitigation |
|------|------------|
| rustc not installed | Fallback to interpreter with warning |
| Compilation slow for large programs | Binary caching (already works) |
| Native binary larger than needed | Release mode, strip symbols |
| Some edge case fails only in native | Keep interpreter tests, run both in CI |
| External crate version mismatch | Pin versions in loft.toml |

---

## Success criteria

1. `cargo run --bin loft -- program.loft` compiles and runs natively by default
2. All 108 native tests + 16 package tests pass
3. Breakout game runs natively
4. `--interpret` flag available for debugging
5. No performance regression vs interpreter
