# @PLN114 probe corpus — tuple element placement

The boundary matrix behind the plan. Each cell varies **one** axis of a tuple
(element kind, position, arity, destination) and prints a value that was
hand-computed before the run — agreement between two binaries is not a pass.

Run them with `./run.sh` (see below). Everything here is the state as of
2026-07-21, before any fix.

## The cells

`FAIL(w)` = width mismatch reported by the `generate_call` assert under a
debug-assertions build (`expected` vs `pushed`). `SEGV` = SIGSEGV with no assert.
Every cell passes the tuple as a **call argument** unless noted.

| probe | tuple | expected output | result |
|---|---|---|---|
| `int2` | `(integer, integer)` | `11,22` | pass |
| `text2` | `(text, text)` | `aa,bb` | pass |
| `int_text` | `(integer, text)` | `11,bb` | pass |
| `nested` | `((integer,integer), integer)` | `1,2,3` | pass |
| `ref_int` | `(P, integer)` | `10,22` | pass — **by coincidence** |
| `int_ref` | `(integer, P)` | `22,10` | pass — **by coincidence** |
| `vec_int` | `(vector<integer>, integer)` | `7,22` | pass — **by coincidence** |
| `fn_int_call` | `(fn, integer)`, calls `t.0(7)` | `5` then `49` | pass |
| `fn2` | `(fn, fn)`, calls both | `14` | pass |
| `ref2_local` | `(P, P)` as a **local**, not an arg | `10,20` | pass |
| `ref2` | `(P, P)` | `10,20` | **FAIL(w) 24 v 32** |
| `ref3` | `(P, P, P)` | `1,2,3` | **FAIL(w) 40 v 48** |
| `vec2` | `(vector, vector)` | `7,9` | **FAIL(w) 24 v 32** |
| `ref_text` | `(P, text)` | `10,x` | **SEGV (interpret only)** |
| `fn_text_read` | `(fn, text)`, reads `t.1` only | `tag` | **SEGV (interpret only)** |
| `fn_text_call` | `(fn, text)`, reads `t.1` + calls `t.0` | `tag` then `49` | **SEGV (interpret only)** |

The width family mismatches on **both** backends; the text family crashes on
`--interpret` and produces the **correct** output on `--native`. That split is the
main reason the plan treats them as separate defects — see below.

`ref2_min` / `fn_text_call_min` are the two originals reduced from
`tests/tuple_matrix.rs::e5_d2_struct_ref_arg` and `::e4_d2_closure_arg` — keep them
byte-stable so the plan's fix can be checked against the exact shapes CI runs.

## How to read the two failure families

**Width family** (`ref2`, `ref3`, `vec2`) — packed vs stepped element advance.
Packed `DbRef` is 12B, stepped is 16B, so the deficit is 4B per reference and scales
with arity (24 v 32 at n=2, 40 v 48 at n=3). The passing mixed cells are two errors
cancelling: `(P,integer)` declares 12+8 = 20 → stepped to 24, and pushes 16+8 = 24.
**Those three "pass" rows are the canaries** — any layout change moves them, and they
must stay green with hand-checked values, not by a compensating adjustment.

**Text family** (`ref_text`, `fn_text_read`, `fn_text_call`) — *a different bug*.
The widths already agree (`(P,text)`: packed 12 → text aligns 8 → offset 16, total
32; pushed 16+16 = 32) and it still segfaults. Decisively, it segfaults on
`--interpret` and returns the **correct** value on `--native`: both generators read
the same IR and the same `element_offsets`, so a fault only one of them shows cannot
be in the layout they share. `Str { *const u8, u32 }` is a raw pointer, so
interpreter-side element lifetime (a dangling `ptr` after the source is freed) is
the likely mechanism. Plan step 2 confirms with `LOFT_LOG=ref_debug` +
`--show-ownership`; expect a separate plan. Do not fold them in because they share a
test file.

`fn2` is worth keeping for a different reason: my first hand-computed expectation for
it was wrong (13; the answer is `3*3 + 4+1` = 14) and loft was right. The cell earns
its place by having caught the *reviewer*.

## Running

```bash
./run.sh                          # release build, both backends
./run.sh ../../../../target-da/release/loft   # debug-assertions build: widths attribute themselves
```

A plain build compiles every lib-side `debug_assert!` out
(`[profile.dev.package.loft] debug-assertions = false`), so the width mismatches are
**silent** there — they only segfault. Use the @PLN85 calibration build to see the
`expected NB … pushed MB` message. Build it in a separate target dir; never set
`RUSTFLAGS` against the main `target/`:

```bash
mkdir -p target-da/release && ln -sfn ../../default target-da/release/default
RUSTFLAGS="-C debug-assertions=on" CARGO_TARGET_DIR=target-da cargo build --release --bin loft
```

Leak checking needs `--interpret` — a bare `loft` is native and skips
`check_store_leaks`.
