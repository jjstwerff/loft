<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->
# Issue labels — what they mean

For anyone (human or agent) triaging or fixing a loft issue **without** reading
the loft source.  Each label's GitHub *description* is the one-line gloss; this
file is the detail.  Convention + workflow: [`doc/claude/ISSUE_TRACKING.md`](../doc/claude/ISSUE_TRACKING.md).

Apply **one `sev:`, one `wa:`, and one or more `area:`** to every bug.

## `sev:` — severity (how bad WHEN you hit it)

| Label | Meaning |
|---|---|
| `sev:high` | crash / memory corruption / data loss / a soundness hole (the program does something silently wrong) |
| `sev:medium` | wrong result, hang, or miscompile in a real-world shape — but no corruption |
| `sev:low` | cosmetic, an edge case few will hit, a false-positive warning, a confusing-but-recoverable diagnostic |

## `wa:` — workaround (can you KEEP MOVING, or are you blocked)

The agent's primary "route-around or blocked?" signal.  **Always VERIFIED** — the
workaround was actually run on current HEAD, both backends; see
[ISSUE_TRACKING.md § Workarounds](../doc/claude/ISSUE_TRACKING.md#workarounds--the-agents-can-you-keep-moving-signal).
A wrong workaround is worse than `wa:none`.

| Label | Meaning |
|---|---|
| `wa:clean` | a simple, idiomatic alternative exists (verified) |
| `wa:partial` | a workaround exists but is awkward / loses the intended behaviour (verified) |
| `wa:none` | nothing works — this **blocks** whoever hits it (the most urgent triage axis, often above `sev:`) |

## `area:` — which part of loft (plain-English, with orienting files — NOT required reading)

loft is a tree-walking interpreter **and** a native code generator for a
statically-typed language.  Source flows: **text → parser → IR → codegen →
{ bytecode interpreter | native Rust }**, over a **store-based heap**.

| Label | What it is | Bugs here look like | Orienting files |
|---|---|---|---|
| `area:parser` | turning source text into typed IR: lexer, two-pass parser, type resolution, scope/lifetime analysis | parse errors, a construct mis-resolved, wrong inferred type/scope, a bad diagnostic | `src/parser/`, `src/lexer.rs`, `src/typedef.rs`, `src/scopes.rs` |
| `area:codegen` | turning correct IR into runnable code — the **bytecode** generator (interpreter) and the **native** Rust generator | the program parsed + type-checked fine, but the emitted code is wrong: wrong value, panic, slot/stack drift, a miscompile on one backend | `src/state/codegen.rs`, `src/state/fill.rs`, `src/generation/` |
| `area:store-lifetime` | the heap: store alloc/free, dependency tracking, value-semantic vector/struct lifetime, the watermark | leaks, use-after-free, double-free, a store freed too early/late, high memory watermark | `src/database/`, `src/store.rs`, `src/scopes.rs` (the free analysis) |
| `area:closures` | fn-refs, lambdas, captured cells, the closure-record layout | capture/ownership bugs, closures stored in containers, fn-ref dispatch/iteration | `src/parser/vectors.rs` (capture), closure-record handling in `src/database/` |
| `area:runtime` | executing bytecode — the 233 opcodes + the runtime stack/store ops (interpreter only) | a specific opcode misbehaves, a runtime panic not in codegen | `src/state/`, `src/fill.rs` |
| `area:native` | the `--native` backend specifically: Rust generation, the native runtime shim, `rustc`/`cc` linkage | a native-only divergence from the interpreter, an ABI/marshalling bug, a link/rlib failure | `src/generation/`, `src/codegen_runtime.rs` |
| `area:wasm` | the browser / WASM target: `wasm32`, the virtual FS, host bridges, threading | WASM-only failures, a missing host bridge, a feature gated off in the browser | `src/wasm.rs`, `wasm/` |
| `area:stdlib` | the standard library (`default/*.loft`) + native stdlib functions | a stdlib function returns the wrong thing / is missing an edge | `default/*.loft`, the native-fn registry |
| `area:packages` | the package format, registry, multi-file `use` resolution, library extraction | a cross-package resolution bug, a manifest/registry issue, a build-pipeline gap | `src/package.rs`, `src/manifest.rs`, `doc/claude/lib_plans/` |

## Cross-cutting

| Label | Meaning |
|---|---|
| `both-backends` | reproduces on BOTH `--interpret` and `--native` (vs a single-backend divergence) |
| `needs-design` | the fix needs a design decision, not a mechanical change — don't just patch it |
| `bug` / `enhancement` / `documentation` / … | the GitHub defaults; keep `bug` on every bug |

> **Multi-repo:** `sev:`/`wa:`/cross-cutting are shared across the loft-family
> repos (loft / dryopea / lavition / `loft-lang/*`).  `area:` is loft-specific;
> each game/engine repo defines its own `area:` set in its own `LABELS.md`.
