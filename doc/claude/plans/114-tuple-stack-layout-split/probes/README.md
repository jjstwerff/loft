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
| `int_ref_int` | `(integer, P, integer)` — ref in the **middle** | `1,5,9` | pass — **by coincidence** |
| `ref_int_ref` | `(P, integer, P)` | `5,9,7` | pass — **by coincidence** |
| `ref2_field` | `(P,P)` into a **struct field** | `10,20` | pass |
| `ref2_return` | `(P,P)` **returned** from a fn | `10,20` | pass |
| `tupleput_ref` | `(P,P)` then `t.0 = c` (**TuplePut**) | `30,20` | pass |
| `ref2` | `(P, P)` | `10,20` | **FAIL(w) 24 v 32** |
| `ref3` | `(P, P, P)` | `1,2,3` | **FAIL(w) 40 v 48** |
| `ref4` | `(P, P, P, P)` | `1,2,3,4` | **FAIL(w) 48 v 64** |
| `vec2` | `(vector, vector)` | `7,9` | **FAIL(w) 24 v 32** |
| `ref_float` | `(P, float)` — next elem aligns **8** | `10,2.5` | pass |
| `ref_bool` | `(P, boolean)` — next elem aligns **1** | `10,true` | **SEGV** |
| `ref_char` | `(P, character)` — next elem aligns **4** | `10,A` | **broken** |
| `ref_text` | `(P, text)` | `10,x` | **SEGV (interpret only)** |
| `fn_text_read` | `(fn, text)`, reads `t.1` only | `tag` | **SEGV (interpret only)** |
| `fn_text_call` | `(fn, text)`, reads `t.1` + calls `t.0` | `tag` then `49` | **SEGV (interpret only)** |

**Every failure here is `--interpret` only; `--native` is correct for all 27 cells.**
Beware a trap this corpus already sprang once: under the debug-assertions build the
width mismatch is reported for `--native` too, because the assert lives in
`generate_call` (codegen), which runs before either backend executes. That is *not*
a native runtime failure. Check runtime behaviour with the release binary.

`ref2_min` / `fn_text_call_min` are the two originals reduced from
`tests/tuple_matrix.rs::e5_d2_struct_ref_arg` and `::e4_d2_closure_arg` — keep them
byte-stable so the plan's fix can be checked against the exact shapes CI runs.

## How to read the two failure families

**Width family** (`ref2`, `ref3`, `ref4`, `vec2`) — packed vs stepped element
advance. Packed `DbRef` is 12B, stepped is 16B, so the deficit is 4B per reference
and scales linearly with arity: 24 v 32 at n=2, 40 v 48 at n=3, 48 v 64 at n=4. No
fixed header term.

**Only the call-argument destination is broken.** `ref2_local`, `ref2_field`,
`ref2_return` and `tupleput_ref` all place two references correctly — so the packed
layout is not just *declared*, it is what every other destination already writes and
reads. That is why @PLN114 fixes the argument push instead of rewriting the 26
`element_offsets` sites.

The coincidence cells (`ref_int`, `int_ref`, `int_ref_int`, `ref_int_ref`) are two
errors cancelling: `(P,integer)` declares 12+8 = 20 → padded to 24, and pushes
16+8 = 24; `(P,integer,P)` declares 36 → 40 and pushes 16+8+16 = 40. **Those rows
are the canaries** — any layout change moves them, and they must stay green with
hand-checked values, never via a compensating adjustment.

**Text family — RESOLVED (step 2): the same bug, and `text` was never the axis.**
What decides a cell is the **packed alignment of the element following a sub-8-byte
one**. `element_align` (`data.rs:1977`) gives `Str` 4, `boolean` 1, `character` 4,
while `integer`/`float`/`Function` are 8. After a `Reference` (12B, align 4):

| next element | packed offset of element 1 | push lands at | cell | outcome |
|---|---|---|---|---|
| `float` (align 8) | 12 → **16** | 16 | `ref_float` | pass |
| `integer` (align 8) | 12 → **16** | 16 | `ref_int` | pass |
| `boolean` (align 1) | **12** | 16 | `ref_bool` | **SIGSEGV** |
| `character` (align 4) | **12** | 16 | `ref_char` | **broken** |
| `text` (align 4) | **12** | 16 | `ref_text` | **SIGSEGV** |
| `P` (align 4) | **12** | 16 | `ref2` | **broken** |

`ref_bool` is the decisive cell: no `text` anywhere and it fails identically. The
lifetime hypothesis is dead — `(text,text)` and `(integer,text)` both pass, so text
elements are not inherently unsafe.

<details><summary>the earlier "probably a different bug" reasoning (superseded)</summary>
 The widths already agree (`(P,text)`: packed 12 → text aligns 8 → offset 16,
total 32; pushed 16+16 = 32) and it still segfaults, and no assert fires.
`Str { *const u8, u32 }` is a raw pointer, so interpreter-side element lifetime (a
dangling `ptr` after the source is freed) is the likely mechanism. Plan step 2
confirms with `LOFT_LOG=ref_debug` + `--show-ownership`; expect a separate plan. Do
not fold them in because they share a test file — and do not use "it's
interpret-only" as the argument, because so is everything else here.

</details>

`fn2` is worth keeping for a different reason: my first hand-computed expectation for
it was wrong (13; the answer is `3*3 + 4+1` = 14) and loft was right. The cell earns
its place by having caught the *reviewer*.

## Silent corruption, not just crashes

`ref4` on `--interpret` with a **release** binary prints
`2,null,0,360287970323857408` instead of `1,2,3,4`. It does not crash. A 4-element
tuple of references passed as an argument silently returns garbage — including a
`null` where a live reference should be.

That is the strongest argument for plan step 1 (make the mismatch loud) and step 7
(un-ignore these suites): the crashing cells were merely the loud members of a family
whose quiet members return wrong answers.

Note the counts differ by build — `run.sh` reports 11 differing rows on the release
binary (of 54 cell/backend combinations) and more on the @PLN85 debug-assertions build. The release number is the
**under**-report: without the asserts, a width mismatch either segfaults or silently
corrupts, and the corrupting cells look like ordinary wrong answers.

## Running

```bash
./run.sh        # release build, both backends
./run.sh ../../../../../target-da/release/loft   # @PLN85 build: widths attribute themselves
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
