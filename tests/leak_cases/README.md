<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Cross-backend leak cases

Each `*.loft` file here is a self-contained leak case: it defines
`fn test()` (no `main` — the harness appends one) using only built-ins
and its own struct/enum declarations, so it needs no library on the
load path.

The harness in [`tests/leak_cases.rs`](../leak_cases.rs) runs **every**
file through both the interpreter (`--interpret`) and the native
Rust-codegen backend (`--native`, with `LOFT_NATIVE_LEAK_CHECK=1` so the
generated binary runs the same exit-time store-leak report the
interpreter prints unconditionally).  The folder a file lives in is its
**expected** leak behaviour:

| Folder | interp | native | meaning |
|---|---|---|---|
| `clean/` | no leak | no leak | the contract — a regression on either backend fails the suite |
| `known_<id>_both/` | LEAK | LEAK | open bug; the file asserts the leak still reproduces.  When fixed, move the file to `clean/` |
| `known_<id>_native/` | no leak | LEAK | open bug, native-only; move to `clean/` when fixed |

There are currently **no open-bug folders** — @P297 (both backends) and
@P298 (native-only) were fixed 2026-05-21 and their cases moved to
`clean/` (`p297_nested_call_arg_temp.loft`, `direct_return_no_leak.loft`).
Recreate a `known_<id>_*` folder (and add the matching arm in
`folder_expect` in `tests/leak_cases.rs`) when a new leak is found that
can't be fixed immediately.

**Why this exists:** before @P297, the `tests/leak.rs` suite ran only on
the interpreter — native had no exit-time leak check at all, so
native-only leaks (like @P298) went undetected.  This directory is the
permanent both-backend layer: drop a `.loft` file in `clean/` and it is
guarded on native too, for free.

Run the native half (heavy — `rustc` per file, so `#[ignore]`d):

```bash
cargo test --release --test leak_cases -- --ignored
```

The interpreter half runs in the default `cargo test` path.

[@P297]: ../claude/PROBLEMS.md
[@P298]: ../claude/PROBLEMS.md
