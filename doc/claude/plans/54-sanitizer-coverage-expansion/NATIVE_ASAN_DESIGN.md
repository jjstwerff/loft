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

**BLOCKED (well-localized):** the ASan cdylib build fails
`E0463: can't find crate for curve25519_dalek_derive which loft depends on`.
Root: libloft depends on a **proc-macro** (`curve25519_dalek_derive`); proc-macros
are HOST artifacts, so under a cross-target ASan build they live in
`target/release/deps` (host) while the cdylib's `-L dependency` points at the ASan
target deps (`target/x86_64-unknown-linux-gnu/release/deps`), which lack them.
This is the same curve25519-proc-macro class the interpreter `asan` job already
sidesteps (it drops doctests for the identical E0463).

**The fix (routed, not done):** add the host `deps/` dir to the ASan cdylib
build's `-L dependency` search path (or resolve the proc-macro rlib explicitly),
so the cross-target build finds host proc-macros. Then the S9 CI job = build an
ASan loft binary (with the `default/` stdlib symlink the non-standard target dir
needs) + run the `datalib` mixed corpus under `LOFT_NATIVE_ASAN=1`, asserting a
cross-boundary UAF/OOB is caught (positive control: inject an OOB store read into
the cdylib source). Until the proc-macro `-L` fix lands, the S9 gate would go red
on E0463, so it is NOT wired into CI yet.

## Done criteria

- **S6:** ✅ `native-asan` job green; curated `--native` corpus ASan-clean;
  positive control fires.
- **S9:** cdylib injection landed; end-to-end blocked on the proc-macro `-L` fix
  above; CI job deferred until that lands (documented, not silent).

## See also

- [README.md](README.md) § Concrete steps S6 / S9.
- `src/main.rs` (S6 injection) · `src/native_lib.rs::build_shared_cdylib` (S9).
- [THREADING.md] / [DATABASE.md] — the store model the generated code accesses.
- The interpreter `asan` job in `.github/workflows/miri.yml` — the in-process
  sibling that the S6/S9 out-of-process jobs complement.
