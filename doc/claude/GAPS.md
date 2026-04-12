# Test Coverage Gaps

Standalone catalogue of untested or under-tested code paths, lifted
from `TESTING.md` for easier iteration.  Update this file whenever a
gap is closed or a new untested path is discovered.

Last refresh: 2026-04-12.  Baseline: 71.3 % line / 74.9 % function.

## Files with 0 % or critically low coverage

| File | Line % | Key gaps |
|---|---|---|
| `src/documentation.rs` | 0 % | HTML doc generation — exercised only by `gendoc` binary, not by unit tests |
| `src/radix_tree.rs` | 0 % | Planned feature, currently unused |
| `src/native_utils.rs` | 12 % | WASM path resolution, installed-layout fallback |
| `src/database/allocation.rs` | 39 % | Store growth, boundary conditions |
| `src/logger.rs` | 39 % | Production mode, rotation, `from_config_file` happy-path |
| `src/extensions.rs` | 46 % | Plugin dedup, library-load failure modes |
| `src/variables/validate.rs` | 46 % | Scope cycle detection, sibling conflicts, `short_type` |
| `src/database/search.rs` | 47 % | Multi-key range queries |

## Priority additions

Ordered by impact × feasibility:

1. **Logger production mode + `from_config_file`** — already testable via
   `tests/logger_severity.rs`; no interpreter needed.  Production mode
   turns panics into `had_fatal`, which is the public API for native
   targets and currently has zero direct coverage.
2. **Database store boundaries** — extend `tests/data_structures.rs`
   with record-count at the resize threshold (`alloc_records` > 32 K).
3. **Multi-key range queries on sorted/index collections** — scriptable
   as a new `tests/scripts/` file.
4. **Parser stress / error recovery** — create `tests/parser_stress.rs`
   feeding malformed sources and checking the recovery path catches
   them without panicking.
5. **`variables/validate.rs` helper coverage** — synthetic IR tests
   targeting `build_scope_parents` and `scopes_can_conflict`.

## Features covered only in Rust integration tests (no `.loft` script)

Not a gap per se — documented so future authors know where to look:

| Feature | File |
|---|---|
| Parallel worker API | `tests/threading.rs` |
| Stores / tree / hash directly | `tests/data_structures.rs` |
| Logger severity routing | `tests/logger_severity.rs` |
| Codegen invariants | `tests/issues.rs` |
| Formatter roundtrips | `tests/format.rs` |
| Native compilation pipeline | `tests/native.rs` |
| WASM compilation | `tests/wasm_entry.rs` |

## How to refresh

```bash
cargo install cargo-llvm-cov   # one-time
rustup component add llvm-tools-preview
cargo llvm-cov --release --open
```

Known gotcha: tests that spawn the `loft` binary via `CARGO_BIN_EXE`
assume `default/*.loft` is findable relative to the binary; under
`cargo-llvm-cov`'s isolated target dir they panic.  Skip them with
`--skip exit_codes` when running coverage, or set `LOFT_DEFAULT_DIR`
once such an env override exists.
