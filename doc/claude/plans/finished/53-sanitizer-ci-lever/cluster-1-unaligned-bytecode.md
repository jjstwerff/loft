<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Cluster 1 — Unaligned `&mut T` into the bytecode buffer

**Detector:** Miri (`cargo +nightly miri test`).
**First surfaced:** 2026-05-29, PLAN53 Stage A1 spike, against a
trivial cluster-I-shaped program (any program triggers it).
**Status:** mechanism VERIFIED; **FIX LANDED 2026-05-29** (see
§ Fix landed) — full test suite green; Miri re-confirmation in
progress.  Fixed independently of the PLAN52 gate because the
fix surface (`src/state/mod.rs` bytecode accessors) is disjoint
from PLAN52's value-block-borrow family — PLAN52 does not touch
this code (user-confirmed).

## Severity

Two fields tracked separately (PLAN51 lesson — never conflate):

| Axis | Rating |
|---|---|
| Corruption / panic / hang | **Latent UB** — unaligned reference is UB per the Rust abstract machine.  Masked today on x86-64 (hardware tolerates unaligned access; rustc 1.95 emits a plain load/store).  A future rustc/LLVM that assumes alignment on the `&mut T` could miscompile.  Same masking-shift mechanism as @P383. |
| Leak | none |

## Backend asymmetry

**Universal — not backend-specific.**  The fault is in
`byte_code` (the IR→bytecode lowering), which runs for *every*
loft program on *both* the interpreter and the `--native` path
(native codegen still builds bytecode first).  Miri aborts here
before any execution-phase or codegen-phase UB can be observed
(see § Gating effect).

## Verified mechanism

| Statement | Status | Evidence |
|---|---|---|
| `code_add::<T>` casts a `*u8` into the byte-granular `Vec<u8>` bytecode buffer at `code_pos`, then writes through `*off.as_mut() = value` | ✅ VERIFIED | `src/state/mod.rs:1383-1387` (read 2026-05-29) |
| `code_put::<T>` (write-at-offset) has the identical `Arc::make_mut(..).as_mut_ptr().offset(on).cast::<T>()` + `*off.as_mut() = value` pattern | ✅ VERIFIED | `src/state/mod.rs:1358-1365` |
| When `code_pos` (or `on`) is odd and `T = u16`/`u32`, `<*mut T>::as_mut()` constructs a reference that violates `align_of::<T>()` → UB | ✅ VERIFIED | Miri report: `constructing invalid value of type &mut u16: encountered an unaligned reference (required 2 byte alignment but found 1)` at `core/src/ptr/mut_ptr.rs:586` (`as_mut`) |
| The triggering call chain is `byte_code → byte_code_from → def_code → add_return → code_add::<u16>` | ✅ VERIFIED | Miri stack backtrace, 2026-05-29 |
| `code_add_str` is NOT affected — it `copy_to`s bytes (no typed reference materialized) | ✅ VERIFIED | `src/state/mod.rs:1390-1401` |
| rustc 1.95 / x86-64 masks it (the spike program printed correct output natively) | ✅ VERIFIED | `/tmp/p53/clusterI.loft` ran clean on stable interpret, 2026-05-29 |

## Gating effect (important for Stage A2 sequencing)

Because this fires inside `byte_code`, **Miri sees nothing past
the compile step for any program.**  Every execution-phase UB —
including the entire PLAN52 cluster-I `_ncc_N` family this lever
was built to detect — is masked behind this one finding under
Miri.  Consequence:

- **Empirical Miri confirmation of cluster-I is blocked** until
  this cluster is resolved (or locally patched).  The conceptual
  case for cluster-I (textbook heap-use-after-free) is strong and
  ASan can confirm it independently (ASan does *not* detect
  alignment UB, so it runs straight past this cluster — see the
  tooling-decision doc).
- A Miri CI gate is **not viable** until this is fixed: it would
  red-flag on the first program compiled.

## Fix landed (2026-05-29)

Replaced the unaligned typed-reference writes/reads with the
unaligned pointer intrinsics, in `src/state/mod.rs`:

- `code_put::<T>` and `code_add::<T>`: `*off.as_mut() = value`
  → `off.write_unaligned(value)`.
- `code<T>()` (the read accessor): now returns `T` **by value**
  via `off.read_unaligned()` instead of returning `&T`.  An
  unaligned `&T` is UB the instant it's constructed, so the
  reference-returning API could not be made sound — the value
  return is the fix.  Bound widened to `T: Copy` (all
  instantiations are Copy primitives: u8/u16/u32/i8/i16/i64/
  bool/char/f32/f64).
- 241 call sites updated mechanically: `*x.code::<T>()`
  → `x.code::<T>()` across `src/{create,fill}.rs` and
  `src/state/{mod,io,debug,text}.rs`.  Semantically identical for
  Copy types.

`write_unaligned` / `read_unaligned` lower to the same load/store
on x86-64 (zero perf change) and are defined at any alignment —
the canonical idiom for typed access into a byte buffer.

**Verification:**

- `cargo build --lib` clean; `cargo test --test issues` 681/0;
  `find_problems` full suite — no regression attributable to this
  change (the unrelated `spacial_*` / `doc_examples_js` failures
  are pre-existing on the `macos-clippy-fixes` base).
- **Miri re-confirmation ✅ CONFIRMED 2026-05-29** —
  `production_mode_no_error_had_fatal_false` under
  `cargo +nightly miri test` now traverses the **entire** stdlib
  `byte_code` pass (where the pre-fix spike aborted at the first
  `code_add`) and proceeds into `execute` with **no `code_*`
  diagnostic**.  Cluster 1 is gone.

**Sibling surfaced (expected):** getting past `code_add` let Miri
reach a *different* unaligned site — `Store::addr_mut::<Str>`
(`src/store.rs:1366`) during `const_text` execution.  That is the
SAME UB class in the store heap accessor, filed as **cluster 2**
([`cluster-2-unaligned-store-access.md`](cluster-2-unaligned-store-access.md)) —
NOT a regression of this fix.

## Reproducer

Any loft program reproduces it under Miri.  Minimal:

```bash
# /tmp/p53/clusterI.loft (or literally any .loft program)
MIRIFLAGS=-Zmiri-disable-isolation \
  cargo +nightly miri test --test issues <any-interpreter-test>
# → error: Undefined Behavior: ... unaligned reference ...
#   at loft::state::State::code_add::<u16>  (src/state/mod.rs:1386)
```

Native (rustc 1.95) masks it: the same program runs clean.

## Probe pairing

- **Problem probe:** any program (the bytecode emitter is
  universal).  A minimal one — `probes/` TBD when the fix phase
  authors the regression.
- **Reference probe:** n/a — there is no "doesn't trigger"
  variant; the fix is what makes it not trigger.
