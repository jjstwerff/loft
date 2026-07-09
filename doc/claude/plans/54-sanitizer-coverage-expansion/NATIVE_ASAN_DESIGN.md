<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# S6 + S9 — ASan over a generated build (design + status)

> **Part of [@PLN54](README.md).** S6 (native-backend ASan) and S9
> (mixed-boundary cdylib ASan) share ONE mechanism: thread `-Zsanitizer=address`
> into the rustc that loft spawns for a `--native` build, gated on a single
> opt-in `LOFT_NATIVE_ASAN=1`. **Status: S6 DONE + validated; S9 mechanism landed,
> end-to-end BLOCKED on a well-localized proc-macro issue (below).**

## The shared mechanism

`LOFT_NATIVE_ASAN=1` makes loft add `-Zsanitizer=address` (and select nightly
rustc, `RUSTUP_TOOLCHAIN=nightly` when unset) at the two rustc sites that compile
generated native code:

- **S6** — the standalone native binary: `src/main.rs` (`loft_native_bin_<pid>`).
  Per-PID output, never cached, so no artifact-cache-key interaction.
- **S9** — the auto-built shared-store cdylib: `native_lib::build_shared_cdylib`
  (the `@argfile` rustc invocation).

Both are opt-in and off by default (zero cost to normal builds).

## S6 — native-backend ASan — ✅ DONE + VALIDATED (2026-07-09)

**The invariant it checks:** loft's `--native` codegen emits Rust that accesses
the store by raw pointer; a codegen bug that produces an out-of-bounds or
use-after-free store access runs SILENT today because `--native` is a separate,
uninstrumented process (the in-process interpreter `asan` job cannot see it).
ASan over the generated binary makes that class loud.

**Key finding (the design-protocol probe):** ASan does NOT need an instrumented
libloft — unlike TSan's `-Zbuild-std` requirement, **ASan tolerates linking the
generated (instrumented) crate against the precompiled, uninstrumented
`libloft.rlib`** (no "mixing -Zsanitizer" ABI error). The generated crate's own
raw-pointer accesses are still instrumented, which is exactly the codegen surface
S6 targets. So S6 is a ~10-line flag injection, not a runtime rebuild.

**Validated:**
- ASan runtime confirmed active on the generated binary (the AddressSanitizer
  banner prints under `ASAN_OPTIONS=verbosity=1`).
- **Green-drive: 14/14 curated store-heavy scripts** (`--native` +
  `LOFT_NATIVE_ASAN=1`) ASan-clean — vectors / structs / enums / collections /
  map-filter-reduce / text / match, **including the `131-keyed-nested-struct-uaf`
  and `132-vector-elemset-inline-literal-uaf` regression scripts** (the exact UAF
  class ASan catches).
- **Positive control:** a standalone raw-pointer read 100 past a heap buffer,
  compiled with the same `-Zsanitizer=address`, is caught (`heap-buffer-overflow`)
  — so a clean run is non-vacuous.

**CI:** the nightly `native-asan` job in `miri.yml` runs the curated `--native`
corpus under `LOFT_NATIVE_ASAN=1` on nightly rustc.

## S9 — mixed-boundary (C71) cdylib ASan — MECHANISM LANDED, end-to-end BLOCKED

**The surface:** an interpreted script `use`s a native library; loft auto-builds
the library's cdylib and shares its `*mut Stores` with it BY RAW POINTER
(zero-marshalling). ASan is the only tool that can see a cross-boundary UAF/OOB
on that shared store — but only if BOTH sides are instrumented: the host
(interpreter) AND the dlopen'd cdylib. The mixed path runs in a **spawned loft
process** (`tests/n3_parity.rs` shells out to the loft binary), so — unlike the
interpreter `asan` job — covering it needs an **ASan-instrumented loft binary**
as the host, plus the cdylib injection above.

**Landed:** `build_shared_cdylib` adds `-Zsanitizer=address` under
`LOFT_NATIVE_ASAN=1` (parallel to S6). Architecture validated as far as: an ASan
loft binary (built `RUSTFLAGS=-Zsanitizer=address cargo build --target
x86_64-unknown-linux-gnu`, 870 `__asan` symbols) loads the stdlib and drives the
mixed `datalib` path up to the cdylib build.

**BLOCKED — first blocker E0463; the "simple fix" was PROBED and FALSIFIED.**
The ASan cdylib build fails `E0463: can't find crate for curve25519_dalek_derive
which loft depends on`. Immediate root: the ASan loft binary is a **cross-target**
build (`--target x86_64-unknown-linux-gnu`, needed so host proc-macros are NOT
sanitized), which splits libloft's deps — normal rlibs to
`target/x86_64-.../release/deps` (where the cdylib's `-L` points), but **proc-macros
are HOST artifacts** in `target/release/deps`. libloft transitively needs **~6
proc-macros** (`curve25519_dalek_derive`, `displaydoc`, `thiserror_impl`,
`yoke_derive`, `zerofrom_derive`, `zerovec_derive`).

Three probes (design-protocol — the clean fix breaks at the case the prose skipped):
1. **Add the host `deps/` to `-L`** (the fix this doc originally named) — **FALSIFIED.**
   Clears E0463 but drags the whole host graph in: `#[alloc_error_handler] in std
   conflicts` (two stds) + `mixing -Zsanitizer will cause an ABI mismatch` (×many).
   The host `deps/` carries a non-ASan std + non-ASan rlibs. WRONG.
2. **Explicit `--extern <proc-macro>=<host .so>` (no host `-L`)** — avoids the
   double-std, but needs the VERSION-matched `.so` (there are 4 `curve25519_dalek_derive`
   hashes) for EACH of the ~6 proc-macros — a version-resolution whack-a-mole like
   `loft_ffi_for_libloft`. Deterministic but fiddly + fragile.
3. **STABLE host (complete-deps libloft) + `LOFT_NATIVE_ASAN`** — **COMPILES CLEAN,
   ran correct output.** The stable `target/release` build has ALL deps incl.
   proc-macros, so an ASan cdylib links it with no E0463. BUT the stable host is not
   ASan, so the shared store it allocates is not ASan-tracked → no cross-boundary
   coverage. It isolates the problem to *the cross-target deps split*, not the
   cdylib-ASan itself.

**The concrete design the probes point to (candidate, NOT yet validated):** under
`LOFT_NATIVE_ASAN`, make `build_shared_cdylib` link the **complete-deps (stable
`target/release`) libloft** rather than the running ASan binary's cross-target
libloft, while still compiling the cdylib with `-Zsanitizer=address` and loading it
into the **ASan host** (whose malloc is intercepted, so the shared store IS
ASan-tracked). This is sound only if two facts hold — both need validation:
(a) ASan tolerates the cdylib linking a non-ASan libloft (S6 shows it does for the
binary; re-confirm for a cdylib); (b) a cdylib carrying a *stable* libloft copy,
dlopen'd into an ASan host carrying an *ASan* libloft copy, shares the `*mut Stores`
correctly — the `Store` struct layout is ASan-invariant (redzones wrap allocations,
not struct fields), so this should hold, but it is unproven. The alternative is
probe-2's per-proc-macro version-matched `--extern`.

**Status: NOT a validated concrete design — a probe-grounded candidate.** The S9
CI job (ASan loft binary + `default/` symlink + `datalib` mixed corpus + an
injected-OOB positive control) is deferred until one of the two approaches is
built and validated end-to-end. Estimated effort: the harder, unvalidated part of
S6+S9 — a focused session, not a one-liner.

## Done criteria

- **S6:** ✅ `native-asan` job green; curated `--native` corpus ASan-clean;
  positive control fires.
- **S9:** cdylib `-Zsanitizer=address` injection landed. End-to-end NOT designed
  to a validated point — the "host `deps/` on `-L`" fix was probed and falsified;
  the probe-grounded candidate (link the complete-deps stable libloft, ASan the
  cdylib, load into the ASan host) is unvalidated. CI job deferred.

## See also

- [README.md](README.md) § Concrete steps S6 / S9.
- `src/main.rs` (S6 injection) · `src/native_lib.rs::build_shared_cdylib` (S9).
- [THREADING.md] / [DATABASE.md] — the store model the generated code accesses.
- The interpreter `asan` job in `.github/workflows/miri.yml` — the in-process
  sibling that the S6/S9 out-of-process jobs complement.
