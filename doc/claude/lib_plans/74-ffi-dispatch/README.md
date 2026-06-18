<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 25 — FFI generated-dispatch

Replace the hand-written interpreter marshal (the ~98-arm
`dispatch_call` in `src/extensions.rs`) with a per-function bridge that
each native library **generates from its own Rust signatures**, so new
signatures/widths never touch loft-core and libraries own their FFI
typing.

Promotes FFI.1 / FFI.3 out of the kitchen-sink
[`../05-game-infra/`](../61-game-infra/README.md#ffi1--generic-type-marshaller)
(those sub-arcs are **superseded by this plan** — see § Reconciling the
earlier note).

## Status

**F1 + F2 + F3 SHIPPED; F4 ROLLOUT SHIPPED 2026-05-27.**  All four monorepo
native libs — **imaging, web, server, graphics** — now dispatch through
generated `#[loft_native]` bridges (`extensions.rs` `BRIDGE_REGISTRY` +
`dispatch_via_bridge`), each runtime-verified via the env-gated probe.
graphics was selective (hand-written register + dual-ABI: only the registered
interpret fns annotated; the raw-ptr `loft_gl_*` direct-native + `vec_wrapper!`
fns keep the legacy arms).  **The arm DELETION is deferred** (the remaining F4
step): `tests/lib/*/native` fixtures and the already-published external libs
(`loft-libs-core`/`-net`) still register raw pointers with no bridges, so the
legacy `dispatch_call` is retained as their fallback until they migrate
(Phase 6r / [`../../12-library-extraction/`](../12-library-extraction/README.md)).
**F5 SHIPPED** — PACKAGES.md `## Function binding model` rewritten to the
`#[loft_native]` + source-scan + bridge pattern with a complete 3-fn example.
The plan's only remaining work is the trigger-gated arm deletion (Phase 6r).

F3 = one-library proof: `lib/imaging`'s `n_load_png`/`n_save_png`;
`loft-ffi-build` emits the `loft_register_bridges!` list; additive —
non-bridge symbols + `--native` untouched.

**F1 + F2 detail:**  F1 = `loft-ffi` transport (`LoftValue` +
`LoftBridgeFn` + `loft_register_bridges!`).  F2 = new `loft-ffi-macros` crate
with the `#[loft_native]` proc-macro generating `<fn>__loft_bridge` from each
fn's real Rust signature (scalars with impl-width casts, `i32::MIN` sentinel
widen, bool/float, text→`(*const u8, usize)`, `LoftRef`, `LoftStore`-first,
`LoftStr`/`LoftRef`/void returns).  Both additive — no interpreter wiring yet
(legacy raw-ptr arms still run).  F3–F5 open.  Inspected 2026-05-27; the two
load-bearing facts below are confirmed against the tree.

## Migration state (2026-06-18) — full inventory + the publish blocker

Re-audited for a "migrate every loft-lang library to bridges, delete the legacy
arms" pass.  Two findings reframe the "F4 rollout shipped" status above:

**1. The bridge infra was never published.**  `generate_register_from_loft_with_bridges`,
the `#[loft_native]` macro, and `LoftValue`/`LoftBridgeFn` live ONLY in the local loft
repo crates (`loft-ffi 0.1.1`, `loft-ffi-build 0.2.1`, `loft-ffi-macros 0.1.0`).
crates.io has only `loft-ffi-build 0.1.0`/`0.2.0` — **neither** has `_with_bridges`.
So **no external library can build with bridges**, and `imaging`'s "migrated" state in
`loft-libs-graphics` is non-buildable against published crates.  This is why F4 stalled
at the monorepo (path-dep) libs and the legacy arms remain the only working path.
⇒ **Prerequisite for the whole pass: publish the bridge-capable `loft-ffi` /
`loft-ffi-build` / `loft-ffi-macros` to crates.io.**

**2. Only 7 native libraries exist** (every other loft-lang lib is pure-loft → no FFI):

| Repo | Lib | FFI now | Action |
|---|---|---|---|
| loft-libs-core | crypto | manifest (`generate_register_invocation`) | → bridges (+ manifest→source-scan) |
| loft-libs-core | **regex** | legacy → **MIGRATED (proof below)** | publish infra, path→version |
| loft-libs-core | random | legacy source-scan | → bridges |
| loft-libs-net | web | manifest | → bridges |
| loft-libs-net | server | hand-written register (no build.rs) | → bridges |
| loft-libs-graphics | graphics | hand-written, dual-ABI raw_gl | → bridges (selective) |
| loft-libs-graphics | imaging | `_with_bridges` (unbuildable vs published) | publish infra → buildable |

Plus: monorepo `lib/*/native` are vestigial stubs (no `lib.rs`) → delete; 2 test
fixtures (`native_pkg`, `native_scalar_pkg`) → migrate.

**Recipe (per source-scan lib), proven on `regex`:** add `loft-ffi-macros` dep,
`#[loft_native]` on each `n_*`, `build.rs` → `generate_register_from_loft_with_bridges`.
`regex` migrated against the local infra (path deps); with the legacy arms REVERTED
from loft, all 8 regex tests pass on `--interpret` through the bridge — including the
new `match_groups` `(text,text,integer)` and `replace` `(text,text,text)` signatures
that have NO arm.  (Two arms wrongly hand-added to `extensions.rs` were reverted —
the bridge is the fix.)

**Order:** publish infra → migrate the 6 libs + 2 fixtures (re-publish each) → delete
`dispatch_call` + `ArgT`/`ArgVal` arms in loft → remove vestigial stubs → consolidate +
fix the ~15 stale FFI docs.

## Goal

`--interpret`'s call into a `#native` function dispatches through ONE
uniform bridge per function (generated by a `#[loft_native]` proc-macro
from the real Rust signature).  Adding a native function with any new
signature/width requires **zero** edits to loft-core.  Behaviour
identical on `--interpret` (and `--native` is untouched).

## Non-goals

- **`--native` is not touched.**  It already links each library's rlib
  and emits direct typed calls (`output_native_direct_call`,
  `src/generation/mod.rs:2232`) with `as _` width casts — the perf path,
  zero marshal.  This plan only reshapes the `--interpret` dlopen marshal.
- No `libffi` (C dep + per-call prep) and no perf regression: the bridge
  call is one indirect call into generated, direct-typed decode — no
  slower than the current transmute-arms.

## Background — what we replace (confirmed)

`--interpret` dlopens each library `.so`; `loft_register_v1`
(`loft-ffi`'s `loft_register!` macro) registers **symbol → raw
`*const ()`** into a global registry (`src/extensions.rs:27`).  On a call:

1. `native_auto_dispatch` (`extensions.rs:421`) reads the loft signature
   (`NATIVE_SIGS`, computed from the `#native` decl), pops each stack arg,
   and marshals it into a **uniform `Vec<ArgVal>`** (`ArgVal::I32/I64/F64/
   F32/Bool/Text(ptr,len)/Ref/…`).
2. `dispatch_call` (`extensions.rs:609`) matches on the exact
   `(&[ArgT], ret)` shape — **~98 arms** — `transmute`s the raw pointer to
   that concrete `extern "C" fn`, and calls it.

The arm-explosion is the whole problem: every signature/width combination
a library uses needs an arm in loft-core (@PLAN48 P1b hand-added ~26 of
them).  **The uniform `ArgVal` array already exists** — `dispatch_call`'s
only job is the final transmute-to-typed-call.

## Two facts that settle the design

1. **Library native crates are pure Rust + `loft-ffi` only** — NO
   `loft-core` dependency (`lib/*/native/Cargo.toml` depend on `loft-ffi`
   for the `#[repr(C)]` handles `LoftRef`/`LoftStore`/`LoftStr`, plus their
   own deps).  Their `n_*` fns are already typed `#[no_mangle] extern "C"`.
2. **Only the library knows its own concrete signatures**, and they are
   dlopen'd (loft-core cannot enumerate them at its own build time).  So
   the per-signature typed call **must** live library-side.

⇒ The bridge lives in the library crate.  loft-core calls it through one
uniform signature and never reconstructs a concrete C signature again.

## Design

### 1. Uniform transport — `loft-ffi::LoftValue`

A `#[repr(C)]` tagged value (the `repr(C)` form of today's internal
`ArgVal`): `{ tag: u8, payload: union { i: i64, f: f64, text: (ptr,len),
r: LoftRef } }`.  The interpreter already builds this array; expose it
across the ABI.

### 2. Uniform bridge calling convention

Each native fn `n_foo` gets a generated sibling:

```rust
unsafe extern "C" fn n_foo__loft_bridge(
    store: LoftStore,              // for allocating return text / refs
    args: *const LoftValue, n: usize,
    ret:  *mut LoftValue,
);
```

loft-core calls **only** this shape — no per-signature knowledge.

### 3. `#[loft_native]` proc-macro (new crate `loft-ffi-macros`)

Applied to the real impl, it reads the **real Rust signature** and emits
the bridge: decode `args[i]` to each concrete param (`as i32` / `as i64`
per the impl's actual width — this is why it must read the *Rust* sig, not
the loft decl, which is width-ambiguous per @P370), call `n_foo(...)`,
encode the return into `*ret` (text/ref returns allocate via `store`).

```rust
#[loft_native]
#[no_mangle] pub extern "C" fn n_foo(a: i32, b: LoftStr) -> i64 { … }
// generates n_foo__loft_bridge that decodes args→(i32, LoftStr), calls
// n_foo, writes the i64 into *ret.
```

`loft_register!` (driven by `loft-ffi-build`'s existing `#native`
source-scan) registers the **bridge** under the loft symbol `"n_foo"`.

### 4. Interpreter simplification

`native_auto_dispatch` marshals the stack into `[LoftValue]` (it already
does, as `ArgVal`) and calls the single bridge signature.  **`dispatch_call`
and the `ArgT`/`ArgVal` arms are deleted.**  `NATIVE_SIGS` keeps only what
the marshal needs to read each arg off the stack (param kinds), not the
return-type-specific arm selection.

## Phase ordering — small verifiable steps

| Phase | Scope | Verify |
|---|---|---|
| **F1** ✅ | `loft-ffi`: define `LoftValue` + the bridge calling convention + the `n_*__loft_bridge` registration hook. No behaviour change yet (old arms still used). | **DONE 2026-05-27** — `LoftValue`/`LoftPayload`/`LoftTag` + `LoftBridgeFn` + `loft_register_bridges!`; 4 unit tests; full suite green. |
| **F2** ✅ | New `loft-ffi-macros` crate: `#[loft_native]` proc-macro generating bridges for the primitive set (i64/i32/f64/f32/bool/text/ref/vec). | **DONE 2026-05-27** — 8 integration tests drive the generated bridges (scalar width from impl sig, sentinel widen, bool/float, text ptr+len, LoftStr/LoftRef returns, LoftStore-first, void); clippy+fmt clean. Tested standalone (aux-crate convention, same as `loft-ffi-build`); enters root CI at F3 when a library deps it. |
| **F3** ✅ | **One-library proof** — apply `#[loft_native]` to one small native lib (candidate: `lib/imaging` or a crypto fn), register its bridges, and route `--interpret` through the bridge for that lib (fallback to old arms for the rest). | **DONE 2026-05-27** — `lib/imaging` (load/save PNG) via `#[loft_native]`; `generate_register_from_loft_with_bridges` emits the bridge list; `BRIDGE_REGISTRY` + `dispatch_via_bridge` in `extensions.rs`; imaging PNG round-trip green on `--interpret` (bridge path confirmed) + `--native` unaffected. `loft-ffi-macros` now enters root CI (imaging deps it). |
| **F4** (rollout ✅) | Roll out to `web`/`server`/`imaging`/`graphics`; once all monorepo libs use bridges, **delete `dispatch_call` + `ArgT`/`ArgVal` arms**.  Bump `loft-ffi`/`loft-ffi-build`/`loft-ffi-macros` versions for external libs (`loft-libs-core`/`-net`/`-graphics`). | **ROLLOUT DONE 2026-05-27** — all 4 monorepo libs on bridges, probe-verified, full suite green.  **Deletion deferred**: `tests/lib/*/native` fixtures + published external libs still register raw ptrs (no bridges) → legacy arms retained as fallback until they migrate (Phase 6r). |
| **F5** ✅ | FFI.4 doc — zero-boilerplate native-fn guide in [PACKAGES.md](../../PACKAGES.md). | **DONE 2026-05-27** — rewrote the stale `## Function binding model` (was `#[loft_fn]`/`&Stores`/`loft.toml [native.functions]`) to the real `#[loft_native]` + source-scan + bridge pattern: type-mapping table, zero-boilerplate registration (`build.rs` + `Cargo.toml`), 3-execution-paths, and a complete 3-function example.  doc-drift + broken-link gates green. |

External libs (already published) keep working during F1–F4 via the
**legacy raw-ptr path retained as a fallback** until they re-publish
against the new `loft-ffi` (a Phase-6r-style re-clean, coordinated with
[`../../12-library-extraction/`](../12-library-extraction/README.md)).

## Open questions

- **Text / ref returns** — the bridge allocates the return text via the
  `LoftStore` callback (`ffi_claim`, `extensions.rs:514`), the same path
  hand-written `push_loft_str` uses today.  Confirm the proc-macro can emit
  this for any return type.
- **Vector args** — today's marshal deref's the vector handle
  (`extensions.rs:472`).  The bridge needs the same; decide whether
  `LoftValue::Vec` carries the dereferenced record or the raw handle.
- **`LoftStore` threading** — when is `store` needed (any ref/text/vector
  touch)?  Pass always vs. only when the signature needs it.
- **Transition window** — dual-path (bridge + legacy arms) adds a registry
  flag per symbol; remove it in F4 once all libs are migrated.

## Reconciling the earlier note

[`../05-game-infra/` FFI.1 § Design decision](../61-game-infra/README.md)
"rejected a uniform LoftCell arg-array shim … for no benefit."  That
rejection assumed the uniform array was *added* indirection.  Inspection
shows the interpreter **already** builds the uniform `ArgVal` array; the
generated bridge merely moves the final typed call library-side.  So the
uniform transport is not new cost — it is the existing cost, minus the
98-arm match.  This plan supersedes FFI.1/FFI.3 there.

## See also

- `src/extensions.rs` — current marshal (`native_auto_dispatch`,
  `dispatch_call`, the `ArgT`/`ArgVal` arms).
- `loft-ffi/src/lib.rs` — `LoftRef`/`LoftStore`/`LoftStr`, `loft_register!`.
- `loft-ffi-build/src/lib.rs` — `#native` source-scan → register list.
- `src/generation/mod.rs:2232` — `output_native_direct_call` (the
  `--native` path this plan must NOT regress).
