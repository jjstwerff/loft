<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 24 — `#c`: call a C library with no Rust in it

## Status

**Arcs A and C are done; B, D, E, F open.** The architecture probe is built and has run
(`tests/fixtures/c_abi/`, `make check && make probe`): the fixture is a C
library with one function per loft type, and the probe is loft-core's proposed
caller run against all of it. Two of the issue's premises came back changed and
one open question is now answered, so the design below is not the one on the
issue — read this, not that.

Arc A ships the declaration and its check (`src/c_signature.rs`): `#c "sym"
"<c-signature>"` parses, is checked against the loft declaration it annotates,
and survives the IR round trip. **Nothing calls a `#c` function yet** — `native`
is deliberately left empty so the Rust dispatch path cannot pick one up.
`#native` (Rust) is today's path and stays it.

**Arc C works end to end**: `--native` compiles a `#c` declaration into a typed
`extern "C"` and calls it. Proven against **libc** — linked into every Rust
binary, so it needs no build step and nothing installed — with `strlen`, `abs`,
`atoi` and `write`. The cell that matters is `atoi("-1")` answering **-1**: the
declared width goes into the extern, so rustc truncates at the ABI and the cast
sign-extends. A signature-blind caller reads the same return as 4294967295.

Three things the build corrected, which is why C was ordered before B:

1. **The pointee spelling is not needed for emission.** The design said the
   parser keeps it "for the `--native` extern emission, which has to write a
   real Rust type". It does not: every pointer is `*const c_void`, because
   pointers share one ABI whatever they point at. The spelling stays for
   diagnostics only.
2. **A vector's count must sit immediately after its pointer**, and real C APIs
   do not always oblige — `memchr(ptr, ch, n)` and `fwrite(ptr, size, n, f)`
   separate them. The adjacent pair covers the common case (`write(fd, ptr, n)`
   works); anything else needs a shim. That is consistent with the plan's
   complexity sink, but it is a real limit and it is now written down rather
   than discovered by a library author.
3. **"Inert" was not inert.** A `#c` call under `--interpret` compiled and
   returned a plausible wrong number — `strlen("hello")` answered **7562**. A
   program correct on one backend and silently wrong on the other is the
   divergence class the ship gate exists to catch, not a missing feature, so the
   interpreter now REFUSES the call with a message naming `--native`. Declaring
   a binding stays fine on every backend, which is what keeps arc A inert; arc B
   replaces the refusal with the real caller.

One thing arc A learned the hard way, worth carrying into B: the baked IR field
offsets in `data_store.rs` are a MIRROR of the registered schema, and the schema
packs by size rather than declaration order — so two new `text` fields moved
every trailing boolean and the stride. Hand-guessed offsets segfaulted the
round-trip test; `baked_layout_mirrors_loft_schema` is the instrument that names
the real ones, and it must be read *before* the constants are written, not
after.

## Goal

A loft library binds directly to a system C library — **no rustc anywhere in
the library**, no libffi — and reaches the same standard of proof as a Rust
`#native` library: the same four-target matrix, the same signature checking,
the same failure reporting, the same capability declarations.

- **Effort:** M for arcs A–D (the native + interpreter halves). Arc E (the other
  two targets) is separate and larger, and is where "full parity" is bought.
- **Design:** ✓ — invariant named, claims probed, one open question left (E).

## The invariant

> **The `#c` declaration is the sole authority on the C signature, and every
> caller derives its call from that one parsed signature.**

Not a preference — a measurement. The probe pointed loft's proposed caller at
deliberately wrong signatures and **nothing failed**: an arity-1 symbol called
through an arity-3 trampoline returned the right answer (extra register
arguments are ignored), and a variadic function called through a non-variadic
one returned the right answer too (by luck — SysV wants `AL` set, and a wrong
answer was available). There is **no runtime signal** distinguishing a correct
binding from a wrong one, so nothing can reconcile a second source of truth
against the first. One authority, checked at compile time, or none.

This is the load-bearing difference from `#native`, and it is forced rather
than chosen. There, the **Rust impl's signature** is authoritative: the
`#[loft_native]` macro reads the real fn and generates the marshal bridge, so
the declaration can be loose and the impl still lands correctly. A `#c` library
has no Rust and therefore nothing to read — the declaration is all there is.

### Re-assertion sites — the brittleness, counted before any code

The signature has to be restated by: (1) the interpreter's trampoline choice,
(2) the `--native` `extern "C"` emission, (3) the compile-time check, and (4)
each target arc E adds. Omission is **silent at every one** (the probe again).
`N × silence` is the whole risk of this plan, so the design collapses N to one:
a single parsed `CSignature`, produced once from the declaration, that all four
consult. Nothing re-derives it from the loft types, because the loft types
cannot express the C widths — which is the next section.

## What the probe settled

Full numbers in [`tests/fixtures/c_abi/README.md`](../../../../tests/fixtures/c_abi/README.md).

- **The trampolines work — for arguments.** Every integer-class argument (int,
  long, pointer, `char *`, bool) crosses correctly through a `u64` slot at
  arity 0/1/6/7/12, including across the SysV register/stack boundary, with no
  libffi and no generated code. `dlopen` + transmute is sound.
- **They do not work for returns, and this answers the issue's stated
  load-bearing question.** `int32_t` returning −1, read back as `u64`, is
  **4294967295**: SysV leaves the upper half unspecified and x86-64
  zero-extends. The failure is not a crash but a plausible large positive — the
  shape that silently defeated `loft-libs-net`'s `server::listen`. So *"arity
  alone parameterises the trampoline set"* is true of the **call** and false of
  the **answer**. The set stays ~13 functions; the declaration must carry the C
  **return width**. The issue leaned option (a) — spell the C type — and (a) is
  now the only option left.
- **The shims are real.** Every case the issue routes to an ANSI-C shim
  (double/float argument, struct by value, varargs, out-parameter) works through
  the integer trampolines once wrapped, and each wrapper is three lines. The
  trade the plan is built on — complexity in the shim, not in loft-core — holds.

## Parity with the Rust path, row by row

`#native` is the standard to meet. Each row is either *reuse it*, *forced to
differ*, or *simpler*.

| | `#native` (Rust) | `#c` | |
|---|---|---|---|
| Declaration | `#native` / `#native "sym"` | `#c "sym" "<c-signature>"` | differs — the signature string is load-bearing |
| Signature authority | the Rust impl (macro reads it) | **the declaration** | forced — no Rust to read |
| Checking | macro + compiler check | compile-time parse + arity/width check | forced — no runtime signal exists |
| Interpreter call | `dlopen` → generated uniform bridge (`LoftBridgeFn`), one ABI for every fn | `dlopen` → `dlsym` → fixed per-arity trampoline, keyed on (arity, return class) | differs — nobody can generate a per-fn adapter |
| `--native` call | typed `extern "C"` decl + direct call (`add_native_extern_flags`) | **the same emission**, from the declared C types | **reuse — parity is nearly free here** |
| `--native-wasm` | cross-compiled wasm rlib | the C compiled to wasm, where the capability exists | arc E |
| `--html` | hand-built `[wasm.bridge]` crate | same options, same cost | arc E |
| Registration | `build.rs` source-scans `.loft`, generates registers | **nothing to register** | simpler — there is no artifact to build |
| Manifest | `[native] crate` / `runtime-libs` / `build-deps` | `[c] lib` / optional `shim` | reuse the shape |
| Effects / capability | declared in the loft signature (`fs#read`) | identical, declared not inferred | reuse |
| Crash reporting | the signal handler names the frame | identical | reuse |

The row that matters most is `--native`: loft **already** links packages by C
ABI and emits `unsafe extern "C" { … }` declarations with typed signatures
(`native_utils::add_native_extern_flags`, the `native_cabi` branch in
`generation/mod.rs`). That is a `#c` binding in everything but where the types
come from. Arc C is pointing existing machinery at a different source, not new
machinery.

And the row that reframes "parity": **`#native` does not get the browser for
free either** — it needs a hand-built bridge crate. Parity is therefore *"a
defined answer in every column"*, not *"works everywhere automatically"*. That
is what makes arc E affordable to state honestly.

## The null and fault boundary — a decision, taken

loft's hard rule is that there are no runtime errors, ever: an undefined or
faulting computation yields **null** and the program continues (C80). C has no
such model — it traps, or it corrupts. **The decision is that `#c` is the
declared edge of loft's totality**, and the concession is deliberate rather
than an oversight to be litigated later.

What it costs, precisely — less than it sounds:

- **Arguments cost nothing new.** Non-null is already loft's default (`not null`
  is a deprecated no-op), and the null-flow analysis (@PLN25) already refuses a
  `τ?` where a non-null value is required, without a discharge (`?? 0`, `x?`,
  `match`). A `#c` parameter is an ordinary non-null parameter. **The illegal
  call does not compile**, so the concession is not observable at runtime.
- **A fault inside C is not loft's null.** Its own division by zero, its own
  null dereference: undefined, declared illegal, not modelled. This is not a new
  risk class — it is exactly the failure mode `#native` already has, reported
  through the same crash handler. `#c` widens that surface; it does not open it.
- **A NULL pointer return maps to loft null**, because that is already the
  sentinel for `text` and `reference` and the mapping is the useful one
  (`PQerrorMessage` returns NULL routinely).
- **A scalar return colliding with a sentinel is illegal** — a C function that
  legitimately returns `i64::MIN`, `NaN`, or 255-as-a-bool cannot be bound
  directly. It gets a shim, which is what shims are for.

Isolation for C that cannot be trusted to be well-behaved is @PLN119's job
(out-of-process placement), not this plan's.

## Sub-arcs

| Item | Status |
|---|---|
| **A** — `#c "sym" "<signature>"`: parser, the `CSignature` type, the compile-time check | **Done** — `src/c_signature.rs`, inert; widths resolve per target |
| **B** — the interpreter caller: `dlopen`/`dlsym` + the per-arity trampolines + return-width dispatch | Open |
| **C** — the `--native` caller: emit the typed `extern "C"` decl from `CSignature` | **Done** — loft calls libc directly, no rustc in any library |
| **D** — packaging: `[c] lib` / `shim`, `cc` shim build, `loft install` | Open |
| **E** — the other two targets (the parity arc) | Open — see below |
| **F** — prove it: a libpq subset for @PLN23, zero rustc | Open |

## Phase ordering

1. **A** first, and alone: the signature type is what every other arc consults,
   so it is the chokepoint the invariant lives or dies on. It lands inert.
2. **C before B.** The `--native` path is mostly existing machinery, so it is
   the cheaper proof that `CSignature` carries what a caller needs — and if it
   does not, that is much cheaper to learn there than inside new trampolines.
3. **B** — the trampolines, against `tests/fixtures/c_abi/` as its composition
   matrix. Both backends must agree cell for cell; the fixture's `make check`
   is the calibration that says the oracle itself is sound.
4. **D**, then **F** — a real library is the only thing that finds what the
   fixture did not imagine.
5. **E** last, and only against a real consumer's actual target list.

## Composition matrix — Stage A

**Built, and it ran before the design was written.**
[`tests/fixtures/c_abi/`](../../../../tests/fixtures/c_abi/) is the matrix: one
C function per loft type (every integer width, boundaries at the null sentinel,
`boolean`'s three states, `text` with an interior NUL, three vector shapes,
opaque handles, arities straddling the register/stack split) with hand-computed
expectations and a C self-test that validates the oracle before any loft is
involved. Arc B is done when every cell is green on both backends.

## Open design questions

1. ~~Does the C integer width need spelling in the declaration?~~ **Settled by
   measurement: yes, at least for returns.** Whether arguments also need it is a
   narrower question — the probe says they cross correctly regardless, so the
   spelling may be *required for the return, checked for the arguments*.
2. **Arc E — what does a `#c` library do on wasm and in the browser?** Three
   honest answers, and the choice is per library rather than global: compile the
   C to wasm with `clang --target=wasm32-wasi` and link it statically (works
   where the capability exists in wasm — a codec; not a database client, which
   needs sockets a browser cannot open at all); **relocate out-of-process**
   (@PLN119 — "another process" and "another machine" are already one mechanism
   there, which makes a browser calling a native-hosted C library the *same*
   shape as a second process); or declare the target unsupported, which the
   loft-ship gate already prefers over a claimed-but-broken column. The cost the
   owner flagged is bought here.
3. **Does a `#c` symbol need an effect declaration to be admissible?** The loft
   signature carries effects (`fs#read`) and the sandbox admits on them. A C
   symbol can do anything, so the declaration is an assertion the compiler
   cannot check — which may mean `#c` is simply inadmissible under `--sandbox`.
   Decide before D, not after someone ships a library.

## Cross-arc dependencies

- **@PLN23** (MariaDB + PostgreSQL clients) — the first consumer and the reason
  the fixture carries the opaque-handle shape. Blocked on this.
- **@PLN119** (out-of-process libraries) — arc E's most likely answer for any
  `#c` library whose capability does not exist in wasm at all.
- **@PLN21** (prebuilt native libs) — a `#c` library needs no rustc prebuild,
  only the system lib and the optional `cc` shim; the manifest keys are shared.

## See also

- [PACKAGES.md § Direct C binding](../../PACKAGES.md) — the reference home; this
  plan's outcome lands there.
- [NATIVE.md](../../NATIVE.md) — the Rust path this is measured against.
- [`tests/fixtures/c_abi/`](../../../../tests/fixtures/c_abi/) — the fixture and
  the probe, with the measured findings.
- [@PLN24](https://github.com/loft-lang/plans/issues/24) — the tracker issue.
