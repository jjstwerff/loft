<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 01b — Loft-callable binding for `Store::open_durable`

**Status:** ✅ **SHIPPED** (verified 2026-07-07). The loft-callable binding landed on `main`:
the Rust `Store::durable_check` / `Store::durable_seal` + the round-trip test
`tests/store_durable_loft.rs` via [PR #220](https://github.com/loft-lang/loft/pull/220)
(commit `b307ef03`, "store-durable (PLAN38)"), with the stdlib binding
(`store_durable_check` / `store_durable_seal` in `default/02_files.loft`) refined through
[PR #225](https://github.com/loft-lang/loft/pull/225). Design as committed below — the
**explicit check + seal** pair, NOT an embedded callback (which hit the `&mut State`
borrow-conflict wall). Test coverage: `store_durable_loft.rs` 2/2 green (incl.
`corrupted_sidecar_detected_via_loft_binding`, the seal→check→corrupt-sidecar→detect
round-trip); callable on both backends. Builds on the merged Phase 00 + 01
([PR #219](https://github.com/loft-lang/loft/pull/219), commit `d494edc`). The
`store-durable-phase1b` branch is gone (merged + deleted).

## Goal

Expose the Phase 01 Rust API (`Store::open_durable` +
`DurabilityMode::IntegrityOnly`) to loft programs so the
training-port migration's `store.loft` can call it directly
and drop its file-snapshot persistence workaround.

This is the slice that **unlocks the training port's
capability gap #2** in their `MIGRATION.md` — the gap that
explicitly names the durable native store as the missing
piece.  Without this binding, the Rust API exists but no
loft program can reach it.

## Background — what's already shipped

[`d494edc` (PR #219)](https://github.com/loft-lang/loft/pull/219)
landed Phase 00 + 01 on `main`.  Concretely available today
in Rust:

- `loft::store::Store::open_durable(&Path, DurabilityMode) -> io::Result<Store>`
- `DurabilityMode::IntegrityOnly { on_corruption: Box<dyn Fn(&Path) -> io::Result<()>> }`
- Sidecar `.dmeta` (40-byte: signature + tier + CRC + length + last-clean-ns)
- Drop impl flushes mmap + atomically rewrites sidecar on clean close
- `auto-heal` semantics: any post-callback Ok path → sidecar rewritten from
  current main-file state
- Trace gating via `LOFT_STORE_DURABLE_TRACE=1` (file-based, env-gated)

Test coverage: 10/10 `store_durable_format`, 7/7
`store_durable_tier1`, no regressions in 397 lib tests.

## Design — explicit check + seal, not embedded callback

### The closure-callback wall

The original phase-01 Rust API takes a
`Box<dyn Fn(&Path) -> io::Result<()>>` rebuild callback.  For
a *Rust* consumer that closure can freely close over arbitrary
state (filesystem handles, channels, allocators).  For a
*loft* consumer the closure body is itself a loft function,
which must execute on the **same** interpreter `State` that's
currently in the middle of executing the `open_durable` call.
A naive `Box<dyn Fn>` captured around `&mut State` is a borrow
conflict — the State is borrowed exclusively by the in-flight
native fn while the callback wants to run on it.

Loft already calls back into the interpreter from native code
in two places: `parallel_fold` (worker threads, own State
clones — not applicable) and the lambda-invocation
codegen-runtime helpers (synchronous, but specific to lambda
internals).  Building a third pattern just for the durable
callback isn't worth the surface area for what the consumer
actually needs.

### Shipped API surface (loft-side)

Drop the embedded callback.  Surface two native functions
that the loft consumer composes imperatively:

```loft
// Returns true iff the .dmeta sidecar at `<path>.dmeta`
// validates against the main file at `<path>` (signature,
// header CRC, payload length, payload CRC, tier_id all OK).
//
// Returns false if any check fails OR the sidecar/main file
// is missing.  This is the loft equivalent of a
// `StoreIntegrity::Clean` verdict from the Rust API; the
// `CorruptReason` detail isn't surfaced because the loft
// caller can't act on the variants distinctly (every
// non-Clean case routes to the same "rebuild from source"
// response).
fn store_durable_check(path: text) -> bool;

// Write a fresh sidecar at `<path>.dmeta` capturing the
// current main-file's byte length + CRC32 + a clean-close
// timestamp.  Returns true on success, false on I/O error
// (out-of-space, permission, parent dir missing).
//
// The loft caller invokes this after a successful write
// session — equivalent to the Rust API's Drop impl that
// rewrites the sidecar on clean close, but explicit instead
// of implicit.  Explicit is the right shape for loft
// because there's no Store-typed value the caller holds
// onto; the durable layer is "metadata for a path" rather
// than "wrapper around a Store handle."
fn store_durable_seal(path: text) -> bool;
```

### Loft-side usage pattern

```loft
// startup
if !store_durable_check("data.bin") {
    rebuild_data_bin("data.bin");  // consumer-defined
}
// ... use the database that lives in data.bin ...

// graceful shutdown
flush_database();          // consumer's existing flush
store_durable_seal("data.bin");
```

If the program crashes between `flush_database` and
`store_durable_seal`, the sidecar's `last_clean_ns` stays
stale relative to the on-disk file → next start's
`store_durable_check` returns false → rebuild fires.  Same
recovery semantics as the Rust callback API; the only
difference is the loft caller has explicit control over
where the seal point lands.

### Rust API surface — additions

The two loft natives are thin wrappers around two new
`Store` associated functions that don't take a callback:

```rust
impl Store {
    /// Loft-binding entry: the verdict-bool form of
    /// `validate_integrity`.  Returns `Ok(true)` iff
    /// `validate_integrity` returns `StoreIntegrity::Clean`.
    /// Any I/O error during validation collapses to
    /// `Ok(false)` (the caller treats it the same as
    /// "corrupt" — proceed to rebuild).
    pub fn durable_check(path: &Path) -> bool { ... }

    /// Loft-binding entry: write a fresh `.dmeta` sidecar
    /// for the file at `path`, capturing its current
    /// length + CRC + clean-close timestamp.  Returns
    /// `false` on any I/O error.
    pub fn durable_seal(path: &Path) -> bool { ... }
}
```

Both are implemented in terms of the already-shipped
helpers (`validate_integrity`, `compute_payload_crc`,
`encode_sidecar`, `write_sidecar_atomic`) — no new
algorithmic code.

### What does NOT ship in phase 01b

- No `open_durable`-style "open the file as a managed
  resource" handle for loft.  Loft programs already manage
  their database files through the existing `database_*`
  APIs; durability is a metadata layer on top, not a new
  open API.
- No closure-callback binding.  If a future consumer needs
  the embedded-callback shape (e.g. a Rust-side test that
  exercises both the Rust API and the loft binding from
  one process), the Rust `Store::open_durable` API is still
  available unchanged.
- No `database_*` integration.  Phase 01b ships the
  primitive; integrating with the existing
  `database_named` / `database_open` flow can come later
  if it surfaces as a real need.

## Critical files

| Path | Action |
|---|---|
| `src/store.rs` | EXTEND: add `Store::durable_check` + `Store::durable_seal` (free-function wrappers over Phase 00/01 helpers) |
| `default/01_code.loft` | ADD `store_durable_check` + `store_durable_seal` native fn declarations with `#rust"..."` bodies that call into `crate::store::Store::durable_check` / `durable_seal` |
| `src/native.rs` | ADD `n_store_durable_check` + `n_store_durable_seal` native handlers (string-arg unmarshalling + bool-return) |
| `tests/store_durable_loft.rs` | NEW: end-to-end loft-driven test that drives the full check → rebuild → seal lifecycle |
| `tests/scripts/store_durable_smoke.loft` | NEW: tiny loft script that exercises the binding (run via the existing `code!` / `cross_mode!` macros) |
| `doc/claude/STDLIB.md` | ADD § "Durable stores" entry documenting the two functions |
| `doc/claude/plans/43-loft-store-durable/README.md` | UPDATE phases table: insert 01b between 01 and 02 |

## Existing functions / utilities to reuse

- `Store::validate_integrity` (phase 00) — `durable_check`
  is a one-line wrapper.
- `compute_payload_crc` + `encode_sidecar` +
  `write_sidecar_atomic` (phase 01) — `durable_seal` is
  the existing `write_initial_sidecar` body lifted into a
  pub function.
- Existing native-fn handler patterns in `src/native.rs`
  for text-arg unmarshalling (`stores.get::<i64>(stack)`
  for DbRef → text decoding).

## Test surface

`tests/store_durable_loft.rs`:

- Rust-driven test that programmatically invokes the
  native binding through the existing
  `code!`/`cross_mode!` test harness, exercising:
  - Fresh path → `store_durable_check` returns `false`
  - Caller-supplied rebuild → main file exists → `store_durable_seal` returns `true`
  - Re-`store_durable_check` returns `true`
  - Corrupt sidecar (XOR a CRC byte) → `store_durable_check` returns `false`
  - Re-seal → `store_durable_check` returns `true` again

`tests/scripts/store_durable_smoke.loft`:

- Standalone loft script that drives the same lifecycle
  with `print` statements; verifies the binding is
  reachable from a real loft program (not just from the
  Rust test harness).

## Acceptance

- `cargo test --test store_durable_loft` passes.
- The loft smoke script (`tests/scripts/store_durable_smoke.loft`)
  runs clean under `target/release/loft --run`.
- `cargo test --release --lib` stays at 397/397, all
  pre-existing `store_durable_format` / `store_durable_tier1`
  tests stay green (no regression in phase 00 + 01 paths).
- `cargo clippy -- -D warnings` clean on Rust 1.95.
- `cargo fmt -- --check` clean.
- `doc/claude/STDLIB.md` documents the two new functions
  with usage example.
- Training port (in `personal/training`,
  `loft-migration` branch) can `use store_durable;` (or
  equivalent stdlib entry) and replace its file-snapshot
  primitive.  Verification is on their side, not blocking
  this PR.

## Risks

| Risk | Mitigation |
|---|---|
| Text-arg unmarshalling in `n_store_durable_*` differs from existing pattern | Copy from a known-working native fn that takes text args (search `src/native.rs` for `stores.get::<i64>(stack)` + path-shaped use) |
| Loft caller forgets to call `store_durable_seal` after a clean write | This is intentional behaviour — sidecar stays stale → next start rebuilds.  Documented as the trade-off in `STDLIB.md` |
| Smoke script's filesystem touches the test runner's working directory | Use `tempdir()` equivalent in loft (probably `dir_create` + `dir_remove` pair, or env-var-controlled scratch path).  Investigate during impl. |
| Future Rust-side change to `validate_integrity` semantics breaks the loft binding | Both code paths are in the same crate, same module; CI catches at compile time |

## Cross-references

- [PR #219](https://github.com/loft-lang/loft/pull/219)
  (commit `d494edc`) — shipped Phase 00 + 01, the foundation
  this phase wraps.
- [Phase 01 — Tier 1 IntegrityOnly](01-tier-1-integrity.md)
  — the underlying Rust API.
- `personal/training` repo (`loft-migration` branch),
  `MIGRATION.md` § "Loft capability gaps" #2 — the consumer
  that drove this phase.
- [Phase 02 — Tier 2 snapshots](02-tier-2-snapshots.md) —
  next tier up; reuses the same loft-binding pattern
  (`store_durable_snapshot(path)` etc. when phase 02 ships).
