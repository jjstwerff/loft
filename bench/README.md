# Loft Benchmark Suite

Performance comparison across five targets: Python, loft interpreter, loft native, loft wasm, and Rust.

> **Not a CI suite** — run manually to compare performance.

## Benchmarks

| # | Name | Description |
|---|------|-------------|
| 01 | fibonacci | Recursive fib(38) |
| 02 | sum_loop | Sum 0..10,000,000 |
| 03 | sieve | Count primes to 100,000 (trial division) |
| 04 | collatz | Collatz sequence lengths 1..1,000,000 |
| 05 | mandelbrot | 200×200 Mandelbrot, 256 max iters |
| 06 | newton_sqrt | Newton's method sqrt, 1M calls |
| 07 | string_build | 5,000 string appends |
| 08 | word_count | Hash-based word frequency, 100K ops |
| 09 | matrix_mul | Float dot product, 1M elements |
| 10 | sort | Merge sort (index-bound), 3,000 integers |

## Targets

- **python** — CPython interpreter
- **loft-interp** — loft interpreter (`loft run`)
- **loft-native** — loft native binary (`loft build --native`)
- **loft-wasm** — loft WASM via wasmtime (`loft build --wasm`)
- **rust** — Rust release build (`rustc -O`)

## Prerequisites

- `loft` in PATH (with `--path` pointing to stdlib)
- `python3` in PATH
- `rustc` in PATH
- `wasmtime` in PATH (for wasm target)

## Usage

```bash
cd bench
./run_bench.sh                        # run all benchmarks, all targets
./run_bench.sh --only 01_fibonacci    # single benchmark
./run_bench.sh --skip-python          # skip Python
./run_bench.sh --skip-wasm            # skip wasm
./run_bench.sh --no-build             # skip compilation step
./run_bench.sh --warmup               # run once before timing
```

## Output

```
bench            python    loft-interp  loft-native  loft-wasm    rust
01_fibonacci     1823ms    612ms        18ms         22ms         8ms
...
```
