
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
- **Benchmarks exist**: `bench/run_bench.sh` with 10 test cases

### Architecture

Both modes share the same pipeline up to bytecode compilation:

```
Parse → Scopes → Bytecode compile → Extensions loaded
                                     ↓
                        ┌────────────┴────────────┐
                        ↓                         ↓
              Native codegen (1645)      Interpreter (1912)
              Output::output_native()    state.execute_argv()
              → Rust source → rustc     → Dispatch loop
              → Binary → Execute
```

Divergence: `main.rs:1645` checks `native_mode`.

---

## Step 1: Fix package path resolution

### Problem

`loft --lib lib --native /tmp/test.loft` with `use random` fails:
"Unknown function rand". The `make test-packages` target works because
it uses `loft test` (auto-detects `loft.toml` and adds `src/` to
lib_dirs).

### Root cause

The `--lib lib` flag pushes the RELATIVE path `"lib"` to `lib_dirs`
(main.rs:1153). The parser's `lib_path()` (mod.rs:2052-2170) searches
`lib_dirs` for `<dir>/<id>.loft` and `<dir>/<id>/src/<id>.loft`. But
relative paths break when the parser's working directory differs from
the CLI's.

### Design

**Option A: Resolve `--lib` paths to absolute** (recommended)

In `main.rs` after flag parsing (before line 1510), canonicalize all
`lib_dirs` entries:

```rust
let lib_dirs: Vec<String> = lib_dirs
    .into_iter()
    .map(|d| std::fs::canonicalize(&d)
        .unwrap_or_else(|_| std::path::PathBuf::from(&d))
        .to_string_lossy()
        .into_owned())
    .collect();
```

**Option B: Auto-add project lib/ to search path**

When the source file is inside a project directory (has `loft.toml`
or a `lib/` sibling), automatically add `lib/` to `lib_dirs`. The
`test` subcommand already does this (main.rs:1249-1261).

**Recommendation: Do both.** Option A fixes the immediate bug. Option B
makes `use` work without explicit `--lib` flags.

### Files

- `src/main.rs:1153-1155` (--lib parsing)
- `src/main.rs:1450-1510` (lib_dirs setup before parser)
- `src/parser/mod.rs:2052-2170` (lib_path search)

### Verification

```bash
cargo run --bin loft -- --lib lib /tmp/test.loft           # interpreter
cargo run --bin loft -- --lib lib --native /tmp/test.loft   # native
```

Both must resolve `use random` and run successfully.

---

## Step 2: Wire `--native` as default

### Design

**main.rs changes (lines 1100-1210):**

1. Initialize `native_mode = true` (was `false`)
2. Add `--interpret` flag:
   ```rust
   } else if a == "--interpret" || a == "--bytecode" {
       native_mode = false;
   }
   ```
3. Keep `--native` as no-op (already default)

**Rustc fallback (before line 1645):**

Check for rustc before attempting native compilation. If missing,
fall back to interpreter:

```rust
if native_mode {
    // Check rustc availability before committing to native path
    match std::process::Command::new("rustc").arg("--version").output() {
        Ok(_) => {} // proceed with native
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            eprintln!("Warning: rustc not found, falling back to interpreter");
            native_mode = false;
        }
        Err(e) => {
            eprintln!("Warning: rustc check failed ({e}), falling back to interpreter");
            native_mode = false;
        }
    }
}
```

This goes BEFORE the native codegen block (line 1645) but AFTER
bytecode compilation (line 1526), so the interpreter path is ready.

**Help text update:**

```
loft [options] <file>
  Native compilation is the default. Use --interpret for bytecode mode.
  
  --interpret          run in interpreter/bytecode mode instead of native
  --native-release     native compilation with optimizations
  --native-emit <file> generate Rust source without compiling
```

### Files

- `src/main.rs:1100-1210` (flag handling)
- `src/main.rs:1645` (native mode check)
- `src/main.rs:1870-1936` (help text)

### Verification

```bash
cargo run --bin loft -- program.loft          # runs native (default)
cargo run --bin loft -- --interpret prog.loft  # runs interpreter
# On a system without rustc:
cargo run --bin loft -- program.loft          # falls back to interpreter
```

---

## Step 3: Validate packages in native mode

### Design

**New Makefile target:**

```makefile
test-packages-native:
	@pass=0; fail=0; total=0; \
	for pkg in lib/*/; do \
	  for f in $$pkg/src/*.loft $$pkg/tests/*.loft; do \
	    [ -f "$$f" ] || continue; \
	    total=$$((total + 1)); \
	    if $(LOFT) --native "$$f" 2>&1 | grep -q "^Error\|panicked"; then \
	      echo "  FAIL $$f"; fail=$$((fail + 1)); \
	    else \
	      echo "  ok $$f"; pass=$$((pass + 1)); \
	    fi \
	  done \
	done; \
	echo "$$total package tests, $$fail failed"
```

**Expected issues and fixes:**

| Package | #native funcs | Status | Action |
|---------|--------------|--------|--------|
| random | 3 | Built-in (`n_rand` etc.) | Should work |
| graphics | 45 | Has rlib + `[native] crate` | Test linking |
| server | 12 | Has `#native` | Needs `[native] crate` in loft.toml |
| crypto | 6 | Has `#native` | Needs `[native] crate` in loft.toml |
| imaging | 2 | Has `#native` | Needs `[native] crate` in loft.toml |
| web | 2 | Has `#native` | Needs `[native] crate` in loft.toml |
| shapes | 0 | Pure loft | Should work |
| arguments | 0 | Pure loft | Should work |

Packages missing `[native] crate = "..."` in loft.toml will get
`todo!("native function ...")` stubs. Add the crate field for each.

### Files

- `Makefile` (new target)
- `lib/*/loft.toml` (add `[native] crate` where missing)

### Verification

```bash
make test-packages          # interpreter: 16/16
make test-packages-native   # native: 16/16
```

---

## Step 4: Game validation

### Design

Test the Breakout game in native mode:

```bash
cargo run --bin loft -- --native lib/graphics/examples/25-breakout.loft
```

### Requirements

The graphics package has 45 `#native` functions and a compiled rlib.
The `loft.toml` already has `[native] crate = "loft-graphics-native"`.
The codegen should emit `loft_graphics_native::symbol()` calls via
`output_native_direct_call`.

### Expected issues

1. **OpenGL context**: Native binary needs the same GL context setup
   as the interpreter. The `gl_create_window` native function must
   link correctly.
2. **Frame yield**: The interpreter's `frame_yield` mechanism pauses
   at `gl_swap_buffers()`. Native code needs equivalent — probably
   a loop calling the swap function directly.
3. **Asset paths**: Texture/shader paths must resolve relative to the
   script, not the binary.

### Files

- `lib/graphics/native/src/lib.rs` (native GL bindings)
- `lib/graphics/src/graphics.loft` (45 #native declarations)
- `src/generation/dispatch.rs` (native call dispatch)

---

## Step 5: Performance baseline

### Design

Use the existing benchmark suite at `bench/run_bench.sh`:

```bash
cd bench && ./run_bench.sh
```

This runs 10 benchmarks comparing Python, loft interpreter, loft
native, and Rust reference implementations.

**Key metrics to validate:**

| Benchmark | Expected native/interpreter ratio |
|-----------|----------------------------------|
| Fibonacci | 10-50x faster |
| Sum loop | 20-100x faster |
| Sieve | 10-50x faster |
| String build | 2-10x faster |
| Matrix mul | 10-50x faster |

**If native is slower than expected:**

Profile with `RUSTFLAGS="-C debuginfo=2"` and `cargo flamegraph`.
Common issues: unnecessary store allocation, bounds checks in tight
loops, string allocation overhead.

### Files

- `bench/run_bench.sh`
- `doc/claude/PERFORMANCE.md` (update with results)

---

## Step 6: Documentation cleanup

### Changes

| File | Update |
|------|--------|
| `CLAUDE.md` | Key commands: remove `--native` from examples (it's default) |
| `doc/claude/DEVELOPMENT.md` | Native-first workflow |
| `doc/claude/PROBLEMS.md` | Mark P61 fixed, update P79 status |
| `doc/claude/NATIVE.md` | Update architecture for default mode |
| `CHANGELOG.md` | Native-as-default entry |
| `--help` output | "Native compilation is the default" |

---

## Risk assessment

| Risk | Mitigation |
|------|------------|
| rustc not installed | Auto-fallback to interpreter with warning |
| Compilation slow for large programs | Binary caching (already works) |
| Native binary larger than needed | `--native-release` strips + optimizes |
| Edge case fails only in native | Run both native + interpreter in CI |
| External crate version mismatch | Pin in loft.toml, validate at parse |
| WASM builds can't use native | WASM path is separate (`--native-wasm`) |

---

## Success criteria

1. `loft program.loft` compiles and runs natively by default
2. `loft --interpret program.loft` runs the interpreter
3. All 108 native tests pass
4. All 16 package tests pass in native mode
5. Breakout game runs natively with OpenGL
6. Graceful fallback when rustc is missing
7. No performance regression vs interpreter
